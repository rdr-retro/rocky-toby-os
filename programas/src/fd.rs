//! `fd`: abre `/bin/mensaje`, lo lee, lo escribe por la consola y lo cierra.
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{abrir, cerrar, escribir, leer, salir, O_RDONLY};

const RUTA: &[u8] = b"/bin/mensaje\0";
const MSG: &[u8] = b"rocky: descriptores de fichero (open/read/close) en anillo 3.\n";
const FALLO: &[u8] = b"rocky: la capa de descriptores fallo.\n";

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let fd = abrir(RUTA.as_ptr(), O_RDONLY);
    if fd < 0 {
        escribir(1, FALLO);
        salir(1);
    }
    let mut buf = [0u8; 128];
    let n = leer(fd as u64, &mut buf);
    if n < 0 {
        cerrar(fd as u64);
        escribir(1, FALLO);
        salir(1);
    }
    escribir(1, &buf[..n as usize]);
    cerrar(fd as u64);
    escribir(1, MSG);
    salir(0)
}
