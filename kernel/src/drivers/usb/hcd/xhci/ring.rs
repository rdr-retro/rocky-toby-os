//! Las estructuras de memoria que un controlador xHCI necesita.
//!
//! Un controlador xHCI no tiene memoria propia: el kernel le tiene que
//! preparar cinco estructuras en RAM y darle las direcciones fisicas.
//!
//! | estructura | para que sirve | tamaño |
//! | --- | --- | --- |
//! | DCBAA | array de contextos base: un puntero por hueco de dispositivo | `MaxSlots + 1` entradas de 8 bytes |
//! | anillo de comandos | donde el kernel pondria los comandos | `TRB_COMANDOS` TRBs |
//! | anillo de eventos | donde el controlador deja los eventos | `TRB_EVENTOS` TRBs |
//! | ERST | tabla de segmentos del anillo de eventos | 1 entrada de 16 bytes |
//! | scratchpads |Buffers de trabajo que pide el controlador | lo que diga `HCSPARAMS2` |
//!
//! Las tres primeras tienen requisitos de alineacion y de contenido que
//! dependen de la especificacion (§6.1, §6.5, §4.20), asi que estan aqui y no
//! en el driver.
//!
//! Aparte hay dos regiones mas, que solo hacen falta cuando se enumeran
//! dispositivos: la de **contextos** (contexto de salida, contexto de entrada
//! y anillos de transferencia del endpoint 0) y la de **buffers** de
//! transferencia.
//!
//! Referencia: Intel, "eXtensible Host Controller Interface for Universal
//! Serial Bus (xHCI) Requirements Specification", Rev 1.2, §4.20, §6.1 y §6.5.
//!
//! Codigo propio de rocky.

use crate::drivers::usb::dma::MarcosDelKernel;
use rockyhw::dma::{DmaRegion, ErrorDma, LIMITE_4GIB};
use rockyhw::xhci::caps::Capacidades;
use rockyhw::xhci::ring::{erst_entrada, Trb};

/// Numero de TRBs del anillo de comandos.
///
/// La especificacion pide entre 16 y 1024 TRBs por anillo (§4.9.6). Con 256
/// caben de sobra para la fase 2 (que aun no emite comandos) y ocupa una
/// pagina.
pub const TRB_COMANDOS: u16 = 256;

/// Numero de TRBs del anillo de eventos.
///
/// La especificacion pide entre 16 y 4096 TRBs (§6.5). Con 256 caben de
/// sobra para el sondeo de la fase 2 y ocupa una pagina.
pub const TRB_EVENTOS: u16 = 256;

/// Numero de huecos de dispositivo que rocky gestiona a la vez.
///
/// Es un tope de rocky, no del controlador: el limite real es `MaxSlots` de
/// las capacidades. Ocho huecos bastan de sobra para un teclado, un raton y
/// un pendrive, y acota la memoria reservada.
pub const MAX_HUECOS: usize = 8;

/// Bytes de un contexto de dispositivo de salida.
///
/// Lo que el controlador lee: un contexto de slot (32 bytes) seguido de los
/// contextos de endpoint (32 bytes cada uno). Con 31 endpoints son 1024
/// bytes, que es ademas multiplo de 64 como pide §4.1.2.
pub const TAM_CTX_SALIDA: usize = 1024;

/// Desplazamiento del contexto de endpoint 0 dentro del contexto de entrada.
///
/// El contexto de control (32 bytes) y el de slot (32 bytes) van primero; el
/// primer contexto de endpoint es el del endpoint 0 (§4.1.3).
pub const DESPLAZAMIENTO_EP0_ENTRADA: usize = 64;

/// Bytes de un contexto de entrada.
///
/// Lo que el software escribe para el comando Address Device: un contexto de
/// control (32 bytes), un contexto de slot (32 bytes) y los contextos de
/// endpoint (32 bytes cada uno) (§4.1.3). Se reserva para 31 endpoints desde
/// el desplazamiento del endpoint 0 y se redondea a 64 bytes, como requiere
/// §4.1.2: 1056 bytes de contenido, 1088 bytes reservados.
pub const TAM_CTX_ENTRADA: usize = (DESPLAZAMIENTO_EP0_ENTRADA + 31 * 32 + 63) & !63;

