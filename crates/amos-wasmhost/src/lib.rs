//! Native host for AMOS programs compiled to WebAssembly (wasmtime).
//!
//! [`CompiledProgram`] instantiates a module produced by `amos-compiler`
//! with its imports bound to [`amos_core::compiled::Runtime`], and runs it
//! with the same shape as the interpreter: [`CompiledProgram::run`] takes a
//! time budget and returns a [`RunState`]; [`CompiledProgram::vbl`] is the
//! equivalent of `Machine::vbl` for a [`Machine`] running a compiled
//! program.

use std::ptr::NonNull;
use std::rc::Rc;

use amos_core::compiled::{Env, RUN_RUNNING, RUN_STOPPED, Runtime};
use amos_core::interp::verify::Compiled;
use amos_core::interp::{RunState, StopInfo, StopReason, StopReasonOrError};
use amos_core::{Machine, Program};
use wasmtime::{
    Caller, Engine, Global, GlobalType, Linker, Memory, MemoryType, Module, Mutability, Store, TypedFunc, Val, ValType,
};

pub use amos_core::compiled::describe_stop;

/// Store data: the runtime, the machine during a `run` call, the memory.
struct Ctx {
    rt: Runtime,
    env: Option<NonNull<dyn Env>>,
    mem: Option<Memory>,
}

/// Calls `f` with the runtime, the machine and the module memory.
fn with<R>(c: &mut Caller<'_, Ctx>, f: impl FnOnce(&mut Runtime, &mut dyn Env, &mut [u8]) -> R) -> R {
    let mem = c.data().mem.expect("memory");
    let (data, ctx) = mem.data_and_store_mut(c);
    // SAFETY: `env` is set by `CompiledProgram::run` from a `&mut dyn Env`
    // that outlives the call into the module, and cleared after it; the
    // module only calls imports during that call.
    let env = unsafe { ctx.env.expect("machine").as_mut() };
    f(&mut ctx.rt, env, data)
}

macro_rules! def {
    ($l:expr, $m:literal $n:literal |$rt:ident, $env:ident, $mem:ident $(, $a:ident : $t:ty)*| $(-> $r:ty)? $body:block) => {
        $l.func_wrap($m, $n, |mut c: Caller<'_, Ctx> $(, $a: $t)*| $(-> $r)? {
            #[allow(unused_variables)]
            with(&mut c, |$rt, $env, $mem| $body)
        })
        .map_err(|e| e.to_string())?;
    };
}

/// A compiled program instantiated for one machine.
pub struct CompiledProgram {
    store: Store<Ctx>,
    run: TypedFunc<i32, i32>,
    /// The machine went on with another (interpreted) program (`Run`).
    detached: bool,
}

/// Engine shared by all compiled programs.
pub fn engine() -> &'static Engine {
    static ENGINE: std::sync::OnceLock<Engine> = std::sync::OnceLock::new();
    ENGINE.get_or_init(Engine::default)
}

impl CompiledProgram {
    /// Instantiates `wasm`, compiled from the verified program `prg` (the
    /// program the machine's interpreter was started with).
    pub fn new(wasm: &[u8], prg: Rc<Compiled>) -> Result<CompiledProgram, String> {
        let engine = engine();
        let module = Module::new(engine, wasm).map_err(|e| e.to_string())?;
        let rt = Runtime::new(prg, 0);
        let pages = rt.layout.pages;
        let mut store = Store::new(engine, Ctx { rt, env: None, mem: None });
        let memory = Memory::new(&mut store, MemoryType::new(pages, None)).map_err(|e| e.to_string())?;
        store.data_mut().mem = Some(memory);
        let base = Global::new(&mut store, GlobalType::new(ValType::I32, Mutability::Const), Val::I32(0))
            .map_err(|e| e.to_string())?;
        let mut l: Linker<Ctx> = Linker::new(engine);
        l.define(&store, "env", "memory", memory).map_err(|e| e.to_string())?;
        l.define(&store, "env", "base", base).map_err(|e| e.to_string())?;
        Self::define_imports(&mut l)?;
        let instance = l.instantiate(&mut store, &module).map_err(|e| e.to_string())?;
        let global = |store: &mut Store<Ctx>, name: &str| -> Result<i32, String> {
            match instance.get_global(&mut *store, name).map(|g| g.get(&mut *store)) {
                Some(Val::I32(v)) => Ok(v),
                _ => Err(format!("not an AMOS module (no {name})")),
            }
        };
        let abi = global(&mut store, "amos_abi")?;
        let hash = global(&mut store, "amos_hash")? as u32;
        store.data().rt.check_module(abi, hash)?;
        let run = instance.get_typed_func::<i32, i32>(&mut store, "run").map_err(|e| e.to_string())?;
        Ok(CompiledProgram { store, run, detached: false })
    }

