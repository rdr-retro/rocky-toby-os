//! Los puertos raiz de un controlador xHCI.
//!
//! Un puerto xHCI declara su protocolo (USB 2.0 o USB 3.x) en las capacidades
//! extendidas y tiene su propia configuracion. Algunos detalles importantes:
//!
//! - Un puerto **USB 2.0** necesita un **reinicio por software** (`PR`) para que
//!   el campo de velocidad sea valido (§4.19.7). El reinicio se hace una sola
//!   vez.
//! - Un puerto **USB 3.x** suele tener la velocidad valida tras la conexion, pero
//!   hay que tener paciencia: en algunos casos tarda unos milisegundos.
//! - Los bits de cambio (`CSC`, `PRC`, ...) se limpian escribiendo un 1.
//! - Los hubs se sondean despues de los puertos raiz, un puerto por ronda. La
//!   vuelta se repite para detectar conexiones y desconexiones en caliente.
//!
//! Codigo propio de rocky.

use crate::drivers::usb::core::hub;
use crate::drivers::usb::core::transfer::{
    setup_hub_clear_port_feature, setup_hub_get_port_status, setup_hub_set_port_feature,
};
use crate::pit;
use crate::tarea;
use rockyhw::xhci::caps::Protocolo;
use rockyhw::xhci::port::{EstadoPuerto, Velocidad};
use rockyhw::xhci::regs::{PORTSC_CAMBIO, PORTSC_ESCRIBIBLES, PORTSC_PR, PORTSC_PRC};
use rockyhw::xhci::ring::AnilloTransferencia;

use super::{enumeracion, Controlador, PLAZO_PUERTO};

/// En que punto esta la enumeracion de un puerto.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EstadoEnum {
    /// Hay un dispositivo conectado y todavia no se ha intentado enumerar.
    Pendiente,
    /// Se esta enumerando ahora mismo.
    EnCurso,
    /// El dispositivo se enumero.
    Hecho,
    /// Se intento y no se pudo. No se reintenta solo: un dispositivo roto no
    /// puede gastar una ronda de sondeo detras de otra.
    Fallido,
}

/// El estado de un puerto raiz, tal y como lo lleva el driver.
#[derive(Clone, Copy)]
pub struct Puerto {
    /// Numero de puerto (desde 1).
    pub numero: u8,
    /// El protocolo que declara el controlador.
    pub protocolo: Option<Protocolo>,
    /// Lo que dice el registro de estado del puerto.
    pub estado: EstadoPuerto,
    /// Si ya se ha hecho el reinicio por software (un puerto USB 2.0 solo lo
    /// necesita una vez).
    pub reiniciado: bool,
    /// En que punto esta la enumeracion del dispositivo del puerto.
    pub enumeracion: EstadoEnum,
}

impl Puerto {
    /// Un puerto que todavia no se ha leido.
    pub fn vacio(numero: u8, protocolo: Option<Protocolo>) -> Puerto {
        Puerto {
            numero,
            protocolo,
            estado: EstadoPuerto::decodificar(0),
            reiniciado: false,
            enumeracion: EstadoEnum::Pendiente,
        }
    }
}

impl Controlador {
    /// Prepara la lista de puertos raiz a partir de las capacidades.
    pub fn preparar_puertos(&mut self) {
        let caps = self.regs.capacidades();
        let mut numero = 0u8;
        let mut contador = 0usize;
        // Recorrer los puertos en orden numérico (1..=MaxPorts). Las capacidades
        // dicen qué protocolo usa cada rango de puertos.
        for i in 1..=(caps.max_ports as u8) {
            if contador >= self.puertos.len() {
                break;
            }
            let protocolo = caps.protocolos.protocolo_de(i);
            self.puertos[contador] = Puerto::vacio(i, protocolo);
            contador += 1;
            numero = i;
        }
        self.num_puertos = contador as u8;
        let _ = numero;
    }