/// Numero de TRBs del anillo de transferencia del endpoint 0.
///
/// El endpoint 0 es el de control: por el pasan todas las peticiones de
/// configuracion del dispositivo (§4.7.3).
///
/// Con 64 TRBs caben 63 utiles (el ultimo es el TRB Link) y 21 tandas de
/// control completas (setup, datos y estado): de sobra para enumerar un
/// dispositivo, que gasta tres.
pub const TRB_EP0: u16 = 64;

/// Bytes de un anillo de transferencia del endpoint 0.
pub const TAM_ANILLO_EP0: usize = TRB_EP0 as usize * 16;

/// Maximo de endpoints de interrupcion activos a la vez.
///
/// Es el mismo tope que el de huecos: un dispositivo de esta fase aporta como
/// mucho un endpoint de interrupcion (el del teclado o el del raton).
pub const MAX_EP_INTERRUPCION: usize = MAX_HUECOS;

/// TRBs de cada anillo de interrupcion.
///
/// Con 32 caben 31 utiles y el TRB Link final. Como solo hay una transferencia
/// pendiente por endpoint, el anillo da para 31 informes antes de dar la
/// vuelta: de sobra, porque cada informe se repone en cuanto llega.
pub const TRB_INTERRUPCION: u16 = 32;

/// Bytes de un anillo de interrupcion (multiplo de 64, como pide §4.1.2).
pub const TAM_ANILLO_INTERRUPCION: usize = TRB_INTERRUPCION as usize * 16;

/// Bytes del buffer de un endpoint de interrupcion.
///
/// Un informe boot de teclado son 8 bytes y uno de raton 3, pero el endpoint
/// puede declarar un paquete mayor: se reserva espacio para el mayor de los
/// dos sin pasarse (64 bytes siguen siendo un solo cache line por endpoint).
pub const TAM_BUFFER_INTERRUPCION: usize = 64;

/// Bytes de la region de interrupcion: los anillos primero, alineados a
/// [`TAM_ANILLO_INTERRUPCION`], y detras los buffers, alineados a 64.
pub const TAM_INTERRUPCION: usize =
    MAX_EP_INTERRUPCION * (TAM_ANILLO_INTERRUPCION + TAM_BUFFER_INTERRUPCION);

/// Bytes de la region de contextos.
///
/// Por cada hueco: el contexto de salida, el contexto de entrada y el anillo
/// de transferencia del endpoint 0.
pub const TAM_CONTEXTOS: usize = MAX_HUECOS * (TAM_CTX_SALIDA + TAM_CTX_ENTRADA + TAM_ANILLO_EP0);

/// Tamano de un TRB en bytes (§4.11.1).
const TAM_TRB: u64 = 16;

/// Tamano de marco de pagina.
const MARCO: usize = 4096;

/// La region donde viven los contextos de los dispositivos.
///
/// La region se reparte en tres zonas, todas alineadas a 64 bytes:
///
/// ```text
/// 0x0000  contextos de salida  (device contexts): MAX_HUECOS * 1024
/// 0x2000  contextos de entrada  (input contexts):  MAX_HUECOS * 1088
/// 0x4900  anillos del endpoint 0:                  MAX_HUECOS * 1024
/// ```
///
/// El contexto de salida es el que el controlador lee (§4.1.2); el de
/// entrada es el que el software rellena para el comando Address Device
/// (§4.1.3); y el anillo del endpoint 0 es donde van los TRB de las
/// transferencias de control (§4.7.3).
#[derive(Clone, Copy, Debug)]
pub struct Contextos {
    base: u64,
    bytes: u64,
}

impl Contextos {
    /// Prepara la vista de la region de contextos.
    pub fn nueva(base: u64, bytes: u64) -> Contextos {
        Contextos { base, bytes }
    }

    /// La direccion del contexto de **salida** de un hueco.
    ///
    /// Devuelve `None` si el hueco no cabe en la region: un hueco fuera de
    /// rango significaria que el controlador leeria memoria ajena.
    pub fn salida(&self, hueco: usize) -> Option<u64> {
        self.despues(hueco, TAM_CTX_SALIDA, 0)
    }

