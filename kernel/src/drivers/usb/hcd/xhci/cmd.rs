//! Envio de comandos al controlador de host xHCI.
//!
//! ## Como funciona un comando
//!
//! Un comando xHCI no es un registro: es un TRB que el software escribe en el
//! **anillo de comandos** y luego avisa al controlador tocando la **campana
//! del controlador de host** (`doorbell[0]`). El controlador lo ejecuta y
//! deja un **evento de fin de comando** en el anillo de eventos, con el
//! resultado (§4.6.4, §6.4.2.2).
//!
//! El camino completo es:
//!
//! 1. Se reserva el siguiente TRB del anillo de comandos.
//! 2. Se escribe el TRB con el bit de ciclo del productor.
//! 3. Se escribe la barrera de memoria (`mfence`): hasta que el TRB no esta
//!    en memoria, el controlador no puede leerlo.
//! 4. Se toca la campana del controlador de host.
//! 5. Se sondea el anillo de eventos hasta que llega el evento de fin de
//!    comando, con un **plazo** y un **limite de iteraciones**.
//!
//! ## El valor de la campana del controlador de host
//!
//! La campana del controlador de host se escribe con **cero**, y no con el
//! indice del proximo TRB. No es un descuido:
//!
//! - Linux escribe `DB_VALUE_HOST`, que vale `0x00000000`
//!   (`xhci_ring_cmd_db` en `drivers/usb/host/xhci-ring.c`).
//! - QEMU **rechaza** cualquier otro valor: en `xhci_doorbell_write` solo
//!   procesa comandos si el valor es cero, y si no, avisa por el log.
//! - El controlador real no usa el valor: mantiene su propio puntero de
//!   extraccion en el registro `CRCR` (§5.4.5).
//!
//! La campana es, en el caso del controlador de host, un simple aviso: "hay
//! TRB nuevos en el anillo".
//!
//! ## Plazos
//!
//! Ninguna espera es infinita: todas tienen un plazo en tic del PIT (100 tic
//! = 1 segundo) y un limite de iteraciones. Un controlador que no responde
//! deja el dispositivo sin enumerar, pero no cuelga el arranque.
//!
//! Codigo propio de rocky.

use super::ring;
use super::{Controlador, ErrorXhci};
use crate::drivers::usb::mmio;
use crate::pit;
use crate::tarea;
use rockyhw::xhci::regs::{INT_ERDP, RT_INTERRUPTER};
use rockyhw::xhci::ring::{TipoEvento, Trb, ERDP_EHB, FINALIZACION_CORRECTA};

/// Valor de la campana del controlador de host.
///
/// Cero, por lo que explican el comentario de este modulo.
pub const VALOR_CAMPANA_CONTROLADOR: u32 = 0;

/// Plazo por defecto para esperar un evento de fin de comando, en tic del PIT
/// (100 tic = 1 segundo).
pub const PLAZO_COMANDO: u64 = 100;

/// Limite de iteraciones por defecto del sondeo del anillo de eventos.
///
/// Cada iteracion es una lectura del anillo, no una espera: el plazo es lo
/// que de verdad acota el tiempo. Este limite evita que un anillo con datos
/// basura haga que el bucle se quede dentro del sondeo para siempre.
pub const MAX_ITERACIONES: usize = 1000;

impl Controlador {
    /// Encola un comando y toca la campana del controlador de host.
    ///
    /// El TRB se escribe con el bit de ciclo del productor: el controlador
    /// compara ese bit con su propio estado de ciclo para saber si el TRB es
    /// nuevo o es el hueco que dejo la vez anterior (§4.9.4).
    ///
    /// No espera el resultado: eso lo hace [`Controlador::esperar_comando`].
    pub fn enviar_comando(&mut self, mut trb: Trb) -> Result<(), ErrorXhci> {
        let (direccion, ciclo) = self.cmd_ring.siguiente();
        trb.set_ciclo(ciclo);
        ring::escribir_trb(direccion, &trb);
        // El TRB tiene que estar en memoria antes de que el controlador lo
        // pueda leer: sin esta barrera, el controlador podria ver un TRB a
        // medias. En x86 la escritura puede quedarse en la cache del
        // procesador, asi que ademas se tira la linea antes de la campana.
        mmio::clflush_rango(direccion, 16);
        mmio::mfence();
        self.regs
            .escribir_doorbell(0, VALOR_CAMPANA_CONTROLADOR)
            .map_err(|_| ErrorXhci::VentanaInvalida)?;
        Ok(())
    }

