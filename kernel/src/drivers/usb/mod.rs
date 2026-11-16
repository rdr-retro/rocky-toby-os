//! Pila USB de rocky: controlador xHCI, enumeracion y HID.
//!
//! ## Que hay aqui (y que no)
//!
//! Estan hechas las **fases 0 a 6** del plan USB (`usb-hid-universal-plan.md`):
//!
//! - **Fase 0/1**: inventario observable de los controladores de host del bus,
//!   con primitivas seguras para trabajar con ellos ([`pci`], [`mmio`],
//!   [`dma`]: BAR, MMIO acotado y reserva DMA por debajo de 4 GiB).
//! - **Fase 2**: el HCD **xHCI** se inicializa en una tarea de fondo (handoff
//!   BIOS/OS con timeout, halt/reset/start, DCBAA, anillos, ERST y puertos
//!   raiz), sin MSI/MSI-X y sin frenar el arranque.
//! - **Fase 3**: enumeracion estandar (Enable Slot, Address Device,
//!   descriptores, `SET_CONFIGURATION`) con topes fijos y validacion de cada
//!   descriptor. Los dispositivos van en puertos raiz; los **hubs** USB 2.0 se
//!   configuran.
//! - **Fase 4**: HID de **protocolo boot** (teclado y raton) con endpoint de
//!   interrupcion, `SET_PROTOCOL(Boot)`/`SET_IDLE(0)`, desconexion segura y
//!   liberacion de modificadores.
//! - **Fase 5**: los eventos desembocan en [`crate::input`] (como juego 1) y en
//!   [`crate::mouse`], la misma capa que usa PS/2, y los contadores salen en
//!   `usb`.
//! - **Fase 6**: un HID que no es boot se lee por su **descriptor de informe**
//!   ([`rockyhw::hid::informe`]): items cortos y largos, `Push`/`Pop`,
//!   `Report ID`, campos con signo y matrices de teclas. Se aceptan teclados y
//!   ratones **relativos**; una tableta absoluta se rechaza (sus X/Y son una
//!   posicion, no un desplazamiento). Si un dispositivo que declara boot
//!   rechaza `SET_PROTOCOL`, se prueba esta via.
//!
//! Lo que **no** hay todavia: EHCI/UHCI/OHCI ni validacion en hardware fisico
//! (fase 7/8). Por eso este modulo solo **reconoce** [`TipoHcd::Xhci`], y un
//! HID del que no se pueda sacar un teclado o un raton se descarta sin afectar
//! al resto. Los hubs USB 2.0 se enrutan con *route string* y TT.
//!
//! ## Reglas que respeta este modulo
//!
//! - El inventario de arranque es **de solo lectura**: no escribe en el
//!   espacio de configuracion del controlador ni lo habilita. El bus mastering
//!   no se enciende hasta que el HCD esta preparado.
//! - La emulacion USB heredada **no** se toca: PS/2 y puerto serie siguen
//!   activos, y si no hay xHCI o falla la inicializacion no se pierde entrada.
//! - La inicializacion es una **tarea de fondo**: el arranque solo inventaria
//!   (seis registros por funcion). El sondeo de puertos y los informes HID
//!   corren con limites por paso.
//! - El inventario vive en un array fijo de un `static` porque se ejecuta antes
//!   de que exista el heap.
//!
//! ## Subcomandos
//!
//! - `usb`: inventario de solo lectura, estado del HCD, puertos raiz,
//!   dispositivos enumerados y contadores.
//! - `usb medir`: mide el tamano de las ventanas del BAR. **Escribe** en los
//!   registros del BAR (es la unica forma de saber el tamano), asi que apaga
//!   la decodificacion y el bus mastering mientras dura la sonda. Con el HCD
//!   en marcha se rechaza.
//! - `usb preparar`: valida el BAR, abre la ventana de registros, enciende la
//!   decodificacion de **memoria** (no el bus mastering) y comprueba que se
//!   puede reservar memoria DMA por debajo de 4 GiB. Deja el controlador
//!   marcado como en uso; con el HCD en marcha tambien se rechaza.
//!
//! Codigo propio de rocky.
//!
//! Codigo propio de rocky.

