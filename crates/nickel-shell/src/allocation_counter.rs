use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

static ALLOCATION_OPERATIONS: AtomicU64 = AtomicU64::new(0);
static TRACKING_INSTALLED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static LIVE_REQUESTED_BYTES: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
thread_local! {
    static THREAD_ALLOCATION_OPERATIONS: Cell<u64> = const { Cell::new(0) };
    #[cfg(test)]
    static THREAD_REQUESTED_BYTES: Cell<u64> = const { Cell::new(0) };
    #[cfg(test)]
    static THREAD_LIVE_REQUESTED_DELTA: Cell<i64> = const { Cell::new(0) };
}

#[cfg(test)]
fn record_live_change(delta: i64) {
    LIVE_REQUESTED_BYTES.fetch_add(delta, Ordering::Relaxed);
    THREAD_LIVE_REQUESTED_DELTA.with(|bytes| bytes.set(bytes.get() + delta));
}

fn record_allocation(_requested_bytes: usize) {
    TRACKING_INSTALLED.store(true, Ordering::Relaxed);
    ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
    THREAD_ALLOCATION_OPERATIONS.with(|operations| operations.set(operations.get() + 1));
    #[cfg(test)]
    THREAD_REQUESTED_BYTES
        .with(|bytes| bytes.set(bytes.get().saturating_add(_requested_bytes as u64)));
}

/// System allocator wrapper used by the native shell's process-wide telemetry.
///
/// The counter measures allocation operations rather than bytes. Sampling it
/// around a frame is conservative: allocations from any shell thread during
/// that interval are charged to the frame.
pub struct CountingSystemAllocator;

// Library workloads use the same observational allocator as the executable.
// This is test-only: downstream binaries still choose their own global allocator.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: CountingSystemAllocator = CountingSystemAllocator;

/// Thread-local deltas exclude unrelated parallel tests and backend workers.
/// They count allocation/reallocation calls, not retained bytes or successful allocations.
#[cfg(test)]
pub(crate) fn thread_allocation_operations() -> u64 {
    THREAD_ALLOCATION_OPERATIONS.with(Cell::get)
}

/// Requested allocation volume on this thread, including the full new size of
/// reallocations and failed requests. Not live memory, RSS, or V8's own allocator.
#[cfg(test)]
pub(crate) fn thread_requested_bytes() -> u64 {
    THREAD_REQUESTED_BYTES.with(Cell::get)
}

/// Outstanding successful Rust System requests across all threads. Excludes
/// allocator rounding/metadata, C/C++ allocation and V8's own heap. Concurrent
/// operations can cross a sample boundary; this is not an ownership census.
/// Test-only accounting leaves the shipped allocator's hot path unchanged.
#[cfg(test)]
pub(crate) fn live_requested_bytes() -> i64 {
    LIVE_REQUESTED_BYTES.load(Ordering::Relaxed)
}

// SAFETY: Every allocation operation is forwarded to `System` unchanged. The
// relaxed atomic counter is observational and neither retains nor modifies
// pointers or layouts.
unsafe impl GlobalAlloc for CountingSystemAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        // SAFETY: Forwarding the caller-provided layout unchanged.
        let pointer = unsafe { System.alloc(layout) };
        #[cfg(test)]
        if !pointer.is_null() {
            record_live_change(layout.size() as i64);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        // SAFETY: Forwarding the caller-provided layout unchanged.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        #[cfg(test)]
        if !pointer.is_null() {
            record_live_change(layout.size() as i64);
        }
        pointer
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        #[cfg(test)]
        record_live_change(-(layout.size() as i64));
        // SAFETY: Forwarding the allocation's original pointer and layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation(new_size);
        // SAFETY: Forwarding the original pointer/layout and requested size.
        let replacement = unsafe { System.realloc(ptr, layout, new_size) };
        #[cfg(test)]
        if !replacement.is_null() {
            record_live_change(new_size as i64 - layout.size() as i64);
        }
        replacement
    }
}

pub(crate) fn allocation_operations() -> Option<u64> {
    TRACKING_INSTALLED
        .load(Ordering::Relaxed)
        .then(|| ALLOCATION_OPERATIONS.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinking_allocation_releases_requested_live_bytes() {
        let original = Layout::from_size_align(64, 8).unwrap();
        let smaller = Layout::from_size_align(24, 8).unwrap();
        let before = THREAD_LIVE_REQUESTED_DELTA.with(Cell::get);
        // SAFETY: Both layouts are valid. On failed realloc the original
        // pointer remains owned; on success only the replacement is freed.
        unsafe {
            let pointer = CountingSystemAllocator.alloc(original);
            assert!(!pointer.is_null());
            assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get) - before, 64);
            let replacement = CountingSystemAllocator.realloc(pointer, original, smaller.size());
            if replacement.is_null() {
                assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get) - before, 64);
                CountingSystemAllocator.dealloc(pointer, original);
            } else {
                assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get) - before, 24);
                CountingSystemAllocator.dealloc(replacement, smaller);
            }
        }
        assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get), before);
    }

    #[test]
    fn requested_volume_counts_full_reallocation_size_not_live_memory() {
        let small = Layout::from_size_align(16, 8).unwrap();
        let zeroed = Layout::from_size_align(32, 8).unwrap();
        let resized = Layout::from_size_align(48, 8).unwrap();
        let operations = thread_allocation_operations();
        let bytes = thread_requested_bytes();
        let live = THREAD_LIVE_REQUESTED_DELTA.with(Cell::get);
        // SAFETY: Layouts are valid and each successful allocation is freed once
        // with its current layout; a failed realloc leaves the old pointer live.
        unsafe {
            let first = CountingSystemAllocator.alloc(small);
            assert!(!first.is_null());
            assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get) - live, 16);
            let second = CountingSystemAllocator.alloc_zeroed(zeroed);
            assert!(!second.is_null());
            assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get) - live, 48);
            let replacement = CountingSystemAllocator.realloc(first, small, resized.size());
            if replacement.is_null() {
                CountingSystemAllocator.dealloc(first, small);
            } else {
                assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get) - live, 80);
                CountingSystemAllocator.dealloc(replacement, resized);
            }
            CountingSystemAllocator.dealloc(second, zeroed);
        }
        assert_eq!(thread_allocation_operations() - operations, 3);
        assert_eq!(thread_requested_bytes() - bytes, 16 + 32 + 48);
        assert_eq!(THREAD_LIVE_REQUESTED_DELTA.with(Cell::get), live);
    }
}
