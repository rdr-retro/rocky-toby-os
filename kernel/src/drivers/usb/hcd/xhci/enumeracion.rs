//! La enumeracion de dispositivos: del puerto raiz a los descriptores.
//!
//! ## El camino
//!
//! Una enumeracion son cuatro pasos, en este orden:
//!
//! 1. **Enable Slot** (§4.6.4): el controlador reserva un **hueco** de
//!    dispositivo y devuelve su numero.
//! 2. **Address Device** (§4.6.9): el software rellena el **contexto de
//!    entrada** del hueco (contexto de control, contexto de slot y contexto del
//!    endpoint 0) y se lo manda al controlador. El controlador pone el
//!    dispositivo en el estado *Addressed*, le asigna una direccion USB y deja
//!    el endpoint 0 en marcha.
//! 3. **Transferencias de control por el endpoint 0**: `GET_DESCRIPTOR` del
//!    dispositivo, `GET_DESCRIPTOR` de la configuracion y `SET_CONFIGURATION`.
//! 4. **Recorrido de los descriptores** de configuracion, interfaz y endpoint.
//!    La clase se lee de cada **interfaz**, nunca de la clase del dispositivo;
//!    el descriptor de clase HID (0x21) se copia de la configuracion.
//!
//! ## La transferencia de control (§4.11.2.2)
//!
//! Una transferencia de control son dos o tres TRB encadenados:
//!
//! ```text
//! Setup Stage    el paquete de 8 bytes, dentro del propio TRB (bit IDT)
//! Data Stage     solo si hay datos; la longitud va en el estado del TRB
//! Status Stage   el ultimo; es el que genera el evento (bit IOC)
//! ```
//!
//! Los tres TRB llevan el mismo bit de ciclo (el del productor): el controlador
//! los lee seguidos y para cuando encuentra un TRB con otro ciclo. El bit
//! `Chain` no hace falta: para una transferencia de control el controlador
//! encadena desde el `Setup` hasta el `Status` por el propio tipo de TRB.
//!
//! ## El endpoint 0
//!
//! En la numeracion del controlador el endpoint 0 es el **endpoint 1** (el
//! *Device Context Index* 1, §4.7.2), asi que la campana del hueco se toca con
//! el valor 1.
//!
//! ## Lo que se valida
//!
//! - El hueco que devuelve el controlador tiene que ser uno de los que rocky
//!   tiene reservados: si no, se devuelve y el puerto queda fallido.
//! - El tamano de paquete del endpoint 0 que declara el dispositivo (§9.6.1)
//!   tiene que ser uno de los validos para su velocidad. Si no es el que se
//!   programo, se corrige con un comando **Evaluate Context**.
//! - `wTotalLength` de la configuracion se lee del descriptor y se valida antes
//!   de pedir la configuracion entera.
//! - Los descriptores se parsean con el nucleo USB, que valida longitudes y
//!   tramos. Un descriptor que no cuadra solo tumba **ese** dispositivo.
//!
//! ## Hubs (fase 7, parcial)
//!
//! Un hub se reconoce por su clase (9), se configura y se alimentan sus
//! puertos. El descriptor propio se pide con una transferencia de clase, y el
//! estado de cada puerto se lee y se muestra en la consola.
//!
//! Lo que **no** se hace todavia es enumerar los dispositivos que cuelgan del
//! hub: para eso hacen falta el *route string*, la informacion de TT y el
//! estado de cada puerto a lo largo del sondeo. Un dispositivo conectado a un
//! hub se ve, pero no se le asigna hueco.
//!
//! Referencia: Intel, "eXtensible Host Controller Interface for Universal
//! Serial Bus (xHCI) Requirements Specification", Rev 1.2, §4.6.4, §4.6.9,
//! §4.7.2, §4.11.2 y §6.2.6; USB 2.0 §9.6.
//!
//! Codigo propio de rocky.

use super::ring;
use super::Controlador;
use crate::drivers::usb::core::descriptor::{
    parsear_configuracion, parsear_dispositivo, parsear_endpoint, parsear_interfaz,
    recorrer_descriptores, DescConfiguracion, DescDispositivo, TipoDescriptor,
};
use crate::drivers::usb::core::hub;
use crate::drivers::usb::core::transfer::{
    setup_get_descriptor, setup_hub_get_descriptor, setup_hub_set_port_feature,
    setup_set_configuration,
    PaqueteSetup, DT_CONFIG, DT_DEVICE,
};
use crate::drivers::usb::core::MAX_DESCRIPTORES_BYTES;
use crate::drivers::usb::mmio;
use crate::pit;
use crate::tarea;
use rockyhw::hid::descriptor::{TAM_DESCRIPTOR_HID, TIPO_DESCRIPTOR_HID};
use rockyhw::xhci::context::{
    EndpointContext, InputControlContext, SlotContext, AÑADIR_EP0, AÑADIR_SLOT, EP_ESTADO_INVALIDO,
    EP_TCONTROL, VELOCIDAD_FULL, VELOCIDAD_HIGH, VELOCIDAD_LOW, VELOCIDAD_SUPER,
};
use rockyhw::xhci::port::Velocidad;
use rockyhw::xhci::ring::{
    dcbaa_entrada, trb_address_device, trb_data_stage, trb_evaluate_context, trb_link,
    trb_setup_stage, trb_status_stage, AnilloTransferencia, TipoEvento, FINALIZACION_CORRECTA,
    FINALIZACION_ERROR_BABBLE, FINALIZACION_ERROR_STALL, FINALIZACION_ERROR_TRANSACCION,
    FINALIZACION_PAQUETE_CORTO, RESIDUAL_MASCARA, TRT_DATOS_IN, TRT_DATOS_OUT, TRT_SIN_DATOS,
};

/// El identificador del endpoint 0 en la numeracion del controlador.
///
/// El controlador numera los endpoints de control desde el 1: el *Device
/// Context Index* 1 es el endpoint 0 del dispositivo (§4.7.2).
pub const EP_ID_CONTROL: u8 = 1;

/// El valor de la campana del endpoint 0.
///
/// La campana de un hueco se escribe con `DB_VALUE(ep_id, stream_id)`: el
/// identificador del endpoint en los bits 7:0 y el del flujo en los bits 31:16
/// (§5.4.4). Para el endpoint 0 de control el identificador es 1.
pub const VALOR_CAMPANA_EP0: u32 = EP_ID_CONTROL as u32;

/// Plazo para esperar el evento de una transferencia, en tic del PIT
/// (100 tic = 1 segundo).
pub const PLAZO_TRANSFERENCIA: u64 = 100;

/// Limite de iteraciones del sondeo del anillo de eventos al esperar una
/// transferencia.
///
/// Cada iteracion es una lectura del anillo, no una espera: el plazo es lo que
/// de verdad acota el tiempo. Este limite evita que un anillo con datos basura
/// deje el bucle dentro del sondeo para siempre.
pub const MAX_ITERACIONES_TRANSFERENCIA: usize = 1000;

/// La longitud de la cabecera de un descriptor de dispositivo (§9.6.1).
pub const TAM_CABECERA_DISPOSITIVO: u16 = 8;

/// La longitud del descriptor de dispositivo (§9.6.1).
pub const TAM_DESCRIPTOR_DISPOSITIVO: u16 = 18;

/// La longitud de la cabecera de un descriptor de configuracion (§9.6.3).
pub const TAM_CABECERA_CONFIGURACION: u16 = 9;

/// Cuantos bytes del buffer de DMA se usan como maximo en una transferencia.
///
/// Es el tope del nucleo USB para un descriptor (`MAX_DESCRIPTORES_BYTES`): una
/// configuracion mas grande que esto se rechaza en vez de pedirse a medias.
pub const MAX_BUFFER_CONFIGURACION: u16 = MAX_DESCRIPTORES_BYTES as u16;

/// Cuantas interfaces se guardan de un dispositivo.
pub const MAX_INTERFACES_DISPOSITIVO: usize = 16;

