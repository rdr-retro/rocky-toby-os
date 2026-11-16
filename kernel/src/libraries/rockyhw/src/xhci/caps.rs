//! Lectura y validacion de las capacidades de un controlador xHCI.
//!
//! Aqui vive todo lo que se puede decidir **sin tocar el hardware**: que
//! version de la especificacion implementa el controlador, cuantos huecos de
//! dispositivo, cuantos puertos, cuantos buffers de scratchpad pide, que
//! paginas usa, y que protocolos (USB 2.0 o USB 3.x) soporta cada puerto.
//!
//! La lectura se hace a traves de [`Registro`], que es una ventana de solo
//! lectura sobre los registros. Asi el mismo codigo que usa el driver con MMIO
//! se puede probar en el anfitrion con un mapa de registros de mentira.
//!
//! Referencia: Intel, "eXtensible Host Controller Interface for Universal
//! Serial Bus (xHCI) Requirements Specification", Rev 1.2, §5.3 y §7.
//!
//! Codigo propio de rocky.

use crate::xhci::regs::*;

/// Una ventana de solo lectura sobre los registros del controlador.
///
/// Los registros de 64 bits se leen como dos dwords, con el bajo primero
/// (§5.1 de la especificacion).
pub trait Registro {
    /// Lee el dword de la posicion `off`. Devuelve `None` si la posicion no
    /// existe en la ventana.
    fn leer32(&self, off: usize) -> Option<u32>;

    /// Lee el registro de 64 bits de la posicion `off`.
    ///
    /// El dword bajo va primero: es la convencion de la especificacion para
    /// `DCBAAP`, `CRCR`, `ERSTBA` y `ERDP`.
    fn leer64(&self, off: usize) -> Option<u64> {
        let lo = self.leer32(off)? as u64;
        let hi = self.leer32(off.checked_add(4)?)? as u64;
        Some(lo | (hi << 32))
    }
}

/// Por que no se puede usar un controlador xHCI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCap {
    /// Un registro que hace falta esta fuera de la ventana de registros.
    RegistroInaccesible,
    /// `CAPLENGTH` es menor que el minimo: no caben ni los parametros
    /// estructurales.
    CapLengthInvalido,
    /// La version de la interfaz es anterior a la 1.0.
    VersionInvalida,
    /// `MaxSlots` vale cero, que la especificacion reserva.
    MaxSlotsInvalido,
    /// `MaxIntrs` vale cero, que la especificacion declara indefinido.
    MaxIntrosInvalido,
    /// `MaxPorts` vale cero, que la especificacion reserva.
    MaxPuertosInvalido,
    /// El desplazamiento de los registros de ejecucion no cabe en la ventana.
    DesplazamientoInvalido,
    /// La tabla de segmentos del anillo de eventos no admite ni una entrada.
    ErstInvalido,
    /// No hay ninguna capacidad de "protocolos soportados": es obligatoria.
    SinProtocolos,
    /// Dos capacidades de protocolos se solapan o no cubren todos los puertos.
    ProtocolosInconsistentes,
    /// La lista de capacidades extendidas no termina (bucle) o se sale de la
    /// ventana.
    CapacidadesExtendidasInvalidas,
    /// El controlador no admite punteros de 64 bits y sus estructuras no
    /// caben en el espacio de direcciones de 32 bits.
    Solo32Bits,
}

/// Un rango de puertos raiz que soportan un mismo protocolo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangoPuertos {
    /// Primer puerto del rango (numerado desde 1).
    pub primero: u8,
    /// Numero de puertos consecutivos del rango.
    pub cantidad: u8,
}

impl RangoPuertos {
    /// ¿El numero de puerto cae dentro del rango?
    pub fn contiene(&self, puerto: u8) -> bool {
        puerto >= self.primero && puerto < self.primero.saturating_add(self.cantidad)
    }
}

/// Que protocolo habla un puerto raiz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocolo {
    /// USB 2.0 (baja, completa y alta velocidad).
    Usb2,
    /// USB 3.x (SuperSpeed y SuperSpeed Plus).
    Usb3,
}

impl Protocolo {
    /// Nombre corto, para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            Protocolo::Usb2 => "USB 2.0",
            Protocolo::Usb3 => "USB 3.x",
        }
    }
}

