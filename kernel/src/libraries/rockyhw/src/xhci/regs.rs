//! Direcciones y bits de los registros del xHCI.
//!
//! Los offsets y los campos de bit estan tomados de la especificacion
//! "eXtensible Host Controller Interface for Universal Serial Bus (xHCI)",
//! Intel, Rev 1.2 (mayo de 2019):
//!
//! - §5.3: registros de capacidades (base del BAR de registros).
//! - §5.4: registros operacionales (base + `CAPLENGTH` bytes).
//! - §5.5: registros de tiempo de ejecucion (base + `RTSOFF` bytes).
//! - §5.4.8: `PORTSC`.
//! - §5.5.2: el juego de registros del interrupter.
//!
//! Aqui **solo hay numeros**: ni una lectura, ni una escritura, ni una
//! decision. Eso permite comprobar los campos de bit contra el texto de la
//! especificacion sin hardware delante, y deja la logica que decide en otro
//! sitio.
//!
//! Codigo propio de rocky.

// =====================================================================
// Base de capacidades: `Base + offset`
// =====================================================================

/// Longitud de los registros de capacidades, en bytes.
///
/// El valor es el desplazamiento, en bytes, desde `Base` hasta el **comienzo
/// de los registros operacionales** (§5.3.1). La base operacional sale
/// alineada a dword, asi que el valor es multiplo de cuatro.
pub const CAP_CAPLENGTH: usize = 0x00;
/// Version de la interfaz de controlador de host, en BCD (§5.3.2).
pub const CAP_HCIVERSION: usize = 0x02;
/// Parametros estructurales 1 (§5.3.3).
pub const CAP_HCSPARAMS1: usize = 0x04;
/// Parametros estructurales 2 (§5.3.4).
pub const CAP_HCSPARAMS2: usize = 0x08;
/// Parametros estructurales 3 (§5.3.5).
pub const CAP_HCSPARAMS3: usize = 0x0C;
/// Parametros de capacidad 1 (§5.3.6).
pub const CAP_HCCPARAMS1: usize = 0x10;
/// Desplazamiento de la array de campanas, en bytes desde `Base` (§5.3.7).
pub const CAP_DBOFF: usize = 0x14;
/// Desplazamiento de los registros de tiempo de ejecucion, en bytes desde
/// `Base` (§5.3.8).
pub const CAP_RTSOFF: usize = 0x18;
/// Parametros de capacidad 2 (§5.3.9).
pub const CAP_HCCPARAMS2: usize = 0x1C;
/// Desplazamiento de los registros VTIO, en dwords desde `Base` (§5.3.10).
pub const CAP_VTIOSOFF: usize = 0x20;

/// Longitud minima de la base de capacidades, en bytes.
///
/// Un valor menor que este no puede contener ni `DBOFF` ni `RTSOFF`, asi que
/// el controlador no se puede ni leer.
pub const CAPLENGTH_MINIMO: u8 = 0x1C;

// --- HCSPARAMS1 (§5.3.3) ---

/// Numero de huecos de dispositivo (`MaxSlots`), bits 7:0.
pub const HCS1_MAXSLOTS_MASCARA: u32 = 0x0000_00FF;
/// Numero de interrumpers (`MaxIntrs`), bits 18:8.
pub const HCS1_MAXINTRS_MASCARA: u32 = 0x0007_FF00;
/// Desplazamiento del campo `MaxIntrs` dentro de la palabra.
pub const HCS1_MAXINTRS_DESPLAZAMIENTO: u32 = 8;
/// Numero maximo de puertos (`MaxPorts`), bits 31:24.
pub const HCS1_MAXPORTS_MASCARA: u32 = 0xFF00_0000;
/// Desplazamiento del campo `MaxPorts` dentro de la palabra.
pub const HCS1_MAXPORTS_DESPLAZAMIENTO: u32 = 24;

// --- HCSPARAMS2 (§5.3.4) ---

