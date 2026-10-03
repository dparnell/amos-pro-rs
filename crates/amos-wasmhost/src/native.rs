//! Native backend: wasmtime (cranelift JIT).

use std::ptr::NonNull;
use std::rc::Rc;

use amos_core::compiled::{Env, HeapKind, RUN_RUNNING, RUN_STOPPED, Runtime, ST_GROW};
use amos_core::interp::verify::Compiled;
use amos_core::interp::{RunState, StopInfo, StopReason, StopReasonOrError};
use amos_core::{Machine, Program};
use wasmtime::{
    Caller, Engine, Global, GlobalType, Linker, Memory, MemoryType, Module, Mutability, Store, TypedFunc, Val, ValType,
};

/// Store data: the runtime, the machine during a `run` call, the memory.
struct Ctx {
    rt: Runtime,
    env: Option<NonNull<dyn Env>>,
    mem: Option<Memory>,
}

/// Calls `f` with the runtime, the machine and the module memory.
fn with<R>(c: &mut Caller<'_, Ctx>, f: impl FnOnce(&mut Runtime, &mut dyn Env, &mut [u8]) -> R) -> R {
    let mem = c.data().mem.expect("memory");
    let (r, reserve) = {
        let (data, ctx) = mem.data_and_store_mut(&mut *c);
        // SAFETY: `env` is set by `CompiledProgram::run` from a `&mut dyn
        // Env` that outlives the call into the module, and cleared after it;
        // the module only calls imports during that call.
        let env = unsafe { ctx.env.expect("machine").as_mut() };
        let len = data.len();
        // Gosubs done by the module go to the interpreter's stack first.
        ctx.rt.flush(env, data);
        let r = f(&mut ctx.rt, env, data);
        (r, ctx.rt.reserve_bytes(len))
    };
    // Keep free memory after the heap, so that the runtime never has to
    // grow the memory in the middle of a call (strings, arrays).
    if reserve > 0 {
        let _ = mem.grow(&mut *c, (reserve as u64).div_ceil(65536));
    }
    r
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
        CompiledProgram::new_cached(wasm, prg, None)
    }

    /// `new`, with the native code compiled from `wasm` kept in the
    /// directory `cache_dir` (if given): a later run of the same module
    /// (same wasmtime, configuration and target) loads it instead of
    /// compiling. Cache problems are ignored (the module is compiled).
    pub fn new_cached(
        wasm: &[u8],
        prg: Rc<Compiled>,
        cache_dir: Option<&std::path::Path>,
    ) -> Result<CompiledProgram, String> {
        let engine = engine();
        let module = match cache_dir {
            Some(dir) => crate::cache::module(engine, wasm, dir)?,
            None => Module::new(engine, wasm).map_err(|e| e.to_string())?,
        };
        let rt = Runtime::new(prg, 0, HeapKind::Linear);
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
        CompiledProgram::start_cached(machine, program, wasm, None)
    }

    /// `start`, with the compiled code cache of `new_cached`.
    pub fn start_cached(
        machine: &mut Machine,
        program: &Program,
        wasm: &[u8],
        cache_dir: Option<&std::path::Path>,
    ) -> Result<CompiledProgram, String> {
        machine
            .run_program(program)
            .map_err(|e| format!("{} (at {})", amos_core::errors::test_message(e.code), e.pos))?;
        let prg = machine.interp.prg.clone().ok_or("no program")?;
        CompiledProgram::new_cached(wasm, prg, cache_dir)
    }

    pub fn runtime(&self) -> &Runtime {
        &self.store.data().rt
    }

    /// Size of the module's memory in bytes (for tests).
    pub fn memory_size(&self) -> usize {
        self.store.data().mem.map_or(0, |m| m.data_size(&self.store))
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
                let trapped = self.store.data_mut().rt.take_trap_error();
                if let Some(mut env) = env {
                    // SAFETY: as in `with`, the machine is still borrowed.
                    let (it, _) = unsafe { env.as_mut() }.parts();
                    it.running = false;
                    it.wait = None;
                }
                let reason = match trapped {
                    Some(n) => StopReasonOrError::Error(n),
                    None => StopReasonOrError::Message(msg),
                };
                RunState::Stopped(StopInfo { reason, pos: 0 })
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
        include!("imports.rs");
        // Dim may need the memory to grow (arrays live in the module's
        // memory, after the procedure frames).
        l.func_wrap("host", "dim", |mut c: Caller<'_, Ctx>, p: i32, s: i32, t: i32, n: i32, r: i32| -> i32 {
            loop {
                let st = with(&mut c, |rt, env, mem| rt.dim(env, mem, p, s, t, n, r));
                if st != ST_GROW {
                    return st;
                }
                let bytes = c.data().rt.grow_bytes() as u64;
                let mem = c.data().mem.expect("memory");
                if mem.grow(&mut c, bytes.div_ceil(65536)).is_err() {
                    return with(&mut c, |rt, env, mem| rt.grow_failed(env, mem, p));
                }
            }
        })
        .map_err(|e| e.to_string())?;
        // A string chunk may need the memory to grow too.
        l.func_wrap("host", "str_chunk", |mut c: Caller<'_, Ctx>, need: i32| -> i32 {
            loop {
                let st = with(&mut c, |rt, _, mem| rt.str_chunk(mem, need));
                if st != ST_GROW {
                    return st;
                }
                let bytes = c.data().rt.grow_bytes() as u64;
                let mem = c.data().mem.expect("memory");
                if mem.grow(&mut c, bytes.div_ceil(65536)).is_err() {
                    return amos_core::compiled::ST_STOP;
                }
            }
        })
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
