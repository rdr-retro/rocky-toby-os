//! Driver del controlador de host xHCI.
//!
//! ## Que hace
//!
//! - **Detecta** el controlador por su clase de PCI (`0C/03/30`), nunca por
//!   su fabricante.
//! - **Toma el control** desde el firmware (el semaforo `USBLEGSUP`), con un
//!   plazo: si el firmware no suelta en un segundo, el controlador se
//!   descarta y se sigue con PS/2.
//! - **Detiene y reinicia** el controlador, y lo **programa** con las
//!   estructuras que necesita (DCBAA, anillos, ERST, scratchpads).
//! - **Arranca** el controlador y **sondea** sus puertos raiz.
//! - **Enumera** los dispositivos conectados a los puertos raiz: reserva un
//!   hueco, lo direcciona, lee sus descriptores y lo configura
//!   ([`enumeracion`]).
//! - **Configura** los hubs USB 2.0: los alimenta, sondea sus puertos uno por
//!   ronda, usa *route string* y TT, y enruta a sus hijos. Los hubs anidados se
//!   recorren con tope de profundidad.
//! - **Detecta** conexiones y desconexiones en caliente, tanto en los puertos
//!   raiz como en los puertos de un hub. Al quitar un hub se retiran antes sus
//!   descendientes y sus endpoints.
//!
//! ## Que NO hace (todavia)
//!
//! - No usa interrupciones: el anillo de eventos se sondea desde la tarea de
//!   fondo, con un limite de eventos por ronda.
//! - No hay soporte para EHCI/UHCI/OHCI ni para hubs USB 3.x.
//!
//! ## Reglas del plan que respeta
//!
//! - La inicializacion es una **tarea de fondo** con plazos por paso: el
//!   arranque no espera.
//! - El bus mastering no se enciende hasta que hay memoria DMA reservada.
//! - Un dispositivo que no se puede enumerar solo tumba su puerto: su hueco se
//!   devuelve al controlador y el resto sigue.
//! - La emulacion USB heredada no se toca: si algo falla, PS/2 sigue.
//!
//! Referencia: Intel, "eXtensible Host Controller Interface for Universal
//! Serial Bus (xHCI) Requirements Specification", Rev 1.2.
//!
//! Codigo propio de rocky.

pub mod cmd;
pub mod enumeracion;
mod hid;
pub mod ports;
pub mod regs;
pub mod ring;

use crate::drivers::usb::hcd::{Contadores, MAX_PUERTOS};
use crate::drivers::usb::mmio;
use crate::drivers::usb::pci::Controlador as Inventario;
use crate::pci::{Bar, Funcion};
use crate::pit;
use crate::tarea;
use rockyhw::xhci::caps::{Capacidades, ErrorCap};
use rockyhw::xhci::regs::*;
use rockyhw::xhci::ring::{
    AnilloComando, AnilloEvento, MemoriaTrb, TipoEvento, FINALIZACION_CORRECTA,
};

use ports::Puerto;
use regs::{Registros, VentanaCapacidades};
use ring::Estructuras;

/// Plazo para esperar al firmware, en tic del PIT (100 tic = 1 segundo).
const PLAZO_FIRMWARE: u64 = 100;
/// Plazo para las esperas del controlador, en tic del PIT (100 tic = 1 segundo).
const PLAZO_CONTROLADOR: u64 = 100;
/// Plazo para el reinicio de un puerto, en tic del PIT (100 tic = 1 segundo).
pub const PLAZO_PUERTO: u64 = 100;
/// Maximo de eventos que se leen del anillo en una ronda.
const MAX_EVENTOS_POR_RONDA: usize = 64;
/// Cada cuanto se refrescan los puertos, en milisegundos.
const REFRESCO_PUERTOS_MS: u64 = 100;
/// Cada cuanto se sondea el anillo cuando hay un HID activo, en milisegundos.
///
/// Un teclado USB avisa por su endpoint de interrupcion; con una ronda cada
/// 100 ms la escritura se notaria a tirones. `dormir` redondea a tics del PIT,
/// asi que 10 ms es el minimo real y basta para un endpoint que sondea cada
/// 8 ms.
const REFRESCO_HID_MS: u64 = 10;

