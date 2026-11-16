//! Entrada de teclado: una **unica cola de eventos** que llena la tarea de
//! fondo del USB, mas el sondeo directo del controlador PS/2 y del puerto serie
//! (util para probar rocky de forma automatica con `qemu -serial stdio`).
//!
//! Los teclados USB se encolan como eventos neutrales ([`Evento`]: tecla
//! fisica, extendida y pulsada/soltada) y la distribucion se aplica al drenar,
//! en el mismo orden en que llegaron. PS/2 y serie se leen del hardware en ese
//! mismo momento, asi que no hay una segunda cola de eventos que pueda
//! adelantarse a la primera. [`poll_tecla`] es el unico adaptador de todas las
//! fuentes; [`poll_event`] es el adaptador de compatibilidad que solo entrega
//! caracteres.
//!
//! El teclado trabaja en el **juego de barridos 1** (el controlador 8042
//! traduce del 2), y la distribucion es la **espanola (es_ES)**: la tecla de la
//! derecha de la P produce los acentos muertos, la de la L la ene, la fila
//! numerica da `! " · $ % & / ( ) =` y hay teclas dedicadas de `¡ ¿ º ª`.
//!
//! Los caracteres se devuelven en **CP437**, que es el juego con el que la VGA
//! dibuja tanto en modo texto como con la fuente de la GUI. Los acentos en
//! mayuscula no existen en el CP437 de la ROM y se componen al arrancar (ver
//! [`crate::bios::get_font`]).

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use crate::cpu::inb;
use crate::serial::Serial;
use crate::sync::SyncCell;
use rockyhw::cola::Cola;

static SHIFT: AtomicBool = AtomicBool::new(false);
/// Control izquierdo o derecho.
static CONTROL: AtomicBool = AtomicBool::new(false);
/// Bloq Mayus.
static MAYUS: AtomicBool = AtomicBool::new(false);
/// Bloq Num.
static NUM: AtomicBool = AtomicBool::new(false);
/// Bloq Despl (solo lleva la luz al dia; rocky no desplaza nada).
static SCROLL: AtomicBool = AtomicBool::new(false);
/// AltGr (la pulsacion es 0xE0 0x38).
static ALTGR: AtomicBool = AtomicBool::new(false);
static EXTENDED: AtomicBool = AtomicBool::new(false);
/// Bytes que quedan por saltar tras un 0xE1 (la cadena larga de la tecla Pausa).
static E1: AtomicU8 = AtomicU8::new(0);
/// Tecla muerta esperando una letra (`0` = ninguna); su valor es uno de los
/// tipos `MUERTE_*`.
static MUERTA: AtomicU8 = AtomicU8::new(0);
/// Caracter pendiente de salida cuando una tecla muerta no combina con nada.
static PEND: AtomicU8 = AtomicU8::new(0);

/// Bufer circular que llena la IRQ4 del puerto serie.
const RING_SIZE: usize = 512;

struct Ring {
    buf: [u8; RING_SIZE],
    head: usize,
    tail: usize,
}

impl Ring {
    const fn new() -> Self {
        Ring {
            buf: [0; RING_SIZE],
            head: 0,
            tail: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        let next = (self.head + 1) % RING_SIZE;
        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
    }

    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            None
        } else {
            let byte = self.buf[self.tail];
            self.tail = (self.tail + 1) % RING_SIZE;
            Some(byte)
        }
    }
}

static RING: SyncCell<Ring> = SyncCell::new(Ring::new());

/// Algunos comandos PS/2 llegan mientras se inicializa el raton. Conservamos
/// las teclas que comparten el búfer del 8042 para que inicializar un
/// dispositivo no consuma la entrada destinada a Kira.
const PS2_PENDING_SIZE: usize = 256;

struct PendingScancodes {
    buf: [u8; PS2_PENDING_SIZE],
    head: usize,
    tail: usize,
}

impl PendingScancodes {
    const fn new() -> Self {
        Self {
            buf: [0; PS2_PENDING_SIZE],
            head: 0,
            tail: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        let next = (self.head + 1) % PS2_PENDING_SIZE;
        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
    }

    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            None
        } else {
            let byte = self.buf[self.tail];
            self.tail = (self.tail + 1) % PS2_PENDING_SIZE;
            Some(byte)
        }
    }
}

static PS2_PENDING: SyncCell<PendingScancodes> = SyncCell::new(PendingScancodes::new());

