//! Enumeración USB estándar (esqueleto acotado).
//!
//! Fase 3: obtener descriptors, configurar y enlazar. Sin código de HCD aquí.
//! Los detalles de Enable Slot/Address Device/Transfer Rings se añadirán al
//! HCD xHCI conforme avance la implementación.

use super::{DispositivoUsb, ErrorUsb, VelocidadUsb, MAX_DESCRIPTORES_BYTES};

#[derive(Clone, Copy, Debug)]
pub struct EstadoEnum {
    pub dispositivo: DispositivoUsb,
    pub fase: u8, // 0: slot, 1: addr, 2: desc, 3: config, 4: interfaz
}

impl Default for EstadoEnum {
    fn default() -> Self {
        EstadoEnum {
            dispositivo: DispositivoUsb::default(),
            fase: 0,
        }
    }
}

pub fn nuevo_estado(velocidad: VelocidadUsb) -> EstadoEnum {
    let mut d = DispositivoUsb::default();
    d.velocidad = velocidad;
    EstadoEnum {
        dispositivo: d,
        fase: 0,
    }
}

pub fn clasificar_por_clase(clase: u8, subclase: u8) -> &'static str {
    match clase {
        0x00 => "desconocido (clase del interfaz)",
        0x03 => match subclase {
            0x01 => "HID - teclado",
            0x02 => "HID - ratón",
            _ => "HID",
        },
        0x09 => "hub",
        0x08 => "masivo",
        _ => "otra",
    }
}

/// Validación básica del descriptor completo de configuración.
pub fn validar_config_completa(buf: &[u8]) -> Result<(), ErrorUsb> {
    if buf.len() < 9 || buf.len() > MAX_DESCRIPTORES_BYTES {
        return Err(ErrorUsb::Desborde);
    }
    Ok(())
}
