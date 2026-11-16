//! Los endpoints de interrupcion de los dispositivos HID.
//!
//! ## Que hace
//!
//! Cuando la enumeracion ya ha configurado el dispositivo, recorre sus
//! interfaces y, para cada interfaz HID, programa su endpoint de interrupcion
//! de entrada. Hay dos formas de leer los informes:
//!
//! - **Protocolo boot** (subclase 1, protocolo 1 teclado o 2 raton): formato
//!   fijo. Si el ajuste elegido no es el 0 se activa con `SET_INTERFACE`
//!   (seccion 9.4.10), se piden `SET_PROTOCOL(Boot)` y `SET_IDLE(0)` y se
//!   comprueba que el dispositivo los acepta.
//! - **Descriptor de informe** (fase 6): para cualquier otro HID. Se pide el
//!   descriptor de informe, se interpreta con `rockyhw::hid::informe` y se
//!   guardan los campos. Solo se aceptan si resultan ser un teclado (pagina de
//!   teclado) o un raton **relativo**; una tableta absoluta se rechaza, porque
//!   sus X/Y son una posicion y no un desplazamiento. `SET_PROTOCOL` no se
//!   manda y `SET_IDLE` es opcional. Si un dispositivo que declara boot
//!   rechaza `SET_PROTOCOL`, se intenta esta segunda via antes de descartarlo.
//!
//! En los dos casos se programa el endpoint con `Configure Endpoint` (seccion
//! 4.6.6) y se deja una transferencia `Normal` preparada en su anillo.
//!
//! ## Quien consume los informes
//!
//! Nada de esto dibuja ni escribe en la consola desde el evento: el evento solo
//! deja bytes en la cola de [`crate::input`] (teclado) o acumula el movimiento
//! en [`crate::mouse`]. La traduccion a caracteres la hace el consumidor, en la
//! misma capa que el PS/2, para que la distribucion espanola sea una sola.
//!
//! ## Teclado
//!
//! El informe (boot o generico) se convierte a modificadores + hasta seis usos
//! HID. Se compara con el anterior y se encola la diferencia como codigos de
//! barrido del juego 1, que es lo que ya entiende el resto del kernel. Un
//! `ErrorRollOver` suelta todo y no pulsa nada: inventar teclas es peor que
//! perder una.
//!
//! ## Raton
//!
//! Botones, `dx` y `dy` con signo. El eje Y crece hacia abajo, que es justo el
//! sistema de la pantalla. La rueda se ignora.
//!//! Referencia: USB HID 1.11, apendice B; xHCI 1.2, §4.6.6, §4.7.3 y §6.2.3;
//! Linux `drivers/hid/usbhid/hid-core.c` y `drivers/usb/host/xhci-ring.c`.
//!
//! Codigo propio de rocky.

use super::enumeracion::{
    escribir_contexto, velocidad_contexto, Endpoint, Interfaz, Topologia, TIPO_INTERRUPCION,
};
use super::ring;
use super::Controlador;
use crate::drivers::usb::core::transfer::{
    setup_set_idle, setup_set_interface, setup_set_protocol, PaqueteSetup, DT_REPORT,
    REQ_GET_DESCRIPTOR,
};
use crate::drivers::usb::mmio;
use rockyhw::hid::boot::{InformeRaton, InformeTeclado};
use rockyhw::hid::descriptor::largo_del_informe;
use rockyhw::hid::informe::{self, Dispositivo, Tipo as TipoInforme};
use rockyhw::hid::teclado;
use rockyhw::xhci::context::{
    ep_id, EndpointContext, InputControlContext, SlotContext, AÑADIR_SLOT, EP_ESTADO_INVALIDO,
    EP_TINT_IN,
};
use rockyhw::xhci::port::{intervalo_interrupcion, Velocidad};
use rockyhw::xhci::ring::{
    trb_configure_endpoint, trb_link, trb_normal, AnilloTransferencia, Trb, FINALIZACION_CORRECTA,
    FINALIZACION_ERROR_STALL, FINALIZACION_PAQUETE_CORTO,
};

/// El unico protocolo que esta fase sabe interpretar (HID 1.11, B.1).
const PROTOCOLO_BOOT: u16 = 0;

/// Subclase de interfaz "boot" (§4.2 de la especificacion de HID).
const SUBCLASE_BOOT: u8 = 1;

