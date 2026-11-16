//! ACPI minimo: lo justo para apagar la maquina por el estado S5.
//!
//! En vez de adivinar en que puerto vive `PM1_CNT` (que cambia de un chipset a
//! otro), se leen las tablas que deja el firmware: el `RSDP` apunta al `RSDT`/
//! `XSDT`, ahi esta el `FADT` (`FACP`), que dice el puerto real de control, y el
//! `DSDT` define el paquete `\_S5` con el `SLP_TYP` que apaga de verdad.
//!
//! El bootloader deja identidad-mapeados los primeros 4 GiB, asi que estas
//! tablas (que viven en RAM por debajo de esa cota en cualquier equipo de esta
//! epoca) se leen como memoria normal. Solo se leen direcciones de ese rango;
//! si algo se sale de ahi, se descarta y el apagado cae en los metodos clasicos
//! ([`crate::cpu::shutdown`]).

use crate::cpu::{io_wait, outw};
use core::ptr;

/// Lo que hizo falta para apagar, para poder informar por pantalla.
pub struct S5 {
    /// Puerto `PM1a_CNT` (el que suele bastar).
    pub pm1a: u16,
    /// Puerto `PM1b_CNT`, o 0 si el equipo no lo tiene.
    pub pm1b: u16,
    /// Valor de `SLP_TYP` para PM1a.
    pub typ_a: u8,
    /// Valor de `SLP_TYP` para PM1b.
    pub typ_b: u8,
    /// `true` si el `SLP_TYP` salio del DSDT (y no del valor por defecto).
    pub del_dsdt: bool,
}

/// Tope de memoria fisica que se puede leer (la identidad del arranque).
const TOPE: usize = 0x1_0000_0000;

// --- Lectura de memoria fisica (identidad 0..4 GiB) -------------------------

#[inline]
fn u8_at(p: usize) -> u8 {
    unsafe { ptr::read_volatile(p as *const u8) }
}

#[inline]
fn u16_at(p: usize) -> u16 {
    unsafe { ptr::read_unaligned(p as *const u16) }
}

#[inline]
fn u32_at(p: usize) -> u32 {
    unsafe { ptr::read_unaligned(p as *const u32) }
}

#[inline]
fn u64_at(p: usize) -> u64 {
    unsafe { ptr::read_unaligned(p as *const u64) }
}

/// `true` si en `p` empieza la firma `sig` (y la direccion es utilizable).
fn firma(p: usize, sig: &[u8; 4]) -> bool {
    if p < 0x1000 || p >= TOPE {
        return false;
    }
    (0..4).all(|i| u8_at(p + i) == sig[i])
}

/// Suma de comprobacion de un bloque de bytes.
fn suma(p: usize, len: usize) -> u8 {
    let mut s = 0u8;
    for i in 0..len {
        s = s.wrapping_add(u8_at(p + i));
    }
    s
}

// --- Localizacion de las tablas ---------------------------------------------

/// Busca el `RSDP`: primero en la EBDA (su segmento esta en 0x40E) y despues en
/// la zona de ROM del BIOS.
fn buscar_rsdp() -> Option<usize> {
    let seg = u16_at(0x40E) as usize;
    if seg != 0 {
        let base = seg << 4;
        if let Some(p) = rastrear(base, 1024) {
            return Some(p);
        }
    }
    rastrear(0xE0000, 0x20000)
}

/// Rastrea `len` bytes desde `base` buscando el `RSDP` (alineado a 16).
fn rastrear(base: usize, len: usize) -> Option<usize> {
    const FIRMA: &[u8; 8] = b"RSD PTR ";
    let mut p = base;
    while p + 20 <= base + len {
        if (0..8).all(|i| u8_at(p + i) == FIRMA[i]) && suma(p, 20) == 0 {
            return Some(p);
        }
        p += 16;
    }
    None
}

