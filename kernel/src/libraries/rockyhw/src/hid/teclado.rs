//! De **uso HID** a **codigo de barrido del juego 1** y diferencias entre
//! informes de teclado.
//!
//! La tabla es por **posicion fisica**, no por la leyenda: el uso 0x2F (la
//! tecla `[` de un teclado de EE. UU.) es la misma tecla que en un teclado
//! español esta a la derecha de la P y escribe `` ` ``/`^`. Por eso se traduce
//! a la posicion 0x1A del juego 1, que es exactamente la que manda el PS/2 en
//! esa tecla, y la distribucion española del kernel hace el resto.
//!
//! Referencia: HID Usage Tables 1.5, seccion 10, y la tabla `hid_keyboard` de
//! Linux (`drivers/hid/hid-input.c`).

use super::boot::{es_tecla, InformeTeclado};

/// Un codigo de barrido del juego 1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Barrido {
    /// El codigo, **sin** el bit de liberacion (0x80).
    pub codigo: u8,
    /// Si va precedido del prefijo 0xE0 (teclas extendidas).
    pub extendido: bool,
}

impl Barrido {
    const fn normal(codigo: u8) -> Barrido {
        Barrido {
            codigo,
            extendido: false,
        }
    }

    const fn extendido(codigo: u8) -> Barrido {
        Barrido {
            codigo,
            extendido: true,
        }
    }
}