pub mod core;
pub mod dma;
pub mod hcd;
pub mod mmio;
pub mod pci;

use crate::sync::SyncCell;
use ::core::slice;
use pci::Controlador;
use rockyhw::pci::{Bar, TipoHcd};

/// Maximo de controladores de host USB que se guardan en el inventario.
///
/// Un equipo tiene uno o dos. El array es fijo porque el inventario se arma
/// antes de que exista el heap.
const MAX: usize = 8;

/// En que estado esta el subsistema USB.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// No hay ningun controlador de host USB en el bus.
    SinControlador,
    /// Hay controladores de host USB, pero ninguno de un tipo que rocky sepa
    /// manejar (todavia no se ha escrito ningun driver de UHCI/OHCI/EHCI).
    SinSoporte,
    /// Hay un controlador de un tipo soportado, inventariado, pero **sin
    /// inicializar**: la inicializacion del HCD es la fase 2 del plan.
    Detectado,
    /// Se intento inicializar y fallo. Se sigue con PS/2.
    Fallido,
    /// El controlador esta inicializado y en marcha.
    Inicializado,
}

impl Estado {
    /// Nombre corto del estado, para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            Estado::SinControlador => "sin controlador",
            Estado::SinSoporte => "HCD detectado pero aun no soportado",
            Estado::Detectado => "HCD detectado (sin inicializar)",
            Estado::Fallido => "fallo al inicializar el HCD",
            Estado::Inicializado => "HCD inicializado",
        }
    }
}

/// El inventario: los controladores de host USB que se han encontrado.
static INVENTARIO: SyncCell<([Controlador; MAX], usize)> =
    SyncCell::new(([Controlador::VACIO; MAX], 0));

/// El estado global del subsistema.
static ESTADO: SyncCell<Estado> = SyncCell::new(Estado::SinControlador);

fn inventario() -> &'static mut ([Controlador; MAX], usize) {
    unsafe { &mut *INVENTARIO.get() }
}

fn estado_global() -> &'static mut Estado {
    unsafe { &mut *ESTADO.get() }
}

/// El estado actual del subsistema USB.
pub fn estado() -> Estado {
    *estado_global()
}

/// Los controladores de host USB que se han inventariado.
pub fn controladores() -> &'static [Controlador] {
    let (lista, cuenta) = inventario();
    let n = *cuenta;
    let p: *const Controlador = lista.as_ptr();
    // Seguro: la tabla es estatica y solo se escribe en `init` y en las
    // subcomandos del diagnostico, que corren en la tarea del nucleo.
    unsafe { slice::from_raw_parts(p, n) }
}

/// Inventaria los controladores de host USB del bus.
///
/// Es **de solo lectura**: no escribe en el espacio de configuracion ni
/// habilita nada en el dispositivo. Solo lee el identificador y los BAR que
/// el firmware asigno.
///
/// Se llama desde [`crate::drivers::init`], antes de que exista el heap: por
/// eso el inventario es un array fijo.
pub fn init() {
    {
        let (lista, cuenta) = inventario();
        *cuenta = 0;
        for f in crate::pci::funciones() {
            let c = match pci::inventariar(*f) {
                Some(c) => c,
                None => continue,
            };
            if *cuenta < MAX {
                lista[*cuenta] = c;
                *cuenta += 1;
            }
        }
    }

    let estado = recalcular_estado();
    *estado_global() = estado;

    // Una sola linea: el arranque no debe perder el tiempo con USB, y si
    // algo va mal se vera en la consola.
    match estado {
        Estado::SinControlador => {
            crate::println!("rocky: USB: sin controladores de host en el bus.");
        }
        Estado::SinSoporte => {
            crate::println!(
                "rocky: USB: {} controlador(es) de host, ninguno de un tipo soportado todavia.",
                controladores().len()
            );
        }
        Estado::Detectado => {
            crate::println!(
                "rocky: USB: {} controlador(es) de host (xHCI), inventariados y sin inicializar.",
                controladores().len()
            );
        }
        Estado::Fallido => {
            crate::println!("rocky: USB: la inicializacion del controlador fallo.");
        }
        Estado::Inicializado => {
            crate::println!("rocky: USB: controlador inicializado.");
        }
    }
}

