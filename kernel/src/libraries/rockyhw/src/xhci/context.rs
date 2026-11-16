//! Contextos de dispositivo/entrada xHCI (estructuras acotadas).
//!
//! Los contextos son arrays de dwords alineados a 64 bytes. Esta implementacion
//! es la suficiente para Enable Slot / Address Device / Configure Endpoint
//! en el caso mas comun (un dispositivo sin hubs).
//!
//! ## Disposicion verificada
//!
//! Los campos de bit estan tomados de la cabecera del driver de Linux
//! (`drivers/usb/host/xhci.h`), que es la referencia de hardware real, y
//! contrastados con el emulador (`hw/usb/hcd-xhci.c` de QEMU). Los dos
//! coinciden.
//!
//! ```text
//! Slot Context (32 bytes)
//!   dword 0  Route String[19:0] | Speed[23:20] | MTT[25] | Hub[26] | Context Entries[31:27]
//!   dword 1  Max Exit Latency[15:0] | Root Hub Port Number[23:16] | Max Ports[31:24]
//!   dword 2  TT Slot[7:0] | TT Port[15:8] | TTT[17:16] | Interrupter Target[31:22]
//!   dword 3  Device Address[7:0] | Slot State[31:27]
//!
//! Endpoint Context (32 bytes)
//!   dword 0  EP State[2:0] | Mult[9:8] | MaxPStreams[14:10] | LSA[15] | Interval[23:16] | MaxESIT-hi[31:24]
//!   dword 1  ForceEvent[0] | ErrorCount[2:1] | EP Type[5:3] | HostInitDisable[7]
//!            | MaxBurst[15:8] | MaxPacketSize[31:16]
//!   dword 2  TR Dequeue Pointer[31:4] | DCS[0]
//!   dword 3  TR Dequeue Pointer[63:32]
//!   dword 4  Average TRB Length[15:0] | Max ESIT Payload[31:16]
//!
//! Input Control Context (32 bytes)
//!   dword 0  Drop Flags
//!   dword 1  Add Flags (bit 0 = Slot Context, bit 1 = Endpoint 0)
//!   dword 2..7 reservados
//! ```
//!
//! Ojo con dos trampas:
//!
//! - Los **Context Entries** van en el dword 0 (bits 31:27), no en el 1.
//! - El **Max Burst Size** es un campo *con base cero*: el valor real es el
//!   del campo mas uno (Linux: `EP_MAX_BURST`).
//!
//! Codigo propio de rocky.

/// Bytes de un dword.
pub const CONTEXTO_DWORD: usize = 4;
/// El contexto de slot ocupa 32 bytes (8 dwords).
pub const SLOT_CONTEXT_DWORDS: usize = 8;
/// El contexto de endpoint ocupa 32 bytes (8 dwords).
pub const ENDPOINT_CONTEXT_DWORDS: usize = 8;
/// El contexto de control de entrada ocupa 32 bytes (8 dwords).
pub const INPUT_CONTROL_CONTEXT_DWORDS: usize = 8;
/// Numero maximo de endpoints que se gestionan en un dispositivo.
pub const MAX_EP: usize = 32;

/// Estados del endpoint (Linux: `EP_STATE_*`).
pub const EP_ESTADO_INVALIDO: u8 = 0;
/// Endpoint habilitado y en marcha.
pub const EP_ESTADO_EN_MARCHA: u8 = 1;
/// Endpoint parado por un error.
pub const EP_ESTADO_DETENIDO: u8 = 2;
/// Endpoint detenido a proposito por el software.
pub const EP_ESTADO_PARADO: u8 = 3;
/// Endpoint con error irrecuperable.
pub const EP_ESTADO_ERROR: u8 = 4;

/// Tipos de endpoint (Linux: `EP_TYPE_*`; bits 5:3 del dword 1).
pub const EP_TIPO_INVALIDO: u8 = 0;
/// Isochronous OUT.
pub const EP_TISO_OUT: u8 = 1;
/// Isochronous IN.
pub const EP_TISO_IN: u8 = 2;
/// Interrupt OUT.
pub const EP_TINT_OUT: u8 = 3;
/// Control (el endpoint 0 de cualquier dispositivo).
pub const EP_TCONTROL: u8 = 4;
/// Bulk OUT.
pub const EP_TBULK_OUT: u8 = 5;
/// Bulk IN.
pub const EP_TBULK_IN: u8 = 6;
/// Interrupt IN (el endpoint de teclado, raton o gamepad).
pub const EP_TINT_IN: u8 = 7;