/// Direccion del `RSDT`/`XSDT` y el tamano de cada entrada (4 u 8 bytes).
fn raiz() -> Option<(usize, usize)> {
    let rsdp = buscar_rsdp()?;
    let rev = u8_at(rsdp + 15);
    let rsdt = u32_at(rsdp + 16) as usize;
    let xsdt = if rev >= 2 {
        u64_at(rsdp + 24) as usize
    } else {
        0
    };

    if xsdt != 0 && xsdt < TOPE && firma(xsdt, b"XSDT") {
        return Some((xsdt, 8));
    }
    if rsdt != 0 && rsdt < TOPE && firma(rsdt, b"RSDT") {
        return Some((rsdt, 4));
    }
    None
}

/// Recorre las tablas colgadas del `RSDT`/`XSDT`; `f` recibe la direccion y la
/// firma de cada una y devuelve `true` para parar.
fn recorrer<F: FnMut(usize, &[u8; 4]) -> bool>(mut f: F) {
    let Some((tabla, ancho)) = raiz() else {
        return;
    };
    let len = u32_at(tabla + 4) as usize;
    if !(36..=0x1_0000).contains(&len) {
        return;
    }
    let n = (len - 36) / ancho;
    for i in 0..n {
        let e = if ancho == 8 {
            u64_at(tabla + 36 + i * 8) as usize
        } else {
            u32_at(tabla + 36 + i * 4) as usize
        };
        if e == 0 || e >= TOPE {
            continue;
        }
        let mut sig = [0u8; 4];
        for (k, b) in sig.iter_mut().enumerate() {
            *b = u8_at(e + k);
        }
        if f(e, &sig) {
            return;
        }
    }
}

/// Direccion del `FADT` (`FACP`).
fn buscar_fadt() -> Option<usize> {
    let mut fadt = None;
    recorrer(|dir, sig| {
        if sig == b"FACP" {
            fadt = Some(dir);
            true
        } else {
            false
        }
    });
    fadt
}

/// `\_S5` es el nombre AML `5F 53 35 5F` (`_S5_`).
const NOMBRE_S5: u32 = 0x5F35_535F;

/// Busca el paquete `\_S5` en el DSDT y, si no esta, en las SSDT.
///
/// Devuelve `(SLP_TYP de PM1a, SLP_TYP de PM1b)`.
fn buscar_s5(dsdt: usize) -> Option<(u8, u8)> {
    if dsdt != 0 && dsdt < TOPE {
        if let Some(v) = s5_en(dsdt) {
            return Some(v);
        }
    }
    let mut hallado = None;
    recorrer(|dir, sig| {
        if sig == b"SSDT" {
            if let Some(v) = s5_en(dir) {
                hallado = Some(v);
                return true;
            }
        }
        false
    });
    hallado
}

/// Rastrea una tabla de AML buscando `\_S5`.
fn s5_en(tabla: usize) -> Option<(u8, u8)> {
    let len = u32_at(tabla + 4) as usize;
    if !(36..=0x10_0000).contains(&len) {
        return None;
    }
    let mut i = 36;
    while i + 4 <= len {
        if u32_at(tabla + i) == NOMBRE_S5 {
            // Detras del nombre viene el `PackageOp` (0x12); se mira un margen
            // corto por si el firmware mete antes un `NameOp` (0x08).
            let fin = core::cmp::min(i + 32, len);
            let mut j = i + 4;
            while j < fin {
                if u8_at(tabla + j) == 0x12 {
                    if let Some(v) = leer_paquete(tabla + j) {
                        return Some(v);
                    }
                }
                j += 1;
            }
        }
        i += 1;
    }
    None
}