/// Traduce un uso HID de la pagina de teclado (0x07) a su posicion en el juego
/// de barrido 1.
///
/// Devuelve `None` para los usos que no son una tecla, para los que rocky no
/// sabe traducir (Impr Pant y Pausa mandan secuencias de varios bytes que la
/// capa de traduccion no interpreta) y para los que no existen en un teclado
/// español.
pub fn uso_a_barrido(uso: u8) -> Option<Barrido> {
    Some(match uso {
        // Letras: la posicion es la de un QWERTY de EE. UU.
        0x04 => Barrido::normal(0x1E), // a
        0x05 => Barrido::normal(0x30), // b
        0x06 => Barrido::normal(0x2E), // c
        0x07 => Barrido::normal(0x20), // d
        0x08 => Barrido::normal(0x12), // e
        0x09 => Barrido::normal(0x21), // f
        0x0A => Barrido::normal(0x22), // g
        0x0B => Barrido::normal(0x23), // h
        0x0C => Barrido::normal(0x17), // i
        0x0D => Barrido::normal(0x24), // j
        0x0E => Barrido::normal(0x25), // k
        0x0F => Barrido::normal(0x26), // l
        0x10 => Barrido::normal(0x32), // m
        0x11 => Barrido::normal(0x31), // n
        0x12 => Barrido::normal(0x18), // o
        0x13 => Barrido::normal(0x19), // p
        0x14 => Barrido::normal(0x10), // q
        0x15 => Barrido::normal(0x13), // r
        0x16 => Barrido::normal(0x1F), // s
        0x17 => Barrido::normal(0x14), // t
        0x18 => Barrido::normal(0x16), // u
        0x19 => Barrido::normal(0x2F), // v
        0x1A => Barrido::normal(0x11), // w
        0x1B => Barrido::normal(0x2D), // x
        0x1C => Barrido::normal(0x15), // y
        0x1D => Barrido::normal(0x2C), // z
        // Fila numerica.
        0x1E => Barrido::normal(0x02), // 1
        0x1F => Barrido::normal(0x03), // 2
        0x20 => Barrido::normal(0x04), // 3
        0x21 => Barrido::normal(0x05), // 4
        0x22 => Barrido::normal(0x06), // 5
        0x23 => Barrido::normal(0x07), // 6
        0x24 => Barrido::normal(0x08), // 7
        0x25 => Barrido::normal(0x09), // 8
        0x26 => Barrido::normal(0x0A), // 9
        0x27 => Barrido::normal(0x0B), // 0
        // Edicion.
        0x28 => Barrido::normal(0x1C), // Enter
        0x29 => Barrido::normal(0x01), // Escape
        0x2A => Barrido::normal(0x0E), // Retroceso
        0x2B => Barrido::normal(0x0F), // Tabulador
        0x2C => Barrido::normal(0x39), // Espacio
        // Puntuacion, por posicion.
        0x2D => Barrido::normal(0x0C), // -
        0x2E => Barrido::normal(0x0D), // =
        0x2F => Barrido::normal(0x1A), // [
        0x30 => Barrido::normal(0x1B), // ]
        0x31 => Barrido::normal(0x2B), // \
        0x33 => Barrido::normal(0x27), // ;
        0x34 => Barrido::normal(0x28), // '
        0x35 => Barrido::normal(0x29), // `
        0x36 => Barrido::normal(0x33), // ,
        0x37 => Barrido::normal(0x34), // .
        0x38 => Barrido::normal(0x35), // /
        0x64 => Barrido::normal(0x56), // tecla ISO junto al Shift izquierdo
        // Bloqueos.
        0x39 => Barrido::normal(0x3A), // Bloq Mayus
        0x47 => Barrido::normal(0x46), // Bloq Despl
        0x53 => Barrido::normal(0x45), // Bloq Num
        // Funcion.
        0x3A => Barrido::normal(0x3B), // F1
        0x3B => Barrido::normal(0x3C),
        0x3C => Barrido::normal(0x3D),
        0x3D => Barrido::normal(0x3E),
        0x3E => Barrido::normal(0x3F),
        0x3F => Barrido::normal(0x40),
        0x40 => Barrido::normal(0x41),
        0x41 => Barrido::normal(0x42),
        0x42 => Barrido::normal(0x43),
        0x43 => Barrido::normal(0x44), // F10
        0x44 => Barrido::normal(0x57), // F11
        0x45 => Barrido::normal(0x58), // F12
        // Navegacion (extendidas).
        0x49 => Barrido::extendido(0x52), // Insert
        0x4A => Barrido::extendido(0x47), // Inicio
        0x4B => Barrido::extendido(0x49), // Re Pag
        0x4C => Barrido::extendido(0x53), // Supr
        0x4D => Barrido::extendido(0x4F), // Fin
        0x4E => Barrido::extendido(0x51), // Av Pag
        0x4F => Barrido::extendido(0x4D), // Derecha
        0x50 => Barrido::extendido(0x4B), // Izquierda
        0x51 => Barrido::extendido(0x50), // Abajo
        0x52 => Barrido::extendido(0x48), // Arriba
        // Teclado numerico.
        0x54 => Barrido::extendido(0x35), // /
        0x55 => Barrido::normal(0x37),    // *
        0x56 => Barrido::normal(0x4A),    // -
        0x57 => Barrido::normal(0x4E),    // +
        0x58 => Barrido::extendido(0x1C), // Intro
        0x59 => Barrido::normal(0x4F),    // 1
        0x5A => Barrido::normal(0x50),    // 2
        0x5B => Barrido::normal(0x51),    // 3
        0x5C => Barrido::normal(0x4B),    // 4
        0x5D => Barrido::normal(0x4C),    // 5
        0x5E => Barrido::normal(0x4D),    // 6
        0x5F => Barrido::normal(0x47),    // 7
        0x60 => Barrido::normal(0x48),    // 8
        0x61 => Barrido::normal(0x49),    // 9
        0x62 => Barrido::normal(0x52),    // 0
        0x63 => Barrido::normal(0x53),    // .
        // Modificadores: el byte de modificadores va de 0xE0 a 0xE7.
        0xE0 => Barrido::normal(0x1D),    // Ctrl izquierdo
        0xE1 => Barrido::normal(0x2A),    // Shift izquierdo
        0xE2 => Barrido::normal(0x38),    // Alt izquierdo
        0xE3 => Barrido::extendido(0x5B), // Tecla Windows izquierda
        0xE4 => Barrido::extendido(0x1D), // Ctrl derecho
        0xE5 => Barrido::normal(0x36),    // Shift derecho
        0xE6 => Barrido::extendido(0x38), // Alt derecho (AltGr)
        0xE7 => Barrido::extendido(0x5C), // Tecla Windows derecha
        _ => return None,
    })
}

/// Traduce un **bit** del byte de modificadores (0 = Ctrl izquierdo) a su
/// codigo de barrido.
pub fn modificador_a_barrido(bit: u8) -> Option<Barrido> {
    if bit > 7 {
        return None;
    }
    uso_a_barrido(0xE0 + bit)
}

