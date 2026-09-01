use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

static ALLOCATION_OPERATIONS: AtomicU64 = AtomicU64::new(0);
static TRACKING_INSTALLED: AtomicBool = AtomicBool::new(false);

/// System allocator wrapper used by the native shell's process-wide telemetry.
///
/// The counter measures allocation operations rather than bytes. Sampling it
/// around a frame is conservative: allocations from any shell thread during
/// that interval are charged to the frame.
#[allow(dead_code)] // The reusable fixture library reads the counter but does not install it.
pub(crate) struct CountingSystemAllocator;

// SAFETY: Every allocation operation is forwarded to `System` unchanged. The
// relaxed atomic counter is observational and neither retains nor modifies
// pointers or layouts.
unsafe impl GlobalAlloc for CountingSystemAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        TRACKING_INSTALLED.store(true, Ordering::Relaxed);
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Forwarding the caller-provided layout unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        TRACKING_INSTALLED.store(true, Ordering::Relaxed);
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Forwarding the caller-provided layout unchanged.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forwarding the allocation's original pointer and layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        TRACKING_INSTALLED.store(true, Ordering::Relaxed);
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Forwarding the original pointer/layout and requested size.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

pub(crate) fn allocation_operations() -> Option<u64> {
    TRACKING_INSTALLED
        .load(Ordering::Relaxed)
        .then(|| ALLOCATION_OPERATIONS.load(Ordering::Relaxed))
}
