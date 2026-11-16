//! Reloj de tiempo real (RTC/CMOS) y datos de memoria del BIOS.

use crate::cpu::{inb, outb};

fn read(register: u8) -> u8 {
    unsafe {
        outb(0x70, register);
        // Pequena pausa para que el CMOS procese el indice.
        for _ in 0..8 {
            let _ = inb(0x80);
        }
        inb(0x71)
    }
}

fn update_in_progress() -> bool {
    read(0x0A) & 0x80 != 0
}

fn bcd_to_bin(value: u8) -> u8 {
    (value & 0x0F) + ((value >> 4) * 10)
}

fn bcd_mode() -> bool {
    // Registro de estado B, bit 2 = 1 -> binario.
    read(0x0B) & 0x04 == 0
}

fn decode(value: u8) -> u8 {
    if bcd_mode() {
        bcd_to_bin(value)
    } else {
        value
    }
}

/// Devuelve (horas, minutos, segundos).
pub fn time() -> (u8, u8, u8) {
    while update_in_progress() {}
    let s = decode(read(0x00));
    let m = decode(read(0x02));
    let h = decode(read(0x04)) & 0x7F;
    (h, m, s)
}

/// Devuelve (dia, mes, anio) con el anio completo (p. ej. 2026).
pub fn date() -> (u8, u8, u16) {
    while update_in_progress() {}
    let day = decode(read(0x07));
    let month = decode(read(0x08));
    let year = decode(read(0x09)) as u16;
    let full_year = if year < 80 {
        2000 + year as u16
    } else {
        1900 + year as u16
    };
    (day, month, full_year)
}

/// Segundos desde el 1 de enero de 1970 según el RTC local.
pub fn unix_time() -> i64 {
    const DIA: i64 = 86_400;
    let (hora, min, seg) = time();
    let (dia, mes, anio) = date();
    if anio < 1970 {
        return 0;
    }
    fn bisiesto(a: u16) -> bool {
        (a % 4 == 0 && a % 100 != 0) || a % 400 == 0
    }
    let mut res: i64 = 0;
    for a in 1970..anio {
        res += 365 * DIA;
        if bisiesto(a) {
            res += DIA;
        }
    }
    let acum: [i64; 12] = if bisiesto(anio) {
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335]
    } else {
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
    };
    let m = (mes as usize).clamp(1, 12) - 1;
    res += acum[m] * DIA;
    res += (dia.max(1) as i64 - 1) * DIA;
    res += hora as i64 * 3600 + min as i64 * 60 + seg as i64;
    res
}

/// Memoria extendida instalada, en KiB (por encima de 1 MiB).
#[allow(dead_code)] // Se conserva como consulta del CMOS; el PMM usa el mapa E820.
pub fn extended_memory_kb() -> u32 {
    let low = read(0x17) as u32 | ((read(0x18) as u32) << 8);
    if low != 0 {
        low
    } else {
        let high = read(0x34) as u32 | ((read(0x35) as u32) << 8);
        high * 64
    }
}
