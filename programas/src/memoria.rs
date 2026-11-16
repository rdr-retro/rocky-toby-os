//! `memoria`: brk, paginacion bajo demanda y copia-al-escribir (Fase 5).
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{brk, escribir, fork, salir, wait4};

const OK: &[u8] = b"rocky: memoria bajo demanda, brk y copia-al-escribir OK.\n";
const MAL: &[u8] = b"rocky: la prueba de memoria/COW fallo.\n";

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let p = brk(0);
    brk(p + 8192);
    unsafe {
        *(p as *mut u8) = b'A';
        *((p + 4096) as *mut u8) = b'B';
        if *(p as *const u8) != b'A' {
            escribir(1, MAL);
            salir(1);
        }
        if *((p + 4096) as *const u8) != b'B' {
            escribir(1, MAL);
            salir(1);
        }
    }
    let pid = fork();
    if pid == 0 {
        unsafe { *(p as *mut u8) = b'Z' };
        salir(0);
    }
    let _ = wait4(-1, core::ptr::null_mut());
    unsafe {
        if *(p as *const u8) != b'A' {
            escribir(1, MAL);
            salir(1);
        }
    }
    escribir(1, OK);
    salir(0)
}