/// Un evento de teclado **neutral**: lo que produce el dispositivo, todavia
/// sin distribucion ni teclas muertas.
///
/// La tecla va como posicion fisica en el juego de barridos 1, sin el prefijo
/// `0xE0` (que pasa a [`Evento::extendido`]) y sin el bit `0x80` (que pasa a
/// [`Evento::pulsada`]). La traduccion a caracter se hace al **drenar**, en el
/// mismo orden en que llegaron los eventos: asi un Shift sigue puesto cuando
/// llega su letra aunque el consumidor tarde en pedirla.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Evento {
    /// Codigo de juego 1, sin `0x80` y sin el `0xE0` de las extendidas.
    pub codigo: u8,
    /// La tecla es extendida (su flujo original llevaba delante `0xE0`).
    pub extendido: bool,
    /// Se pulsa (`true`) o se suelta (`false`).
    pub pulsada: bool,
}

impl Evento {
    /// Valor de relleno del almacenamiento de la cola; nunca se entrega.
    pub const VACIO: Evento = Evento {
        codigo: 0,
        extendido: false,
        pulsada: false,
    };
}

/// Un evento tal y como se guarda en la cola: con el tic en que se encolo.
///
/// El tic (10 ms) no es parte del evento: sirve para medir la latencia entre
/// la tarea de fondo del xHCI y el consumidor, que es lo que pide el plan.
#[derive(Clone, Copy)]
struct Encolado {
    evento: Evento,
    tick: u32,
}

impl Encolado {
    const VACIO: Encolado = Encolado {
        evento: Evento::VACIO,
        tick: 0,
    };
}

/// Eventos de teclado HID que se pueden acumular a la espera de consumidor.
///
/// Un informe de arranque no llega a 32 eventos; con 64 caben dos informes
/// enteros de margen para que un repintado lento no haga perder pulsaciones.
const HID_CAPACIDAD: usize = 64;

/// Tope de eventos que puede traer un solo informe; el informe entero se
/// encola de una vez y este limite mantiene el bufer temporal en la pila.
const HID_MAX_INFORME: usize = 32;

/// La **unica** cola de eventos del sistema.
///
/// La llena la tarea de fondo del xHCI y la vacia quien pide la entrada. PS/2
/// y el puerto serie no tienen cola de eventos: sus bytes se leen del hardware
/// en el momento de drenar, y [`poll_tecla`] es el unico adaptador de todas las
/// fuentes, asi que no hay dos colas que puedan desordenarse entre si.
static HID_PENDIENTE: SyncCell<Cola<Encolado, HID_CAPACIDAD>> =
    SyncCell::new(Cola::nueva(Encolado::VACIO));
/// Informes que no entraron por no caber en la cola. Un teclado que pierde
/// pulsaciones se nota, y el contador lo dice sin mentir.
static HID_DESCARTES: AtomicU32 = AtomicU32::new(0);
/// Eventos de tecla publicados por el teclado USB.
static HID_PUBLICADOS: AtomicU32 = AtomicU32::new(0);
/// Ocupacion maxima vista de la cola, en eventos.
static HID_COLA_MAX: AtomicU32 = AtomicU32::new(0);
/// Latencia maxima observada, en tics del PIT (10 ms).
static HID_LATENCIA_MAX: AtomicU32 = AtomicU32::new(0);

/// Encola los eventos de un informe de teclado HID.
///
/// Es **todo o nada** por informe: o entran todos sus eventos o no entra
/// ninguno. Un informe partido dejaria, por ejemplo, la liberacion de Shift
/// fuera y el consumidor se quedaria con el modificador pegado. Si no caben,
/// se descartan y se cuenta.
pub fn publicar_hid(eventos: &[Evento]) {
    if eventos.is_empty() {
        return;
    }
    // Un informe de arranque no llega ni de lejos; el tope esta para que la
    // pila de la tarea de fondo no dependa de lo que diga el dispositivo.
    if eventos.len() > HID_MAX_INFORME {
        HID_DESCARTES.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let tick = crate::pit::ticks() as u32;
    let mut encolados = [Encolado::VACIO; HID_MAX_INFORME];
    for (hueco, evento) in encolados.iter_mut().zip(eventos) {
        *hueco = Encolado {
            evento: *evento,
            tick,
        };
    }
    let encolados = &encolados[..eventos.len()];
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            let cola = &mut *HID_PENDIENTE.get();
            if !cola.empujar_todo(encolados) {
                HID_DESCARTES.fetch_add(1, Ordering::Relaxed);
                return;
            }
            HID_PUBLICADOS.fetch_add(eventos.len() as u32, Ordering::Relaxed);
            HID_COLA_MAX.fetch_max(cola.largo() as u32, Ordering::Relaxed);
        });
    }
}