    /// Refresca todos los puertos raiz.
    ///
    /// Para un puerto USB 2.0 conectado que nunca se ha reiniciado, se hace el
    /// reinicio por software una sola vez: sin él, el campo de velocidad no es
    /// válido (§4.19.7). Para un puerto USB 3.x, se espera un poco si la
    /// velocidad aún es indefinida.
    pub fn refrescar_puertos(&mut self) {
        let num = self.num_puertos as usize;
        for i in 0..num {
            self.refrescar_puerto(i);
        }
    }

    /// Escanea los puertos raiz por primera vez tras arrancar el controlador.
    pub fn escanear_puertos(&mut self) {
        self.refrescar_puertos();
    }

    /// Refresca un puerto.
    fn refrescar_puerto(&mut self, indice: usize) {
        let puerto = self.puertos[indice];
        let num = puerto.numero;
        let por = match self.regs.leer_puerto(num) {
            Ok(v) => v,
            Err(_) => {
                self.contadores.errores += 1;
                return;
            }
        };
        let mut estado = EstadoPuerto::decodificar(por);

        // Un puerto USB 2.0 necesita un reinicio por software para que la
        // velocidad sea válida.
        if estado.conectado && !puerto.reiniciado {
            if matches!(puerto.protocolo, Some(Protocolo::Usb2)) {
                if self.reiniciar_puerto(num) {
                    self.puertos[indice].reiniciado = true;
                    if let Ok(v2) = self.regs.leer_puerto(num) {
                        estado = EstadoPuerto::decodificar(v2);
                    }
                } else {
                    // Aunque falle, marcamos como "intentado" para no bloquear el
                    // bucle de sondeo con reintentos interminables.
                    self.puertos[indice].reiniciado = true;
                    self.contadores.errores += 1;
                }
            } else if matches!(puerto.protocolo, Some(Protocolo::Usb3)) {
                // Para USB 3.x, a veces la velocidad tarda en aparecer.
                if estado.velocidad == Velocidad::Indefinida {
                    let inicio = pit::ticks();
                    while pit::ticks().saturating_sub(inicio) < 20 {
                        tarea::dormir(1);
                        if let Ok(v2) = self.regs.leer_puerto(num) {
                            let e2 = EstadoPuerto::decodificar(v2);
                            if e2.velocidad != Velocidad::Indefinida {
                                estado = e2;
                                break;
                            }
                        }
                    }
                    self.puertos[indice].reiniciado = true;
                } else {
                    self.puertos[indice].reiniciado = true;
                }
            }
        }

        // Si el dispositivo se ha ido, se devuelve su hueco y se olvida: un
        // hueco que no se libera no se puede volver a usar, y el controlador
        // solo tiene unos pocos.
        if !estado.conectado && puerto.enumeracion != EstadoEnum::Pendiente {
            if puerto.enumeracion == EstadoEnum::Hecho {
                self.olvidar_puerto(num);
            }
            // Al volver a conectar puede ser otro dispositivo y, en USB 2.0, la
            // velocidad solo es fiable despues de un reinicio de puerto.
            self.puertos[indice].reiniciado = false;
            self.puertos[indice].enumeracion = EstadoEnum::Pendiente;
        }

        self.puertos[indice].estado = estado;
    }

