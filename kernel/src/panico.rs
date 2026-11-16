//! Subsistema de emergencia de rocky: la pantalla azul del kernel.
//!
//! Toma el control ante un fallo fatal (una excepcion de la CPU o un `panic!`
//! de Rust) y muestra un diagnostico legible en una pantalla azul propia. Es una
//! **ruta aislada** de proposito:
//!
//! * no usa TobyUI, ni el planificador, ni el heap, ni bloqueos ni esperas;
//! * corta las interrupciones enmascarables y evita la reentrada;
//! * captura el estado **antes** de tocar nada mas, y lo que no puede capturar
//!   lo rotula como "no capturado" (nunca se inventa un valor);
//! * **no intenta reanudar** un nucleo posiblemente corrupto: termina en un
//!   bucle con las interrupciones apagadas a la espera de un reinicio.
//!
//! La salida de video la pone [`crate::pantalla`] (framebuffer VESA o texto VGA)
//! y ademas el puerto serie; aqui solo se captura, se formatea y se para.
//!
//! La distincion entre un fallo del nucleo y una excepcion de un proceso de
//! usuario la hace [`crate::idt`], que es quien ve el marco de interrupcion: una
//! excepcion de anillo 3 se contiene terminando ese proceso, no el sistema.

use core::arch::asm;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::idt::InterruptStack;
use crate::pantalla;

/// Impide la reentrada. Si la propia ruta de emergencia vuelve a fallar (por
/// ejemplo al pintar), no se reintenta: se avisa por serie y se para.
static EN_PANICO: AtomicBool = AtomicBool::new(false);

/// Registros generales del marco de interrupcion.
#[derive(Clone, Copy)]
pub struct Registros {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

/// Todo lo que la pantalla azul puede mostrar.
///
/// Los campos que no se hayan capturado van a `None` y se rotulan como "no
/// capturado": es preferible un hueco honesto a un dato inventado.
pub struct Diagnostico<'a> {
    pub motivo: &'a str,
    pub ubicacion: Option<&'a str>,
    pub vector: Option<u64>,
    pub error: Option<u64>,
    pub rip: Option<u64>,
    pub cs: Option<u64>,
    pub rflags: Option<u64>,
    pub rsp: Option<u64>,
    pub ss: Option<u64>,
    pub cr2: Option<u64>,
    pub cr3: Option<u64>,
    pub registros: Option<Registros>,
    pub pid: Option<u64>,
}

/// Entra en la pantalla azul por una excepcion de la CPU.
pub fn desde_excepcion(vector: u64, marco: &InterruptStack) -> ! {
    let d = Diagnostico {
        motivo: crate::idt::vector_name(vector),
        ubicacion: None,
        vector: Some(vector),
        error: Some(marco.error),
        rip: Some(marco.rip),
        cs: Some(marco.cs),
        rflags: Some(marco.rflags),
        rsp: Some(marco.rsp),
        ss: Some(marco.ss),
        cr2: if vector == 14 { leer_cr2() } else { None },
        cr3: leer_cr3(),
        registros: Some(Registros {
            rax: marco.rax,
            rbx: marco.rbx,
            rcx: marco.rcx,
            rdx: marco.rdx,
            rsi: marco.rsi,
            rdi: marco.rdi,
            rbp: marco.rbp,
            r8: marco.r8,
            r9: marco.r9,
            r10: marco.r10,
            r11: marco.r11,
            r12: marco.r12,
            r13: marco.r13,
            r14: marco.r14,
            r15: marco.r15,
        }),
        pid: crate::tarea::pid_actual(),
    };
    entrar(d)
}

/// Entra en la pantalla azul por un `panic!` de Rust.
///
/// Aqui no hay marco de interrupcion, asi que los registros y el vector van a
/// "no capturado"; el mensaje del `panic!` y su ubicacion son el diagnostico.
pub fn desde_panico(info: &core::panic::PanicInfo) -> ! {
    let mut msg = [0u8; 160];
    let mut ubic = [0u8; 96];

    let n_msg = {
        let mut w = BufferTexto::new(&mut msg);
        let _ = fmt::write(&mut w, format_args!("{}", info.message()));
        w.len
    };
    let n_ubic = match info.location() {
        Some(loc) => {
            let mut w = BufferTexto::new(&mut ubic);
            let _ = fmt::write(&mut w, format_args!("{}:{}", loc.file(), loc.line()));
            w.len
        }
        None => 0,
    };

    let motivo = core::str::from_utf8(&msg[..n_msg]).unwrap_or("(mensaje no legible)");
    let ubicacion = match info.location() {
        Some(_) => Some(core::str::from_utf8(&ubic[..n_ubic]).unwrap_or("(ubicacion no legible)")),
        None => None,
    };

    let d = Diagnostico {
        motivo,
        ubicacion,
        vector: None,
        error: None,
        rip: None,
        cs: None,
        rflags: None,
        rsp: None,
        ss: None,
        cr2: None,
        cr3: leer_cr3(),
        registros: None,
        pid: crate::tarea::pid_actual(),
    };
    entrar(d)
}

