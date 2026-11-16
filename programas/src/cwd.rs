//! `cwd`: `chdir` + `getcwd` + rutas relativas/absolutas (Fase 9).
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{abrir, cerrar, chdir, escribir, getcwd, salir, O_RDONLY};

const MSG_RAIZ: &[u8] = b"cwd: tras chdir(/), getcwd ve la raiz\n";
const MSG_CHDIR: &[u8] = b"cwd: chdir entra en /home/escritorio\n";
const MSG_GET: &[u8] = b"cwd: getcwd ve /home/escritorio (16 bytes)\n";
const MSG_REL: &[u8] = b"cwd: open relativo encuentra hola.rka\n";
const MSG_NO: &[u8] = b"cwd: open relativo NO ve lo de la raiz\n";
const MSG_ABS: &[u8] = b"cwd: open absoluto sigue viendo /bienvenida\n";
const MSG_VOL: &[u8] = b"cwd: de vuelta a la raiz, open bin/hola\n";
const OK: &[u8] = b"rocky: cwd (chdir + getcwd + rutas relativas) OK\n";
const FALLO: &[u8] = b"rocky: la prueba de cwd fallo\n";

const RUTA_ESC: &[u8] = b"/home/escritorio\0";
const RUTA_HOLA: &[u8] = b"hola.rka\0";
const RUTA_BIEN: &[u8] = b"bienvenida\0";
const RUTA_ABS: &[u8] = b"/bienvenida\0";
const RUTA_RAIZ: &[u8] = b"/\0";
const RUTA_BIN: &[u8] = b"bin/hola\0";

fn fallo() -> ! {
    escribir(1, FALLO);
    salir(1)
}

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let mut buf = [0u8; 64];

    // 1. Normaliza a la raiz y comprueba con getcwd.
    if chdir(RUTA_RAIZ.as_ptr()) < 0 {
        fallo();
    }
    if getcwd(buf.as_mut_ptr(), 64) != 1 {
        fallo();
    }
    escribir(1, MSG_RAIZ);

    // 2. chdir("/home/escritorio") y getcwd (16 bytes).
    if chdir(RUTA_ESC.as_ptr()) < 0 {
        fallo();
    }
    escribir(1, MSG_CHDIR);
    if getcwd(buf.as_mut_ptr(), 64) != 16 {
        fallo();
    }
    escribir(1, MSG_GET);

    // 3. open("hola.rka") relativo al cwd: debe existir.
    let fd = abrir(RUTA_HOLA.as_ptr(), O_RDONLY);
    if fd < 0 {
        fallo();
    }
    cerrar(fd as u64);
    escribir(1, MSG_REL);

    // 4. open("bienvenida") relativo al cwd: NO debe existir (esta en la raiz).
    let fd = abrir(RUTA_BIEN.as_ptr(), O_RDONLY);
    if fd >= 0 {
        cerrar(fd as u64);
        fallo();
    }
    escribir(1, MSG_NO);

    // 5. open("/bienvenida") absoluto: debe existir.
    let fd = abrir(RUTA_ABS.as_ptr(), O_RDONLY);
    if fd < 0 {
        fallo();
    }
    cerrar(fd as u64);
    escribir(1, MSG_ABS);

    // 6. chdir("/") y open("bin/hola") relativo a la raiz.
    if chdir(RUTA_RAIZ.as_ptr()) < 0 {
        fallo();
    }
    let fd = abrir(RUTA_BIN.as_ptr(), O_RDONLY);
    if fd < 0 {
        fallo();
    }
    cerrar(fd as u64);
    escribir(1, MSG_VOL);

    // 7. getcwd final: otra vez "/".
    if getcwd(buf.as_mut_ptr(), 64) != 1 {
        fallo();
    }

    escribir(1, OK);
    salir(0)
}