/// Estados del hueco de dispositivo (Linux: `SLOT_STATE`).
pub const HUECO_HABILITADO: u8 = 0;
/// Dispositivo conectado, todavia sin direccion.
pub const HUECO_POR_DEFECTO: u8 = 1;
/// Dispositivo con direccion asignada.
pub const HUECO_DIRECCIONADO: u8 = 2;
/// Dispositivo configurado.
pub const HUECO_CONFIGURADO: u8 = 3;

/// Velocidades del dispositivo (Linux: `USB_SPEED_*`).
/// Full speed (1.5 MB/s).
pub const VELOCIDAD_FULL: u8 = 1;
/// Low speed (0.5 MB/s).
pub const VELOCIDAD_LOW: u8 = 2;
/// High speed (12 MB/s).
pub const VELOCIDAD_HIGH: u8 = 3;
/// SuperSpeed (5 Gbps).
pub const VELOCIDAD_SUPER: u8 = 4;
/// SuperSpeed Plus (10 Gbps).
pub const VELOCIDAD_SUPER_PLUS: u8 = 5;

/// Contexto de slot (Slot Context) - 32 bytes (8 dwords)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SlotContext {
    pub d: [u32; SLOT_CONTEXT_DWORDS],
}

impl Default for SlotContext {
    fn default() -> Self {
        SlotContext {
            d: [0; SLOT_CONTEXT_DWORDS],
        }
    }
}

impl SlotContext {
    /// Pone la cadena de ruta, la velocidad, las banderas de hub y MTT.
    ///
    /// - `route`: cadena de ruta del puerto, 20 bits.
    /// - `speed`: una de las constantes `VELOCIDAD_*`.
    /// - `hub`: el dispositivo es un hub.
    /// - `mtt`: el dispositivo tiene transaccion multi-transferencia.
    pub fn set_route_speed(&mut self, route: u32, speed: u8, hub: bool, mtt: bool) {
        self.d[0] = ((route & 0xF_FFFF) << 0)
            | (((speed as u32) & 0xF) << 20)
            | (if mtt { 1 << 25 } else { 0 })
            | (if hub { 1 << 26 } else { 0 });
    }

    /// Numero de contextos de endpoint que siguen al contexto de slot.
    ///
    /// Va en los bits 31:27 del **dword 0**. El valor minimo es 1 (el
    /// endpoint 0 de control siempre existe).
    pub fn set_context_entries(&mut self, entradas: u8) {
        self.d[0] = (self.d[0] & !(0x1F << 27)) | (((entradas as u32) & 0x1F) << 27);
    }

    /// Lee el numero de contextos de endpoint.
    pub fn get_context_entries(&self) -> u8 {
        ((self.d[0] >> 27) & 0x1F) as u8
    }

    /// Latencia maxima de salida, en unidades del temporizador del
    /// controlador (bits 15:0 del dword 1).
    pub fn set_max_exit_latency(&mut self, latencia: u16) {
        self.d[1] = (self.d[1] & !0xFFFF) | (latencia as u32);
    }

    /// Numero de puerto del hub raiz al que esta conectado el dispositivo
    /// (bits 23:16 del dword 1).
    pub fn set_root_hub_port(&mut self, puerto: u8) {
        self.d[1] = (self.d[1] & !(0xFF << 16)) | (((puerto as u32) & 0xFF) << 16);
    }

    /// Lee el numero de puerto del hub raiz.
    pub fn get_root_hub_port(&self) -> u8 {
        ((self.d[1] >> 16) & 0xFF) as u8
    }

    /// Numero maximo de puertos, solo para hubs (bits 31:24 del dword 1).
    pub fn set_max_ports(&mut self, puertos: u8) {
        self.d[1] = (self.d[1] & !(0xFF << 24)) | (((puertos as u32) & 0xFF) << 24);
    }

    /// Informacion de transaccion de un hub (bits 0:17 del dword 2).
    pub fn set_tt_info(&mut self, tt_hub_slot: u8, tt_port: u8, ttt: u8) {
        self.d[2] = (self.d[2] & !(0x0003_FFFF))
            | ((tt_hub_slot as u32) & 0xFF)
            | (((tt_port as u32) & 0xFF) << 8)
            | (((ttt as u32) & 0x3) << 16);
    }