    /// La direccion del contexto de **entrada** de un hueco.
    pub fn entrada(&self, hueco: usize) -> Option<u64> {
        self.despues(hueco, TAM_CTX_ENTRADA, MAX_HUECOS * TAM_CTX_SALIDA)
    }

    /// La direccion del anillo de transferencia del endpoint 0 de un hueco.
    pub fn anillo_ep0(&self, hueco: usize) -> Option<u64> {
        self.despues(
            hueco,
            TAM_ANILLO_EP0,
            MAX_HUECOS * (TAM_CTX_SALIDA + TAM_CTX_ENTRADA),
        )
    }

    /// Comprueba que todos los contextos gestionados caben en DMA y respetan
    /// la alineacion que necesita xHCI antes de publicar la region al hardware.
    pub fn es_valido(&self) -> bool {
        if self.base % 64 != 0
            || self.base.checked_add(self.bytes).is_none()
            || self.bytes < TAM_CONTEXTOS as u64
        {
            return false;
        }

        for hueco in 0..MAX_HUECOS {
            let (Some(salida), Some(entrada), Some(ep0)) = (
                self.salida(hueco),
                self.entrada(hueco),
                self.anillo_ep0(hueco),
            ) else {
                return false;
            };
            if salida % 64 != 0 || entrada % 64 != 0 || ep0 % 64 != 0 {
                return false;
            }
        }
        true
    }

    /// Comprueba que una zona concreta cabe en la region.
    fn despues(&self, hueco: usize, tam: usize, desplazamiento: usize) -> Option<u64> {
        if hueco >= MAX_HUECOS {
            return None;
        }
        let inicio = desplazamiento + hueco * tam;
        let fin = inicio.checked_add(tam)?;
        if fin > self.bytes as usize {
            return None;
        }
        Some(self.base + inicio as u64)
    }
}

/// La region de los endpoints de interrupcion: un anillo y un buffer por
/// endpoint activo.
///
/// ```text
/// 0x0000  anillos (31 TRB utiles + Link):  MAX_EP_INTERRUPCION * 512
/// 0x1000  buffers de los informes:         MAX_EP_INTERRUPCION * 64
/// ```
///
/// Cada anillo queda alineado a 64 bytes (512 es multiplo) y cada buffer a 64,
/// que es lo que pide §4.1.2 para las estructuras que lee el controlador.
#[derive(Clone, Copy, Debug)]
pub struct Interrupciones {
    base: u64,
    bytes: u64,
}

impl Interrupciones {
    /// Prepara la vista de la region de interrupcion.
    pub fn nueva(base: u64, bytes: u64) -> Interrupciones {
        Interrupciones { base, bytes }
    }

    /// El anillo de transferencia del endpoint `indice`.
    pub fn anillo(&self, indice: usize) -> Option<u64> {
        if indice >= MAX_EP_INTERRUPCION {
            return None;
        }
        let inicio = indice.checked_mul(TAM_ANILLO_INTERRUPCION)?;
        if inicio.checked_add(TAM_ANILLO_INTERRUPCION)? > self.bytes as usize {
            return None;
        }
        Some(self.base + inicio as u64)
    }

    /// El buffer donde el controlador deja el informe del endpoint `indice`.
    pub fn buffer(&self, indice: usize) -> Option<u64> {
        if indice >= MAX_EP_INTERRUPCION {
            return None;
        }
        let inicio = MAX_EP_INTERRUPCION * TAM_ANILLO_INTERRUPCION
            + indice.checked_mul(TAM_BUFFER_INTERRUPCION)?;
        if inicio.checked_add(TAM_BUFFER_INTERRUPCION)? > self.bytes as usize {
            return None;
        }
        Some(self.base + inicio as u64)
    }

    /// Comprueba que todos los anillos y buffers caben y estan alineados.
    pub fn es_valido(&self) -> bool {
        if self.base % 64 != 0
            || self.base.checked_add(self.bytes).is_none()
            || self.bytes < TAM_INTERRUPCION as u64
        {
            return false;
        }
        for indice in 0..MAX_EP_INTERRUPCION {
            let (Some(anillo), Some(buffer)) = (self.anillo(indice), self.buffer(indice)) else {
                return false;
            };
            if anillo % 64 != 0 || buffer % 64 != 0 {
                return false;
            }
        }
        true
    }
}

