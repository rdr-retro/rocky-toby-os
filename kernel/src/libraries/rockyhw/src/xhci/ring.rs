//! TRB, anillos y la tabla de segmentos del anillo de eventos.
//!
//! Un TRB (*Transfer Request Block*) son 16 bytes con una plantilla fija
//! (§4.11.1):
//!
//! ```text
//! palabra 0-1  Parameter
//! palabra 2    Status    (bits 31:24)
//! palabra 3    Control   (tipo de TRB en 15:10, bit de ciclo en el bit 0)
//! ```
//!
//! Este modulo sabe leer y escribir esa plantilla, y sabe como se vacila un
//! anillo de eventos: el controlador produce, el software consume, y el bit de
//! ciclo dice si lo que hay en el puntero de extraccion es un evento de verdad
//! o el hueco que se dejo la vez anterior (§4.9.4).
//!
//! Referencia: Intel, "eXtensible Host Controller Interface for Universal
//! Serial Bus (xHCI) Requirements Specification", Rev 1.2, §4.9, §4.11, §6.5.
//!
//! Codigo propio de rocky.

/// Bytes que ocupa un TRB.
pub const TAM_TRB: u64 = 16;

/// Mascara del tipo de TRB: bits 15:10 del componente de control (§4.11.1).
///
/// Ojo: el campo `TRB Type` son **seis** bits, del 15 al 10. La mascara es
/// `0xFC00`: con una mascara mas corta (por ejemplo `0x3C00`) se perderian los
/// bits altos y el tipo se leeria mal. Los tipos de evento van del 32 al 39
/// (tabla 6-91), asi que el bit 15 tiene que estar en la mascara.
pub const TRB_TIPO_MASCARA: u32 = 0x0000_FC00;
/// Desplazamiento del tipo de TRB dentro del componente de control.
pub const TRB_TIPO_DESPLAZAMIENTO: u32 = 10;
/// Mascara del bit de ciclo: bit 0 del componente de control (§4.11.1).
pub const TRB_CICLO_MASCARA: u32 = 1;

/// Un TRB: cuatro palabras de 32 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trb {
    /// Las cuatro palabras del TRB, en orden.
    pub palabras: [u32; 4],
}

impl Trb {
    /// Un TRB vacio (todo a cero).
    pub const NULO: Trb = Trb { palabras: [0; 4] };

    /// Crea un TRB a partir de sus cuatro palabras.
    pub fn nueva(palabras: [u32; 4]) -> Trb {
        Trb { palabras }
    }

    /// El componente `Parameter` (las dos primeras palabras).
    pub fn parameter(&self) -> u64 {
        self.palabras[0] as u64 | ((self.palabras[1] as u64) << 32)
    }

    /// El componente `Status` (palabra 2). El codigo de finalizacion esta en
    /// los bits 31:24.
    pub fn status(&self) -> u32 {
        self.palabras[2]
    }

    /// El codigo de finalizacion de un evento (bits 31:24 del estado).
    pub fn codigo_finalizacion(&self) -> u32 {
        (self.palabras[2] >> 24) & 0xFF
    }

    /// El componente `Control` (palabra 3).
    pub fn control(&self) -> u32 {
        self.palabras[3]
    }

    /// El tipo de TRB (bits 15:10 del control).
    pub fn tipo(&self) -> u32 {
        (self.palabras[3] & TRB_TIPO_MASCARA) >> TRB_TIPO_DESPLAZAMIENTO
    }

    /// El bit de ciclo del TRB.
    pub fn ciclo(&self) -> bool {
        self.palabras[3] & TRB_CICLO_MASCARA != 0
    }

    /// El identificador de puerto de un evento de cambio de puerto.
    ///
    /// El campo `Port ID` esta en los bits 31:24 del componente `Parameter`
    /// (§6.4.2.6).
    pub fn puerto(&self) -> u8 {
        ((self.palabras[0] >> 24) & 0xFF) as u8
    }

    /// El hueco de dispositivo de un evento de hueco de dispositivo.
    ///
    /// El campo `Slot ID` esta en los bits 31:24 del componente `Parameter`
    /// (§6.4.6.5).
    pub fn hueco(&self) -> u8 {
        ((self.palabras[0] >> 24) & 0xFF) as u8
    }

    /// El hueco de dispositivo de un **evento**.
    ///
    /// En los TRB de evento, el campo `Slot ID` va en los bits 31:24 del
    /// componente `Control` (§6.4.6.2, tabla 6-64). Ojo: es al reves que en
    /// el evento de cambio de hueco, donde el hueco va en el `Parameter`.
    ///
    /// Comprueba: Linux (`TRB_TO_SLOT_ID`) y QEMU (`event->slotid << 24` en
    /// `xhci_write_event`) ponen y leen el hueco en el `Control`.
    pub fn slot_id(&self) -> u8 {
        ((self.palabras[3] >> 24) & 0xFF) as u8
    }

    /// El identificador de endpoint de un **evento** (bits 23:16 del control).
    ///
    /// El endpoint 0 de control es el endpoint 1 en la numeracion del
    /// controlador: el valor 0 esta reservado (§6.4.6.2).
    pub fn ep_id(&self) -> u8 {
        ((self.palabras[3] >> 16) & 0x1F) as u8
    }

    /// Fija el bit de ciclo del TRB.
    ///
    /// El productor del anillo pone el bit de ciclo en cada TRB que escribe:
    /// es la marca de que el TRB es suyo y el controlador todavia no lo ha
    /// tocado (§4.9.4).
    pub fn set_ciclo(&mut self, ciclo: bool) {
        if ciclo {
            self.palabras[3] |= TRB_CICLO_MASCARA;
        } else {
            self.palabras[3] &= !TRB_CICLO_MASCARA;
        }
    }
}

/// Tipos de TRB de comando/transferencia (§4.11.2, tabla 6-88).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TipoTrb {
    /// TRB normal (transferencia) - tipo 1
    Normal = 1,
    /// Setup Stage TRB - tipo 2
    SetupStage = 2,
    /// Data Stage TRB - tipo 3
    DataStage = 3,
    /// Status Stage TRB - tipo 4
    StatusStage = 4,
    /// Link TRB - tipo 6
    Link = 6,
    /// No-Op TRB (transferencia) - tipo 8
    NoOpTransfer = 8,
    /// Enable Slot Command - tipo 9
    EnableSlot = 9,
    /// Disable Slot Command - tipo 10
    DisableSlot = 10,
    /// Address Device Command - tipo 11
    AddressDevice = 11,
    /// Configure Endpoint Command - tipo 12
    ConfigureEndpoint = 12,
    /// Evaluate Context Command - tipo 13
    EvaluateContext = 13,
    /// Reset Endpoint Command - tipo 14
    ResetEndpoint = 14,
    /// Stop Endpoint Command - tipo 15
    StopEndpoint = 15,
    /// Set TR Dequeue Pointer Command - tipo 16
    SetTrDequeue = 16,
    /// Reset Device Command - tipo 17
    ResetDevice = 17,
    /// Force Event Command - tipo 18
    ForceEvent = 18,
    /// Negotiate Bandwidth Command - tipo 19
    NegotiateBandwidth = 19,
    /// Set Latency Tolerance Value Command - tipo 20
    SetLatencyTolerance = 20,
    /// Get Port Bandwidth Command - tipo 21
    GetPortBandwidth = 21,
    /// Force Header Command - tipo 22
    ForceHeader = 22,
    /// No-Op Command - tipo 23
    NoOpCommand = 23,
    /// Get Extended Property Command - tipo 24
    GetExtendedProperty = 24,
    /// Set Extended Property Command - tipo 25
    SetExtendedProperty = 25,
    /// TRB desconocido
    Desconocido = 0xFFFF,
}

