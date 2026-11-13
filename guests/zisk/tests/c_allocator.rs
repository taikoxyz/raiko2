//! Native checks for the guest's C allocation bridge, without ZisK dependencies.
#[path = "../src/sys.rs"]
mod sys;

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ptr,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

thread_local! {
    static TRACK_ALLOCATION: Cell<bool> = const { Cell::new(false) };
}
static LAST_RAW_ALLOCATION: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static WATCH_FREE: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static MATCHED_DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_SIZE: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_ALIGNMENT: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATED_SIZE: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATED_ALIGNMENT: AtomicUsize = AtomicUsize::new(0);

struct TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if TRACK_ALLOCATION.with(Cell::get) {
            LAST_RAW_ALLOCATION.store(pointer, Ordering::SeqCst);
            ALLOCATED_SIZE.store(layout.size(), Ordering::SeqCst);
            ALLOCATED_ALIGNMENT.store(layout.align(), Ordering::SeqCst);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer == WATCH_FREE.load(Ordering::SeqCst) {
            MATCHED_DEALLOCATIONS.fetch_add(1, Ordering::SeqCst);
            DEALLOCATED_SIZE.store(layout.size(), Ordering::SeqCst);
            DEALLOCATED_ALIGNMENT.store(layout.align(), Ordering::SeqCst);
        }
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

#[test]
fn c_allocator_preserves_alignment_contents_and_real_deallocation() {
    unsafe { sys::guest_free(ptr::null_mut()) };
    for length in [0, 1, 15, 16, 17, 127, 4096] {
        TRACK_ALLOCATION.with(|active| active.set(true));
        let allocation = unsafe { sys::guest_malloc(length) };
        TRACK_ALLOCATION.with(|active| active.set(false));
        assert!(!allocation.is_null(), "malloc({length}) failed");
        assert_eq!(allocation as usize % 16, 0);
        let bytes = allocation.cast::<u8>();
        for offset in 0..length {
            unsafe { bytes.add(offset).write((offset % 251) as u8) };
        }
        for offset in 0..length {
            assert_eq!(unsafe { bytes.add(offset).read() }, (offset % 251) as u8);
        }
        let raw = LAST_RAW_ALLOCATION.load(Ordering::SeqCst);
        assert!(!raw.is_null());
        WATCH_FREE.store(raw, Ordering::SeqCst);
        let before = MATCHED_DEALLOCATIONS.load(Ordering::SeqCst);
        unsafe { sys::guest_free(allocation) };
        assert_eq!(MATCHED_DEALLOCATIONS.load(Ordering::SeqCst), before + 1);
        assert_eq!(
            DEALLOCATED_SIZE.load(Ordering::SeqCst),
            ALLOCATED_SIZE.load(Ordering::SeqCst)
        );
        assert_eq!(
            DEALLOCATED_ALIGNMENT.load(Ordering::SeqCst),
            ALLOCATED_ALIGNMENT.load(Ordering::SeqCst)
        );
        WATCH_FREE.store(ptr::null_mut(), Ordering::SeqCst);
    }

    for _ in 0..64 {
        let allocation = unsafe { sys::guest_calloc(37, 19) };
        assert!(!allocation.is_null());
        assert_eq!(allocation as usize % 16, 0);
        let bytes = unsafe { std::slice::from_raw_parts(allocation.cast::<u8>(), 37 * 19) };
        assert!(bytes.iter().all(|byte| *byte == 0));
        unsafe { sys::guest_free(allocation) };
    }
    for (count, size) in [(0, 0), (0, 8), (8, 0)] {
        let allocation = unsafe { sys::guest_calloc(count, size) };
        assert!(!allocation.is_null());
        unsafe { sys::guest_free(allocation) };
    }
    assert!(unsafe { sys::guest_malloc(usize::MAX) }.is_null());
    assert!(unsafe { sys::guest_malloc(isize::MAX as usize) }.is_null());
    assert!(unsafe { sys::guest_calloc(usize::MAX, 2) }.is_null());
}
