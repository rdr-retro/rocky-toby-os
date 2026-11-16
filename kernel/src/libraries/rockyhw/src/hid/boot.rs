//! Los dos informes del protocolo boot: teclado (8 bytes) y raton (3 bytes).
//!
//! El protocolo boot los deja en un formato fijo, sin descriptores de informe
//! que interpretar (HID 1.11, apendice B):
//!
//! - **Teclado**: `[modificadores, reservado, tecla1..tecla6]`. Los seis
//!   bytes de teclas son usos HID; el 0 significa "hueco libre".
//! - **Raton**: `[botones, dx, dy]`, con los desplazamientos en complemento a
//!   dos y el eje Y creciendo hacia abajo.
//!
//! Un raton puede mandar un cuarto byte (la rueda) o mas; los tres primeros
//! son siempre los mismos, asi que se leen esos y se ignoran los demas.

/// Bytes de un informe de teclado boot.
pub const TAM_INFORME_TECLADO: usize = 8;
/// Bytes minimos de un informe de raton boot.
pub const TAM_INFORME_RATON: usize = 3;
/// Teclas simultaneas que caben en un informe boot.
pub const TECLAS_SIMULTANEAS: usize = 6;

/// Uso HID que significa "se han pulsado mas teclas de las que caben".
///
/// No es una tecla: es un aviso de desbordamiento. Convertirlo en una
/// pulsacion inventaria una tecla que nadie ha pulsado.
pub const USO_ERROR_ROLLOVER: u8 = 0x01;
/// Uso HID de "fallo del autotest" (tampoco es una tecla).
pub const USO_FALLO_POST: u8 = 0x02;
/// Uso HID de "error indefinido" (tampoco es una tecla).
pub const USO_ERROR_INDEFINIDO: u8 = 0x03;

/// Si un uso es una tecla de verdad.
///
/// Los tres primeros usos de la pagina de teclado son avisos de error, y el 0
/// es "ninguna tecla". Ninguno de los cuatro se puede traducir.
pub fn es_tecla(uso: u8) -> bool {
    uso > USO_ERROR_INDEFINIDO
}

/// Un informe de teclado boot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InformeTeclado {
    /// Los ocho modificadores, un bit por cada uno (0xE0..0xE7).
    pub modificadores: u8,
    /// Los usos de las teclas pulsadas; 0 es un hueco libre.
    pub teclas: [u8; TECLAS_SIMULTANEAS],
}

impl InformeTeclado {
    /// Lee un informe. Devuelve `None` si es mas corto de lo que manda el
    /// protocolo: con menos de ocho bytes no se puede saber que teclas lleva.
    ///
    /// Un informe mas largo **si** vale: el protocolo boot fija los ocho
    /// primeros bytes y deja el resto para el dispositivo.
    pub fn leer(bytes: &[u8]) -> Option<InformeTeclado> {
        if bytes.len() < TAM_INFORME_TECLADO {
            return None;
        }
        let mut teclas = [0u8; TECLAS_SIMULTANEAS];
        teclas.copy_from_slice(&bytes[2..TAM_INFORME_TECLADO]);
        Some(InformeTeclado {
            modificadores: bytes[0],
            teclas,
        })
    }

    /// Si el informe dice que ha habido desbordamiento de teclas.
    pub fn hay_rollover(&self) -> bool {
        self.teclas.contains(&USO_ERROR_ROLLOVER)
    }

    /// Cuantas teclas lleva pulsadas (sin contar huecos ni avisos de error).
    pub fn cuantas(&self) -> u8 {
        self.teclas.iter().filter(|t| es_tecla(**t)).count() as u8
    }
}

/// Un informe de raton boot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InformeRaton {
    /// Botones: bit 0 izquierdo, bit 1 derecho, bit 2 central.
    pub botones: u8,
    /// Desplazamiento horizontal (positivo: derecha).
    pub dx: i8,
    /// Desplazamiento vertical (positivo: **abajo**).
    pub dy: i8,
}

impl InformeRaton {
    /// Lee un informe. Devuelve `None` si le faltan los tres bytes.
    ///
    /// Solo se usan los tres primeros botones: los bits 3 y 4 del primer byte
    /// son de otros botones y el resto de bytes (rueda o modos) no forman
    /// parte del protocolo boot.
    pub fn leer(bytes: &[u8]) -> Option<InformeRaton> {
        if bytes.len() < TAM_INFORME_RATON {
            return None;
        }
        Some(InformeRaton {
            botones: bytes[0] & 0x07,
            dx: bytes[1] as i8,
            dy: bytes[2] as i8,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teclado_lee_el_formato_boot() {
        let bytes = [0x02, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00];
        let r = InformeTeclado::leer(&bytes).unwrap();
        assert_eq!(r.modificadores, 0x02);
        assert_eq!(r.teclas, [0x04, 0, 0, 0, 0, 0]);
        assert!(!r.hay_rollover());
        assert_eq!(r.cuantas(), 1);
    }

    #[test]
    fn teclado_corto_no_vale() {
        assert!(InformeTeclado::leer(&[0; 7]).is_none());
        assert!(InformeTeclado::leer(&[]).is_none());
    }

    #[test]
    fn teclado_largo_vale_y_lee_los_ocho_primeros() {
        let bytes = [0x00, 0x00, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0xFF, 0xFF];
        let r = InformeTeclado::leer(&bytes).unwrap();
        assert_eq!(r.teclas, [0x04, 0x05, 0x06, 0x07, 0x08, 0x09]);
    }

    #[test]
    fn rollover_se_reconoce_y_no_cuenta_como_tecla() {
        let bytes = [0x00, 0x00, 0x01, 0x04, 0x00, 0x00, 0x00, 0x00];
        let r = InformeTeclado::leer(&bytes).unwrap();
        assert!(r.hay_rollover());
        assert_eq!(r.cuantas(), 1);
        assert!(!es_tecla(USO_ERROR_ROLLOVER));
        assert!(!es_tecla(USO_FALLO_POST));
        assert!(!es_tecla(USO_ERROR_INDEFINIDO));
        assert!(es_tecla(0x04));
    }

    #[test]
    fn raton_lee_botones_y_signos() {
        let r = InformeRaton::leer(&[0x05, 0xF6, 0x0A]).unwrap();
        assert_eq!(r.botones, 0x05);
        assert_eq!(r.dx, -10);
        assert_eq!(r.dy, 10);
    }

    #[test]
    fn raton_ignora_la_rueda_y_los_botones_de_mas() {
        let r = InformeRaton::leer(&[0x3F, 0x01, 0xFF, 0x7F]).unwrap();
        assert_eq!(r.botones, 0x07);
        assert_eq!(r.dx, 1);
        assert_eq!(r.dy, -1);
    }

    #[test]
    fn raton_corto_no_vale() {
        assert!(InformeRaton::leer(&[0x00, 0x01]).is_none());
        assert!(InformeRaton::leer(&[]).is_none());
    }
}