impl TipoTrb {
    pub fn de_codigo(codigo: u32) -> TipoTrb {
        match codigo {
            1 => TipoTrb::Normal,
            2 => TipoTrb::SetupStage,
            3 => TipoTrb::DataStage,
            4 => TipoTrb::StatusStage,
            6 => TipoTrb::Link,
            8 => TipoTrb::NoOpTransfer,
            9 => TipoTrb::EnableSlot,
            10 => TipoTrb::DisableSlot,
            11 => TipoTrb::AddressDevice,
            12 => TipoTrb::ConfigureEndpoint,
            13 => TipoTrb::EvaluateContext,
            14 => TipoTrb::ResetEndpoint,
            15 => TipoTrb::StopEndpoint,
            16 => TipoTrb::SetTrDequeue,
            17 => TipoTrb::ResetDevice,
            18 => TipoTrb::ForceEvent,
            19 => TipoTrb::NegotiateBandwidth,
            20 => TipoTrb::SetLatencyTolerance,
            21 => TipoTrb::GetPortBandwidth,
            22 => TipoTrb::ForceHeader,
            23 => TipoTrb::NoOpCommand,
            24 => TipoTrb::GetExtendedProperty,
            25 => TipoTrb::SetExtendedProperty,
            _ => TipoTrb::Desconocido,
        }
    }
    pub fn codigo(self) -> u32 {
        self as u32
    }
}

/// Los tipos de TRB de evento (§4.11.3 y tabla 6-91).
///
/// Los valores son los identificadores de la tabla 6-91 del §6.4.6, que es la
/// tabla que define el campo `TRB Type` de los eventos ("The Event Type field
/// shall indicate Event Ring TRB Types as defined in Table 6-91", §4.11.3).
/// Ojo: no son 1, 2, 3 y 4 como en algunos numeros internos de otros
/// controladores: los eventos van del **32** al **39**.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipoEvento {
    /// Evento de transferencia (tipo 32).
    Transferencia,
    /// Evento de fin de comando (tipo 33).
    ComandoCompletado,
    /// Evento de cambio de estado de un puerto (tipo 34).
    CambioPuerto,
    /// Evento de solicitud de ancho de banda (tipo 35).
    SolicitudAnchoBanda,
    /// Evento de puerta (tipo 36).
    EventoPuerta,
    /// Evento del controlador de host (tipo 37).
    ControladorHost,
    /// Evento de notificacion de dispositivo (tipo 38).
    NotificacionDispositivo,
    /// Evento de vuelta del indice de microframes (tipo 39).
    VueltaMicroframes,
    /// Un tipo que esta especificacion no conoce.
    Desconocido(u32),
}

impl TipoEvento {
    /// Traduce el valor del campo de tipo de TRB.
    pub fn de_codigo(codigo: u32) -> TipoEvento {
        match codigo {
            32 => TipoEvento::Transferencia,
            33 => TipoEvento::ComandoCompletado,
            34 => TipoEvento::CambioPuerto,
            35 => TipoEvento::SolicitudAnchoBanda,
            36 => TipoEvento::EventoPuerta,
            37 => TipoEvento::ControladorHost,
            38 => TipoEvento::NotificacionDispositivo,
            39 => TipoEvento::VueltaMicroframes,
            otro => TipoEvento::Desconocido(otro),
        }
    }

    /// Nombre corto, para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            TipoEvento::Transferencia => "transferencia",
            TipoEvento::ComandoCompletado => "fin de comando",
            TipoEvento::CambioPuerto => "cambio de puerto",
            TipoEvento::SolicitudAnchoBanda => "solicitud de ancho de banda",
            TipoEvento::EventoPuerta => "puerta",
            TipoEvento::ControladorHost => "controlador de host",
            TipoEvento::NotificacionDispositivo => "notificacion de dispositivo",
            TipoEvento::VueltaMicroframes => "vuelta de microframes",
            TipoEvento::Desconocido(_) => "desconocido",
        }
    }
}

/// Codigo de finalizacion "correcto" de un evento de fin de comando.
pub const FINALIZACION_CORRECTA: u32 = 1;

/// Codigo de finalizacion "babble": el dispositivo ha enviado mas datos de los
/// que caben en el buffer (§6.4.6.3, tabla 6-60).
pub const FINALIZACION_ERROR_BABBLE: u32 = 3;
/// Codigo de finalizacion "error de transaccion": el dispositivo no ha
/// contestado (§6.4.6.3, tabla 6-60).
pub const FINALIZACION_ERROR_TRANSACCION: u32 = 4;
/// Codigo de finalizacion "stall": el endpoint del dispositivo se ha
/// quedado colgado (§6.4.6.3, tabla 6-60).
pub const FINALIZACION_ERROR_STALL: u32 = 6;
/// Codigo de finalizacion "paquete corto": el dispositivo ha devuelto menos
/// bytes de los que se le pidieron (§6.4.6.3, tabla 6-60).
///
/// No es un error: el resto de los bytes los dice el campo `residual` del
/// evento.
pub const FINALIZACION_PAQUETE_CORTO: u32 = 13;

/// Mascara del campo `residual` de un evento de transferencia.
///
/// El residual es lo que **no** se transfirio: va en los bits 23:0 del
/// componente de estado del evento, y el codigo de finalizacion en los bits
/// 31:24 (§6.4.6.2). Linux: `EVENT_TRB_LEN(p) = p & 0xffffff`.
pub const RESIDUAL_MASCARA: u32 = 0x00FF_FFFF;

/// Memoria donde se pueden leer TRB.
///
/// Es una abstraccion para que el vaciado del anillo de eventos se pueda
/// probar en el anfitrion sin un controlador de verdad.
pub trait MemoriaTrb {
    /// Lee el TRB de la direccion `addr`. Devuelve `None` si no se puede leer.
    fn leer_trb(&self, addr: u64) -> Option<Trb>;
}

