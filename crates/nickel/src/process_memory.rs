use nickel_session_protocol::{
    CacheDiagnostics, MemoryTrimDiagnostics, MemoryTrimSnapshot, ProcessMemoryDiagnostics,
};
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const AUTOMATIC_TRIM_FREE_THRESHOLD_BYTES: u64 = 24 * 1024 * 1024;
const AUTOMATIC_TRIM_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AutomaticTrimDiagnostics {
    attempts: u64,
    releases: u64,
    reclaimed_rss_bytes: u64,
    last_duration_us: u64,
    skipped_threshold: u64,
    skipped_cooldown: u64,
}

#[derive(Debug, Default)]
struct AutomaticTrimState {
    last_attempt: Option<Instant>,
    diagnostics: AutomaticTrimDiagnostics,
}

static AUTOMATIC_TRIM_STATE: OnceLock<Mutex<AutomaticTrimState>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProcessMemorySnapshot {
    rss_bytes: Option<u64>,
    pss_bytes: Option<u64>,
    private_bytes: Option<u64>,
    anonymous_bytes: Option<u64>,
    swap_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AllocatorMemorySnapshot {
    arena_bytes: Option<u64>,
    mmap_bytes: Option<u64>,
    live_bytes: Option<u64>,
    free_bytes: Option<u64>,
    releasable_bytes: Option<u64>,
    free_chunks: Option<u64>,
    mmap_regions: Option<u64>,
}

pub(crate) fn diagnostics(cache: &CacheDiagnostics) -> ProcessMemoryDiagnostics {
    let process = process_memory_snapshot();
    let allocator = allocator_memory_snapshot();
    let automatic_trim = automatic_trim_diagnostics();
    let internal_ui_cpu_bytes = [
        cache.internal_ui_software_frame_bytes,
        cache.internal_ui_fallback_raster_bytes,
        cache.internal_ui_text_scratch_bytes,
        cache.internal_ui_text_private_cache_bytes,
        cache.internal_ui_image_cache_bytes,
        cache.internal_ui_text_cache_bytes,
    ]
    .into_iter()
    .fold(0_u64, u64::saturating_add);
    let shell_image_cpu_bytes = cache.internal_shell_wallpaper_bytes;
    let preview_cpu_bytes = cache
        .preview_bytes
        .saturating_add(cache.native_preview_work.pending_readback_bytes);
    let window_metadata_cpu_bytes = cache.metadata_live_snapshot_bytes;
    let decoration_cpu_bytes = [
        cache.titlebar_live_bytes,
        cache.titlebar_renderer_bytes.unwrap_or_default(),
        cache.recovery_live_bytes,
        cache.recovery_renderer_bytes.unwrap_or_default(),
        cache.identify_live_bytes,
        cache.identify_renderer_bytes.unwrap_or_default(),
    ]
    .into_iter()
    .fold(0_u64, u64::saturating_add);
    let accounted_cpu_bytes = [
        internal_ui_cpu_bytes,
        shell_image_cpu_bytes,
        preview_cpu_bytes,
        window_metadata_cpu_bytes,
        decoration_cpu_bytes,
    ]
    .into_iter()
    .fold(0_u64, u64::saturating_add);

    ProcessMemoryDiagnostics {
        process_rss_bytes: process.rss_bytes,
        process_pss_bytes: process.pss_bytes,
        process_private_bytes: process.private_bytes,
        process_anonymous_bytes: process.anonymous_bytes,
        process_swap_bytes: process.swap_bytes,
        allocator_arena_bytes: allocator.arena_bytes,
        allocator_mmap_bytes: allocator.mmap_bytes,
        allocator_live_bytes: allocator.live_bytes,
        allocator_free_bytes: allocator.free_bytes,
        allocator_releasable_bytes: allocator.releasable_bytes,
        allocator_free_chunks: allocator.free_chunks,
        allocator_mmap_regions: allocator.mmap_regions,
        automatic_trim_attempts: automatic_trim.attempts,
        automatic_trim_releases: automatic_trim.releases,
        automatic_trim_reclaimed_rss_bytes: automatic_trim.reclaimed_rss_bytes,
        automatic_trim_last_duration_us: automatic_trim.last_duration_us,
        automatic_trim_skipped_threshold: automatic_trim.skipped_threshold,
        automatic_trim_skipped_cooldown: automatic_trim.skipped_cooldown,
        internal_ui_cpu_bytes,
        shell_image_cpu_bytes,
        preview_cpu_bytes,
        window_metadata_cpu_bytes,
        decoration_cpu_bytes,
        accounted_cpu_bytes,
        accounted_gpu_payload_bytes: cache.native_preview_work.pending_texture_bytes,
        unaccounted_private_bytes: process
            .private_bytes
            .map(|private| private.saturating_sub(accounted_cpu_bytes)),
    }
}

pub(crate) fn maybe_trim() {
    let allocator = allocator_memory_snapshot();
    let retained_free_bytes = allocator.free_bytes.unwrap_or_default();
    let now = Instant::now();
    let mut state = automatic_trim_state();
    if retained_free_bytes < AUTOMATIC_TRIM_FREE_THRESHOLD_BYTES {
        state.diagnostics.skipped_threshold = state.diagnostics.skipped_threshold.saturating_add(1);
        return;
    }
    if state
        .last_attempt
        .is_some_and(|last| now.saturating_duration_since(last) < AUTOMATIC_TRIM_COOLDOWN)
    {
        state.diagnostics.skipped_cooldown = state.diagnostics.skipped_cooldown.saturating_add(1);
        return;
    }
    state.last_attempt = Some(now);
    state.diagnostics.attempts = state.diagnostics.attempts.saturating_add(1);
    drop(state);

    let before_rss = process_memory_snapshot().rss_bytes;
    let started = Instant::now();
    #[cfg(target_env = "gnu")]
    // SAFETY: `malloc_trim` preserves every live allocation. Automatic calls
    // are delayed until after a shell teardown and bounded by threshold/cooldown.
    let released = unsafe { libc::malloc_trim(0) } != 0;
    #[cfg(not(target_env = "gnu"))]
    let released = false;
    let elapsed = started.elapsed();
    let after_rss = process_memory_snapshot().rss_bytes;

    let mut state = automatic_trim_state();
    state.diagnostics.releases = state
        .diagnostics
        .releases
        .saturating_add(u64::from(released));
    state.diagnostics.reclaimed_rss_bytes =
        state
            .diagnostics
            .reclaimed_rss_bytes
            .saturating_add(match (before_rss, after_rss) {
                (Some(before), Some(after)) => before.saturating_sub(after),
                _ => 0,
            });
    state.diagnostics.last_duration_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
}

fn automatic_trim_state() -> std::sync::MutexGuard<'static, AutomaticTrimState> {
    AUTOMATIC_TRIM_STATE
        .get_or_init(|| Mutex::new(AutomaticTrimState::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn automatic_trim_diagnostics() -> AutomaticTrimDiagnostics {
    automatic_trim_state().diagnostics
}

pub(crate) fn trim_diagnostics() -> MemoryTrimDiagnostics {
    let before = trim_snapshot();
    #[cfg(target_env = "gnu")]
    let (supported, allocator_reported_release) = {
        // SAFETY: `malloc_trim` takes no pointers and preserves every live
        // allocation. This process-wide mutation is exposed only through the
        // explicit test-control command.
        (true, unsafe { libc::malloc_trim(0) } != 0)
    };
    #[cfg(not(target_env = "gnu"))]
    let (supported, allocator_reported_release) = (false, false);
    let after = trim_snapshot();
    MemoryTrimDiagnostics {
        supported,
        allocator_reported_release,
        before,
        after,
    }
}

pub(crate) fn trim_snapshot() -> MemoryTrimSnapshot {
    let process = process_memory_snapshot();
    let allocator = allocator_memory_snapshot();
    MemoryTrimSnapshot {
        process_rss_bytes: process.rss_bytes,
        process_private_bytes: process.private_bytes,
        process_anonymous_bytes: process.anonymous_bytes,
        allocator_arena_bytes: allocator.arena_bytes,
        allocator_mmap_bytes: allocator.mmap_bytes,
        allocator_live_bytes: allocator.live_bytes,
        allocator_free_bytes: allocator.free_bytes,
        allocator_releasable_bytes: allocator.releasable_bytes,
    }
}

#[cfg(target_env = "gnu")]
fn allocator_memory_snapshot() -> AllocatorMemorySnapshot {
    // SAFETY: `mallinfo2` takes no pointers, mutates no caller-owned state, and
    // returns a by-value snapshot maintained by the process's glibc allocator.
    let info = unsafe { libc::mallinfo2() };
    AllocatorMemorySnapshot {
        arena_bytes: u64::try_from(info.arena).ok(),
        mmap_bytes: u64::try_from(info.hblkhd).ok(),
        live_bytes: u64::try_from(info.uordblks).ok(),
        free_bytes: u64::try_from(info.fordblks).ok(),
        releasable_bytes: u64::try_from(info.keepcost).ok(),
        free_chunks: u64::try_from(info.ordblks).ok(),
        mmap_regions: u64::try_from(info.hblks).ok(),
    }
}

#[cfg(not(target_env = "gnu"))]
fn allocator_memory_snapshot() -> AllocatorMemorySnapshot {
    AllocatorMemorySnapshot::default()
}

fn process_memory_snapshot() -> ProcessMemorySnapshot {
    std::fs::read_to_string("/proc/self/smaps_rollup")
        .ok()
        .map(|contents| parse_smaps_rollup(&contents))
        .or_else(|| {
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .map(|contents| ProcessMemorySnapshot {
                    rss_bytes: parse_kib_field(&contents, "VmRSS:")
                        .and_then(|value| value.checked_mul(1024)),
                    ..Default::default()
                })
        })
        .unwrap_or_default()
}

fn parse_smaps_rollup(contents: &str) -> ProcessMemorySnapshot {
    let kib = |name| parse_kib_field(contents, name).and_then(|value| value.checked_mul(1024));
    let private_bytes = match (kib("Private_Clean:"), kib("Private_Dirty:")) {
        (Some(clean), Some(dirty)) => Some(clean.saturating_add(dirty)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    ProcessMemorySnapshot {
        rss_bytes: kib("Rss:"),
        pss_bytes: kib("Pss:"),
        private_bytes,
        anonymous_bytes: kib("Anonymous:"),
        swap_bytes: kib("Swap:"),
    }
}

fn parse_kib_field(contents: &str, name: &str) -> Option<u64> {
    contents.lines().find_map(|line| {
        let value = line.strip_prefix(name)?.trim();
        value.strip_suffix("kB")?.trim().parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_rollup_without_conflating_pss_or_private_memory() {
        let snapshot = parse_smaps_rollup(
            "Rss:                400 kB\n\
             Pss:                300 kB\n\
             Private_Clean:       20 kB\n\
             Private_Dirty:      180 kB\n\
             Anonymous:          160 kB\n\
             Swap:                 7 kB\n",
        );
        assert_eq!(snapshot.rss_bytes, Some(400 * 1024));
        assert_eq!(snapshot.pss_bytes, Some(300 * 1024));
        assert_eq!(snapshot.private_bytes, Some(200 * 1024));
        assert_eq!(snapshot.anonymous_bytes, Some(160 * 1024));
        assert_eq!(snapshot.swap_bytes, Some(7 * 1024));
    }

    #[test]
    fn absent_and_malformed_linux_fields_remain_unknown() {
        let snapshot = parse_smaps_rollup("Rss: nope kB\nPrivate_Dirty: 3 MB\n");
        assert_eq!(snapshot, ProcessMemorySnapshot::default());
    }

    #[test]
    fn aggregates_only_current_owned_bytes() {
        let cache = CacheDiagnostics {
            internal_ui_software_frame_bytes: 1,
            internal_ui_fallback_raster_bytes: 2,
            internal_ui_text_scratch_bytes: 3,
            internal_ui_text_private_cache_bytes: 4,
            internal_ui_image_cache_bytes: 5,
            internal_ui_text_cache_bytes: 6,
            internal_shell_wallpaper_bytes: 7,
            preview_bytes: 8,
            preview_peak_bytes: 80,
            native_preview_work: nickel_session_protocol::NativePreviewWorkDiagnostics {
                pending_texture_bytes: 9,
                pending_readback_bytes: 10,
                peak_pending_payload_bytes: 90,
                ..Default::default()
            },
            metadata_live_snapshot_bytes: 11,
            metadata_peak_snapshot_bytes: 110,
            titlebar_live_bytes: 12,
            titlebar_peak_bytes: 120,
            titlebar_renderer_bytes: Some(13),
            recovery_live_bytes: 14,
            recovery_renderer_bytes: Some(15),
            identify_live_bytes: 16,
            identify_renderer_bytes: Some(17),
            ..Default::default()
        };

        let memory = diagnostics(&cache);
        assert_eq!(memory.internal_ui_cpu_bytes, 21);
        assert_eq!(memory.shell_image_cpu_bytes, 7);
        assert_eq!(memory.preview_cpu_bytes, 18);
        assert_eq!(memory.window_metadata_cpu_bytes, 11);
        assert_eq!(memory.decoration_cpu_bytes, 87);
        assert_eq!(memory.accounted_cpu_bytes, 144);
        assert_eq!(memory.accounted_gpu_payload_bytes, 9);
    }
}
