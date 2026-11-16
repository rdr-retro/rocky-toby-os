//! Hubs USB 2.0 (fase 7).
//!
//! Nucleo acotado: descriptores, estados de puerto y limites. El HCD es el que
//! manda las peticiones de clase al hub (`GET_STATUS`, `SET_FEATURE`,
//! `CLEAR_FEATURE`, `GET_DESCRIPTOR`) con sus propias transferencias de
//! control.
//!
//! Un hub se **reconoce, configura, alimenta y recorre**: sus hijos se
//! enumeran con *route string* y, cuando hace falta, con la informacion del
//! TT. El HCD sondea un puerto por ronda para detectar conexiones y
//! desconexiones en caliente sin bloquear los endpoints HID.

use crate::drivers::usb::core::ErrorUsb;

/// Clase de dispositivo hub (USB 2.0, 11.23.1).
pub const CLASE_HUB: u8 = 0x09;
/// Subclase y protocolo de un hub (11.23.1).
pub const SUBCLASE_HUB: u8 = 0x00;
pub const PROTOCOLO_HUB: u8 = 0x00;
/// `bDeviceProtocol` de un hub de alta velocidad con varios TT (11.23.1).
pub const PROTOCOLO_HUB_MULTI_TT: u8 = 0x02;

/// Tipo de descriptor de hub (11.23.2.1).
pub const TIPO_DESC_HUB: u8 = 0x29;
/// Longitud minima del descriptor de hub (sin los mapas de puertos).
pub const TAM_DESC_HUB: u16 = 9;
/// Longitud que se pide: cabecera y mapas de un hub pequeno.
pub const TAM_DESC_HUB_PEDIDO: u16 = 9;

/// Caracteristicas de puerto que se piden con `SET_FEATURE` (11.24.2).
pub const CARACTERISTICA_PORT_ENABLE: u16 = 0x0001;
pub const CARACTERISTICA_PORT_SUSPEND: u16 = 0x0002;
pub const CARACTERISTICA_PORT_RESET: u16 = 0x0004;
pub const CARACTERISTICA_PORT_POWER: u16 = 0x0008;

/// Cambios de puerto que se limpian con `CLEAR_FEATURE` (11.24.2).
pub const CARACTERISTICA_C_PORT_CONNECTION: u16 = 0x0010;
pub const CARACTERISTICA_C_PORT_ENABLE: u16 = 0x0011;
pub const CARACTERISTICA_C_PORT_SUSPEND: u16 = 0x0012;
pub const CARACTERISTICA_C_PORT_OVER_CURRENT: u16 = 0x0013;
pub const CARACTERISTICA_C_PORT_RESET: u16 = 0x0014;

/// Bits de `wPortStatus` (11.24.2.7.1).
pub const ESTADO_CONECTADO: u16 = 0x0001;
pub const ESTADO_HABILITADO: u16 = 0x0002;
pub const ESTADO_SUSPENDIDO: u16 = 0x0004;
pub const ESTADO_SOBRECORRIENTE: u16 = 0x0008;
pub const ESTADO_RESET: u16 = 0x0010;
pub const ESTADO_ALIMENTADO: u16 = 0x0100;
pub const ESTADO_VELOCIDAD_BAJA: u16 = 0x0200;
pub const ESTADO_VELOCIDAD_ALTA: u16 = 0x0400;

/// Maximo de puertos que se recorren de un hub.
///
/// Un hub real puede declarar hasta 255. Aqui se acota para no recorrer un
/// descriptor absurdo: los puertos que pasan del tope no se tocan y se avisa.
pub const MAX_PUERTOS_HUB: usize = 15;

/// Profundidad maxima de anidamiento de hubs.
pub const MAX_PROFUNDIDAD_HUB: u8 = 4;

/// Descriptor de hub (USB 2.0, 11.23.2.1).
#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct DescHubRaw {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub b_nbr_ports: u8,
    pub w_hub_characteristics: u16,
    pub b_pwr_on2pwr_good: u8,
    pub b_hub_contr_current: u8,
}

/// Lo que interesa del descriptor de un hub.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HubInfo {
    /// Puertos que declara (sin recortar).
    pub puertos: u8,
    /// Cuantos se recorren de verdad.
    pub puertos_utiles: u8,
    /// Si es el hub raiz del controlador (no tiene descriptor propio).
    pub es_root: bool,
    /// `wHubCharacteristics`.
    pub caracteristicas: u16,
    /// `bPwrOn2PwrGood`, en unidades de 2 ms.
    pub tiempo_encendido: u8,
    /// `bHubContrCurrent`, en miliamperios.
    pub corriente: u8,
    /// `bDeviceProtocol` del descriptor de dispositivo: 0 completa, 1 alta
    /// con un TT, 2 alta con varios TT.
    pub protocolo: u8,
}

impl HubInfo {
    /// Si el hub tiene transacciones multiple (MTT).
    ///
    /// El bit de MTT no esta en `wHubCharacteristics`: esta en el
    /// `bDeviceProtocol` del descriptor de dispositivo (§11.23.1).
    pub fn multi_tt(&self) -> bool {
        self.protocolo == PROTOCOLO_HUB_MULTI_TT
    }