/// Cuantos endpoints se guardan de un dispositivo.
pub const MAX_ENDPOINTS_DISPOSITIVO: usize = 16;

/// La clase de interfaz HID (§4.2 de la especificacion de HID).
pub const CLASE_HID: u8 = 3;

/// El tipo de transferencia de un endpoint de control (no se declara en una
/// configuracion: el endpoint 0 siempre es de control).
const TIPO_CONTROL: u8 = 0;

/// El tipo de transferencia de interrupcion (bits 1:0 de `bmAttributes`).
pub(super) const TIPO_INTERRUPCION: u8 = 3;

/// Por que no se pudo enumerar un dispositivo.
///
/// Todos los fallos son **de ese dispositivo**: el puerto queda marcado como
/// fallido, su hueco se devuelve al controlador y el resto sigue igual.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorEnum {
    /// El controlador no contesto a un comando (Enable Slot o Address Device).
    Comando,
    /// El controlador rechazo el comando con un codigo de finalizacion.
    ComandoRechazado(u32),
    /// El hueco que dio el controlador no lo gestiona rocky.
    HuecoInvalido,
    /// El controlador no dio el evento de la transferencia a tiempo.
    SinRespuesta,
    /// El dispositivo pidio que se pare el endpoint (`STALL`).
    Stall,
    /// El dispositivo no contesto: error de transaccion.
    ErrorTransaccion,
    /// El dispositivo mando mas datos de los que caben en el buffer.
    Babble,
    /// El controlador dio un codigo de fin de transferencia desconocido.
    CodigoDesconocido(u32),
    /// El dispositivo devolvio un descriptor que no se puede usar.
    DescriptorInvalido,
    /// La configuracion no declara ninguna interfaz.
    SinInterfaces,
    /// La velocidad del puerto no es utilizable.
    VelocidadInvalida,
    /// El controlador no acepto la campana del endpoint.
    Controlador,
}

impl ErrorEnum {
    /// Nombre corto, para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            ErrorEnum::Comando => "el controlador no contesto al comando",
            ErrorEnum::ComandoRechazado(_) => "el controlador rechazo el comando",
            ErrorEnum::HuecoInvalido => "el controlador dio un hueco fuera de rango",
            ErrorEnum::SinRespuesta => "la transferencia de control no llego a tiempo",
            ErrorEnum::Stall => "el dispositivo paro el endpoint (STALL)",
            ErrorEnum::ErrorTransaccion => "error de transaccion con el dispositivo",
            ErrorEnum::Babble => "el dispositivo mando mas datos de los pedidos",
            ErrorEnum::CodigoDesconocido(_) => "codigo de fin de transferencia desconocido",
            ErrorEnum::DescriptorInvalido => "descriptor de dispositivo invalido",
            ErrorEnum::SinInterfaces => "la configuracion no declara ninguna interfaz",
            ErrorEnum::VelocidadInvalida => "el puerto no declara una velocidad utilizable",
            ErrorEnum::Controlador => "el controlador no acepto la campana del endpoint",
        }
    }

    /// El codigo de finalizacion del controlador, si lo hay.
    ///
    /// Sirve para la consola: un fallo sin codigo es de rocky; con codigo, lo
    /// puso el controlador y se puede buscar en la tabla 6-91.
    pub fn codigo(self) -> Option<u32> {
        match self {
            ErrorEnum::ComandoRechazado(c) | ErrorEnum::CodigoDesconocido(c) => Some(c),
            _ => None,
        }
    }
}

/// Una interfaz de un dispositivo, ya validada.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Interfaz {
    /// El numero de interfaz (`bInterfaceNumber`).
    pub numero: u8,
    /// El ajuste alterno (`bAlternateSetting`).
    pub alterna: u8,
    /// La clase de la interfaz: 3 es HID.
    pub clase: u8,
    /// La subclase.
    pub subclase: u8,
    /// El protocolo.
    pub protocolo: u8,
    /// Cuantos endpoints declara (`bNumEndpoints`).
    pub endpoints: u8,
    /// El indice del primer endpoint en [`Dispositivo::endpoints`].
    pub primer_endpoint: u8,
    /// El descriptor de clase HID (0x21) que declara la interfaz, si lo hay.
    ///
    /// Se copia de la **configuracion**: es donde lo publica el dispositivo y
    /// de donde lo lee tambien Linux. La peticion suelta `GET_DESCRIPTOR(0x21)`
    /// no la contesta todo el mundo (QEMU la para con STALL aunque el
    /// descriptor este en la configuracion), asi que no se pide.
    ///
    /// Todo a cero significa que la interfaz no lo declara.
    pub hid: [u8; TAM_DESCRIPTOR_HID as usize],
}

impl Interfaz {
    /// Una interfaz que no existe.
    pub const VACIA: Interfaz = Interfaz {
        numero: 0,
        alterna: 0,
        clase: 0,
        subclase: 0,
        protocolo: 0,
        endpoints: 0,
        primer_endpoint: 0,
        hid: [0; TAM_DESCRIPTOR_HID as usize],
    };

    /// ¿Es una interfaz HID?
    pub fn es_hid(&self) -> bool {
        self.clase == CLASE_HID
    }
}

/// Un endpoint de un dispositivo, ya validado.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Endpoint {
    /// La direccion (`bEndpointAddress`): bit 7 a uno es entrada.
    pub direccion: u8,
    /// El numero de endpoint (bits 3:0 de la direccion).
    pub numero: u8,
    /// El tipo de transferencia (bits 1:0 de `bmAttributes`).
    pub tipo: u8,
    /// El tamano maximo de paquete (`wMaxPacketSize`, bits 10:0).
    pub max_paquete: u16,
    /// El intervalo de sondeo.
    pub intervalo: u8,
}

impl Endpoint {
    /// Un endpoint que no existe.
    pub const VACIO: Endpoint = Endpoint {
        direccion: 0,
        numero: 0,
        tipo: 0,
        max_paquete: 0,
        intervalo: 0,
    };

    /// ¿Es de entrada?
    pub fn es_entrada(&self) -> bool {
        self.direccion & 0x80 != 0
    }
}

/// Un dispositivo enumerado.
#[derive(Clone, Copy, Debug)]
pub struct Dispositivo {
    /// El hueco del controlador que ocupa.
    pub hueco: u8,
    /// El puerto raiz donde esta conectado.
    pub puerto: u8,
    /// La direccion USB que le ha dado el controlador.
    pub direccion: u8,
    /// La velocidad del enlace.
    pub velocidad: Velocidad,
    /// El descriptor del dispositivo.
    pub descriptor: Option<DescDispositivo>,
    /// El descriptor de configuracion.
    pub configuracion: Option<DescConfiguracion>,
    /// La configuracion que se le ha pedido al dispositivo.
    pub config_actual: u8,
    /// Las interfaces (una entrada por descriptor de interfaz, alternos
    /// incluidos).
    pub interfaces: [Interfaz; MAX_INTERFACES_DISPOSITIVO],
    /// Cuantas interfaces se han leido.
    pub num_interfaces: u8,
    /// Los endpoints, en el orden en que aparecen en la configuracion.
    pub endpoints: [Endpoint; MAX_ENDPOINTS_DISPOSITIVO],
    /// Cuantos endpoints se han leido.
    pub num_endpoints: u8,
    /// Si el dispositivo es un hub, lo que dice su descriptor.
    pub info_hub: Option<hub::HubInfo>,
    /// Cadena de ruta para xHCI (nibbles por nivel).
    pub ruta: u32,
    /// Profundidad de hubs (0 = puerto raiz).
    pub profundidad: u8,
    /// Hueco del hub padre (0 = ninguno).
    pub hub_padre: u8,
    /// Puerto en el hub padre (1..).
    pub puerto_hub: u8,
    /// Es un hub.
    pub es_hub: bool,
    /// Puertos que declara el hub.
    pub puertos_hub: u8,
    /// Siguiente puerto del hub a sondear.
    pub hub_sig: u8,
    /// Estado por puerto de hub: 0=sin intentar,1=vacio,2=enumerado,3=fallido.
    pub hub_estado: [u8; 16],
    /// El estado del anillo del endpoint 0, para seguir hablando con un hub
    /// despues de la enumeracion.
    pub anillo_ep0: AnilloTransferencia,
}