/// Lee los dos primeros enteros de un paquete AML (`PackageOp` ya situado).
fn leer_paquete(p: usize) -> Option<(u8, u8)> {
    // `PkgLength`: los dos bits altos del primer byte dicen cuantos le siguen.
    let b0 = u8_at(p + 1);
    let extra = (b0 >> 6) as usize;
    let mut q = p + 2 + extra;
    let n = u8_at(q) as usize;
    q += 1;

    let mut vals = [0u8; 2];
    let mut k = 0;
    for _ in 0..n {
        if k >= 2 {
            break;
        }
        let op = u8_at(q);
        match op {
            0x00 => {
                vals[k] = 0;
                k += 1;
                q += 1;
            }
            0x01 => {
                vals[k] = 1;
                k += 1;
                q += 1;
            }
            0xFF => {
                vals[k] = 0xFF;
                k += 1;
                q += 1;
            }
            0x0A => {
                vals[k] = u8_at(q + 1);
                k += 1;
                q += 2;
            }
            0x0B => {
                vals[k] = (u16_at(q + 1) & 0xFF) as u8;
                k += 1;
                q += 3;
            }
            0x0C => {
                vals[k] = (u32_at(q + 1) & 0xFF) as u8;
                k += 1;
                q += 5;
            }
            _ => break,
        }
    }
    if k == 0 {
        return None;
    }
    let a = vals[0] & 0x07;
    let b = if k >= 2 { vals[1] & 0x07 } else { a };
    Some((a, b))
}

// --- Apagado ----------------------------------------------------------------

/// Averigua puerto y `SLP_TYP` a partir de las tablas del firmware.
///
/// `None` si no hay ACPI utilizable. Si el DSDT no tiene `\_S5`, se usa el 5,
/// que es el valor de los chipsets Intel (y de QEMU/Bochs).
pub fn preparar() -> Option<S5> {
    let fadt = buscar_fadt()?;
    // Los campos extendidos (de 64 bits) solo son validos si la longitud que
    // declara la tabla los cubre; si no, leeriamos fuera de ella.
    let len = u32_at(fadt + 4) as usize;
    let mut pm1a = u32_at(fadt + 0x40) as u16;
    let mut pm1b = u32_at(fadt + 0x44) as u16;
    // Si el campo de 32 bits viene vacio, se prueba el de 64 bits
    // (`X_PM1a_CNT_BLK`/`X_PM1b_CNT_BLK` son GAS: la direccion va al offset +4).
    if pm1a == 0 && len >= 0xB8 {
        let a = u64_at(fadt + 0xAC + 4) as usize;
        if (1..0x1_0000).contains(&a) {
            pm1a = a as u16;
            if len >= 0xC4 {
                let b = u64_at(fadt + 0xB8 + 4) as usize;
                if (1..0x1_0000).contains(&b) {
                    pm1b = b as u16;
                }
            }
        }
    }
    if pm1a == 0 {
        return None;
    }
    let dsdt32 = u32_at(fadt + 0x28) as usize;
    let dsdt64 = if len >= 0x94 {
        u64_at(fadt + 0x8C) as usize
    } else {
        0
    };
    let dsdt = if dsdt64 != 0 && dsdt64 < TOPE {
        dsdt64
    } else {
        dsdt32
    };

    let (typ_a, typ_b, del_dsdt) = match buscar_s5(dsdt) {
        Some((a, b)) => (a, b, true),
        None => (5, 5, false),
    };
    Some(S5 {
        pm1a,
        pm1b,
        typ_a,
        typ_b,
        del_dsdt,
    })
}

/// Enciende el apagado: escribe `SLP_EN` en el puerto real de control.
///
/// A partir de esta llamada el chipset corta la corriente y no se vuelve. Se
/// separa de [`preparar`] para poder informar por pantalla *antes* de apagar
/// (cuando funciona, ya no hay ocasion de escribir nada).
pub fn encender(s5: &S5) {
    // El `SLP_TYP` va en los bits 12:10 y `SLP_EN` es el bit 13.
    unsafe {
        if s5.pm1b != 0 {
            outw(s5.pm1b, ((s5.typ_b as u16) << 10) | 0x2000);
            io_wait();
        }
        outw(s5.pm1a, ((s5.typ_a as u16) << 10) | 0x2000);
        io_wait();
    }
}