    /// Enumera el dispositivo de un puerto pendiente, si hay alguno.
    ///
    /// Se hace **uno por ronda**: una enumeracion son dos comandos y tres
    /// transferencias de control, y el bucle de sondeo no puede quedarse ahi el
    /// resto de la ronda. Los demas puertos esperan a la ronda siguiente.
    ///
    /// Si no hay ningun puerto raiz pendiente, se sondea **un** puerto de un
    /// hub ya enumerado. Asi los dispositivos que cuelgan de un hub no bloquean
    /// la atencion de los eventos HID.
    pub fn enumerar_pendientes(&mut self) {
        // Sin sitio en la lista de dispositivos no se puede guardar nada: se
        // deja el puerto pendiente antes que enumerar y perder el resultado.
        if self.num_dispositivos as usize >= self.dispositivos.len() {
            return;
        }

        // El primer puerto conectado, habilitado y pendiente, en orden.
        let mut elegido: Option<(usize, u8, Velocidad)> = None;
        for i in 0..self.num_puertos as usize {
            let p = self.puertos[i];
            if !p.estado.conectado || !p.estado.habilitado {
                continue;
            }
            if p.enumeracion != EstadoEnum::Pendiente {
                continue;
            }
            elegido = Some((i, p.numero, p.estado.velocidad));
            break;
        }
        let Some((indice, numero, velocidad)) = elegido else {
            self.sondear_hub();
            return;
        };

        self.puertos[indice].enumeracion = EstadoEnum::EnCurso;
        match self.enumerar_puerto(numero, velocidad) {
            Ok(dispositivo) => {
                self.dispositivos[self.num_dispositivos as usize] = dispositivo;
                self.num_dispositivos += 1;
                self.contadores.dispositivos += 1;
                self.puertos[indice].enumeracion = EstadoEnum::Hecho;
                let (vend, prod, clase) = match &dispositivo.descriptor {
                    Some(d) => (d.id_vendor, d.id_product, d.b_device_class),
                    None => (0, 0, 0),
                };
                crate::println!(
                    "usb: dispositivo en el puerto {}: {:04X}:{:04X} clase {}, {} interfaces ({} HID), {} endpoints ({} de interrupcion), {}, configuracion {}.",
                    numero,
                    vend,
                    prod,
                    clase,
                    dispositivo.num_interfaces,
                    dispositivo.interfaces_hid(),
                    dispositivo.num_endpoints,
                    dispositivo.interrupciones_de_entrada(),
                    velocidad.nombre(),
                    dispositivo.config_actual
                );
            }
            Err(e) => {
                self.contadores.errores_enum += 1;
                self.puertos[indice].enumeracion = EstadoEnum::Fallido;
                match e.codigo() {
                    Some(codigo) => crate::println!(
                        "usb: el puerto {} no se pudo enumerar: {} (codigo {}).",
                        numero,
                        e.nombre(),
                        codigo
                    ),
                    None => crate::println!(
                        "usb: el puerto {} no se pudo enumerar: {}.",
                        numero,
                        e.nombre()
                    ),
                }
            }
        }
    }