impl Dispositivo {
    /// Un dispositivo que no existe.
    pub const VACIO: Dispositivo = Dispositivo {
        hueco: 0,
        puerto: 0,
        direccion: 0,
        velocidad: Velocidad::Indefinida,
        descriptor: None,
        configuracion: None,
        config_actual: 0,
        interfaces: [Interfaz::VACIA; MAX_INTERFACES_DISPOSITIVO],
        num_interfaces: 0,
        endpoints: [Endpoint::VACIO; MAX_ENDPOINTS_DISPOSITIVO],
        num_endpoints: 0,
        info_hub: None,
        ruta: 0,
        profundidad: 0,
        hub_padre: 0,
        puerto_hub: 0,
        es_hub: false,
        puertos_hub: 0,
        hub_sig: 0,
        hub_estado: [0; 16],
        anillo_ep0: AnilloTransferencia {
            base: 0,
            trbs: ring::TRB_EP0,
            indice: 0,
            ciclo: true,
            dio_la_vuelta: false,
        },
    };

    /// Las interfaces leidas.
    pub fn lista_interfaces(&self) -> &[Interfaz] {
        &self.interfaces[..self.num_interfaces as usize]
    }

    /// La topologia de este dispositivo, para calcular la de sus hijos.
    pub fn topologia(&self) -> Topologia {
        Topologia {
            ruta: self.ruta,
            profundidad: self.profundidad,
            hub_padre: self.hub_padre,
            puerto_hub: self.puerto_hub,
            velocidad_padre: self.velocidad,
            ttt: 0,
            mtt: false,
        }
    }

    /// Cuantas interfaces HID tiene.
    ///
    /// Se mira la clase de **cada interfaz**, no la clase del dispositivo: un
    /// teclado o un raton suelen declarar la clase 0 en el dispositivo y la 3
    /// en la interfaz (§4.3 de la especificacion de HID).
    pub fn interfaces_hid(&self) -> u8 {
        let mut n = 0;
        for i in self.lista_interfaces() {
            if i.es_hid() {
                n += 1;
            }
        }
        n
    }

    /// Cuantos endpoints de interrupcion de **entrada** hay.
    ///
    /// Son los que hacen falta para leer un HID: el teclado y el raton avisan
    /// de cada cambio por ahi, con una transferencia de interrupcion. Esta
    /// fase los reconoce y los valida, pero todavia no los programa.
    pub fn interrupciones_de_entrada(&self) -> u8 {
        let mut n = 0;
        for interfaz in self.lista_interfaces() {
            for e in self.endpoints_de(interfaz) {
                if e.es_entrada() && e.tipo == TIPO_INTERRUPCION {
                    n += 1;
                }
            }
        }
        n
    }

    /// Los endpoints de una interfaz.
    pub fn endpoints_de(&self, interfaz: &Interfaz) -> &[Endpoint] {
        let inicio = (interfaz.primer_endpoint as usize).min(self.endpoints.len());
        let fin = (inicio + interfaz.endpoints as usize).min(self.endpoints.len());
        &self.endpoints[inicio..fin]
    }
}

/// De donde cuelga un dispositivo y como llegar a el (§4.5.2).
///
/// `ruta` es el *route string* de xHCI: un nibble por nivel, el menos
/// significativo para el primer hub. Un dispositivo en un puerto raiz tiene
/// ruta cero; el puerto raiz va en el contexto de slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Topologia {
    /// Cadena de ruta acumulada.
    pub ruta: u32,
    /// Cuantos hubs hay por encima (0 = puerto raiz).
    pub profundidad: u8,
    /// Hueco del hub padre (0 = ninguno).
    pub hub_padre: u8,
    /// Puerto en el hub padre (0 = ninguno).
    pub puerto_hub: u8,
    /// Velocidad del hub padre.
    pub velocidad_padre: Velocidad,
    /// *TT Think Time* del hub padre (0 si no aplica).
    pub ttt: u8,
    /// El hub padre tiene multiples TT.
    pub mtt: bool,
}

impl Topologia {
    /// Un dispositivo conectado directamente a un puerto raiz.
    pub const RAIZ: Topologia = Topologia {
        ruta: 0,
        profundidad: 0,
        hub_padre: 0,
        puerto_hub: 0,
        velocidad_padre: Velocidad::Indefinida,
        ttt: 0,
        mtt: false,
    };

    /// La topologia de un dispositivo colgado de este hub.
    ///
    /// Devuelve `None` si ya se ha llegado al limite de anidamiento o si el
    /// puerto no cabe en un nibble (los hubs USB 2.0 tienen 15 como maximo).
    pub fn hijo(
        &self,
        hueco_hub: u8,
        puerto: u8,
        velocidad_hub: Velocidad,
        info: &hub::HubInfo,
    ) -> Option<Topologia> {
        if self.profundidad >= hub::MAX_PROFUNDIDAD_HUB || puerto == 0 || puerto > 15 {
            return None;
        }
        let nivel = (self.profundidad as u32) * 4;
        let ruta = (self.ruta | ((puerto as u32) << nivel)) & 0xF_FFFF;
        Some(Topologia {
            ruta,
            profundidad: self.profundidad + 1,
            hub_padre: hueco_hub,
            puerto_hub: puerto,
            velocidad_padre: velocidad_hub,
            ttt: ((info.caracteristicas >> 5) & 0x3) as u8,
            mtt: info.multi_tt(),
        })
    }

    /// Si el dispositivo necesita informacion de TT (§4.5.2).
    ///
    /// Solo un dispositivo de baja o completa velocidad detras de un hub de
    /// alta velocidad. Los hubs anidados con un hub de completa por medio
    /// necesitarian apuntar al hub de alta mas cercano; eso queda pendiente.
    pub fn necesita_tt(&self, velocidad: Velocidad) -> bool {
        self.hub_padre != 0
            && matches!(velocidad, Velocidad::Baja | Velocidad::Completa)
            && matches!(self.velocidad_padre, Velocidad::Alta)
    }
}

/// El contenido acotado de una configuracion.
struct ContenidoConfig {
    interfaces: [Interfaz; MAX_INTERFACES_DISPOSITIVO],
    num_interfaces: u8,
    endpoints: [Endpoint; MAX_ENDPOINTS_DISPOSITIVO],
    num_endpoints: u8,
}

impl ContenidoConfig {
    /// Un contenido vacio.
    const fn nuevo() -> ContenidoConfig {
        ContenidoConfig {
            interfaces: [Interfaz::VACIA; MAX_INTERFACES_DISPOSITIVO],
            num_interfaces: 0,
            endpoints: [Endpoint::VACIO; MAX_ENDPOINTS_DISPOSITIVO],
            num_endpoints: 0,
        }
    }
}

impl Controlador {
    /// Enumera el dispositivo que hay conectado en un puerto raiz.
    ///
    /// Si algo falla, el hueco se devuelve al controlador y el puerto se queda
    /// como fallido: un dispositivo que no se puede enumerar no puede tumbar el
    /// arranque ni el resto de puertos.
    pub fn enumerar_puerto(
        &mut self,
        puerto: u8,
        velocidad: Velocidad,
    ) -> Result<Dispositivo, ErrorEnum> {
        self.enumerar_puerto_topologia(puerto, velocidad, Topologia::RAIZ)
    }

