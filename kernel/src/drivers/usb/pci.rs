//! Seleccion de los controladores de host USB en el bus PCI.
//!
//! El despacho es **por clase de dispositivo**, nunca por identificador de
//! fabricante: una funcion de clase `0x0C`, subclase `0x03` (bus serie ->
//! controlador de host USB) es un HCD, diga lo que diga su `vendor:device`.
//! El `prog_if` dice de que clase de controlador se trata:
//!
//! | `prog_if` | controlador |
//! | --- | --- |
//! | `0x00` | UHCI (USB 1.1) |
//! | `0x10` | OHCI (USB 1.0) |
//! | `0x20` | EHCI (USB 2.0) |
//! | `0x30` | xHCI (USB 3.0) |
//!
//! Lo que se puede afirmar de un controlador **sin escribir nada** en el es
//! bastante: que existe, que clase tiene, donde dice el firmware que estan sus
//! registros y si esa direccion la cubre el mapa de memoria del arranque.
//!
//! Codigo propio de rocky.

use crate::pci::{Bar, BarLeido, Funcion, TipoHcd};

/// Tope fisico que cubre el mapa por identidad del arranque (4 GiB, exclusivo).
///
/// El bootloader mapea por identidad los primeros 4 GiB con paginas de 2 MiB,
/// asi que un registro de dispositivo por encima de ese limite no se puede
/// leer sin ampliar la paginacion.
pub const TOPE_MAPA: u64 = 1 << 32;

/// Un controlador de host USB encontrado en el bus.
///
/// Es el resultado del **inventario** (fase 0): todo lo que se averigua sin
/// escribir en el dispositivo. El tamano de las ventanas no se incluye,
/// porque averiguarlo exige escribir en los registros.
#[derive(Clone, Copy)]
pub struct Controlador {
    /// La funcion PCI donde vive el controlador.
    pub funcion: Funcion,
    /// Tipo de controlador segun el `prog_if` (si el codigo de clase lo dice).
    pub tipo: Option<TipoHcd>,
    /// Los seis BAR, tal y como los asigno el firmware.
    pub bares: [Option<BarLeido>; 6],
    /// El BAR donde estan los registros del controlador.
    pub registros: Option<BarLeido>,
    /// El BAR de registros **medido** (con su tamano). Solo se rellena si se
    /// ha ejecutado la sonda de `usb medir`.
    pub medido: Option<Bar>,
    /// ¿La base de los registros esta por debajo del tope del mapa?
    pub direccionable: bool,
}

impl Controlador {
    /// Controlador vacio (para rellenar los huecos de la tabla).
    pub const VACIO: Controlador = Controlador {
        funcion: Funcion::VACIA,
        tipo: None,
        bares: [None; 6],
        registros: None,
        medido: None,
        direccionable: false,
    };

    /// Nombre del tipo de controlador (o "tipo desconocido").
    pub fn nombre_tipo(&self) -> &'static str {
        match self.tipo {
            Some(t) => t.nombre(),
            None => "tipo desconocido",
        }
    }
}

/// ¿La funcion es un controlador de host USB?
///
/// Mira la clase (`0x0C`) y la subclase (`0x03`), no el fabricante ni el
/// `prog_if`: un USB de tipo desconocido sigue siendo un USB.
pub fn es_hcd(f: Funcion) -> bool {
    f.es_controlador_usb()
}

/// Inventaria una funcion USB, **sin escribir nada** en ella.
///
/// Devuelve `None` si la funcion no es un controlador de host USB.
pub fn inventariar(f: Funcion) -> Option<Controlador> {
    if !es_hcd(f) {
        return None;
    }

    let bares = crate::pci::bares_solo_lectura(f);

    // El BAR de registros es el primer BAR de memoria con base asignada. Los
    // BAR de espacio de E/S no sirven: sus registros no son memoria.
    let registros = bares
        .iter()
        .flatten()
        .find(|b| !b.es_io && b.base != 0)
        .copied();

    // La base por debajo del tope del mapa es la condicion minima para poder
    // leer los registros. El tamano se comprueba mas adelante, cuando se mida
    // el BAR con el controlador parado.
    let direccionable = match registros {
        Some(b) => b.base < TOPE_MAPA,
        None => false,
    };

    Some(Controlador {
        funcion: f,
        tipo: f.tipo_hcd(),
        bares,
        registros,
        medido: None,
        direccionable,
    })
}