/// Protocolo de teclado en la subclase boot.
const PROTOCOLO_TECLADO: u8 = 1;

/// Protocolo de raton en la subclase boot.
const PROTOCOLO_RATON: u8 = 2;

/// Tope del descriptor de informe que se pide.
///
/// Es el mismo tope que el nucleo USB usa para un descriptor de configuracion
/// (`MAX_DESCRIPTORES_BYTES`). Un descriptor mayor se corta: se interpreta lo
/// que ha llegado y, si no alcanza para un teclado o un raton, la interfaz se
/// descarta.
const MAX_DESCRIPTOR_INFORME: u16 = 512;

/// Eventos maximos que puede emitir un informe.
///
/// Ocho modificadores que se pulsan, seis teclas que se sueltan, seis que se
/// pulsan y ocho modificadores que se sueltan dan 24 como mucho. Con 32 hay
/// margen de sobra y cabe en la pila sin reservar memoria.
const MAX_EVENTOS: usize = 32;

/// Un endpoint de interrupcion de entrada ya programado.
#[derive(Clone, Copy)]
pub struct EpHid {
    /// El hueco del dispositivo al que pertenece.
    pub hueco: u8,
    /// El indice de contexto del endpoint (`DCI`, §4.7.2).
    pub dci: u8,
    /// El numero de endpoint (bits 3:0 de la direccion).
    pub numero: u8,
    /// La interfaz HID a la que sirve.
    pub interfaz: u8,
    /// El protocolo boot: 1 teclado, 2 raton; 0 si se lee por descriptor.
    pub protocolo: u8,
    /// Si se lee por descriptor, que clase de dispositivo es.
    pub tipo_generico: TipoInforme,
    /// El descriptor de informe ya interpretado (vacio en modo boot).
    pub informe: Dispositivo,
    /// El tamano maximo de paquete que declara el endpoint.
    pub max_paquete: u16,
    /// Lo que mide el descriptor de informe que declaro el dispositivo.
    pub largo_reporte: u16,
    /// El anillo de transferencia del endpoint.
    pub anillo: AnilloTransferencia,
    /// El buffer donde el controlador deja el informe.
    pub buffer: u64,
    /// Si hay una transferencia esperando evento.
    pub pendiente: bool,
    /// Si el endpoint se paro (`STALL`) y no se vuelve a reponer.
    pub parado: bool,
    /// El ultimo informe de teclado, para calcular la diferencia.
    pub anteriores: InformeTeclado,
    /// Informes recibidos.
    pub informes: u32,
    /// Informes o eventos que no se pudieron interpretar.
    pub errores: u32,
}

impl EpHid {
    /// Un endpoint que no existe.
    pub const VACIO: EpHid = EpHid {
        hueco: 0,
        dci: 0,
        numero: 0,
        interfaz: 0,
        protocolo: 0,
        tipo_generico: TipoInforme::Desconocido,
        informe: Dispositivo::VACIO,
        max_paquete: 0,
        largo_reporte: 0,
        anillo: AnilloTransferencia {
            base: 0,
            trbs: ring::TRB_INTERRUPCION,
            indice: 0,
            ciclo: true,
            dio_la_vuelta: false,
        },
        buffer: 0,
        pendiente: false,
        parado: false,
        anteriores: InformeTeclado {
            modificadores: 0,
            teclas: [0; 6],
        },
        informes: 0,
        errores: 0,
    };

    /// Nombre del protocolo, para la consola.
    pub fn nombre_protocolo(&self) -> &'static str {
        match (self.protocolo, self.tipo_generico) {
            (PROTOCOLO_TECLADO, _) => "teclado boot",
            (PROTOCOLO_RATON, _) => "raton boot",
            (_, TipoInforme::Teclado) => "teclado HID",
            (_, TipoInforme::Raton) => "raton HID",
            _ => "HID",
        }
    }
}

