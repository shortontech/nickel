//! Fixed-cardinality MCP method measurements. Payloads never enter this collector.
use std::{
    collections::VecDeque,
    fmt::Write,
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub(crate) enum Method {
    RequestLease,
    ListLeases,
    Snapshot,
    DiagnosticAction,
    Status,
    ListWindows,
    FocusWindow,
    Capture,
    CaptureOutput,
    CaptureSurface,
    ListSurfaces,
    WindowAction,
    Pointer,
    Keyboard,
    Semantics,
    SemanticAction,
    SettingsTransaction,
    WorkspaceAction,
    ListOutputs,
    ReadDisplayLayout,
    DisplayLayoutTransaction,
    ListApplications,
    LaunchApplication,
    ReadDesktopEvents,
    EventSubscription,
    ClientConnection,
    SurfaceSemantics,
    SurfaceSemanticAction,
    ReadDeviceSettings,
    ControlDeviceSettings,
    ReadPeripheralControls,
    ControlPeripherals,
    ReadAppearance,
    AppearanceTransaction,
    ReadDefaultAssociation,
    DefaultAssociationTransaction,
    ReadApplicationScale,
    ApplicationScaleTransaction,
    ReadLauncherFavorites,
    LauncherFavoritesTransaction,
    ReadPreferredApplications,
    PreferredApplicationsTransaction,
    NativeSemantics,
    NativeApplicationSemantics,
    NativeSemanticAction,
    ReadWallpaper,
    WallpaperTransaction,
    ReadTerminalPresentation,
    TerminalPresentationTransaction,
    ReadTerminalLaunchPolicy,
    TerminalLaunchPolicyTransaction,
    ReadKeyboardPreference,
    KeyboardPreferenceTransaction,
    ReadFileIcons,
    FileIconsTransaction,
    ReadCodexPreference,
    CodexPreferenceTransaction,
    ReadIdlePreferences,
    IdlePreferencesTransaction,
}

impl Method {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 59] = [
        Self::RequestLease,
        Self::ListLeases,
        Self::Snapshot,
        Self::DiagnosticAction,
        Self::Status,
        Self::ListWindows,
        Self::FocusWindow,
        Self::Capture,
        Self::CaptureOutput,
        Self::CaptureSurface,
        Self::ListSurfaces,
        Self::WindowAction,
        Self::Pointer,
        Self::Keyboard,
        Self::Semantics,
        Self::SemanticAction,
        Self::SettingsTransaction,
        Self::WorkspaceAction,
        Self::ListOutputs,
        Self::ReadDisplayLayout,
        Self::DisplayLayoutTransaction,
        Self::ListApplications,
        Self::LaunchApplication,
        Self::ReadDesktopEvents,
        Self::EventSubscription,
        Self::ClientConnection,
        Self::SurfaceSemantics,
        Self::SurfaceSemanticAction,
        Self::ReadDeviceSettings,
        Self::ControlDeviceSettings,
        Self::ReadPeripheralControls,
        Self::ControlPeripherals,
        Self::ReadAppearance,
        Self::AppearanceTransaction,
        Self::ReadDefaultAssociation,
        Self::DefaultAssociationTransaction,
        Self::ReadApplicationScale,
        Self::ApplicationScaleTransaction,
        Self::ReadLauncherFavorites,
        Self::LauncherFavoritesTransaction,
        Self::ReadPreferredApplications,
        Self::PreferredApplicationsTransaction,
        Self::NativeSemantics,
        Self::NativeApplicationSemantics,
        Self::NativeSemanticAction,
        Self::ReadWallpaper,
        Self::WallpaperTransaction,
        Self::ReadTerminalPresentation,
        Self::TerminalPresentationTransaction,
        Self::ReadTerminalLaunchPolicy,
        Self::TerminalLaunchPolicyTransaction,
        Self::ReadKeyboardPreference,
        Self::KeyboardPreferenceTransaction,
        Self::ReadFileIcons,
        Self::FileIconsTransaction,
        Self::ReadCodexPreference,
        Self::CodexPreferenceTransaction,
        Self::ReadIdlePreferences,
        Self::IdlePreferencesTransaction,
    ];

    fn label(self) -> &'static str {
        METHODS[self as usize]
    }
}