    /// Tiempo que tarda en estabilizarse la alimentacion, en milisegundos.
    pub fn estabilizacion_ms(&self) -> u64 {
        (self.tiempo_encendido as u64) * 2
    }
}

/// Estado de un puerto de hub, ya decodificado.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EstadoPuertoHub {
    pub numero: u8,
    pub conectado: bool,
    pub habilitado: bool,
    pub suspendido: bool,
    pub sobrecorriente: bool,
    pub en_reset: bool,
    pub alimentado: bool,
    pub velocidad_baja: bool,
    pub velocidad_alta: bool,
    pub cambio_conexion: bool,
    pub cambio_habilitacion: bool,
    pub cambio_suspendido: bool,
    pub cambio_sobrecorriente: bool,
    pub cambio_reset: bool,
}

impl EstadoPuertoHub {
    /// Decodifica `wPortStatus` y `wPortChange` (11.24.2.7).
    pub fn decodificar(numero: u8, estado: u16, cambios: u16) -> Self {
        EstadoPuertoHub {
            numero,
            conectado: (estado & ESTADO_CONECTADO) != 0,
            habilitado: (estado & ESTADO_HABILITADO) != 0,
            suspendido: (estado & ESTADO_SUSPENDIDO) != 0,
            sobrecorriente: (estado & ESTADO_SOBRECORRIENTE) != 0,
            en_reset: (estado & ESTADO_RESET) != 0,
            alimentado: (estado & ESTADO_ALIMENTADO) != 0,
            velocidad_baja: (estado & ESTADO_VELOCIDAD_BAJA) != 0,
            velocidad_alta: (estado & ESTADO_VELOCIDAD_ALTA) != 0,
            cambio_conexion: (cambios & ESTADO_CONECTADO) != 0,
            cambio_habilitacion: (cambios & ESTADO_HABILITADO) != 0,
            cambio_suspendido: (cambios & ESTADO_SUSPENDIDO) != 0,
            cambio_sobrecorriente: (cambios & ESTADO_SOBRECORRIENTE) != 0,
            cambio_reset: (cambios & ESTADO_RESET) != 0,
        }
    }

    /// Si el puerto tiene un dispositivo que se pueda intentar usar.
    pub fn con_dispositivo(&self) -> bool {
        self.conectado && !self.sobrecorriente
    }

    /// La velocidad que declara el puerto, para la consola.
    pub fn velocidad(&self) -> &'static str {
        if self.velocidad_baja {
            "baja"
        } else if self.velocidad_alta {
            "alta"
        } else {
            "completa"
        }
    }
}

/// Limites de tiempo para hablar con un hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitesHub {
    /// Espera maxima a que la alimentacion se estabilice.
    pub max_estabilizacion_ms: u64,
    /// Espera maxima a que el hub acepte un cambio.
    pub plazo_transferencia: u64,
}

impl Default for LimitesHub {
    fn default() -> Self {
        LimitesHub {
            max_estabilizacion_ms: 600,
            plazo_transferencia: 100,
        }
    }
}

/// Se puede hablar con este dispositivo como hub.
pub fn es_hub(clase: u8) -> bool {
    clase == CLASE_HUB
}

/// ¿Es la interfaz de un hub?
pub fn es_interfaz_hub(clase: u8, subclase: u8, protocolo: u8) -> bool {
    clase == CLASE_HUB && subclase == SUBCLASE_HUB && protocolo == PROTOCOLO_HUB
}

/// Parsea el descriptor de hub con limites estrictos.
///
/// Acepta descriptores de cualquier numero de puertos y deja en
/// [`HubInfo::puertos_utiles`] los que se van a recorrer. Un descriptor
/// truncado, de otro tipo o con cero puertos se rechaza.
pub fn parsear_hub(bytes: &[u8]) -> Result<HubInfo, ErrorUsb> {
    if bytes.len() < core::mem::size_of::<DescHubRaw>() {
        return Err(ErrorUsb::Truncado);
    }
    // Seguro: se ha comprobado que caben los siete bytes que se leen; el
    // descriptor se copia a un buffer alineado en el HCD.
    let d = unsafe { &*(bytes.as_ptr() as *const DescHubRaw) };
    if d.b_descriptor_type != TIPO_DESC_HUB {
        return Err(ErrorUsb::EstadoInvalido);
    }
    if d.b_length < core::mem::size_of::<DescHubRaw>() as u8 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    if d.b_nbr_ports == 0 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    let utiles = d.b_nbr_ports.min(MAX_PUERTOS_HUB as u8);
    Ok(HubInfo {
        puertos: d.b_nbr_ports,
        puertos_utiles: utiles,
        es_root: false,
        caracteristicas: d.w_hub_characteristics,
        tiempo_encendido: d.b_pwr_on2pwr_good,
        corriente: d.b_hub_contr_current,
        protocolo: PROTOCOLO_HUB,
    })
}