    /// Sondea un puerto de un hub ya enumerado.
    ///
    /// Se hace un puerto por ronda y, al terminar la vuelta, se empieza otra:
    /// asi se detectan tambien los dispositivos que se enchufan o se quitan
    /// despues del arranque. El estado de cada puerto se guarda en el propio
    /// hub (`hub_estado`) y los hijos se buscan por `(hueco del hub, puerto)`.
    fn sondear_hub(&mut self) {
        let Some(dispositivo) = self.hub_pendiente() else {
            return;
        };
        let hueco = dispositivo.hueco;
        let puerto_hub = dispositivo.hub_sig;
        let mut anillo = dispositivo.anillo_ep0;
        let buffer = self.dma.buffers.base() as u64;
        if self.dma.buffers.bytes() < 8 {
            return;
        }

        let Some(mut estado) =
            self.leer_estado_puerto_hub(hueco, &mut anillo, puerto_hub, buffer)
        else {
            self.terminar_puerto_hub(hueco, 3, anillo);
            return;
        };

        // El cambio de conexion se limpia antes de tocar el puerto, pero se
        // recuerda: un puerto que acaba de cambiar puede necesitar que se
        // vuelva a enumerar aunque ya hubiera un hijo.
        let cambio_conexion = estado.cambio_conexion;
        if cambio_conexion {
            let _ = self.control_ep0(
                hueco,
                &mut anillo,
                setup_hub_clear_port_feature(
                    puerto_hub,
                    hub::CARACTERISTICA_C_PORT_CONNECTION,
                ),
                buffer,
            );
        }

        // Un hijo que ya no esta, o que se ha vuelto a conectar, deja de ser
        // valido: se devuelve su hueco antes de decidir.
        if !estado.con_dispositivo() || cambio_conexion {
            self.olvidar_hijo_de_hub(hueco, puerto_hub);
        }

        if !estado.con_dispositivo() {
            self.terminar_puerto_hub(hueco, 1, anillo);
            return;
        }

        // Si ya hay un hijo vivo y el puerto sigue habilitado, no se vuelve a
        // enumerar en cada vuelta: solo se refresca su estado.
        if self.hijo_de_hub(hueco, puerto_hub).is_some() && estado.habilitado {
            self.terminar_puerto_hub(hueco, 2, anillo);
            return;
        }

        // Un puerto que ya fallo y no ha cambiado no se reintenta en cada
        // vuelta; se espera a que el hub avise de un cambio.
        if !cambio_conexion
            && puerto_hub != 0
            && (puerto_hub as usize) <= dispositivo.hub_estado.len()
            && dispositivo.hub_estado[puerto_hub as usize - 1] == 3
        {
            self.terminar_puerto_hub(hueco, 3, anillo);
            return;
        }

        // Tras una conexion nueva el puerto hay que reiniciarlo aunque el hub
        // diga que ya esta habilitado: el dispositivo es otro.
        let mut necesita_reset = cambio_conexion;
        if !estado.habilitado || necesita_reset {
            if !self.resetear_puerto_hub(hueco, &mut anillo, puerto_hub, buffer) {
                crate::println!(
                    "usb: hub {} puerto {}: no se pudo habilitar.",
                    hueco,
                    puerto_hub
                );
                self.terminar_puerto_hub(hueco, 3, anillo);
                return;
            }
            estado = match self.leer_estado_puerto_hub(hueco, &mut anillo, puerto_hub, buffer)
            {
                Some(e) => e,
                None => {
                    self.terminar_puerto_hub(hueco, 3, anillo);
                    return;
                }
            };
            if !estado.habilitado {
                self.terminar_puerto_hub(hueco, 3, anillo);
                return;
            }
            necesita_reset = false;
        }
        let _ = necesita_reset;

        let velocidad = if estado.velocidad_baja {
            Velocidad::Baja
        } else if estado.velocidad_alta {
            Velocidad::Alta
        } else {
            Velocidad::Completa
        };
        let Some(info) = dispositivo.info_hub else {
            self.terminar_puerto_hub(hueco, 3, anillo);
            return;
        };
        let Some(topologia) =
            dispositivo
                .topologia()
                .hijo(hueco, puerto_hub, dispositivo.velocidad, &info)
        else {
            crate::println!(
                "usb: hub {} puerto {}: demasiados hubs anidados.",
                hueco,
                puerto_hub
            );
            self.terminar_puerto_hub(hueco, 3, anillo);
            return;
        };

        // Si no queda sitio para otro dispositivo, el puerto se reintenta en
        // la ronda siguiente: no se pierde.
        if self.num_dispositivos as usize >= self.dispositivos.len() {
            self.guardar_anillo_hub(hueco, anillo);
            return;
        }

        match self.enumerar_puerto_topologia(dispositivo.puerto, velocidad, topologia) {
            Ok(nuevo) => {
                self.dispositivos[self.num_dispositivos as usize] = nuevo;
                self.num_dispositivos += 1;
                self.contadores.dispositivos += 1;
                let (vend, prod) = match &nuevo.descriptor {
                    Some(d) => (d.id_vendor, d.id_product),
                    None => (0, 0),
                };
                crate::println!(
                    "usb: hub {} puerto {}: dispositivo {:04X}:{:04X}, hueco {}, direccion {}, {}.",
                    hueco,
                    puerto_hub,
                    vend,
                    prod,
                    nuevo.hueco,
                    nuevo.direccion,
                    velocidad.nombre()
                );
                self.terminar_puerto_hub(hueco, 2, anillo);
            }
            Err(e) => {
                self.contadores.errores_enum += 1;
                crate::println!(
                    "usb: hub {} puerto {}: no se pudo enumerar: {}.",
                    hueco,
                    puerto_hub,
                    e.nombre()
                );
                self.terminar_puerto_hub(hueco, 3, anillo);
            }
        }
    }