/// Cuantos informes de teclado USB se han descartado por cola llena.
pub fn hid_descartes() -> u32 {
    HID_DESCARTES.load(Ordering::Relaxed)
}

/// Cuantos eventos de tecla USB se han entregado a la cola.
pub fn hid_publicados() -> u32 {
    HID_PUBLICADOS.load(Ordering::Relaxed)
}

/// Ocupacion maxima que ha llegado a tener la cola de eventos.
pub fn hid_cola_max() -> u32 {
    HID_COLA_MAX.load(Ordering::Relaxed)
}

/// Latencia maxima aproximada entre encolar y drenar, en milisegundos.
///
/// La resolucion es la del PIT (10 ms) y el valor es el **maximo** visto: dice
/// si el consumidor se ha quedado atras, no cuanto tarda de media.
pub fn hid_latencia_max_ms() -> u32 {
    HID_LATENCIA_MAX.load(Ordering::Relaxed).saturating_mul(10)
}

/// Saca el evento mas antiguo y anota la latencia con la que salio.
fn sacar_evento() -> Option<Evento> {
    let ahora = crate::pit::ticks() as u32;
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            let cola = &mut *HID_PENDIENTE.get();
            let encolado = cola.sacar()?;
            let espera = ahora.wrapping_sub(encolado.tick);
            HID_LATENCIA_MAX.fetch_max(espera, Ordering::Relaxed);
            Some(encolado.evento)
        })
    }
}

/// Guarda un scancode leído incidentalmente por el controlador PS/2 durante
/// una transacción auxiliar. Lo consumirá el siguiente `poll_event`.
pub fn guardar_scancode_ps2(byte: u8) {
    if !matches!(byte, 0xFA | 0xFE | 0xEE | 0x00 | 0xFF) {
        unsafe {
            crate::cpu::sin_interrupciones(|| (&mut *PS2_PENDING.get()).push(byte));
        }
    }
}

/// Drena el búfer compartido del 8042 desde IRQ1 o IRQ12 y conserva cada byte
/// en su cola correcta. No asigna memoria ni hace trabajo gráfico en contexto
/// de interrupción.
pub fn on_ps2_irq() {
    unsafe {
        for _ in 0..32 {
            let estado = inb(0x64);
            if estado & 0x01 == 0 {
                break;
            }
            let byte = inb(0x60);
            if estado & 0xC0 != 0 {
                continue;
            }
            if estado & 0x20 != 0 {
                crate::mouse::alimentar(byte);
            } else {
                guardar_scancode_ps2(byte);
            }
        }
    }
}

/// Manejador de la IRQ4: vacia el UART en el bufer circular.
pub fn on_serial_irq() {
    recoger();
}

/// Vacia el UART en el bufer circular. Se llama desde la IRQ4 y tambien, como
/// respaldo, desde el sondeo del teclado. Desactiva las interrupciones mientras
/// toca el bufer para no corromperlo si la IRQ llega a mitad.
pub fn recoger() {
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            let serial = Serial::new();
            let ring = &mut *RING.get();
            while let Some(byte) = serial.try_read() {
                ring.push(match byte {
                    b'\r' => b'\n',
                    other => other,
                });
            }
        });
    }
}

/// Resultado de una tecla: un caracter CP437 o una tecla de navegacion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tecla {
    /// Caracter ya traducido (CP437), incluidos Intro, Retroceso, Tabulador y
    /// Escape.
    Caracter(u8),
    /// Tecla que no produce texto.
    Especial(Especial),
}

/// Teclas que no producen texto y que rocky sabe nombrar.
///
/// Son las del bloque de navegacion en su version **extendida**. Las teclas de
/// funcion no estan: ningun consumidor las usa.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Especial {
    /// Flecha arriba.
    Arriba,
    /// Flecha abajo.
    Abajo,
    /// Flecha izquierda.
    Izquierda,
    /// Flecha derecha.
    Derecha,
    /// Inicio.
    Inicio,
    /// Fin.
    Fin,
    /// Re Pag.
    RePag,
    /// Av Pag.
    AvPag,
    /// Insert.
    Insert,
    /// Suprimir.
    Supr,
}