impl Controlador {
    /// Programa los endpoints de interrupcion de las interfaces HID boot.
    ///
    /// Devuelve cuantos endpoints ha dejado activos. Un fallo en una interfaz
    /// **no** tumba el dispositivo: el resto de la enumeracion ya esta hecha y
    /// el dispositivo sigue configurado; simplemente esa interfaz no se usa.
    ///
    /// Se llama justo despues de `SET_CONFIGURATION`, que es cuando los
    /// endpoints de la configuracion existen para el controlador.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn preparar_hid(
        &mut self,
        hueco: u8,
        indice: usize,
        puerto: u8,
        velocidad: Velocidad,
        topologia: &Topologia,
        interfaces: &[Interfaz],
        endpoints: &[Endpoint],
        anillo_ep0: &mut AnilloTransferencia,
        buffer_ep0: u64,
    ) -> u8 {
        if !self.interrupciones.es_valido() {
            crate::println!(
                "usb: HID: la region de los endpoints de interrupcion no es valida; no se programa ninguno."
            );
            return 0;
        }
        let Some(entrada) = self.contextos.entrada(indice) else {
            crate::println!(
                "usb: HID del puerto {}: el contexto de entrada del hueco no es valido.",
                puerto
            );
            return 0;
        };

        let mut activos = 0;
        for interfaz in interfaces {
            if !interfaz.es_hid() {
                continue;
            }
            // Un numero de interfaz puede declarar varios ajustes alternos, pero
            // tras `SET_CONFIGURATION` solo esta activo el 0. Se elige uno por
            // numero: el 0 si es HID boot y, si no, el de menor numero que lo
            // sea. Los demas ajustes de ese numero se ignoran.
            let Some(ajuste) = ajuste_elegido(interfaces, interfaz.numero) else {
                if interfaz.alterna == 0 {
                    // La clase dice HID, pero ni la subclase/protocolo son
                    // boot ni hay descriptor que interpretar. Sin nada de eso
                    // no se toca: inventar teclas es peor que no tenerlas.
                    descartar(puerto, interfaz, "no se puede leer ni como boot ni por descriptor");
                }
                continue;
            };
            if interfaz.alterna != ajuste.alterna {
                continue;
            }
            // Si el HID esta en un ajuste distinto del 0 hay que activarlo
            // (§9.4.10) antes de programar su endpoint; si no, el endpoint no
            // existe para el dispositivo. Con el ajuste 0 sobra: es el que deja
            // activo `SET_CONFIGURATION`.
            if ajuste.alterna != 0 {
                let setup = setup_set_interface(interfaz.numero, ajuste.alterna);
                if self
                    .control_ep0(hueco, anillo_ep0, setup, buffer_ep0)
                    .is_err()
                {
                    descartar(puerto, interfaz, "no acepta SET_INTERFACE");
                    continue;
                }
            }
            let inicio = interfaz.primer_endpoint as usize;
            let fin = (inicio + interfaz.endpoints as usize).min(endpoints.len());
            let Some(endpoint) = endpoints
                .get(inicio..fin)
                .and_then(|lista| {
                    lista
                        .iter()
                        .find(|e| e.es_entrada() && e.tipo == TIPO_INTERRUPCION)
                })
                .copied()
            else {
                descartar(
                    puerto,
                    interfaz,
                    "no tiene endpoint de interrupcion de entrada",
                );
                continue;
            };
            // El DCI 1 es el endpoint 0 de control: un endpoint sin numero se
            // programaria encima de el.
            if endpoint.numero == 0 || endpoint.numero > 15 {
                descartar(puerto, interfaz, "el endpoint no tiene numero valido");
                continue;
            }
            if endpoint.max_paquete == 0
                || endpoint.max_paquete as usize > ring::TAM_BUFFER_INTERRUPCION
            {
                descartar(
                    puerto,
                    interfaz,
                    "el tamano de paquete no cabe en el buffer",
                );
                continue;
            }

            // 1. El descriptor HID ya viene en la configuracion (lo copio el
            //    recorrido de descriptores). Se valida aqui: no se pide suelto
            //    porque no todo el mundo contesta a `GET_DESCRIPTOR(0x21)`
            //    aunque lo declare (QEMU lo para con STALL).
            if interfaz.hid[0] == 0 {
                descartar(puerto, interfaz, "la interfaz no declara descriptor HID");
                continue;
            }
            let Some(largo_reporte) = largo_del_informe(&interfaz.hid) else {
                descartar(
                    puerto,
                    interfaz,
                    "el descriptor HID no declara el de informe",
                );
                continue;
            };
            // 2. El descriptor de informe. En modo boot solo se pide y se
            //    comprueba que el dispositivo contesta lo declarado; en modo
            //    generico es lo que dice que es el dispositivo y como se leen
            //    sus informes.
            let pedido = largo_reporte.min(MAX_DESCRIPTOR_INFORME);
            if pedido == 0 {
                descartar(puerto, interfaz, "el descriptor de informe mide cero");
                continue;
            }
            let setup = PaqueteSetup::nuevo(
                0x81,
                REQ_GET_DESCRIPTOR,
                (DT_REPORT as u16) << 8,
                interfaz.numero as u16,
                pedido,
            );
            let Ok(recibido) = self.control_ep0(hueco, anillo_ep0, setup, buffer_ep0) else {
                descartar(puerto, interfaz, "no contesta al descriptor de informe");
                continue;
            };
            if recibido == 0 {
                descartar(puerto, interfaz, "el descriptor de informe viene vacio");
                continue;
            }
            // El descriptor HID declara una longitud y el dispositivo deberia
            // contestar exactamente esa. Si contesta menos, el descriptor esta
            // mal declarado: se usa lo que ha llegado (rechazarlo dejaria sin
            // teclado a un dispositivo que funciona, y Linux tambien se queda con
            // lo que llega) pero se avisa.
            if recibido < pedido {
                crate::println!(
                    "usb: HID del puerto {}: el descriptor de informe viene corto ({} de {}); se usa igual.",
                    puerto,
                    recibido,
                    largo_reporte
                );
            }

            // Seguro: `recibido` no pasa de `MAX_DESCRIPTOR_INFORME`, que cabe
            // en el buffer de control ya reservado.
            let descriptor = unsafe {
                core::slice::from_raw_parts(buffer_ep0 as *const u8, recibido as usize)
            };

            // 3. Si es boot, se negocia `SET_PROTOCOL(Boot)`. Si lo rechaza
            //    (hay dispositivos que declaran boot y luego no lo aceptan) se
            //    intenta leer por su descriptor antes de darlo por perdido.
            let mut boot = ajuste.boot;
            if boot {
                let setup = setup_set_protocol(interfaz.numero, PROTOCOLO_BOOT as u8);
                if self
                    .control_ep0(hueco, anillo_ep0, setup, buffer_ep0)
                    .is_err()
                {
                    match informe::analizar(descriptor) {
                        Ok(d) if d.tipo != TipoInforme::Desconocido => boot = false,
                        _ => {
                            descartar(
                                puerto,
                                interfaz,
                                "no acepta SET_PROTOCOL(boot) ni sirve su descriptor",
                            );
                            continue;
                        }
                    }
                }
            }

            // 4. El modo: protocolo boot o descriptor de informe.
            let (protocolo, tipo, dispositivo) = if boot {
                (
                    interfaz.protocolo,
                    if interfaz.protocolo == PROTOCOLO_TECLADO {
                        TipoInforme::Teclado
                    } else {
                        TipoInforme::Raton
                    },
                    Dispositivo::VACIO,
                )
            } else {
                let dispositivo = match informe::analizar(descriptor) {
                    Ok(d) => d,
                    Err(e) => {
                        descartar(puerto, interfaz, e.motivo());
                        continue;
                    }
                };
                if dispositivo.tipo == TipoInforme::Desconocido {
                    descartar(
                        puerto,
                        interfaz,
                        "el descriptor de informe no es teclado ni raton",
                    );
                    continue;
                }
                (0, dispositivo.tipo, dispositivo)
            };

            // `SET_IDLE` no es obligatorio en un HID generico: si lo rechaza,
            // se sigue (el dispositivo manda los informes cuando cambian).
            let setup = setup_set_idle(interfaz.numero, 0, 0);
            if self
                .control_ep0(hueco, anillo_ep0, setup, buffer_ep0)
                .is_err()
                && boot
            {
                descartar(puerto, interfaz, "no acepta SET_IDLE(0)");
                continue;
            }

            // 5. El endpoint: `Configure Endpoint` con el contexto de slot y el
            //    del endpoint nuevo. El bit del endpoint 0 **no** se pone: si se
            //    pusiera, el controlador volveria a direccionar el dispositivo
            //    (§4.6.6).
            let dci = ep_id(true, endpoint.numero);
            // El resto de interfaces del mismo dispositivo se quedan sin usar;
            // se dice cual y por que, en vez de callarse.
            let Some(indice_ep) = self.hueco_interrupcion_libre() else {
                descartar(
                    puerto,
                    interfaz,
                    "no queda sitio para otro endpoint de interrupcion",
                );
                break;
            };
            // La region se valido al entrar y el indice sale de la tabla: estos
            // fallos no pueden ocurrir. Si ocurrieran, el endpoint no se
            // programa y el bucle no sigue inventando.
            let (Some(base_anillo), Some(buffer)) = (
                self.interrupciones.anillo(indice_ep),
                self.interrupciones.buffer(indice_ep),
            ) else {
                break;
            };
            ring::borrar(base_anillo, ring::TAM_ANILLO_INTERRUPCION);
            ring::borrar(buffer, ring::TAM_BUFFER_INTERRUPCION);
            // Idem: el anillo se construye sobre una region ya validada.
            let Some(anillo) = AnilloTransferencia::nuevo(base_anillo, ring::TRB_INTERRUPCION)
            else {
                break;
            };
            // El Link se escribe con el ciclo de la vuelta que empieza: el
            // productor empieza en uno, como en el endpoint 0.
            let link = trb_link(base_anillo, true);
            ring::escribir_trb(anillo.direccion_link(), &link);

            let mut control = InputControlContext::default();
            control.set_drop_flags(0);
            control.set_add_flags(AÑADIR_SLOT | (1 << dci));

            let mut slot = SlotContext::default();
            slot.set_route_speed(
                topologia.ruta,
                velocidad_contexto(velocidad),
                false,
                topologia.mtt,
            );
            if topologia.necesita_tt(velocidad) {
                slot.set_tt_info(topologia.hub_padre, topologia.puerto_hub, topologia.ttt);
            }
            slot.set_context_entries(dci);
            slot.set_root_hub_port(puerto);
            slot.set_max_exit_latency(0);
            slot.set_interrupter_target(0);

            let mut ep = EndpointContext::default();
            ep.set_state_mult_pstreams_lsa_interval(
                EP_ESTADO_INVALIDO,
                0,
                0,
                false,
                intervalo_interrupcion(velocidad, endpoint.intervalo),
            );
            ep.set_type_err_hid(EP_TINT_IN, 3, true);
            ep.set_packet_burst(endpoint.max_paquete, 1);
            ep.set_tr_dequeue(base_anillo, true);
            ep.set_avg_trb_len(endpoint.max_paquete);
            ep.set_max_esit(0);

            escribir_contexto(entrada, &control.d);
            escribir_contexto(entrada + 32, &slot.d);
            escribir_contexto(entrada + 32 * (dci as u64 + 1), &ep.d);
            mmio::clflush_rango(entrada, ring::TAM_CTX_ENTRADA);
            mmio::clflush_rango(base_anillo, ring::TAM_ANILLO_INTERRUPCION);
            mmio::mfence();

            let evento = match self.ejecutar_comando(trb_configure_endpoint(entrada, hueco, false))
            {
                Ok(evento) => evento,
                Err(_) => {
                    self.contadores.errores_hid += 1;
                    descartar(puerto, interfaz, "el comando Configure Endpoint fallo");
                    continue;
                }
            };
            if evento.codigo_finalizacion() != FINALIZACION_CORRECTA {
                self.contadores.errores_hid += 1;
                crate::println!(
                    "usb: HID del puerto {}: interfaz {} sin usar (Configure Endpoint acabo con codigo {}).",
                    puerto,
                    interfaz.numero,
                    evento.codigo_finalizacion()
                );
                continue;
            }

            // 6. El endpoint ya existe. Se guarda y se deja la primera
            //    transferencia preparada.
            self.ep_activos[indice_ep] = EpHid {
                hueco,
                dci,
                numero: endpoint.numero,
                interfaz: interfaz.numero,
                protocolo,
                tipo_generico: tipo,
                informe: dispositivo,
                max_paquete: endpoint.max_paquete,
                largo_reporte,
                anillo,
                buffer,
                pendiente: false,
                parado: false,
                anteriores: InformeTeclado {
                    modificadores: 0,
                    teclas: [0; 6],
                },
                informes: 0,
                errores: 0,
            };
            self.num_ep_activos += 1;
            self.armar_interrupcion(indice_ep);
            activos += 1;

            let ep = &self.ep_activos[indice_ep];
            crate::println!(
                "usb: HID en el puerto {}: {}, interfaz {}, endpoint {} IN, informe de {} bytes, descriptor de {} bytes.",
                puerto,
                ep.nombre_protocolo(),
                ep.interfaz,
                ep.numero,
                ep.max_paquete,
                ep.largo_reporte
            );
        }
        activos
    }

    /// El primer hueco libre de la tabla de endpoints de interrupcion.
    ///
    /// Los endpoints se guardan compactos: los huecos no se reservan por
    /// indice, se busca el primero que no este en uso.
    fn hueco_interrupcion_libre(&self) -> Option<usize> {
        if self.num_ep_activos as usize >= ring::MAX_EP_INTERRUPCION {
            return None;
        }
        Some(self.num_ep_activos as usize)
    }

    /// Prepara la siguiente transferencia de un endpoint de interrupcion.
    ///
    /// Solo se llama cuando la anterior ya ha terminado (`pendiente` a cero):
    /// el controlador solo admite una transferencia en vuelo por endpoint.
    fn armar_interrupcion(&mut self, indice: usize) {
        if indice >= self.num_ep_activos as usize {
            return;
        }
        let (hueco, dci) = {
            let ep = &mut self.ep_activos[indice];
            if ep.pendiente || ep.parado {
                return;
            }
            // Un TRB Normal de un solo paquete: la longitud es la del informe
            // y el evento lo genera el propio TRB (`IOC`). El bit `ISP` no hace
            // falta en un `Normal`. El destino de interrupcion es el 0.
            let (direccion, ciclo) = ep.anillo.siguiente();
            let mut trb = trb_normal(ep.buffer, ep.max_paquete as u32, 0, false, true);
            trb.set_ciclo(ciclo);
            ring::escribir_trb(direccion, &trb);
            // Si la reserva ha dado la vuelta, el Link hay que reescribirlo con
            // el ciclo que el controlador va a comparar.
            if ep.anillo.dio_la_vuelta {
                let mut link = trb_link(ep.anillo.base, true);
                link.set_ciclo(!ep.anillo.ciclo);
                ring::escribir_trb(ep.anillo.direccion_link(), &link);
            }
            // El buffer tiene que estar limpio: el controlador escribe en
            // memoria y el procesador no puede quedarse con una copia vieja.
            mmio::clflush_rango(ep.buffer, ep.max_paquete as usize);
            mmio::clflush_rango(direccion, 16);
            mmio::mfence();
            ep.pendiente = true;
            (ep.hueco, ep.dci)
        };

        if self.regs.escribir_doorbell(hueco, dci as u32).is_err() {
            self.ep_activos[indice].pendiente = false;
            self.contadores.errores_hid += 1;
        }
    }

    /// Repone la transferencia de todos los endpoints que ya terminaron.
    ///
    /// Se hace **despues** de drenar el anillo de eventos: tocar la campana
    /// dentro del drenaje volveria a encolar eventos que la ronda actual
    /// todavia no ha leido.
    pub(super) fn rearmar_interrupciones(&mut self) {
        for indice in 0..self.num_ep_activos as usize {
            if !self.ep_activos[indice].pendiente {
                self.armar_interrupcion(indice);
            }
        }
    }

    /// Atiende un evento de transferencia de un endpoint de interrupcion.
    ///
    /// Devuelve `true` si el evento era de un endpoint de esta tabla. El evento
    /// solo deja los datos en las colas de entrada: aqui no se dibuja ni se
    /// escribe en la consola.
    pub(super) fn atender_transferencia(&mut self, trb: &Trb) -> bool {
        let hueco = trb.slot_id();
        let dci = trb.ep_id();
        let Some(indice) = self.ep_activos[..self.num_ep_activos as usize]
            .iter()
            .position(|e| e.hueco == hueco && e.dci == dci)
        else {
            return false;
        };

        let codigo = trb.codigo_finalizacion();
        self.ep_activos[indice].pendiente = false;
        if codigo != FINALIZACION_CORRECTA && codigo != FINALIZACION_PAQUETE_CORTO {
            self.ep_activos[indice].errores += 1;
            self.contadores.errores_hid += 1;
            if codigo == FINALIZACION_ERROR_STALL {
                // Un endpoint parado no se repone solo: se queda sin informes
                // hasta que el dispositivo se reconecte. No se intenta
                // `Reset Endpoint`: esta fase no sabe recuperar un HID parado.
                // Lo que tuviera pulsado se suelta: ya no habra informe que lo
                // libere.
                self.ep_activos[indice].parado = true;
                soltar_dispositivo(&mut self.ep_activos[indice]);
            }
            return true;
        }

        // El evento dice cuantos bytes **faltaron**; lo que llego es la
        // diferencia con lo pedido. Un residual mayor que lo pedido solo puede
        // venir de un evento corrupto: se descarta.
        let pedido = self.ep_activos[indice].max_paquete as u32;
        let residual = trb.status() & 0x00FF_FFFF;
        if residual > pedido {
            self.ep_activos[indice].errores += 1;
            self.contadores.errores_hid += 1;
            return true;
        }
        let leidos = pedido - residual;
        if leidos == 0 {
            return true;
        }

        let (buffer, protocolo) = {
            let ep = &self.ep_activos[indice];
            (ep.buffer, ep.protocolo)
        };
        // El controlador ha escrito en el buffer: se tira la linea de cache
        // antes de leerla.
        mmio::clflush_rango(buffer, leidos as usize);
        mmio::mfence();
        // Seguro: `leidos` no pasa de `max_paquete`, que cabe en el buffer
        // reservado para este endpoint.
        let datos = unsafe { core::slice::from_raw_parts(buffer as *const u8, leidos as usize) };

        match protocolo {
            PROTOCOLO_TECLADO => {
                self.contadores.informes_hid += 1;
                procesar_teclado(&mut self.ep_activos[indice], datos);
            }
            PROTOCOLO_RATON => {
                self.contadores.informes_hid += 1;
                procesar_raton(&mut self.ep_activos[indice], datos);
            }
            _ => {
                // HID leido por su descriptor de informe. Un informe de otro
                // `Report ID` (teclas de consumo, por ejemplo) se ignora sin
                // contarlo como error.
                let (suyo, tipo) = {
                    let ep = &self.ep_activos[indice];
                    (ep.informe.es_suyo(datos), ep.tipo_generico)
                };
                if suyo {
                    if tipo == TipoInforme::Desconocido {
                        self.ep_activos[indice].errores += 1;
                        self.contadores.errores_hid += 1;
                    } else {
                        self.contadores.informes_hid += 1;
                        procesar_generico(&mut self.ep_activos[indice], datos);
                    }
                }
            }
        }
        true
    }

    /// Olvida los endpoints de interrupcion de un hueco.
    ///
    /// Se llama cuando el dispositivo se desconecta o cuando su hueco se va a
    /// reutilizar: los TRB del anterior no pueden quedar en el anillo del
    /// siguiente.
    pub(super) fn olvidar_hid(&mut self, hueco: u8) {
        let mut i = 0;
        while i < self.num_ep_activos as usize {
            if self.ep_activos[i].hueco == hueco {
                // El dispositivo se va: hay que soltar lo que tuviera pulsado
                // antes de perder su ultimo informe.
                soltar_dispositivo(&mut self.ep_activos[i]);
                self.num_ep_activos -= 1;
                let ultimo = self.num_ep_activos as usize;
                self.ep_activos[i] = self.ep_activos[ultimo];
                self.ep_activos[ultimo] = EpHid::VACIO;
            } else {
                i += 1;
            }
        }
    }

    /// Cuantos informes lleva un dispositivo y cuantos HID tiene activos.
    pub(super) fn informes_de(&self, hueco: u8) -> (u8, u32) {
        let mut hid = 0;
        let mut informes = 0;
        for ep in &self.ep_activos[..self.num_ep_activos as usize] {
            if ep.hueco == hueco {
                hid += 1;
                informes += ep.informes;
            }
        }
        (hid, informes)
    }
}