/// Punto unico de entrada: corta interrupciones, evita reentrada, pinta el
/// diagnostico y se detiene.
pub fn entrar(d: Diagnostico) -> ! {
    // Lo primero es cortar las interrupciones enmascarables: el manejador ya
    // corre con ellas tapadas si viene de una excepcion, pero un `panic!` puede
    // llegar con el reloj en marcha. Nada de bloqueos ni de esperas.
    unsafe { asm!("cli", options(nomem, nostack)) };

    if EN_PANICO.swap(true, Ordering::SeqCst) {
        // Reentrada: la ruta de emergencia (pintar, formatear) volvio a fallar.
        // No se reintenta nada; un aviso minimo por serie y a parar.
        pantalla::aviso_serie(
            "\nROCKY KERNEL PANIC (reentrada)\n\
             La ruta de emergencia fallo mientras informaba del fallo.\n\
             El sistema queda detenido.\n",
        );
        parar()
    }

    pantalla::arrancar();
    volcar(&d);
    parar()
}

/// Formatea y pinta el diagnostico. Todo va a la pantalla y al puerto serie.
fn volcar(d: &Diagnostico) {
    pantalla::escribir(format_args!("\n"));
    pantalla::escribir(format_args!("ROCKY KERNEL PANIC\n"));
    pantalla::escribir(format_args!("==================\n"));
    pantalla::escribir(format_args!("Motivo  : {}\n", d.motivo));
    if let Some(u) = d.ubicacion {
        pantalla::escribir(format_args!("Origen  : {}\n", u));
    }
    match d.vector {
        Some(v) => pantalla::escribir(format_args!(
            "Vector  : {:#06X}  {}\n",
            v,
            crate::idt::vector_name(v)
        )),
        None => pantalla::escribir(format_args!("Vector  : no aplicable (panic! de Rust)\n")),
    }
    match d.error {
        Some(e) => pantalla::escribir(format_args!("Error   : {:#018X}\n", e)),
        None => pantalla::escribir(format_args!("Error   : no capturado\n")),
    }
    pantalla::escribir(format_args!("RIP     : {}\n", Hex(d.rip)));
    pantalla::escribir(format_args!("CS      : {}\n", HexCorto(d.cs)));
    pantalla::escribir(format_args!("RFLAGS  : {}\n", Hex(d.rflags)));
    pantalla::escribir(format_args!("RSP     : {}\n", Hex(d.rsp)));
    pantalla::escribir(format_args!("SS      : {}\n", HexCorto(d.ss)));
    match d.cr2 {
        Some(v) => pantalla::escribir(format_args!("CR2     : {:#018X}  (direccion del #PF)\n", v)),
        None => pantalla::escribir(format_args!("CR2     : no aplicable (solo en #PF)\n")),
    }
    pantalla::escribir(format_args!("CR3     : {}\n", Hex(d.cr3)));
    match d.registros {
        Some(r) => {
            pantalla::escribir(format_args!(
                "RAX={:016X} RBX={:016X} RCX={:016X} RDX={:016X}\n",
                r.rax, r.rbx, r.rcx, r.rdx
            ));
            pantalla::escribir(format_args!(
                "RSI={:016X} RDI={:016X} RBP={:016X} R8 ={:016X}\n",
                r.rsi, r.rdi, r.rbp, r.r8
            ));
            pantalla::escribir(format_args!(
                "R9 ={:016X} R10={:016X} R11={:016X} R12={:016X}\n",
                r.r9, r.r10, r.r11, r.r12
            ));
            pantalla::escribir(format_args!(
                "R13={:016X} R14={:016X} R15={:016X}\n",
                r.r13, r.r14, r.r15
            ));
        }
        None => pantalla::escribir(format_args!(
            "Registros: no capturados (no hay marco de interrupcion)\n"
        )),
    }
    match d.pid {
        Some(p) => pantalla::escribir(format_args!("Tarea   : PID {}\n", p)),
        None => pantalla::escribir(format_args!("Tarea   : no determinable\n")),
    }
    pantalla::escribir(format_args!("\n"));
    pantalla::escribir(format_args!(
        "El sistema se ha detenido para evitar mas danos.\n"
    ));
    pantalla::escribir(format_args!(
        "Ruta de emergencia de rocky: sin GUI, planificador ni heap.\n"
    ));
}