/// Maximo de entradas de la tabla de segmentos del anillo de eventos
/// (`ERST Max`), bits 7:4. El numero maximo de entradas es `2^ERST Max`.
pub const HCS2_ERSTMAX_MASCARA: u32 = 0x0000_00F0;
/// Desplazamiento del campo `ERST Max` dentro de la palabra.
pub const HCS2_ERSTMAX_DESPLAZAMIENTO: u32 = 4;
/// Numero alto de buffers de scratchpad, bits 25:21.
pub const HCS2_SCRATCH_HI_MASCARA: u32 = 0x03E0_0000;
/// Desplazamiento del campo de buffers de scratchpad alto.
pub const HCS2_SCRATCH_HI_DESPLAZAMIENTO: u32 = 21;
/// Bit de restauracion de scratchpad (`SPR`), bit 26.
pub const HCS2_SPR: u32 = 1 << 26;
/// Numero bajo de buffers de scratchpad, bits 31:27.
pub const HCS2_SCRATCH_LO_MASCARA: u32 = 0xF800_0000;
/// Desplazamiento del campo de buffers de scratchpad bajo.
pub const HCS2_SCRATCH_LO_DESPLAZAMIENTO: u32 = 27;

// --- HCCPARAMS1 (§5.3.6) ---

/// Direcciones de 64 bits (`AC64`), bit 0.
pub const HCC1_AC64: u32 = 1 << 0;
/// Negociacion de ancho de banda (`BNC`), bit 1.
pub const HCC1_BNC: u32 = 1 << 1;
/// Contextos de 64 bytes (`CSZ`), bit 2.
pub const HCC1_CSZ: u32 = 1 << 2;
/// Control de alimentacion de los puertos (`PPC`), bit 3.
pub const HCC1_PPC: u32 = 1 << 3;
/// Indicadores de puerto (`PIND`), bit 4.
pub const HCC1_PIND: u32 = 1 << 4;
/// Reinicio ligero del controlador (`LHRC`), bit 5.
pub const HCC1_LHRC: u32 = 1 << 5;
/// Contexto ligero (`LTC`), bit 6.
pub const HCC1_LTC: u32 = 1 << 6;
/// Sin flujos secundarios (`NSS`), bit 7.
pub const HCC1_NSS: u32 = 1 << 7;
/// Aceleracion de puerto (`PAE`), bit 8.
pub const HCC1_PAE: u32 = 1 << 8;
/// Contexto de puerto de expedite (`SPC`), bit 9.
pub const HCC1_SPC: u32 = 1 << 9;
/// Estado seguro (`SEC`), bit 10.
pub const HCC1_SEC: u32 = 1 << 10;
/// Fuerza de guardado de contexto (`CFC`), bit 11.
pub const HCC1_CFC: u32 = 1 << 11;
/// Tamano maximo de `PSA` (`MaxPSASize`), bits 15:12.
pub const HCC1_MAXPSASIZE_MASCARA: u32 = 0x0000_F000;
/// Desplazamiento del campo `MaxPSASize`.
pub const HCC1_MAXPSASIZE_DESPLAZAMIENTO: u32 = 12;
/// Desplazamiento de la primera capacidad extendida (`xECP`), bits 31:16, en
/// dwords desde `Base`.
pub const HCC1_XECP_MASCARA: u32 = 0xFFFF_0000;
/// Desplazamiento del campo `xECP`.
pub const HCC1_XECP_DESPLAZAMIENTO: u32 = 16;

// =====================================================================
// Base operacional: `Base operacional + offset`
// =====================================================================

/// Registro de comando de la USB (`USBCMD`, §5.4.1).
pub const OP_USBCMD: usize = 0x00;
/// Registro de estado de la USB (`USBSTS`, §5.4.2).
pub const OP_USBSTS: usize = 0x04;
/// Registro de tamano de pagina (`PAGESIZE`, §5.4.3).
pub const OP_PAGESIZE: usize = 0x08;
/// Control de notificaciones de dispositivo (`DNCTRL`, §5.4.4).
pub const OP_DNCTRL: usize = 0x14;
/// Control del anillo de comandos (`CRCR`, §5.4.5), 64 bits.
pub const OP_CRCR: usize = 0x18;
/// Puntero al array de contextos base (`DCBAAP`, §5.4.6), 64 bits.
pub const OP_DCBAAP: usize = 0x30;
/// Registro de configuracion (`CONFIG`, §5.4.7).
pub const OP_CONFIG: usize = 0x38;
/// Primer conjunto de registros de puerto (§5.4.8).
pub const OP_PUERTOS: usize = 0x400;