/// Las estructuras de memoria de un controlador xHCI.
pub struct Estructuras {
    /// El array de contextos base (`DCBAA`).
    pub dcbaa: DmaRegion,
    /// El anillo de comandos.
    pub comandos: DmaRegion,
    /// El anillo de eventos.
    pub eventos: DmaRegion,
    /// La tabla de segmentos del anillo de eventos (`ERST`).
    pub erst: DmaRegion,
    /// El array de scratchpads, si el controlador pide scratchpads.
    pub array_scratchpad: Option<DmaRegion>,
    /// Los buffers de scratchpad, si el controlador pide scratchpads.
    pub scratchpads: Option<DmaRegion>,
    /// Contextos de dispositivo, contextos de entrada y anillos de
    /// transferencia del endpoint 0.
    pub contextos: DmaRegion,
    /// Buffers de transferencia pequenos.
    pub buffers: DmaRegion,
    /// Anillos y buffers de los endpoints de interrupcion.
    pub interrupcion: DmaRegion,
}

impl Estructuras {
    /// Reserva las estructuras que pide el controlador.
    ///
    /// Si algo falla, las estructuras ya reservadas vuelven al gestor de
    /// memoria: no queda memoria perdida por un fallo a medias.
    pub fn reservar(caps: &Capacidades) -> Result<Estructuras, ErrorDma> {
        // La DCBAA: `MaxSlots + 1` entradas de 8 bytes, alineada a 64 bytes
        // (§6.1). Se alinea a un marco entero, que es mas de lo que pide.
        let entradas = caps.max_slots as usize + 1;
        let dcbaa = DmaRegion::nueva(
            &mut MarcosDelKernel,
            marcos_de(entradas * 8),
            MARCO,
            LIMITE_4GIB,
        )?;

        // El anillo de comandos: `TRB_COMANDOS` TRBs, alineado a 64 bytes.
        let mut comandos = match DmaRegion::nueva(
            &mut MarcosDelKernel,
            marcos_de(TRB_COMANDOS as usize * TAM_TRB as usize),
            MARCO,
            LIMITE_4GIB,
        ) {
            Ok(r) => r,
            Err(e) => return Err(suelta(dcbaa, e)),
        };

        // El anillo de eventos: `TRB_EVENTOS` TRBs, alineado a 64 bytes.
        let mut eventos = match DmaRegion::nueva(
            &mut MarcosDelKernel,
            marcos_de(TRB_EVENTOS as usize * TAM_TRB as usize),
            MARCO,
            LIMITE_4GIB,
        ) {
            Ok(r) => r,
            Err(e) => return Err(suelta(comandos, e)),
        };

        // La tabla de segmentos: una entrada de 16 bytes, alineada a 64 bytes
        // (§6.5).
        let mut erst =
            match DmaRegion::nueva(&mut MarcosDelKernel, marcos_de(16), MARCO, LIMITE_4GIB) {
                Ok(r) => r,
                Err(e) => return Err(suelta(eventos, e)),
            };

        // Los scratchpads, si el controlador los pide (§4.20): un array de
        // punteros (alineado a tamano de pagina) y los buffers a los que
        // apuntan (cada uno de un tamano de pagina).
        let (array_scratchpad, scratchpads) = if caps.max_scratchpads > 0 {
            let n = caps.max_scratchpads as usize;
            let pagina = caps.tam_pagina();
            let array =
                match DmaRegion::nueva(&mut MarcosDelKernel, marcos_de(n * 8), pagina, LIMITE_4GIB)
                {
                    Ok(r) => r,
                    Err(e) => return Err(suelta(erst, e)),
                };
            let buffers = match DmaRegion::nueva(
                &mut MarcosDelKernel,
                marcos_de(n * pagina),
                pagina,
                LIMITE_4GIB,
            ) {
                Ok(r) => r,
                Err(e) => return Err(suelta(array, e)),
            };
            (Some(array), Some(buffers))
        } else {
            (None, None)
        };

        // Contextos y buffers para enumeración (acotados)
        let mut contextos = match DmaRegion::nueva(
            &mut MarcosDelKernel,
            marcos_de(TAM_CONTEXTOS),
            MARCO,
            LIMITE_4GIB,
        ) {
            Ok(r) => r,
            Err(e) => {
                // liberar lo previo
                let _ = comandos.liberar(&mut MarcosDelKernel);
                let _ = eventos.liberar(&mut MarcosDelKernel);
                let _ = erst.liberar(&mut MarcosDelKernel);
                if let Some(mut a) = array_scratchpad {
                    let _ = a.liberar(&mut MarcosDelKernel);
                }
                if let Some(mut s) = scratchpads {
                    let _ = s.liberar(&mut MarcosDelKernel);
                }
                return Err(e);
            }
        };
        let mut buffers = match DmaRegion::nueva(
            &mut MarcosDelKernel,
            marcos_de(64 * 1024),
            MARCO,
            LIMITE_4GIB,
        ) {
            Ok(r) => r,
            Err(e) => {
                let _ = contextos.liberar(&mut MarcosDelKernel);
                let _ = comandos.liberar(&mut MarcosDelKernel);
                let _ = eventos.liberar(&mut MarcosDelKernel);
                let _ = erst.liberar(&mut MarcosDelKernel);
                if let Some(mut a) = array_scratchpad {
                    let _ = a.liberar(&mut MarcosDelKernel);
                }
                if let Some(mut s) = scratchpads {
                    let _ = s.liberar(&mut MarcosDelKernel);
                }
                return Err(e);
            }
        };

        // Los anillos y buffers de los endpoints de interrupcion. Son dos
        // paginas: ocho anillos de 512 bytes y ocho buffers de 64.
        let interrupcion = match DmaRegion::nueva(
            &mut MarcosDelKernel,
            marcos_de(TAM_INTERRUPCION),
            MARCO,
            LIMITE_4GIB,
        ) {
            Ok(r) => r,
            Err(e) => {
                let _ = buffers.liberar(&mut MarcosDelKernel);
                let _ = contextos.liberar(&mut MarcosDelKernel);
                let _ = comandos.liberar(&mut MarcosDelKernel);
                let _ = eventos.liberar(&mut MarcosDelKernel);
                let _ = erst.liberar(&mut MarcosDelKernel);
                if let Some(mut a) = array_scratchpad {
                    let _ = a.liberar(&mut MarcosDelKernel);
                }
                if let Some(mut s) = scratchpads {
                    let _ = s.liberar(&mut MarcosDelKernel);
                }
                return Err(e);
            }
        };

        Ok(Estructuras {
            dcbaa,
            comandos,
            eventos,
            erst,
            array_scratchpad,
            scratchpads,
            contextos,
            buffers,
            interrupcion,
        })
    }

