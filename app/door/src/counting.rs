//! THE FOLD CACHE'S MEASURE (O-69, FC-10): the system allocator, counting each thread's
//! bytes allocated net of those freed, so what a cache entry retains is counted on the
//! thread that builds it, not estimated. pacific-core forbids unsafe code, so the
//! embedding holds the allocator and names its count (`fold_cache::set_measure`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

pub struct Counting;

thread_local! {
    // A const Cell: no lazy init and no destructor, so counting never allocates.
    static NET: Cell<isize> = const { Cell::new(0) };
}

fn count(n: isize) {
    // Wrapping: a panic inside the allocator is undefined behaviour, reachable or not.
    let _ = NET.try_with(|c| c.set(c.get().wrapping_add(n)));
}

/// This thread's bytes allocated, net of those it freed.
pub fn allocated() -> isize {
    NET.try_with(Cell::get).unwrap_or(0)
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            count(l.size() as isize);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() {
            count(l.size() as isize);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        count(-(l.size() as isize));
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            count((new as isize).wrapping_sub(l.size() as isize));
        }
        q
    }
}