/// Bytes que ocupa el conjunto de registros de un puerto.
///
/// `PORTSC`, `PORTPMSC`, `PORTLI` y `PORTHLPMC`: 16 bytes por puerto
/// (§5.4.8, §5.4.9, §5.4.10, §5.4.11).
pub const PUERTO_PASO: usize = 0x10;

// --- USBCMD (§5.4.1) ---

/// Ejecutar/detener (`R/S`), bit 0.
pub const CMD_RUN: u32 = 1 << 0;
/// Reinicio del controlador de host (`HCRST`), bit 1.
pub const CMD_HCRST: u32 = 1 << 1;
/// Habilitar los interrumpers (`INTE`), bit 2.
pub const CMD_INTE: u32 = 1 << 2;
/// Habilitar el error de sistema del host (`HSEIE`), bit 3.
pub const CMD_HSEIE: u32 = 1 << 3;
/// Guardar el estado del controlador (`CSS`), bit 8.
pub const CMD_CSS: u32 = 1 << 8;
/// Restaurar el estado del controlador (`CRS`), bit 9.
pub const CMD_CRS: u32 = 1 << 9;
/// Habilitar el evento de envolvimiento de `MFINDEX` (`EWE`), bit 10.
pub const CMD_EWE: u32 = 1 << 10;

// --- USBSTS (§5.4.2) ---

/// Controlador detenido (`HCH`), bit 0.
pub const STS_HCH: u32 = 1 << 0;
/// Error de sistema del host (`HSE`), bit 2, se limpia escribiendo un 1.
pub const STS_HSE: u32 = 1 << 2;
/// Interrupcion de evento (`EINT`), bit 3, se limpia escribiendo un 1.
pub const STS_EINT: u32 = 1 << 3;
/// Deteccion de cambio de puerto (`PCD`), bit 4, se limpia escribiendo un 1.
pub const STS_PCD: u32 = 1 << 4;
/// Estado de guardado (`SSS`), bit 8.
pub const STS_SSS: u32 = 1 << 8;
/// Estado de restauracion (`RSS`), bit 9.
pub const STS_RSS: u32 = 1 << 9;
/// Error de guardado/restauracion (`SRE`), bit 10, se limpia escribiendo un 1.
pub const STS_SRE: u32 = 1 << 10;
/// Controlador no preparado (`CNR`), bit 11.
pub const STS_CNR: u32 = 1 << 11;
/// Error del controlador de host (`HCE`), bit 12, se limpia escribiendo un 1.
pub const STS_HCE: u32 = 1 << 12;

/// Bits de `USBSTS` que se limpian escribiendo un 1.
pub const STS_RW1C: u32 = STS_HSE | STS_EINT | STS_PCD | STS_SRE | STS_HCE;

// --- CONFIG (§5.4.7) ---

/// Numero de huecos de dispositivo habilitados (`MaxSlotsEn`), bits 7:0.
pub const CONFIG_MAXSLOTSEN_MASCARA: u32 = 0x0000_00FF;

// --- PAGESIZE (§5.4.3) ---

/// Campo de tamano de pagina, bits 15:0.
///
/// El bit `n` a uno significa que se admite un tamano de pagina de
/// `2^(n+12)` bytes.
pub const PAGESIZE_MASCARA: u32 = 0x0000_FFFF;

// =====================================================================
// Base de tiempo de ejecucion: `Base de ejecucion + offset`
// =====================================================================