/// El anillo de eventos: un segmento y el puntero de extraccion.
///
/// Solo se implementa el caso de un unico segmento, que es el que necesita
/// la fase 2. La tabla de segmentos permite mas, pero no hace falta para
/// sonenciar un teclado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnilloEvento {
    /// Direccion fisica base del segmento (alineada a 64 bytes, §6.5).
    pub base: u64,
    /// Numero de TRBs del segmento.
    pub trbs: u16,
    /// Direccion del proximo TRB que hay que mirar.
    pub dequeue: u64,
    /// Estado de ciclo del consumidor (CCS). Empieza a uno.
    pub ciclo: bool,
}

impl AnilloEvento {
    /// Crea el anillo sobre un segmento ya preparado (con los TRBs a cero).
    ///
    /// El puntero de extraccion empieza en el principio del segmento y el
    /// estado de ciclo del consumidor en uno, como manda §4.9.4.
    pub fn nuevo(base: u64, trbs: u16) -> AnilloEvento {
        AnilloEvento {
            base,
            trbs,
            dequeue: base,
            ciclo: true,
        }
    }

    /// Bytes que ocupa el segmento.
    pub fn bytes(&self) -> u64 {
        self.trbs as u64 * TAM_TRB
    }

    /// El siguiente evento del anillo, si lo hay.
    ///
    /// Devuelve `None` cuando el bit de ciclo del TRB no coincide con el
    /// estado de ciclo del consumidor: el anillo esta vacio.
    pub fn siguiente(&self, mem: &impl MemoriaTrb) -> Option<Trb> {
        let trb = mem.leer_trb(self.dequeue)?;
        if trb.ciclo() != self.ciclo {
            return None;
        }
        Some(trb)
    }

    /// Avanza el puntero de extraccion un TRB.
    ///
    /// Al llegar al final del segmento vuelve al principio y da la vuelta al
    /// bit de ciclo del consumidor.
    pub fn avanzar(&mut self) {
        self.dequeue += TAM_TRB;
        if self.dequeue >= self.base + self.bytes() {
            self.dequeue = self.base;
            self.ciclo = !self.ciclo;
        }
    }

    /// Valor que hay que escribir en `ERDP` (§5.5.2.3.3).
    ///
    /// El puntero de extraccion vive en los bits 63:4: los cuatro bits bajos
    /// son de control (`DESI` en 2:0 y `EHB` en 3), asi que se ponen a cero.
    ///
    /// Ojo: el puntero no esta alineado a 64 bytes sino a 16 (el tamano de un
    /// TRB), asi que **no** se puede mascar con `!0x3F`: el bit 5 forma parte
    /// de la direccion y hay que conservarlo.
    pub fn erdp(&self) -> u64 {
        self.dequeue & !0xFu64
    }
}

/// Bit `EHB` del puntero de extraccion del anillo de eventos (§5.5.2.3.3).
///
/// Es de escribir-1-para-limpiar: el controlador lo pone a uno cuando deja
/// un evento, y el software lo limpia escribiendo un uno al actualizar el
/// puntero. Sin limpiarlo, el controlador cree que el software sigue con el
/// anillo a medias.
pub const ERDP_EHB: u64 = 1 << 3;

/// El anillo de comandos: un segmento y el puntero de produccion.
///
/// El anillo de comandos lo escribe el software (productor) y lo lee el
/// controlador (consumidor). Cada parte lleva su propio estado de ciclo: el
/// del software va en el bit 0 de cada TRB que escribe, y el del controlador
/// en el bit `RCS` del registro `CRCR` (§4.6.4, §5.4.5).
///
/// El estado de ciclo del productor empieza a **uno**, igual que el `RCS` que
/// se escribe al programar el controlador: por eso el primer TRB que se
/// escribe lleva el bit de ciclo a uno.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnilloComando {
    /// Direccion fisica base del segmento (alineada a 64 bytes, §6.5).
    pub base: u64,
    /// Numero de TRBs del segmento. Potencia de dos y al menos 16 (§4.6.4).
    pub trbs: u16,
    /// Indice, en TRBs, del proximo TRB que se va a escribir.
    pub indice: u16,
    /// Estado de ciclo del productor (PCS). Empieza a uno.
    pub ciclo: bool,
}

impl AnilloComando {
    /// Crea el anillo sobre un segmento ya preparado (con los TRBs a cero).
    ///
    /// Devuelve `None` si el tamano no cumple lo que pide la especificacion:
    /// una potencia de dos de al menos 16 TRBs. Un anillo maldimensionado haria
    /// que el controlador leyera basura, asi que mejor no arrancar.
    pub fn nuevo(base: u64, trbs: u16) -> Option<AnilloComando> {
        if trbs < 16 || !trbs.is_power_of_two() {
            return None;
        }
        Some(AnilloComando {
            base,
            trbs,
            indice: 0,
            ciclo: true,
        })
    }

    /// Bytes que ocupa el segmento.
    pub fn bytes(&self) -> u64 {
        self.trbs as u64 * TAM_TRB
    }

    /// Reserva el siguiente TRB del anillo.
    ///
    /// Devuelve la direccion donde escribir el TRB y el valor que debe llevar
    /// el bit de ciclo. Al llegar al final del segmento vuelve al principio y
    /// da la vuelta al estado de ciclo del productor: los TRBs de la segunda
    /// vuelta llevan el bit a cero, para que el controlador sepa que son
    /// nuevos y no los que ya leyo.
    pub fn siguiente(&mut self) -> (u64, bool) {
        let addr = self.base + self.indice as u64 * TAM_TRB;
        let ciclo = self.ciclo;
        self.indice += 1;
        if self.indice >= self.trbs {
            self.indice = 0;
            self.ciclo = !self.ciclo;
        }
        (addr, ciclo)
    }

    /// Indice, en TRBs, del proximo TRB del anillo.
    ///
    /// Es el "Next Command TRB Pointer" que la especificacion describe (§4.6.4).
    pub fn indice_siguiente(&self) -> u16 {
        self.indice
    }

    /// ¿Queda sitio en el anillo para un TRB mas?
    ///
    /// El anillo de comandos no tiene un bit de "lleno": el productor no puede
    /// avanzar mas de una vuelta por delante del consumidor, asi que la
    /// comprobacion de hueco la hace el controlador (bit `CRR` del `CRCR`).
    /// Aqui solo se avisa de que el anillo esta en uso.
    pub fn en_uso(&self) -> bool {
        self.indice != 0 || !self.ciclo
    }
}