/// Por que no se pudo inicializar el controlador.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ErrorXhci {
    /// No se pudo abrir la ventana de registros.
    VentanaInvalida,
    /// El firmware no deja encender la decodificacion de memoria.
    Decodificacion,
    /// Las capacidades del controlador no cuadran.
    Capacidades(ErrorCap),
    /// Los registros que declara el controlador no caben en su BAR.
    RegistrosDesbordados,
    /// El firmware no suelta el controlador (el semaforo `USBLEGSUP`).
    FirmwareRetenido,
    /// El controlador no responde.
    SinRespuesta,
    /// No hay memoria para las estructuras DMA.
    SinMemoria,
    /// El bus mastering no se puede encender.
    BusMaster,
}

impl ErrorXhci {
    /// Nombre corto, para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            ErrorXhci::VentanaInvalida => "ventana de registros invalida",
            ErrorXhci::Decodificacion => "no se puede encender la decodificacion de memoria",
            ErrorXhci::Capacidades(ErrorCap::SinProtocolos) => {
                "no hay capacidades de protocolos soportados"
            }
            ErrorXhci::Capacidades(_) => "capacidades del controlador invalidas",
            ErrorXhci::RegistrosDesbordados => "los registros no caben en el BAR",
            ErrorXhci::FirmwareRetenido => "el firmware no suelta el controlador",
            ErrorXhci::SinRespuesta => "el controlador no responde",
            ErrorXhci::SinMemoria => "sin memoria para las estructuras DMA",
            ErrorXhci::BusMaster => "no se puede encender el bus mastering",
        }
    }
}

/// Un controlador xHCI inicializado y en marcha.
pub struct Controlador {
    regs: Registros,
    dma: Estructuras,
    anillo: AnilloEvento,
    /// El anillo de comandos: lo escribe el software, lo lee el controlador.
    cmd_ring: AnilloComando,
    /// La disposicion de los contextos de dispositivo dentro de la region de
    /// contextos.
    contextos: ring::Contextos,
    puertos: [Puerto; MAX_PUERTOS],
    num_puertos: u8,
    /// Los dispositivos enumerados. La posicion en la lista no tiene por que
    /// coincidir con el hueco: un hueco se libera y otro dispositivo puede
    /// ocuparlo.
    dispositivos: [enumeracion::Dispositivo; ring::MAX_HUECOS],
    num_dispositivos: u8,
    /// La disposicion de los anillos y buffers de los endpoints de
    /// interrupcion.
    interrupciones: ring::Interrupciones,
    /// Los endpoints de interrupcion (HID boot) activos.
    ep_activos: [hid::EpHid; ring::MAX_EP_INTERRUPCION],
    /// Cuantos de esos endpoints estan en uso.
    num_ep_activos: u8,
    contadores: Contadores,
}

/// La memoria del anillo de eventos, para leer TRBs.
///
/// El segmento vive en la region DMA que hemos reservado.
#[derive(Clone, Copy)]
struct MemoriaAnillo {
    base: u64,
    fin: u64,
}

impl MemoriaTrb for MemoriaAnillo {
    fn leer_trb(&self, addr: u64) -> Option<rockyhw::xhci::ring::Trb> {
        if addr < self.base {
            return None;
        }
        let fin_trb = addr.checked_add(16)?;
        if fin_trb > self.fin {
            return None;
        }
        // El evento lo ha escrito el controlador, no el procesador: la linea de
        // cache que el procesador pudiera tener de una vuelta anterior no vale.
        // En x86 no hay snooping de la DMA, asi que se tira antes de leer.
        mmio::clflush_rango(addr, 16);
        mmio::mfence();
        Some(ring::leer_trb(addr))
    }
}