/// Indice de microfotograma (`MFINDEX`, §5.5.1).
pub const RT_MFINDEX: usize = 0x00;
/// Control del tiempo de ejecucion (`CTRL`, §5.5).
pub const RT_CTRL: usize = 0x04;
/// Interrupciones asincronas (`AIE`, §5.5).
pub const RT_AIE: usize = 0x08;
/// Primer juego de registros de interrupter (§5.5.2).
pub const RT_INTERRUPTER: usize = 0x20;

/// Bytes que ocupa un juego de registros de interrupter.
pub const INTERRUPTER_PASO: usize = 0x20;

/// Gestion del interrupter (`IMAN`, §5.5.2.1), dentro del juego de registros.
pub const INT_IMAN: usize = 0x00;
/// Moderacion del interrupter (`IMOD`, §5.5.2.2), dentro del juego.
pub const INT_IMOD: usize = 0x04;
/// Tamano de la tabla de segmentos del anillo de eventos (`ERSTSZ`, §5.5.2.3.1).
pub const INT_ERSTSZ: usize = 0x08;
/// Base de la tabla de segmentos del anillo de eventos (`ERSTBA`, §5.5.2.3.2),
/// 64 bits.
pub const INT_ERSTBA: usize = 0x10;
/// Puntero de extraccion del anillo de eventos (`ERDP`, §5.5.2.3.3), 64 bits.
pub const INT_ERDP: usize = 0x18;

/// Interrupcion pendiente (`IP`), bit 0 de `IMAN`, se limpia escribiendo un 1.
pub const IMAN_IP: u32 = 1 << 0;
/// Interrupcion habilitada (`IE`), bit 1 de `IMAN`.
pub const IMAN_IE: u32 = 1 << 1;

// =====================================================================
// PORTSC (§5.4.8)
// =====================================================================

/// Estado de conexion actual (`CCS`), bit 0.
pub const PORTSC_CCS: u32 = 1 << 0;
/// Puerto habilitado (`PED`), bit 1.
pub const PORTSC_PED: u32 = 1 << 1;
/// Sobretension activa (`OCA`), bit 3.
pub const PORTSC_OCA: u32 = 1 << 3;
/// Reinicio del puerto (`PR`), bit 4.
pub const PORTSC_PR: u32 = 1 << 4;
/// Estado del enlace del puerto (`PLS`), bits 8:5.
pub const PORTSC_PLS_MASCARA: u32 = 0x0000_01E0;
/// Desplazamiento del campo `PLS`.
pub const PORTSC_PLS_DESPLAZAMIENTO: u32 = 5;
/// Alimentacion del puerto (`PP`), bit 9.
pub const PORTSC_PP: u32 = 1 << 9;
/// Velocidad del puerto, bits 13:10.
pub const PORTSC_VELOCIDAD_MASCARA: u32 = 0x0000_3C00;
/// Desplazamiento del campo de velocidad.
pub const PORTSC_VELOCIDAD_DESPLAZAMIENTO: u32 = 10;
/// Control de indicadores de puerto (`PIC`), bits 15:14.
pub const PORTSC_PIC_MASCARA: u32 = 0x0000_C000;
/// Estrobillo de escritura del estado del enlace (`LWS`), bit 16.
pub const PORTSC_LWS: u32 = 1 << 16;
/// Cambio de estado de conexion (`CSC`), bit 17.
pub const PORTSC_CSC: u32 = 1 << 17;
/// Cambio de puerto habilitado (`PEC`), bit 18.
pub const PORTSC_PEC: u32 = 1 << 18;
/// Cambio de reinicio en caliente (`WRC`), bit 19.
pub const PORTSC_WRC: u32 = 1 << 19;
/// Cambio de sobretension (`OCC`), bit 20.
pub const PORTSC_OCC: u32 = 1 << 20;
/// Cambio de reinicio del puerto (`PRC`), bit 21.
pub const PORTSC_PRC: u32 = 1 << 21;
/// Cambio de estado del enlace (`PLC`), bit 22.
pub const PORTSC_PLC: u32 = 1 << 22;
/// Error de configuracion del puerto (`CEC`), bit 23.
pub const PORTSC_CEC: u32 = 1 << 23;
/// Estado de conexion en frio (`CAS`), bit 24.
pub const PORTSC_CAS: u32 = 1 << 24;
/// Dispositivo extraible (`DR`), bit 30.
pub const PORTSC_DR: u32 = 1 << 30;