/// Tecla ya resuelta, con los modificadores que habia al resolverla.
///
/// Los modificadores se leen en el momento de traducir, en el mismo orden en
/// que llegaron las teclas: si el consumidor drena tarde, un Ctrl que ya se
/// solto no se le queda pegado a la letra.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pulsacion {
    /// La tecla.
    pub tecla: Tecla,
    /// Mayusculas (Shift) estaba pulsado.
    pub shift: bool,
    /// Control estaba pulsado.
    pub ctrl: bool,
}

/// Tecla de navegacion de una pulsacion extendida.
fn navegacion(codigo: u8, extendido: bool, pulsada: bool) -> Option<Tecla> {
    if !extendido || !pulsada {
        return None;
    }
    Some(Tecla::Especial(match codigo {
        0x48 => Especial::Arriba,
        0x50 => Especial::Abajo,
        0x4B => Especial::Izquierda,
        0x4D => Especial::Derecha,
        0x47 => Especial::Inicio,
        0x4F => Especial::Fin,
        0x49 => Especial::RePag,
        0x51 => Especial::AvPag,
        0x52 => Especial::Insert,
        0x53 => Especial::Supr,
        _ => return None,
    }))
}

/// Aplica una tecla ya separada en sus tres datos: codigo sin `0x80`, si es
/// extendida y si se pulsa.
///
/// Devuelve el caracter cuando la tecla produce texto. Los modificadores y los
/// bloqueos se actualizan aqui; las liberaciones solo cambian modificadores.
fn aplicar(codigo: u8, extendido: bool, pulsada: bool) -> Option<u8> {
    // Teclas extendidas: solo Control derecho (0xE0 0x1D) y AltGr (0xE0 0x38)
    // cambian algo; el resto (flechas, Inicio, teclado numerico) no producen
    // texto y las de navegacion ya se han clasificado con `navegacion`.
    if extendido {
        match codigo {
            0x1D => CONTROL.store(pulsada, Ordering::Relaxed),
            0x38 => ALTGR.store(pulsada, Ordering::Relaxed),
            _ => {}
        }
        return None;
    }
    if !pulsada {
        match codigo {
            0x1D => CONTROL.store(false, Ordering::Relaxed),
            0x2A | 0x36 => SHIFT.store(false, Ordering::Relaxed),
            _ => {}
        }
        return None;
    }

    match codigo {
        0x1D => {
            CONTROL.store(true, Ordering::Relaxed);
            None
        }
        0x2A | 0x36 => {
            SHIFT.store(true, Ordering::Relaxed);
            None
        }
        0x3A => {
            MAYUS.store(!MAYUS.load(Ordering::Relaxed), Ordering::Relaxed);
            sincronizar_leds();
            None
        }
        0x45 => {
            NUM.store(!NUM.load(Ordering::Relaxed), Ordering::Relaxed);
            sincronizar_leds();
            None
        }
        0x46 => {
            SCROLL.store(!SCROLL.load(Ordering::Relaxed), Ordering::Relaxed);
            sincronizar_leds();
            None
        }
        0x1C => {
            MUERTA.store(0, Ordering::Relaxed);
            Some(b'\n')
        }
        0x0E => {
            MUERTA.store(0, Ordering::Relaxed);
            Some(0x08)
        }
        0x39 => Some(combinar(b' ')),
        0x0F => {
            MUERTA.store(0, Ordering::Relaxed);
            Some(b'\t')
        }
        0x01 => {
            MUERTA.store(0, Ordering::Relaxed);
            Some(0x1B) // ESC
        }
        // Teclado numerico: los operadores no dependen del Bloq Num.
        0x37 => Some(b'*'),
        0x4A => Some(b'-'),
        0x4E => Some(b'+'),
        // Los digitos solo si el Bloq Num esta activo; si no, son teclas de
        // cursor y aqui no hacen falta.
        0x47 | 0x48 | 0x49 | 0x4B | 0x4C | 0x4D | 0x4F | 0x50 | 0x51 | 0x52 | 0x53 => {
            if NUM.load(Ordering::Relaxed) {
                Some(tecla_num(codigo))
            } else {
                None
            }
        }
        _ => {
            let mayus = MAYUS.load(Ordering::Relaxed);
            let shift = SHIFT.load(Ordering::Relaxed);
            let altgr = ALTGR.load(Ordering::Relaxed);
            if let Some(m) = muerta_de(codigo, shift, altgr) {
                MUERTA.store(m, Ordering::Relaxed);
                return None;
            }
            let (normal, shifted, altgr_c) = base_char(codigo)?;
            let c = if altgr {
                // El AltGr tiene prioridad: solo unas pocas teclas lo ofrecen y
                // nunca se combinan con Shift.
                altgr_c?
            } else if shift ^ (mayus && es_letra(normal)) {
                // Con el Bloq Mayus las letras se invierten; el resto de teclas
                // (numerica, puntuacion) siguen dependiendo solo del Shift.
                shifted
            } else {
                normal
            };
            Some(combinar(c))
        }
    }
}

