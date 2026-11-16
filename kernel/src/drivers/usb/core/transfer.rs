//! Transferencias USB acotadas (setup/control/interrupt).
//!
//! El HCD se encarga de programar los TRBs y del ciclo/toggle. Aquí solo hay
//! tipos y validación.

use super::{ErrorUsb, ResultadoControl};

/// Paquete SETUP (8 bytes).
#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct PaqueteSetup {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

impl PaqueteSetup {
    pub fn nuevo(
        bm_request_type: u8,
        b_request: u8,
        w_value: u16,
        w_index: u16,
        w_length: u16,
    ) -> Self {
        PaqueteSetup {
            bm_request_type,
            b_request,
            w_value,
            w_index,
            w_length,
        }
    }
}

/// Control transfer: GET_DESCRIPTOR (estándar).
pub const REQ_GET_DESCRIPTOR: u8 = 0x06;
pub const REQ_SET_ADDRESS: u8 = 0x05;
pub const REQ_SET_CONFIGURATION: u8 = 0x09;
pub const REQ_SET_INTERFACE: u8 = 0x0B;
pub const REQ_SET_PROTOCOL: u8 = 0x0B;
pub const REQ_GET_PROTOCOL: u8 = 0x03;
pub const REQ_SET_IDLE: u8 = 0x0A;

pub const DT_DEVICE: u8 = 1;
pub const DT_CONFIG: u8 = 2;
pub const DT_STRING: u8 = 3;
pub const DT_INTERFACE: u8 = 4;
pub const DT_ENDPOINT: u8 = 5;
pub const DT_HID: u8 = 33;
pub const DT_REPORT: u8 = 34;

pub fn setup_get_descriptor(descriptor: u8, indice: u8, longitud: u16) -> PaqueteSetup {
    let w_value = ((descriptor as u16) << 8) | (indice as u16);
    PaqueteSetup::nuevo(0x80, REQ_GET_DESCRIPTOR, w_value, 0, longitud)
}

pub fn setup_set_address(direccion: u8) -> PaqueteSetup {
    PaqueteSetup::nuevo(0x00, REQ_SET_ADDRESS, direccion as u16, 0, 0)
}

pub fn setup_set_configuration(config: u8) -> PaqueteSetup {
    PaqueteSetup::nuevo(0x00, REQ_SET_CONFIGURATION, config as u16, 0, 0)
}

pub fn setup_set_interface(interfaz: u8, alterna: u8) -> PaqueteSetup {
    PaqueteSetup::nuevo(0x01, REQ_SET_INTERFACE, alterna as u16, interfaz as u16, 0)
}

pub fn setup_set_protocol(interfaz: u8, protocolo: u8) -> PaqueteSetup {
    PaqueteSetup::nuevo(0x21, REQ_SET_PROTOCOL, protocolo as u16, interfaz as u16, 0)
}

pub fn setup_set_idle(interfaz: u8, duracion: u8, reporte: u8) -> PaqueteSetup {
    let w_value = ((duracion as u16) << 8) | (reporte as u16);
    PaqueteSetup::nuevo(0x21, REQ_SET_IDLE, w_value, interfaz as u16, 0)
}

/// Peticiones de clase para hubs (USB 2.0, cap. 11).
pub const REQ_GET_STATUS: u8 = 0x00;
pub const REQ_CLEAR_FEATURE: u8 = 0x01;
pub const REQ_SET_FEATURE: u8 = 0x03;
pub const REQ_GET_DESCRIPTOR_HUB: u8 = 0x06;
pub const REQ_CLEAR_TT_BUFFER: u8 = 0x08;
pub const REQ_RESET_TT: u8 = 0x09;
pub const REQ_GET_TT_STATE: u8 = 0x0A;
pub const REQ_STOP_TT: u8 = 0x0B;

pub fn setup_hub_get_descriptor(longitud: u16) -> PaqueteSetup {
    let w_value = ((0x29u16) << 8) | 0;
    PaqueteSetup::nuevo(0xA0, REQ_GET_DESCRIPTOR, w_value, 0, longitud)
}

pub fn setup_hub_get_status() -> PaqueteSetup {
    PaqueteSetup::nuevo(0xA0, REQ_GET_STATUS, 0, 0, 4)
}

pub fn setup_hub_get_port_status(puerto: u8) -> PaqueteSetup {
    PaqueteSetup::nuevo(0xA3, REQ_GET_STATUS, 0, puerto as u16, 4)
}

pub fn setup_hub_set_port_feature(puerto: u8, caracteristica: u16) -> PaqueteSetup {
    PaqueteSetup::nuevo(0x23, REQ_SET_FEATURE, caracteristica, puerto as u16, 0)
}

pub fn setup_hub_clear_port_feature(puerto: u8, caracteristica: u16) -> PaqueteSetup {
    PaqueteSetup::nuevo(0x23, REQ_CLEAR_FEATURE, caracteristica, puerto as u16, 0)
}

pub fn validar_longitud(l: u16, max: u16) -> Result<(), ErrorUsb> {
    if l == 0 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    if l > max {
        return Err(ErrorUsb::Desborde);
    }
    Ok(())
}

pub fn interpretar_resultado(
    stall: bool,
    timeout: bool,
    cancelado: bool,
    transferidos: u16,
) -> ResultadoControl {
    if stall || timeout || cancelado {
        ResultadoControl {
            ok: false,
            bytes_transferidos: transferidos,
        }
    } else {
        ResultadoControl {
            ok: true,
            bytes_transferidos: transferidos,
        }
    }
}