const METHODS: [&str; 59] = [
    "request_control_lease",
    "list_control_leases",
    "diagnostic_snapshot",
    "diagnostic_action",
    "get_control_status",
    "list_windows",
    "focus_window",
    "capture_window",
    "capture_output",
    "capture_surface",
    "list_surfaces",
    "window_action",
    "pointer_action",
    "keyboard_action",
    "inspect_window",
    "semantic_action",
    "shell_behavior_transaction",
    "workspace_action",
    "list_outputs",
    "read_display_layout",
    "display_layout_transaction",
    "list_installed_applications",
    "launch_installed_application",
    "read_desktop_events",
    "event_subscription",
    "client_connection",
    "inspect_surface",
    "surface_semantic_action",
    "read_device_settings",
    "control_device_settings",
    "read_peripheral_controls",
    "control_peripherals",
    "read_appearance",
    "appearance_transaction",
    "read_default_association",
    "default_association_transaction",
    "read_application_scale",
    "application_scale_transaction",
    "read_launcher_favorites",
    "launcher_favorites_transaction",
    "read_preferred_applications",
    "preferred_applications_transaction",
    "inspect_native_window",
    "inspect_native_application",
    "native_semantic_action",
    "read_wallpaper",
    "wallpaper_transaction",
    "read_terminal_presentation",
    "terminal_presentation_transaction",
    "read_terminal_launch_policy",
    "terminal_launch_policy_transaction",
    "read_keyboard_preference",
    "keyboard_preference_transaction",
    "read_file_icons",
    "file_icons_transaction",
    "read_codex_preference",
    "codex_preference_transaction",
    "read_idle_preferences",
    "idle_preferences_transaction",
];
const BOUNDS: [f64; 5] = [0.001, 0.01, 0.1, 1.0, 5.0];

#[cfg(test)]
pub(crate) fn method_labels() -> &'static [&'static str] {
    &METHODS
}

#[derive(Default)]
struct Sample {
    outcomes: [u64; 3],
    buckets: [u64; 5],
    seconds: f64,
    active: u64,
}

struct Samples {
    generation: u64,
    methods: [Sample; METHODS.len()],
    recent_completions: VecDeque<Completion>,
    evicted_completions: u64,
}

impl Default for Samples {
    fn default() -> Self {
        Self {
            generation: 0,
            methods: std::array::from_fn(|_| Sample::default()),
            recent_completions: VecDeque::new(),
            evicted_completions: 0,
        }
    }
}

struct Completion {
    generation: u64,
    collector_uptime_us: u64,
    method: Method,
    outcome: crate::diagnostics::OperationOutcome,
    duration_us: u64,
    authorization: Option<crate::diagnostics::OperationAuthorization>,
}

struct MeasurementContext {
    metrics: Arc<OperationMetrics>,
    authorization: Arc<OnceLock<crate::diagnostics::OperationAuthorization>>,
    lifetime: Arc<RequestLifetime>,
}

/// Required request execution lifetime, shared with worker permits. Successful
/// completion detaches standing operations from the SDK's terminal-response
/// cancellation token; failure or abandonment cancels outstanding work.
#[derive(Default)]
pub(crate) struct RequestLifetime {
    state: std::sync::atomic::AtomicU8,
    transport: OnceLock<tokio_util::sync::CancellationToken>,
}

impl RequestLifetime {
    pub(crate) fn attach(&self, token: tokio_util::sync::CancellationToken) {
        let _ = self.transport.set(token);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        let state = self.state.load(std::sync::atomic::Ordering::Acquire);
        state == 2
            || (state == 0
                && self
                    .transport
                    .get()
                    .is_some_and(|token| token.is_cancelled()))
    }

    fn finish(&self, success: bool) {
        let state = if success && !self.is_cancelled() {
            1
        } else {
            2
        };
        self.state
            .store(state, std::sync::atomic::Ordering::Release);
    }
}

pub(crate) fn current_lifetime(metrics: &Arc<OperationMetrics>) -> Option<Arc<RequestLifetime>> {
    CURRENT_MEASUREMENT
        .try_with(|context| {
            Arc::ptr_eq(metrics, &context.metrics).then(|| context.lifetime.clone())
        })
        .ok()
        .flatten()
}

