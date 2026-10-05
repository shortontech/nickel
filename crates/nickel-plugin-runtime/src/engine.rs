//! Private JavaScript-engine boundary.
//!
//! The JSX/component contract lives in `bootstrap.js`; native reconciliation
//! consumes typed envelopes. Engine values never cross this module.

use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EngineDiagnostics {
    pub invocations: u64,
    /// Source compilation attempts, distinct from direct calls to loaded code.
    pub script_compilations: u64,
    pub failed_invocations: u64,
    pub deadline_terminations: u64,
    pub javascript_micros: u64,
    pub bridge_micros: u64,
    /// JSON value nodes actually converted into V8 arguments (not byte size).
    pub bridge_values: u64,
    pub microtask_checkpoints: u64,
    pub microtask_micros: u64,
    pub gc_collections: u64,
    pub gc_micros: u64,
    /// Foreground tasks actually executed, including batches later rejected.
    pub platform_tasks_serviced: u64,
    pub used_heap_bytes: usize,
    pub peak_used_heap_bytes: usize,
    pub total_heap_bytes: usize,
    pub heap_limit_bytes: usize,
    // V8-owned accounting, not additive to RSS or glibc live bytes. These help
    // distinguish retained contexts/handles from external or engine allocations.
    pub external_memory_bytes: usize,
    pub malloced_memory_bytes: usize,
    pub used_global_handles_bytes: usize,
    pub native_contexts: usize,
    pub detached_contexts: usize,
}