/// Parada segura: con las interrupciones ya apagadas, deja el procesador
/// detenido en un bucle `hlt` hasta una NMI o un reinicio. **No** se intenta
/// reanudar el nucleo: su estado puede estar corrupto.
fn parar() -> ! {
    crate::cpu::halt_loop()
}

/// Lee `CR2` (direccion del `#PF`). Solo tiene sentido tras un `#PF`.
fn leer_cr2() -> Option<u64> {
    let v: u64;
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) };
    Some(v)
}

/// Lee `CR3` (base de la tabla de paginas en curso). Leerlo no tiene efectos.
fn leer_cr3() -> Option<u64> {
    let v: u64;
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) };
    Some(v)
}

/// Formatea un valor de 64 bits en hex, o "no capturado".
struct Hex(Option<u64>);

impl fmt::Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{:016X}", v),
            None => f.write_str("no capturado"),
        }
    }
}

/// Igual que [`Hex`] pero para un selector de 16 bits.
struct HexCorto(Option<u64>);

impl fmt::Display for HexCorto {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{:#06X}", v),
            None => f.write_str("no capturado"),
        }
    }
}

/// Escritor de tamano fijo para formatear sin heap.
struct BufferTexto<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> BufferTexto<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        BufferTexto { buf, len: 0 }
    }
}

impl<'a> fmt::Write for BufferTexto<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let hueco = self.buf.len() - self.len;
        let n = s.len().min(hueco);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Pruebas controladas (orden `panico` del interprete)
// ---------------------------------------------------------------------------

/// `panic!` de Rust: ejercita el camino del `#[panic_handler]`.
pub fn prueba_panico() -> ! {
    panic!("prueba de panico controlada (orden 'panico kernel')");
}

/// Instruccion invalida (`ud2`): provoca `#UD`.
pub fn prueba_ud() -> ! {
    unsafe { asm!("ud2", options(noreturn)) };
}

/// Acceso a una direccion **no canonica**: provoca `#GP`.
pub fn prueba_gp() -> ! {
    unsafe {
        asm!(
            "mov rax, {dir}",
            "mov rbx, [rax]",
            dir = in(reg) 0xDEAD_BEEF_0000_0000u64,
            options(noreturn),
        );
    }
}

/// Acceso a una direccion **canonica sin mapear**: provoca `#PF` (CR2 apuntara
/// a `dir`).
pub fn prueba_pf() -> ! {
    unsafe {
        asm!(
            "mov rax, {dir}",
            "mov rbx, [rax]",
            dir = in(reg) 0xFFFF_C000_0000_0000u64,
            options(noreturn),
        );
    }
}

/// Activa un modo VESA y provoca un `panic!`: la pantalla azul debe pintarse en
/// el **framebuffer** (no en la memoria de texto). Es la prueba del camino
/// grafico; si el equipo no ofrece ningun modo VESA, cae al texto y lo avisa.
pub fn prueba_framebuffer() -> ! {
    match crate::gfx::find_mode(800, 600).or_else(crate::gfx::find_first_demo_mode) {
        Some((modo, _)) => {
            let _ = crate::gfx::set_mode(modo);
            let _ = crate::bios::get_font();
        }
        None => {
            // Sin VESA no hay framebuffer: el panico saldra por texto, que es la
            // alternativa prevista.
            panic!("prueba de panico en modo grafico, pero sin VESA (se usa texto)");
        }
    }
    panic!("prueba de panico controlada en modo grafico (orden 'panico fb')");
}

/// Lleva `rsp` a una direccion no canonica y accede a la pila: el primer fallo
/// no se puede entregar en una pila invalida, asi que el procesador escala a
/// doble fallo (`#DF`), que corre en la pila de emergencia (IST1).
pub fn prueba_df() -> ! {
    unsafe {
        asm!(
            "mov rsp, {malo}",
            "push rax",
            malo = in(reg) 0x8000_0000_0000_0000u64,
            options(noreturn),
        );
    }
}
