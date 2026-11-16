//! Tipos del bus PCI y decodificacion de sus registros.
//!
//! Aqui vive lo que **no** depende del hardware: que clase de controlador de
//! host USB es una funcion, como se interpreta el par de registros de un BAR y
//! cuando la ventana que describe ese BAR es utilizable. El barrido del bus y
//! el acceso al espacio de configuracion (`0xCF8`/`0xCFC`) siguen en el kernel.
//!
//! Decodificar aqui, y no en el driver, tiene una ventaja: las reglas se
//! pueden probar en el anfitrion sin ningun bus PCI delante.
//!
//! Codigo propio de rocky.

/// Clase de dispositivo "bus serie" (USB, SMBus, LPC...).
pub const CLASE_BUS_SERIE: u8 = 0x0C;
/// Subclase "controlador de host USB" dentro de la clase de bus serie.
pub const SUBCLASE_USB: u8 = 0x03;
/// Clase de dispositivo "video".
pub const CLASE_VIDEO: u8 = 0x03;

/// Una funcion PCI (lo que en el lenguaje comun se llama "un dispositivo").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Funcion {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub revision: u8,
    pub clase: u8,
    pub subclase: u8,
    pub prog_if: u8,
}

impl Funcion {
    /// Funcion vacia: la que se usa para rellenar los huecos de la tabla.
    pub const VACIA: Funcion = Funcion {
        bus: 0,
        dev: 0,
        func: 0,
        vendor: 0,
        device: 0,
        revision: 0,
        clase: 0,
        subclase: 0,
        prog_if: 0,
    };

    /// Es un dispositivo de video (clase `0x03`: display/VGA).
    pub fn es_video(&self) -> bool {
        self.clase == CLASE_VIDEO
    }

    /// ¿Es un controlador de host USB? (sin mirar el `prog_if`).
    pub fn es_controlador_usb(&self) -> bool {
        es_controlador_usb(self.clase, self.subclase)
    }

    /// ¿Es un controlador de host USB de un tipo conocido?
    pub fn tipo_hcd(&self) -> Option<TipoHcd> {
        es_hcd_usb(self.clase, self.subclase, self.prog_if)
    }
}

/// Un BAR ya descodificado **y medido** (con su tamano).
///
/// El tamano solo se puede obtener escribiendo todo a unos en el registro, asi
/// que solo lo rellena quien pueda hacerlo con el dispositivo en reposo (ver
/// `pci::medir_bares` del kernel).
#[derive(Clone, Copy, Debug)]
pub struct Bar {
    /// Direccion base de la ventana.
    pub base: u64,
    /// Tamano de la ventana en bytes.
    pub size: u64,
    /// Es un BAR de E/S (puerto) y no de memoria.
    pub es_io: bool,
    /// Es un BAR de 64 bits (ocupa dos registros de 32 bits).
    pub es_64: bool,
}

/// Un BAR leido del espacio de configuracion, **sin medir**.
///
/// Es lo que se puede averiguar sin escribir nada en el dispositivo: donde
/// esta la ventana y de que tipo es. El tamano se deja a cero porque medirlo
/// exige escribir en el registro.
#[derive(Clone, Copy, Debug)]
pub struct BarLeido {
    /// Direccion base asignada por el firmware.
    pub base: u64,
    /// Es un BAR de E/S (puerto) y no de memoria.
    pub es_io: bool,
    /// Es un BAR de 64 bits (ocupa dos registros de 32 bits).
    pub es_64: bool,
}

/// Los cuatro tipos de controlador de host USB que define el codigo de clase.
///
/// El `prog_if` de una funcion USB dice que clase de controlador es:
///
/// | `prog_if` | controlador | version de USB |
/// | --- | --- | --- |
/// | `0x00` | UHCI | 1.1 |
/// | `0x10` | OHCI | 1.0 |
/// | `0x20` | EHCI | 2.0 |
/// | `0x30` | xHCI | 3.0 y ss |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipoHcd {
    /// UHCI: USB 1.1.
    Uhci,
    /// OHCI: USB 1.0.
    Ohci,
    /// EHCI: USB 2.0.
    Ehci,
    /// xHCI: USB 3.0 y superspeed.
    Xhci,
}

impl TipoHcd {
    /// Traduce el `prog_if` de una funcion USB a su tipo de controlador.
    ///
    /// Un `prog_if` que no esta en la tabla no es un USB que no exista: es un
    /// controlador que el codigo de clase no describe (por ejemplo, una
    /// funcion de clase USB con `prog_if` 0x40, que aparece con ademas de la
    /// emulacion de legacy). Se devuelve `None` para no inventar un tipo.
    pub fn de_prog_if(prog_if: u8) -> Option<TipoHcd> {
        match prog_if {
            0x00 => Some(TipoHcd::Uhci),
            0x10 => Some(TipoHcd::Ohci),
            0x20 => Some(TipoHcd::Ehci),
            0x30 => Some(TipoHcd::Xhci),
            _ => None,
        }
    }