/// El anillo de transferencia de un endpoint.
///
/// El software lo escribe (productor) y el controlador lo lee (consumidor).
/// Cada parte lleva su propio estado de ciclo: el del software va en el bit 0
/// de cada TRB que escribe, y el del controlador en el bit `DCS` del contexto
/// de endpoint (§4.7.2.3).
///
/// ## El TRB Link
///
/// El segmento termina con un **TRB Link** que vuelve al principio. Sin el,
/// el controlador no tendria forma de saber que el anillo se ha dado la
/// vuelta: su estado de ciclo no cambiaria y los TRBs de la segunda vuelta se
/// tomarian por viejos (§4.7.3, §4.7.4.3).
///
/// El TRB Link **no** es un TRB util: el productor nunca lo escribe con datos,
/// solo lo reescribe para cambiarle el bit de ciclo.
///
/// ## El ciclo del TRB Link
///
/// El controlador comprueba el bit de ciclo del Link igual que el de
/// cualquier otro TRB: cuando llega al final del segmento, compara el bit del
/// Link con su propio estado de ciclo. Si no coinciden, el anillo le parece
/// vacio.
///
/// Por eso el Link lleva el ciclo de la vuelta que **termina**, que es la que
/// el controlador esta recorriendo:
///
/// - La primera vuelta: el Link se escribe con el ciclo uno, como los TRBs
///   utiles de esa vuelta.
/// - Al cruzar el Link, el Link se reescribe con el ciclo **anterior** (el de
///   la vuelta que acaba de terminar), que es justo el que va a leer el
///   controlador.
///
/// Es lo mismo que hace Linux: al pasar por el Link, `inc_enq_past_link`
/// invierte el bit de ciclo del Link (`link.control ^= TRB_CYCLE`) y, si el
/// Link tiene el bit `TC`, da la vuelta al estado de ciclo del anillo.
///
/// ## No hay carrera
///
/// El productor cruza el Link cuando escribe el **ultimo** TRB util del
/// segmento, y reescribe el Link antes de tocar la campana. El controlador
/// solo puede llegar al Link despues de consumir ese ultimo TRB util, y para
/// consumirlo hace falta la campana: el Link esta escrito antes de que el
/// controlador pueda leerlo.
///
/// ## El TRB Link no se puede partir
///
/// Una tanda de control son varios TRB seguidos (setup, datos y estado). Si
/// esa tanda llegase al Link TRB, el controlador leeria la segunda parte en la
/// vuelta anterior. Por eso [`AnilloTransferencia::cabe`] dice si la tanda
/// cabe entera antes de que el producto reserve sitio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnilloTransferencia {
    /// Direccion fisica base del segmento (alineada a 16 bytes).
    pub base: u64,
    /// Numero de TRBs del segmento, **Link TRB incluido**.
    ///
    /// Potencia de dos y entre 16 y 4096 (§4.7.3).
    pub trbs: u16,
    /// Indice del proximo TRB util que se va a escribir.
    pub indice: u16,
    /// Estado de ciclo del productor (PCS). Empieza a uno.
    pub ciclo: bool,
    /// Si la ultima reserva dio la vuelta del segmento.
    pub dio_la_vuelta: bool,
}

impl AnilloTransferencia {
    /// Crea el anillo sobre un segmento ya preparado (con los TRBs a cero).
    ///
    /// Devuelve `None` si el tamano no cumple lo que pide la especificacion:
    /// una potencia de dos entre 16 y 4096 TRBs. Un anillo maldimensionado
    /// haria que el controlador leyera basura, asi que mejor no arrancar.
    pub fn nuevo(base: u64, trbs: u16) -> Option<AnilloTransferencia> {
        if trbs < 16 || trbs > 4096 || !trbs.is_power_of_two() {
            return None;
        }
        Some(AnilloTransferencia {
            base,
            trbs,
            indice: 0,
            ciclo: true,
            dio_la_vuelta: false,
        })
    }

    /// Cuantos TRBs **utiles** tiene el segmento (sin contar el Link).
    pub fn utiles(&self) -> u16 {
        self.trbs - 1
    }

    /// Bytes que ocupa el segmento.
    pub fn bytes(&self) -> u64 {
        self.trbs as u64 * TAM_TRB
    }

    /// El indice del TRB Link: el ultimo del segmento.
    pub fn indice_link(&self) -> u16 {
        self.trbs - 1
    }

    /// La direccion del TRB Link.
    pub fn direccion_link(&self) -> u64 {
        self.base + self.indice_link() as u64 * TAM_TRB
    }

    /// Reserva el siguiente TRB util.
    ///
    /// Devuelve la direccion donde escribir el TRB y el valor que debe llevar
    /// el bit de ciclo. Al llegar al final de los TRBs utiles vuelve al
    /// principio y da la vuelta al estado de ciclo del productor: los TRBs de
    /// la segunda vuelta llevan el bit a cero, para que el controlador sepa que
    /// son nuevos y no los que ya leyo.
    ///
    /// El Link TRB no se reserva nunca: el productor solo lo reescribe.
    pub fn siguiente(&mut self) -> (u64, bool) {
        let direccion = self.base + self.indice as u64 * TAM_TRB;
        let ciclo = self.ciclo;
        self.dio_la_vuelta = false;
        self.indice += 1;
        if self.indice >= self.utiles() {
            self.indice = 0;
            self.ciclo = !self.ciclo;
            self.dio_la_vuelta = true;
        }
        (direccion, ciclo)
    }

    /// ¿Cabe una tanda de `n` TRBs antes de que el anillo de la vuelta?
    ///
    /// El productor no puede partir una tanda entre el Link TRB y el principio
    /// del segmento: el controlador leeria la segunda parte en la vuelta
    /// anterior. Si no cabe, hay que dejar el anillo para otra tanda.
    pub fn cabe(&self, n: u16) -> bool {
        self.indice
            .checked_add(n)
            .map(|i| i <= self.utiles())
            .unwrap_or(false)
    }
}

/// Codifica una entrada de la tabla de segmentos del anillo de eventos (§6.5).
///
/// La entrada son cuatro dwords:
///
/// - dword 0: bits 31:6 de la direccion base del segmento (alineada a 64
///   bytes); los bits 5:0 estan reservados y valen cero.
/// - dword 1: bits 63:32 de la direccion base.
/// - dword 2: bits 15:0, numero de TRBs del segmento (de 16 a 4096).
/// - dword 3: reservado.
///
/// Ojo: el numero de TRBs **no** va en los bits bajos de la direccion: es un
/// dword aparte (§6.5, tablas 6-94 y 6-95).
pub fn erst_entrada(base: u64, trbs: u16) -> [u32; 4] {
    let dir = base & !0x3F;
    [dir as u32, (dir >> 32) as u32, (trbs as u32) & 0xFFFF, 0]
}

/// Codifica el puntero a una entrada del array de contextos base (§6.1).
///
/// Los 6 bits bajos estan reservados: el contexto de dispositivo tiene que
/// estar alineado a 64 bytes.
pub fn dcbaa_entrada(base: u64) -> u64 {
    base & !0x3F
}