tokio::task_local! {
    static CURRENT_MEASUREMENT: MeasurementContext;
}

pub(crate) fn current_authorization(
    metrics: &Arc<OperationMetrics>,
) -> Option<Arc<OnceLock<crate::diagnostics::OperationAuthorization>>> {
    CURRENT_MEASUREMENT
        .try_with(|context| {
            Arc::ptr_eq(metrics, &context.metrics).then(|| context.authorization.clone())
        })
        .ok()
        .flatten()
}

pub(crate) struct OperationMetrics {
    started: Instant,
    samples: Mutex<Samples>,
    audit: crate::operation_audit::OperationAudit,
}

impl Default for OperationMetrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            samples: Mutex::new(Samples::default()),
            audit: crate::operation_audit::OperationAudit::default(),
        }
    }
}

impl OperationMetrics {
    pub(crate) fn audit(&self) -> &crate::operation_audit::OperationAudit {
        &self.audit
    }

    pub(crate) async fn measure<T, E>(
        self: &Arc<Self>,
        method: Method,
        operation: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        {
            let mut samples = self.samples.lock().unwrap();
            samples.generation = samples.generation.saturating_add(1);
            samples.methods[method as usize].active += 1;
        }
        let authorization = Arc::new(OnceLock::new());
        let lifetime = Arc::new(RequestLifetime::default());
        let mut measurement = Measurement {
            metrics: self.clone(),
            method,
            started: Instant::now(),
            outcome: 2,
            authorization: authorization.clone(),
            lifetime: lifetime.clone(),
        };
        let result = CURRENT_MEASUREMENT
            .scope(
                MeasurementContext {
                    metrics: self.clone(),
                    authorization,
                    lifetime,
                },
                operation,
            )
            .await;
        measurement.outcome = usize::from(result.is_err());
        result
    }

