//! Altavoz interno de la placa (PC speaker) para rocky.
//!
//! El sonido se genera programando el canal 2 del PIT: su salida es la
//! frecuencia que determina el *beeper* de 1.53 MHz del classic PC, y se
//! habilita abriendo la puerta de salida del puerto 0x61 (bits 0-1).

use crate::cpu::{inb, outb};
use crate::pit;

/// Reloj de referencia del PIT (frecuencia del altavoz).
const PIT_HZ: u32 = 1_193_182;

/// Canales del comando (bits 6-7) y modo (bits 1-3) del PIT.
const CMD_LOHI_MODE3: u8 = 0xB6; // canal 2, modo 3, lobyte/hibyte

/// Reproduce un tono continuo de `freq_hz` (0 silencia el altavoz).
///
/// El tono sigue sonando hasta que se llama a [`stop`] o a [`beep`].
pub fn tone(freq_hz: u32) {
    if freq_hz == 0 {
        stop();
        return;
    }
    let divisor = (PIT_HZ / freq_hz).clamp(1, 0xFFFF);
    unsafe {
        outb(0x43, CMD_LOHI_MODE3);
        outb(0x42, divisor as u8);
        outb(0x42, (divisor >> 8) as u8);

        let gate = inb(0x61);
        if gate & 0x03 != 0x03 {
            outb(0x61, gate | 0x03);
        }
    }
}

/// Apaga el altavoz (suena un tono de longitud arbitraria con [`tone`]).
pub fn stop() {
    unsafe {
        let gate = inb(0x61);
        outb(0x61, gate & 0xFC);
    }
}

/// Espera `ms` milisegundos usando el temporizador del sistema (100 Hz).
///
/// Como el PIT tica cada 10 ms, la resolucion minima es de 10 ms; el
/// programa avanza de todas formas y no se pierde el ritmo de la melodia.
///
/// Si la multitarea esta en marcha, la espera **cede el turno**: las demas
/// tareas (el escritorio, el raton) siguen vivos mientras esta duerme.
pub fn sleep_ms(ms: u32) {
    if ms == 0 {
        return;
    }
    if crate::tarea::activo() {
        crate::tarea::dormir(ms as u64);
        return;
    }
    let tics = (ms + 9) / 10;
    let start = pit::ticks();
    while pit::ticks().wrapping_sub(start) < tics as u64 {
        core::hint::spin_loop();
    }
}

/// `BEEP`: tono de `freq_hz` durante `ms` milisegundos.
pub fn beep(freq_hz: u32, ms: u32) {
    tone(freq_hz);
    sleep_ms(ms);
    stop();
}

/// Reproduce una secuencia de notas `(frecuencia, duracion)`.
///
/// Se usar para encadenar notas de una melodia o de la demo grafica.
pub fn play(notes: &[(u32, u32)]) {
    for &(freq, ms) in notes {
        if freq == 0 {
            sleep_ms(ms);
        } else {
            beep(freq, ms);
        }
    }
}
