//! `pipe`: tuberia anonima con `pipe` + `fork` (Fase 6).
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{cerrar, escribir, fork, leer, pipe, salir, wait4};

const HIJO: &[u8] = b"hijo: hola desde la tuberia\n";
const ANTES: &[u8] = b"padre: espero a leer de la tuberia...\n";
const DESPUES: &[u8] = b"padre: segunda lectura = 0 (fin de tuberia)\n";
const OK: &[u8] = b"rocky: tuberia anonima (pipe + fork) OK\n";
const FALLO: &[u8] = b"rocky: la tuberia fallo\n";

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let mut fds = [0u32; 2];
    if pipe(fds.as_mut_ptr()) < 0 {
        escribir(1, FALLO);
        salir(1);
    }
    let pid = fork();
    if pid < 0 {
        escribir(1, FALLO);
        salir(1);
    }
    if pid == 0 {
        escribir(fds[1] as u64, HIJO);
        cerrar(fds[1] as u64);
        salir(0);
    }
    cerrar(fds[1] as u64);
    escribir(1, ANTES);
    let mut buf = [0u8; 64];
    let n = leer(fds[0] as u64, &mut buf);
    if n < 0 {
        escribir(1, FALLO);
        salir(1);
    }
    escribir(1, &buf[..n as usize]);
    escribir(1, DESPUES);
    let _ = leer(fds[0] as u64, &mut buf);
    cerrar(fds[0] as u64);
    let _ = wait4(pid as i64, core::ptr::null_mut());
    escribir(1, OK);
    salir(0)
}