/// Recalcula el estado global a partir del inventario.
fn recalcular_estado() -> Estado {
    let lista = controladores();
    let mut hay_usb = false;
    let mut hay_xhci = false;
    for c in lista {
        hay_usb = true;
        if c.tipo == Some(TipoHcd::Xhci) {
            hay_xhci = true;
        }
    }
    if !hay_usb {
        Estado::SinControlador
    } else if !hay_xhci {
        Estado::SinSoporte
    } else {
        // El estado final lo dice la tarea de fondo del HCD, que publica su
        // resultado en [`hcd::publicado`]. Antes de que esa tarea corra, el
        // controlador esta inventariado pero sin inicializar.
        match hcd::publicado().estado {
            hcd::EstadoHcd::EnMarcha => Estado::Inicializado,
            hcd::EstadoHcd::Fallido => Estado::Fallido,
            hcd::EstadoHcd::SinIniciar => Estado::Detectado,
        }
    }
}

// --- Medida y preparacion de los controladores ---

/// Mide las ventanas de un controlador y guarda el resultado en el inventario.
///
/// Devuelve el BAR de registros medido, o `None` si no se pudo medir (por
/// ejemplo, porque el controlador ya esta en uso).
///
/// La sonda **escribe** en los registros del BAR: es la unica forma de saber
/// el tamano de la ventana. Por eso `medir_bares` apaga la decodificacion y
/// el bus mastering mientras dura, y por eso no se mide un controlador que ya
/// esta en uso.
fn medir(c: &Controlador) -> Option<Bar> {
    let f = c.funcion;
    if crate::pci::esta_en_uso(f) {
        return None;
    }
    // Durante la sonda el controlador cuenta como en uso: asi nadie mas toca
    // sus registros mientras se esta escribiendo en el BAR.
    crate::pci::marcar_en_uso(f);
    let r = sonda(c);
    crate::pci::desmarcar_en_uso(f);
    r
}

/// Mide el BAR de registros de un controlador y lo deja **marcado como en
/// uso**.
///
/// Es la variante que usa el HCD: desde el momento en que el controlador es
/// de rocky, nadie mas mide sus BAR, lo reinicia ni le apaga el bus
/// mastering. Por eso la sonda no puede mirar la marca de uso.
pub(crate) fn medir_para_hcd(c: &Controlador) -> Option<Bar> {
    crate::pci::marcar_en_uso(c.funcion);
    sonda(c)
}

/// La sonda que mide el BAR de registros de un controlador.
///
/// Escribe en los BAR del dispositivo, asi que quien la llama tiene que
/// tener la funcion marcada como en uso.
fn sonda(c: &Controlador) -> Option<Bar> {
    let f = c.funcion;
    let bares = crate::pci::medir_bares_siempre(f);
    // El BAR de registros es el primero de memoria con base.
    let mut registro = None;
    for b in bares.iter().flatten() {
        if !b.es_io && b.base != 0 {
            registro = Some(*b);
            break;
        }
    }
    // Se guarda la medida en el inventario (la funcion es Copy).
    if let Some(b) = registro {
        let (lista, cuenta) = inventario();
        for i in 0..*cuenta {
            if lista[i].funcion.bus == f.bus
                && lista[i].funcion.dev == f.dev
                && lista[i].funcion.func == f.func
            {
                lista[i].medido = Some(b);
                break;
            }
        }
    }
    registro
}