/// Codifica el registro de control del anillo de comandos (`CRCR`, §5.4.5).
///
/// - bit 0: estado de ciclo del anillo (`RCS`), a uno.
/// - bit 1: el anillo esta parado (`CS`), de escribir-1-para-limpiar.
/// - bit 2: el comando en curso se aborta (`CA`), de escribir-1-para-limpiar.
/// - bit 3: el anillo de comandos se esta ejecutando (`CRR`), de solo lectura.
/// - bits 63:6: direccion base del anillo.
///
/// Ojo: **no hay ningun bit "run"** en este registro. `CS` y `CA` son
/// banderas de escribir-1-para-limpiar: escribir un uno en ellas para el
/// anillo o aborta el comando en curso. El anillo de comandos solo corre
/// cuando el controlador esta en marcha (`USBCMD.R/S`) **y** se toca la
/// campana del controlador de host.
pub fn crcr(base: u64) -> u64 {
    (base & !0x3F) | 1
}

/// Bit de ciclo para TRBs.
pub const CICLO: u32 = 1;
/// Bit de cadena (Chain bit) para TRBs de transferencia/comando según uso.
pub const CADENA: u32 = 1 << 4;
/// Bit de interrupción al completar (IOC), bit 5 del control.
///
/// Linux: `TRB_IOC (1<<5)`. U-Boot: `TRB_IOC (1<<5)`. iPXE:
/// `XHCI_TRB_IOC 0x20`. QEMU: `TRB_TR_IOC (1<<5)`. Las cuatro
/// implementaciones coinciden, asi que el bit 5 es el bueno.
pub const IOC: u32 = 1 << 5;
/// Bit de interrupcion inmediata (IDT), bit 6 del control.
///
/// Con IDT a uno, los datos viajan en el propio TRB (8 bytes como maximo) y no
/// hace falta un Data Stage TRB con buffer.
pub const IDT: u32 = 1 << 6;
/// Bit de interrupcion en paquete corto (ISP), bit 2 del control.
///
/// Solo tiene sentido en TRBs normales; obliga al controlador a generar un
/// evento aunque el paquete sea mas corto de lo pedido.
pub const ISP: u32 = 1 << 2;
/// Bit de no-sniffer (NS), bit 3 del control.
pub const NS: u32 = 1 << 3;
/// Bit de direccion (control), bit 16. En un Data Stage o Status Stage vale 1
/// cuando la direccion es IN.
pub const DIR_ENTRADA: u32 = 1 << 16;
/// Tipo de transferencia (TRT) de un Setup Stage TRB: sin etapa de datos.
pub const TRT_SIN_DATOS: u32 = 0;
/// Tipo de transferencia (TRT) de un Setup Stage TRB: etapa de datos OUT.
pub const TRT_DATOS_OUT: u32 = 2;
/// Tipo de transferencia (TRT) de un Setup Stage TRB: etapa de datos IN.
pub const TRT_DATOS_IN: u32 = 3;
/// Bit de evaluacion de contexto (BSS).
pub const BSR: u32 = 1 << 9;
/// Block Set Address Request (BSR) - mismo bit, diferente nombre
pub const TRB_BSR: u32 = 1 << 9;

/// Mascara del campo `TRB Transfer Length` (bits 21:0 del **estado**).
///
/// Ojo: la longitud **no** va en el control. Linux la escribe en el estado
/// (`trb->status = cpu_to_le32(TRB_LEN(trb_len) | ...)`) y QEMU la lee
/// igual (`chunk = trb->status & 0x1ffff`).
pub const TRB_LONGITUD_MASCARA: u32 = 0x001F_FFFF;

