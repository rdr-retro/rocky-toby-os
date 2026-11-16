//! Pantalla de emergencia de rocky: el "modo seguro" de salida de video.
//!
//! La usa el subsistema de emergencia ([`crate::panico`]) para mostrar el
//! diagnostico de un fallo fatal. Deliberadamente **no** depende de TobyUI, del
//! planificador, del heap ni de la consola normal: solo escribe en memoria de
//! video y en el puerto serie, y todo su estado son `static` de tamano fijo.
//!
//! Hay dos caminos:
//!
//! * **Framebuffer VESA**, si hay un modo grafico activo (lo registra
//!   [`crate::gfx::set_mode`] y lo borra [`crate::bios::set_text_mode`]): se
//!   pinta el fondo de azul y los glifos con la fuente del BIOS copiada en RAM.
//! * **Memoria de texto de la VGA** (`0xB8000`, 80x25) si no lo hay. Es lo que
//!   hay durante el arranque y en modo texto.
//!
//! En ambos casos cada caracter va **ademas** al puerto serie COM1 en crudo, que
//! es el canal que leen las pruebas automaticas.

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::bios::Font;
use crate::cpu::{inb, outb};
use crate::serial::COM1;
use crate::sync::SyncCell;

/// Azul del fondo (0x00RRGGBB).
const AZUL: u32 = 0x0010_3A_8C;
/// Color del texto.
const CLARO: u32 = 0x00F0_F4_FF;

/// Memoria de texto de la VGA: 2 bytes por celda (glifo + atributo).
const VGA_TEXTO: usize = 0xB8000;
const VGA_COLS: usize = 80;
const VGA_ROWS: usize = 25;
/// Blanco brillante sobre azul: el equivalente en texto del fondo azul.
const VGA_ATTR: u8 = 0x1F;

/// Modo grafico activo.
#[derive(Clone, Copy)]
pub struct Modo {
    /// Direccion lineal del framebuffer (mapeado por identidad).
    pub lfb: usize,
    pub ancho: usize,
    pub alto: usize,
    /// Paso de linea en **bytes**.
    pub paso: usize,
}

/// Registro del modo grafico activo. Se lee sin tocar el heap; si esta a `None`
/// la ruta de emergencia usa el modo texto.
static MODO: SyncCell<Option<Modo>> = SyncCell::new(None);

/// Salida elegida y su geometria, decidida una sola vez en [`arrancar`].
#[derive(Clone, Copy)]
enum Backend {
    Texto,
    Marco {
        m: Modo,
        escala: usize,
        alto_car: usize,
        fuente: Font,
    },
}

static BACKEND: SyncCell<Backend> = SyncCell::new(Backend::Texto);
static COL: AtomicUsize = AtomicUsize::new(0);
static FILA: AtomicUsize = AtomicUsize::new(0);
static COLS: AtomicUsize = AtomicUsize::new(VGA_COLS);
static FILAS: AtomicUsize = AtomicUsize::new(VGA_ROWS);

/// Fija el modo grafico activo. La llama [`crate::gfx::set_mode`].
pub fn fijar_modo(lfb: usize, ancho: usize, alto: usize, paso: usize) {
    let m = Modo {
        lfb,
        ancho,
        alto,
        paso,
    };
    unsafe { *MODO.get() = Some(m) };
}

/// Olvida el modo grafico (se volvio al texto). La llama
/// [`crate::bios::set_text_mode`].
pub fn olvidar_modo() {
    unsafe { *MODO.get() = None };
}

/// Modo grafico activo (framebuffer, geometria y paso de linea), si lo hay.
///
/// Lo usan las rutas de consola y los controladores que necesitan saber donde
/// esta el framebuffer que dejo VBE (por ejemplo, el cursor de AMD, para
/// deducir la direccion de GPU a partir de la fisica).
pub fn modo() -> Option<Modo> {
    unsafe { *MODO.get() }
}

fn backend() -> Backend {
    unsafe { *BACKEND.get() }
}

fn poner_backend(b: Backend) {
    unsafe { *BACKEND.get() = b };
}

/// Prepara la pantalla de emergencia: elige el backend, pinta el fondo de azul
/// y deja el cursor arriba a la izquierda.
///
/// Si no hay un modo grafico usable (o no se copio antes la fuente del BIOS),
/// cae al modo texto, que siempre esta con el firmware.
pub fn arrancar() {
    if let (Some(m), Some(fuente)) = (modo(), crate::bios::fuente_en_ram()) {
        if modo_utilizable(&m) {
            pintar_azul(&m);
            let escala = escala_para(m.alto);
            let alto_car = (fuente.rows.max(1)).min(32);
            let cols = (m.ancho / (8 * escala)).max(1);
            let filas = (m.alto / (alto_car * escala + escala)).max(1);
            poner_backend(Backend::Marco {
                m,
                escala,
                alto_car,
                fuente,
            });
            COLS.store(cols, Ordering::Relaxed);
            FILAS.store(filas, Ordering::Relaxed);
            COL.store(0, Ordering::Relaxed);
            FILA.store(0, Ordering::Relaxed);
            return;
        }
    }
    limpiar_texto();
    poner_backend(Backend::Texto);
    COLS.store(VGA_COLS, Ordering::Relaxed);
    FILAS.store(VGA_ROWS, Ordering::Relaxed);
    COL.store(0, Ordering::Relaxed);
    FILA.store(0, Ordering::Relaxed);
}

