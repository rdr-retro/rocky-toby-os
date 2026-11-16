//! Los controladores de host USB (HCD) y el estado que publican.
//!
//! ## Que hay aqui (y que no)
//!
//! Aqui esta el **contrato** que cumple todo controlador de host: como se
//! llama, en que estado esta, que contadores lleva y que se ve en sus puertos
//! raiz. El driver de verdad (el xHCI) esta en [`xhci`].
//!
//! ## Estado publicado
//!
//! La inicializacion del controlador corre en una **tarea de fondo**, asi que la
//! consola no puede preguntarle directamente (puede estar en mitad de un
//! sondeo). Lo que hace la tarea es **publicar** una instantanea del estado en
//! un `static`, y lo que hace la consola es **leer** esa instantanea. La
//! instantanea se copia con las interrupciones apagadas, para que no se lea a
//! medias.
//!
//! ## Que NO se hace aqui
//!
//! - No se enumeran dispositivos: eso es trabajo del driver, no del contrato.
//! - No se toca la emulacion USB heredada: PS/2 y puerto serie siguen siendo la
//!   entrada del sistema hasta que haya un HID funcionando.
//!
//! Codigo propio de rocky.

pub mod xhci;

use crate::sync::SyncCell;
use rockyhw::xhci::Velocidad;

/// Maximo de puertos raiz que se publican.
///
/// `HCSPARAMS1` admite hasta 255 puertos, pero ningun controlador real llega
/// a eso. Si un controlador declarara mas, los puertos que no caben aqui no
/// se publican: se cuentan aparte.
pub const MAX_PUERTOS: usize = 32;

/// En que estado esta un controlador de host.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EstadoHcd {
    /// Todavia no se ha intentado inicializarlo.
    SinIniciar,
    /// La inicializacion fallo: el controlador no se usa y se sigue con PS/2.
    Fallido,
    /// El controlador esta inicializado y en marcha.
    EnMarcha,
}

impl EstadoHcd {
    /// Nombre corto del estado, para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            EstadoHcd::SinIniciar => "sin iniciar",
            EstadoHcd::Fallido => "fallo al inicializar",
            EstadoHcd::EnMarcha => "inicializado y en marcha",
        }
    }
}

/// El estado de un puerto raiz, ya traducuido.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InfoPuerto {
    /// Numero de puerto (desde 1).
    pub numero: u8,
    /// El protocolo que habla el puerto: "USB 2.0" o "USB 3.x".
    pub protocolo: &'static str,
    /// Hay un dispositivo conectado.
    pub conectado: bool,
    /// El puerto esta habilitado.
    pub habilitado: bool,
    /// El puerto tiene alimentacion.
    pub alimentado: bool,
    /// La velocidad del enlace.
    pub velocidad: Velocidad,
}

impl InfoPuerto {
    /// Un puerto que no se ha leido.
    pub const VACIO: InfoPuerto = InfoPuerto {
        numero: 0,
        protocolo: "",
        conectado: false,
        habilitado: false,
        alimentado: false,
        velocidad: Velocidad::Indefinida,
    };
}

/// Maximo de dispositivos que se publican.
///
/// El driver puede enumerar hasta `MaxSlots` (255 en el peor caso), pero la
/// consola no tiene sentido con esa lista. Los que no caben se cuentan en los
/// contadores, no se pierden.
pub const MAX_DISPOSITIVOS_PUBLICADOS: usize = 8;

/// Un dispositivo enumerado, ya traducido para la consola.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InfoDispositivo {
    /// El puerto raiz donde esta conectado.
    pub puerto: u8,
    /// La direccion USB que le ha dado el controlador.
    pub direccion: u8,
    /// El hueco que ocupa en el controlador.
    pub hueco: u8,
    /// La velocidad del enlace.
    pub velocidad: Velocidad,
    /// El identificador de fabricante.
    pub id_vendor: u16,
    /// El identificador de producto.
    pub id_product: u16,
    /// La clase que declara el dispositivo (0 si no declara ninguna).
    pub clase: u8,
    /// Cuantas interfaces tiene.
    pub interfaces: u8,
    /// Cuantas de esas interfaces son HID.
    pub interfaces_hid: u8,
    /// Cuantos endpoints declara la configuracion (sin contar el 0).
    pub endpoints: u8,
    /// Cuanto consume como maximo, en miliamperios (`bMaxPower` * 2).
    pub consumo_ma: u16,
    /// La configuracion que se le ha pedido.
    pub configuracion: u8,
    /// Cuantos endpoints HID boot se han programado para este dispositivo.
    pub hid: u8,
    /// Informes HID recibidos de este dispositivo.
    pub informes: u32,
}

