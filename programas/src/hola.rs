//! `hola`: ejecutable de ejemplo que usa `execve`.
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{escribir, salir};

const MSG: &[u8] = b"rocky: execve ha cargado un programa nuevo desde el disco.\n";

#[no_mangle]
pub extern "C" fn rocky_main(argc: u64) -> ! {
    let mut d = [0u8; 2];
    d[0] = b'0' + (argc as u8).min(9);
    d[1] = b'\n';
    escribir(1, &d);
    escribir(1, MSG);
    salir(0)
}