    /// Enumera un dispositivo que puede colgar de un hub.
    ///
    /// `puerto` es siempre el puerto **raiz** del arbol, no el puerto del hub:
    /// es el que necesita el contexto de slot. La posicion dentro del hub va en
    /// `topologia`.
    pub fn enumerar_puerto_topologia(
        &mut self,
        puerto: u8,
        velocidad: Velocidad,
        topologia: Topologia,
    ) -> Result<Dispositivo, ErrorEnum> {
        if velocidad == Velocidad::Indefinida {
            return Err(ErrorEnum::VelocidadInvalida);
        }
        if !self.contextos.es_valido() {
            return Err(ErrorEnum::HuecoInvalido);
        }

        // 1. Enable Slot: el controlador reserva un hueco.
        let hueco = self.habilitar_hueco().map_err(|_| ErrorEnum::Comando)?;

        // El hueco tiene que ser uno de los que rocky tiene reservados. Si el
        // controlador da otro, se devuelve: usar memoria que no es nuestra
        // seria corromper el kernel.
        if hueco == 0 || hueco as usize > ring::MAX_HUECOS {
            let _ = self.deshabilitar_hueco(hueco);
            return Err(ErrorEnum::HuecoInvalido);
        }
        let indice = hueco as usize - 1;

        // 2. A partir de aqui, cualquier fallo devuelve el hueco.
        match self.enumerar_en_hueco(hueco, indice, puerto, velocidad, topologia) {
            Ok(dispositivo) => Ok(dispositivo),
            Err(e) => {
                // Un HID a medias ya no tiene dueno: sus anillos no pueden
                // quedar asociados a un hueco que se va a reutilizar.
                self.olvidar_hid(hueco);
                let _ = self.deshabilitar_hueco(hueco);
                Err(e)
            }
        }
    }

    /// La enumeracion propiamente dicha, con el hueco ya reservado.
    fn enumerar_en_hueco(
        &mut self,
        hueco: u8,
        indice: usize,
        puerto: u8,
        velocidad: Velocidad,
        topologia: Topologia,
    ) -> Result<Dispositivo, ErrorEnum> {
        // El hueco puede venir de un dispositivo anterior: sus endpoints de
        // interrupcion ya no valen para este.
        self.olvidar_hid(hueco);

        let salida = self
            .contextos
            .salida(indice)
            .ok_or(ErrorEnum::HuecoInvalido)?;
        let entrada = self
            .contextos
            .entrada(indice)
            .ok_or(ErrorEnum::HuecoInvalido)?;
        let base_anillo = self
            .contextos
            .anillo_ep0(indice)
            .ok_or(ErrorEnum::HuecoInvalido)?;

        // El buffer de las transferencias: la region de DMA que ya esta
        // reservada. Tiene 64 KiB, de sobra para un descriptor.
        let buffer = self.dma.buffers.base() as u64;
        if self.dma.buffers.bytes() < MAX_BUFFER_CONFIGURACION as usize {
            return Err(ErrorEnum::HuecoInvalido);
        }

        // 3. El array de contextos base tiene que apuntar al contexto de salida
        //    de este hueco (§6.1). Se escribe antes del comando: el controlador
        //    lo lee al direccionar el dispositivo.
        let base_dcbaa = self.dma.dcbaa.base() as u64;
        if (hueco as usize + 1) * 8 > self.dma.dcbaa.bytes() {
            return Err(ErrorEnum::HuecoInvalido);
        }
        let entrada_dcbaa = base_dcbaa + hueco as u64 * 8;
        ring::escribir_u64(entrada_dcbaa, dcbaa_entrada(salida));

        // 4. El anillo del endpoint 0 se borra entero y se le pone su TRB Link.
        //
        //    Borrarlo no es cosmetico: si el hueco se reutiliza, los TRB del
        //    dispositivo anterior seguiriian ahi y el controlador los leeria
        //    como si fueran de este. Un anillo a cero tiene todos los TRB con
        //    el ciclo a cero, asi que el controlador (que empieza con el ciclo
        //    a uno) para en el primero que no es suyo.
        ring::borrar(base_anillo, ring::TAM_ANILLO_EP0);
        let mut anillo = AnilloTransferencia::nuevo(base_anillo, ring::TRB_EP0)
            .ok_or(ErrorEnum::HuecoInvalido)?;
        let link = trb_link(base_anillo, true);
        ring::escribir_trb(anillo.direccion_link(), &link);

        // 5. Address Device (§4.6.9).
        let max_paquete = max_paquete_ep0(velocidad);
        self.escribir_contexto_entrada(
            entrada,
            base_anillo,
            puerto,
            velocidad,
            max_paquete,
            &topologia,
        );

        // El controlador tiene que ver los contextos y el anillo: la cache del
        // procesador no puede quedar por delante de la DMA.
        mmio::clflush_rango(entrada, ring::TAM_CTX_ENTRADA);
        mmio::clflush_rango(base_anillo, ring::TAM_ANILLO_EP0);
        mmio::clflush_rango(entrada_dcbaa, 8);
        mmio::mfence();

        let evento = self
            .ejecutar_comando(trb_address_device(entrada, hueco, false))
            .map_err(|_| ErrorEnum::Comando)?;
        if evento.codigo_finalizacion() != FINALIZACION_CORRECTA {
            return Err(ErrorEnum::ComandoRechazado(evento.codigo_finalizacion()));
        }

        // 6. La direccion que le ha puesto el controlador: la escribe en el
        //    contexto de salida (§4.2.3, "Addressed"). Si no pusiera ninguna
        //    (no deberia), se usa el numero de hueco, que es la direccion que
        //    eligen los controladores reales.
        let direccion = {
            let estado = ring::leer_u32(salida + 12);
            let asignada = (estado & 0xFF) as u8;
            if asignada == 0 {
                hueco
            } else {
                asignada
            }
        };

        // 7. La cabecera del descriptor de dispositivo: los 8 primeros bytes,
        //    que son los que dicen el tamano de paquete del endpoint 0 (§9.6.1).
        let leidos = self.control_ep0(
            hueco,
            &mut anillo,
            setup_get_descriptor(DT_DEVICE, 0, TAM_CABECERA_DISPOSITIVO),
            buffer,
        )?;
        if leidos < TAM_CABECERA_DISPOSITIVO {
            return Err(ErrorEnum::DescriptorInvalido);
        }
        let declarado = {
            // Seguro: el buffer es una region de DMA reservada por el kernel y
            // `leidos` no pasa de lo pedido, que esta acotado.
            let bytes =
                unsafe { core::slice::from_raw_parts(buffer as *const u8, leidos as usize) };
            bytes[7]
        };
        let tamano =
            tamano_ep0_declarado(declarado, velocidad).ok_or(ErrorEnum::DescriptorInvalido)?;
        if tamano != max_paquete {
            // El dispositivo declara otro tamano de paquete para el endpoint 0.
            // Se corrige con Evaluate Context (§4.6.10) antes de pedirle mas
            // datos: si no, las transferencias grandes no cuadran.
            self.evaluar_max_paquete_ep0(entrada, tamano)?;
        }

        // 8. El descriptor de dispositivo completo (§9.6.1, 18 bytes).
        let leidos = self.control_ep0(
            hueco,
            &mut anillo,
            setup_get_descriptor(DT_DEVICE, 0, TAM_DESCRIPTOR_DISPOSITIVO),
            buffer,
        )?;
        if leidos < TAM_DESCRIPTOR_DISPOSITIVO {
            return Err(ErrorEnum::DescriptorInvalido);
        }
        let descriptor = {
            // Seguro: igual que antes.
            let bytes =
                unsafe { core::slice::from_raw_parts(buffer as *const u8, leidos as usize) };
            parsear_dispositivo(bytes).map_err(|_| ErrorEnum::DescriptorInvalido)?
        };

        // 9. La configuracion: primero la cabecera, para saber cuanto mide, y
        //    despues la configuracion entera.
        let leidos = self.control_ep0(
            hueco,
            &mut anillo,
            setup_get_descriptor(DT_CONFIG, 0, TAM_CABECERA_CONFIGURACION),
            buffer,
        )?;
        if leidos < TAM_CABECERA_CONFIGURACION {
            return Err(ErrorEnum::DescriptorInvalido);
        }
        let cabecera = {
            // Seguro: igual que antes.
            let bytes =
                unsafe { core::slice::from_raw_parts(buffer as *const u8, leidos as usize) };
            parsear_configuracion(bytes, 0).map_err(|_| ErrorEnum::DescriptorInvalido)?
        };
        let total = cabecera.w_total_length;
        if total < TAM_CABECERA_CONFIGURACION
            || total > MAX_BUFFER_CONFIGURACION
            || total as usize > self.dma.buffers.bytes()
        {
            return Err(ErrorEnum::DescriptorInvalido);
        }

        let leidos = self.control_ep0(
            hueco,
            &mut anillo,
            setup_get_descriptor(DT_CONFIG, 0, total),
            buffer,
        )?;
        if leidos != total {
            // La configuracion se declara de un tamano y llega de otro: no se
            // puede recorrer sin adivinar.
            return Err(ErrorEnum::DescriptorInvalido);
        }

        let configuracion = {
            // Seguro: igual que antes.
            let bytes =
                unsafe { core::slice::from_raw_parts(buffer as *const u8, leidos as usize) };
            let config =
                parsear_configuracion(bytes, 0).map_err(|_| ErrorEnum::DescriptorInvalido)?;
            if config.w_total_length != total {
                return Err(ErrorEnum::DescriptorInvalido);
            }
            config
        };

        let contenido = {
            // Seguro: igual que antes.
            let bytes =
                unsafe { core::slice::from_raw_parts(buffer as *const u8, leidos as usize) };
            recorrer_configuracion(bytes, configuracion.b_num_interfaces)?
        };
        if contenido.num_interfaces == 0 {
            return Err(ErrorEnum::SinInterfaces);
        }

        // 10. SET_CONFIGURATION (§9.4.7). Se elige la configuracion que dice el
        //     descriptor: la 0 no es valida.
        if configuracion.b_configuration_value == 0 {
            return Err(ErrorEnum::DescriptorInvalido);
        }
        self.control_ep0(
            hueco,
            &mut anillo,
            setup_set_configuration(configuracion.b_configuration_value),
            buffer,
        )?;

        // 11. Los endpoints de interrupcion de las interfaces HID boot (§4.6.6).
        //     Se programan aqui porque hasta `SET_CONFIGURATION` no existen. Un
        //     fallo en una interfaz **no** tumba el dispositivo: se queda
        //     configurado y esa interfaz simplemente no se usa.
        self.preparar_hid(
            hueco,
            indice,
            puerto,
            velocidad,
            &topologia,
            &contenido.interfaces[..contenido.num_interfaces as usize],
            &contenido.endpoints[..contenido.num_endpoints as usize],
            &mut anillo,
            buffer,
        );

        // 12. Un hub: descriptor propio, alimentacion y puertos. Un fallo en
        //     esta parte no puede dejar el hub a medias: se devuelve el hueco
        //     como con cualquier otro dispositivo que no se puede usar.
        let info_hub = if descriptor.b_device_class == hub::CLASE_HUB {
            Some(self.configurar_hub(
                hueco,
                descriptor.b_device_protocol,
                puerto,
                velocidad,
                &topologia,
                &mut anillo,
                buffer,
            )?)
        } else {
            None
        };

        let es_hub = info_hub.is_some();
        let puertos_hub = match &info_hub {
            Some(info) => info.puertos_utiles,
            None => 0,
        };
        Ok(Dispositivo {
            hueco,
            puerto,
            direccion,
            velocidad,
            descriptor: Some(descriptor),
            configuracion: Some(configuracion),
            config_actual: configuracion.b_configuration_value,
            interfaces: contenido.interfaces,
            num_interfaces: contenido.num_interfaces,
            endpoints: contenido.endpoints,
            num_endpoints: contenido.num_endpoints,
            info_hub,
            ruta: topologia.ruta,
            profundidad: topologia.profundidad,
            hub_padre: topologia.hub_padre,
            puerto_hub: topologia.puerto_hub,
            es_hub,
            puertos_hub,
            hub_sig: if es_hub { 1 } else { 0 },
            hub_estado: [0; 16],
            anillo_ep0: anillo,
        })
    }