impl InfoDispositivo {
    /// Un dispositivo que no existe.
    pub const VACIO: InfoDispositivo = InfoDispositivo {
        puerto: 0,
        direccion: 0,
        hueco: 0,
        velocidad: Velocidad::Indefinida,
        id_vendor: 0,
        id_product: 0,
        clase: 0,
        interfaces: 0,
        interfaces_hid: 0,
        endpoints: 0,
        consumo_ma: 0,
        configuracion: 0,
        hid: 0,
        informes: 0,
    };
}

/// Los contadores de actividad del controlador.
///
/// No crecen sin limite: son contadores de 32 bits que cuentan hechos, no
/// bytes, asi que dan la vuelta solo tras miles de millones de eventos.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Contadores {
    /// Rondas de sondeo del anillo de eventos.
    pub sondeos: u32,
    /// Eventos leidos del anillo de eventos.
    pub eventos: u32,
    /// Eventos de cambio de puerto.
    pub cambios_puerto: u32,
    /// Eventos de un tipo que no se conoce.
    pub desconocidos: u32,
    /// Errores del controlador (`HCE`, `HSE`).
    pub errores: u32,
    /// Dispositivos enumerados correctamente.
    pub dispositivos: u32,
    /// Puertos que no se han podido enumerar.
    pub errores_enum: u32,
    /// Informes HID entregados a las colas de entrada.
    pub informes_hid: u32,
    /// Informes o transferencias HID que no se pudieron atender.
    pub errores_hid: u32,
}

/// La instantanea del estado de un controlador, para la consola.
#[derive(Clone, Copy)]
pub struct Publicado {
    /// Nombre del controlador.
    pub nombre: &'static str,
    /// En que estado esta.
    pub estado: EstadoHcd,
    /// El motivo del fallo, si lo hay.
    pub detalle: &'static str,
    /// Contadores de actividad.
    pub contadores: Contadores,
    /// Cuantos puertos raiz hay con informacion.
    pub num_puertos: u8,
    /// El estado de cada puerto raiz.
    pub puertos: [InfoPuerto; MAX_PUERTOS],
    /// Cuantos dispositivos hay enumerados.
    pub num_dispositivos: u8,
    /// Los dispositivos enumerados.
    pub dispositivos: [InfoDispositivo; MAX_DISPOSITIVOS_PUBLICADOS],
}

impl Publicado {
    /// La instantanea vacia: no hay ningun controlador.
    pub const VACIO: Publicado = Publicado {
        nombre: "",
        estado: EstadoHcd::SinIniciar,
        detalle: "",
        contadores: Contadores {
            sondeos: 0,
            eventos: 0,
            cambios_puerto: 0,
            desconocidos: 0,
            errores: 0,
            dispositivos: 0,
            errores_enum: 0,
            informes_hid: 0,
            errores_hid: 0,
        },
        num_puertos: 0,
        puertos: [InfoPuerto::VACIO; MAX_PUERTOS],
        num_dispositivos: 0,
        dispositivos: [InfoDispositivo::VACIO; MAX_DISPOSITIVOS_PUBLICADOS],
    };
}

/// La instantanea publicada.
static PUBLICADO: SyncCell<Publicado> = SyncCell::new(Publicado::VACIO);

/// Publica la instantanea del estado del controlador.
///
/// La copia se hace con las interrupciones apagadas: si no, la consola podria
/// leerla a medias y ver un numero de puertos que no cuadra con la lista.
pub fn publicar(p: Publicado) {
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            *PUBLICADO.get() = p;
        })
    }
}

/// Lee la instantanea publicada.
pub fn publicado() -> Publicado {
    unsafe { crate::cpu::sin_interrupciones(|| *PUBLICADO.get()) }
}