/// Interpreta un byte del flujo del 8042: prefijos, bytes de estado y la tecla
/// en si, que se resuelve con [`navegacion`] y [`aplicar`].
fn traducir_scancode(scancode: u8) -> Option<Tecla> {
    // Bytes que no son pulsaciones: eco (0xEE), ACK (0xFA), reintento (0xFE) y
    // byte invalido (0x00/0xFF). Si se colasen acabarian en pantalla sueltos.
    //
    // El 0xAA *si* es una pulsacion en marcha: es la liberacion del Shift
    // izquierdo (0x2A | 0x80). Durante el arrancado tambien es el "autotest
    // correcto" del teclado, pero ese lo consume `ps2::init` antes de que el
    // escritorio empiece a sondear.
    if matches!(scancode, 0x00 | 0xEE | 0xFA | 0xFE | 0xFF) {
        return None;
    }
    if scancode == 0xE0 {
        EXTENDED.store(true, Ordering::Relaxed);
        return None;
    }
    if scancode == 0xE1 {
        // Pausa manda 6 bytes (E1 1D 45 E1 9D C5); el 0x45 entre ellos es el
        // codigo del Bloq Num y habria que ignorarlo o pararia cada Pausa.
        E1.store(5, Ordering::Relaxed);
        return None;
    }
    if E1.load(Ordering::Relaxed) != 0 {
        E1.fetch_sub(1, Ordering::Relaxed);
        return None;
    }
    let extendido = EXTENDED.swap(false, Ordering::Relaxed);
    let pulsada = scancode & 0x80 == 0;
    let codigo = scancode & 0x7F;
    if let Some(tecla) = navegacion(codigo, extendido, pulsada) {
        return Some(tecla);
    }
    aplicar(codigo, extendido, pulsada).map(Tecla::Caracter)
}

/// Digitos del teclado numerico segun su codigo de barrido.
fn tecla_num(scancode: u8) -> u8 {
    match scancode {
        0x47 => b'7',
        0x48 => b'8',
        0x49 => b'9',
        0x4B => b'4',
        0x4C => b'5',
        0x4D => b'6',
        0x4F => b'1',
        0x50 => b'2',
        0x51 => b'3',
        0x52 => b'0',
        _ => b'.',
    }
}

/// Indica si un caracter es letra (para que el Bloq Mayus lo invierta).
///
/// Ademas de las letras ASCII, lo son la ene (0xA4) y la cedilla (0x87) del
/// juego CP437, que es el que usa la fuente de la VGA.
fn es_letra(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == 0xA4 || c == 0x87
}

/// Teclas muertas de la distribucion española: el acento grave y el
/// circunflejo comparten tecla, y el agudo y la diéresis otra.
///
/// Se guardan como un tipo y no como el caracter que representan a proposito:
/// en CP437 el `¿` vale 0xA8, el mismo byte que se usaba para marcar la
/// diéresis, y mirando el caracter no habria forma de distinguirlos.
const MUERTE_GRAVE: u8 = 1;
const MUERTE_CIRCUNFLEJO: u8 = 2;
const MUERTE_AGUDA: u8 = 3;
const MUERTE_DIERESIS: u8 = 4;
const MUERTE_VIRGULILLA: u8 = 5;

/// Tecla muerta que produce una tecla, si es que es una.
///
/// El AltGr manda: las que en la fila `0x1A`/`0x28` son muertas con Mayus o
/// a secas, con AltGr dan `[ {` y no hay que tocar la composicion.
fn muerta_de(scancode: u8, shift: bool, altgr: bool) -> Option<u8> {
    if altgr {
        return match scancode {
            0x05 => Some(MUERTE_VIRGULILLA), // AltGr+4 -> ~
            _ => None,
        };
    }
    match scancode {
        0x1A => Some(if shift {
            MUERTE_CIRCUNFLEJO
        } else {
            MUERTE_GRAVE
        }),
        0x28 => Some(if shift { MUERTE_DIERESIS } else { MUERTE_AGUDA }),
        _ => None,
    }
}

