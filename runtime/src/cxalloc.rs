extern crate alloc;

use buddy_alloc::{NonThreadsafeAlloc};
use core::alloc::{Layout, GlobalAlloc};
use cortex_m::asm;

// define what happens in an Out Of Memory (OOM) condition
#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    asm::bkpt();
    loop {}
}

pub struct CortexMSafeAlloc(
    pub NonThreadsafeAlloc,
);

unsafe impl GlobalAlloc for CortexMSafeAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        cortex_m::interrupt::free(|_cs| self.0.alloc(layout))
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        cortex_m::interrupt::free(|_cs| self.0.dealloc(ptr, layout))
    }
}