    /// Espera a que llegue el evento de fin de un comando.
    ///
    /// Sondea el anillo de eventos y se queda con el primer evento de fin de
    /// comando que encuentra. Los demas eventos que aparezcan mientras espera
    /// (cambios de puerto, por ejemplo) se cuentan y se descartan: el estado
    /// de los puertos se lee de sus registros, no de los eventos.
    ///
    /// El plazo esta en tic del PIT (100 tic = 1 segundo). Si se agota, o si
    /// se supera el limite de iteraciones, devuelve `SinRespuesta`.
    pub fn esperar_comando(
        &mut self,
        plazo: u64,
        max_iteraciones: usize,
    ) -> Result<Trb, ErrorXhci> {
        let memoria = self.memoria_eventos();
        let inicio = pit::ticks();
        let mut iteracion = 0usize;
        loop {
            iteracion += 1;
            match self.anillo.siguiente(&memoria) {
                Some(trb) => {
                    self.anillo.avanzar();
                    self.contadores.eventos += 1;
                    // Devolverle el puntero de extraccion al controlador, con
                    // `EHB` para que sepa que el software ya ha leido lo que
                    // habia (§5.5.2.3.3).
                    self.devolver_puntero_eventos();
                    if TipoEvento::de_codigo(trb.tipo()) == TipoEvento::ComandoCompletado {
                        return Ok(trb);
                    }
                    // Un evento que no es de comando: se cuenta y se sigue.
                    self.contadores.desconocidos += 1;
                }
                None => {
                    if iteracion >= max_iteraciones {
                        return Err(ErrorXhci::SinRespuesta);
                    }
                    if pit::ticks().saturating_sub(inicio) >= plazo {
                        return Err(ErrorXhci::SinRespuesta);
                    }
                    tarea::dormir(1);
                }
            }
        }
    }

    /// Encola un comando y espera a que termine.
    ///
    /// Devuelve el evento de fin de comando, con su codigo de finalizacion.
    pub fn ejecutar_comando(&mut self, trb: Trb) -> Result<Trb, ErrorXhci> {
        self.enviar_comando(trb)?;
        self.esperar_comando(PLAZO_COMANDO, MAX_ITERACIONES)
    }

    /// Habilita un hueco de dispositivo (comando Enable Slot, §4.6.4).
    ///
    /// Devuelve el numero de hueco que le ha dado el controlador.
    pub fn habilitar_hueco(&mut self) -> Result<u8, ErrorXhci> {
        let evento = self.ejecutar_comando(rockyhw::xhci::ring::trb_enable_slot(0))?;
        if evento.codigo_finalizacion() != FINALIZACION_CORRECTA {
            self.contadores.errores += 1;
            return Err(ErrorXhci::SinRespuesta);
        }
        Ok(evento.slot_id())
    }

    /// Libera un hueco de dispositivo (comando Disable Slot, §4.6.4).
    ///
    /// El comando no devuelve nada: el controlador lo ejecuta y lo dice con un
    /// evento de fin de comando, pero sin informacion util.
    pub fn deshabilitar_hueco(&mut self, hueco: u8) -> Result<(), ErrorXhci> {
        let evento = self.ejecutar_comando(rockyhw::xhci::ring::trb_disable_slot(hueco))?;
        if evento.codigo_finalizacion() != FINALIZACION_CORRECTA {
            self.contadores.errores += 1;
            return Err(ErrorXhci::SinRespuesta);
        }
        Ok(())
    }

    /// Le devuelve al controlador el puntero de extraccion del anillo de
    /// eventos.
    ///
    /// Se escribe `EHB` para limpiar la bandera de "manejador ocupado": si
    /// se queda a uno, el controlador cree que el software no ha terminado
    /// con el anillo y deja de producir eventos (§5.5.2.3.3).
    pub(crate) fn devolver_puntero_eventos(&mut self) {
        let _ = self
            .regs
            .escribir64_rt(RT_INTERRUPTER + INT_ERDP, self.anillo.erdp() | ERDP_EHB);
    }
}