/// Prepara un controlador para que la fase 2 pueda inicializarlo.
///
/// Hace, en este orden:
///
/// 1. Mide el BAR (si no esta medido) y valida que la ventana de registros
///    quepa en el mapa por identidad del arranque.
/// 2. Enciende la decodificacion de **memoria** y comprueba que el bus
///    mastering sigue **apagado**: no se enciende hasta que haya estructuras
///    DMA reservadas y el HCD inicializado (regla de la fase 1).
/// 3. Reserva una region DMA de prueba por debajo de 4 GiB y la devuelve al
///    gestor: comprueba de verdad que la reserva DMA funciona en esta
///    maquina. No son estructuras del controlador, solo una prueba.
/// 4. Marca el controlador como en uso, para que nadie mida sus BAR otra vez.
///
/// Devuelve un mensaje con el resultado, o el motivo del fallo.
fn preparar(c: &Controlador) -> Result<(), &'static str> {
    let f = c.funcion;
    if crate::pci::esta_en_uso(f) {
        return Err("el controlador ya esta en uso");
    }

    // 1. BAR medido y ventana de registros.
    let bar = match c.medido {
        Some(b) => b,
        None => match medir(c) {
            Some(b) => b,
            None => return Err("no se pudo medir el BAR"),
        },
    };
    let ventana = match mmio::ventana(&bar) {
        Ok(v) => v,
        Err(mmio::ErrorMmio::NoEsMemoria) => {
            return Err("el BAR de registros es de espacio de E/S, no de memoria")
        }
        Err(mmio::ErrorMmio::FueraDelMapa) => {
            return Err("la ventana de registros esta por encima de 4 GiB")
        }
        Err(_) => return Err("la ventana de registros no es valida"),
    };

    // 2. Decodificacion de memoria encendida, bus mastering apagado.
    crate::pci::habilitar_memoria(f);
    crate::pci::deshabilitar_bus_master(f);
    // Que la activacion se vea antes de tocar ningun registro del dispositivo.
    mmio::mfence();
    if !crate::pci::memoria_activa(f) {
        return Err("el firmware no deja encender la decodificacion de memoria");
    }
    if crate::pci::bus_master_activo(f) {
        return Err("el bus mastering sigue encendido y no se puede apagar");
    }

    // 3. Prueba de reserva DMA.
    let region = dma::reservar(1, 4096).map_err(|e| match e {
        rockyhw::dma::ErrorDma::SinMarcos => "no hay memoria contigua por debajo de 4 GiB",
        rockyhw::dma::ErrorDma::LimiteFisico => "la region se pasaria de 4 GiB",
        _ => "no se pudo reservar la region DMA",
    })?;

    // 4. El controlador pasa a ser de rocky.
    crate::pci::marcar_en_uso(f);

    crate::println!(
        "usb: controlador {:02X}:{:02X}.{}: ventana de registros 0x{:X}-0x{:X} ({} bytes)",
        f.bus,
        f.dev,
        f.func,
        ventana.base(),
        ventana.fin(),
        ventana.size()
    );
    crate::println!("usb: decodificacion de memoria encendida, bus mastering apagado.");
    crate::println!(
        "usb: region DMA de prueba reservada en 0x{:X} ({} bytes) y devuelta al gestor.",
        region.base(),
        region.bytes()
    );
    crate::println!("usb: controlador marcado como en uso.");
    crate::println!("usb: la inicializacion del HCD (fase 2) sigue pendiente.");

    // La region de prueba se devuelve: no la usa el dispositivo.
    let mut region = region;
    let _ = region.liberar(&mut dma::MarcosDelKernel);
    Ok(())
}

/// Arranca la inicializacion del controlador de host en segundo plano.
///
/// La inicializacion es una tarea de fondo: el arranque no espera a que el
/// controlador este listo (regla 2 del plan). La tarea publica su estado y la
/// consola lo muestra.
pub fn arrancar() {
    // No reservar pila ni planificar trabajo de fondo si el bus no contiene
    // ningun HCD xHCI compatible. En equipos sin USB xHCI, el inventario ya
    // dejo constancia del estado y PS/2 sigue siendo la entrada activa.
    if !controladores()
        .iter()
        .any(|c| c.tipo == Some(TipoHcd::Xhci))
    {
        return;
    }

    let parent = crate::tarea::pid_actual().unwrap_or(0);
    let _ =
        crate::tarea::crear_con_clase("usb-hcd", tarea_usb, 0, crate::tarea::Clase::Normal, parent);
}