mod implementation {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Once, OnceLock, mpsc};
    use std::time::{Duration, Instant};

    enum NativeCallResult {
        Boolean(bool),
        JsonText(String),
    }

    const INITIAL_HEAP_BYTES: usize = 4 * 1024 * 1024;
    const MAX_HEAP_BYTES: usize = 64 * 1024 * 1024;
    const EXECUTION_DEADLINE: Duration = Duration::from_millis(100);
    fn pump_ready_task_batch(
        mut run_ready_task: impl FnMut() -> bool,
        mut stop: impl FnMut() -> bool,
    ) -> usize {
        let mut count = 0;
        while count < 8 && !stop() {
            if !run_ready_task() {
                break;
            }
            count += 1;
        }
        count
    }

    #[test]
    fn foreground_task_admission_preserves_remaining_work_and_stop_boundary() {
        use std::cell::Cell;
        let queued = Cell::new(20);
        let run = || {
            queued.set(queued.get() - 1);
            true
        };
        assert_eq!(pump_ready_task_batch(run, || false), 8);
        assert_eq!(queued.get(), 12);
        assert_eq!(pump_ready_task_batch(run, || true), 0);
        assert_eq!(queued.get(), 12);
        assert_eq!(pump_ready_task_batch(run, || queued.get() == 9), 3);
        assert_eq!(queued.get(), 9);
        let probes = Cell::new(0);
        assert_eq!(
            pump_ready_task_batch(
                || {
                    probes.set(probes.get() + 1);
                    false
                },
                || false
            ),
            0
        );
        assert_eq!(probes.get(), 1, "empty queue must not spin or wait");
    }
    static INITIALIZE_V8: Once = Once::new();
    static NEXT_DEADLINE_ID: AtomicU64 = AtomicU64::new(1);
    static WATCHDOG: OnceLock<mpsc::Sender<Deadline>> = OnceLock::new();

    struct Deadline {
        id: u64,
        expires: Instant,
        active: Arc<AtomicU64>,
        isolate: v8::IsolateHandle,
    }

    struct DeadlineGuard(Arc<AtomicU64>);

    #[derive(Default)]
    struct GcDiagnostics {
        collections: u64,
        micros: u64,
        started: Option<Instant>,
    }

    unsafe extern "C" fn gc_prologue(
        _isolate: v8::UnsafeRawIsolatePtr,
        _kind: v8::GCType,
        _flags: v8::GCCallbackFlags,
        data: *mut std::ffi::c_void,
    ) {
        // SAFETY: `data` points at the engine-owned boxed state, which remains
        // pinned for the complete isolate lifetime. V8 invokes GC callbacks on
        // the isolate's owning thread and does not re-enter this callback.
        let state = unsafe { &mut *data.cast::<GcDiagnostics>() };
        state.started = Some(Instant::now());
    }

    unsafe extern "C" fn gc_epilogue(
        _isolate: v8::UnsafeRawIsolatePtr,
        _kind: v8::GCType,
        _flags: v8::GCCallbackFlags,
        data: *mut std::ffi::c_void,
    ) {
        // SAFETY: see `gc_prologue`; both callbacks receive the same stable
        // engine-owned allocation.
        let state = unsafe { &mut *data.cast::<GcDiagnostics>() };
        state.collections = state.collections.saturating_add(1);
        if let Some(started) = state.started.take() {
            state.micros = state.micros.saturating_add(elapsed_micros(started));
        }
    }

    fn elapsed_micros(started: Instant) -> u64 {
        started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }

    impl Drop for DeadlineGuard {
        fn drop(&mut self) {
            self.0.store(0, Ordering::Release);
        }
    }

    fn watchdog() -> &'static mpsc::Sender<Deadline> {
        WATCHDOG.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Deadline>();
            std::thread::Builder::new()
                .name("nickel-v8-watchdog".into())
                .spawn(move || {
                    let mut pending = Vec::<Deadline>::new();
                    loop {
                        let now = Instant::now();
                        let wait = pending
                            .iter()
                            .map(|deadline| deadline.expires.saturating_duration_since(now))
                            .min();
                        let received = match wait {
                            Some(wait) => receiver.recv_timeout(wait),
                            None => match receiver.recv() {
                                Ok(deadline) => {
                                    pending.push(deadline);
                                    continue;
                                }
                                Err(_) => break,
                            },
                        };
                        match received {
                            Ok(deadline) => pending.push(deadline),
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                        let now = Instant::now();
                        pending.retain(|deadline| {
                            if deadline.expires > now {
                                return true;
                            }
                            if deadline.active.load(Ordering::Acquire) == deadline.id {
                                deadline.isolate.terminate_execution();
                            }
                            false
                        });
                    }
                })
                .expect("Nickel must be able to start the V8 execution watchdog");
            sender
        })
    }

    pub(crate) struct JavascriptEngine {
        context: v8::Global<v8::Context>,
        isolate: v8::OwnedIsolate,
        active_deadline: Arc<AtomicU64>,
        diagnostics: EngineDiagnostics,
        gc_diagnostics: Box<GcDiagnostics>,
    }

    impl JavascriptEngine {
        pub(crate) fn new() -> Self {
            INITIALIZE_V8.call_once(|| {
                // Nickel runs bounded synchronous calls, not a V8 foreground
                // event loop. Speculative minor-GC tasks otherwise accumulate
                // as canceled task objects in DefaultForegroundTaskRunner.
                // Keep ordinary allocation-triggered GC; do not schedule work
                // that this embedding cannot service. Revisit with any future
                // transaction-aware foreground-task integration.
                v8::V8::set_flags_from_string("--no-minor-gc-task");
                let platform = v8::new_default_platform(0, false).make_shared();
                v8::V8::initialize_platform(platform);
                v8::V8::initialize();
            });
            let params =
                v8::CreateParams::default().heap_limits(INITIAL_HEAP_BYTES, MAX_HEAP_BYTES);
            let mut isolate = v8::Isolate::new(params);
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            let mut gc_diagnostics = Box::<GcDiagnostics>::default();
            let gc_data = (&mut *gc_diagnostics as *mut GcDiagnostics).cast();
            isolate.add_gc_prologue_callback(gc_prologue, gc_data, v8::GCType::kGCTypeAll);
            isolate.add_gc_epilogue_callback(gc_epilogue, gc_data, v8::GCType::kGCTypeAll);
            let context = {
                v8::scope!(let scope, &mut isolate);
                let context = v8::Context::new(scope, Default::default());
                v8::Global::new(scope, context)
            };
            // SAFETY: each runtime owns its isolate and no handle scope is
            // live. Keeping idle isolates unentered permits package runtimes
            // to be retained and destroyed in arbitrary ownership order.
            unsafe { isolate.exit() };
            let heap = isolate.get_heap_statistics();
            Self {
                context,
                isolate,
                active_deadline: Arc::new(AtomicU64::new(0)),
                diagnostics: EngineDiagnostics {
                    used_heap_bytes: heap.used_heap_size(),
                    peak_used_heap_bytes: heap.used_heap_size(),
                    total_heap_bytes: heap.total_heap_size(),
                    heap_limit_bytes: heap.heap_size_limit(),
                    ..EngineDiagnostics::default()
                },
                gc_diagnostics,
            }
        }

        pub(crate) fn diagnostics(&mut self) -> EngineDiagnostics {
            self.refresh_heap_diagnostics();
            let mut diagnostics = self.diagnostics;
            diagnostics.gc_collections = self.gc_diagnostics.collections;
            diagnostics.gc_micros = self.gc_diagnostics.micros;
            diagnostics
        }

        pub(crate) fn microtask_checkpoint_count(&self) -> u64 {
            self.diagnostics.microtask_checkpoints
        }

        pub(crate) fn platform_tasks_serviced(&self) -> u64 {
            self.diagnostics.platform_tasks_serviced
        }

        pub(crate) fn diagnostic_pump_platform_tasks(&mut self) -> Result<usize, String> {
            let deadline = self.arm_deadline();
            let platform = v8::V8::get_current_platform();
            let context = self.context.clone();
            let started = Instant::now();
            // SAFETY: the exclusive engine borrow owns this isolate and all
            // foreground tasks execute synchronously on its owner thread.
            unsafe { self.isolate.enter() };
            let (count, ready, terminated, caught, microtask_micros) = {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Local::new(scope, context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let scope = std::pin::pin!(v8::TryCatch::new(scope));
                let mut scope = scope.init();
                // Read bootstrap transaction state without the ordinary bridge's
                // microtask checkpoint: a rejected diagnostic must not advance
                // provisional render/event work before returning to its owner.
                let readiness = v8::String::new(&scope, "__nickelPlatformMaintenanceReady")
                    .and_then(|key| context.global(&scope).get(&scope, key.into()))
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
                let probe_ready = || {
                    readiness
                        .and_then(|function| {
                            function.call(&scope, v8::undefined(&scope).into(), &[])
                        })
                        .is_some_and(|value| value.is_true())
                };
                let mut ready = probe_ready();
                let count = pump_ready_task_batch(
                    || v8::Platform::pump_message_loop(&platform, &scope, false),
                    || {
                        if !ready
                            || started.elapsed() >= Duration::from_millis(2)
                            || scope.has_caught()
                            || scope.has_terminated()
                        {
                            return true;
                        }
                        // A foreground task can run JavaScript and leave a new
                        // provisional render/event. Never admit another task
                        // based only on the batch's initial idle state.
                        ready = probe_ready();
                        !ready
                    },
                );
                // The count/time bound may end the batch immediately after a
                // task, without another admission check. Do not report success
                // with the readiness value from before that final task.
                if ready && !scope.has_caught() && !scope.has_terminated() {
                    ready = probe_ready();
                }
                let microtask_micros =
                    if count > 0 && ready && !scope.has_caught() && !scope.has_terminated() {
                        let checkpoint_started = Instant::now();
                        scope.perform_microtask_checkpoint();
                        let elapsed = checkpoint_started
                            .elapsed()
                            .as_micros()
                            .min(u128::from(u64::MAX)) as u64;
                        if !scope.has_caught() && !scope.has_terminated() {
                            ready = readiness
                                .and_then(|function| {
                                    function.call(&scope, v8::undefined(&scope).into(), &[])
                                })
                                .is_some_and(|value| value.is_true());
                        }
                        Some(elapsed)
                    } else {
                        None
                    };
                (
                    count,
                    ready,
                    scope.has_terminated(),
                    scope.has_caught(),
                    microtask_micros,
                )
            };
            // SAFETY: every foreground task and Rust handle scope has ended.
            unsafe { self.isolate.exit() };
            drop(deadline);
            self.diagnostics.platform_tasks_serviced = self
                .diagnostics
                .platform_tasks_serviced
                .saturating_add(count as u64);
            if let Some(micros) = microtask_micros {
                self.diagnostics.microtask_checkpoints =
                    self.diagnostics.microtask_checkpoints.saturating_add(1);
                self.diagnostics.microtask_micros =
                    self.diagnostics.microtask_micros.saturating_add(micros);
            }
            if terminated || self.isolate.is_execution_terminating() {
                self.isolate.cancel_terminate_execution();
                self.diagnostics.deadline_terminations =
                    self.diagnostics.deadline_terminations.saturating_add(1);
                return Err("JavaScript execution deadline exceeded".into());
            }
            if caught {
                return Err("V8 foreground task raised an exception".into());
            }
            if !ready {
                return Err("platform task diagnostic requires an idle valid runtime".into());
            }
            Ok(count)
        }

        pub(crate) fn diagnostic_heap_snapshot(&mut self) -> Result<Vec<u8>, String> {
            const MAX_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;
            let mut bytes = Vec::new();
            let mut overflow = false;
            let mut completed = false;
            // SAFETY: the exclusive engine borrow owns the isolate; there are
            // no outstanding handle scopes. Snapshot callbacks only copy bytes.
            unsafe { self.isolate.enter() };
            self.isolate.take_heap_snapshot(|chunk| {
                if chunk.is_empty() {
                    completed = true;
                    return true;
                }
                if chunk.len() > MAX_SNAPSHOT_BYTES - bytes.len() {
                    overflow = true;
                    return false;
                }
                bytes.extend_from_slice(chunk);
                true
            });
            // SAFETY: snapshot serialization has finished and created no Rust
            // handle scopes. Restore the engine's ordinary unentered state.
            unsafe { self.isolate.exit() };
            self.refresh_heap_diagnostics();
            if overflow {
                Err("diagnostic heap snapshot exceeds 128 MiB".into())
            } else if !completed || bytes.is_empty() {
                Err("diagnostic heap snapshot did not complete".into())
            } else {
                Ok(bytes)
            }
        }

        pub(crate) fn eval(&mut self, source: &str) -> Result<(), String> {
            self.eval_value(source).map(|_| ())
        }

        pub(crate) fn eval_json<T: DeserializeOwned>(&mut self, source: &str) -> Result<T, String> {
            let value = self.eval_value(source)?;
            let bridge_started = Instant::now();
            let context = self.context.clone();
            // SAFETY: calls are synchronous and `JsxRuntime` already requires
            // exclusive access, so this isolate cannot be entered elsewhere.
            unsafe { self.isolate.enter() };
            let result = (|| {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Local::new(scope, context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let value = v8::Local::new(scope, value);
                let text = value
                    .to_string(scope)
                    .ok_or("JavaScript result could not be converted to a string")?
                    .to_rust_string_lossy(scope);
                serde_json::from_str(&text).map_err(|error| error.to_string())
            })();
            // SAFETY: every handle/context scope created above has dropped.
            unsafe { self.isolate.exit() };
            self.diagnostics.bridge_micros = self
                .diagnostics
                .bridge_micros
                .saturating_add(elapsed_micros(bridge_started));
            self.refresh_heap_diagnostics();
            result
        }

        pub(crate) fn call_global_void(
            &mut self,
            name: &str,
            arguments: &[Value],
        ) -> Result<(), String> {
            self.call_global(name, arguments, false).map(|_| ())
        }

        pub(crate) fn call_global_object_fields(
            &mut self,
            name: &str,
            fields: &[(&str, &Value)],
        ) -> Result<(), String> {
            self.call_global_arguments(
                name,
                std::iter::once(NativeArgument::Object(fields)),
                false,
                true,
            )
            .map(|_| ())
        }

        pub(crate) fn call_global_bool(
            &mut self,
            name: &str,
            arguments: &[Value],
        ) -> Result<bool, String> {
            let NativeCallResult::Boolean(value) = self.call_global(name, arguments, false)? else {
                unreachable!("boolean call requested a JSON result")
            };
            Ok(value)
        }

        pub(crate) fn call_global_bool_without_microtasks(
            &mut self,
            name: &str,
            arguments: &[Value],
        ) -> Result<bool, String> {
            let NativeCallResult::Boolean(value) = self.call_global_arguments(
                name,
                arguments.iter().map(NativeArgument::Value),
                false,
                false,
            )?
            else {
                unreachable!("boolean call requested a JSON result")
            };
            Ok(value)
        }

        pub(crate) fn call_global_json<T: DeserializeOwned>(
            &mut self,
            name: &str,
            arguments: &[Value],
        ) -> Result<T, String> {
            let NativeCallResult::JsonText(text) = self.call_global(name, arguments, true)? else {
                unreachable!("JSON call requested a boolean result")
            };
            let started = Instant::now();
            let result = serde_json::from_str(&text).map_err(|error| error.to_string());
            self.diagnostics.bridge_micros = self
                .diagnostics
                .bridge_micros
                .saturating_add(elapsed_micros(started));
            result
        }

        fn eval_value(&mut self, source: &str) -> Result<v8::Global<v8::Value>, String> {
            self.diagnostics.script_compilations =
                self.diagnostics.script_compilations.saturating_add(1);
            let deadline = self.arm_deadline();
            let context = self.context.clone();
            let invocation_started = Instant::now();
            let mut terminated = false;
            // SAFETY: calls are synchronous and exclusively borrow the owner.
            unsafe { self.isolate.enter() };
            let result = (|| {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Local::new(scope, context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let scope = std::pin::pin!(v8::TryCatch::new(scope));
                let mut scope = scope.init();
                let javascript_started = Instant::now();
                let source =
                    v8::String::new(&scope, source).ok_or("JavaScript source is too large")?;
                let script = match v8::Script::compile(&scope, source, None) {
                    Some(script) => script,
                    None => {
                        terminated = scope.has_terminated();
                        let error = scope
                            .message()
                            .map(|message| message.get(&scope).to_rust_string_lossy(&scope))
                            .unwrap_or_else(|| "JavaScript compilation failed".into());
                        return Err(error);
                    }
                };
                let value = match script.run(&scope) {
                    Some(value) => value,
                    None => {
                        terminated = scope.has_terminated();
                        let error = scope
                            .message()
                            .map(|message| message.get(&scope).to_rust_string_lossy(&scope))
                            .unwrap_or_else(|| "JavaScript execution failed".into());
                        return Err(error);
                    }
                };
                let javascript_micros = elapsed_micros(javascript_started);
                let microtask_started = Instant::now();
                scope.perform_microtask_checkpoint();
                terminated = scope.has_terminated();
                let microtask_micros = elapsed_micros(microtask_started);
                Ok((
                    v8::Global::new(&scope, value),
                    javascript_micros,
                    microtask_micros,
                ))
            })();
            // SAFETY: every handle/context scope created above has dropped.
            unsafe { self.isolate.exit() };
            drop(deadline);
            self.diagnostics.invocations = self.diagnostics.invocations.saturating_add(1);
            if let Ok((_, javascript_micros, microtask_micros)) = &result {
                self.diagnostics.javascript_micros = self
                    .diagnostics
                    .javascript_micros
                    .saturating_add(*javascript_micros);
                self.diagnostics.microtask_checkpoints =
                    self.diagnostics.microtask_checkpoints.saturating_add(1);
                self.diagnostics.microtask_micros = self
                    .diagnostics
                    .microtask_micros
                    .saturating_add(*microtask_micros);
                let accounted = javascript_micros.saturating_add(*microtask_micros);
                self.diagnostics.bridge_micros = self
                    .diagnostics
                    .bridge_micros
                    .saturating_add(elapsed_micros(invocation_started).saturating_sub(accounted));
            } else {
                self.diagnostics.failed_invocations =
                    self.diagnostics.failed_invocations.saturating_add(1);
            }
            self.refresh_heap_diagnostics();
            if terminated || self.isolate.is_execution_terminating() {
                self.isolate.cancel_terminate_execution();
                self.diagnostics.deadline_terminations =
                    self.diagnostics.deadline_terminations.saturating_add(1);
                return Err("JavaScript execution deadline exceeded".into());
            }
            result.map(|(value, _, _)| value)
        }

        fn call_global(
            &mut self,
            name: &str,
            arguments: &[Value],
            json_result: bool,
        ) -> Result<NativeCallResult, String> {
            self.call_global_arguments(
                name,
                arguments.iter().map(NativeArgument::Value),
                json_result,
                true,
            )
        }

        fn call_global_arguments<'a>(
            &mut self,
            name: &str,
            arguments: impl IntoIterator<Item = NativeArgument<'a>>,
            json_result: bool,
            checkpoint_microtasks: bool,
        ) -> Result<NativeCallResult, String> {
            let deadline = self.arm_deadline();
            let context = self.context.clone();
            let invocation_started = Instant::now();
            let mut terminated = false;
            let mut bridge_values = 0u64;
            // SAFETY: calls are synchronous and exclusively borrow the owner.
            unsafe { self.isolate.enter() };
            let result = (|| {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Local::new(scope, context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let scope = std::pin::pin!(v8::TryCatch::new(scope));
                let mut scope = scope.init();
                let global = context.global(&scope);
                let key =
                    v8::String::new(&scope, name).ok_or("global function name is too large")?;
                let function = global
                    .get(&scope, key.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
                terminated = scope.has_terminated();
                let function =
                    function.ok_or_else(|| format!("global function {name:?} is not callable"))?;
                let arguments = arguments
                    .into_iter()
                    .map(|argument| match argument {
                        NativeArgument::Value(value) => {
                            json_to_v8(&mut scope, value, &mut bridge_values)
                        }
                        NativeArgument::Object(fields) => {
                            bridge_values = bridge_values.saturating_add(1);
                            object_fields_to_v8(
                                &mut scope,
                                fields.iter().copied(),
                                &mut bridge_values,
                            )
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let receiver = v8::undefined(&scope).into();
                let javascript_started = Instant::now();
                let result = match function.call(&scope, receiver, &arguments) {
                    Some(result) => result,
                    None => {
                        terminated = scope.has_terminated();
                        let error = scope
                            .message()
                            .map(|message| message.get(&scope).to_rust_string_lossy(&scope))
                            .unwrap_or_else(|| "JavaScript function call failed".into());
                        return Err(error);
                    }
                };
                let javascript_micros = elapsed_micros(javascript_started);
                let microtask_started = Instant::now();
                if checkpoint_microtasks {
                    scope.perform_microtask_checkpoint();
                }
                terminated = scope.has_terminated();
                let microtask_micros = if checkpoint_microtasks {
                    elapsed_micros(microtask_started)
                } else {
                    0
                };
                // Ordinary calls match eval_json: execute queued microtasks before
                // converting the result. Do not retain engine values outside
                // this entered isolate or compile a wrapper expression.
                let result = if json_result {
                    let text = result.to_string(&scope);
                    terminated |= scope.has_terminated();
                    NativeCallResult::JsonText(
                        text.ok_or("JavaScript result could not be converted to a string")?
                            .to_rust_string_lossy(&scope),
                    )
                } else {
                    NativeCallResult::Boolean(result.boolean_value(&scope))
                };
                Ok((result, javascript_micros, microtask_micros))
            })();
            // SAFETY: every handle/context scope created above has dropped.
            unsafe { self.isolate.exit() };
            drop(deadline);
            self.diagnostics.invocations = self.diagnostics.invocations.saturating_add(1);
            if let Ok((_, javascript_micros, microtask_micros)) = &result {
                self.diagnostics.javascript_micros = self
                    .diagnostics
                    .javascript_micros
                    .saturating_add(*javascript_micros);
                if checkpoint_microtasks {
                    self.diagnostics.microtask_checkpoints =
                        self.diagnostics.microtask_checkpoints.saturating_add(1);
                }
                self.diagnostics.microtask_micros = self
                    .diagnostics
                    .microtask_micros
                    .saturating_add(*microtask_micros);
                let accounted = javascript_micros.saturating_add(*microtask_micros);
                self.diagnostics.bridge_micros = self
                    .diagnostics
                    .bridge_micros
                    .saturating_add(elapsed_micros(invocation_started).saturating_sub(accounted));
            } else {
                self.diagnostics.failed_invocations =
                    self.diagnostics.failed_invocations.saturating_add(1);
            }
            self.diagnostics.bridge_values =
                self.diagnostics.bridge_values.saturating_add(bridge_values);
            self.refresh_heap_diagnostics();
            if terminated || self.isolate.is_execution_terminating() {
                self.isolate.cancel_terminate_execution();
                self.diagnostics.deadline_terminations =
                    self.diagnostics.deadline_terminations.saturating_add(1);
                return Err("JavaScript execution deadline exceeded".into());
            }
            result.map(|(value, _, _)| value)
        }

        fn refresh_heap_diagnostics(&mut self) {
            let heap = self.isolate.get_heap_statistics();
            self.diagnostics.used_heap_bytes = heap.used_heap_size();
            self.diagnostics.peak_used_heap_bytes = self
                .diagnostics
                .peak_used_heap_bytes
                .max(heap.used_heap_size());
            self.diagnostics.total_heap_bytes = heap.total_heap_size();
            self.diagnostics.heap_limit_bytes = heap.heap_size_limit();
            self.diagnostics.external_memory_bytes = heap.external_memory();
            self.diagnostics.malloced_memory_bytes = heap.malloced_memory();
            self.diagnostics.used_global_handles_bytes = heap.used_global_handles_size();
            self.diagnostics.native_contexts = heap.number_of_native_contexts();
            self.diagnostics.detached_contexts = heap.number_of_detached_contexts();
        }

        fn arm_deadline(&self) -> DeadlineGuard {
            let id = NEXT_DEADLINE_ID.fetch_add(1, Ordering::Relaxed);
            self.active_deadline.store(id, Ordering::Release);
            watchdog()
                .send(Deadline {
                    id,
                    expires: Instant::now() + EXECUTION_DEADLINE,
                    active: Arc::clone(&self.active_deadline),
                    isolate: self.isolate.thread_safe_handle(),
                })
                .expect("Nickel V8 watchdog unexpectedly stopped");
            DeadlineGuard(Arc::clone(&self.active_deadline))
        }
    }

    impl Drop for JavascriptEngine {
        fn drop(&mut self) {
            // SAFETY: idle isolates are deliberately left unentered; the
            // `OwnedIsolate` destructor requires its isolate to be current and
            // performs the matching exit while disposing it.
            unsafe { self.isolate.enter() };
        }
    }

    enum NativeArgument<'a> {
        Value(&'a Value),
        Object(&'a [(&'a str, &'a Value)]),
    }

    fn object_fields_to_v8<'scope, 'value>(
        scope: &mut v8::PinScope<'scope, '_, v8::Context>,
        fields: impl IntoIterator<Item = (&'value str, &'value Value)>,
        bridge_values: &mut u64,
    ) -> Result<v8::Local<'scope, v8::Value>, String> {
        let object = v8::Object::new(scope);
        for (key, value) in fields {
            let key = v8::String::new(scope, key).ok_or("JSON object key is too large for V8")?;
            let value = json_to_v8(scope, value, bridge_values)?;
            // Own data properties must not invoke guest prototype setters.
            if object.create_data_property(scope, key.into(), value) != Some(true) {
                return Err("V8 rejected a JSON object property".into());
            }
        }
        Ok(object.into())
    }

    fn json_to_v8<'scope>(
        scope: &mut v8::PinScope<'scope, '_, v8::Context>,
        value: &Value,
        bridge_values: &mut u64,
    ) -> Result<v8::Local<'scope, v8::Value>, String> {
        *bridge_values = bridge_values.saturating_add(1);
        Ok(match value {
            Value::Null => v8::null(scope).into(),
            Value::Bool(value) => v8::Boolean::new(scope, *value).into(),
            Value::Number(value) => v8::Number::new(
                scope,
                value
                    .as_f64()
                    .ok_or("JSON number cannot be represented by V8")?,
            )
            .into(),
            Value::String(value) => v8::String::new(scope, value)
                .ok_or("JSON string is too large for V8")?
                .into(),
            Value::Array(values) => {
                let values = values
                    .iter()
                    .map(|value| json_to_v8(scope, value, bridge_values))
                    .collect::<Result<Vec<_>, _>>()?;
                v8::Array::new_with_elements(scope, &values).into()
            }
            Value::Object(values) => object_fields_to_v8(
                scope,
                values.iter().map(|(key, value)| (key.as_str(), value)),
                bridge_values,
            )?,
        })
    }
}

pub(crate) use implementation::JavascriptEngine;