impl Controlador {
    /// Inicializa el controlador xHCI de una funcion de PCI.
    pub fn iniciar(funcion: Funcion, bar: Bar) -> Result<Controlador, ErrorXhci> {
        // 1. La ventana de registros tiene que caber en el mapa por identidad.
        let ventana = mmio::ventana(&bar).map_err(|_| ErrorXhci::VentanaInvalida)?;

        // 2. Encender la decodificacion de memoria. El bus mastering sigue
        //    apagado: no se enciende hasta que el dispositivo tenga memoria
        //    reservada.
        crate::pci::habilitar_memoria(funcion);
        mmio::mfence();
        if !crate::pci::memoria_activa(funcion) {
            return Err(ErrorXhci::Decodificacion);
        }

        // 3. Las capacidades del controlador.
        let caps =
            Capacidades::leer(&VentanaCapacidades(ventana)).map_err(ErrorXhci::Capacidades)?;

        // 4. Los registros que declara el controlador tienen que caber en su BAR.
        if !caps.cabe_en(ventana.size()) {
            return Err(ErrorXhci::RegistrosDesbordados);
        }
        let regs = Registros::nueva(ventana, caps);

        // 5. Tomar el control desde el firmware.
        tomar_el_control(&regs, PLAZO_FIRMWARE)?;

        // 6. Detener el controlador.
        detener(&regs, PLAZO_CONTROLADOR)?;

        // 7. Reiniciar el controlador.
        reiniciar(&regs, PLAZO_CONTROLADOR)?;

        // 8. Esperar a que el controlador este preparado.
        esperar_preparado(&regs, PLAZO_CONTROLADOR)?;

        // 9. Reservar e inicializar las estructuras de memoria.
        let mut dma =
            Estructuras::reservar(regs.capacidades()).map_err(|_| ErrorXhci::SinMemoria)?;
        dma.inicializar(regs.capacidades());
        let contextos =
            ring::Contextos::nueva(dma.contextos.base() as u64, dma.contextos.bytes() as u64);
        if !contextos.es_valido() {
            return Err(ErrorXhci::SinMemoria);
        }
        let interrupciones = ring::Interrupciones::nueva(
            dma.interrupcion.base() as u64,
            dma.interrupcion.bytes() as u64,
        );
        if !interrupciones.es_valido() {
            return Err(ErrorXhci::SinMemoria);
        }
        mmio::mfence();

        // 10. Ahora si: el bus mastering.
        if !crate::pci::habilitar_memoria_y_bus_master(funcion) {
            return Err(ErrorXhci::BusMaster);
        }

        // 11. Programar los registros.
        programar(&regs, &dma, regs.capacidades())?;

        // 12. Arrancar el controlador.
        arrancar(&regs, PLAZO_CONTROLADOR)?;

        // 13. Preparar y sondear los puertos raiz.
        let anillo_comandos =
            match AnilloComando::nuevo(dma.comandos.base() as u64, ring::TRB_COMANDOS) {
                Some(a) => a,
                None => return Err(ErrorXhci::SinMemoria),
            };
        let mut controlador = Controlador {
            regs,
            anillo: AnilloEvento::nuevo(dma.eventos.base() as u64, ring::TRB_EVENTOS),
            cmd_ring: anillo_comandos,
            contextos,
            puertos: [Puerto::vacio(0, None); MAX_PUERTOS],
            num_puertos: 0,
            dispositivos: [enumeracion::Dispositivo::VACIO; ring::MAX_HUECOS],
            num_dispositivos: 0,
            interrupciones,
            ep_activos: [hid::EpHid::VACIO; ring::MAX_EP_INTERRUPCION],
            num_ep_activos: 0,
            contadores: Contadores::default(),
            dma,
        };
        controlador.preparar_puertos();
        controlador.escanear_puertos();

        // Una linea: el arranque no debe perder tiempo con USB, pero hay que
        // poder ver desde la consola que el controlador ha woken up.
        crate::println!(
            "usb: xHCI inicializado: {} puertos raiz, controlador en marcha.",
            controlador.num_puertos
        );

        Ok(controlador)
    }

    /// La memoria del anillo de eventos, para leer TRBs.
    fn memoria_eventos(&self) -> MemoriaAnillo {
        let base = self.dma.eventos.base() as u64;
        MemoriaAnillo {
            base,
            fin: base + self.dma.eventos.bytes() as u64,
        }
    }