/// Deja constancia de por que una interfaz HID se queda sin usar.
///
/// Un HID que no se puede leer como boot no es un error del dispositivo: se
/// avisa y se sigue. Lo que no se hace nunca es tratarlo como boot igualmente.
fn descartar(puerto: u8, interfaz: &Interfaz, motivo: &str) {
    crate::println!(
        "usb: HID del puerto {}: interfaz {} sin usar ({}).",
        puerto,
        interfaz.numero,
        motivo
    );
}

/// El ajuste alterno que se debe usar para una interfaz HID.
///
/// Se prefiere el 0, que es el que deja activo `SET_CONFIGURATION`; si el 0 no
/// sirve se usa el de menor numero. Dentro de un mismo numero de interfaz se
/// prefiere un ajuste **boot** (menos sorpresas y ya probado) y, si no lo hay,
/// el primero que sea HID para leerlo por su descriptor de informe.
#[derive(Clone, Copy)]
struct AjusteHid {
    alterna: u8,
    boot: bool,
}

fn ajuste_elegido(interfaces: &[Interfaz], numero: u8) -> Option<AjusteHid> {
    let mut boot_alt: Option<u8> = None;
    let mut generico_alt: Option<u8> = None;
    for i in interfaces
        .iter()
        .filter(|i| i.es_hid() && i.numero == numero)
    {
        let boot = i.subclase == SUBCLASE_BOOT
            && (i.protocolo == PROTOCOLO_TECLADO || i.protocolo == PROTOCOLO_RATON);
        if boot {
            if i.alterna == 0 {
                return Some(AjusteHid {
                    alterna: 0,
                    boot: true,
                });
            }
            boot_alt = Some(boot_alt.map_or(i.alterna, |a| a.min(i.alterna)));
        } else {
            generico_alt = Some(generico_alt.map_or(i.alterna, |a| a.min(i.alterna)));
        }
    }
    if let Some(alterna) = boot_alt {
        return Some(AjusteHid {
            alterna,
            boot: true,
        });
    }
    generico_alt.map(|alterna| AjusteHid {
        alterna,
        boot: false,
    })
}

