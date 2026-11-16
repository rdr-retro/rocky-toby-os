//! Acceso acotado a la memoria de un dispositivo (MMIO).
//!
//! Los registros de un controlador no son memoria normal: escribir en ellos
//! cambia el estado del dispositivo. Por eso no se puede tratarlos como un
//! `&mut [u8]` normal (el compilador podria reordenar o eliminar accesos) y
//! menos como una referencia de tamano ilimitado (un `offset` equivocado
//! escribiria en el registro de al lado).
//!
//! [`Mmio`] resuelve las dos cosas:
//!
//! - Los accesos son **volatiles** y de ancho fijo (`leer32`, `escribir32`...),
//!   asi que el compilador no los reordena ni los elimina.
//! - Cada acceso **comprueba el rango** contra el tamano del BAR antes de
//!   tocar nada, y devuelve un error si se sale. No hay forma de leer fuera
//!   de la ventana.
//!
//! ## Orden de memoria
//!
//! En x86_64 las escrituras a memoria son ordenadas con respecto a las
//! escrituras a MMIO: no hace falta ninguna barrera para que un `write` de un
//! registro llegue antes que el siguiente `write` de MMIO. Lo que si hace
//! falta son barreras explicitas en los puntos en los que el **dispositivo**
//! tiene que ver un estado coherente, o en los que el CPU tiene que ver lo que
//! el dispositivo escribio. Para eso estan [`sfence`] (ordena las
//! escrituras) y [`mfence`] (ordena escrituras y lecturas).
//!
//! ## Coherencia de DMA
//!
//! **Hipotesis de rocky:** en x86_64 la cache del CPU es coherente con la
//! memoria del sistema, y el bus PCI tambien. Un dispositivo que hace DMA ve
//! lo que el CPU ha escrito y viceversa, sin necesidad de lavar la cache a
//! mano. Por eso esta libreria **no** anade flushes de cache: anadirlos solo
//! costaria tiempo.
//!
//! Esa hipotesis es valida mientras el DMA use direcciones fisicas directas.
//! Si el equipo tiene una IOMMU (VT-d/VT-x) activada y no la tenemos
//! configurada, las direcciones que el dispositivo ve **no** son las fisicas,
//! y el DMA se sale a un sitio cualquiera. Rocky no configura ninguna IOMMU:
//! por eso, antes de activar Bus Master hay que comprobar que la maquina no
//! tiene una IOMMU en uso, y si la tiene, ese controlador se descarta y se
//! sigue con PS/2.
//!
//! Codigo propio de rocky.

use crate::pci::Bar;

/// Tamano de pagina (4 KiB). Es la alineacion natural de una ventana MMIO.
pub const PAGE_SIZE: usize = 4096;

/// Por que un acceso MMIO no se puede hacer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorMmio {
    /// La ventana no tiene base o no tiene tamano.
    VentanaInvalida,
    /// La base no esta alineada a 4 bytes, que es lo que exigen los registros.
    BaseDesalineada,
    /// El BAR es de espacio de E/S: sus registros no son memoria.
    NoEsMemoria,
    /// La ventana esta por encima del tope que cubre el mapa por identidad.
    FueraDelMapa,
    /// El acceso se sale de la ventana del BAR.
    FueraDeRango {
        /// Offset pedido dentro de la ventana.
        offset: usize,
        /// Longitud pedida.
        len: usize,
    },
}

/// Una ventana de registros de un dispositivo, con los limites del BAR.
///
/// No se puede construir a mano con una referencia de Rust: la ventana se
/// guarda como base y tamano, y cada acceso los comprueba. Asi una referencia
/// a un registro que no existe no se puede ni leer ni escribir.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mmio {
    /// Direccion fisica base de la ventana.
    base: usize,
    /// Tamano de la ventana en bytes.
    size: usize,
}