extern "C" fn tarea_usb(_arg: u64) -> ! {
    // Recopilar los controladores xHCI. La tarea corre despues de tarea::init(),
    // asi que todo lo necesario ya esta inicializado.
    let lista: &[Controlador] = controladores();
    for c in lista {
        if c.tipo != Some(TipoHcd::Xhci) {
            continue;
        }
        match hcd::xhci::iniciar_controlador(c) {
            Ok(controlador) => {
                // Publicar el estado inicial.
                controlador.publicar_estado();
                // El subsistema USB ya tiene un controlador en marcha.
                {
                    let e = estado_global();
                    *e = Estado::Inicializado;
                }
                // Entrar en el bucle de sondeo (nunca vuelve).
                controlador.bucle();
            }
            Err(e) => {
                crate::println!(
                    "usb: el controlador xHCI no se pudo inicializar: {}",
                    e.nombre()
                );
                hcd::publicar(hcd::Publicado {
                    nombre: "xHCI",
                    estado: hcd::EstadoHcd::Fallido,
                    detalle: e.nombre(),
                    contadores: hcd::Contadores::default(),
                    num_puertos: 0,
                    puertos: [hcd::InfoPuerto::VACIO; hcd::MAX_PUERTOS],
                    num_dispositivos: 0,
                    dispositivos: [hcd::InfoDispositivo::VACIO; hcd::MAX_DISPOSITIVOS_PUBLICADOS],
                });
                // Actualizar el estado global del subsistema.
                {
                    let e = estado_global();
                    *e = Estado::Fallido;
                }
                loop {
                    crate::tarea::dormir(1000);
                }
            }
        }
    }
    // No hay ningun controlador xHCI que inicializar.
    loop {
        crate::tarea::dormir(1000);
    }
}

/// Orden `usb`: inventario de los controladores de host USB y estado de la pila.
///
/// Distingue con claridad los hitos del plan (HCD detectado, soportado,
/// inicializado y en marcha, puerto con algo conectado, dispositivo enumerado
/// e HID activo). Lo que no esta disponible se dice como "no", para no dar la
/// impresion de que hay mas de lo que hay.
///
/// Con `medir` o `preparar` hace mas cosas (y escribe en el dispositivo); con
/// el HCD ya en marcha ambas se rechazan para no tocar un controlador vivo.
pub fn comando(arg: &str) {
    match arg {
        "medir" => sub_medir(),
        "preparar" => sub_preparar(),
        "" => sub_inventario(),
        otro => {
            crate::println!("usb: subcomando desconocido: {}", otro);
            crate::println!("usb: uso: usb [medir|preparar]");
        }
    }
}