    /// Lee el estado de un puerto del hub (11.24.2.7).
    ///
    /// La peticion devuelve `wPortStatus` y `wPortChange` seguidos, cuatro
    /// bytes en total. Se leen del buffer de DMA que acaba de rellenar el
    /// controlador.
    fn leer_estado_puerto_hub(
        &mut self,
        hueco: u8,
        anillo: &mut AnilloTransferencia,
        puerto: u8,
        buffer: u64,
    ) -> Option<hub::EstadoPuertoHub> {
        let leidos = self
            .control_ep0(
                hueco,
                anillo,
                setup_hub_get_port_status(puerto),
                buffer,
            )
            .ok()?;
        if leidos < 4 {
            return None;
        }
        let datos = self.dma.buffers.bytes_region();
        if datos.len() < 4 {
            return None;
        }
        let estado = u16::from_le_bytes([datos[0], datos[1]]);
        let cambios = u16::from_le_bytes([datos[2], datos[3]]);
        Some(hub::EstadoPuertoHub::decodificar(puerto, estado, cambios))
    }

    /// Reinicia un puerto de un hub y espera a que el hub avise.
    ///
    /// `SET_FEATURE(PORT_RESET)` es asincrono: el hub pone `C_PORT_RESET`
    /// cuando termina. Hasta entonces el puerto no se puede usar. El cambio se
    /// limpia al final para que la siguiente vuelta no lo confunda con un
    /// reinicio nuevo.
    fn resetear_puerto_hub(
        &mut self,
        hueco: u8,
        anillo: &mut AnilloTransferencia,
        puerto: u8,
        buffer: u64,
    ) -> bool {
        if self
            .control_ep0(
                hueco,
                anillo,
                setup_hub_set_port_feature(puerto, hub::CARACTERISTICA_PORT_RESET),
                buffer,
            )
            .is_err()
        {
            return false;
        }
        let inicio = pit::ticks();
        loop {
            if pit::ticks().saturating_sub(inicio) >= PLAZO_PUERTO {
                return false;
            }
            tarea::dormir(1);
            let Some(estado) = self.leer_estado_puerto_hub(hueco, anillo, puerto, buffer) else {
                return false;
            };
            if estado.cambio_reset {
                let _ = self.control_ep0(
                    hueco,
                    anillo,
                    setup_hub_clear_port_feature(
                        puerto,
                        hub::CARACTERISTICA_C_PORT_RESET,
                    ),
                    buffer,
                );
                return estado.habilitado;
            }
        }
    }

    /// El siguiente hub a sondear, repartiendo las vueltas entre todos.
    fn hub_pendiente(&self) -> Option<enumeracion::Dispositivo> {
        let n = self.num_dispositivos as usize;
        if n == 0 {
            return None;
        }
        let inicio = (self.contadores.sondeos as usize) % n;
        for k in 0..n {
            let d = self.dispositivos[(inicio + k) % n];
            if d.es_hub && d.hub_sig != 0 && d.hub_sig <= d.puertos_hub {
                return Some(d);
            }
        }
        None
    }

    /// Anota el resultado de un puerto y avanza al siguiente.
    ///
    /// Cuando da la vuelta se empieza otra vez por el puerto 1: el hub se
    /// queda vigilado para detectar cambios en caliente.
    fn terminar_puerto_hub(
        &mut self,
        hueco: u8,
        resultado: u8,
        anillo: AnilloTransferencia,
    ) {
        let Some(indice) = self.indice_por_hueco(hueco) else {
            return;
        };
        let d = &mut self.dispositivos[indice];
        let puerto = d.hub_sig;
        if puerto != 0 && (puerto as usize) <= d.hub_estado.len() {
            d.hub_estado[puerto as usize - 1] = resultado;
        }
        d.hub_sig = if d.hub_sig >= d.puertos_hub {
            1
        } else {
            d.hub_sig + 1
        };
        d.anillo_ep0 = anillo;
    }