/// Un framebuffer solo se acepta si la direccion parece estar en el tramo que el
/// cargador mapea por identidad (los primeros 4 GiB) y sus dimensiones cuadran.
///
/// Es una defensa barata: si el registro del modo estuviera a medias, es
/// preferible caer al modo texto que arriesgar otro fallo al pintar.
fn modo_utilizable(m: &Modo) -> bool {
    m.lfb >= 0x10_0000
        && m.lfb < 0x1_0000_0000
        && m.ancho >= 80
        && m.alto >= 25
        && m.ancho <= 8192
        && m.alto <= 8192
        && m.paso >= m.ancho * 4
        && m.paso <= m.ancho * 8
}

/// Escala de los glifos (`8x16` se queda pequeno en pantallas grandes).
fn escala_para(alto: usize) -> usize {
    (alto / 480).clamp(1, 4)
}

/// Escribe por la pantalla y por el puerto serie.
pub fn escribir(args: fmt::Arguments) {
    let mut salida = Salida;
    let _ = fmt::Write::write_fmt(&mut salida, args);
}

/// Escribe un texto **solo** por el puerto serie. Lo usa la ruta de reentrada,
/// que no debe volver a tocar la pantalla (fue lo que fallo).
pub fn aviso_serie(texto: &str) {
    for &b in texto.as_bytes() {
        if b == b'\n' {
            serie(b'\r');
        }
        serie(b);
    }
}

struct Salida;

impl fmt::Write for Salida {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            byte(b);
        }
        Ok(())
    }
}

fn byte(b: u8) {
    // El puerto serie se mantiene siempre: si el video no es de fiar, el
    // diagnostico sigue saliendo por ahi.
    if b == b'\n' {
        serie(b'\r');
    }
    serie(b);

    match backend() {
        Backend::Texto => byte_texto(b),
        Backend::Marco {
            m,
            escala,
            alto_car,
            fuente,
        } => byte_marco(m, escala, alto_car, fuente, b),
    }
}

// ---------------------------------------------------------------------------
// Camino de texto (VGA 80x25)
// ---------------------------------------------------------------------------

fn limpiar_texto() {
    for i in 0..VGA_COLS * VGA_ROWS {
        unsafe {
            core::ptr::write_volatile((VGA_TEXTO + i * 2) as *mut u8, b' ');
            core::ptr::write_volatile((VGA_TEXTO + i * 2 + 1) as *mut u8, VGA_ATTR);
        }
    }
}

fn byte_texto(b: u8) {
    let mut col = COL.load(Ordering::Relaxed);
    let mut fila = FILA.load(Ordering::Relaxed);
    match b {
        b'\n' => {
            col = 0;
            fila += 1;
        }
        b'\r' => col = 0,
        b'\t' => {
            for _ in 0..4 {
                if col >= VGA_COLS {
                    col = 0;
                    fila += 1;
                }
                celda_texto(fila, col, b' ');
                col += 1;
            }
        }
        c if c >= 0x20 => {
            if col >= VGA_COLS {
                col = 0;
                fila += 1;
            }
            celda_texto(fila, col, c);
            col += 1;
        }
        _ => {}
    }
    while fila >= VGA_ROWS {
        desplazar_texto();
        fila -= 1;
    }
    COL.store(col, Ordering::Relaxed);
    FILA.store(fila, Ordering::Relaxed);
}

fn celda_texto(fila: usize, col: usize, ch: u8) {
    if fila >= VGA_ROWS || col >= VGA_COLS {
        return;
    }
    let off = (fila * VGA_COLS + col) * 2;
    unsafe {
        core::ptr::write_volatile((VGA_TEXTO + off) as *mut u8, ch);
        core::ptr::write_volatile((VGA_TEXTO + off + 1) as *mut u8, VGA_ATTR);
    }
}

