//! Web backend: the module is compiled and instantiated by the JavaScript
//! engine (`WebAssembly.instantiate`), its imports are JavaScript functions
//! (wasm-bindgen closures) calling the shared [`Runtime`].
//!
//! The module imports the memory of the runtime itself (`env.memory`): its
//! variables live in a block allocated here, whose address is `env.base`,
//! so the runtime reads and writes them directly.

use std::cell::RefCell;
use std::rc::Rc;

use amos_core::compiled::{Env, HeapKind, RUN_RUNNING, RUN_STOPPED, Runtime};
use amos_core::interp::verify::Compiled;
use amos_core::interp::{RunState, StopInfo, StopReason, StopReasonOrError};
use amos_core::{Machine, Program};
use js_sys::{Function, Object, Reflect, WebAssembly};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

struct Ctx {
    rt: Runtime,
    /// The machine during a `run` call.
    env: Option<*mut (dyn Env + 'static)>,
    /// The module's memory block (owned by the `CompiledProgram`).
    mem: *mut u8,
    len: usize,
}

type Shared = Rc<RefCell<Ctx>>;

/// Calls `f` with the runtime, the machine and the module memory.
fn with<R>(ctx: &Shared, f: impl FnOnce(&mut Runtime, &mut dyn Env, &mut [u8]) -> R) -> R {
    let mut c = ctx.borrow_mut();
    let Ctx { rt, env, mem, len } = &mut *c;
    // SAFETY: the block is allocated by `CompiledProgram::new_async` and
    // freed only when the program is dropped (after which the module is not
    // called any more); `env` is set by `run` from a `&mut dyn Env` that
    // outlives the call into the module, and the module calls its imports
    // only during that call.
    let mem = unsafe { std::slice::from_raw_parts_mut(*mem, *len) };
    let env = unsafe { &mut *env.expect("imports are only called during run") };
    f(rt, env, mem)
}

/// The import objects being built.
struct Imports {
    ctx: Shared,
    host: Object,
    rt: Object,
}

macro_rules! def {
    ($l:expr, $m:literal $n:literal |$rt:ident, $env:ident, $mem:ident $(, $a:ident : $t:ty)*| $(-> $r:ty)? $body:block) => {{
        let c = $l.ctx.clone();
        let f = Closure::<dyn FnMut($($t),*) $(-> $r)?>::new(move |$($a: $t),*| $(-> $r)? {
            #[allow(unused_variables)]
            with(&c, |$rt, $env, $mem| $body)
        });
        let target = if $m == "host" { &$l.host } else { &$l.rt };
        Reflect::set(target, &JsValue::from_str($n), &f.into_js_value()).map_err(js_err)?;
    }};
}

fn js_err(e: JsValue) -> String {
    e.as_string()
        .or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())))
        .unwrap_or_else(|| format!("{e:?}"))
}

fn define_imports(l: &mut Imports) -> Result<(), String> {
    include!("imports.rs");
    // Arrays are separate allocations of the runtime: no memory to grow.
    def!(l, "host" "dim" |rt, env, mem, p: i32, s: i32, t: i32, n: i32, r: i32| -> i32 { rt.dim(env, mem, p, s, t, n, r) });
    Ok(())
}

/// A compiled program instantiated for one machine.
pub struct CompiledProgram {
    ctx: Shared,
    run: Function,
    /// The module's memory block (`Box<[u8]>` turned into a raw pointer:
    /// the module writes it behind Rust's back).
    block: *mut [u8],
    detached: bool,
}

impl Drop for CompiledProgram {
    fn drop(&mut self) {
        // SAFETY: allocated with `Box::into_raw` in `new_async`.
        drop(unsafe { Box::from_raw(self.block) });
    }
}

impl CompiledProgram {
    /// Compiles and instantiates `wasm`, compiled from the verified program
    /// `prg`, with the browser's WebAssembly API.
    pub async fn new_async(wasm: &[u8], prg: Rc<Compiled>) -> Result<CompiledProgram, String> {
        let size = amos_core::compiled::layout::Layout::new(&prg).size as usize;
        let block: *mut [u8] = Box::into_raw(vec![0u8; size].into_boxed_slice());
        let base = block as *mut u8 as usize as u32;
        let ctx: Shared = Rc::new(RefCell::new(Ctx {
            rt: Runtime::new(prg, base, HeapKind::Owned),
            env: None,
            mem: block as *mut u8,
            len: size,
        }));
        let result = Self::instantiate(wasm, &ctx, base).await;
        let (run, abi, hash) = match result {
            Ok(v) => v,
            Err(e) => {
                // SAFETY: allocated above, not shared with a module.
                drop(unsafe { Box::from_raw(block) });
                return Err(e);
            }
        };
        let program = CompiledProgram { ctx, run, block, detached: false };
        program.ctx.borrow().rt.check_module(abi, hash)?;
        Ok(program)
    }