    /// Interrupter objetivo del dispositivo (bits 31:22 del dword 2).
    ///
    /// Es el indice del interrupter al que se mandan los eventos de este
    /// dispositivo. Con cero (el valor por defecto) el controlador usa el
    /// interrupter 0.
    pub fn set_interrupter_target(&mut self, target: u16) {
        self.d[2] = (self.d[2] & !(0x3FF << 22)) | (((target as u32) & 0x3FF) << 22);
    }

    /// Lee el interrupter objetivo.
    pub fn get_interrupter_target(&self) -> u16 {
        ((self.d[2] >> 22) & 0x3FF) as u16
    }

    /// Estado del hueco (bits 31:27 del dword 3).
    ///
    /// Lo escribe el controlador, no el software: el software lo lee para
    /// saber en que punto esta el dispositivo.
    pub fn set_slot_state(&mut self, estado: u8) {
        self.d[3] = (self.d[3] & !(0x1F << 27)) | (((estado as u32) & 0x1F) << 27);
    }

    /// Lee el estado del hueco.
    pub fn get_slot_state(&self) -> u8 {
        ((self.d[3] >> 27) & 0x1F) as u8
    }

    /// Direccion asignada al dispositivo (bits 7:0 del dword 3).
    pub fn set_device_address(&mut self, direccion: u8) {
        self.d[3] = (self.d[3] & !0xFF) | (direccion as u32);
    }

    /// Lee la direccion del dispositivo.
    pub fn get_device_address(&self) -> u8 {
        (self.d[3] & 0xFF) as u8
    }
}

/// Contexto de endpoint (Endpoint Context) - 32 bytes (8 dwords)
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct EndpointContext {
    pub d: [u32; ENDPOINT_CONTEXT_DWORDS],
}

impl Default for EndpointContext {
    fn default() -> Self {
        EndpointContext {
            d: [0; ENDPOINT_CONTEXT_DWORDS],
        }
    }
}

impl EndpointContext {
    /// Pone el estado, el multiplicador, el numero de streams, el LSA y el
    /// intervalo (dword 0).
    pub fn set_state_mult_pstreams_lsa_interval(
        &mut self,
        state: u8,
        mult: u8,
        max_pstreams: u8,
        lsa: bool,
        interval: u8,
    ) {
        self.d[0] = ((state as u32) & 0x7)
            | (((mult as u32) & 0x3) << 8)
            | (((max_pstreams as u32) & 0x1F) << 10)
            | (if lsa { 1 << 15 } else { 0 })
            | (((interval as u32) & 0xFF) << 16);
    }

    /// Pone el tipo de endpoint, el contador de errores y el bit de
    /// deshabilitacion iniciada por el host (dword 1, bits 7:1).
    pub fn set_type_err_hid(&mut self, ep_type: u8, cerr: u8, hid: bool) {
        self.d[1] = (self.d[1] & !(0xFF << 1))
            | (((cerr as u32) & 0x3) << 1)
            | (((ep_type as u32) & 0x7) << 3)
            | (if hid { 1 << 7 } else { 0 });
    }

    /// Pone el tamano maximo de paquete y el de rafaga (dword 1, bits 31:8).
    ///
    /// - `max_packet`: tamano maximo de paquete en bytes.
    /// - `max_burst`: numero **real** de paquetes por rafaga. El campo es
    ///   base cero, asi que se guarda `max_burst - 1`.
    pub fn set_packet_burst(&mut self, max_packet: u16, max_burst: u8) {
        let burst = if max_burst == 0 { 0 } else { max_burst - 1 };
        self.d[1] = (self.d[1] & 0xFF)
            | (((burst as u32) & 0xFF) << 8)
            | (((max_packet as u32) & 0xFFFF) << 16);
    }

    /// Lee el tamano maximo de paquete.
    pub fn get_max_packet(&self) -> u16 {
        ((self.d[1] >> 16) & 0xFFFF) as u16
    }

    /// Lee el numero real de paquetes por rafaga (el campo mas uno).
    pub fn get_max_burst(&self) -> u8 {
        (((self.d[1] >> 8) & 0xFF) as u8).saturating_add(1)
    }