    /// Prepara el contenido de las estructuras.
    ///
    /// - La `DCBAA`: la entrada 0 es el puntero al array de scratchpads y las
    ///   demas son los contextos de dispositivo, que en esta fase no existen
    ///   (§6.1).
    /// - El array de scratchpads: un puntero por buffer (§4.20).
    /// - La `ERST`: una entrada con el segmento de eventos y su numero de
    ///   TRBs (§6.5).
    /// - El anillo de comandos: se deja a cero. En esta fase no se emite
    ///   ningun comando, y un anillo parado no se lee nunca.
    /// - Los contextos y los buffers: a cero. Las paginas del gestor de
    ///   memoria pueden venir sucias de una reserva anterior, y un contexto de
    ///   dispositivo con basura haria que el controlador leiese un endpoint
    ///   que no existe.
    pub fn inicializar(&mut self, caps: &Capacidades) {
        // La DCBAA.
        let base_dcbaa = self.dcbaa.base() as u64;
        let puntero_scratchpad = match &self.array_scratchpad {
            Some(a) => a.base() as u64,
            None => 0,
        };
        escribir_u64(base_dcbaa, puntero_scratchpad);
        for i in 1..=caps.max_slots as u64 {
            escribir_u64(base_dcbaa + i * 8, 0);
        }

        // El array de scratchpads: un puntero por buffer, cada uno alineado a
        // un tamano de pagina.
        if let (Some(array), Some(buffers)) = (&self.array_scratchpad, &self.scratchpads) {
            let base_array = array.base() as u64;
            let base_buffers = buffers.base() as u64;
            let pagina = caps.tam_pagina() as u64;
            for i in 0..caps.max_scratchpads as u64 {
                escribir_u64(base_array + i * 8, base_buffers + i * pagina);
            }
        }

        // La tabla de segmentos del anillo de eventos.
        let entrada = erst_entrada(self.eventos.base() as u64, TRB_EVENTOS);
        let base_erst = self.erst.base() as u64;
        for (i, valor) in entrada.iter().enumerate() {
            escribir_u32(base_erst + 4 * i as u64, *valor);
        }

        // Los contextos y los buffers, a cero.
        poner_a_cero(&self.contextos);
        poner_a_cero(&self.buffers);
        poner_a_cero(&self.interrupcion);
    }
}