    /// Nombre corto del controlador (para la consola).
    pub fn nombre(self) -> &'static str {
        match self {
            TipoHcd::Uhci => "UHCI",
            TipoHcd::Ohci => "OHCI",
            TipoHcd::Ehci => "EHCI",
            TipoHcd::Xhci => "xHCI",
        }
    }
}

/// ¿La clase y la subclase son las de un controlador de host USB?
///
/// No mira el `prog_if`: asi se detecta tambien un USB de tipo desconocido.
pub fn es_controlador_usb(clase: u8, subclase: u8) -> bool {
    clase == CLASE_BUS_SERIE && subclase == SUBCLASE_USB
}

/// ¿La funcion es un controlador de host USB de un tipo conocido?
pub fn es_hcd_usb(clase: u8, subclase: u8, prog_if: u8) -> Option<TipoHcd> {
    if es_controlador_usb(clase, subclase) {
        TipoHcd::de_prog_if(prog_if)
    } else {
        None
    }
}

/// Nombre del tipo de HCD a partir del `prog_if` (o `None` si no se conoce).
pub fn nombre_hcd(prog_if: u8) -> Option<&'static str> {
    TipoHcd::de_prog_if(prog_if).map(|t| t.nombre())
}

/// Decodifica el par de registros de un BAR **sin medirlo**.
///
/// `lo` es el registro de 32 bits del BAR. `hi` es el hermano alto, y solo
/// tiene sentido para un BAR de 64 bits (el bit 2 del registro bajo vale `1`).
///
/// Devuelve `None` cuando el BAR no esta implementado (registro a cero) o
/// cuando la direccion base es cero, que en la practica significa "sin
/// ventana asignada".
pub fn decodificar_bar(lo: u32, hi: Option<u32>) -> Option<BarLeido> {
    if lo == 0 {
        return None;
    }
    // Bit 0: 1 = espacio de E/S, 0 = espacio de memoria.
    if lo & 1 != 0 {
        let base = (lo & !0x3) as u64;
        if base == 0 {
            return None;
        }
        return Some(BarLeido {
            base,
            es_io: true,
            es_64: false,
        });
    }
    // Bits 2:1: 00 = 32 bits, 10 = 64 bits, 01 = reservado.
    let tipo = (lo >> 1) & 0x3;
    let es_64 = tipo == 0x2;
    if tipo == 0x1 {
        // Tipo reservado: la direccion no significa nada. Se descarta en vez
        // de adivinar.
        return None;
    }
    let mut base = (lo & !0xF) as u64;
    if es_64 {
        base |= (hi.unwrap_or(0) as u64) << 32;
    }
    if base == 0 {
        return None;
    }
    Some(BarLeido {
        base,
        es_io: false,
        es_64,
    })
}

/// Por que un BAR todavia no se puede usar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemaBar {
    /// El BAR no tiene direccion asignada.
    SinAsignar,
    /// El tamano medido no es una potencia de dos: no es un BAR valido.
    TamanoInvalido,
    /// La ventana se sale del limite fisico admitido.
    FueraDelLimite,
    /// La base mas el tamano desborda la direccion de 64 bits.
    Desbordamiento,
}