/// Los puertos raiz y que protocolo habla cada uno.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Protocolos {
    /// Puertos USB 2.0, si el controlador los declara.
    pub usb2: Option<RangoPuertos>,
    /// Puertos USB 3.x, si el controlador los declara.
    pub usb3: Option<RangoPuertos>,
}

impl Protocolos {
    /// Que protocolo habla el puerto `puerto` (numerado desde 1).
    ///
    /// Un puerto que ninguna capacidad declara **no es** un puerto USB 2.0:
    /// se devuelve `None` en vez de adivinar.
    pub fn protocolo_de(&self, puerto: u8) -> Option<Protocolo> {
        if let Some(r) = self.usb2 {
            if r.contiene(puerto) {
                return Some(Protocolo::Usb2);
            }
        }
        if let Some(r) = self.usb3 {
            if r.contiene(puerto) {
                return Some(Protocolo::Usb3);
            }
        }
        None
    }

    /// Numero total de puertos declarados.
    pub fn total(&self) -> u16 {
        let a = self.usb2.map(|r| r.cantidad as u16).unwrap_or(0);
        let b = self.usb3.map(|r| r.cantidad as u16).unwrap_or(0);
        a + b
    }
}

/// Las capacidades de un controlador xHCI, ya leidas y validadas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacidades {
    /// Version de la interfaz de controlador de host, en BCD (`0x0110` = 1.1).
    pub version: u16,
    /// Numero de huecos de dispositivo (`MaxSlots`).
    pub max_slots: u8,
    /// Numero de interrumpers implementados (`MaxIntrs`).
    pub max_intrs: u16,
    /// Numero maximo de puertos raiz (`MaxPorts`): los puertos van de 1 a este
    /// valor.
    pub max_ports: u8,
    /// Numero maximo de entradas de la tabla de segmentos del anillo de
    /// eventos (`ERST Max`).
    pub max_entradas_erst: u16,
    /// Numero de buffers de scratchpad que pide el controlador.
    pub max_scratchpads: u16,
    /// El controlador usa punteros de 64 bits (`AC64`).
    pub ac64: bool,
    /// El contexto de dispositivo ocupa 64 bytes (`CSZ`).
    pub contexto_64: bool,
    /// El controlador tiene conmutadores de alimentacion de puerto (`PPC`).
    pub ppc: bool,
    /// El controlador admite el reinicio ligero (`LHRC`).
    pub lhrc: bool,
    /// El controlador no usa flujos secundarios (`NSS`).
    pub nss: bool,
    /// `log2` del tamano de pagina que usa el controlador.
    pub log2_pagina: u32,
    /// Desplazamiento, en bytes desde `Base`, de la base operacional.
    pub base_operacional: usize,
    /// Desplazamiento, en bytes desde `Base`, de los registros de ejecucion.
    pub base_ejecucion: usize,
    /// Desplazamiento, en bytes desde `Base`, de la array de campanas.
    pub base_campanas: usize,
    /// Desplazamiento, en bytes desde `Base`, de `USBLEGSUP`, si existe.
    pub usblegsup: Option<usize>,
    /// Puertos y protocolos de cada puerto raiz.
    pub protocolos: Protocolos,
}

