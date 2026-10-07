//! The WebAssembly runtime: compiling a plugin's module and running one filter call.
//!
//! WebAssembly is not, by itself, a sandbox. A module is confined only by what the
//! host gives it, and this file is where that is decided, so it is deliberately the
//! smallest thing it can be:
//!
//! * **One import, and it does nothing but log.** The module is linked against
//!   `photoforge.log` and nothing else. There is no WASI: no filesystem, no clock,
//!   no environment, no random numbers, no sockets. A module that imports anything
//!   else is refused when it is compiled, naming the import, so the refusal happens
//!   at install time and not in the middle of an edit.
//! * **A fresh instance for every call.** State cannot leak from one tile to the
//!   next, or from one image to another, because nothing survives a call. A filter
//!   is a pure function of its input pixels and its parameters, which is also what
//!   lets the renderer cache and tile it.
//! * **Memory is capped** by a [`ResourceLimiter`], including the module's initial
//!   size, so a module that declares gigabytes fails to instantiate.
//! * **Work is capped by fuel and time by an epoch.** Fuel is the deterministic
//!   bound on instructions executed; the epoch is the wall-clock bound and the path
//!   cancellation takes. Both are checked at loop back-edges, so a loop that never
//!   returns is stopped.
//! * **Results are reproducible.** NaN bit patterns are canonicalised, relaxed SIMD
//!   (whose results vary by machine) is off, and the engine is built without thread
//!   support at all (the `threads` feature is not enabled), so a filter that declares
//!   itself deterministic can be.
//! * **The output is checked before it is believed:** the right size, and finite.
//!
//! The ABI is in `docs/plugin-api.md`.
use super::job::{TileJob, TileResult};
use super::limits::{self, CallLimits};
use super::manifest::API_VERSION;
use super::PluginError;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use wasmtime::{
    Caller, Config, Engine, Extern, ExternType, FuncType, Linker, Memory, Module, ResourceLimiter,
    Store, Trap, UpdateDeadline, ValType,
};

/// The module name and function the host provides.
pub const HOST_MODULE: &str = "photoforge";
pub const HOST_LOG: &str = "log";

const STOP_NONE: u8 = 0;
const STOP_TIMEOUT: u8 = 1;
const STOP_CANCELLED: u8 = 2;

/// A module that has been compiled and whose imports and exports have been checked.
#[derive(Clone)]
pub struct Compiled {
    module: Module,
}

struct Limiter {
    memory_bytes: usize,
    exceeded: bool,
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.memory_bytes {
            self.exceeded = true;
            return Ok(false);
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= limits::MAX_TABLE_ELEMENTS as usize)
    }

    fn instances(&self) -> usize {
        limits::MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        limits::MAX_TABLES
    }

    fn memories(&self) -> usize {
        limits::MAX_MEMORIES
    }
}

struct Host {
    limiter: Limiter,
    logs: Vec<String>,
}

pub struct Runtime {
    engine: Engine,
    stop_ticker: Arc<AtomicBool>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop_ticker.store(true, Ordering::Release);
    }
}

fn runtime_error(message: impl std::fmt::Display) -> PluginError {
    PluginError::Runtime(message.to_string())
}

impl Runtime {
    pub const fn available() -> bool {
        true
    }