/// Signo que se escribe cuando una tecla muerta no llega a combinar.
///
/// El CP437 solo trae glifos del grave, el circunflejo y la virgulilla; el
/// acento agudo y la diéresis se sustituyen por los ASCII mas parecidos.
fn signo_muerta(tipo: u8) -> u8 {
    match tipo {
        MUERTE_GRAVE => b'`',
        MUERTE_CIRCUNFLEJO => b'^',
        MUERTE_AGUDA => b'\'',
        MUERTE_DIERESIS => b'"',
        _ => b'~',
    }
}

/// Combina lo que haya pendiente con el siguiente caracter.
///
/// Si no hay tecla muerta, o si la combinacion no existe, el caracter sale tal
/// cual (y el acento, si lo hubo, se emite antes via [`PEND`]).
fn combinar(c: u8) -> u8 {
    let muerta = MUERTA.swap(0, Ordering::Relaxed);
    if muerta == 0 {
        return c;
    }
    match combinar_letra(muerta, c) {
        Some(r) => r,
        None => {
            PEND.store(c, Ordering::Relaxed);
            signo_muerta(muerta)
        }
    }
}

/// Devuelve el acentuado que resulta de juntar una tecla muerta con una letra.
fn combinar_letra(muerta: u8, c: u8) -> Option<u8> {
    // Con espacio se escribe el propio signo del acento.
    if c == b' ' {
        return Some(signo_muerta(muerta));
    }
    if !c.is_ascii_alphabetic() {
        return None;
    }
    let alto = c.is_ascii_uppercase();
    let baja = c.to_ascii_lowercase();
    let (min, may) = match (muerta, baja) {
        (MUERTE_GRAVE, b'a') => (0x85, 0xC0),       // à À
        (MUERTE_GRAVE, b'e') => (0x8A, 0xC8),       // è È
        (MUERTE_GRAVE, b'i') => (0x8D, 0xCC),       // ì Ì
        (MUERTE_GRAVE, b'o') => (0x95, 0xD2),       // ò Ò
        (MUERTE_GRAVE, b'u') => (0x97, 0xD9),       // ù Ù
        (MUERTE_CIRCUNFLEJO, b'a') => (0x83, 0xC2), // â Â
        (MUERTE_CIRCUNFLEJO, b'e') => (0x88, 0xCA), // ê Ê
        (MUERTE_CIRCUNFLEJO, b'i') => (0x8C, 0xCE), // î Î
        (MUERTE_CIRCUNFLEJO, b'o') => (0x93, 0xD4), // ô Ô
        (MUERTE_CIRCUNFLEJO, b'u') => (0x96, 0xDB), // û Û
        (MUERTE_AGUDA, b'a') => (0xA0, 0xC1),       // á Á
        (MUERTE_AGUDA, b'e') => (0x82, 0xC9),       // é É
        (MUERTE_AGUDA, b'i') => (0xA1, 0xCD),       // í Í
        (MUERTE_AGUDA, b'o') => (0xA2, 0xD3),       // ó Ó
        (MUERTE_AGUDA, b'u') => (0xA3, 0xDA),       // ú Ú
        (MUERTE_AGUDA, b'y') => (0xFD, 0xDD),       // ý Ý
        (MUERTE_DIERESIS, b'a') => (0x84, 0x8E),    // ä Ä
        (MUERTE_DIERESIS, b'e') => (0x89, 0xCB),    // ë Ë
        (MUERTE_DIERESIS, b'i') => (0x8B, 0xCF),    // ï Ï
        (MUERTE_DIERESIS, b'o') => (0x94, 0x99),    // ö Ö
        (MUERTE_DIERESIS, b'u') => (0x81, 0x9A),    // ü Ü
        (MUERTE_VIRGULILLA, b'a') => (0xE3, 0xC3),  // ã Ã
        (MUERTE_VIRGULILLA, b'n') => (0xA4, 0xA5),  // ñ Ñ
        (MUERTE_VIRGULILLA, b'o') => (0xF5, 0xD5),  // õ Õ
        _ => return None,
    };
    Some(if alto { may } else { min })
}