impl Capacidades {
    /// Lee y valida las capacidades de un controlador.
    ///
    /// `reg` es la ventana de registros del controlador (la base del BAR).
    /// Devuelve el error concreto cuando algo no cuadra: no hay suposiciones
    /// ni valores por defecto en los datos que salen de aqui.
    pub fn leer(reg: &impl Registro) -> Result<Capacidades, ErrorCap> {
        // El dword 0 lleva CAPLENGTH en 7:0 y HCIVERSION en 31:16 (§5.3.1 y
        // §5.3.2).
        let dword0 = reg
            .leer32(CAP_CAPLENGTH)
            .ok_or(ErrorCap::RegistroInaccesible)?;
        let caplength = (dword0 & 0xFF) as u8;
        let version = ((dword0 >> 16) & 0xFFFF) as u16;

        if caplength < CAPLENGTH_MINIMO {
            return Err(ErrorCap::CapLengthInvalido);
        }
        // La base operacional tiene que quedar alineada a dword (§5.4).
        if caplength % 4 != 0 {
            return Err(ErrorCap::CapLengthInvalido);
        }
        // La version mayor es el byte alto del BCD: 0x0100 = 1.0.
        if (version >> 8) < 1 {
            return Err(ErrorCap::VersionInvalida);
        }

        let hcsparams1 = reg
            .leer32(CAP_HCSPARAMS1)
            .ok_or(ErrorCap::RegistroInaccesible)?;
        let hcsparams2 = reg
            .leer32(CAP_HCSPARAMS2)
            .ok_or(ErrorCap::RegistroInaccesible)?;
        let hccparams1 = reg
            .leer32(CAP_HCCPARAMS1)
            .ok_or(ErrorCap::RegistroInaccesible)?;
        let dboff = reg.leer32(CAP_DBOFF).ok_or(ErrorCap::RegistroInaccesible)?;
        let rtsoff = reg
            .leer32(CAP_RTSOFF)
            .ok_or(ErrorCap::RegistroInaccesible)?;

        // MaxSlots es el numero de huecos (1..255); cero esta reservado.
        let max_slots = (hcsparams1 & HCS1_MAXSLOTS_MASCARA) as u8;
        if max_slots == 0 {
            return Err(ErrorCap::MaxSlotsInvalido);
        }
        // MaxIntrs es el numero de interrumpers; cero esta indefinido.
        let max_intrs =
            ((hcsparams1 & HCS1_MAXINTRS_MASCARA) >> HCS1_MAXINTRS_DESPLAZAMIENTO) as u16;
        if max_intrs == 0 {
            return Err(ErrorCap::MaxIntrosInvalido);
        }
        // MaxPorts es el numero de puerto mas alto, no un contador.
        let max_ports = (hcsparams1 >> HCS1_MAXPORTS_DESPLAZAMIENTO) as u8;
        if max_ports == 0 {
            return Err(ErrorCap::MaxPuertosInvalido);
        }

        // ERST Max es un exponente: el maximo de entradas es 2^ERST Max.
        let erst_max = ((hcsparams2 & HCS2_ERSTMAX_MASCARA) >> HCS2_ERSTMAX_DESPLAZAMIENTO) as u32;
        let max_entradas_erst = 1u32 << erst_max;

        // El numero de scratchpads es un campo de 10 bits repartido en dos.
        let scratch_hi = (hcsparams2 & HCS2_SCRATCH_HI_MASCARA) >> HCS2_SCRATCH_HI_DESPLAZAMIENTO;
        let scratch_lo = (hcsparams2 & HCS2_SCRATCH_LO_MASCARA) >> HCS2_SCRATCH_LO_DESPLAZAMIENTO;
        let max_scratchpads = ((scratch_hi << 5) | scratch_lo) as u16;

        let ac64 = hccparams1 & HCC1_AC64 != 0;
        let contexto_64 = hccparams1 & HCC1_CSZ != 0;
        let ppc = hccparams1 & HCC1_PPC != 0;
        let lhrc = hccparams1 & HCC1_LHRC != 0;
        let nss = hccparams1 & HCC1_NSS != 0;
        let xecp = ((hccparams1 & HCC1_XECP_MASCARA) >> HCC1_XECP_DESPLAZAMIENTO) as u16;

        // Los desplazamientos se limpian de los bits reservados: `DBOFF` son
        // bytes con los bits 1:0 reservados (§5.3.7) y `RTSOFF` son bytes con
        // los bits 4:0 reservados (§5.3.8).
        let db_off = (dboff & 0xFFFF_FFFC) as usize;
        let rtsoff = (rtsoff & 0xFFFF_FFE0) as usize;
        if db_off == 0 || rtsoff == 0 {
            return Err(ErrorCap::DesplazamientoInvalido);
        }

        // `CAPLENGTH` es el desplazamiento, en bytes, hasta los registros
        // operacionales (§5.3.1).
        let base_operacional = caplength as usize;
        let base_ejecucion = rtsoff;
        let base_campanas = db_off;

        // El tamano de pagina se lee ya en la base operacional (§5.4.3).
        let pagesize = reg
            .leer32(base_operacional + OP_PAGESIZE)
            .ok_or(ErrorCap::RegistroInaccesible)?;
        let log2_pagina = log2_pagina(pagesize);

        // Las capacidades extendidas empiezan en `xECP` (dwords desde `Base`).
        let mut usblegsup = None;
        let mut protocolos = Protocolos::default();
        let mut hubo_protocolos = false;

        if xecp != 0 {
            let mut off = xecp as usize * 4;
            // Tope de visitas: una lista sana tiene un par de entradas. El tope
            // corta un bucle (un puntero que se apunta a si mismo).
            for _ in 0..32 {
                let dword = match reg.leer32(off) {
                    Some(d) => d,
                    None => return Err(ErrorCap::CapacidadesExtendidasInvalidas),
                };
                let id = (dword & EXT_ID_MASCARA) as u8;
                let siguiente =
                    ((dword & EXT_SIGUIENTE_MASCARA) >> EXT_SIGUIENTE_DESPLAZAMIENTO) as usize;

                match id {
                    EXT_LEGACY => {
                        usblegsup = Some(off);
                    }
                    EXT_PROTOCOLOS => {
                        let (protocolo, rango) = leer_protocolos(reg, off)?;
                        match protocolo {
                            Some(Protocolo::Usb2) => protocolos.usb2 = Some(rango),
                            Some(Protocolo::Usb3) => protocolos.usb3 = Some(rango),
                            None => {}
                        }
                        hubo_protocolos = true;
                    }
                    _ => {}
                }

                if siguiente == 0 {
                    break;
                }
                let nuevo = off
                    .checked_add(siguiente * 4)
                    .ok_or(ErrorCap::CapacidadesExtendidasInvalidas)?;
                if nuevo <= off {
                    return Err(ErrorCap::CapacidadesExtendidasInvalidas);
                }
                off = nuevo;
            }
        }

        if !hubo_protocolos {
            return Err(ErrorCap::SinProtocolos);
        }
        validar_protocolos(&protocolos, max_ports)?;

        Ok(Capacidades {
            version,
            max_slots,
            max_intrs,
            max_ports,
            max_entradas_erst: max_entradas_erst as u16,
            max_scratchpads,
            ac64,
            contexto_64,
            ppc,
            lhrc,
            nss,
            log2_pagina,
            base_operacional,
            base_ejecucion,
            base_campanas,
            usblegsup,
            protocolos,
        })
    }