impl Mmio {
    /// Crea una ventana MMIO a partir de una base y un tamano.
    ///
    /// La base tiene que estar alineada a 4 bytes (los registros del
    /// controlador se leen y escriben de 4 en 4 como minimo).
    pub fn nueva(base: usize, size: usize) -> Result<Mmio, ErrorMmio> {
        if base == 0 || size == 0 {
            return Err(ErrorMmio::VentanaInvalida);
        }
        if base % 4 != 0 {
            return Err(ErrorMmio::BaseDesalineada);
        }
        // La ventana tiene que caber en el rango de direcciones.
        if (base as u64).checked_add(size as u64).is_none() {
            return Err(ErrorMmio::VentanaInvalida);
        }
        Ok(Mmio { base, size })
    }

    /// Crea una ventana MMIO que tiene que caber en el mapa por identidad.
    ///
    /// `limite` es el tope **exclusivo** que cubre el mapa. Con `1 << 32` la
    /// ventana tiene que quedar por debajo de 4 GiB, que es lo que prepara el
    /// arranque. Una ventana de 64 bits por encima de ese limite no se puede
    /// leer sin ampliar la paginacion, asi que se rechaza aqui y no mas tarde.
    pub fn nueva_en_mapa(base: usize, size: usize, limite: u64) -> Result<Mmio, ErrorMmio> {
        let m = Mmio::nueva(base, size)?;
        if !m.cabe_en_identidad(limite) {
            return Err(ErrorMmio::FueraDelMapa);
        }
        Ok(m)
    }

    /// Crea la ventana a partir de un BAR de memoria ya medido.
    ///
    /// Un BAR de espacio de E/S no sirve: sus registros no son memoria y hay
    /// que leerlos por puertos.
    pub fn de_bar(bar: &Bar) -> Result<Mmio, ErrorMmio> {
        if bar.es_io {
            return Err(ErrorMmio::NoEsMemoria);
        }
        Mmio::nueva(bar.base as usize, bar.size as usize)
    }

    /// Crea la ventana a partir de un BAR de memoria medido, exigiendo que
    /// quepa en el mapa por identidad.
    pub fn de_bar_en_mapa(bar: &Bar, limite: u64) -> Result<Mmio, ErrorMmio> {
        if bar.es_io {
            return Err(ErrorMmio::NoEsMemoria);
        }
        Mmio::nueva_en_mapa(bar.base as usize, bar.size as usize, limite)
    }

    /// Direccion fisica base de la ventana.
    pub fn base(&self) -> usize {
        self.base
    }

    /// Tamano de la ventana en bytes.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Direccion del primer byte posterior a la ventana.
    pub fn fin(&self) -> usize {
        self.base + self.size
    }

    /// ¿La ventana completa cabe por debajo de `limite` (tope exclusivo)?
    ///
    /// Con `limite = 1 << 32` se comprueba que la ventana esta por debajo de
    /// 4 GiB, que es lo que cubre el mapa por identidad del arranque.
    pub fn cabe_en_identidad(&self, limite: u64) -> bool {
        self.fin() as u64 <= limite
    }

    /// Comprueba que un acceso (offset y longitud) cae dentro de la ventana.
    pub fn comprobar(&self, offset: usize, len: usize) -> Result<(), ErrorMmio> {
        let fin = (offset as u64).checked_add(len as u64);
        match fin {
            Some(fin) if fin <= self.size as u64 => Ok(()),
            _ => Err(ErrorMmio::FueraDeRango { offset, len }),
        }
    }

    /// Direccion absoluta de un offset dentro de la ventana.
    fn direccion(&self, offset: usize) -> usize {
        self.base + offset
    }

    /// Lee un byte de un registro.
    pub fn leer8(&self, offset: usize) -> Result<u8, ErrorMmio> {
        self.comprobar(offset, 1)?;
        // Seguro: el rango se ha comprobado contra el tamano del BAR y la
        // ventana es memoria de dispositivo mapeada.
        Ok(unsafe { core::ptr::read_volatile(self.direccion(offset) as *const u8) })
    }

