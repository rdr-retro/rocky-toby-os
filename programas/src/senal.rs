//! `senal`: `sigaction` + `kill` + `sigreturn` (Fase 7).
//!
//! El manejador imprime y vuelve; el `ret` salta al **restaurador**, que pide
//! `sigreturn` para recuperar el contexto. Al final se envia `SIGTERM`, cuya
//! accion por defecto termina el proceso.
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{escribir, getpid, kill, salir, sigaction, sigreturn, SIGTERM, SIGUSR1};

const MSG_HAND: &[u8] = b"senal: manejador SIGUSR1 en ejecucion\n";
const MSG_VUELTA: &[u8] = b"senal: de vuelta del manejador\n";
const OK: &[u8] = b"rocky: senales (sigaction + kill + sigreturn) OK\n";
const MSG_NUNCA: &[u8] = b"rocky: ERROR no deberia verse (SIGTERM debio matar)\n";
const FALLO: &[u8] = b"rocky: la prueba de senales fallo\n";

/// Manejador de `SIGUSR1`: imprime y vuelve (el `ret` salta al restaurador).
extern "C" fn manejador() {
    escribir(1, MSG_HAND);
}

/// Restaurador: pide `sigreturn` para recuperar el contexto salvado.
extern "C" fn restaurador() {
    sigreturn();
}

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    // struct sigaction { manejador, restaurador, banderas }
    let act = [
        manejador as *const () as u64,
        restaurador as *const () as u64,
        0u64,
    ];
    if sigaction(SIGUSR1, act.as_ptr()) < 0 {
        escribir(1, FALLO);
        salir(1);
    }
    // El manejador ya ha corrido al volver de `kill`.
    kill(getpid(), SIGUSR1);
    escribir(1, MSG_VUELTA);
    escribir(1, OK);
    // Accion por defecto: el proceso muere aqui.
    kill(getpid(), SIGTERM);
    escribir(1, MSG_NUNCA);
    salir(1)
}
