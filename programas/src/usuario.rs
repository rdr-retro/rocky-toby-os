//! `usuario`: el programa de ejemplo mas simple. Imprime un saludo y termina.
//!
//! ```text
//!   write(1, msg, len)
//!   exit(0)
//! ```
//!
//! Es el `.roka` -> `.rkx` de rocky (orden `usuario` del interprete).

#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{escribir, salir};

const MSG: &[u8] = b"rocky: hola desde el anillo 3 (programa de usuario).\n";

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    escribir(1, MSG);
    salir(0)
}
