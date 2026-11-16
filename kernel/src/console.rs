//! Consola unificada: escribe a la vez en VGA y en el puerto serie.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::string::String;

use crate::serial::Serial;
use crate::sync::SyncCell;
use crate::vga::Vga;

pub struct Console {
    vga: Vga,
    serial: Serial,
}

impl Console {
    const fn new() -> Self {
        Console {
            vga: Vga::new(),
            serial: Serial::new(),
        }
    }

    pub fn clear(&mut self) {
        if !crate::kira::comp::terminal_clear() {
            self.vga.clear();
        }
    }

    pub fn put_byte(&mut self, byte: u8) {
        // La salida por serie se mantiene siempre (depuracion y pruebas).
        self.serial.write_byte(byte);
        if !crate::kira::comp::terminal_write_byte(byte) {
            self.vga.put_byte(byte);
        }
    }
}

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let byte = b[i];
            if byte < 0x80 {
                if byte == b'\n' {
                    self.put_byte(b'\r');
                }
                self.put_byte(byte);
                i += 1;
                continue;
            }
            // El kernel se escribe en ASCII, pero los textos y los ficheros que
            // se imprimen con `{}` vienen en UTF-8 y la pantalla es CP437: aqui
            // se traduce para que los acentos salgan bien. En modo texto solo
            // vale la ROM (sin los acentos compuestos de bios::COMPONER), asi
            // que se usa la tabla base de `texto`.
            let (glifo, ancho) = match crate::texto::utf8_en(b, i) {
                Some((c, n)) => (glifo_rom(c), n),
                None => (b'?', 1),
            };
            self.put_byte(glifo);
            i += ancho;
        }
        Ok(())
    }
}

/// Glifo CP437 base del caracter Unicode `c` (tabla [`crate::texto::CP437`]).
///
/// La consola escribe en modo texto, donde el generador de caracteres de la
/// ROM no trae los acentos compuestos de `bios::COMPONER`: esos caracteres
/// salen como `?`. La GUI, que dibuja con la fuente ya preparada, si los
/// tiene (ver `texto::glifo_de`).
fn glifo_rom(c: u32) -> u8 {
    crate::texto::CP437
        .iter()
        .find(|(_, u)| *u == c)
        .map(|(g, _)| *g)
        .unwrap_or(b'?')
}

/// Caracter Unicode del glifo CP437 `b`.
///
/// Es el camino de vuelta de [`glifo_rom`]: lo que se teclea (que son bytes
/// CP437) se convierte aqui a texto para poder guardarlo en un fichero.
pub fn char_de_cp437(b: u8) -> char {
    crate::texto::char_de_glifo(b)
}

/// Texto (UTF-8) a partir de una linea tecleada, que viene en CP437.
pub fn texto_cp437(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        s.push(char_de_cp437(b));
    }
    s
}

static CONSOLE: SyncCell<Console> = SyncCell::new(Console::new());

/// Si otra tarea esta imprimiendo. La consola es un unico `Console` para todo
/// el nucleo: sin esta guarda, dos tareas pueden entrar a la vez en
/// `write_str` y, ademas de mezclar las lineas, tener dos `&mut` vivos.
static OCUPADA: AtomicBool = AtomicBool::new(false);

/// Turno de impresion, mientras vive.
///
/// Se suelta con `Drop`, tambien si la impresion termina por un camino de
/// error. Se toma **solo** si se puede esperar: con las interrupciones
/// apagadas o dentro de una seccion critica el que tiene el turno no podria
/// volver a ejecutarse y la espera seria un bloqueo mutuo; ahi se imprime sin
/// guarda (son las rutas de arranque temprano, que son monotarea, o secciones
/// que no deberian imprimir).
struct Turno;

impl Turno {
    fn tomar() -> Option<Turno> {
        if !crate::cpu::interrupciones_activas() || crate::tarea::en_critica() {
            return None;
        }
        while OCUPADA.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        Some(Turno)
    }
}

impl Drop for Turno {
    fn drop(&mut self) {
        OCUPADA.store(false, Ordering::Release);
    }
}

fn console() -> &'static mut Console {
    // Seguro: el acceso pasa siempre por `Turno` (o por una ruta de arranque
    // monotarea), que garantiza que no hay dos referencias vivas a la vez.
    unsafe { &mut *CONSOLE.get() }
}

pub fn init() {
    let _turno = Turno::tomar();
    let c = console();
    c.serial.init();
    c.clear();
}

pub fn clear() {
    let _turno = Turno::tomar();
    console().clear();
}

/// Copia de la ultima pantalla de texto (memoria VGA y posicion del cursor).
///
/// Poner un modo VESA y volver con `INT 10h AX=0003h` deja la memoria de texto
/// en blanco, porque el BIOS la limpia al fijar el modo. Para que el interprete
/// recupere exactamente lo que tenia, aqui se guarda antes de entrar en
/// grafico y se repone al volver al modo texto.
static TEXTO: SyncCell<([u8; crate::vga::TAM_TEXTO], (usize, usize))> =
    SyncCell::new(([0; crate::vga::TAM_TEXTO], (0, 0)));

fn texto() -> &'static mut ([u8; crate::vga::TAM_TEXTO], (usize, usize)) {
    // Seguro: solo se usa desde la ruta de impresion del nucleo, de corta
    // duracion, igual que el propio `Console`.
    unsafe { &mut *TEXTO.get() }
}

/// Guarda la pantalla de texto actual (antes de poner un modo grafico).
pub fn guardar_texto() {
    let _turno = Turno::tomar();
    let c = console();
    let t = texto();
    c.vga.volcar(&mut t.0);
    t.1 = c.vga.posicion();
}

/// Repone la pantalla de texto guardada (tras volver de un modo grafico).
pub fn restaurar_texto() {
    let _turno = Turno::tomar();
    let c = console();
    let t = texto();
    c.vga.cargar(&t.0);
    c.vga.situar(t.1 .0, t.1 .1);
}

pub fn put(byte: u8) {
    let _turno = Turno::tomar();
    console().put_byte(byte);
}

pub fn write_bytes(bytes: &[u8]) {
    let _turno = Turno::tomar();
    let c = console();
    for &b in bytes {
        c.put_byte(b);
    }
}

pub fn _print(args: fmt::Arguments) {
    let _turno = Turno::tomar();
    let _ = console().write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

/// Imprime un fallo irrecuperable del kernel.
///
/// Es el `#[panic_handler]`: no imprime por la consola normal (que depende del
/// heap y de la GUI) sino que entra en la ruta de emergencia de
/// [`crate::panico`], que pinta su propia pantalla azul y se detiene.
pub fn panic_report(info: &core::panic::PanicInfo) -> ! {
    crate::panico::desde_panico(info)
}