/// Suelta lo que un dispositivo tuviera pulsado.
///
/// Un HID que se desconecta (o cuyo endpoint se para) no manda un informe con
/// todo a cero: si se va con Shift/Ctrl/Alt abajo, el consumidor se queda con
/// el modificador pegado y ya no hay informe que lo libere. Se reutiliza el
/// camino de un informe vacio, el mismo que el de un `ErrorRollOver`.
fn soltar_dispositivo(ep: &mut EpHid) {
    match (ep.protocolo, ep.tipo_generico) {
        (PROTOCOLO_TECLADO, _) => procesar_teclado(ep, &[0u8; 8]),
        (PROTOCOLO_RATON, _) => crate::mouse::soltar_usb(),
        (_, TipoInforme::Teclado) => procesar_teclado(ep, &[0u8; 8]),
        (_, TipoInforme::Raton) => crate::mouse::soltar_usb(),
        _ => {}
    }
}

/// Convierte un informe de teclado en eventos neutrales y los encola.
fn procesar_teclado(ep: &mut EpHid, datos: &[u8]) {
    let Some(actual) = InformeTeclado::leer(datos) else {
        ep.errores += 1;
        return;
    };
    publicar_teclado(ep, actual);
}

/// Encola la diferencia entre el informe anterior y `actual`.
fn publicar_teclado(ep: &mut EpHid, actual: InformeTeclado) {
    let mut eventos = [crate::input::Evento::VACIO; MAX_EVENTOS];
    let mut n = 0usize;
    teclado::eventos(&ep.anteriores, &actual, &mut |uso, pulsada| {
        let Some(barrido) = teclado::uso_a_barrido(uso) else {
            return;
        };
        if n == eventos.len() {
            return;
        }
        eventos[n] = crate::input::Evento {
            codigo: barrido.codigo,
            extendido: barrido.extendido,
            pulsada,
        };
        n += 1;
    });

    ep.anteriores = actual;
    ep.informes = ep.informes.saturating_add(1);
    if n > 0 {
        crate::input::publicar_hid(&eventos[..n]);
    }
}

/// Interpreta un informe con el descriptor de la fase 6.
fn procesar_generico(ep: &mut EpHid, datos: &[u8]) {
    match ep.tipo_generico {
        TipoInforme::Teclado => {
            let Some(actual) = ep.informe.teclado(datos) else {
                ep.errores += 1;
                return;
            };
            publicar_teclado(ep, actual);
        }
        TipoInforme::Raton => {
            let Some(actual) = ep.informe.raton(datos) else {
                ep.errores += 1;
                return;
            };
            ep.informes = ep.informes.saturating_add(1);
            crate::mouse::alimentar_usb(actual.dx as i32, actual.dy as i32, actual.botones);
        }
        TipoInforme::Desconocido => ep.errores += 1,
    }
}

/// Aplica un informe de raton al cursor.
fn procesar_raton(ep: &mut EpHid, datos: &[u8]) {
    let Some(actual) = InformeRaton::leer(datos) else {
        ep.errores += 1;
        return;
    };
    ep.informes = ep.informes.saturating_add(1);
    crate::mouse::alimentar_usb(actual.dx as i32, actual.dy as i32, actual.botones);
}
