//! Sector de arranque de rocky.
//!
//! Todo el codigo real vive en `boot.s`, incluido con `global_asm!`. Este
//! fichero solo existe para que `rustc` produzca un objeto enlazable.

#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

global_asm!(include_str!("boot.s"));

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