    /// Deja el anillo del endpoint 0 del hub en su sitio.
    fn guardar_anillo_hub(&mut self, hueco: u8, anillo: AnilloTransferencia) {
        if let Some(indice) = self.indice_por_hueco(hueco) {
            self.dispositivos[indice].anillo_ep0 = anillo;
        }
    }

    /// El indice en la lista de un dispositivo por su hueco.
    fn indice_por_hueco(&self, hueco: u8) -> Option<usize> {
        self.dispositivos[..self.num_dispositivos as usize]
            .iter()
            .position(|d| d.hueco == hueco)
    }

    /// El hueco del hijo que cuelga de un puerto concreto de un hub.
    fn hijo_de_hub(&self, hueco_hub: u8, puerto_hub: u8) -> Option<u8> {
        self.dispositivos[..self.num_dispositivos as usize]
            .iter()
            .find(|d| d.hub_padre == hueco_hub && d.puerto_hub == puerto_hub)
            .map(|d| d.hueco)
    }

    /// Quita un dispositivo de la lista y devuelve su hueco.
    fn quitar_dispositivo(&mut self, hueco: u8) {
        let Some(indice) = self.indice_por_hueco(hueco) else {
            return;
        };
        let d = self.dispositivos[indice];
        crate::println!(
            "usb: dispositivo del puerto {} (hueco {}) desconectado.",
            d.puerto,
            d.hueco
        );
        self.olvidar_hid(hueco);
        // Si el comando falla, el hueco queda perdido, pero no se puede hacer
        // mas: el dispositivo ya no esta para volver a intentarlo.
        let _ = self.deshabilitar_hueco(hueco);
        self.num_dispositivos -= 1;
        let ultimo = self.num_dispositivos as usize;
        self.dispositivos[indice] = self.dispositivos[ultimo];
        self.dispositivos[ultimo] = enumeracion::Dispositivo::VACIO;
    }

    /// Quita todos los dispositivos que cuelgan de un hub, en cualquier nivel.
    fn olvidar_descendientes(&mut self, hueco_hub: u8) {
        loop {
            let Some((hueco, es_hub)) = self.dispositivos[..self.num_dispositivos as usize]
                .iter()
                .find(|d| d.hub_padre == hueco_hub)
                .map(|d| (d.hueco, d.es_hub))
            else {
                return;
            };
            if es_hub {
                self.olvidar_descendientes(hueco);
            }
            self.quitar_dispositivo(hueco);
        }
    }

    /// Quita el hijo de un puerto concreto de un hub (y sus descendientes).
    fn olvidar_hijo_de_hub(&mut self, hueco_hub: u8, puerto_hub: u8) {
        let Some(hueco) = self.hijo_de_hub(hueco_hub, puerto_hub) else {
            return;
        };
        let es_hub = self
            .indice_por_hueco(hueco)
            .map(|i| self.dispositivos[i].es_hub)
            .unwrap_or(false);
        if es_hub {
            self.olvidar_descendientes(hueco);
        }
        self.quitar_dispositivo(hueco);
    }

    /// Olvida el dispositivo de un puerto que ya no esta conectado.
    ///
    /// Le devuelve el hueco al controlador y quita el dispositivo de la lista.
    /// El ultimo de la lista ocupa su sitio: el orden no significa nada, la
    /// posicion solo es un hueco en el array.
    fn olvidar_puerto(&mut self, numero: u8) {
        // Un hub y todos sus hijos comparten el puerto raiz. Al desconectar el
        // hub hay que devolver **todos** sus huecos, no solo el primero.
        loop {
            let Some((hueco, es_hub)) = self.dispositivos[..self.num_dispositivos as usize]
                .iter()
                .find(|d| d.puerto == numero)
                .map(|d| (d.hueco, d.es_hub))
            else {
                return;
            };
            if es_hub {
                self.olvidar_descendientes(hueco);
            }
            self.quitar_dispositivo(hueco);
        }
    }