    /// Tamano de pagina, en bytes, que usa el controlador.
    pub fn tam_pagina(&self) -> usize {
        1usize << self.log2_pagina
    }

    /// ¿Caben todos los registros que vamos a usar en una ventana de `tamano`
    /// bytes?
    ///
    /// Comprueba la base operacional (con sus puertos), la base de ejecucion
    /// (con sus interrumpers) y la array de campanas. Un BAR mas pequeno que
    /// lo que declara el controlador no se puede usar.
    pub fn cabe_en(&self, tamano: usize) -> bool {
        let puertos = self.max_ports as usize * PUERTO_PASO;
        let fin_operacional = self
            .base_operacional
            .checked_add(OP_PUERTOS)
            .and_then(|v| v.checked_add(puertos));
        let fin_ejecucion = self
            .base_ejecucion
            .checked_add(RT_INTERRUPTER)
            .and_then(|v| v.checked_add(INTERRUPTER_PASO));
        let fin_campanas = self.base_campanas.checked_add(4);

        match (fin_operacional, fin_ejecucion, fin_campanas) {
            (Some(a), Some(b), Some(c)) => a <= tamano && b <= tamano && c <= tamano,
            _ => false,
        }
    }
}

/// `log2` del tamano de pagina que declara el registro `PAGESIZE`.
///
/// El bit `n` a uno significa que se admite `2^(n+12)` (§5.4.3). Si el
/// registro valiese cero, se usa el tamano obligatorio de 4 KiB.
pub fn log2_pagina(pagesize: u32) -> u32 {
    let mask = pagesize & PAGESIZE_MASCARA;
    if mask == 0 {
        return 12;
    }
    // El bit mas alto a uno es el tamano mas grande que admite el controlador.
    12 + (31 - mask.leading_zeros())
}