    /// Configura un hub recien enumerado (USB 2.0, cap. 11).
    ///
    /// Se pide el descriptor de hub, se marca el hueco como hub con Evaluate
    /// Context, se alimentan los puertos y se lee su estado. No se enumeran
    /// los dispositivos que cuelgan de el: eso necesita route string y TT y
    /// queda para la fase siguiente.
    fn configurar_hub(
        &mut self,
        hueco: u8,
        protocolo_dispositivo: u8,
        puerto: u8,
        velocidad: Velocidad,
        topologia: &Topologia,
        anillo: &mut AnilloTransferencia,
        buffer: u64,
    ) -> Result<hub::HubInfo, ErrorEnum> {
        let leidos = self.control_ep0(
            hueco,
            anillo,
            setup_hub_get_descriptor(hub::TAM_DESC_HUB_PEDIDO),
            buffer,
        )?;
        if leidos < hub::TAM_DESC_HUB {
            return Err(ErrorEnum::DescriptorInvalido);
        }
        let mut info = {
            // Seguro: el buffer es DMA reservada y `leidos` no pasa de lo
            // pedido, que esta acotado.
            let bytes =
                unsafe { core::slice::from_raw_parts(buffer as *const u8, leidos as usize) };
            hub::parsear_hub(bytes).map_err(|_| ErrorEnum::DescriptorInvalido)?
        };
        // El descriptor de hub no dice si hay uno o varios TT: eso viene en el
        // `bDeviceProtocol` del descriptor de dispositivo.
        info.protocolo = protocolo_dispositivo;
        crate::println!(
            "usb: hub {}: descriptor de {} puertos, {} mA, alimentacion en {} ms.",
            hueco,
            info.puertos,
            info.corriente,
            info.estabilizacion_ms()
        );
        if info.puertos > info.puertos_utiles {
            crate::println!(
                "usb: hub {}: solo se recorren {} de sus {} puertos.",
                hueco,
                info.puertos_utiles,
                info.puertos
            );
        }

        // El contexto de slot tiene que decir que es un hub y cuantos puertos
        // tiene. Un fallo aqui no impide usar las peticiones de clase, asi que
        // no se tumba el dispositivo.
        if let Err(e) = self.marcar_hub(hueco, puerto, velocidad, topologia, &info) {
            crate::println!(
                "usb: hub {}: no se pudo marcar como hub en el contexto de slot ({}).",
                hueco,
                e.nombre()
            );
        }

        // Alimentar los puertos (11.24.2.1). Si un puerto ya estaba alimentado,
        // el hub puede contestar STALL: se cuenta como alimentado.
        let mut alimentados = 0u8;
        for p in 1..=info.puertos_utiles {
            match self.control_ep0(
                hueco,
                anillo,
                setup_hub_set_port_feature(p, hub::CARACTERISTICA_PORT_POWER),
                buffer,
            ) {
                Ok(_) | Err(ErrorEnum::Stall) => alimentados += 1,
                Err(_) => {}
            }
        }

        // Esperar a que la alimentacion se estabilice, con un tope.
        let espera = info
            .estabilizacion_ms()
            .min(hub::LimitesHub::default().max_estabilizacion_ms);
        tarea::dormir(espera);

        crate::println!(
            "usb: hub {}: {} puertos alimentados; se sondearan en las rondas siguientes.",
            hueco,
            alimentados
        );
        Ok(info)
    }