    async fn instantiate(wasm: &[u8], ctx: &Shared, base: u32) -> Result<(Function, i32, u32), String> {
        let mut l = Imports { ctx: ctx.clone(), host: Object::new(), rt: Object::new() };
        define_imports(&mut l)?;
        let env = Object::new();
        Reflect::set(&env, &"memory".into(), &wasm_bindgen::memory()).map_err(js_err)?;
        let desc = Object::new();
        Reflect::set(&desc, &"value".into(), &"i32".into()).map_err(js_err)?;
        Reflect::set(&desc, &"mutable".into(), &false.into()).map_err(js_err)?;
        let base = WebAssembly::Global::new(&desc, &JsValue::from(base as i32)).map_err(js_err)?;
        Reflect::set(&env, &"base".into(), &base).map_err(js_err)?;
        let imports = Object::new();
        Reflect::set(&imports, &"env".into(), &env).map_err(js_err)?;
        Reflect::set(&imports, &"host".into(), &l.host).map_err(js_err)?;
        Reflect::set(&imports, &"rt".into(), &l.rt).map_err(js_err)?;
        let promise = WebAssembly::instantiate_buffer(wasm, &imports);
        let result = wasm_bindgen_futures::JsFuture::from(promise).await.map_err(js_err)?;
        let instance = Reflect::get(&result, &"instance".into()).map_err(js_err)?;
        let exports = Reflect::get(&instance, &"exports".into()).map_err(js_err)?;
        let global = |name: &str| -> Result<i32, String> {
            let g = Reflect::get(&exports, &name.into()).map_err(js_err)?;
            let v = Reflect::get(&g, &"value".into()).map_err(js_err)?;
            v.as_f64().map(|v| v as i32).ok_or_else(|| format!("not an AMOS module (no {name})"))
        };
        let (abi, hash) = (global("amos_abi")?, global("amos_hash")? as u32);
        let run = Reflect::get(&exports, &"run".into())
            .map_err(js_err)?
            .dyn_into::<Function>()
            .map_err(|_| "not an AMOS module (no run)".to_string())?;
        Ok((run, abi, hash))
    }

    /// Starts `program` on `machine` (like `Machine::run_program`) and
    /// instantiates its compiled module.
    pub async fn start_async(machine: &mut Machine, program: &Program, wasm: &[u8]) -> Result<CompiledProgram, String> {
        machine
            .run_program(program)
            .map_err(|e| format!("{} (at {})", amos_core::errors::test_message(e.code), e.pos))?;
        let prg = machine.interp.prg.clone().ok_or("no program")?;
        CompiledProgram::new_async(wasm, prg).await
    }

    /// Runs the program until it waits, stops, or `budget` instructions
    /// were executed.
    pub fn run(&mut self, env: &mut (dyn Env + 'static), budget: usize) -> RunState {
        {
            let (it, _) = env.parts();
            if !it.running {
                return RunState::Idle;
            }
        }
        let env_ptr: *mut (dyn Env + 'static) = env;
        self.ctx.borrow_mut().env = Some(env_ptr);
        let r = self.run.call1(&JsValue::NULL, &JsValue::from(budget.min(i32::MAX as usize) as i32));
        self.ctx.borrow_mut().env = None;
        match r.map(|v| v.as_f64().map(|v| v as i32)) {
            Ok(Some(RUN_RUNNING)) => RunState::Running,
            Ok(Some(RUN_STOPPED)) => {
                let info = self.ctx.borrow_mut().rt.take_stop();
                RunState::Stopped(info.unwrap_or(StopInfo { reason: StopReasonOrError::Stop(StopReason::End), pos: 0 }))
            }
            other => {
                let msg = match other {
                    Err(e) => format!("Compiled program failed: {}", js_err(e)),
                    Ok(s) => format!("Compiled program returned {s:?}"),
                };
                let (it, _) = env.parts();
                it.running = false;
                it.wait = None;
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
}
