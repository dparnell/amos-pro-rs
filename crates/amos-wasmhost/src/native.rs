//! Native backend: wasmtime (cranelift JIT).

use std::ptr::NonNull;
use std::rc::Rc;

use amos_core::compiled::{Env, RUN_RUNNING, RUN_STOPPED, Runtime};
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
        include!("imports.rs");
        Ok(())
    }
}
