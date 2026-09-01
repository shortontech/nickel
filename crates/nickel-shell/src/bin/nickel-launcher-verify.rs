//! Non-live launcher render and release-budget verifier.

#[cfg(not(debug_assertions))]
use std::time::Instant;

#[cfg(not(debug_assertions))]
use nickel_shell::ShellFixtureProvider;
#[cfg(not(debug_assertions))]
use nickel_ui_testkit::{ActivationVia, FixtureProvider, FixtureRegistry};

#[cfg(not(debug_assertions))]
const OPEN_P95_BUDGET_MS: f64 = 100.0;
#[cfg(not(debug_assertions))]
const FIRST_INPUT_P95_BUDGET_MS: f64 = 16.0;
#[cfg(not(debug_assertions))]
const SAMPLES: usize = 40;
#[cfg(not(debug_assertions))]
const IDLE_RENDER_REQUESTS: usize = 120;

#[cfg(not(debug_assertions))]
fn percentile_95(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[(samples.len() * 95).div_ceil(100).saturating_sub(1)]
}

fn main() {
    #[cfg(debug_assertions)]
    panic!("run this verifier with --release");
    #[cfg(not(debug_assertions))]
    verify_release();
}

#[cfg(not(debug_assertions))]
fn verify_release() {
    let mut registry = FixtureRegistry::new();
    ShellFixtureProvider
        .register(&mut registry)
        .expect("register shell fixtures");
    let entry = registry
        .finish()
        .into_iter()
        .find(|entry| entry.metadata.id == "shell.launcher-dashboard")
        .expect("launcher dashboard fixture");
    let variant = entry.metadata.variants[0];
    let mut open = Vec::with_capacity(SAMPLES);
    let mut input = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let mut session = entry.open_configuration(variant);
        let _ = session.render(1.0);
        open.push(started.elapsed().as_secs_f64() * 1_000.0);

        let started = Instant::now();
        session
            .activate(ActivationVia::Controller)
            .expect("controller activation through production semantics");
        input.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let open_p95_ms = percentile_95(&mut open);
    let first_input_p95_ms = percentile_95(&mut input);

    let idle_session = entry.open_configuration(variant);
    let idle_generation = idle_session.inspect().frame_generation;
    for _ in 0..IDLE_RENDER_REQUESTS {
        let _ = idle_session.render(1.0);
    }
    let idle_generation_delta = idle_session
        .inspect()
        .frame_generation
        .saturating_sub(idle_generation);
    let passed = open_p95_ms <= OPEN_P95_BUDGET_MS
        && first_input_p95_ms <= FIRST_INPUT_P95_BUDGET_MS
        && idle_generation_delta == 0;
    println!(
        "{{\"schema\":\"nickel.launcher-headless-verification.v1\",\"profile\":\"release\",\"native_acceptance\":false,\"samples\":{SAMPLES},\"open_to_headless_frame_p95_ms\":{open_p95_ms:.3},\"open_budget_ms\":{OPEN_P95_BUDGET_MS:.1},\"first_controller_semantic_input_transition_p95_ms\":{first_input_p95_ms:.3},\"first_input_budget_ms\":{FIRST_INPUT_P95_BUDGET_MS:.1},\"idle_render_requests\":{IDLE_RENDER_REQUESTS},\"idle_frame_generation_delta\":{idle_generation_delta},\"passed\":{passed}}}"
    );
    if !passed {
        std::process::exit(1);
    }
}