/// Lee una capacidad de "protocolos soportados" (§7.2).
///
/// Devuelve el protocolo y el rango de puertos que declara la capacidad.
fn leer_protocolos(
    reg: &impl Registro,
    off: usize,
) -> Result<(Option<Protocolo>, RangoPuertos), ErrorCap> {
    // Dword 0: identificador, puntero a la siguiente, revision mayor y menor.
    let dword0 = reg
        .leer32(off)
        .ok_or(ErrorCap::CapacidadesExtendidasInvalidas)?;
    let mayor = (dword0 >> PROTOCOLO_MAYOR_DESPLAZAMIENTO) & 0xFF;

    // Dword 2: puerto inicial compatible, numero de puertos compatibles y
    // numero de palabras de velocidad de protocolo.
    let dword2 = reg
        .leer32(off + 8)
        .ok_or(ErrorCap::CapacidadesExtendidasInvalidas)?;
    let primero = (dword2 & PROTOCOLO_OFFSET_MASCARA) as u8;
    let cantidad = ((dword2 & PROTOCOLO_CUENTA_MASCARA) >> PROTOCOLO_CUENTA_DESPLAZAMIENTO) as u8;
    let psic = (dword2 >> PROTOCOLO_PSIC_DESPLAZAMIENTO) & 0xF;

    if primero == 0 || cantidad == 0 {
        return Err(ErrorCap::ProtocolosInconsistentes);
    }

    // La tabla 7-11 identifica el protocolo por la version mayor: 02h es
    // USB 2.0 y 03h es USB 3.x.
    let rango = RangoPuertos { primero, cantidad };
    let protocolo = match mayor {
        2 => Some(Protocolo::Usb2),
        3 => Some(Protocolo::Usb3),
        // Un protocolo que no conocemos (por ejemplo USB 4.0) no se puede
        // usar como USB 2.0: se ignora su rango.
        _ => None,
    };

    // Las palabras de velocidad de protocolo se leen para validar que la
    // capacidad esta completa, pero no se usan todavia.
    for i in 0..psic as usize {
        reg.leer32(off + 0x10 + i * 4)
            .ok_or(ErrorCap::CapacidadesExtendidasInvalidas)?;
    }

    Ok((protocolo, rango))
}

