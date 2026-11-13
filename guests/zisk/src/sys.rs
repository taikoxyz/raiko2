//! Guest-only C allocation bridge for the protocol's C zlib backend.
//!
//! All C allocations use the guest's Rust global allocator (ZisK dlmalloc).
//! A fixed aligned header preserves the exact Layout required by Rust dealloc.
//! Exporting C names is target-gated to avoid interposing on native test libc.
use std::{
    alloc::{alloc, dealloc, Layout},
    ffi::c_void,
    mem::size_of,
    ptr,
};

// RISC-V LP64's max_align_t includes a 16-byte aligned long double.
const C_ALIGNMENT: usize = 16;

#[repr(C, align(16))]
struct AllocationHeader {
    total_size: usize,
}

/// Allocate C storage, returning null on size overflow or allocation failure.
/// Zero-size allocation returns a unique, freeable allocation.
///
/// # Safety
/// The result must be freed only by `guest_free`; Rust collections must not own it.
#[cfg_attr(target_vendor = "zisk", export_name = "malloc")]
pub unsafe extern "C" fn guest_malloc(size: usize) -> *mut c_void {
    let Some(total_size) = size.max(1).checked_add(size_of::<AllocationHeader>()) else {
        return ptr::null_mut();
    };
    let Ok(layout) = Layout::from_size_align(total_size, C_ALIGNMENT) else {
        return ptr::null_mut();
    };
    // SAFETY: layout is valid and nonzero; the selected runtime owns the allocator.
    let base = unsafe { alloc(layout) };
    if base.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: layout includes the aligned header and at least one payload byte.
    unsafe {
        base.cast::<AllocationHeader>()
            .write(AllocationHeader { total_size });
        base.add(size_of::<AllocationHeader>()).cast()
    }
}

/// Allocate zero-initialized C storage; multiplication overflow returns null.
///
/// # Safety
/// The result has the same ownership contract as `guest_malloc`.
#[cfg_attr(target_vendor = "zisk", export_name = "calloc")]
pub unsafe extern "C" fn guest_calloc(count: usize, size: usize) -> *mut c_void {
    let Some(length) = count.checked_mul(size) else {
        return ptr::null_mut();
    };
    // SAFETY: this function forwards the same allocation/ownership contract.
    let payload = unsafe { guest_malloc(length) };
    if !payload.is_null() {
        // SAFETY: malloc reserved at least length bytes, and length cannot overflow.
        unsafe { ptr::write_bytes(payload.cast::<u8>(), 0, length) };
    }
    payload
}

/// Release C storage through the Rust allocator; null is accepted.
///
/// # Safety
/// A non-null pointer must be a live allocation returned by this bridge. It must
/// not be offset, previously freed, or allocated by a different allocator.
#[cfg_attr(target_vendor = "zisk", export_name = "free")]
pub unsafe extern "C" fn guest_free(payload: *mut c_void) {
    if payload.is_null() {
        return;
    }
    // SAFETY: the ownership contract guarantees an intact header directly before
    // payload; its stored size is the exact original allocation Layout.
    unsafe {
        let base = payload.cast::<u8>().sub(size_of::<AllocationHeader>());
        let total_size = (*base.cast::<AllocationHeader>()).total_size;
        let layout = Layout::from_size_align_unchecked(total_size, C_ALIGNMENT);
        dealloc(base, layout);
    }
}
