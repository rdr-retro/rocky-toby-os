//! Núcleo USB acotado: tipos, descriptores, enumeración y transferencias.
//!
//! Este módulo sigue las reglas del plan: sin asignación en IRQ, colas acotadas,
//! validación estricta y sin asumir longitudes no confiables. El HCD traduce
//! las operaciones a TRBs; el núcleo USB no los manipula.
#![allow(dead_code)] // API preparada; se conectará al HCD al completar enumeración USB.

pub mod descriptor;
pub mod enumerate;
pub mod hub;
pub mod transfer;

use core::time::Duration;

/// Velocidad de un dispositivo USB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VelocidadUsb {
    Desconocida,
    Baja,
    Completa,
    Alta,
    Super,
    SuperPlus,
}

impl From<rockyhw::xhci::Velocidad> for VelocidadUsb {
    fn from(v: rockyhw::xhci::Velocidad) -> Self {
        use rockyhw::xhci::Velocidad::*;
        match v {
            Indefinida => VelocidadUsb::Desconocida,
            Completa => VelocidadUsb::Completa,
            Baja => VelocidadUsb::Baja,
            Alta => VelocidadUsb::Alta,
            _ => VelocidadUsb::Super,
        }
    }
}

/// Límites duros (regla 5).
pub const MAX_DISPOSITIVOS: usize = 32;
pub const MAX_CONFIGURACIONES: usize = 8;
pub const MAX_INTERFACES: usize = 16;
pub const MAX_ENDPOINTS: usize = 32;
pub const MAX_HUBS: usize = 8;
pub const MAX_DESCRIPTORES_BYTES: usize = 512;

/// Dirección de dispositivo USB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DireccionUsb(u8);

impl DireccionUsb {
    pub const fn nueva(d: u8) -> Option<Self> {
        if d == 0 || d > 127 {
            None
        } else {
            Some(DireccionUsb(d))
        }
    }
    pub const fn valor(self) -> u8 {
        self.0
    }
}

/// Tipo de transferencia.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipoTransferencia {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

/// Información acotada de un endpoint.
#[derive(Clone, Copy, Debug)]
pub struct EndpointInfo {
    pub direccion: u8, // bit 7 = dirección (1=IN,0=OUT)
    pub numero: u8,
    pub tipo: TipoTransferencia,
    pub tamano_paquete: u16,
    pub intervalo: u8,
}

/// Configuración acotada.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConfigInfo {
    pub interfaces: u8,
    pub valor: u8,
}

/// Interfaz acotada.
#[derive(Clone, Copy, Debug, Default)]
pub struct InterfaceInfo {
    pub clase: u8,
    pub subclase: u8,
    pub protocolo: u8,
    pub endpoints: u8,
}

/// Dispositivo USB acotado.
#[derive(Clone, Copy, Debug)]
pub struct DispositivoUsb {
    pub direccion: Option<DireccionUsb>,
    pub velocidad: VelocidadUsb,
    pub clase: u8,
    pub subclase: u8,
    pub protocolo: u8,
    pub vend: u16,
    pub prod: u16,
    pub config_actual: Option<u8>,
}

impl Default for DispositivoUsb {
    fn default() -> Self {
        DispositivoUsb {
            direccion: None,
            velocidad: VelocidadUsb::Desconocida,
            clase: 0,
            subclase: 0,
            protocolo: 0,
            vend: 0,
            prod: 0,
            config_actual: None,
        }
    }
}

/// Resultado acotado para transferencias de control.
#[derive(Clone, Copy, Debug)]
pub struct ResultadoControl {
    pub ok: bool,
    pub bytes_transferidos: u16,
}

/// Errores del núcleo USB (recuperables).
#[derive(Clone, Copy, Debug)]
pub enum ErrorUsb {
    LongitudInvalida,
    Truncado,
    PuestoInvalido,
    Desborde,
    DireccionInvalida,
    SinHueco,
    SinMemoria,
    Timeout,
    Stall,
    Cancelado,
    NoSoportado,
    EstadoInvalido,
}

/// Parámetros para timeout con deadline.
#[derive(Clone, Copy, Debug)]
pub struct Limites {
    pub timeout: Duration,
    pub max_iter: u32,
}

impl Default for Limites {
    fn default() -> Self {
        Limites {
            timeout: Duration::from_millis(1000),
            max_iter: 10000,
        }
    }
}