    pub fn new() -> Result<Self, PluginError> {
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .epoch_interruption(true)
            .cranelift_nan_canonicalization(true)
            .wasm_relaxed_simd(false)
            .wasm_multi_memory(false)
            .wasm_memory64(false)
            .max_wasm_stack(512 * 1024);
        let engine = Engine::new(&config).map_err(runtime_error)?;

        // One thread advances the epoch for every store of this engine.
        let stop_ticker = Arc::new(AtomicBool::new(false));
        {
            let engine = engine.clone();
            let stop = Arc::clone(&stop_ticker);
            std::thread::Builder::new()
                .name("photoforge-plugin-epoch".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        std::thread::sleep(limits::EPOCH_TICK);
                        engine.increment_epoch();
                    }
                })
                .map_err(runtime_error)?;
        }
        Ok(Self {
            engine,
            stop_ticker,
        })
    }

    /// Compiles a module and checks that it asks for nothing the host does not give
    /// and offers everything a filter needs.
    pub fn compile(&self, wasm: &[u8]) -> Result<Compiled, PluginError> {
        let module = Module::new(&self.engine, wasm)
            .map_err(|error| PluginError::Module(format!("it does not compile: {error}")))?;

        let log_type = FuncType::new(&self.engine, std::iter::repeat_n(ValType::I32, 3), []);
        for import in module.imports() {
            let allowed = import.module() == HOST_MODULE
                && import.name() == HOST_LOG
                && matches!(import.ty(), ExternType::Func(ty) if ty.matches(&log_type));
            if !allowed {
                return Err(PluginError::Module(format!(
                    "it imports {}.{}, which PhotoForge does not provide. A plugin can import \
                     only {HOST_MODULE}.{HOST_LOG}; there is no filesystem, clock, network or \
                     environment to ask for.",
                    import.module(),
                    import.name()
                )));
            }
        }

        let exports: Vec<(String, ExternType)> = module
            .exports()
            .map(|export| (export.name().to_string(), export.ty()))
            .collect();
        let find = |name: &str| exports.iter().find(|(n, _)| n == name).map(|(_, ty)| ty);
        match find("memory") {
            Some(ExternType::Memory(memory)) if !memory.is_64() && !memory.is_shared() => {}
            _ => {
                return Err(PluginError::Module(
                    "it does not export a 32-bit memory named \"memory\"".into(),
                ))
            }
        }
        let expect =
            |name: &str, params: &[ValType], results: &[ValType]| -> Result<(), PluginError> {
                let wanted = FuncType::new(
                    &self.engine,
                    params.iter().cloned(),
                    results.iter().cloned(),
                );
                match find(name) {
                    Some(ExternType::Func(actual)) if actual.matches(&wanted) => Ok(()),
                    Some(_) => Err(PluginError::Module(format!(
                        "its export {name} has the wrong signature"
                    ))),
                    None => Err(PluginError::Module(format!("it does not export {name}"))),
                }
            };
        expect("pf_abi_version", &[], &[ValType::I32])?;
        expect("pf_alloc", &[ValType::I32], &[ValType::I32])?;
        expect(
            "pf_filter",
            &[
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
            ],
            &[ValType::I32],
        )?;
        Ok(Compiled { module })
    }

    /// Runs one filter call in a fresh instance.
    pub fn call(
        &self,
        compiled: &Compiled,
        job: &TileJob<'_>,
        limits: CallLimits,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<TileResult, PluginError> {
        let input_pixels = job.input_rect.pixels();
        let output_pixels = job.output_rect.pixels();
        let required =
            limits::call_memory_required(input_pixels, output_pixels, job.parameters.len());
        if required > limits.memory_bytes {
            return Err(PluginError::Refused(format!(
                "this needs about {} MiB for the plugin to work in and it may use {} MiB",
                required.div_ceil(limits::MIB),
                limits.memory_bytes / limits::MIB
            )));
        }
        if job.input.len() as u64 != input_pixels * limits::BYTES_PER_PIXEL {
            return Err(runtime_error("the input is not the size of its window"));
        }

        let mut store = Store::new(
            &self.engine,
            Host {
                limiter: Limiter {
                    memory_bytes: limits.memory_bytes as usize,
                    exceeded: false,
                },
                logs: Vec::new(),
            },
        );
        store.limiter(|host| &mut host.limiter);
        store.set_fuel(limits.fuel).map_err(runtime_error)?;

        let stop = Arc::new(AtomicU8::new(STOP_NONE));
        let deadline = std::time::Instant::now() + limits.time;
        {
            let stop = Arc::clone(&stop);
            let cancel = cancel.cloned();
            store.set_epoch_deadline(1);
            store.epoch_deadline_callback(move |_| {
                if cancel
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Acquire))
                {
                    stop.store(STOP_CANCELLED, Ordering::Release);
                    return Ok(UpdateDeadline::Interrupt);
                }
                if std::time::Instant::now() >= deadline {
                    stop.store(STOP_TIMEOUT, Ordering::Release);
                    return Ok(UpdateDeadline::Interrupt);
                }
                Ok(UpdateDeadline::Continue(1))
            });
        }

        let mut linker = Linker::<Host>::new(&self.engine);
        linker
            .func_wrap(
                HOST_MODULE,
                HOST_LOG,
                |mut caller: Caller<'_, Host>, level: i32, pointer: i32, length: i32| {
                    // Guest text is untrusted: bounded in count and length, and
                    // read defensively. An out-of-range request is dropped, not
                    // an error — a plugin may not fail a render by logging badly.
                    if caller.data().logs.len() >= limits::LOG_LINES_PER_CALL {
                        return;
                    }
                    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                        return;
                    };
                    let (Ok(start), Ok(length)) =
                        (usize::try_from(pointer), usize::try_from(length))
                    else {
                        return;
                    };
                    let length = length.min(limits::LOG_LINE_BYTES);
                    let data = memory.data(&caller);
                    let Some(bytes) = start
                        .checked_add(length)
                        .and_then(|end| data.get(start..end))
                    else {
                        return;
                    };
                    let text: String = String::from_utf8_lossy(bytes)
                        .chars()
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .collect();
                    let label = match level {
                        0 => "debug",
                        1 => "info",
                        2 => "warn",
                        _ => "error",
                    };
                    caller.data_mut().logs.push(format!("{label}: {text}"));
                },
            )
            .map_err(runtime_error)?;

        let classify =
            |error: wasmtime::Error, stop: &AtomicU8, store: &Store<Host>| -> PluginError {
                match stop.load(Ordering::Acquire) {
                    STOP_CANCELLED => return PluginError::Cancelled,
                    STOP_TIMEOUT => return PluginError::TimedOut,
                    _ => {}
                }
                match error.downcast_ref::<Trap>() {
                    Some(Trap::OutOfFuel) => PluginError::OutOfFuel,
                    _ if store.data().limiter.exceeded => PluginError::Trap(format!(
                        "it tried to use more than the {} MiB it is allowed: {error}",
                        limits.memory_bytes / limits::MIB
                    )),
                    _ => PluginError::Trap(format!("{error}")),
                }
            };

        let instance = linker
            .instantiate(&mut store, &compiled.module)
            .map_err(|error| classify(error, &stop, &store))?;
        let memory: Memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| PluginError::Module("it does not export its memory".into()))?;
        let version = instance
            .get_typed_func::<(), i32>(&mut store, "pf_abi_version")
            .map_err(runtime_error)?
            .call(&mut store, ())
            .map_err(|error| classify(error, &stop, &store))?;
        if version != API_VERSION as i32 {
            return Err(PluginError::Module(format!(
                "it was built for host interface {version} and this build provides {API_VERSION}"
            )));
        }
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "pf_alloc")
            .map_err(runtime_error)?;
        let filter = instance
            .get_typed_func::<(
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
                i32,
            ), i32>(&mut store, "pf_filter")
            .map_err(runtime_error)?;

        let allocate = |store: &mut Store<Host>, bytes: u64| -> Result<i32, PluginError> {
            let size = i32::try_from(bytes).map_err(|_| {
                PluginError::Refused(
                    "a buffer would exceed what a 32-bit module can address".into(),
                )
            })?;
            let pointer = alloc
                .call(&mut *store, size)
                .map_err(|error| classify(error, &stop, store))?;
            let start = usize::try_from(pointer)
                .map_err(|_| PluginError::Trap("pf_alloc returned a negative pointer".into()))?;
            let end = start.checked_add(bytes as usize);
            if pointer == 0 || end.is_none_or(|end| end > memory.data_size(&*store)) {
                return Err(PluginError::Trap(
                    "pf_alloc returned a pointer outside the module's memory".into(),
                ));
            }
            Ok(pointer)
        };

        let input_bytes = job.input.len() as u64;
        let output_bytes = output_pixels * limits::BYTES_PER_PIXEL;
        let params_bytes = (job.parameters.len() as u64) * 8;
        let input_pointer = allocate(&mut store, input_bytes)?;
        let output_pointer = allocate(&mut store, output_bytes)?;
        let params_pointer = if params_bytes == 0 {
            0
        } else {
            allocate(&mut store, params_bytes)?
        };

        {
            let data = memory.data_mut(&mut store);
            data[input_pointer as usize..input_pointer as usize + job.input.len()]
                .copy_from_slice(job.input);
            for (index, value) in job.parameters.iter().enumerate() {
                let at = params_pointer as usize + index * 8;
                data[at..at + 8].copy_from_slice(&value.to_le_bytes());
            }
        }

        let signed = |value: u32| -> Result<i32, PluginError> {
            i32::try_from(value).map_err(|_| {
                PluginError::Refused("a dimension exceeds what a 32-bit module can address".into())
            })
        };
        let code = filter
            .call(
                &mut store,
                (
                    signed(job.filter_index)?,
                    input_pointer,
                    signed(job.input_rect.x)?,
                    signed(job.input_rect.y)?,
                    signed(job.input_rect.width)?,
                    signed(job.input_rect.height)?,
                    output_pointer,
                    signed(job.output_rect.x)?,
                    signed(job.output_rect.y)?,
                    signed(job.output_rect.width)?,
                    signed(job.output_rect.height)?,
                    signed(job.image_width)?,
                    signed(job.image_height)?,
                    params_pointer,
                    job.parameters.len() as i32,
                ),
            )
            .map_err(|error| classify(error, &stop, &store))?;
        if code != 0 {
            return Err(PluginError::Reported { code });
        }

        let start = output_pointer as usize;
        let end = start + output_bytes as usize;
        let output = memory
            .data(&store)
            .get(start..end)
            .ok_or_else(|| PluginError::Trap("the output lies outside the module's memory".into()))?
            .to_vec();
        let fuel_used = limits.fuel.saturating_sub(store.get_fuel().unwrap_or(0));
        let logs = std::mem::take(&mut store.data_mut().logs);
        Ok(TileResult {
            output,
            logs,
            fuel_used,
        })
    }
}