    /// Pone el puntero de extraccion del anillo de transferencia (dwords 2 y 3).
    ///
    /// El puntero va en los bits 63:4 y el bit 0 es el estado de ciclo del
    /// consumidor (`DCS`). Los bits 3:1 estan reservados y valen cero, asi que
    /// la direccion se enmascara con `!0xF`.
    pub fn set_tr_dequeue(&mut self, ptr: u64, dcs: bool) {
        let dir = ptr & !0xF;
        self.d[2] = (dir as u32 & !0xF) | (if dcs { 1 } else { 0 });
        self.d[3] = (dir >> 32) as u32;
    }

    /// Lee el puntero de extraccion del anillo de transferencia.
    pub fn get_tr_dequeue(&self) -> u64 {
        ((self.d[3] as u64) << 32) | ((self.d[2] as u64) & !0xF)
    }

    /// Pone la longitud media de los TRB y la carga maxima de ESIT (dword 4).
    pub fn set_avg_trb_len(&mut self, len: u16) {
        self.d[4] = (self.d[4] & !(0xFFFF << 0)) | (len as u32);
    }

    /// Pone la carga maxima de ESIT (bits 31:16 del dword 4).
    pub fn set_max_esit(&mut self, max_esit: u16) {
        self.d[4] = (self.d[4] & 0xFFFF) | (((max_esit as u32) & 0xFFFF) << 16);
    }
}

/// Input Control Context - 32 bytes
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct InputControlContext {
    pub d: [u32; INPUT_CONTROL_CONTEXT_DWORDS],
}

impl Default for InputControlContext {
    fn default() -> Self {
        InputControlContext {
            d: [0; INPUT_CONTROL_CONTEXT_DWORDS],
        }
    }
}

impl InputControlContext {
    /// Pone las banderas de "quitar contexto".
    pub fn set_drop_flags(&mut self, drop: u32) {
        self.d[0] = drop;
    }

    /// Pone las banderas de "anadir contexto".
    pub fn set_add_flags(&mut self, add: u32) {
        self.d[1] = add;
    }

    /// Marca un contexto para ser quitado.
    pub fn drop_context(&mut self, idx: u32) {
        self.d[0] |= 1 << idx;
    }

    /// Marca un contexto para ser anadido.
    pub fn add_context(&mut self, idx: u32) {
        self.d[1] |= 1 << idx;
    }
}

/// Bandera de "anadir contexto de slot" del contexto de control de entrada.
pub const AÑADIR_SLOT: u32 = 1 << 0;
/// Bandera de "anadir contexto de endpoint 0" del contexto de control de
/// entrada.
pub const AÑADIR_EP0: u32 = 1 << 1;

/// Device Context (output context) - base structure
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DeviceContext {
    pub slot: SlotContext,
    pub eps: [EndpointContext; 31], // ep0 + 30
}

impl Default for DeviceContext {
    fn default() -> Self {
        DeviceContext {
            slot: SlotContext::default(),
            eps: [EndpointContext::default(); 31],
        }
    }
}

/// Input Context - 33 contexts * 32 bytes (max) but we allocate contiguously
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct InputContext {
    pub ctrl: InputControlContext,
    pub slot: SlotContext,
    pub eps: [EndpointContext; 31],
}

impl Default for InputContext {
    fn default() -> Self {
        InputContext {
            ctrl: InputControlContext::default(),
            slot: SlotContext::default(),
            eps: [EndpointContext::default(); 31],
        }
    }
}

/// El identificador de endpoint que usa el controlador.
///
/// El endpoint 0 de control es el **1** en la numeracion del controlador: el
/// valor 0 esta reservado. Los demas van en pares: el par es el numero de
/// endpoint y el impar es el de entrada.
pub fn ep_id(direccion_in: bool, ep_num: u8) -> u8 {
    if ep_num == 0 {
        return 1;
    }
    if direccion_in {
        2 * ep_num + 1
    } else {
        2 * ep_num
    }
}

/// Codifica un puntero de extraccion de anillo con su bit de ciclo.
///
/// El puntero de extraccion de un contexto de endpoint lleva la direccion en
/// los bits 63:4 y el estado de ciclo del consumidor en el bit 0.
pub fn trb_pointer_con_dcs(ptr: u64, dcs: bool) -> u64 {
    (ptr & !0xF) | (if dcs { 1 } else { 0 })
}
