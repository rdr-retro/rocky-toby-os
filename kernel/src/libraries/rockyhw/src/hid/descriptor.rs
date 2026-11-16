//! El **descriptor de clase HID** (0x21) y lo que declara.
//!
//! Es el descriptor que va detras de la interfaz y dice cuantas clases de
//! descriptor declara el dispositivo y cuanto mide el de informe. Para el
//! protocolo boot no hace falta interpretar el de informe (los informes son
//! fijos), pero si comprobar que existe y pedirlo: un dispositivo que no lo
//! declara no se puede tratar como HID completo.
//!
//! Referencia: HID 1.11, §6.2.1 (tabla 6-1) y apendice B.

/// Bytes del descriptor HID (con un unico descriptor de clase declarado).
pub const TAM_DESCRIPTOR_HID: u16 = 9;

/// Tipo de descriptor de clase HID (`bDescriptorType`).
pub const TIPO_DESCRIPTOR_HID: u8 = 0x21;

/// Tipo de descriptor de informe (`bDescriptorType` de la entrada).
pub const TIPO_DESCRIPTOR_INFORME: u8 = 0x22;

/// Lee el descriptor HID y devuelve lo que declara medir el descriptor de
/// informe.
///
/// Devuelve `None` si el descriptor no sirve para lo que hace falta:
///
/// - no cabe en lo que se ha leido;
/// - no es un descriptor HID;
/// - no declara ningun descriptor de clase;
/// - el primero que declara no es el de informe (0x22);
/// - el de informe mide cero.
pub fn largo_del_informe(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < TAM_DESCRIPTOR_HID as usize {
        return None;
    }
    if bytes[0] < TAM_DESCRIPTOR_HID as u8 || bytes[1] != TIPO_DESCRIPTOR_HID {
        return None;
    }
    // `bNumDescriptors` y la primera entrada `{tipo, longitud}`.
    if bytes[5] == 0 || bytes[6] != TIPO_DESCRIPTOR_INFORME {
        return None;
    }
    let largo = u16::from_le_bytes([bytes[7], bytes[8]]);
    if largo == 0 {
        return None;
    }
    Some(largo)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUENO: [u8; 9] = [
        9,
        TIPO_DESCRIPTOR_HID,
        0x11,
        0x01,
        0,
        1,
        TIPO_DESCRIPTOR_INFORME,
        63,
        0,
    ];

    #[test]
    fn lee_la_longitud_del_informe() {
        assert_eq!(largo_del_informe(&BUENO), Some(63));
    }

    #[test]
    fn sin_descriptores_no_vale() {
        let mut bytes = BUENO;
        bytes[5] = 0;
        assert_eq!(largo_del_informe(&bytes), None);
    }

    #[test]
    fn el_primero_tiene_que_ser_el_informe() {
        let mut bytes = BUENO;
        bytes[6] = 0x23;
        assert_eq!(largo_del_informe(&bytes), None);
    }

    #[test]
    fn longitud_cero_no_vale() {
        let mut bytes = BUENO;
        bytes[7] = 0;
        bytes[8] = 0;
        assert_eq!(largo_del_informe(&bytes), None);
    }

    #[test]
    fn tiene_que_ser_un_descriptor_hid() {
        let mut bytes = BUENO;
        bytes[1] = 0x22;
        assert_eq!(largo_del_informe(&bytes), None);
        // bLength menor que el descriptor: no cabe la entrada.
        let mut corto = BUENO;
        corto[0] = 8;
        assert_eq!(largo_del_informe(&corto), None);
    }

    #[test]
    fn un_descriptor_incompleto_no_vale() {
        assert_eq!(largo_del_informe(&BUENO[..8]), None);
        assert_eq!(largo_del_informe(&[]), None);
    }
}
