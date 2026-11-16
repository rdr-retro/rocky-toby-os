//! `fork`: prueba fork() y wait4().
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{escribir, fork, salir, wait4};

const MSG_PADRE: &[u8] = b"rocky: soy el padre; espero al hijo...\n";
const MSG_HIJO: &[u8] = b"rocky: soy el hijo (anillo 3).\n";
const MSG_PADRE_2: &[u8] = b"rocky: el hijo termino; sigo yo.\n";

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let pid = fork();
    if pid == 0 {
        escribir(1, MSG_HIJO);
        salir(0);
    } else {
        escribir(1, MSG_PADRE);
        let _ = wait4(-1, core::ptr::null_mut());
        escribir(1, MSG_PADRE_2);
        salir(0);
    }
}