    /// Lee 16 bits de un registro.
    pub fn leer16(&self, offset: usize) -> Result<u16, ErrorMmio> {
        self.comprobar(offset, 2)?;
        Ok(unsafe { core::ptr::read_volatile(self.direccion(offset) as *const u16) })
    }

    /// Lee 32 bits de un registro.
    pub fn leer32(&self, offset: usize) -> Result<u32, ErrorMmio> {
        self.comprobar(offset, 4)?;
        Ok(unsafe { core::ptr::read_volatile(self.direccion(offset) as *const u32) })
    }

    /// Lee 64 bits de un registro.
    pub fn leer64(&self, offset: usize) -> Result<u64, ErrorMmio> {
        self.comprobar(offset, 8)?;
        Ok(unsafe { core::ptr::read_volatile(self.direccion(offset) as *const u64) })
    }

    /// Escribe un byte en un registro.
    pub fn escribir8(&self, offset: usize, valor: u8) -> Result<(), ErrorMmio> {
        self.comprobar(offset, 1)?;
        unsafe { core::ptr::write_volatile(self.direccion(offset) as *mut u8, valor) }
        Ok(())
    }

    /// Escribe 16 bits en un registro.
    pub fn escribir16(&self, offset: usize, valor: u16) -> Result<(), ErrorMmio> {
        self.comprobar(offset, 2)?;
        unsafe { core::ptr::write_volatile(self.direccion(offset) as *mut u16, valor) }
        Ok(())
    }

    /// Escribe 32 bits en un registro.
    pub fn escribir32(&self, offset: usize, valor: u32) -> Result<(), ErrorMmio> {
        self.comprobar(offset, 4)?;
        unsafe { core::ptr::write_volatile(self.direccion(offset) as *mut u32, valor) }
        Ok(())
    }

    /// Escribe 64 bits en un registro.
    pub fn escribir64(&self, offset: usize, valor: u64) -> Result<(), ErrorMmio> {
        self.comprobar(offset, 8)?;
        unsafe { core::ptr::write_volatile(self.direccion(offset) as *mut u64, valor) }
        Ok(())
    }

    /// Copia un tramo de la ventana a un buffer normal, byte a byte.
    ///
    /// Se lee de uno en uno y de forma volatil: una copia de bloque sobre
    /// memoria de dispositivo podria ser cacheada o reordenada.
    pub fn copiar_a(&self, offset: usize, destino: &mut [u8]) -> Result<(), ErrorMmio> {
        self.comprobar(offset, destino.len())?;
        for (i, b) in destino.iter_mut().enumerate() {
            *b = unsafe { core::ptr::read_volatile(self.direccion(offset + i) as *const u8) };
        }
        Ok(())
    }

    /// Escribe un buffer normal en un tramo de la ventana, byte a byte.
    pub fn copiar_desde(&self, offset: usize, origen: &[u8]) -> Result<(), ErrorMmio> {
        self.comprobar(offset, origen.len())?;
        for (i, b) in origen.iter().enumerate() {
            unsafe { core::ptr::write_volatile(self.direccion(offset + i) as *mut u8, *b) };
        }
        Ok(())
    }
}

/// Barrera de orden de las escrituras (`SFENCE`).
///
/// En x86_64 las escrituras ya van en orden, asi que esta barrera solo
/// documenta la intencion: se usa antes de avisar al dispositivo de que una
/// estructura ya esta lista.
#[inline]
pub fn sfence() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_sfence();
    }
}

/// Barrera de orden de escrituras y lecturas (`MFENCE`).
///
/// Se usa cuando el CPU tiene que ver lo que el dispositivo ha escrito, o
/// cuando el dispositivo tiene que ver lo que el CPU ha escrito y hay una
/// lectura en medio.
#[inline]
pub fn mfence() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_mfence();
    }
}