    pub(crate) fn snapshot(&self) -> Option<crate::diagnostics::OperationMetricsSnapshot> {
        use crate::diagnostics::{DurationBucket, MethodMetrics, OperationMetricsSnapshot};
        let samples = self.samples.try_lock().ok()?;
        Some(OperationMetricsSnapshot {
            generation: samples.generation,
            collector_uptime_us: self.started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            recent_completions: samples
                .recent_completions
                .iter()
                .map(|completion| crate::diagnostics::OperationCompletion {
                    generation: completion.generation,
                    collector_uptime_us: completion.collector_uptime_us,
                    method: completion.method.label().to_owned(),
                    outcome: completion.outcome,
                    duration_us: completion.duration_us,
                    authorization: completion.authorization,
                })
                .collect(),
            evicted_completions: samples.evicted_completions,
            methods: METHODS
                .iter()
                .zip(&samples.methods)
                .map(|(method, sample)| MethodMetrics {
                    method: (*method).to_owned(),
                    success: sample.outcomes[0],
                    error: sample.outcomes[1],
                    cancelled: sample.outcomes[2],
                    in_flight: sample.active,
                    duration_seconds_sum: sample.seconds,
                    duration_buckets: BOUNDS
                        .iter()
                        .zip(sample.buckets)
                        .map(|(bound, count)| DurationBucket {
                            upper_bound_seconds: *bound,
                            cumulative_count: count,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    pub(crate) fn exposition(&self) -> String {
        let state = self.samples.lock().unwrap();
        let samples = &state.methods;
        let mut text = String::from(
            "# HELP nickel_mcp_requests_total Typed MCP method completions; cancelled means the method future was dropped.\n# TYPE nickel_mcp_requests_total counter\n",
        );
        for (method, sample) in METHODS.iter().zip(samples.iter()) {
            for (outcome, count) in ["success", "error", "cancelled"]
                .iter()
                .zip(sample.outcomes)
            {
                let _ = writeln!(
                    text,
                    "nickel_mcp_requests_total{{method=\"{method}\",outcome=\"{outcome}\"}} {count}"
                );
            }
        }
        text.push_str("# HELP nickel_mcp_request_duration_seconds Typed method latency through its result, excluding response transmission.\n# TYPE nickel_mcp_request_duration_seconds histogram\n");
        for (method, sample) in METHODS.iter().zip(samples.iter()) {
            for (bound, count) in BOUNDS.iter().zip(sample.buckets) {
                let _ = writeln!(
                    text,
                    "nickel_mcp_request_duration_seconds_bucket{{method=\"{method}\",le=\"{bound}\"}} {count}"
                );
            }
            let count = sample
                .outcomes
                .iter()
                .fold(0u64, |total, count| total.saturating_add(*count));
            let _ = writeln!(
                text,
                "nickel_mcp_request_duration_seconds_bucket{{method=\"{method}\",le=\"+Inf\"}} {count}"
            );
            let _ = writeln!(
                text,
                "nickel_mcp_request_duration_seconds_count{{method=\"{method}\"}} {count}"
            );
            let _ = writeln!(
                text,
                "nickel_mcp_request_duration_seconds_sum{{method=\"{method}\"}} {}",
                sample.seconds
            );
        }
        text.push_str("# HELP nickel_mcp_tool_calls_in_flight Currently executing typed MCP method futures.\n# TYPE nickel_mcp_tool_calls_in_flight gauge\n");
        let active: u64 = samples.iter().map(|sample| sample.active).sum();
        let _ = writeln!(text, "nickel_mcp_tool_calls_in_flight {active}");
        text
    }
}

struct Measurement {
    metrics: Arc<OperationMetrics>,
    method: Method,
    started: Instant,
    outcome: usize,
    authorization: Arc<OnceLock<crate::diagnostics::OperationAuthorization>>,
    lifetime: Arc<RequestLifetime>,
}
impl Drop for Measurement {
    fn drop(&mut self) {
        self.lifetime.finish(self.outcome == 0);
        let mut samples = self.metrics.samples.lock().unwrap();
        samples.generation = samples.generation.saturating_add(1);
        let sample = &mut samples.methods[self.method as usize];
        sample.active -= 1;
        sample.outcomes[self.outcome] = sample.outcomes[self.outcome].saturating_add(1);
        let duration = self.started.elapsed();
        let elapsed = duration.as_secs_f64();
        sample.seconds += elapsed;
        for (bound, count) in BOUNDS.iter().zip(&mut sample.buckets) {
            if elapsed <= *bound {
                *count = count.saturating_add(1);
            }
        }
        use crate::diagnostics::{MAX_RECENT_OPERATION_COMPLETIONS, OperationOutcome};
        if samples.recent_completions.len() == MAX_RECENT_OPERATION_COMPLETIONS {
            samples.recent_completions.pop_front();
            samples.evicted_completions = samples.evicted_completions.saturating_add(1);
        }
        let outcome = match self.outcome {
            0 => OperationOutcome::Success,
            1 => OperationOutcome::Error,
            _ => OperationOutcome::Cancelled,
        };
        let authorization = self.authorization.get().copied();
        let completion = Completion {
            generation: samples.generation,
            collector_uptime_us: self
                .metrics
                .started
                .elapsed()
                .as_micros()
                .min(u64::MAX as u128) as u64,
            method: self.method,
            outcome,
            duration_us: duration.as_micros().min(u64::MAX as u128) as u64,
            authorization,
        };
        samples.recent_completions.push_back(completion);
        drop(samples);
        self.metrics.audit.record(
            self.method.label(),
            authorization.map(|authorization| authorization.lease_id),
            duration,
            match outcome {
                OperationOutcome::Success => crate::operation_audit::Outcome::Success,
                OperationOutcome::Error => crate::operation_audit::Outcome::Error,
                OperationOutcome::Cancelled => crate::operation_audit::Outcome::Cancelled,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correlation(id: u64) -> crate::diagnostics::OperationAuthorization {
        crate::diagnostics::OperationAuthorization {
            client_id: Some(id),
            lease_id: id,
            operation_id: id,
            lease_operation_generation: id,
        }
    }

    #[test]
    fn every_fixed_method_and_outcome_is_bounded_and_payload_free() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let metrics = Arc::new(OperationMetrics::default());
        runtime.block_on(async {
            for method in Method::ALL {
                metrics
                    .measure(method, async {
                        Ok::<_, &'static str>("private-result-canary")
                    })
                    .await
                    .unwrap();
                assert!(
                    metrics
                        .measure(method, async {
                            Err::<(), _>("private-resource-title-path-credential-keystroke-canary")
                        })
                        .await
                        .is_err()
                );

                let cancelled_metrics = metrics.clone();
                let task = tokio::spawn(async move {
                    cancelled_metrics
                        .measure(method, std::future::pending::<Result<(), ()>>())
                        .await
                });
                while metrics.snapshot().unwrap().methods[method as usize].in_flight == 0 {
                    tokio::task::yield_now().await;
                }
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
        });

        let text = metrics.exposition();
        assert!(
            text.len() < 64 * 1024,
            "metrics response grew to {} bytes",
            text.len()
        );
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("nickel_mcp_requests_total{"))
                .count(),
            METHODS.len() * 3
        );
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("nickel_mcp_request_duration_seconds_bucket{"))
                .count(),
            METHODS.len() * (BOUNDS.len() + 1)
        );
        for method in Method::ALL {
            let label = method.label();
            assert!(
                label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            );
            for outcome in ["success", "error", "cancelled"] {
                assert!(text.contains(&format!(
                    "nickel_mcp_requests_total{{method=\"{label}\",outcome=\"{outcome}\"}} 1"
                )));
            }
        }
        assert_eq!(
            METHODS
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            METHODS.len()
        );
        for secret in [
            "private-result-canary",
            "private-resource-title-path-credential-keystroke-canary",
            "resource",
            "title",
            "path",
            "credential",
            "keystroke",
        ] {
            assert!(!text.contains(secret), "metrics retained {secret}");
        }
    }

    #[test]
    fn interleaved_method_futures_keep_distinct_authorization_cells() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let metrics = Arc::new(OperationMetrics::default());
        runtime.block_on(async {
            let run = |method, id| {
                let request_metrics = metrics.clone();
                metrics.measure(method, async move {
                    let cell = current_authorization(&request_metrics).unwrap();
                    tokio::task::yield_now().await;
                    assert!(Arc::ptr_eq(
                        &cell,
                        &current_authorization(&request_metrics).unwrap()
                    ));
                    cell.set(correlation(id)).unwrap();
                    Ok::<_, ()>(())
                })
            };
            let (first, second) = tokio::join!(run(Method::Pointer, 11), run(Method::Keyboard, 22));
            first.unwrap();
            second.unwrap();
        });
        let snapshot = metrics.snapshot().unwrap();
        assert_eq!(snapshot.recent_completions.len(), 2);
        for (method, id) in [("pointer_action", 11), ("keyboard_action", 22)] {
            let record = snapshot
                .recent_completions
                .iter()
                .find(|record| record.method == method)
                .unwrap();
            assert_eq!(record.authorization, Some(correlation(id)));
        }
        let (audit, evicted) = metrics.audit().snapshot().unwrap();
        assert_eq!(evicted, 0);
        assert_eq!(audit.len(), 2);
        for (method, lease_id) in [("pointer_action", 11), ("keyboard_action", 22)] {
            let event = audit.iter().find(|event| event.method == method).unwrap();
            assert_eq!(event.matched_lease_id, Some(lease_id));
            assert_eq!(event.outcome, crate::operation_audit::Outcome::Success);
        }
        assert!(current_authorization(&metrics).is_none());
    }

    #[test]
    fn cancellation_freezes_correlation_and_late_worker_updates_cannot_rewrite_history() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for before_cancel in [false, true] {
            let metrics = Arc::new(OperationMetrics::default());
            runtime.block_on(async {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let worker_metrics = metrics.clone();
                let task = tokio::spawn(async move {
                    worker_metrics
                        .measure(Method::Capture, async {
                            let cell = current_authorization(&worker_metrics).unwrap();
                            if before_cancel {
                                cell.set(correlation(33)).unwrap();
                            }
                            tx.send(cell).unwrap();
                            std::future::pending::<Result<(), ()>>().await
                        })
                        .await
                });
                let retained = rx.await.unwrap();
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                let expected = before_cancel.then(|| correlation(33));
                let before = metrics.snapshot().unwrap();
                assert_eq!(before.recent_completions.len(), 1);
                assert_eq!(before.recent_completions[0].authorization, expected);
                assert!(matches!(
                    before.recent_completions[0].outcome,
                    crate::diagnostics::OperationOutcome::Cancelled
                ));
                let _ = retained.set(correlation(44));
                let after = metrics.snapshot().unwrap();
                assert_eq!(after.generation, before.generation);
                assert_eq!(after.recent_completions[0].authorization, expected);
                assert!(after.methods.iter().all(|method| method.in_flight == 0));
            });
        }
    }

    #[test]
    fn fully_correlated_history_retains_its_record_and_serialization_bounds() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let metrics = Arc::new(OperationMetrics::default());
        let capacity = crate::diagnostics::MAX_RECENT_OPERATION_COMPLETIONS;
        runtime.block_on(async {
            for _ in 0..capacity + 17 {
                metrics
                    .measure(Method::LaunchApplication, async {
                        current_authorization(&metrics)
                            .unwrap()
                            .set(correlation(u64::MAX))
                            .unwrap();
                        Ok::<_, ()>(())
                    })
                    .await
                    .unwrap();
            }
        });
        let snapshot = metrics.snapshot().unwrap();
        assert_eq!(snapshot.recent_completions.len(), capacity);
        assert_eq!(snapshot.evicted_completions, 17);
        assert!(
            snapshot
                .recent_completions
                .iter()
                .all(|record| record.authorization == Some(correlation(u64::MAX)))
        );
        assert!(
            snapshot
                .recent_completions
                .windows(2)
                .all(|pair| pair[0].generation < pair[1].generation)
        );
        assert!(serde_json::to_vec(&snapshot).unwrap().len() < 65_536);
    }

    #[test]
    fn measurement_context_is_scoped_to_its_collector_and_does_not_block_snapshots() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let metrics = Arc::new(OperationMetrics::default());
        let other = Arc::new(OperationMetrics::default());
        assert!(current_authorization(&metrics).is_none());
        runtime
            .block_on(metrics.measure(Method::Snapshot, async {
                assert!(current_authorization(&metrics).is_some());
                assert!(current_authorization(&other).is_none());
                let held = metrics.samples.lock().unwrap();
                let (tx, rx) = std::sync::mpsc::channel();
                let reader = metrics.clone();
                let thread =
                    std::thread::spawn(move || tx.send(reader.snapshot().is_none()).unwrap());
                let result = rx.recv_timeout(std::time::Duration::from_secs(1));
                drop(held);
                thread.join().unwrap();
                assert!(result.unwrap());
                Ok::<_, ()>(())
            }))
            .unwrap();
        assert!(current_authorization(&metrics).is_none());
        assert!(metrics.snapshot().is_some());
    }

    #[test]
    fn success_error_and_cancelled_calls_remain_bounded_and_payload_free() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let metrics = Arc::new(OperationMetrics::default());
        runtime.block_on(async {
            assert!(
                metrics
                    .measure(Method::Keyboard, async { Ok::<_, String>("typed-secret") })
                    .await
                    .is_ok()
            );
            assert!(
                metrics
                    .measure(Method::Capture, async {
                        Err::<(), _>("credential-and-path")
                    })
                    .await
                    .is_err()
            );
            let pending = metrics.measure(Method::Snapshot, async {
                let snapshot = metrics.snapshot().unwrap();
                let method = snapshot
                    .methods
                    .iter()
                    .find(|method| method.method == "diagnostic_snapshot")
                    .unwrap();
                assert_eq!(method.in_flight, 1);
                assert_eq!(method.cancelled, 0);
                assert_eq!(snapshot.generation, 5);
                std::future::pending::<Result<(), ()>>().await
            });
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(1), pending)
                    .await
                    .is_err()
            );
        });
        let text = metrics.exposition();
        assert!(text.contains("method=\"keyboard_action\",outcome=\"success\"} 1"));
        assert!(text.contains("method=\"capture_window\",outcome=\"error\"} 1"));
        assert!(text.contains("method=\"diagnostic_snapshot\",outcome=\"cancelled\"} 1"));
        assert!(text.contains("nickel_mcp_tool_calls_in_flight 0"));
        assert!(!text.contains("typed-secret") && !text.contains("credential-and-path"));
        let snapshot = metrics.snapshot().unwrap();
        assert_eq!(snapshot.generation, 6);
        assert_eq!(snapshot.evicted_completions, 0);
        assert_eq!(snapshot.recent_completions.len(), 3);
        for (completion, (generation, method, outcome)) in snapshot.recent_completions.iter().zip([
            (
                2,
                "keyboard_action",
                crate::diagnostics::OperationOutcome::Success,
            ),
            (
                4,
                "capture_window",
                crate::diagnostics::OperationOutcome::Error,
            ),
            (
                6,
                "diagnostic_snapshot",
                crate::diagnostics::OperationOutcome::Cancelled,
            ),
        ]) {
            assert_eq!(completion.generation, generation);
            assert_eq!(completion.method, method);
            assert_eq!(completion.outcome, outcome);
            assert!(completion.collector_uptime_us <= snapshot.collector_uptime_us);
            assert!(completion.duration_us <= completion.collector_uptime_us);
        }
        assert_eq!(snapshot.methods.len(), METHODS.len());
        assert!(snapshot.methods.iter().all(|method| method.in_flight == 0));
        assert!(
            snapshot
                .methods
                .iter()
                .all(|method| method.duration_buckets.len() == BOUNDS.len())
        );
        assert_eq!(
            snapshot
                .methods
                .iter()
                .map(|method| method.success)
                .sum::<u64>(),
            1
        );
        assert_eq!(
            snapshot
                .methods
                .iter()
                .map(|method| method.error)
                .sum::<u64>(),
            1
        );
        assert_eq!(
            snapshot
                .methods
                .iter()
                .map(|method| method.cancelled)
                .sum::<u64>(),
            1
        );
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(
            !serialized.contains("typed-secret") && !serialized.contains("credential-and-path")
        );
        assert!(
            !serialized.contains("operation_audit"),
            "the trusted local audit must not enter MCP diagnostics"
        );
        let (audit, evicted) = metrics.audit().snapshot().unwrap();
        assert_eq!(evicted, 0);
        assert_eq!(audit.len(), 3);
        for (event, (method, outcome)) in audit.iter().zip([
            ("keyboard_action", crate::operation_audit::Outcome::Success),
            ("capture_window", crate::operation_audit::Outcome::Error),
            (
                "diagnostic_snapshot",
                crate::operation_audit::Outcome::Cancelled,
            ),
        ]) {
            assert_eq!(event.method, method);
            assert_eq!(event.outcome, outcome);
            assert_eq!(event.matched_lease_id, None);
        }
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("nickel_mcp_requests_total{"))
                .count(),
            super::METHODS.len() * 3
        );
        let samples = metrics.samples.lock().unwrap();
        for sample in samples.methods.iter() {
            assert!(sample.buckets.windows(2).all(|pair| pair[0] <= pair[1]));
            assert!(sample.buckets[4] <= sample.outcomes.iter().sum());
        }
    }

    #[test]
    fn completion_history_evicts_oldest_without_losing_aggregate_counts() {
        use crate::diagnostics::MAX_RECENT_OPERATION_COMPLETIONS;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let metrics = Arc::new(OperationMetrics::default());
        let count = MAX_RECENT_OPERATION_COMPLETIONS * 3;
        runtime.block_on(async {
            for _ in 0..count {
                metrics
                    .measure(Method::Status, async { Ok::<(), ()>(()) })
                    .await
                    .unwrap();
            }
        });
        let snapshot = metrics.snapshot().unwrap();
        assert_eq!(snapshot.generation, (count * 2) as u64);
        assert_eq!(
            snapshot.evicted_completions,
            (count - MAX_RECENT_OPERATION_COMPLETIONS) as u64
        );
        assert_eq!(
            snapshot.recent_completions.len(),
            MAX_RECENT_OPERATION_COMPLETIONS
        );
        assert_eq!(
            snapshot.recent_completions.first().unwrap().generation,
            ((count - MAX_RECENT_OPERATION_COMPLETIONS + 1) * 2) as u64
        );
        assert_eq!(
            snapshot.recent_completions.last().unwrap().generation,
            snapshot.generation
        );
        assert!(
            snapshot
                .recent_completions
                .windows(2)
                .all(|pair| pair[0].generation < pair[1].generation
                    && pair[0].collector_uptime_us <= pair[1].collector_uptime_us)
        );
        assert_eq!(
            snapshot.methods[Method::Status as usize].success,
            count as u64
        );
        // The typed projection has a finite serialized bound even at maximum retention.
        assert!(serde_json::to_vec(&snapshot).unwrap().len() < 64 * 1024);
    }
}