    /// Starts `program` on `machine` (like `Machine::run_program`) and
    /// instantiates its compiled module.
    pub fn start(machine: &mut Machine, program: &Program, wasm: &[u8]) -> Result<CompiledProgram, String> {
        machine
            .run_program(program)
            .map_err(|e| format!("{} (at {})", amos_core::errors::test_message(e.code), e.pos))?;
        let prg = machine.interp.prg.clone().ok_or("no program")?;
        CompiledProgram::new(wasm, prg)
    }

    pub fn runtime(&self) -> &Runtime {
        &self.store.data().rt
    }

    /// Runs the program until it waits, stops, or `budget` jumps were done.
    pub fn run(&mut self, env: &mut (dyn Env + 'static), budget: usize) -> RunState {
        {
            let (it, _) = env.parts();
            if !it.running {
                return RunState::Idle;
            }
        }
        self.store.data_mut().env = Some(NonNull::from(env));
        let r = self.run.call(&mut self.store, budget.min(i32::MAX as usize) as i32);
        let env = self.store.data_mut().env.take();
        match r {
            Ok(RUN_RUNNING) => RunState::Running,
            Ok(RUN_STOPPED) => {
                let info = self.store.data_mut().rt.take_stop();
                RunState::Stopped(info.unwrap_or(StopInfo { reason: StopReasonOrError::Stop(StopReason::End), pos: 0 }))
            }
            other => {
                let msg = match other {
                    Err(e) => format!("Compiled program failed: {e}"),
                    Ok(s) => format!("Compiled program returned {s}"),
                };
                if let Some(mut env) = env {
                    // SAFETY: as in `with`, the machine is still borrowed.
                    let (it, _) = unsafe { env.as_mut() }.parts();
                    it.running = false;
                    it.wait = None;
                }
                RunState::Stopped(StopInfo { reason: StopReasonOrError::Message(msg), pos: 0 })
            }
        }
    }

    /// One vertical blank of a machine running this program (the compiled
    /// equivalent of `Machine::vbl`).
    pub fn vbl(&mut self, m: &mut Machine) {
        if self.detached {
            m.vbl();
            return;
        }
        self.detached = amos_core::compiled::machine_vbl(m, |m, budget| self.run(m, budget));
    }

    /// True once the machine runs another program (`Run`) with the
    /// interpreter.
    pub fn detached(&self) -> bool {
        self.detached
    }

    fn define_imports(l: &mut Linker<Ctx>) -> Result<(), String> {
        def!(l, "host" "enter" |rt, env, mem| -> i32 { rt.enter(env, mem) });
        def!(l, "host" "suspend" |rt, env, mem, p: i32| { rt.suspend(env, p) });
        def!(l, "host" "end_program" |rt, env, mem, last: i32| -> i32 { rt.end_program(env, last) });
        def!(l, "host" "raise" |rt, env, mem, p: i32, c: i32| -> i32 { rt.raise(env, mem, p, c) });
        def!(l, "host" "test_point" |rt, env, mem, p: i32| -> i32 { rt.test_point(env, mem, p) });
        def!(l, "host" "interp" |rt, env, mem, p: i32| -> i32 { rt.interp(env, mem, p) });
        def!(l, "host" "keyword" |rt, env, mem, p: i32| -> i32 { rt.keyword(env, mem, p) });
        def!(l, "host" "fn_i" |rt, env, mem, p: i32, f: i32| -> i32 { rt.fn_i(env, mem, p, f) });
        def!(l, "host" "fn_f" |rt, env, mem, p: i32, f: i32| -> f64 { rt.fn_f(env, mem, p, f) });
        def!(l, "host" "fn_n" |rt, env, mem, p: i32, f: i32| -> f64 { rt.fn_n(env, mem, p, f) });
        def!(l, "host" "fn_s" |rt, env, mem, p: i32, f: i32| -> i32 { rt.fn_s(env, mem, p, f) });
        def!(l, "host" "push_i" |rt, env, mem, v: i32| { rt.push_i(v) });
        def!(l, "host" "push_f" |rt, env, mem, v: f64| { rt.push_f(v) });
        def!(l, "host" "push_s" |rt, env, mem, v: i32| { rt.push_s(v) });
        def!(l, "host" "push_n" |rt, env, mem, v: f64, t: i32| { rt.push_n(v, t) });
        def!(l, "host" "print_begin" |rt, env, mem| { rt.print_begin() });
        def!(l, "host" "print_i" |rt, env, mem, v: i32| { rt.print_i(v) });
        def!(l, "host" "print_f" |rt, env, mem, v: f64| { rt.print_f(env, v) });
        def!(l, "host" "print_n" |rt, env, mem, v: f64, t: i32| { rt.print_n(env, v, t) });
        def!(l, "host" "print_s" |rt, env, mem, v: i32| { rt.print_s(v) });
        def!(l, "host" "print_tab" |rt, env, mem| { rt.print_tab() });
        def!(l, "host" "print_end" |rt, env, mem, p: i32, nl: i32| -> i32 { rt.print_end(env, mem, p, nl) });
        def!(l, "host" "aref" |rt, env, mem, s: i32, n: i32, a: i32, b: i32, c2: i32, d: i32, e: i32, f: i32, g: i32, h: i32| -> i32 {
            rt.aref(env, mem, s, n, [a, b, c2, d, e, f, g, h])
        });
        def!(l, "host" "aget_i" |rt, env, mem, s: i32, i: i32| -> i32 { rt.aget_i(env, s, i) });
        def!(l, "host" "aget_f" |rt, env, mem, s: i32, i: i32| -> f64 { rt.aget_f(env, s, i) });
        def!(l, "host" "aget_s" |rt, env, mem, s: i32, i: i32| -> i32 { rt.aget_s(env, mem, s, i) });
        def!(l, "host" "aset_i" |rt, env, mem, s: i32, i: i32, v: i32| { rt.aset_i(env, mem, s, i, v) });
        def!(l, "host" "aset_f" |rt, env, mem, s: i32, i: i32, v: f64| { rt.aset_f(env, mem, s, i, v) });
        def!(l, "host" "aset_s" |rt, env, mem, s: i32, i: i32, v: i32| { rt.aset_s(env, mem, s, i, v) });
        def!(l, "host" "for_push" |rt, env, mem, p: i32, s: i32, fl: i32, lim: i32, st: i32, b: i32, x: i32| -> i32 {
            rt.for_push(env, mem, p, s, fl, lim, st, b, x)
        });
        def!(l, "host" "next" |rt, env, mem, p: i32| -> i32 { rt.next(env, mem, p) });
        def!(l, "host" "loop_push" |rt, env, mem, p: i32, k: i32, b: i32, x: i32| -> i32 { rt.loop_push(env, mem, p, k, b, x) });
        def!(l, "host" "while_push" |rt, env, mem, p: i32, b: i32, x: i32| -> i32 { rt.while_push(env, mem, p, b, x) });
        def!(l, "host" "until" |rt, env, mem, p: i32, c: i32| -> i32 { rt.until(env, mem, p, c) });
        def!(l, "host" "wend" |rt, env, mem, p: i32| -> i32 { rt.wend(env, mem, p) });
        def!(l, "host" "loop_end" |rt, env, mem, p: i32| -> i32 { rt.loop_end(env, mem, p) });
        def!(l, "host" "exit" |rt, env, mem, p: i32, n: i32, t: i32| -> i32 { rt.exit(env, mem, p, n, t) });
        def!(l, "host" "goto_pos" |rt, env, mem, p: i32, t: i32| -> i32 { rt.goto_pos(env, mem, p, t) });
        def!(l, "host" "goto_label" |rt, env, mem, p: i32, i: i32| -> i32 { rt.goto_label(env, mem, p, i) });
        def!(l, "host" "gosub_label" |rt, env, mem, p: i32, i: i32, r: i32| -> i32 { rt.gosub_label(env, mem, p, i, r) });
        def!(l, "host" "return" |rt, env, mem, p: i32| -> i32 { rt.return_(env, mem, p) });
        def!(l, "host" "call" |rt, env, mem, p: i32, i: i32, r: i32, n: i32| -> i32 { rt.call(env, mem, p, i, r, n) });
        def!(l, "host" "proc_check" |rt, env, mem, p: i32, pop: i32| -> i32 { rt.proc_check(env, mem, p, pop) });
        def!(l, "host" "set_param_i" |rt, env, mem, v: i32| { rt.set_param_i(env, v) });
        def!(l, "host" "set_param_f" |rt, env, mem, v: f64| { rt.set_param_f(env, v) });
        def!(l, "host" "set_param_s" |rt, env, mem, v: i32| { rt.set_param_s(env, v) });
        def!(l, "host" "proc_return" |rt, env, mem, p: i32| -> i32 { rt.proc_return(env, mem, p) });
        def!(l, "host" "dyn_op" |rt, env, mem, op: i32, a: f64, ta: i32, b: f64, tb: i32| -> f64 {
            rt.dyn_op(env, mem, op, a, ta, b, tb)
        });
        def!(l, "host" "next_done" |rt, env, mem, p: i32| -> i32 { rt.next_done(env, mem, p) });
        def!(l, "host" "str_f" |rt, env, mem, x: f64| -> i32 { rt.str_f(env, mem, x) });
        def!(l, "host" "param_i" |rt, env, mem| -> i32 { rt.param_i(env) });
        def!(l, "host" "param_f" |rt, env, mem| -> f64 { rt.param_f(env) });
        def!(l, "host" "param_s" |rt, env, mem| -> i32 { rt.param_s(env, mem) });
        def!(l, "rt" "str_len" |rt, env, mem, h: i32| -> i32 { rt.str_len(h) });
        def!(l, "rt" "str_asc" |rt, env, mem, h: i32| -> i32 { rt.str_asc(h) });
        def!(l, "rt" "chr" |rt, env, mem, n: i32| -> i32 { rt.chr(mem, n) });
        def!(l, "rt" "left_right" |rt, env, mem, h: i32, n: i32, r: i32| -> i32 { rt.left_right(mem, h, n, r) });
        def!(l, "rt" "mid" |rt, env, mem, h: i32, p: i32, n: i32, hn: i32| -> i32 { rt.mid(mem, h, p, n, hn) });
        def!(l, "rt" "str_i" |rt, env, mem, n: i32| -> i32 { rt.str_i(mem, n) });
        def!(l, "rt" "instr" |rt, env, mem, h: i32, n: i32, s: i32, hs: i32| -> i32 { rt.instr(mem, h, n, s, hs) });
        def!(l, "rt" "change_case" |rt, env, mem, h: i32, lw: i32| -> i32 { rt.change_case(mem, h, lw) });
        def!(l, "rt" "int_f" |rt, env, mem, x: f64| -> f64 { rt.int_f(x) });
        def!(l, "host" "wait" |rt, env, mem, p: i32, n: i32| -> i32 { rt.wait(env, mem, p, n) });
        def!(l, "rt" "str_const" |rt, env, mem, p: i32| -> i32 { rt.str_const(mem, p) });
        def!(l, "rt" "str_concat" |rt, env, mem, a: i32, b: i32| -> i32 { rt.str_concat(mem, a, b) });
        def!(l, "rt" "str_minus" |rt, env, mem, a: i32, b: i32| -> i32 { rt.str_minus(mem, a, b) });
        def!(l, "rt" "str_cmp" |rt, env, mem, a: i32, b: i32| -> i32 { rt.str_cmp(a, b) });
        def!(l, "rt" "i2f" |rt, env, mem, v: i32| -> f64 { rt.i2f(v) });
        def!(l, "rt" "fadd" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fadd(a, b) });
        def!(l, "rt" "fsub" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fsub(a, b) });
        def!(l, "rt" "fmul" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fmul(a, b) });
        def!(l, "rt" "fdiv" |rt, env, mem, a: f64, b: f64| -> f64 { rt.fdiv(a, b) });
        def!(l, "rt" "fcmp" |rt, env, mem, a: f64, b: f64| -> i32 { rt.fcmp(a, b) });
        def!(l, "rt" "pow" |rt, env, mem, a: f64, b: f64| -> f64 { rt.pow(a, b) });
        Ok(())
    }
}
