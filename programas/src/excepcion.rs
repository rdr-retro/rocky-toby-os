//! `excepcion`: ejecuta una instruccion invalida (`ud2`) en anillo 3.
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    unsafe { core::arch::asm!("ud2") }
    loop {
        core::hint::spin_loop();
    }
}