/// `usb`: inventario de solo lectura.
fn sub_inventario() {
    let lista = controladores();
    if lista.is_empty() {
        crate::println!("usb: inventario de controladores de host USB");
        crate::println!("usb: no hay ninguna funcion de clase 0C/0C03 en el bus.");
        crate::println!("usb: estado: {}", Estado::SinControlador.nombre());
        crate::println!("usb: la entrada del sistema sigue siendo PS/2 y el puerto serie.");
        return;
    }

    crate::println!("usb: inventario de controladores de host USB (solo lectura)");
    for c in lista {
        let f = &c.funcion;
        crate::print!(
            "  {:02X}:{:02X}.{}  {:04X}:{:04X}  clase {:02X} subclase {:02X} prog_if {:02X}  {}",
            f.bus,
            f.dev,
            f.func,
            f.vendor,
            f.device,
            f.clase,
            f.subclase,
            f.prog_if,
            c.nombre_tipo()
        );
        crate::println!();
        crate::print!(
            "    fabricante: {}",
            crate::pci::fabricante_nombre(f.vendor)
        );
        crate::println!();

        // Los BAR, tal y como los asigno el firmware.
        for (i, b) in c.bares.iter().enumerate() {
            if let Some(b) = b {
                crate::print!(
                    "    BAR{}  base 0x{:012X}  {}  {}",
                    i,
                    b.base,
                    if b.es_io { "espacio de E/S" } else { "memoria" },
                    if b.es_64 { "64 bits" } else { "32 bits" }
                );
                crate::println!();
            }
        }

        match c.registros {
            Some(b) => {
                crate::print!("    registros en 0x{:012X}", b.base);
                if c.direccionable {
                    crate::println!("  (cubierto por el mapa por identidad)");
                } else {
                    crate::println!("  (ATENCION: por encima de 4 GiB, no direccionable)");
                }
            }
            None => crate::println!("    registros: sin BAR de memoria asignado"),
        }

        if let Some(b) = c.medido {
            crate::println!(
                "    ventana de registros medida: 0x{:X} + 0x{:X} bytes",
                b.base,
                b.size
            );
        }

        // El estado del bus mastering, leido (no se toca nada).
        crate::print!(
            "    bus mastering: {}",
            if crate::pci::bus_master_activo(*f) {
                "encendido (lo dejo el firmware)"
            } else {
                "apagado"
            }
        );
        crate::println!();
    }

    let estado = estado();
    crate::println!("usb: estado: {}", estado.nombre());

    // Los hitos del plan, uno a uno. Los que no existen se dicen como "no".
    let detectable = estado != Estado::SinControlador;
    let soportable = estado != Estado::SinControlador && estado != Estado::SinSoporte;
    crate::println!("usb: hitos:");
    crate::println!(
        "    sin controlador ........ {}",
        if estado == Estado::SinControlador {
            "si"
        } else {
            "no"
        }
    );
    crate::println!(
        "    HCD detectado .......... {}",
        if detectable { "si" } else { "no" }
    );
    crate::println!(
        "    HCD soportado por rocky . {}",
        if soportable { "si" } else { "no" }
    );
    crate::println!(
        "    HCD inicializado ....... {}",
        if estado == Estado::Inicializado {
            "si"
        } else {
            "no"
        }
    );
    crate::println!(
        "    fallo al iniciar ....... {}",
        if estado == Estado::Fallido {
            "si"
        } else {
            "no"
        }
    );
    // El estado del HCD desde la tarea de fondo.
    let hcd_pub = hcd::publicado();
    if !hcd_pub.nombre.is_empty() {
        crate::println!("usb: HCD {}: {}", hcd_pub.nombre, hcd_pub.estado.nombre());
    }
    if hcd_pub.estado == hcd::EstadoHcd::EnMarcha {
        let mut hay_conectado = false;
        for p in hcd_pub.puertos.iter().take(hcd_pub.num_puertos as usize) {
            if p.conectado {
                hay_conectado = true;
                break;
            }
        }
        crate::println!(
            "    puerto conectado ....... {}",
            if hay_conectado { "si" } else { "no" }
        );
        // Mostrar los puertos raiz.
        if hcd_pub.num_puertos > 0 {
            crate::println!("usb: puertos raiz (xHCI):");
            for p in hcd_pub.puertos.iter().take(hcd_pub.num_puertos as usize) {
                crate::println!(
                    "    {}  {}  {}  {}  {}",
                    p.numero,
                    p.protocolo,
                    if p.conectado {
                        "conectado"
                    } else {
                        "sin dispositivo"
                    },
                    p.velocidad.nombre(),
                    if p.habilitado {
                        "habilitado"
                    } else {
                        "no habilitado"
                    }
                );
            }
        }
        crate::println!(
            "usb: contadores: sondeos {}, eventos {}, cambios de puerto {}, desconocidos {}, errores {}, dispositivos {}, puertos fallidos {}, informes HID {}, errores HID {}, descartes HID {}, teclas HID {}, cola HID max {}, latencia HID max {} ms",
            hcd_pub.contadores.sondeos,
            hcd_pub.contadores.eventos,
            hcd_pub.contadores.cambios_puerto,
            hcd_pub.contadores.desconocidos,
            hcd_pub.contadores.errores,
            hcd_pub.contadores.dispositivos,
            hcd_pub.contadores.errores_enum,
            hcd_pub.contadores.informes_hid,
            hcd_pub.contadores.errores_hid,
            crate::input::hid_descartes(),
            crate::input::hid_publicados(),
            crate::input::hid_cola_max(),
            crate::input::hid_latencia_max_ms()
        );
        // Los dispositivos enumerados, con lo que hace falta para saber que son.
        if hcd_pub.num_dispositivos > 0 {
            crate::println!("usb: dispositivos enumerados:");
            for d in hcd_pub
                .dispositivos
                .iter()
                .take(hcd_pub.num_dispositivos as usize)
            {
                crate::println!(
                    "    puerto {}  hueco {}  direccion {}  {:04X}:{:04X}  clase {}  {} interfaces ({} HID)  {} endpoints  hasta {} mA  configuracion {}  {} HID activos  {} informes",
                    d.puerto,
                    d.hueco,
                    d.direccion,
                    d.id_vendor,
                    d.id_product,
                    d.clase,
                    d.interfaces,
                    d.interfaces_hid,
                    d.endpoints,
                    d.consumo_ma,
                    d.configuracion,
                    d.hid,
                    d.informes
                );
            }
        }
    } else if hcd_pub.estado == hcd::EstadoHcd::Fallido {
        crate::println!("    puerto conectado ....... no");
        if !hcd_pub.detalle.is_empty() {
            crate::println!("usb: motivo del fallo: {}", hcd_pub.detalle);
        }
    } else {
        crate::println!("    puerto conectado ....... no (todavia no se sondea)");
    }
    crate::println!(
        "    dispositivo enumerado .. {}",
        if hcd_pub.num_dispositivos > 0 {
            "si"
        } else if hcd_pub.estado == hcd::EstadoHcd::EnMarcha {
            "no (ninguno conectado o ninguno se pudo enumerar)"
        } else {
            "no"
        }
    );
    let hid_activos: u32 = hcd_pub
        .dispositivos
        .iter()
        .take(hcd_pub.num_dispositivos as usize)
        .map(|d| d.hid as u32)
        .sum();
    crate::println!(
        "    HID activo ............. {} ({} endpoints, {} informes, {} errores)",
        if hid_activos > 0 { "si" } else { "no" },
        hid_activos,
        hcd_pub.contadores.informes_hid,
        hcd_pub.contadores.errores_hid
    );
    crate::println!(
        "usb: la emulacion USB heredada no se toca; PS/2 y puerto serie siguen activos."
    );
}