/// Descarta de la cache del procesador la linea que contiene la direccion.
///
/// En x86 la DMA escribe en la memoria, pero el procesador puede tener una
/// copia vieja en su cache de datos: si el CPU lee lo que ha escrito el
/// dispositivo sin tirar antes esa copia, lee lo que habia antes.
///
/// `CLFLUSH` descarta la linea **sin escribirla**: al contrario que
/// `CLFLUSHOPT`, no es una instruccion hinted, asi que el contenido de la
/// memoria no se pierde.
///
/// Hay que poner una `mfence` despues: `CLFLUSH` no ordena por si sola las
/// lecturas que vienen despues.
#[inline]
pub fn clflush(direccion: u64) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "clflush [{0}]",
            in(reg) direccion,
            options(nostack, preserves_flags, readonly)
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = direccion;
}

/// Descarta de la cache todas las lineas de un rango.
///
/// El rango se redondea a lineas completas de 64 bytes, que es el tamano de la
/// linea de cache de datos en x86.
///
/// Sirve para los dos sentidos de la DMA:
///
/// - Antes de que el dispositivo lea: el CPU ha escrito la estructura y el
///   dispositivo tiene que ver esos bytes, no la copia de la cache.
/// - Despues de que el dispositivo escriba: el CPU tiene que leer de la
///   memoria lo que el dispositivo ha puesto, no su copia vieja.
pub fn clflush_rango(direccion: u64, bytes: usize) {
    if bytes == 0 {
        return;
    }
    let inicio = direccion & !63;
    let fin = (direccion.saturating_add(bytes as u64).saturating_add(63)) & !63;
    let mut dir = inicio;
    while dir < fin {
        clflush(dir);
        dir += 64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crea_una_ventana_valida() {
        let m = Mmio::nueva(0xE000_0000, 0x1000).expect("ventana valida");
        assert_eq!(m.base(), 0xE000_0000);
        assert_eq!(m.size(), 0x1000);
        assert_eq!(m.fin(), 0xE000_1000);
    }

    #[test]
    fn rechaza_una_ventana_sin_base_o_sin_tamano() {
        assert_eq!(Mmio::nueva(0, 0x1000), Err(ErrorMmio::VentanaInvalida));
        assert_eq!(Mmio::nueva(0xE000_0000, 0), Err(ErrorMmio::VentanaInvalida));
    }

    #[test]
    fn rechaza_una_base_desalineada() {
        assert_eq!(
            Mmio::nueva(0xE000_0001, 0x1000),
            Err(ErrorMmio::BaseDesalineada)
        );
    }

    #[test]
    fn rechaza_un_acceso_fuera_de_rango() {
        let m = Mmio::nueva(0xE000_0000, 0x1000).expect("ventana valida");
        // Justo dentro: vale.
        assert_eq!(m.comprobar(0xFFC, 4), Ok(()));
        assert_eq!(m.comprobar(0, 0x1000), Ok(()));
        // Se sale por la derecha.
        assert_eq!(
            m.comprobar(0xFFC, 8),
            Err(ErrorMmio::FueraDeRango {
                offset: 0xFFC,
                len: 8
            })
        );
        assert_eq!(
            m.comprobar(0x1000, 1),
            Err(ErrorMmio::FueraDeRango {
                offset: 0x1000,
                len: 1
            })
        );
        // Y por la izquierda, con un offset enorme.
        assert!(m.comprobar(usize::MAX, 1).is_err());
    }

    #[test]
    fn leer_y_escribir_un_registro_de_32_bits() {
        // El acceso de verdad va contra un array normal: lo que se prueba aqui
        // es el camino completo (comprobar -> volatile -> devolver).
        let mut buffer = [0u8; 64];
        buffer[0] = 0xAA;
        let base = buffer.as_ptr() as usize;
        let m = Mmio::nueva(base, buffer.len()).expect("ventana valida");
        m.escribir32(8, 0x1234_5678).expect("escritura valida");
        assert_eq!(m.leer32(8).expect("lectura valida"), 0x1234_5678);
        // Y el buffer de verdad se ha modificado.
        assert_eq!(&buffer[8..12], &0x1234_5678u32.to_le_bytes());
        // Lo que no se ha tocado, sigue igual.
        assert_eq!(buffer[0], 0xAA);
    }

    #[test]
    fn un_acceso_fuera_de_rango_no_escribe_nada() {
        let mut buffer = [0u8; 64];
        buffer[0] = 0xAA;
        let base = buffer.as_ptr() as usize;
        let m = Mmio::nueva(base, buffer.len()).expect("ventana valida");
        // Offset 62: 62 + 4 se sale de la ventana de 64 bytes.
        assert!(m.escribir32(62, 0xFFFF_FFFF).is_err());
        // No ha escrito nada.
        assert_eq!(buffer[0], 0xAA);
        assert_eq!(buffer[62], 0);
        assert_eq!(buffer[63], 0);
        assert!(m.leer32(62).is_err());
    }

    #[test]
    fn copia_una_ventana_a_un_buffer() {
        let mut buffer = [0u8; 64];
        for (i, b) in buffer.iter_mut().enumerate() {
            *b = i as u8;
        }
        let base = buffer.as_ptr() as usize;
        let m = Mmio::nueva(base, buffer.len()).expect("ventana valida");
        let mut destino = [0u8; 16];
        m.copiar_a(8, &mut destino).expect("copia valida");
        assert_eq!(
            destino,
            [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23]
        );
        // Y una copia que se sale, no.
        assert!(m.copiar_a(60, &mut [0u8; 16]).is_err());
    }

    #[test]
    fn comprueba_si_la_ventana_cabe_en_el_mapa_por_identidad() {
        let m = Mmio::nueva(0xE000_0000, 0x1000).expect("ventana valida");
        assert!(m.cabe_en_identidad(1 << 32));
        // Una ventana de 64 bits por encima de 4 GiB no la cubre el arranque.
        let m = Mmio::nueva(0x1_0000_0000, 0x1000).expect("ventana valida");
        assert!(!m.cabe_en_identidad(1 << 32));
    }
    #[test]
    fn un_bar_de_espacio_de_es_no_es_una_ventana_de_memoria() {
        let bar = Bar {
            base: 0xC0,
            size: 0x20,
            es_io: true,
            es_64: false,
        };
        assert_eq!(Mmio::de_bar(&bar), Err(ErrorMmio::NoEsMemoria));
    }

    #[test]
    fn un_bar_de_memoria_da_la_ventana() {
        let bar = Bar {
            base: 0xE000_0000,
            size: 0x1000,
            es_io: false,
            es_64: true,
        };
        let m = Mmio::de_bar(&bar).expect("ventana valida");
        assert_eq!(m.base(), 0xE000_0000);
        assert_eq!(m.size(), 0x1000);
    }

    #[test]
    fn una_ventana_por_encima_de_4_gib_no_se_abre() {
        // Un BAR de 64 bits por encima de 4 GiB: el mapa por identidad del
        // arranque no lo cubre, asi que no se puede usar.
        let bar = Bar {
            base: 0x1_0000_0000,
            size: 0x1000,
            es_io: false,
            es_64: true,
        };
        assert_eq!(
            Mmio::de_bar_en_mapa(&bar, 1 << 32),
            Err(ErrorMmio::FueraDelMapa)
        );
        // Por debajo de 4 GiB, si.
        let bar = Bar {
            base: 0xE000_0000,
            size: 0x1000,
            es_io: false,
            es_64: true,
        };
        assert!(Mmio::de_bar_en_mapa(&bar, 1 << 32).is_ok());
    }
}
