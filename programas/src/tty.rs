//! `tty`: `ioctl` de terminal (TCGETS/TCSETS/TIOCSPGRP/TIOCGPGRP) y `setsid`
//! (Fase 8).
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{
    escribir, getpid, ioctl, leer, salir, setsid, ECHO, ICANON, TCGETS, TCSETS, TIOCGPGRP,
    TIOCSPGRP,
};

const MSG_TC: &[u8] = b"tty: TCGETS ok (termios canonico por defecto)\n";
const MSG_SETSID: &[u8] = b"tty: setsid creo sesion y grupo nuevos\n";
const MSG_CRUDO: &[u8] = b"tty: modo crudo (ECHO/ICANON off) y vuelta\n";
const MSG_PGRP: &[u8] = b"tty: TIOCSPGRP / TIOCGPGRP ok\n";
const MSG_LEE: &[u8] = b"tty: esperando una linea por el teclado...\n";
const OK: &[u8] = b"rocky: tty (termios + ioctl + setsid) OK\n";
const FALLO: &[u8] = b"rocky: la prueba de tty fallo\n";

fn fallo() -> ! {
    escribir(1, FALLO);
    salir(1)
}

/// Lee `c_lflag` (u32 en el desplazamiento 12) de un buffer termios.
fn lflag(t: &[u8; 64]) -> u32 {
    u32::from_le_bytes([t[12], t[13], t[14], t[15]])
}

/// Escribe `c_lflag` en el buffer termios.
fn poner_lflag(t: &mut [u8; 64], v: u32) {
    t[12..16].copy_from_slice(&v.to_le_bytes());
}

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let mut t = [0u8; 64];

    // 1. TCGETS: el termios por defecto debe ser canonico.
    if ioctl(0, TCGETS, t.as_mut_ptr() as u64) < 0 {
        fallo();
    }
    if lflag(&t) & ICANON == 0 {
        fallo();
    }
    escribir(1, MSG_TC);

    // 2. setsid() debe devolver el propio PID.
    let pid = getpid();
    if setsid() != pid {
        fallo();
    }
    escribir(1, MSG_SETSID);

    // 3. Apagar ECHO|ICANON con TCSETS y comprobar que se aplico.
    let f = lflag(&t) & !(ICANON | ECHO);
    poner_lflag(&mut t, f);
    if ioctl(0, TCSETS, t.as_mut_ptr() as u64) < 0 {
        fallo();
    }
    if ioctl(0, TCGETS, t.as_mut_ptr() as u64) < 0 {
        fallo();
    }
    if lflag(&t) & ECHO != 0 {
        fallo();
    }
    escribir(1, MSG_CRUDO);

    // 4. Restaurar ECHO|ICANON para la lectura que viene.
    let f = lflag(&t) | ICANON | ECHO;
    poner_lflag(&mut t, f);
    if ioctl(0, TCSETS, t.as_mut_ptr() as u64) < 0 {
        fallo();
    }

    // 5. Grupo en primer plano: TIOCSPGRP(getpid()) y TIOCGPGRP.
    let mut pgid = [0u8; 4];
    pgid.copy_from_slice(&(getpid() as u32).to_le_bytes());
    if ioctl(0, TIOCSPGRP, pgid.as_mut_ptr() as u64) < 0 {
        fallo();
    }
    let mut pgid2 = [0u8; 4];
    if ioctl(0, TIOCGPGRP, pgid2.as_mut_ptr() as u64) < 0 {
        fallo();
    }
    if pgid2 != pgid {
        fallo();
    }
    escribir(1, MSG_PGRP);

    // 6. Leer una linea de la consola (espera al teclado) y reescribirla.
    escribir(1, MSG_LEE);
    let mut buf = [0u8; 32];
    let n = leer(0, &mut buf);
    if n < 0 {
        fallo();
    }
    escribir(1, &buf[..n as usize]);

    escribir(1, OK);
    salir(0)
}