    /// El bucle principal: sondea el anillo de eventos y refresca los puertos.
    ///
    /// Nunca vuelve. Publica su estado tras cada ronda.
    pub fn bucle(mut self) -> ! {
        let memoria = self.memoria_eventos();
        loop {
            self.contadores.sondeos += 1;
            // Sondear el anillo de eventos (acotado).
            let mut leidos = 0;
            while leidos < MAX_EVENTOS_POR_RONDA {
                match self.anillo.siguiente(&memoria) {
                    Some(trb) => {
                        self.anillo.avanzar();
                        leidos += 1;
                        self.contadores.eventos += 1;
                        // Los tipos de evento son los de la tabla 6-91:
                        // 32 transferencia, 33 fin de comando, 34 cambio de
                        // puerto, 35 ancho de banda, 36 puerta, 37
                        // controlador de host, 38 notificacion de dispositivo
                        // y 39 vuelta de microframes.
                        match TipoEvento::de_codigo(trb.tipo()) {
                            TipoEvento::CambioPuerto => {
                                self.contadores.cambios_puerto += 1;
                            }
                            TipoEvento::Transferencia => {
                                // Los eventos de las transferencias de control
                                // los consume `control_ep0`; aqui llegan los
                                // de los endpoints de interrupcion.
                                if !self.atender_transferencia(&trb) {
                                    self.contadores.desconocidos += 1;
                                }
                            }
                            TipoEvento::Desconocido(_) => {
                                self.contadores.desconocidos += 1;
                            }
                            TipoEvento::ControladorHost => {
                                let codigo = trb.codigo_finalizacion();
                                if codigo != FINALIZACION_CORRECTA {
                                    self.contadores.errores += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    None => break,
                }
            }
            // Devolver el puntero de extraccion al controlador. Se escribe
            // tambien `EHB` para limpiar la bandera de "manejador ocupado":
            // mientras siga a uno, el controlador cree que el software no ha
            // terminado con el anillo (§5.5.2.3.3).
            self.devolver_puntero_eventos();
            // Reponer la transferencia de los endpoints de interrupcion que ya
            // terminaron. Se hace aqui, y no dentro del drenaje, para no encolar
            // eventos que esta ronda ya no va a leer.
            self.rearmar_interrupciones();
            // Limpiar las banderas de "escribir un 1 para limpiar" del estado
            // del controlador: la de cambio de puerto y la de interrupcion
            // pendiente (§5.4.2). Sin esto, `PCD` se quedaria a uno para
            // siempre.
            let _ = self.regs.escribir_usbsts(STS_PCD | STS_EINT);
            // Refrescar los puertos.
            self.refrescar_puertos();
            // Enumerar, como mucho, un dispositivo por ronda: una enumeracion
            // son varios comandos y tres transferencias de control, y el bucle
            // no puede quedarse ahi el resto de la ronda.
            self.enumerar_pendientes();
            // Publicar el estado.
            crate::drivers::usb::hcd::publicar(self.captura_publicada());
            // Esperar antes de la siguiente ronda. Con un HID activo hay que
            // sondear mucho mas a menudo: el teclado solo avisa de cada tecla
            // por su endpoint.
            let espera = if self.num_ep_activos > 0 {
                REFRESCO_HID_MS
            } else {
                REFRESCO_PUERTOS_MS
            };
            tarea::dormir(espera);
        }
    }

    /// Publica una instantanea del estado.
    pub fn publicar_estado(&self) {
        crate::drivers::usb::hcd::publicar(self.captura_publicada());
    }
}

/// Espera activa con plazo.
///
/// `plazo` es el numero de tic del PIT (100 tic = 1 segundo). Devuelve `true`
/// si la condicion se cumple antes de que se agote el plazo.
fn esperar(plazo: u64, mut condicion: impl FnMut() -> bool) -> bool {
    let inicio = pit::ticks();
    loop {
        if condicion() {
            return true;
        }
        if pit::ticks().saturating_sub(inicio) >= plazo {
            return false;
        }
        tarea::dormir(1);
    }
}

/// Toma el control del controlador desde el firmware (§4.22.1).
fn tomar_el_control(regs: &Registros, plazo: u64) -> Result<(), ErrorXhci> {
    let off = match regs.capacidades().usblegsup {
        Some(o) => o,
        None => return Ok(()),
    };
    let valor = regs
        .ventana()
        .leer32(off)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // El sistema operativo pone su semaforo a uno (siempre, para tomar la
    // propiedad).
    let valor = valor | LEGSUP_OS;
    regs.ventana()
        .escribir32(off, valor)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // Si la BIOS ya lo habia soltado, no hay nada que esperar.
    if valor & LEGSUP_BIOS == 0 {
        return Ok(());
    }
    let solto = esperar(plazo, || match regs.ventana().leer32(off) {
        Ok(v) => v & LEGSUP_BIOS == 0,
        Err(_) => false,
    });
    if !solto {
        return Err(ErrorXhci::FirmwareRetenido);
    }
    Ok(())
}

/// Detiene el controlador (§4.2).
fn detener(regs: &Registros, plazo: u64) -> Result<(), ErrorXhci> {
    let cmd = regs.usbcmd().map_err(|_| ErrorXhci::VentanaInvalida)?;
    if cmd & CMD_RUN != 0 {
        regs.escribir_usbcmd(cmd & !CMD_RUN)
            .map_err(|_| ErrorXhci::VentanaInvalida)?;
    }
    let parado = esperar(plazo, || match regs.usbsts() {
        Ok(s) => s & STS_HCH != 0,
        Err(_) => false,
    });
    if !parado {
        return Err(ErrorXhci::SinRespuesta);
    }
    Ok(())
}

/// Reinicia el controlador (§4.2).
fn reiniciar(regs: &Registros, plazo: u64) -> Result<(), ErrorXhci> {
    let cmd = regs.usbcmd().map_err(|_| ErrorXhci::VentanaInvalida)?;
    regs.escribir_usbcmd(cmd | CMD_HCRST)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // El bit se borra solo cuando el reinicio termina.
    let listo = esperar(plazo, || match regs.usbcmd() {
        Ok(c) => c & CMD_HCRST == 0,
        Err(_) => false,
    });
    if !listo {
        return Err(ErrorXhci::SinRespuesta);
    }
    Ok(())
}

/// Espera a que el controlador este preparado (§4.2).
fn esperar_preparado(regs: &Registros, plazo: u64) -> Result<(), ErrorXhci> {
    let listo = esperar(plazo, || match regs.usbsts() {
        Ok(s) => s & STS_CNR == 0,
        Err(_) => false,
    });
    if !listo {
        return Err(ErrorXhci::SinRespuesta);
    }
    Ok(())
}

/// Programa los registros del controlador (§4.2).
fn programar(regs: &Registros, dma: &Estructuras, caps: &Capacidades) -> Result<(), ErrorXhci> {
    // DCBAAP: el array de contextos base.
    regs.escribir64_op(OP_DCBAAP, dma.dcbaa.base() as u64)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // CRCR: el anillo de comandos, con RCS = 1. No se arranca con RR: este
    // registro no tiene ningun bit de "arrancar" (§5.4.5).
    regs.escribir64_op(
        OP_CRCR,
        rockyhw::xhci::ring::crcr(dma.comandos.base() as u64),
    )
    .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // CONFIG: cuantos huecos de dispositivo admite el controlador.
    regs.escribir_op(OP_CONFIG, caps.max_slots as u32)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;

    // Interrupter 0.
    // ERSTSZ: numero de entradas en la ERST.
    regs.escribir_rt(RT_INTERRUPTER + INT_ERSTSZ, 1)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // ERSTBA: la tabla de segmentos. Su escritura de la parte alta es la que
    // hace que el controlador lea la tabla (§5.5.2.3.2).
    regs.escribir64_rt(RT_INTERRUPTER + INT_ERSTBA, dma.erst.base() as u64)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // ERDP: apunta al principio del anillo de eventos.
    regs.escribir64_rt(RT_INTERRUPTER + INT_ERDP, dma.eventos.base() as u64)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // IMOD: moderacion de interrupciones. En esta fase no hay interrupciones
    // habilitadas, pero dejamos un valor razonable.
    regs.escribir_rt(RT_INTERRUPTER + INT_IMOD, 0)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    // IMAN: sin interrupciones habilitadas.
    regs.escribir_rt(RT_INTERRUPTER + INT_IMAN, 0)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;

    mmio::mfence();
    Ok(())
}

/// Arranca el controlador (§4.2).
fn arrancar(regs: &Registros, plazo: u64) -> Result<(), ErrorXhci> {
    let cmd = regs.usbcmd().map_err(|_| ErrorXhci::VentanaInvalida)?;
    // Arrancar el controlador. Las interrupciones no se habilitan.
    regs.escribir_usbcmd(cmd | CMD_RUN)
        .map_err(|_| ErrorXhci::VentanaInvalida)?;
    let corriendo = esperar(plazo, || match regs.usbsts() {
        Ok(s) => s & STS_HCH == 0,
        Err(_) => false,
    });
    if !corriendo {
        return Err(ErrorXhci::SinRespuesta);
    }
    Ok(())
}

/// Inicializa el controlador xHCI que hay en el inventario.
pub fn iniciar_controlador(c: &Inventario) -> Result<Controlador, ErrorXhci> {
    // El controlador pasa a ser de rocky **antes** de tocarlo: a partir de
    // aqui nadie mas mide sus BAR, lo reinicia ni le apaga el bus mastering.
    // Por eso la sonda no puede mirar la marca de uso.
    crate::pci::marcar_en_uso(c.funcion);
    let bar = match c.medido {
        Some(b) => b,
        None => crate::drivers::usb::medir_para_hcd(c).ok_or(ErrorXhci::VentanaInvalida)?,
    };
    Controlador::iniciar(c.funcion, bar)
}