/// Pone los tres LEDs del teclado al dia con el estado interno.
///
/// El teclado puede arrancar con luces encendidas de una sesion anterior, asi
/// que conviene llamarlo tambien una vez al empezar.
pub fn sincronizar_leds() {
    let mut m = 0u8;
    if SCROLL.load(Ordering::Relaxed) {
        m |= 0x01;
    }
    if NUM.load(Ordering::Relaxed) {
        m |= 0x02;
    }
    if MAYUS.load(Ordering::Relaxed) {
        m |= 0x04;
    }
    crate::ps2::leds_teclado(m);
}

/// Distribucion española es_ES: (normal, con Shift, con AltGr).
///
/// El `None` del AltGr significa que esa tecla no ofrece nada con la tecla
/// derecha; el 0x00 no se usa para poder distinguirlo.
fn base_char(scancode: u8) -> Option<(u8, u8, Option<u8>)> {
    Some(match scancode {
        0x29 => (0xA7, 0xA6, Some(b'\\')), // º ª
        0x02 => (b'1', b'!', Some(b'|')),  // 1 !
        0x03 => (b'2', b'"', Some(b'@')),  // 2 "
        0x04 => (b'3', 0xFA, Some(b'#')),  // 3 ·  (CP437: punto medio = 0xFA)
        0x05 => (b'4', b'$', Some(b'~')),  // 4 $  (AltGr: virgulilla muerta)
        0x06 => (b'5', b'%', None),        // 5 %
        0x07 => (b'6', b'&', Some(0xAC)),  // 6 &  (AltGr: ¬)
        0x08 => (b'7', b'/', None),        // 7 /
        0x09 => (b'8', b'(', None),        // 8 (
        0x0A => (b'9', b')', None),        // 9 )
        0x0B => (b'0', b'=', None),        // 0 =
        0x0C => (b'\'', b'?', None),       // ' ?
        0x0D => (0xAD, 0xA8, None),        // ¡ ¿
        0x1A => (b'`', b'^', Some(b'[')),  // ` ^  (ambas muertas)
        0x1B => (b'+', b'*', Some(b']')),  // + *
        0x27 => (0xA4, 0xA5, None),        // ñ Ñ
        0x28 => (0xB4, 0xA8, Some(b'{')),  // ´ ¨  (ambas muertas)
        0x2B => (0x87, 0x80, Some(b'}')),  // ç Ç
        0x33 => (b',', b';', None),        // , ;
        0x34 => (b'.', b':', None),        // . :
        0x35 => (b'-', b'_', None),        // - _
        0x56 => (b'<', b'>', None),        // < >  (tecla ISO, a la izq. de Z)
        // Las letras son identicas a las de cualquier teclado QWERTY.
        0x10 => (b'q', b'Q', None),
        0x11 => (b'w', b'W', None),
        0x12 => (b'e', b'E', None),
        0x13 => (b'r', b'R', None),
        0x14 => (b't', b'T', None),
        0x15 => (b'y', b'Y', None),
        0x16 => (b'u', b'U', None),
        0x17 => (b'i', b'I', None),
        0x18 => (b'o', b'O', None),
        0x19 => (b'p', b'P', None),
        0x1E => (b'a', b'A', None),
        0x1F => (b's', b'S', None),
        0x20 => (b'd', b'D', None),
        0x21 => (b'f', b'F', None),
        0x22 => (b'g', b'G', None),
        0x23 => (b'h', b'H', None),
        0x24 => (b'j', b'J', None),
        0x25 => (b'k', b'K', None),
        0x26 => (b'l', b'L', None),
        0x2C => (b'z', b'Z', None),
        0x2D => (b'x', b'X', None),
        0x2E => (b'c', b'C', None),
        0x2F => (b'v', b'V', None),
        0x30 => (b'b', b'B', None),
        0x31 => (b'n', b'N', None),
        0x32 => (b'm', b'M', None),
        _ => return None,
    })
}

/// Saca eventos de la cola neutral hasta encontrar uno que el consumidor pueda
/// usar.
///
/// Lo que no produce caracter ni es navegacion (un Shift, un bloqueo, una
/// tecla de funcion) se aplica y se sigue con el siguiente: los modificadores
/// ya han quedado puestos para la tecla que venga detras.
fn tecla_hid() -> Option<Pulsacion> {
    loop {
        let evento = sacar_evento()?;
        if let Some(tecla) = navegacion(evento.codigo, evento.extendido, evento.pulsada) {
            return Some(pulsacion(tecla));
        }
        if let Some(caracter) = aplicar(evento.codigo, evento.extendido, evento.pulsada) {
            return Some(pulsacion(Tecla::Caracter(caracter)));
        }
    }
}