    /// Reinicia un puerto por software (§4.19.7).
    ///
    /// Pone el bit `PR` y espera a que el controlador ponga `PRC`. Devuelve
    /// `true` si el reinicio se completó.
    fn reiniciar_puerto(&self, numero: u8) -> bool {
        let por = match self.regs.leer_puerto(numero) {
            Ok(v) => v,
            Err(_) => return false,
        };
        // Conservamos lo que se puede conservar y limpiamos las banderas de
        // cambio. El bit de reinicio tiene que empezar a cero.
        let valor = (por & PORTSC_ESCRIBIBLES) | PORTSC_PR | PORTSC_CAMBIO;
        if self.regs.escribir_puerto(numero, valor).is_err() {
            return false;
        }
        let inicio = pit::ticks();
        loop {
            if pit::ticks().saturating_sub(inicio) >= PLAZO_PUERTO {
                return false;
            }
            match self.regs.leer_puerto(numero) {
                Ok(v) => {
                    if v & PORTSC_PRC != 0 {
                        // Limpiar la bandera de cambio del reinicio.
                        let _ = self
                            .regs
                            .escribir_puerto(numero, (v & PORTSC_ESCRIBIBLES) | PORTSC_PRC);
                        return true;
                    }
                }
                Err(_) => return false,
            }
            tarea::dormir(1);
        }
    }

    /// Convierte el estado de los puertos en lo que publica el HCD.
    pub fn captura_publicada(&self) -> crate::drivers::usb::hcd::Publicado {
        let mut puertos =
            [crate::drivers::usb::hcd::InfoPuerto::VACIO; crate::drivers::usb::hcd::MAX_PUERTOS];
        let caps = self.regs.capacidades();
        let n = (self.num_puertos as usize).min(puertos.len());
        for i in 0..n {
            let p = self.puertos[i];
            let proto = match p.protocolo {
                Some(Protocolo::Usb2) => "USB 2.0",
                Some(Protocolo::Usb3) => "USB 3.x",
                None => "desconocido",
            };
            let estado = p.estado;
            puertos[i] = crate::drivers::usb::hcd::InfoPuerto {
                numero: p.numero,
                protocolo: proto,
                conectado: estado.conectado,
                habilitado: estado.habilitado,
                alimentado: estado.alimentado,
                velocidad: estado.velocidad,
            };
        }
        // El nombre viene del PCI, pero el driver solo sabe que es xHCI.
        let _ = caps;

        // Los dispositivos enumerados, traducidos para la consola.
        let mut dispositivos = [crate::drivers::usb::hcd::InfoDispositivo::VACIO;
            crate::drivers::usb::hcd::MAX_DISPOSITIVOS_PUBLICADOS];
        let n = (self.num_dispositivos as usize).min(dispositivos.len());
        for i in 0..n {
            let d = self.dispositivos[i];
            let (vend, prod, clase) = match &d.descriptor {
                Some(desc) => (desc.id_vendor, desc.id_product, desc.b_device_class),
                None => (0, 0, 0),
            };
            // `bMaxPower` esta en unidades de 2 mA (§9.6.3).
            let consumo = match &d.configuracion {
                Some(c) => c.b_max_power as u16 * 2,
                None => 0,
            };
            let (hid, informes) = self.informes_de(d.hueco);
            dispositivos[i] = crate::drivers::usb::hcd::InfoDispositivo {
                puerto: d.puerto,
                direccion: d.direccion,
                hueco: d.hueco,
                velocidad: d.velocidad,
                id_vendor: vend,
                id_product: prod,
                clase,
                interfaces: d.num_interfaces,
                interfaces_hid: d.interfaces_hid(),
                endpoints: d.num_endpoints,
                consumo_ma: consumo,
                configuracion: d.config_actual,
                hid: hid,
                informes,
            };
        }

        crate::drivers::usb::hcd::Publicado {
            nombre: "xHCI",
            estado: crate::drivers::usb::hcd::EstadoHcd::EnMarcha,
            detalle: "",
            contadores: self.contadores,
            num_puertos: self.num_puertos,
            puertos,
            num_dispositivos: self.num_dispositivos,
            dispositivos,
        }
    }
}