    /// Marca el hueco como hub y declara su numero de puertos (§4.6.10).
    ///
    /// El bit de hub y `MaxPorts` no se pueden poner en el contexto de slot del
    /// Address Device, porque hasta leer el descriptor no se sabe que el
    /// dispositivo es un hub. Se actualizan con Evaluate Context, igual que
    /// hace Linux en `xhci_update_hub_device`.
    fn marcar_hub(
        &mut self,
        hueco: u8,
        puerto: u8,
        velocidad: Velocidad,
        topologia: &Topologia,
        info: &hub::HubInfo,
    ) -> Result<(), ErrorEnum> {
        let indice = hueco as usize - 1;
        let entrada = self
            .contextos
            .entrada(indice)
            .ok_or(ErrorEnum::HuecoInvalido)?;

        let mut control = InputControlContext::default();
        control.set_drop_flags(0);
        control.set_add_flags(AÑADIR_SLOT);

        let mut slot = SlotContext::default();
        slot.set_route_speed(
            topologia.ruta,
            velocidad_contexto(velocidad),
            true,
            info.multi_tt(),
        );
        if topologia.necesita_tt(velocidad) {
            slot.set_tt_info(topologia.hub_padre, topologia.puerto_hub, topologia.ttt);
        }
        slot.set_context_entries(1);
        slot.set_root_hub_port(puerto);
        slot.set_max_ports(info.puertos);
        slot.set_max_exit_latency(0);
        slot.set_interrupter_target(0);

        escribir_contexto(entrada, &control.d);
        escribir_contexto(entrada + 32, &slot.d);
        mmio::clflush_rango(entrada, ring::TAM_CTX_ENTRADA);
        mmio::mfence();

        let evento = self
            .ejecutar_comando(trb_evaluate_context(entrada, hueco))
            .map_err(|_| ErrorEnum::Comando)?;
        if evento.codigo_finalizacion() != FINALIZACION_CORRECTA {
            return Err(ErrorEnum::ComandoRechazado(evento.codigo_finalizacion()));
        }
        Ok(())
    }

    /// Envia una transferencia de control por el endpoint 0 de un hueco.
    ///
    /// Devuelve cuantos bytes de datos se han transferido de verdad.
    ///
    /// La tanda se escribe en el anillo del endpoint 0 y se avisa al
    /// controlador con la campana del hueco. El evento lo genera el TRB de
    /// estado, que es el ultimo.
    pub(super) fn control_ep0(
        &mut self,
        hueco: u8,
        anillo: &mut AnilloTransferencia,
        setup: PaqueteSetup,
        buffer: u64,
    ) -> Result<u16, ErrorEnum> {
        let pedido = setup.w_length;
        let hay_datos = pedido != 0;
        let dir_in = setup.bm_request_type & 0x80 != 0;

        // La tanda no se puede partir entre el final del segmento y el TRB
        // Link: el controlador leeria la segunda parte en la vuelta anterior.
        let cuantos: u16 = if hay_datos { 3 } else { 2 };
        if !anillo.cabe(cuantos) {
            // No queda sitio antes del Link del final. Como todas las
            // transferencias son sincronas, no hay nada en vuelo: se pone un
            // Link en el hueco actual, que apunta al principio, y se empieza
            // la vuelta nueva con el ciclo invertido. El Link del final se
            // reescribira cuando el productor vuelva a llegar a el.
            let dir_link = anillo.base + anillo.indice as u64 * 16;
            let mut link = trb_link(anillo.base, true);
            link.set_ciclo(anillo.ciclo);
            ring::escribir_trb(dir_link, &link);
            mmio::clflush_rango(dir_link, 16);
            anillo.indice = 0;
            anillo.ciclo = !anillo.ciclo;
            anillo.dio_la_vuelta = false;
        }

        let (dir_setup, ciclo_setup) = anillo.siguiente();
        let (dir_datos, ciclo_datos) = if hay_datos {
            anillo.siguiente()
        } else {
            (0, false)
        };
        let (dir_estado, ciclo_estado) = anillo.siguiente();

        // Si la reserva ha dado la vuelta al anillo, el TRB Link hay que
        // reescribirlo con el ciclo que el controlador va a comparar: el de la
        // vuelta que termina, no el de la que empieza. Se hace **antes** de
        // tocar la campana, y no hay carrera: el controlador solo puede llegar
        // al Link despues de consumir el ultimo TRB util, y para eso hace falta
        // la campana.
        if anillo.dio_la_vuelta {
            let mut link = trb_link(anillo.base, true);
            link.set_ciclo(!anillo.ciclo);
            ring::escribir_trb(anillo.direccion_link(), &link);
            mmio::clflush_rango(anillo.direccion_link(), 16);
        }

        // El tipo de etapa de datos que declara el setup (§6.4.1.2.2).
        let trt = if !hay_datos {
            TRT_SIN_DATOS
        } else if dir_in {
            TRT_DATOS_IN
        } else {
            TRT_DATOS_OUT
        };

        // Setup Stage: el paquete de 8 bytes va dentro del TRB (IDT). Linux no
        // le pone IOC: el evento de la tanda lo genera el estado.
        let mut trb_setup = trb_setup_stage(
            setup.bm_request_type,
            setup.b_request,
            setup.w_value,
            setup.w_index,
            setup.w_length,
            trt,
            false,
        );
        trb_setup.set_ciclo(ciclo_setup);
        ring::escribir_trb(dir_setup, &trb_setup);

        // Data Stage: la longitud va en el estado del TRB y la direccion en el
        // bit DIR. El bit ISP solo se pone en las entradas: es el que avisa de
        // un paquete corto, que es justo lo que hace falta para saber cuantos
        // bytes han llegado de verdad.
        if hay_datos {
            let mut trb_datos = trb_data_stage(buffer, pedido as u32, dir_in, dir_in, false, false);
            trb_datos.set_ciclo(ciclo_datos);
            ring::escribir_trb(dir_datos, &trb_datos);
        }

        // Status Stage: es el ultimo TRB de la tanda y el que genera el evento.
        // La etapa de estado es una entrada salvo que los datos hayan sido de
        // entrada: en ese caso confirma el host.
        let mut trb_estado = trb_status_stage(!(hay_datos && dir_in), false, true);
        trb_estado.set_ciclo(ciclo_estado);
        ring::escribir_trb(dir_estado, &trb_estado);

        // Que el controlador vea los TRB (y los datos, si van de salida) antes
        // de que suene la campana.
        mmio::clflush_rango(dir_setup, cuantos as usize * 16);
        if hay_datos {
            mmio::clflush_rango(buffer, pedido as usize);
        }
        mmio::mfence();

        self.regs
            .escribir_doorbell(hueco, VALOR_CAMPANA_EP0)
            .map_err(|_| ErrorEnum::Controlador)?;

        let transferidos = self.esperar_transferencia(hueco, pedido)?;

        // En una entrada, el dispositivo ha escrito en el buffer: hay que tirar
        // la copia que el procesador tenga en su cache antes de leerlo.
        if hay_datos && dir_in {
            mmio::clflush_rango(buffer, pedido as usize);
            mmio::mfence();
        }

        Ok(transferidos)
    }

    /// Espera el evento de transferencia de un hueco.
    ///
    /// Devuelve cuantos bytes se han transferido. Un **paquete corto** no es un
    /// error: el evento dice cuantos bytes faltaban, y la tanda sigue con el
    /// evento del TRB de estado, que es el que la cierra.
    fn esperar_transferencia(&mut self, hueco: u8, pedido: u16) -> Result<u16, ErrorEnum> {
        let memoria = self.memoria_eventos();
        let inicio = pit::ticks();
        let mut iteracion = 0usize;
        let mut transferidos: Option<u16> = None;

        loop {
            iteracion += 1;
            match self.anillo.siguiente(&memoria) {
                Some(trb) => {
                    self.anillo.avanzar();
                    self.contadores.eventos += 1;
                    self.devolver_puntero_eventos();

                    if TipoEvento::de_codigo(trb.tipo()) != TipoEvento::Transferencia {
                        self.contadores.desconocidos += 1;
                        continue;
                    }
                    if trb.slot_id() != hueco || trb.ep_id() != EP_ID_CONTROL {
                        // Es de otro hueco o de otro endpoint: no es nuestro.
                        self.contadores.desconocidos += 1;
                        continue;
                    }

                    let codigo = trb.codigo_finalizacion();
                    // El residual dice cuantos bytes no se han transferido. Se
                    // acota: el controlador podria decir cualquier cosa.
                    let residual = (trb.status() & RESIDUAL_MASCARA) as u16;
                    let hechos = pedido.saturating_sub(residual);

                    match codigo {
                        FINALIZACION_CORRECTA => {
                            return Ok(transferidos.unwrap_or(hechos));
                        }
                        FINALIZACION_PAQUETE_CORTO => {
                            // La tanda aun no ha terminado: falta el evento del
                            // TRB de estado. Se guarda lo que ha llegado.
                            transferidos = Some(hechos);
                        }
                        FINALIZACION_ERROR_STALL => return Err(ErrorEnum::Stall),
                        FINALIZACION_ERROR_TRANSACCION => return Err(ErrorEnum::ErrorTransaccion),
                        FINALIZACION_ERROR_BABBLE => return Err(ErrorEnum::Babble),
                        otro => return Err(ErrorEnum::CodigoDesconocido(otro)),
                    }
                }
                None => {
                    if iteracion >= MAX_ITERACIONES_TRANSFERENCIA {
                        return Err(ErrorEnum::SinRespuesta);
                    }
                    if pit::ticks().saturating_sub(inicio) >= PLAZO_TRANSFERENCIA {
                        return Err(ErrorEnum::SinRespuesta);
                    }
                    tarea::dormir(1);
                }
            }
        }
    }