/// Saca un byte del 8042 o de lo que dejo pendiente la inicializacion del
/// raton. Devuelve `None` si no hay nada.
fn leer_ps2() -> Option<u8> {
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            if let Some(byte) = (&mut *PS2_PENDING.get()).pop() {
                return Some(byte);
            }
            let estado = inb(0x64);
            if estado & 0x01 == 0 {
                return None;
            }
            let byte = inb(0x60);
            // Los bits 6 y 7 del puerto de estado avisan de un byte con error
            // de temporizacion o de paridad: se descarta.
            if estado & 0xC0 != 0 {
                return None;
            }
            // El bit 5 distingue el raton del teclado: no se interpreta como
            // tecla; el sondeo del raton termina de vaciar el paquete.
            if estado & 0x20 != 0 {
                crate::mouse::alimentar(byte);
                return None;
            }
            Some(byte)
        })
    }
}

/// Interpreta bytes del PS/2 hasta encontrar una tecla util.
///
/// El bucle esta acotado: con una tecla mantenida el 8042 no para de mandar
/// repeticiones y no conviene comerse el teclado (ni los bytes del raton)
/// entero en una sola llamada.
fn tecla_ps2() -> Option<Pulsacion> {
    for _ in 0..8 {
        let byte = leer_ps2()?;
        if let Some(tecla) = traducir_scancode(byte) {
            return Some(pulsacion(tecla));
        }
    }
    None
}

fn serial_key() -> Option<u8> {
    // El bufer circular lo llena la IRQ4; ademas, por si el pin del teclado se
    // quedo sin atender, vaciamos el UART aqui antes de sacar nada.
    recoger();
    unsafe { crate::cpu::sin_interrupciones(|| (&mut *RING.get()).pop()) }
}

/// Lee los modificadores en el momento de resolver la tecla.
fn pulsacion(tecla: Tecla) -> Pulsacion {
    Pulsacion {
        tecla,
        shift: SHIFT.load(Ordering::Relaxed),
        ctrl: CONTROL.load(Ordering::Relaxed),
    }
}

/// Saca la siguiente tecla ya resuelta: un caracter o una tecla especial.
///
/// Es el **unico adaptador** de la entrada. Mira, en este orden, el caracter
/// pendiente de una tecla muerta, la cola neutral de USB, los bytes del 8042 y
/// el puerto serie. PS/2 y serie se leen del hardware al drenar, asi que no
/// tienen una segunda cola de eventos que pueda adelantarse a la primera.
///
/// El orden entre fuentes es fijo (USB antes que PS/2) y el estado de
/// modificadores es comun a todas: la ultima tecla vista manda. Con un solo
/// teclado, que es el caso normal, el orden es exactamente el de las teclas.
pub fn poll_tecla() -> Option<Pulsacion> {
    let pend = PEND.swap(0, Ordering::Relaxed);
    if pend != 0 {
        return Some(pulsacion(Tecla::Caracter(pend)));
    }
    if let Some(p) = tecla_hid() {
        return Some(p);
    }
    if let Some(p) = tecla_ps2() {
        return Some(p);
    }
    serial_key().map(|caracter| Pulsacion {
        tecla: Tecla::Caracter(caracter),
        shift: false,
        ctrl: false,
    })
}

/// Devuelve la siguiente pulsacion **de texto** y el estado de Ctrl observado
/// al leerla. Para bytes del puerto serie, Ctrl siempre es `false`.
///
/// Es el adaptador de [`poll_tecla`] para los consumidores que solo entienden
/// caracteres: las teclas de navegacion se descartan aqui, como se hacian
/// antes de existir la cola neutral. Quien las quiera usar llama a
/// [`poll_tecla`].
pub fn poll_event() -> Option<(u8, bool)> {
    loop {
        let p = poll_tecla()?;
        if let Tecla::Caracter(caracter) = p.tecla {
            return Some((caracter, p.ctrl));
        }
    }
}

/// Devuelve la siguiente pulsacion disponible, si la hay.
pub fn poll() -> Option<u8> {
    poll_event().map(|(tecla, _)| tecla)
}

/// Espera hasta que haya una tecla, sondeando en bucle (estilo DOS).
pub fn wait() -> u8 {
    loop {
        if let Some(key) = poll() {
            return key;
        }
        core::hint::spin_loop();
    }
}