/// Comprueba que los rangos de puertos no se solapen y cubren lo que deben.
fn validar_protocolos(p: &Protocolos, max_ports: u8) -> Result<(), ErrorCap> {
    if let (Some(a), Some(b)) = (p.usb2, p.usb3) {
        let fin_a = a.primero as u16 + a.cantidad as u16;
        let fin_b = b.primero as u16 + b.cantidad as u16;
        let solapan = (a.primero as u16..fin_a).any(|p| p >= b.primero as u16 && p < fin_b)
            || (b.primero as u16..fin_b).any(|p| p >= a.primero as u16 && p < fin_a);
        if solapan {
            return Err(ErrorCap::ProtocolosInconsistentes);
        }
    }
    // El puerto 1 tiene que estar en algun rango: la especificacion exige que
    // una de las capacidades tenga el desplazamiento 1.
    if p.protocolo_de(1).is_none() {
        return Err(ErrorCap::ProtocolosInconsistentes);
    }
    // Ningun puerto declarado puede pasar de `MaxPorts`.
    let total = p.total();
    if total == 0 || total > max_ports as u16 {
        return Err(ErrorCap::ProtocolosInconsistentes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // La libreria es `no_std` (va dentro del kernel), pero las pruebas si
    // pueden usar la biblioteca estandar.
    extern crate std;
    use std::vec::Vec;

    /// Un mapa de registros de mentira: pares (desplazamiento, valor).
    struct Mapa {
        celdas: Vec<(usize, u32)>,
    }

    impl Mapa {
        fn nuevo() -> Mapa {
            Mapa { celdas: Vec::new() }
        }

        fn con(mut self, off: usize, valor: u32) -> Mapa {
            self.celdas.push((off, valor));
            self
        }

        /// Escribe un dword de la base operacional.
        fn op(self, off: usize, valor: u32) -> Mapa {
            self.con(0x40 + off, valor)
        }
    }

    impl Registro for Mapa {
        fn leer32(&self, off: usize) -> Option<u32> {
            self.celdas.iter().find(|(o, _)| *o == off).map(|(_, v)| *v)
        }
    }

    /// Un controlador xHCI de mentira, con 4 huecos, 8 puertos y 2 USB2 + 2 USB3.
    fn controlador() -> Mapa {
        let mut m = Mapa::nuevo();
        // dword 0: CAPLENGTH = 0x40 bytes, HCIVERSION = 0x0110.
        m = m.con(0x00, 0x0110_0040);
        // HCSPARAMS1: MaxSlots = 64, MaxIntrs = 1, MaxPorts = 8.
        m = m.con(CAP_HCSPARAMS1, (8 << 24) | (1 << 8) | 64);
        // HCSPARAMS2: ERST Max = 7 (128 entradas), 1 scratchpad.
        m = m.con(CAP_HCSPARAMS2, (7 << 4) | (1 << 27));
        // HCCPARAMS1: AC64 y CSZ; xECP = 0x40 dwords -> offset 0x100.
        m = m.con(CAP_HCCPARAMS1, HCC1_AC64 | HCC1_CSZ | (0x40 << 16));
        // DBOFF = 0x1000 bytes, RTSOFF = 0x2000 bytes.
        m = m.con(CAP_DBOFF, 0x1000);
        m = m.con(CAP_RTSOFF, 0x2000);
        // PAGESIZE: solo 4 KiB (bit 0).
        m = m.op(OP_PAGESIZE, 1);
        m
    }

    #[test]
    fn lee_las_campos_basicos() {
        let mut m = controlador();
        // Capacidad de protocolos soportados en 0x100: USB 2.0 (revision mayor
        // 02h), puerto inicial 1, 2 puertos. El puntero a la siguiente
        // capacidad esta en los bits 15:8 y cuenta dwords.
        m = m.con(0x100, (2 << 24) | EXT_PROTOCOLOS as u32 | (4 << 8));
        m = m.con(0x104, 0);
        m = m.con(0x108, (2 << 8) | 1);
        m = m.con(0x10C, 0);
        // Capacidad USB 3.x en 0x110: puerto inicial 3, 2 puertos.
        m = m.con(0x110, (3 << 24) | EXT_PROTOCOLOS as u32);
        m = m.con(0x114, 0);
        m = m.con(0x118, (2 << 8) | 3);
        m = m.con(0x11C, 0);

        let c = Capacidades::leer(&m).expect("deberia leer");
        assert_eq!(c.version, 0x0110);
        assert_eq!(c.max_slots, 64);
        assert_eq!(c.max_intrs, 1);
        assert_eq!(c.max_ports, 8);
        assert_eq!(c.max_entradas_erst, 128);
        assert_eq!(c.max_scratchpads, 1);
        assert!(c.ac64);
        assert!(c.contexto_64);
        assert_eq!(c.log2_pagina, 12);
        assert_eq!(c.base_operacional, 0x40);
        assert_eq!(c.base_ejecucion, 0x2000);
        assert_eq!(c.base_campanas, 0x1000);
        assert_eq!(
            c.protocolos.usb2,
            Some(RangoPuertos {
                primero: 1,
                cantidad: 2
            })
        );
        assert_eq!(
            c.protocolos.usb3,
            Some(RangoPuertos {
                primero: 3,
                cantidad: 2
            })
        );
        assert_eq!(c.protocolos.protocolo_de(1), Some(Protocolo::Usb2));
        assert_eq!(c.protocolos.protocolo_de(2), Some(Protocolo::Usb2));
        assert_eq!(c.protocolos.protocolo_de(3), Some(Protocolo::Usb3));
        assert_eq!(c.protocolos.protocolo_de(9), None);
    }

    #[test]
    fn rechaza_caplength_corto() {
        let m = Mapa::nuevo().con(0x00, 0x0110_0008);
        assert_eq!(Capacidades::leer(&m), Err(ErrorCap::CapLengthInvalido));
    }

    #[test]
    fn rechaza_caplength_desalineado() {
        // La base operacional tiene que quedar alineada a dword.
        let m = Mapa::nuevo().con(0x00, 0x0110_0022);
        assert_eq!(Capacidades::leer(&m), Err(ErrorCap::CapLengthInvalido));
    }

    #[test]
    fn comprueba_que_los_registros_caben_en_la_ventana() {
        let mut m = controlador();
        m = m.con(0x100, (2 << 24) | EXT_PROTOCOLOS as u32 | (4 << 8));
        m = m.con(0x108, (2 << 8) | 1);
        m = m.con(0x110, (3 << 24) | EXT_PROTOCOLOS as u32);
        m = m.con(0x118, (2 << 8) | 3);
        let c = Capacidades::leer(&m).expect("deberia leer");

        // Base operacional 0x40 + 0x400 + 8 puertos * 0x10 = 0x4C0.
        // Base de ejecucion 0x2000 + 0x20 + 0x20 = 0x2040.
        // Array de campanas 0x1000 + 4 = 0x1004.
        assert!(c.cabe_en(0x2040));
        assert!(!c.cabe_en(0x203F));
        assert!(!c.cabe_en(0x4C0));
    }

    #[test]
    fn rechaza_version_antigua() {
        let m = Mapa::nuevo().con(0x00, 0x0090_0040);
        assert_eq!(Capacidades::leer(&m), Err(ErrorCap::VersionInvalida));
    }

    #[test]
    fn rechaza_max_slots_cero() {
        let mut m = controlador();
        m.celdas.retain(|(o, _)| *o != CAP_HCSPARAMS1);
        m = m.con(CAP_HCSPARAMS1, 8 << 24);
        assert_eq!(Capacidades::leer(&m), Err(ErrorCap::MaxSlotsInvalido));
    }

    #[test]
    fn rechaza_max_ports_cero() {
        let mut m = controlador();
        m.celdas.retain(|(o, _)| *o != CAP_HCSPARAMS1);
        m = m.con(CAP_HCSPARAMS1, 1 << 8 | 64);
        assert_eq!(Capacidades::leer(&m), Err(ErrorCap::MaxPuertosInvalido));
    }

    #[test]
    fn rechaza_registro_ausente() {
        let m = Mapa::nuevo();
        assert_eq!(Capacidades::leer(&m), Err(ErrorCap::RegistroInaccesible));
    }

    #[test]
    fn rechaza_sin_capacidad_de_protocolos() {
        let mut m = controlador();
        // Sin xECP no hay capacidades extendidas: falta el dato obligatorio.
        m.celdas.retain(|(o, _)| *o != CAP_HCCPARAMS1);
        m = m.con(CAP_HCCPARAMS1, HCC1_AC64 | HCC1_CSZ);
        assert_eq!(Capacidades::leer(&m), Err(ErrorCap::SinProtocolos));
    }

    #[test]
    fn detecta_el_semoforo_de_la_bios() {
        let mut m = controlador();
        // Capacidad USB heredado en 0x100, que apunta a la de protocolos.
        m = m.con(0x100, EXT_LEGACY as u32 | (4 << 8));
        m = m.con(0x104, 0);
        m = m.con(0x108, (2 << 8) | 1);
        m = m.con(0x10C, 0);
        m = m.con(0x110, (2 << 24) | EXT_PROTOCOLOS as u32);
        m = m.con(0x114, 0);
        m = m.con(0x118, (2 << 8) | 1);
        m = m.con(0x11C, 0);

        let c = Capacidades::leer(&m).expect("deberia leer");
        assert_eq!(c.usblegsup, Some(0x100));
    }

    #[test]
    fn rechaza_protocolos_solapados() {
        let mut m = controlador();
        // USB2 cubre los puertos 1..3 y USB3 tambien el 3.
        m = m.con(0x100, (2 << 24) | EXT_PROTOCOLOS as u32 | (4 << 8));
        m = m.con(0x104, 0);
        m = m.con(0x108, (3 << 8) | 1);
        m = m.con(0x10C, 0);
        m = m.con(0x110, (3 << 24) | EXT_PROTOCOLOS as u32);
        m = m.con(0x114, 0);
        m = m.con(0x118, (2 << 8) | 2);
        m = m.con(0x11C, 0);
        assert_eq!(
            Capacidades::leer(&m),
            Err(ErrorCap::ProtocolosInconsistentes)
        );
    }

    #[test]
    fn calcula_el_tamano_de_pagina() {
        assert_eq!(log2_pagina(0b0001), 12);
        assert_eq!(log2_pagina(0b0010), 13);
        assert_eq!(log2_pagina(0b0100), 14);
        assert_eq!(log2_pagina(0b1000), 15);
        assert_eq!(log2_pagina(0b0011), 13);
        assert_eq!(log2_pagina(0), 12);
    }
}
