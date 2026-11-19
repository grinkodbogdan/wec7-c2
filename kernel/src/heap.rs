//! Kernel heap: size-class free lists over a static arena.
//!
//! Small blocks (16 B .. 4 KiB, powers of two) come from per-class free
//! lists; larger blocks are page multiples kept on a first-fit list. Blocks
//! are never split or merged, which keeps the allocator tiny and
//! predictable; kernel allocations are few and long-lived.

use crate::global::Global;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, addr_of_mut};

const HEAP_SIZE: usize = 16 << 20;
const CLASSES: usize = 9; // 16 << 0 ..= 16 << 8 (4096)
const PAGE: usize = 4096;

#[repr(C, align(4096))]
struct Arena([u8; HEAP_SIZE]);

static mut ARENA: Arena = Arena([0; HEAP_SIZE]);

struct State {
    next: usize,
    end: usize,
    small: [usize; CLASSES],
    /// Singly linked list of large blocks: [next, size] stored in the block.
    large: usize,
}

static STATE: Global<State> = Global::new(State { next: 0, end: 0, small: [0; CLASSES], large: 0 });

pub fn init() {
    // SAFETY: boot-time, single reference.
    let s = unsafe { STATE.get() };
    let start = addr_of_mut!(ARENA) as usize;
    s.next = start;
    s.end = start + HEAP_SIZE;
}

fn class_of(size: usize) -> usize {
    let size = size.max(16);
    (size.next_power_of_two().trailing_zeros() - 4) as usize
}

struct Heap;

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let s = STATE.get();
        let size = layout.size().max(layout.align());
        if size <= PAGE {
            let c = class_of(size);
            let block = 16usize << c;
            if s.small[c] != 0 {
                let p = s.small[c];
                s.small[c] = *(p as *const usize);
                return p as *mut u8;
            }
            let p = (s.next + block - 1) & !(block - 1);
            if p + block > s.end {
                return ptr::null_mut();
            }
            s.next = p + block;
            return p as *mut u8;
        }
        let size = (layout.size() + PAGE - 1) & !(PAGE - 1);
        let align = layout.align().max(PAGE);
        // First fit on the large list.
        let mut prev: *mut usize = &mut s.large;
        let mut cur = s.large;
        while cur != 0 {
            let next = *(cur as *const usize);
            let bsize = *((cur + 8) as *const usize);
            if bsize >= size && cur.is_multiple_of(align) {
                *prev = next;
                return cur as *mut u8;
            }
            prev = cur as *mut usize;
            cur = next;
        }
        let p = (s.next + align - 1) & !(align - 1);
        if p + size > s.end {
            return ptr::null_mut();
        }
        s.next = p + size;
        p as *mut u8
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        let s = STATE.get();
        let size = layout.size().max(layout.align());
        if size <= PAGE {
            let c = class_of(size);
            *(p as *mut usize) = s.small[c];
            s.small[c] = p as usize;
        } else {
            // Large blocks keep their original (page-rounded) size.
            let size = (layout.size() + PAGE - 1) & !(PAGE - 1);
            *(p as *mut usize) = s.large;
            *((p as usize + 8) as *mut usize) = size;
            s.large = p as usize;
        }
    }
}

#[global_allocator]
static HEAP: Heap = Heap;