/// Construye un TRB Link.
pub fn trb_link(segmento_siguiente: u64, toggle_cycle: bool) -> Trb {
    let mut ctrl = (TipoTrb::Link.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    if toggle_cycle {
        ctrl |= 1 << 1; // TC
    }
    Trb::nueva([
        segmento_siguiente as u32,
        (segmento_siguiente >> 32) as u32,
        0,
        ctrl,
    ])
}

/// TRB Enable Slot Command.
pub fn trb_enable_slot(_slot_type: u8) -> Trb {
    let ctrl = (TipoTrb::EnableSlot.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    Trb::nueva([0, 0, 0, ctrl])
}

/// TRB Disable Slot Command.
pub fn trb_disable_slot(slot_id: u8) -> Trb {
    let ctrl = (TipoTrb::DisableSlot.codigo() << TRB_TIPO_DESPLAZAMIENTO)
        | CICLO
        | ((slot_id as u32) << 24);
    Trb::nueva([0, 0, 0, ctrl])
}

/// TRB Address Device Command.
pub fn trb_address_device(input_context_ptr: u64, slot_id: u8, bsr: bool) -> Trb {
    let mut ctrl = (TipoTrb::AddressDevice.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    if bsr {
        ctrl |= BSR;
    }
    ctrl |= (slot_id as u32) << 24;
    Trb::nueva([
        input_context_ptr as u32,
        (input_context_ptr >> 32) as u32,
        0,
        ctrl,
    ])
}

/// TRB Configure Endpoint Command.
pub fn trb_configure_endpoint(input_context_ptr: u64, slot_id: u8, deconfigure: bool) -> Trb {
    let mut ctrl = (TipoTrb::ConfigureEndpoint.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    if deconfigure {
        ctrl |= 1 << 9; // DC
    }
    ctrl |= (slot_id as u32) << 24;
    Trb::nueva([
        input_context_ptr as u32,
        (input_context_ptr >> 32) as u32,
        0,
        ctrl,
    ])
}

/// TRB Evaluate Context Command.
pub fn trb_evaluate_context(input_context_ptr: u64, slot_id: u8) -> Trb {
    let ctrl = (TipoTrb::EvaluateContext.codigo() << TRB_TIPO_DESPLAZAMIENTO)
        | CICLO
        | ((slot_id as u32) << 24);
    Trb::nueva([
        input_context_ptr as u32,
        (input_context_ptr >> 32) as u32,
        0,
        ctrl,
    ])
}

/// TRB Reset Endpoint Command.
pub fn trb_reset_endpoint(endpoint_id: u8, slot_id: u8) -> Trb {
    let mut ctrl = (TipoTrb::ResetEndpoint.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    ctrl |= ((endpoint_id as u32) & 0x1F) << 16;
    ctrl |= (slot_id as u32) << 24;
    Trb::nueva([0, 0, 0, ctrl])
}

/// TRB Stop Endpoint Command.
pub fn trb_stop_endpoint(endpoint_id: u8, slot_id: u8, suspend: bool) -> Trb {
    let mut ctrl = (TipoTrb::StopEndpoint.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    if suspend {
        ctrl |= 1 << 23;
    }
    ctrl |= ((endpoint_id as u32) & 0x1F) << 16;
    ctrl |= (slot_id as u32) << 24;
    Trb::nueva([0, 0, 0, ctrl])
}

/// TRB Set TR Dequeue Pointer Command.
pub fn trb_set_tr_dequeue(
    dequeue_ptr: u64,
    stream_id: u16,
    endpoint_id: u8,
    slot_id: u8,
    cycle: bool,
) -> Trb {
    let mut ctrl = (TipoTrb::SetTrDequeue.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    ctrl |= ((endpoint_id as u32) & 0x1F) << 16;
    ctrl |= (slot_id as u32) << 24;
    let mut palabras = [
        dequeue_ptr as u32,
        (dequeue_ptr >> 32) as u32,
        stream_id as u32,
        ctrl,
    ];
    if cycle {
        palabras[0] = (palabras[0] & !1) | 1;
    }
    Trb::nueva(palabras)
}

/// TRB No-Op Command.
pub fn trb_noop_comando() -> Trb {
    Trb::nueva([
        0,
        0,
        0,
        (TipoTrb::NoOpCommand.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO,
    ])
}

/// TRB No-Op Transfer.
pub fn trb_noop_transfer(entropy: u32) -> Trb {
    Trb::nueva([
        0,
        0,
        entropy,
        (TipoTrb::NoOpTransfer.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO,
    ])
}

/// Setup Stage TRB (control).
///
/// El campo `TRT` (tipo de transferencia) va en los bits 17:16 del control y
/// dice si habra etapa de datos: 2 = OUT, 3 = IN, 0 = sin datos (§6.4.1.2.1,
/// tabla 6-23). El estado lleva la longitud de la transferencia, que para el
/// setup siempre son 8 bytes (tabla 6-25).
///
/// El bit `IDT` (datos inmediatos) va a **uno**: el paquete de setup de ocho
/// bytes viaja dentro del propio TRB, no en un buffer (§6.4.1.2.1). Linux hace
/// lo mismo (`TRB_IDT | TRB_TYPE(TRB_SETUP)`) y QEMU lo **exige**: si el bit
/// no esta puesto, `xhci_fire_ctl_transfer` rechaza la transferencia entera.
///
/// Ojo: el bit 3 del control esta reservado (Linux lo llama `NS`, no-snoop)
/// y debe valer cero.
///
/// El bit `IOC` no lo pone el setup: en una transferencia de control el evento
/// lo genera el **Status Stage TRB**, que es el ultimo de la tanda. Linux
/// tampoco lo pone (§`xhci_queue_ctrl_tx`).
pub fn trb_setup_stage(
    bm_request_type: u8,
    b_request: u8,
    w_value: u16,
    w_index: u16,
    w_length: u16,
    transfer_type: u32,
    ioc: bool,
) -> Trb {
    let param: u64 = (bm_request_type as u64)
        | ((b_request as u64) << 8)
        | ((w_value as u64) << 16)
        | ((w_index as u64) << 32)
        | ((w_length as u64) << 48);
    let mut ctrl = (TipoTrb::SetupStage.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO | IDT;
    ctrl |= (transfer_type & 0x3) << 16;
    if ioc {
        ctrl |= IOC;
    }
    Trb::nueva([param as u32, (param >> 32) as u32, 8, ctrl])
}

/// Data Stage TRB.
///
/// La longitud va en el **estado** (bits 21:0), no en el control, y la
/// direccion en el bit 16 del control.
///
/// El bit `ISP` (interrupcion en paquete corto) solo se pone en las
/// **entradas**: obliga al controlador a avisar cuando el dispositivo devuelve
/// menos bytes de los pedidos, que es justo el dato que hace falta para saber
/// cuantos se han transferido. Linux hace lo mismo: `TRB_ISP | TRB_TYPE(TRB_DATA)`
/// para una entrada y `TRB_TYPE(TRB_DATA)` para una salida.
///
/// El bit `IOC` tampoco se pone aqui (Linux tampoco): el evento de la tanda de
/// control lo genera el Status Stage TRB.
pub fn trb_data_stage(
    buffer_ptr: u64,
    length: u32,
    dir_in: bool,
    isp: bool,
    chain: bool,
    ioc: bool,
) -> Trb {
    let mut ctrl = (TipoTrb::DataStage.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    if dir_in {
        ctrl |= DIR_ENTRADA;
    }
    if isp {
        ctrl |= ISP;
    }
    if chain {
        ctrl |= CADENA;
    }
    if ioc {
        ctrl |= IOC;
    }
    let status = length & TRB_LONGITUD_MASCARA;
    Trb::nueva([buffer_ptr as u32, (buffer_ptr >> 32) as u32, status, ctrl])
}

/// Status Stage TRB.
///
/// La direccion va en el bit 16 del control: vale 1 (IN) cuando la
/// transferencia **no** ha tenido etapa de datos de entrada.
///
/// En una transferencia de control, la etapa de estado es siempre una
/// entrada, salvo que la etapa de datos haya sido una entrada: en ese caso la
/// etapa de estado es una salida (el dispositivo devuelve el estado, que son
/// cero bytes, y el host confirma). Linux: `if (transfer_buffer_length > 0 &&
/// setup->bRequestType & USB_DIR_IN) field = 0; else field = TRB_DIR_IN;`.
///
/// El bit `IOC` va a **uno**: este TRB es el ultimo de la tanda y el que
/// genera el evento de transferencia.
pub fn trb_status_stage(dir_in: bool, chain: bool, ioc: bool) -> Trb {
    let mut ctrl = (TipoTrb::StatusStage.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    if dir_in {
        ctrl |= DIR_ENTRADA;
    }
    if chain {
        ctrl |= CADENA;
    }
    if ioc {
        ctrl |= IOC;
    }
    Trb::nueva([0, 0, 0, ctrl])
}

/// Normal TRB para bulk/interrupt.
///
/// La longitud va en el **estado** (bits 21:0) y el interrupter objetivo
/// tambien, en los bits 31:22 del estado (Linux `TRB_INTR_TARGET`,
/// QEMU `TRB_INTR`).
pub fn trb_normal(buffer_ptr: u64, length: u32, interrupter: u16, chain: bool, ioc: bool) -> Trb {
    let mut ctrl = (TipoTrb::Normal.codigo() << TRB_TIPO_DESPLAZAMIENTO) | CICLO;
    if chain {
        ctrl |= CADENA;
    }
    if ioc {
        ctrl |= IOC;
    }
    let status = (length & TRB_LONGITUD_MASCARA) | ((interrupter as u32 & 0x3FF) << 22);
    Trb::nueva([buffer_ptr as u32, (buffer_ptr >> 32) as u32, status, ctrl])
}

#[cfg(test)]
mod tests {
    use super::*;
    // La libreria es `no_std` (va dentro del kernel), pero las pruebas si
    // pueden usar la biblioteca estandar.
    extern crate std;
    use std::vec::Vec;

    /// Memoria de mentira: un mapa de TRBs.
    struct MemoriaFalsa {
        trbs: Vec<(u64, Trb)>,
    }

    impl MemoriaFalsa {
        fn nuevo() -> MemoriaFalsa {
            MemoriaFalsa { trbs: Vec::new() }
        }

        fn con(mut self, addr: u64, trb: Trb) -> MemoriaFalsa {
            self.trbs.push((addr, trb));
            self
        }
    }

    impl MemoriaTrb for MemoriaFalsa {
        fn leer_trb(&self, addr: u64) -> Option<Trb> {
            self.trbs.iter().find(|(a, _)| *a == addr).map(|(_, t)| *t)
        }
    }

    /// Un TRB de evento con el bit de ciclo a uno.
    fn trb_evento(tipo: u32) -> Trb {
        Trb::nueva([
            0,
            0,
            0,
            (tipo << TRB_TIPO_DESPLAZAMIENTO) | TRB_CICLO_MASCARA,
        ])
    }

    #[test]
    fn lee_el_tipo_y_el_ciclo() {
        // Tipo 34: evento de cambio de estado de un puerto (tabla 6-91).
        let t = trb_evento(34);
        assert_eq!(t.tipo(), 34);
        assert!(t.ciclo());
        assert_eq!(t.control(), (34 << 10) | 1);
    }

    #[test]
    fn el_tipo_de_trb_ocupa_los_bits_15_10() {
        // El campo `TRB Type` son seis bits (15:10). Con una mascara mas
        // corta el tipo se leeria mal: los eventos van del 32 al 39 y el bit
        // 15 tiene que estar dentro de la mascara.
        let t = Trb::nueva([0, 0, 0, 0x8801]);
        assert_eq!(t.tipo(), 34);
        // Un tipo de comando (9 = Enable Slot) tambien sale bien.
        let t = Trb::nueva([0, 0, 0, (9 << 10) | 1]);
        assert_eq!(t.tipo(), 9);
        // Los bits de abajo (cadena, interrupcion...) no se colan en el tipo.
        let t = Trb::nueva([0, 0, 0, (34 << 10) | 0x3FF]);
        assert_eq!(t.tipo(), 34);
    }

    #[test]
    fn decodifica_los_tipos_de_evento_de_la_tabla_6_91() {
        assert_eq!(TipoEvento::de_codigo(32), TipoEvento::Transferencia);
        assert_eq!(TipoEvento::de_codigo(33), TipoEvento::ComandoCompletado);
        assert_eq!(TipoEvento::de_codigo(34), TipoEvento::CambioPuerto);
        assert_eq!(TipoEvento::de_codigo(35), TipoEvento::SolicitudAnchoBanda);
        assert_eq!(TipoEvento::de_codigo(36), TipoEvento::EventoPuerta);
        assert_eq!(TipoEvento::de_codigo(37), TipoEvento::ControladorHost);
        assert_eq!(
            TipoEvento::de_codigo(38),
            TipoEvento::NotificacionDispositivo
        );
        assert_eq!(TipoEvento::de_codigo(39), TipoEvento::VueltaMicroframes);
        // Lo que no esta en la tabla se marca como desconocido.
        assert_eq!(TipoEvento::de_codigo(31), TipoEvento::Desconocido(31));
        assert_eq!(TipoEvento::de_codigo(40), TipoEvento::Desconocido(40));
    }

    #[test]
    fn lee_el_puerto_de_un_evento_de_cambio() {
        // Port ID en los bits 31:24 del Parameter.
        let t = Trb::nueva([0x0500_0000, 0, 0, 34 << 10]);
        assert_eq!(t.puerto(), 5);
    }

    #[test]
    fn el_anillo_ignora_el_hueco_del_ciclo_anterior() {
        let mut mem = MemoriaFalsa::nuevo();
        // Direccion del hueco: bit de ciclo a cero.
        mem = mem.con(0x1000, Trb::nueva([0, 0, 0, 34 << 10]));
        let anillo = AnilloEvento::nuevo(0x1000, 16);
        assert!(anillo.siguiente(&mem).is_none());
    }

    #[test]
    fn el_anillo_entrega_el_evento_y_avanza() {
        let mut mem = MemoriaFalsa::nuevo();
        mem = mem.con(0x1000, trb_evento(34));
        mem = mem.con(0x1010, trb_evento(37));
        let mut anillo = AnilloEvento::nuevo(0x1000, 16);

        let t = anillo.siguiente(&mem).expect("evento");
        assert_eq!(t.tipo(), 34);
        anillo.avanzar();
        assert_eq!(anillo.dequeue, 0x1010);

        let t = anillo.siguiente(&mem).expect("evento");
        assert_eq!(t.tipo(), 37);
        anillo.avanzar();
        assert_eq!(anillo.dequeue, 0x1020);
        assert!(anillo.siguiente(&mem).is_none());
    }

    #[test]
    fn al_dar_la_vuelta_cambia_el_ciclo_del_consumidor() {
        let mut mem = MemoriaFalsa::nuevo();
        // Un segmento de un solo TRB.
        mem = mem.con(0x1000, trb_evento(34));
        let mut anillo = AnilloEvento::nuevo(0x1000, 1);
        assert!(anillo.ciclo);
        assert!(anillo.siguiente(&mem).is_some());
        anillo.avanzar();
        assert_eq!(anillo.dequeue, 0x1000);
        assert!(!anillo.ciclo);
        // Con el ciclo del consumidor a cero, el TRB viejo ya no vale.
        assert!(anillo.siguiente(&mem).is_none());
    }

    #[test]
    fn codifica_las_entradas_de_la_tabla_de_segmentos() {
        // La direccion va en los dos primeros dwords (con los 6 bits bajos
        // reservados a cero) y el numero de TRBs en el tercero.
        assert_eq!(erst_entrada(0x1234, 256), [0x1200, 0, 256, 0]);
        assert_eq!(erst_entrada(0x1000, 16), [0x1000, 0, 16, 0]);
        // Una direccion de 64 bits se parte en dos dwords.
        assert_eq!(
            erst_entrada(0x1_0000_1234, 16),
            [0x1200, 0x0000_0001, 16, 0]
        );
    }

    #[test]
    fn codifica_el_puntero_de_contextos() {
        assert_eq!(dcbaa_entrada(0x2001), 0x2000);
    }

    #[test]
    fn codifica_el_control_del_anillo_de_comandos() {
        // Solo se pone el estado de ciclo: los bits 1 y 2 son banderas de
        // escribir-1-para-limpiar que paran el anillo de comandos.
        assert_eq!(crcr(0x3004), 0x3000 | 1);
    }

    #[test]
    fn el_puntero_de_extraccion_conserva_el_bit_5() {
        // El puntero de extraccion esta alineado a 16 bytes (el tamano de un
        // TRB), no a 64: el tercer TRB del segmento vive en base + 32.
        let mut anillo = AnilloEvento::nuevo(0x1000, 16);
        anillo.avanzar();
        anillo.avanzar();
        assert_eq!(anillo.dequeue, 0x1020);
        assert_eq!(anillo.erdp(), 0x1020);
    }

    #[test]
    fn el_anillo_de_transferencia_rechaza_los_tamanos_malos() {
        // Ni menos de 16 TRBs, ni mas de 4096, ni una potencia de dos que no
        // sea: un segmento maldimensionado haria que el controlador leyera
        // basura.
        assert!(AnilloTransferencia::nuevo(0x1000, 0).is_none());
        assert!(AnilloTransferencia::nuevo(0x1000, 15).is_none());
        assert!(AnilloTransferencia::nuevo(0x1000, 24).is_none());
        assert!(AnilloTransferencia::nuevo(0x1000, 8192).is_none());
        assert!(AnilloTransferencia::nuevo(0x1000, 16).is_some());
        assert!(AnilloTransferencia::nuevo(0x1000, 64).is_some());
        assert!(AnilloTransferencia::nuevo(0x1000, 4096).is_some());
    }

    #[test]
    fn el_anillo_de_transferencia_no_reserva_el_link() {
        let mut anillo = AnilloTransferencia::nuevo(0x1000, 16).expect("anillo");
        // 16 TRBs, uno de ellos el Link: 15 utiles.
        assert_eq!(anillo.utiles(), 15);
        assert_eq!(anillo.indice_link(), 15);
        assert_eq!(anillo.direccion_link(), 0x1000 + 15 * 16);
        // Los primeros TRB utiles van seguidos.
        let (dir, ciclo) = anillo.siguiente();
        assert_eq!(dir, 0x1000);
        assert!(ciclo);
        let (dir, ciclo) = anillo.siguiente();
        assert_eq!(dir, 0x1010);
        assert!(ciclo);
    }

    #[test]
    fn el_anillo_de_transferencia_da_la_vuelta_en_el_link() {
        let mut anillo = AnilloTransferencia::nuevo(0x1000, 16).expect("anillo");
        // 16 TRBs, uno de ellos el Link: 15 utiles.
        assert_eq!(anillo.utiles(), 15);
        // Los primeros TRB utiles van seguidos y con el ciclo a uno.
        for i in 0..14u64 {
            let (dir, ciclo) = anillo.siguiente();
            assert_eq!(dir, 0x1000 + i * 16);
            assert!(ciclo);
            assert!(!anillo.dio_la_vuelta);
        }
        // El ultimo TRB util es el que hace cruzar el Link: el productor pasa
        // al principio del segmento y da la vuelta al ciclo.
        let (dir, ciclo) = anillo.siguiente();
        assert_eq!(dir, 0x1000 + 14 * 16);
        assert!(ciclo, "el ultimo TRB de la vuelta lleva el ciclo viejo");
        assert!(anillo.dio_la_vuelta);
        assert!(!anillo.ciclo, "la vuelta siguiente lleva el ciclo a cero");
        // Y el siguiente TRB vuelve al principio del segmento.
        let (dir, ciclo) = anillo.siguiente();
        assert_eq!(dir, 0x1000);
        assert!(!ciclo);
        assert!(!anillo.dio_la_vuelta);
    }

    #[test]
    fn el_anillo_de_transferencia_dice_si_cabe_una_tanda() {
        let mut anillo = AnilloTransferencia::nuevo(0x1000, 16).expect("anillo");
        // Una tanda de control con datos son tres TRBs (setup, datos,
        // estado).
        assert!(anillo.cabe(3));
        for _ in 0..12 {
            anillo.siguiente();
        }
        // Quedan tres TRBs utiles: la tanda cabe justa.
        assert_eq!(anillo.indice, 12);
        assert!(anillo.cabe(3));
        assert!(!anillo.cabe(4), "la tanda no se puede partir en el Link");
        anillo.siguiente();
        assert!(!anillo.cabe(3), "la tanda no se puede partir en el Link");
    }

    #[test]
    fn el_setup_lleva_idt_y_longitud_ocho() {
        // QEMU exige el bit IDT y una longitud de 8 en el Setup TRB
        // (`xhci_fire_ctl_transfer`): sin ellos rechaza la transferencia.
        let t = trb_setup_stage(0x80, 0x06, 0x0100, 0, 18, TRT_DATOS_IN, false);
        assert_eq!(t.status(), 8);
        assert_ne!(t.control() & IDT, 0, "falta el bit IDT");
        assert_eq!(t.control() & IOC, 0, "el setup no genera el evento");
        // El paquete de setup va en el parametro, en orden de bytes.
        assert_eq!(t.parameter() & 0xFF, 0x80);
        assert_eq!((t.parameter() >> 8) & 0xFF, 0x06);
        assert_eq!((t.parameter() >> 16) & 0xFFFF, 0x0100);
        assert_eq!((t.parameter() >> 32) & 0xFFFF, 0);
        assert_eq!((t.parameter() >> 48) & 0xFFFF, 18);
        // El TRT va en los bits 17:16.
        assert_eq!((t.control() >> 16) & 0x3, TRT_DATOS_IN);
    }

    #[test]
    fn el_data_stage_pone_isp_solo_en_las_entradas() {
        let entrada = trb_data_stage(0x2000, 18, true, true, false, false);
        assert_ne!(
            entrada.control() & ISP,
            0,
            "la entrada avisa de paquete corto"
        );
        assert_ne!(entrada.control() & DIR_ENTRADA, 0);
        assert_eq!(entrada.control() & IOC, 0, "el evento lo genera el status");
        // La longitud va en el estado, no en el control.
        assert_eq!(entrada.status(), 18);
        assert_eq!(entrada.parameter(), 0x2000);

        let salida = trb_data_stage(0x2000, 18, false, false, false, false);
        assert_eq!(salida.control() & ISP, 0, "una salida no tiene ISP");
        assert_eq!(salida.control() & DIR_ENTRADA, 0);
    }

    #[test]
    fn el_status_stage_es_la_ultima_etapa_y_avisa() {
        // Sin datos: la etapa de estado es una entrada.
        let t = trb_status_stage(true, false, true);
        assert_ne!(t.control() & DIR_ENTRADA, 0);
        assert_ne!(t.control() & IOC, 0, "el status genera el evento");
        // Con datos de entrada: la etapa de estado es una salida.
        let t = trb_status_stage(false, false, true);
        assert_eq!(t.control() & DIR_ENTRADA, 0);
    }
}