/// `usb medir`: mide el tamano de las ventanas del BAR.
fn sub_medir() {
    let n = controladores().len();
    if n == 0 {
        crate::println!("usb: no hay ningun controlador de host USB que medir.");
        return;
    }
    crate::println!("usb: midiendo las ventanas de los BAR (escribe en los registros del BAR)...");
    for i in 0..n {
        // Copia: el inventario se puede modificar durante la sonda, asi que
        // no se puede dejar un prestamo vivo sobre el.
        let c = controladores()[i];
        let f = c.funcion;
        match medir(&c) {
            Some(b) => {
                let (n, u) = crate::pci::tam_txt(b.size);
                crate::println!(
                    "  {:02X}:{:02X}.{}  ventana de registros 0x{:012X}, tamano {} {}",
                    f.bus,
                    f.dev,
                    f.func,
                    b.base,
                    n,
                    u
                );
            }
            None => crate::println!(
                "  {:02X}:{:02X}.{}  no se pudo medir (el controlador esta en uso)",
                f.bus,
                f.dev,
                f.func
            ),
        }
    }
    crate::println!(
        "usb: durante la sonda, la decodificacion y el bus mastering estuvieron apagados."
    );
}

/// `usb preparar`: deja el controlador listo para la fase 2.
fn sub_preparar() {
    let n = controladores().len();
    if n == 0 {
        crate::println!("usb: no hay ningun controlador de host USB que preparar.");
        return;
    }
    for i in 0..n {
        // Copia, por el mismo motivo que en `usb medir`.
        let c = controladores()[i];
        match preparar(&c) {
            Ok(()) => {}
            Err(motivo) => {
                let f = &c.funcion;
                crate::println!(
                    "usb: controlador {:02X}:{:02X}.{}: {}",
                    f.bus,
                    f.dev,
                    f.func,
                    motivo
                );
            }
        }
    }
}