/// Pone a cero una region de DMA.
///
/// La region es memoria recien reservada, pero el gestor de memoria puede
/// devolver paginas que tenia ocupadas antes: un contexto de dispositivo con
// bytes sucios haria que el controlador creyera que hay mas endpoints de los
/// que hay.
fn poner_a_cero(region: &DmaRegion) {
    let base = region.base() as *mut u8;
    // Seguro: `base` apunta a `region.bytes()` bytes reservados por el
    // kernel, y la region no se suelta hasta que se libere.
    unsafe { core::ptr::write_bytes(base, 0, region.bytes()) };
}

/// Devuelve una region al gestor de memoria y devuelve el error original.
///
/// Se usa cuando una reserva posterior falla: la region anterior no se
/// desperdicia.
fn suelta(mut region: DmaRegion, error: ErrorDma) -> ErrorDma {
    let _ = region.liberar(&mut MarcosDelKernel);
    error
}

/// Marcos que hacen falta para `bytes` bytes.
fn marcos_de(bytes: usize) -> usize {
    (bytes + MARCO - 1) / MARCO
}

/// Escribe un dword en la memoria de DMA.
///
/// El arranque mapea por identidad los primeros 4 GiB, asi que la direccion
/// fisica de la region sirve tambien de direccion virtual para el kernel.
pub(crate) fn escribir_u32(dir: u64, valor: u32) {
    // Seguro: la direccion viene de una region reservada por el kernel y el
    // campo esta dentro de ella.
    unsafe { core::ptr::write_volatile(dir as *mut u32, valor) }
}

/// Escribe un qword en la memoria de DMA.
pub(crate) fn escribir_u64(dir: u64, valor: u64) {
    // Seguro: igual que en `escribir_u32`.
    unsafe { core::ptr::write_volatile(dir as *mut u64, valor) }
}

/// Lee un dword de la memoria de DMA.
pub(crate) fn leer_u32(dir: u64) -> u32 {
    // Seguro: la direccion viene de una region reservada por el kernel.
    unsafe { core::ptr::read_volatile(dir as *const u32) }
}

/// Pone a cero una zona de memoria de DMA.
///
/// Se usa al reutilizar el anillo de un hueco: los TRB que dejo el dispositivo
/// anterior no pueden quedarse ahi, porque el controlador nuevo los leeria como
/// si fueran suyos.
pub(crate) fn borrar(dir: u64, bytes: usize) {
    // Seguro: la direccion y el tamano vienen de una region reservada por el
    // kernel.
    unsafe { core::ptr::write_bytes(dir as *mut u8, 0, bytes) };
}

/// Escribe un TRB en la memoria de DMA.
///
/// El TRB son cuatro dwords seguidos (§4.11.1). Se escriben con
/// `write_volatile` para que el compilador no los reordene ni los fusione.
pub fn escribir_trb(dir: u64, trb: &Trb) {
    for (i, palabra) in trb.palabras.iter().enumerate() {
        escribir_u32(dir + 4 * i as u64, *palabra);
    }
}

/// Lee un TRB de la memoria de DMA.
pub fn leer_trb(dir: u64) -> Trb {
    let mut palabras = [0u32; 4];
    for (i, palabra) in palabras.iter_mut().enumerate() {
        // Seguro: la direccion viene de una region reservada por el kernel.
        *palabra = unsafe { core::ptr::read_volatile((dir + 4 * i as u64) as *const u32) };
    }
    Trb::nueva(palabras)
}
