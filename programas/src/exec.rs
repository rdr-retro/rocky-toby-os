//! `exec`: ejecuta execve("/bin/hola", ["uno","dos",NULL], NULL).
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{escribir, execve, salir};

const RUTA: &[u8] = b"/bin/hola\0";
const A0: &[u8] = b"uno\0";
const A1: &[u8] = b"dos\0";
const FALLO: &[u8] = b"rocky: execve no pudo cargar /bin/hola.\n";

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let argv = [A0.as_ptr() as u64, A1.as_ptr() as u64, 0u64];
    let res = execve(RUTA.as_ptr(), argv.as_ptr(), core::ptr::null());
    if res != 0 {
        escribir(1, FALLO);
        salir(1);
    }
    salir(0);
}