/// Bits de `PORTSC` que son banderas de cambio y se limpian escribiendo un 1.
pub const PORTSC_CAMBIO: u32 =
    PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_OCC | PORTSC_PRC | PORTSC_PLC | PORTSC_CEC;

/// Bits de `PORTSC` que el software puede escribir aparte de las banderas de
/// cambio.
///
/// Solo `PR` (el reinicio de puerto), `PP` (la alimentacion) y `PIC` (el
/// control de los indicadores). Deliberadamente **no** estan:
///
/// - `PED`, que es de escribir-1-para-limpiar: escribir un uno **deshabilita**
///   el puerto.
/// - `CCS`, `OCA` y `PLS`, que son de solo lectura (el estado del enlace se
///   cambia con `LWS`).
/// - `WCE`, `WDE`, `WOE` y `WPR`, que no hacen falta para esta fase.
pub const PORTSC_ESCRIBIBLES: u32 = PORTSC_PR | PORTSC_PP | PORTSC_PIC_MASCARA;

/// Valor de `PLS` que significa "RxDetect" (recepcion).
pub const PLS_RXDETECT: u32 = 5;

// =====================================================================
// Capacidades extendidas (§7)
// =====================================================================

/// Identificador de la capacidad de soporte USB heredado (§7.1).
pub const EXT_LEGACY: u8 = 1;
/// Identificador de la capacidad de protocolos soportados (§7.2).
pub const EXT_PROTOCOLOS: u8 = 2;

/// Identificador de la capacidad, bits 7:0 del puntero de capacidad extendida.
pub const EXT_ID_MASCARA: u32 = 0x0000_00FF;
/// Puntero a la siguiente capacidad extendida, bits 15:8 (dwords relativos).
pub const EXT_SIGUIENTE_MASCARA: u32 = 0x0000_FF00;
/// Desplazamiento del puntero a la siguiente capacidad extendida.
pub const EXT_SIGUIENTE_DESPLAZAMIENTO: u32 = 8;

/// Semaforo de propiedad de la BIOS (`HC BIOS Owned Semaphore`), bit 16 de
/// `USBLEGSUP` (§7.1.1).
pub const LEGSUP_BIOS: u32 = 1 << 16;
/// Semaforo de propiedad del sistema operativo (`HC OS Owned Semaphore`), bit
/// 24 de `USBLEGSUP` (§7.1.1).
pub const LEGSUP_OS: u32 = 1 << 24;

/// Revision mayor de la capacidad de protocolos soportados, bits 31:24.
pub const PROTOCOLO_MAYOR_DESPLAZAMIENTO: u32 = 24;
/// Revision menor de la capacidad de protocolos soportados, bits 23:16.
pub const PROTOCOLO_MENOR_DESPLAZAMIENTO: u32 = 16;

/// Desplazamiento del puerto inicial compatible, bits 7:0 (§7.2).
pub const PROTOCOLO_OFFSET_MASCARA: u32 = 0x0000_00FF;
/// Numero de puertos compatibles, bits 15:8 (§7.2).
pub const PROTOCOLO_CUENTA_MASCARA: u32 = 0x0000_FF00;
/// Desplazamiento del campo de numero de puertos compatibles.
pub const PROTOCOLO_CUENTA_DESPLAZAMIENTO: u32 = 8;

/// Numero de entradas de velocidad de protocolo (`PSIC`), bits 31:28 (§7.2).
pub const PROTOCOLO_PSIC_DESPLAZAMIENTO: u32 = 28;
/// Numero de palabras de velocidad de protocolo, en el desplazamiento `PSIC`.
pub const PROTOCOLO_PSI_PALABRAS: usize = 1;