    /// Rellena el contexto de entrada para el comando Address Device (§4.1.3).
    ///
    /// Son tres contextos de 32 bytes:
    ///
    /// - el de **control**, que dice que se anaden el de slot y el del endpoint
    ///   0 (y que no se quita ninguno);
    /// - el de **slot**, que dice donde esta el dispositivo (puerto raiz y
    ///   velocidad) y cuantos contextos de endpoint lleva;
    /// - el del **endpoint 0**, que dice como es su anillo de transferencia.
    fn escribir_contexto_entrada(
        &self,
        entrada: u64,
        base_anillo: u64,
        puerto: u8,
        velocidad: Velocidad,
        max_paquete: u16,
        topologia: &Topologia,
    ) {
        // Contexto de control: drop = 0, add = slot + endpoint 0.
        let mut control = InputControlContext::default();
        control.set_drop_flags(0);
        control.set_add_flags(AÑADIR_SLOT | AÑADIR_EP0);

        // Contexto de slot: la ruta (cero si cuelga de un puerto raiz), la
        // velocidad en la codificacion del controlador, el puerto raiz y un
        // solo contexto de endpoint (el 0). El bit de hub se pone despues,
        // cuando el descriptor dice que lo es.
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
        slot.set_context_entries(1);
        slot.set_root_hub_port(puerto);
        slot.set_max_exit_latency(0);
        slot.set_interrupter_target(0);

        // Contexto del endpoint 0: control, tres reintentos, el tamano de
        // paquete de su velocidad y el anillo que acaba de prepararse.
        let mut ep0 = EndpointContext::default();
        ep0.set_state_mult_pstreams_lsa_interval(
            EP_ESTADO_INVALIDO,
            0,
            0,
            false,
            intervalo_ep0(velocidad),
        );
        ep0.set_type_err_hid(EP_TCONTROL, 3, false);
        ep0.set_packet_burst(max_paquete, 1);
        ep0.set_tr_dequeue(base_anillo, true);
        // §6.2.3: para un endpoint de control, la longitud media de TRB es 8.
        ep0.set_avg_trb_len(8);
        ep0.set_max_esit(0);

        escribir_contexto(entrada, &control.d);
        escribir_contexto(entrada + 32, &slot.d);
        escribir_contexto(entrada + ring::DESPLAZAMIENTO_EP0_ENTRADA as u64, &ep0.d);
    }

    /// Corrige el tamano de paquete del endpoint 0 con Evaluate Context.
    ///
    /// Solo se rellena lo que el comando mira (§4.6.10): la marca del contexto
    /// de endpoint 0 y su tamano de paquete. El puntero al anillo no se toca:
    /// Evaluate Context no lo actualiza, y escribir otro valor en el contexto de
    /// entrada no cambia nada (`xhci_evaluate_slot` en QEMU solo lee el tamano
    /// de paquete).
    fn evaluar_max_paquete_ep0(&mut self, entrada: u64, max_paquete: u16) -> Result<(), ErrorEnum> {
        let mut control = InputControlContext::default();
        control.set_drop_flags(0);
        control.set_add_flags(AÑADIR_EP0);

        let mut ep0 = EndpointContext::default();
        ep0.set_state_mult_pstreams_lsa_interval(EP_ESTADO_INVALIDO, 0, 0, false, 0);
        ep0.set_type_err_hid(EP_TCONTROL, 3, false);
        ep0.set_packet_burst(max_paquete, 1);

        escribir_contexto(entrada, &control.d);
        escribir_contexto(entrada + ring::DESPLAZAMIENTO_EP0_ENTRADA as u64, &ep0.d);
        mmio::clflush_rango(entrada, ring::TAM_CTX_ENTRADA);
        mmio::mfence();

        // El comando necesita el numero de hueco, que es el dueno del contexto
        // de entrada. Se saca del propio contexto de salida: el primer dword
        // del contexto de slot no lo dice, pero el hueco es el que pidio la
        // enumeracion. Para no arrastrarlo por toda la funcion, se busca por la
        // direccion del contexto de entrada.
        let hueco = match self.hueco_de_entrada(entrada) {
            Some(h) => h,
            None => return Err(ErrorEnum::HuecoInvalido),
        };

        let evento = self
            .ejecutar_comando(trb_evaluate_context(entrada, hueco))
            .map_err(|_| ErrorEnum::Comando)?;
        if evento.codigo_finalizacion() != FINALIZACION_CORRECTA {
            return Err(ErrorEnum::ComandoRechazado(evento.codigo_finalizacion()));
        }
        Ok(())
    }

    /// El hueco al que pertenece un contexto de entrada.
    fn hueco_de_entrada(&self, entrada: u64) -> Option<u8> {
        for indice in 0..ring::MAX_HUECOS {
            if self.contextos.entrada(indice) == Some(entrada) {
                return Some(indice as u8 + 1);
            }
        }
        None
    }
}

