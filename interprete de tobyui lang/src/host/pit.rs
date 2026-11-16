//! Reloj monotónico del anfitrión, expresado en tics de 10 ms como el PIT.

use std::sync::OnceLock;
use std::time::Instant;

static INICIO: OnceLock<Instant> = OnceLock::new();

/// Tics transcurridos desde que se inicializó el anfitrión (100 Hz).
pub fn ticks() -> u64 {
    INICIO.get_or_init(Instant::now).elapsed().as_millis() as u64 / 10
}