/// Recorre las diferencias entre dos informes y llama a `emite` con cada uso
/// que se pulsa (`true`) o se suelta (`false`).
///
/// El orden importa y por eso no es el orden del informe:
///
/// 1. primero los modificadores que se pulsan, para que un Shift este puesto
///    cuando llegue su letra;
/// 2. despues las teclas que se sueltan;
/// 3. despues las que se pulsan, sin repetir las que ya venian repetidas en el
///    mismo informe;
/// 4. al final los modificadores que se sueltan, para que una letra se suelte
///    antes que su Shift.
///
/// Si el informe trae `ErrorRollOver` se sueltan todas las teclas anteriores y
/// **no se pulsa ninguna**: el desbordamiento no dice cuales son las teclas
/// nuevas, e inventarlas seria peor que perder una.
pub fn eventos(
    anteriores: &InformeTeclado,
    actual: &InformeTeclado,
    emite: &mut impl FnMut(u8, bool),
) {
    for bit in 0..8u8 {
        let mascara = 1 << bit;
        if actual.modificadores & mascara != 0 && anteriores.modificadores & mascara == 0 {
            emite(0xE0 + bit, true);
        }
    }

    for uso in anteriores.teclas.iter().copied() {
        if !es_tecla(uso) {
            continue;
        }
        if actual.hay_rollover() || !actual.teclas.contains(&uso) {
            emite(uso, false);
        }
    }

    if !actual.hay_rollover() {
        for (i, uso) in actual.teclas.iter().copied().enumerate() {
            if !es_tecla(uso) || anteriores.teclas.contains(&uso) {
                continue;
            }
            if actual.teclas[..i].contains(&uso) {
                continue;
            }
            emite(uso, true);
        }
    }

    for bit in 0..8u8 {
        let mascara = 1 << bit;
        if actual.modificadores & mascara == 0 && anteriores.modificadores & mascara != 0 {
            emite(0xE0 + bit, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    extern crate std;
    use std::vec::Vec;

    fn informe(modificadores: u8, teclas: [u8; 6]) -> InformeTeclado {
        InformeTeclado {
            modificadores,
            teclas,
        }
    }

    fn recoger(anteriores: &InformeTeclado, actual: &InformeTeclado) -> Vec<(u8, bool)> {
        let mut v = Vec::new();
        eventos(anteriores, actual, &mut |uso, pulsada| {
            v.push((uso, pulsada))
        });
        v
    }

    #[test]
    fn letras_y_numeros_por_posicion() {
        assert_eq!(uso_a_barrido(0x04), Some(Barrido::normal(0x1E))); // a
        assert_eq!(uso_a_barrido(0x1D), Some(Barrido::normal(0x2C))); // z
        assert_eq!(uso_a_barrido(0x1E), Some(Barrido::normal(0x02))); // 1
        assert_eq!(uso_a_barrido(0x27), Some(Barrido::normal(0x0B))); // 0
        assert_eq!(uso_a_barrido(0x28), Some(Barrido::normal(0x1C))); // Intro
        assert_eq!(uso_a_barrido(0x2C), Some(Barrido::normal(0x39))); // Espacio
    }

    #[test]
    fn la_puntuacion_es_por_posicion_no_por_leyenda() {
        // La tecla `[` de EE. UU. es la de `` ` ``/`^` en español.
        assert_eq!(uso_a_barrido(0x2F), Some(Barrido::normal(0x1A)));
        assert_eq!(uso_a_barrido(0x30), Some(Barrido::normal(0x1B)));
        assert_eq!(uso_a_barrido(0x33), Some(Barrido::normal(0x27))); // ñ
        assert_eq!(uso_a_barrido(0x34), Some(Barrido::normal(0x28))); // ´
        assert_eq!(uso_a_barrido(0x35), Some(Barrido::normal(0x29))); // º
        assert_eq!(uso_a_barrido(0x64), Some(Barrido::normal(0x56))); // <>
    }

    #[test]
    fn extendidas_llevan_prefijo() {
        assert_eq!(uso_a_barrido(0x4F), Some(Barrido::extendido(0x4D))); // derecha
        assert_eq!(uso_a_barrido(0x52), Some(Barrido::extendido(0x48))); // arriba
        assert_eq!(uso_a_barrido(0x54), Some(Barrido::extendido(0x35))); // KP /
    }

    #[test]
    fn modificadores() {
        assert_eq!(modificador_a_barrido(0), Some(Barrido::normal(0x1D)));
        assert_eq!(modificador_a_barrido(1), Some(Barrido::normal(0x2A)));
        assert_eq!(modificador_a_barrido(5), Some(Barrido::normal(0x36)));
        assert_eq!(modificador_a_barrido(6), Some(Barrido::extendido(0x38)));
        assert_eq!(modificador_a_barrido(4), Some(Barrido::extendido(0x1D)));
        assert_eq!(modificador_a_barrido(8), None);
    }

    #[test]
    fn las_funciones_no_se_comen_el_bloqueo_numerico() {
        // F1..F12, que en el juego 1 son 0x3B..0x44 y 0x57/0x58.
        for (uso, codigo) in (0x3A..=0x43).zip(0x3B..=0x44) {
            assert_eq!(uso_a_barrido(uso), Some(Barrido::normal(codigo)));
        }
        assert_eq!(uso_a_barrido(0x44), Some(Barrido::normal(0x57))); // F11
        assert_eq!(uso_a_barrido(0x45), Some(Barrido::normal(0x58))); // F12
    }

    #[test]
    fn lo_que_no_se_sabe_no_se_traduce() {
        for uso in [0x00, 0x01, 0x02, 0x03, 0x32, 0x46, 0x48, 0x65, 0xE8, 0xFF] {
            assert_eq!(uso_a_barrido(uso), None, "uso {uso:#04X}");
        }
    }

    #[test]
    fn pulsar_y_soltar_una_tecla() {
        let vacio = informe(0, [0; 6]);
        let a = informe(0, [0x04, 0, 0, 0, 0, 0]);
        assert_eq!(recoger(&vacio, &a), [(0x04, true)]);
        assert_eq!(recoger(&a, &vacio), [(0x04, false)]);
    }

    #[test]
    fn una_tecla_repetida_no_se_pulsa_dos_veces() {
        let a = informe(0, [0x04, 0, 0, 0, 0, 0]);
        assert!(recoger(&a, &a).is_empty());
        // Ni aunque el informe la traiga dos veces.
        let dos = informe(0, [0x04, 0x04, 0, 0, 0, 0]);
        assert!(recoger(&a, &dos).is_empty());
        // Una tecla nueva repetida en el mismo informe se pulsa una vez.
        assert_eq!(recoger(&informe(0, [0; 6]), &dos), [(0x04, true)]);
    }

    #[test]
    fn el_shift_va_antes_que_su_letra() {
        let vacio = informe(0, [0; 6]);
        let mayus_a = informe(0x02, [0x04, 0, 0, 0, 0, 0]);
        assert_eq!(recoger(&vacio, &mayus_a), [(0xE1, true), (0x04, true)]);
        // Al soltar, la letra se suelta antes que el Shift.
        assert_eq!(recoger(&mayus_a, &vacio), [(0x04, false), (0xE1, false)]);
    }

    #[test]
    fn varias_teclas_a_la_vez() {
        let vacio = informe(0, [0; 6]);
        let dos = informe(0, [0x04, 0x05, 0, 0, 0, 0]);
        assert_eq!(recoger(&vacio, &dos), [(0x04, true), (0x05, true)]);
    }

    #[test]
    fn rollover_suelta_todo_y_no_pulsa_nada() {
        let a_y_b = informe(0, [0x04, 0x05, 0, 0, 0, 0]);
        let rollover = informe(0, [0x01, 0x06, 0x07, 0x08, 0x09, 0x0A]);
        assert_eq!(recoger(&a_y_b, &rollover), [(0x04, false), (0x05, false)]);
        // Desde un informe vacio no se inventa ninguna pulsacion.
        assert!(recoger(&informe(0, [0; 6]), &rollover).is_empty());
    }

    #[test]
    fn los_huecos_no_son_teclas() {
        let vacio = informe(0, [0; 6]);
        let dos_teclas = informe(0, [0x00, 0x04, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(recoger(&vacio, &dos_teclas), [(0x04, true)]);
    }
}