fn desplazar_texto() {
    unsafe {
        for fila in 1..VGA_ROWS {
            for col in 0..VGA_COLS {
                let src = (fila * VGA_COLS + col) * 2;
                let dst = ((fila - 1) * VGA_COLS + col) * 2;
                let g = core::ptr::read_volatile((VGA_TEXTO + src) as *const u8);
                let a = core::ptr::read_volatile((VGA_TEXTO + src + 1) as *const u8);
                core::ptr::write_volatile((VGA_TEXTO + dst) as *mut u8, g);
                core::ptr::write_volatile((VGA_TEXTO + dst + 1) as *mut u8, a);
            }
        }
        for col in 0..VGA_COLS {
            let off = ((VGA_ROWS - 1) * VGA_COLS + col) * 2;
            core::ptr::write_volatile((VGA_TEXTO + off) as *mut u8, b' ');
            core::ptr::write_volatile((VGA_TEXTO + off + 1) as *mut u8, VGA_ATTR);
        }
    }
}

// ---------------------------------------------------------------------------
// Camino de framebuffer (VESA)
// ---------------------------------------------------------------------------

fn pintar_azul(m: &Modo) {
    let px = m.paso / 4;
    let base = m.lfb as *mut u32;
    for y in 0..m.alto {
        for x in 0..px {
            unsafe { core::ptr::write_volatile(base.add(y * px + x), AZUL) };
        }
    }
}

fn byte_marco(m: Modo, escala: usize, alto_car: usize, fuente: Font, b: u8) {
    let cols = COLS.load(Ordering::Relaxed);
    let filas = FILAS.load(Ordering::Relaxed);
    let mut col = COL.load(Ordering::Relaxed);
    let mut fila = FILA.load(Ordering::Relaxed);
    match b {
        b'\n' => {
            col = 0;
            fila += 1;
        }
        b'\r' => col = 0,
        b'\t' => {
            for _ in 0..4 {
                if col >= cols {
                    col = 0;
                    fila += 1;
                }
                glifo_fb(&m, escala, alto_car, &fuente, fila, col, b' ');
                col += 1;
            }
        }
        c if c >= 0x20 => {
            if col >= cols {
                col = 0;
                fila += 1;
            }
            glifo_fb(&m, escala, alto_car, &fuente, fila, col, c);
            col += 1;
        }
        _ => {}
    }
    while fila >= filas {
        desplazar_marco(&m, escala, alto_car);
        fila -= 1;
    }
    COL.store(col, Ordering::Relaxed);
    FILA.store(fila, Ordering::Relaxed);
}

/// Dibuja un glifo de la fuente del BIOS. Todas las escrituras van comprobadas
/// contra el tamano del framebuffer: en una ruta de emergencia no puede haber
/// un desbordamiento que provoque otro fallo.
fn glifo_fb(
    m: &Modo,
    escala: usize,
    alto_car: usize,
    fuente: &Font,
    fila: usize,
    col: usize,
    ch: u8,
) {
    let x0 = col * 8 * escala;
    let y0 = fila * (alto_car * escala + escala);
    if x0 >= m.ancho || y0 >= m.alto {
        return;
    }
    let base = fuente.addr + ch as usize * fuente.bytes_per_char;
    let px = m.paso / 4;
    let lfb = m.lfb as *mut u32;
    for gy in 0..alto_car {
        let bits = unsafe { core::ptr::read_volatile((base + gy) as *const u8) };
        if bits == 0 {
            continue;
        }
        let py0 = y0 + gy * escala;
        for bit in 0..8 {
            if bits & (0x80 >> bit) == 0 {
                continue;
            }
            let px0 = x0 + bit * escala;
            for dy in 0..escala {
                let yy = py0 + dy;
                if yy >= m.alto {
                    break;
                }
                for dx in 0..escala {
                    let xx = px0 + dx;
                    if xx >= m.ancho {
                        break;
                    }
                    unsafe { core::ptr::write_volatile(lfb.add(yy * px + xx), CLARO) };
                }
            }
        }
    }
}

/// Sube el contenido del framebuffer una linea de texto (desplazamiento) y
/// deja la ultima en azul.
fn desplazar_marco(m: &Modo, escala: usize, alto_car: usize) {
    let alto_linea = alto_car * escala + escala;
    let px = m.paso / 4;
    if alto_linea == 0 || alto_linea >= m.alto {
        pintar_azul(m);
        return;
    }
    let base = m.lfb as *mut u32;
    // `copy` es un memmove: origen y destino se solapan a proposito.
    unsafe {
        core::ptr::copy(base.add(alto_linea * px), base, (m.alto - alto_linea) * px);
    }
    for y in (m.alto - alto_linea)..m.alto {
        for x in 0..px {
            unsafe { core::ptr::write_volatile(base.add(y * px + x), AZUL) };
        }
    }
}

// ---------------------------------------------------------------------------
// Puerto serie crudo
// ---------------------------------------------------------------------------

fn serie(b: u8) {
    unsafe {
        // Espera (con tope) a que el transmisor este libre. En un equipo sin
        // COM1 el registro lee 0xFF, que ya tiene el bit puesto: no se cuelga.
        let mut vueltas = 0u32;
        while inb(COM1 + 5) & 0x20 == 0 && vueltas < 200_000 {
            vueltas += 1;
        }
        outb(COM1, b);
    }
}