/// Recorre los descriptores de una configuracion y devuelve las interfaces y
/// los endpoints, ya validados.
///
/// Se valida, ademas de lo que valida el nucleo USB:
///
/// - que cada descriptor de endpoint este dentro de una interfaz;
/// - que el numero de endpoints vistos cuadre con `bNumEndpoints`;
/// - que no haya dos endpoints con la misma direccion en una misma interfaz;
/// - que no haya dos descriptores de interfaz con el mismo numero **y** el
///   mismo ajuste alterno;
/// - que el numero de interfaces distintas cuadre con `bNumInterfaces`;
/// - que el descriptor de clase HID (0x21) de una interfaz HID quepa entero,
///   y se copia en la interfaz (de ahi sale la longitud del de informe).
fn recorrer_configuracion(
    bytes: &[u8],
    interfaces_declaradas: u8,
) -> Result<ContenidoConfig, ErrorEnum> {
    let mut contenido = ContenidoConfig::nuevo();
    let mut abierta: Option<usize> = None;
    let mut vistos: u8 = 0;
    let mut fallo: Option<ErrorEnum> = None;

    let recorrido = recorrer_descriptores(bytes, |offset, cabecera| {
        match cabecera.b_descriptor_type {
            t if t == TipoDescriptor::Interfaz as u8 => {
                // Cerrar la interfaz anterior y comprobar que declaro los
                // endpoints que han aparecido.
                if let Some(i) = abierta {
                    if contenido.interfaces[i].endpoints != vistos {
                        fallo = Some(ErrorEnum::DescriptorInvalido);
                        return false;
                    }
                }
                let desc = match parsear_interfaz(bytes, offset) {
                    Ok(d) => d,
                    Err(_) => {
                        fallo = Some(ErrorEnum::DescriptorInvalido);
                        return false;
                    }
                };
                // Interfaces repetidas: mismo numero y mismo ajuste alterno.
                for i in 0..contenido.num_interfaces as usize {
                    let otra = contenido.interfaces[i];
                    if otra.numero == desc.b_interface_number
                        && otra.alterna == desc.b_alternate_setting
                    {
                        fallo = Some(ErrorEnum::DescriptorInvalido);
                        return false;
                    }
                }
                if contenido.num_interfaces as usize >= MAX_INTERFACES_DISPOSITIVO {
                    fallo = Some(ErrorEnum::DescriptorInvalido);
                    return false;
                }
                let indice = contenido.num_interfaces as usize;
                contenido.interfaces[indice] = Interfaz {
                    numero: desc.b_interface_number,
                    alterna: desc.b_alternate_setting,
                    clase: desc.b_interface_class,
                    subclase: desc.b_interface_sub_class,
                    protocolo: desc.b_interface_protocol,
                    endpoints: desc.b_num_endpoints,
                    primer_endpoint: contenido.num_endpoints,
                    hid: [0; TAM_DESCRIPTOR_HID as usize],
                };
                contenido.num_interfaces += 1;
                abierta = Some(indice);
                vistos = 0;
            }
            t if t == TIPO_DESCRIPTOR_HID => {
                let Some(i) = abierta else {
                    // Un descriptor de clase suelto, sin interfaz: no se puede
                    // asociar a nada.
                    fallo = Some(ErrorEnum::DescriptorInvalido);
                    return false;
                };
                // En una interfaz que no es HID no pinta nada: se ignora en vez
                // de tumbar el dispositivo entero.
                if contenido.interfaces[i].clase == CLASE_HID {
                    let fin = match offset.checked_add(TAM_DESCRIPTOR_HID as usize) {
                        Some(fin) => fin,
                        None => {
                            fallo = Some(ErrorEnum::DescriptorInvalido);
                            return false;
                        }
                    };
                    if (cabecera.b_length as usize) < TAM_DESCRIPTOR_HID as usize
                        || fin > bytes.len()
                    {
                        fallo = Some(ErrorEnum::DescriptorInvalido);
                        return false;
                    }
                    contenido.interfaces[i]
                        .hid
                        .copy_from_slice(&bytes[offset..fin]);
                }
            }
            t if t == TipoDescriptor::Endpoint as u8 => {
                let Some(i) = abierta else {
                    // Un endpoint suelto, sin interfaz: no se puede asociar.
                    fallo = Some(ErrorEnum::DescriptorInvalido);
                    return false;
                };
                let desc = match parsear_endpoint(bytes, offset) {
                    Ok(d) => d,
                    Err(_) => {
                        fallo = Some(ErrorEnum::DescriptorInvalido);
                        return false;
                    }
                };
                let direccion = desc.b_endpoint_address;
                let numero = direccion & 0x0F;
                let tipo = desc.bm_attributes & 0x03;
                let max_paquete = desc.w_max_packet_size & 0x07FF;
                // El endpoint 0 no se declara en una configuracion, el tipo
                // control tampoco, y un paquete de cero bytes no existe.
                if numero == 0 || tipo == TIPO_CONTROL || max_paquete == 0 || max_paquete > 1024 {
                    fallo = Some(ErrorEnum::DescriptorInvalido);
                    return false;
                }
                // Dos endpoints con la misma direccion en la misma interfaz.
                let interfaz = contenido.interfaces[i];
                let inicio = interfaz.primer_endpoint as usize;
                let fin = (inicio + vistos as usize).min(contenido.endpoints.len());
                for e in &contenido.endpoints[inicio..fin] {
                    if e.direccion == direccion {
                        fallo = Some(ErrorEnum::DescriptorInvalido);
                        return false;
                    }
                }
                if contenido.num_endpoints as usize >= MAX_ENDPOINTS_DISPOSITIVO {
                    fallo = Some(ErrorEnum::DescriptorInvalido);
                    return false;
                }
                contenido.endpoints[contenido.num_endpoints as usize] = Endpoint {
                    direccion,
                    numero,
                    tipo,
                    max_paquete,
                    intervalo: desc.b_interval,
                };
                contenido.num_endpoints += 1;
                vistos = vistos.saturating_add(1);
            }
            _ => {}
        }
        true
    });
    let _ = recorrido;

    if let Some(e) = fallo {
        return Err(e);
    }
    if recorrido.is_err() {
        return Err(ErrorEnum::DescriptorInvalido);
    }
    // La ultima interfaz tambien tiene que cuadrar.
    if let Some(i) = abierta {
        if contenido.interfaces[i].endpoints != vistos {
            return Err(ErrorEnum::DescriptorInvalido);
        }
    }

    // El numero de interfaces distintas tiene que ser el que declara la
    // configuracion.
    let mut distintas = 0u8;
    for i in 0..contenido.num_interfaces as usize {
        let mut repetida = false;
        for j in 0..i {
            if contenido.interfaces[j].numero == contenido.interfaces[i].numero {
                repetida = true;
                break;
            }
        }
        if !repetida {
            distintas += 1;
        }
    }
    if distintas != interfaces_declaradas {
        return Err(ErrorEnum::DescriptorInvalido);
    }

    Ok(contenido)
}

/// Escribe un contexto (ocho palabras) en la memoria de DMA.
pub(super) fn escribir_contexto(direccion: u64, palabras: &[u32; 8]) {
    for (i, palabra) in palabras.iter().enumerate() {
        ring::escribir_u32(direccion + 4 * i as u64, *palabra);
    }
}

/// La velocidad del enlace en la codificacion del contexto de slot (§4.3.3).
///
/// SuperSpeedPlus se declara como SuperSpeed: la tabla de velocidades del
/// contexto de slot que usa rocky llega hasta SuperSpeed, igual que Linux
/// (`SLOT_SPEED_SS`), que tampoco distingue las dos.
pub(super) fn velocidad_contexto(velocidad: Velocidad) -> u8 {
    match velocidad {
        Velocidad::Baja => VELOCIDAD_LOW,
        Velocidad::Completa => VELOCIDAD_FULL,
        Velocidad::Alta => VELOCIDAD_HIGH,
        Velocidad::Super | Velocidad::SuperPlus => VELOCIDAD_SUPER,
        Velocidad::Indefinida => VELOCIDAD_FULL,
    }
}

/// El tamano maximo de paquete del endpoint 0, segun la velocidad (§4.7.2.5).
///
/// Es el valor que se programa **antes** de saber lo que declara el
/// dispositivo: SuperSpeed 512, alta y completa 64, baja 8.
fn max_paquete_ep0(velocidad: Velocidad) -> u16 {
    match velocidad {
        Velocidad::Baja => 8,
        Velocidad::Super | Velocidad::SuperPlus => 512,
        Velocidad::Completa | Velocidad::Alta | Velocidad::Indefinida => 64,
    }
}

/// El intervalo del endpoint 0, segun la velocidad.
///
/// Para una velocidad baja la especificacion pide 1; en las demas, 0.
fn intervalo_ep0(velocidad: Velocidad) -> u8 {
    match velocidad {
        Velocidad::Baja => 1,
        _ => 0,
    }
}

/// Traduce el `bMaxPacketSize0` del descriptor de dispositivo (§9.6.1) al
/// tamano real, si es valido para la velocidad del enlace.
///
/// - Velocidad completa y baja: 8, 16, 32 o 64 bytes, tal cual.
/// - Alta velocidad: 64 (la especificacion solo admite 64 para el endpoint 0).
/// - SuperSpeed y SuperSpeedPlus: 9, que son 2^9 = 512 bytes.
fn tamano_ep0_declarado(declarado: u8, velocidad: Velocidad) -> Option<u16> {
    match velocidad {
        Velocidad::Super | Velocidad::SuperPlus => {
            if declarado == 9 {
                Some(512)
            } else {
                None
            }
        }
        Velocidad::Alta => {
            if declarado == 64 {
                Some(64)
            } else {
                None
            }
        }
        Velocidad::Baja | Velocidad::Completa => match declarado {
            8 | 16 | 32 | 64 => Some(declarado as u16),
            _ => None,
        },
        Velocidad::Indefinida => None,
    }
}