/// Comprueba que la ventana de un BAR de memoria es utilizable.
///
/// `limite` es el tope fisico **exclusivo**: con `1 << 32` se exige que toda la
/// ventana quepa por debajo de 4 GiB, que es lo que puede desreferenciar el
/// mapa por identidad que prepara el arranque sin ampliarlo.
///
/// Un BAR de E/S nunca pasa esta comprobacion: sus registros no son memoria.
pub fn validar_bar_memoria(bar: &Bar, limite: u64) -> Result<(), ProblemaBar> {
    if bar.es_io {
        return Err(ProblemaBar::SinAsignar);
    }
    if bar.base == 0 {
        return Err(ProblemaBar::SinAsignar);
    }
    if bar.size == 0 || !bar.size.is_power_of_two() {
        return Err(ProblemaBar::TamanoInvalido);
    }
    let fin = bar
        .base
        .checked_add(bar.size)
        .ok_or(ProblemaBar::Desbordamiento)?;
    if fin > limite {
        return Err(ProblemaBar::FueraDelLimite);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clasifica_los_cuatro_tipos_de_controlador_usb() {
        assert_eq!(TipoHcd::de_prog_if(0x00), Some(TipoHcd::Uhci));
        assert_eq!(TipoHcd::de_prog_if(0x10), Some(TipoHcd::Ohci));
        assert_eq!(TipoHcd::de_prog_if(0x20), Some(TipoHcd::Ehci));
        assert_eq!(TipoHcd::de_prog_if(0x30), Some(TipoHcd::Xhci));
        // Un prog_if que no esta en la tabla no se inventa.
        assert_eq!(TipoHcd::de_prog_if(0x40), None);
        assert_eq!(TipoHcd::de_prog_if(0x99), None);
    }

    #[test]
    fn el_tipo_de_hcd_se_saca_de_la_clase_no_del_fabricante() {
        // Una funcion USB de cualquier fabricante con prog_if 0x30 es xHCI.
        assert_eq!(es_hcd_usb(0x0C, 0x03, 0x30), Some(TipoHcd::Xhci));
        assert_eq!(
            es_hcd_usb(0x0C, 0x03, 0x30).map(|t| t.nombre()),
            Some("xHCI")
        );
        // Mismo prog_if pero otra clase: no es USB.
        assert_eq!(es_hcd_usb(0x0C, 0x05, 0x30), None);
        assert_eq!(es_hcd_usb(0x03, 0x03, 0x30), None);
    }

    #[test]
    fn un_usb_de_tipo_desconocido_sigue_siendo_usb() {
        assert!(es_controlador_usb(0x0C, 0x03));
        assert!(es_hcd_usb(0x0C, 0x03, 0x40).is_none());
        assert_eq!(nombre_hcd(0x40), None);
        assert_eq!(nombre_hcd(0x20), Some("EHCI"));
    }

    #[test]
    fn decodifica_un_bar_de_memoria_de_32_bits() {
        // base 0xE0000000, tipo 00b (32 bits).
        let b = decodificar_bar(0xE000_0000, None).expect("BAR valido");
        assert_eq!(b.base, 0xE000_0000);
        assert!(!b.es_io);
        assert!(!b.es_64);
    }

    #[test]
    fn decodifica_un_bar_de_64_bits() {
        // Tipo 10b: el registro bajo vale 0x4 y la base alta va en el hermano.
        // Los 4 bits bajos son de tipo, asi que la base sale en 0x2_0000_0000.
        let b = decodificar_bar(0x4, Some(0x0000_0002)).expect("BAR valido");
        assert_eq!(b.base, 0x2_0000_0000);
        assert!(b.es_64);
        assert!(!b.es_io);
    }

    #[test]
    fn decodifica_un_bar_de_64_bits_por_debajo_de_4_gib() {
        // base 0x0000_E000_0000 repartida en los dos registros.
        let b = decodificar_bar(0xE000_0004, Some(0x0000_0000)).expect("BAR valido");
        assert_eq!(b.base, 0xE000_0000);
        assert!(b.es_64);
    }

    #[test]
    fn decodifica_un_bar_de_espacio_de_es() {
        let b = decodificar_bar(0x0000_00C1, None).expect("BAR valido");
        assert_eq!(b.base, 0xC0);
        assert!(b.es_io);
        assert!(!b.es_64);
    }

    #[test]
    fn un_bar_vacio_o_sin_base_no_devuelve_nada() {
        assert!(decodificar_bar(0, None).is_none());
        assert!(decodificar_bar(0x4, Some(0)).is_none());
        // Tipo 01b (reservado) con base no nula: no se interpreta.
        assert!(decodificar_bar(0xE000_0002, None).is_none());
    }

    #[test]
    fn un_bar_de_64_bits_sin_hermano_alto_no_inventa_la_parte_alta() {
        let b = decodificar_bar(0xE000_0004, None).expect("BAR valido");
        assert_eq!(b.base, 0xE000_0000);
    }

    #[test]
    fn valida_una_ventana_de_memoria_valida() {
        let bar = Bar {
            base: 0xE000_0000,
            size: 0x1_0000,
            es_io: false,
            es_64: true,
        };
        assert_eq!(validar_bar_memoria(&bar, 1 << 32), Ok(()));
    }

    #[test]
    fn rechaza_una_ventana_sin_asignar() {
        let bar = Bar {
            base: 0,
            size: 0x1000,
            es_io: false,
            es_64: false,
        };
        assert_eq!(
            validar_bar_memoria(&bar, 1 << 32),
            Err(ProblemaBar::SinAsignar)
        );
    }

    #[test]
    fn rechaza_una_ventana_por_encima_de_4_gib() {
        let bar = Bar {
            base: 0x1_0000_0000,
            size: 0x1000,
            es_io: false,
            es_64: true,
        };
        assert_eq!(
            validar_bar_memoria(&bar, 1 << 32),
            Err(ProblemaBar::FueraDelLimite)
        );
    }

    #[test]
    fn rechaza_un_tamano_que_no_es_potencia_de_dos() {
        let bar = Bar {
            base: 0xE000_0000,
            size: 0x3000,
            es_io: false,
            es_64: false,
        };
        assert_eq!(
            validar_bar_memoria(&bar, 1 << 32),
            Err(ProblemaBar::TamanoInvalido)
        );
    }

    #[test]
    fn rechaza_una_ventana_que_desborda() {
        let bar = Bar {
            base: u64::MAX - 0x1000,
            size: 0x2000,
            es_io: false,
            es_64: false,
        };
        assert_eq!(
            validar_bar_memoria(&bar, 1 << 32),
            Err(ProblemaBar::Desbordamiento)
        );
    }

    #[test]
    fn un_bar_de_espacio_de_es_no_es_una_ventana_de_memoria() {
        let bar = Bar {
            base: 0xC0,
            size: 0x20,
            es_io: true,
            es_64: false,
        };
        assert_eq!(
            validar_bar_memoria(&bar, 1 << 32),
            Err(ProblemaBar::SinAsignar)
        );
    }
}
