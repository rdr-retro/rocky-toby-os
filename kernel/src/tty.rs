//! TTY: la consola vista como **terminal**, con disciplina de linea y `termios`.
//!
//! Es la **Fase 8** del portado, el `kernel/tty_io.c` (y el `drivers/char/` de
//! las versiones siguientes) de Linux 0.01. Hasta aqui la consola (los
//! descriptores 0, 1 y 2) era un simple "teclado y pantalla": [`crate::fd::leer`]
//! leia pulsaciones y las echaba tal cual. Un terminal de Unix, en cambio, tiene
//! una **disciplina de linea** entre el teclado y el programa:
//!
//! * en **modo canonico** (`ICANON`) el programa no ve los caracteres segun se
//!   teclean, sino **linea a linea**: el nucleo acumula la linea, deja editar
//!   (retroceso y matar linea) y solo la entrega al llegar el `\n`;
//! * el **eco** (`ECHO`) lo pinta el nucleo, no el programa;
//! * los **caracteres de control** disparan trabajo del terminal (`ISIG`:
//!   `^C`/`^\`/`^Z` mandan una senal al grupo en primer plano);
//! * el comportamiento se configura con un `struct termios` (banderas de
//!   entrada, salida, control y locales, mas los caracteres de control `c_cc`),
//!   que el programa lee y escribe con `ioctl`.
//!
//! Y un terminal tiene **sesiones y grupos de procesos**: `setsid` crea una
//! sesion nueva y `TIOCSPGRP`/`TIOCGPGRP` gestionan el grupo en primer plano, al
//! que el terminal dirige las senales de control.
//!
//! Es el mismo modelo que Linux 0.01:
//!
//! | Linux 0.01 (`kernel/tty_io.c`)              | rocky                                    |
//! |---------------------------------------------|------------------------------------------|
//! | `struct tty_struct`                          | [`Tty`]                                  |
//! | `struct termios` (`include/termios.h`)       | [`Termios`]                              |
//! | `tty_read` / `tty_write` (disciplina N_TTY)  | [`leer`] / [`escribir`]                  |
//! | `tty_ioctl`                                  | [`ioctl`]                                |
//! | `sys_setsid`                                 | [`setsid`]                               |
//! | `current->tty` / `pgrp` (primer plano)       | [`Tarea::pgrp`](crate::tarea::Tarea)     |
//!
//! Diferencias conscientes con Linux 0.01:
//!
//! * rocky todavia no tiene **puestos de trabajo** (job control): el grupo en
//!   primer plano se fija a mano con `TIOCSPGRP` y no hay comprobacion de
//!   permisos sobre el terminal.
//! * El modo no canonico no espera con `VMIN`/`VTIME`: devuelve en cuanto hay
//!   `VMIN` bytes (con `VMIN = 1` por defecto, en cuanto llega uno).
//! * Solo existe **un** terminal (la consola); no hay `/dev/tty0`... ni
//!   pseudoterminales (`pty`).
//! * Los caracteres de entrada llegan del teclado y del puerto serie, que rocky
//!   todavia no traduce a caracteres de control (no hay `^C` tecleable).

use crate::sync::SyncCell;

/// Numero de caracteres de control de `c_cc` (Linux 0.01: `NCCS`).
pub const NCCS: usize = 17;

// Indices dentro de `c_cc` (los mismos que Linux 0.01, `include/termios.h`).
pub const VINTR: usize = 0;
pub const VQUIT: usize = 1;
pub const VERASE: usize = 2;
pub const VKILL: usize = 3;
pub const VEOF: usize = 4;
pub const VMIN: usize = 6;
pub const VSUSP: usize = 10;
pub const VEOL: usize = 11;

// --- Banderas de `c_iflag` (entrada) ---
/// Ignorar el retorno de carro.
pub const IGNCR: u32 = 0o000200;
/// Traducir el retorno de carro a salto de linea.
pub const ICRNL: u32 = 0o000400;
/// Control de flujo por `VSTART`/`VSTOP` (`XON`/`XOFF`).
pub const IXON: u32 = 0o002000;

// --- Banderas de `c_oflag` (salida) ---
/// Procesar la salida (es lo que activa [`ONLCR`]).
pub const OPOST: u32 = 0o000001;
/// Al escribir `\n`, mandar antes `\r`.
pub const ONLCR: u32 = 0o000004;

// --- Banderas de `c_cflag` (control del puerto) ---
/// Ocho bits de datos (en `CSIZE`).
pub const CS8: u32 = 0o000060;
/// Habilitar la recepcion.
pub const CREAD: u32 = 0o000200;
/// Bajar la linea al cerrar.
pub const HUPCL: u32 = 0o002000;
/// Velocidad 9600 baudios.
pub const B9600: u32 = 0o000015;

// --- Banderas de `c_lflag` (locales) ---
/// Los caracteres de control generan senales.
pub const ISIG: u32 = 0o000001;
/// Modo canonico: se lee linea a linea.
pub const ICANON: u32 = 0o000002;
/// Eco de lo que se teclea.
pub const ECHO: u32 = 0o000010;
/// El eco del retroceso borra el caracter.
pub const ECHOE: u32 = 0o000020;
/// El eco de matar linea borra la linea.
pub const ECHOK: u32 = 0o000040;
/// Los caracteres de control se echan como `^X`.
pub const ECHOCTL: u32 = 0o001000;
/// El eco de `VKILL`/`VERASE` borra lo que quito.
pub const ECHOKE: u32 = 0o004000;
/// Extensiones de entrada de la implementacion.
pub const IEXTEN: u32 = 0o100000;

// --- Peticiones de `ioctl` (los mismos numeros que Linux) ---
/// Leer el `termios` del terminal.
pub const TCGETS: u64 = 0x5401;
/// Fijar el `termios` del terminal.
pub const TCSETS: u64 = 0x5402;
/// Como [`TCSETS`], esperando a que se vacie la salida.
pub const TCSETSW: u64 = 0x5403;
/// Como [`TCSETS`], vaciando ademas la entrada.
pub const TCSETSF: u64 = 0x5404;
/// Leer el grupo de procesos en primer plano.
pub const TIOCGPGRP: u64 = 0x540F;
/// Fijar el grupo de procesos en primer plano.
pub const TIOCSPGRP: u64 = 0x5410;
/// Leer el tamano de la ventana (`struct winsize`: cuatro `u16`).
pub const TIOCGWINSZ: u64 = 0x5413;

/// El `struct termios` que ven los programas (el de Linux 0.01, con los campos
/// de 32 bits de rocky).
///
/// El orden y los desplazamientos son los de Linux: `c_iflag` (0), `c_oflag`
/// (4), `c_cflag` (8), `c_lflag` (12), `c_line` (16) y `c_cc` (17). Un programa
/// que solo mire `c_lflag` puede contar con el desplazamiento 12.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Termios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; NCCS],
}

impl Termios {
    /// El ajuste por defecto de un terminal recien abierto.
    ///
    /// Es el de Linux (`tty_io.c`): `\r` se convierte en `\n` a la entrada, la
    /// salida convierte `\n` en `\r\n`, hay modo canonico y eco, y los caracteres
    /// de control estan activos. El unico cambio consciente es `VERASE`: el
    /// teclado de rocky manda `0x08` (retroceso) donde Linux manda `0x7F`.
    pub const fn defecto() -> Termios {
        Termios {
            c_iflag: ICRNL | IXON,
            c_oflag: OPOST | ONLCR,
            c_cflag: B9600 | CS8 | CREAD | HUPCL,
            c_lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHOCTL | ECHOKE | IEXTEN,
            c_line: 0,
            c_cc: [
                3,    // VINTR  ^C
                28,   // VQUIT  ^\
                0x08, // VERASE retroceso (el teclado de rocky manda 0x08)
                21,   // VKILL  ^U
                4,    // VEOF   ^D
                0,    // VTIME
                1,    // VMIN
                0,    // VSWTC
                17,   // VSTART ^Q
                19,   // VSTOP  ^S
                26,   // VSUSP  ^Z
                0,    // VEOL
                18,   // VREPRINT ^R
                15,   // VDISCARD ^O
                23,   // VWERASE  ^W
                22,   // VLNEXT   ^V
                0,    // VEOL2
            ],
        }
    }
}

/// Estado del terminal de la consola.
struct Tty {
    /// Ajustes en vigor ([`Termios`]).
    termios: Termios,
    /// Grupo de procesos **en primer plano**: a el van las senales de control
    /// (`^C`...) y el trabajo del terminal. 0 = ninguno fijado.
    pgrp_frente: u64,
    /// Hay un programa dentro de [`leer`] (leyendo de la consola). Lo consulta
    /// el interprete de ordenes para no robarle el teclado.
    ocupado: bool,
}

static CONSOLA: SyncCell<Tty> = SyncCell::new(Tty {
    termios: Termios::defecto(),
    pgrp_frente: 0,
    ocupado: false,
});

/// Devuelve una copia de los ajustes en vigor.
pub fn termios() -> Termios {
    unsafe { (*CONSOLA.get()).termios }
}

/// Instala unos ajustes nuevos.
pub fn poner_termios(t: Termios) {
    unsafe { (*CONSOLA.get()).termios = t };
}

/// Indica si alguien esta leyendo de la consola (esta dentro de [`leer`]).
pub fn ocupado() -> bool {
    unsafe { (*CONSOLA.get()).ocupado }
}

fn marcar_ocupado(v: bool) {
    unsafe { (*CONSOLA.get()).ocupado = v };
}

/// Escribe en la consola aplicando el procesado de salida (`OPOST`/`ONLCR`).
///
/// Es el `tty_write` de Linux: el programa escribe lo que quiere y el terminal
/// decide como se ve. Con `OPOST | ONLCR` (lo normal) cada `\n` sale como `\r\n`,
/// que es lo que hace falta para que el cursor vuelva al margen.
pub fn escribir(datos: &[u8]) {
    let t = termios();
    for &b in datos {
        if t.c_oflag & OPOST != 0 && b == b'\n' && t.c_oflag & ONLCR != 0 {
            crate::console::put(b'\r');
        }
        crate::console::put(b);
    }
}

/// Lee una linea (o lo que pida el modo) de la consola, aplicando la disciplina
/// de linea. Devuelve los bytes leidos o `-1`.
///
/// Bloquea cediendo el turno mientras no haya teclas; la entrega al programa la
/// decide [`termios`]:
///
/// * **canonico** (`ICANON`): acumula en `buf` y solo vuelve al llegar `\n` (o
///   al llenarse `buf`). El retroceso y matar linea editan la linea y, con
///   `ECHO`, el nucleo los pinta.
/// * **no canonico**: vuelve en cuanto hay `VMIN` bytes (1 por defecto).
/// * `VEOF` sobre una linea vacia devuelve `0` (fin de fichero).
/// * `ISIG`: `^C`/`^\`/`^Z` mandan una senal al grupo en primer plano y la
///   lectura vuelve (como el `EINTR` de Linux), para que la senal se entregue.
pub fn leer(buf: &mut [u8]) -> i64 {
    marcar_ocupado(true);
    let r = leer_interno(buf);
    marcar_ocupado(false);
    r
}

fn leer_interno(buf: &mut [u8]) -> i64 {
    let t = termios();
    let mut n = 0usize;
    loop {
        if n >= buf.len() {
            return n as i64;
        }
        let c = match crate::input::poll() {
            Some(c) => c,
            None => {
                // Si al proceso en curso le han dejado una senal pendiente, la
                // lectura vuelve para que se le entregue (es lo que hace que
                // `kill` pueda desbloquear a un lector).
                if crate::tarea::senal_pendiente() {
                    return n as i64;
                }
                unsafe { crate::tarea::ceder() };
                continue;
            }
        };

        // Caracteres de control que generan senales (`ISIG`).
        if t.c_lflag & ISIG != 0 {
            if let Some(s) = isig(&t, c) {
                senal_frente(s);
                return n as i64;
            }
        }

        // Procesado de entrada: `ICRNL` convierte el retorno de carro en salto
        // de linea; con `IGNCR` el retorno de carro se descarta.
        let mut c = c;
        if c == b'\r' {
            if t.c_iflag & IGNCR != 0 {
                continue;
            }
            if t.c_iflag & ICRNL != 0 {
                c = b'\n';
            }
        }

        if t.c_lflag & ICANON != 0 {
            if c == t.c_cc[VERASE] || c == 0x08 {
                if n > 0 {
                    n -= 1;
                    if t.c_lflag & ECHO != 0 {
                        escribir(b"\x08 \x08");
                    }
                }
                continue;
            }
            if c == t.c_cc[VKILL] {
                while n > 0 {
                    n -= 1;
                    if t.c_lflag & ECHO != 0 {
                        escribir(b"\x08 \x08");
                    }
                }
                continue;
            }
            if c == t.c_cc[VEOF] {
                if n > 0 {
                    return n as i64; // entrega la linea acumulada (sin el VEOF)
                }
                return 0; // fin de fichero
            }
            if n >= buf.len() {
                return n as i64; // sin sitio: se entrega lo que hay
            }
            buf[n] = c;
            n += 1;
            if t.c_lflag & ECHO != 0 {
                escribir(&buf[n - 1..n]);
            }
            if c == b'\n' || (t.c_cc[VEOL] != 0 && c == t.c_cc[VEOL]) {
                return n as i64;
            }
        } else {
            // Modo no canonico: en cuanto hay `VMIN` caracteres.
            if n >= buf.len() {
                return n as i64;
            }
            buf[n] = c;
            n += 1;
            if t.c_lflag & ECHO != 0 {
                escribir(&buf[n - 1..n]);
            }
            let vmin = t.c_cc[VMIN].max(1) as usize;
            if n >= vmin {
                return n as i64;
            }
        }
    }
}

/// Senal que dispara un caracter de control segun `ISIG`, si es uno de ellos.
fn isig(t: &Termios, c: u8) -> Option<u32> {
    if c == t.c_cc[VINTR] {
        Some(crate::senal::SIGINT)
    } else if c == t.c_cc[VQUIT] {
        Some(crate::senal::SIGQUIT)
    } else if c == t.c_cc[VSUSP] {
        Some(crate::senal::SIGTSTP)
    } else {
        None
    }
}

/// Manda una senal al grupo en primer plano (o al proceso en curso si no hay
/// ninguno fijado): es lo que hace el terminal con `^C`, `^\` y `^Z`.
pub fn senal_frente(signr: u32) {
    let pgrp = unsafe { (*CONSOLA.get()).pgrp_frente };
    if pgrp != 0 {
        crate::tarea::enviar_senal_grupo(pgrp, signr);
    } else {
        let pid = crate::tarea::actual_id();
        if pid != 0 {
            crate::tarea::enviar_senal(pid, signr);
        }
    }
}

/// Grupo de procesos en primer plano (0 = ninguno).
pub fn pgrp_frente() -> u64 {
    unsafe { (*CONSOLA.get()).pgrp_frente }
}

/// `ioctl` sobre el terminal: es el `tty_ioctl` de Linux.
///
/// Solo las peticiones que rocky entiende; el resto devuelven `-1` (el `ENOTTY`
/// que hoy no se distingue).
pub fn ioctl(request: u64, arg: u64) -> i64 {
    let tam = core::mem::size_of::<Termios>() as u64;
    match request {
        TCGETS => {
            if !crate::proceso::poblar_rango_escritura(arg, tam) {
                return -1;
            }
            let t = termios();
            unsafe {
                core::ptr::copy_nonoverlapping(
                    &t as *const Termios as *const u8,
                    arg as *mut u8,
                    tam as usize,
                );
            }
            0
        }
        TCSETS | TCSETSW | TCSETSF => {
            if !crate::proceso::poblar_rango(arg, tam) {
                return -1;
            }
            let t = unsafe { core::ptr::read_unaligned(arg as *const Termios) };
            poner_termios(t);
            0
        }
        TIOCGPGRP => {
            if !crate::proceso::poblar_rango_escritura(arg, 4) {
                return -1;
            }
            unsafe { core::ptr::write_volatile(arg as *mut i32, pgrp_frente() as i32) };
            0
        }
        TIOCSPGRP => {
            if !crate::proceso::poblar_rango(arg, 4) {
                return -1;
            }
            let p = unsafe { core::ptr::read_volatile(arg as *const i32) } as u64;
            unsafe { (*CONSOLA.get()).pgrp_frente = p };
            0
        }
        TIOCGWINSZ => {
            if !crate::proceso::poblar_rango_escritura(arg, 8) {
                return -1;
            }
            // struct winsize { ws_row, ws_col, ws_xpixel, ws_ypixel }
            unsafe {
                let p = arg as *mut u16;
                core::ptr::write_volatile(p, 25);
                core::ptr::write_volatile(p.add(1), 80);
                core::ptr::write_volatile(p.add(2), 0);
                core::ptr::write_volatile(p.add(3), 0);
            }
            0
        }
        _ => -1,
    }
}

/// `setsid`: crea una **sesion** (y un grupo de procesos) nuevos para el proceso
/// en curso. Devuelve el nuevo grupo, o `-1` si el proceso ya era lider de su
/// grupo (Linux: un lider de grupo no puede crear una sesion).
pub fn setsid() -> i64 {
    let pid = crate::tarea::actual_id();
    if pid == 0 {
        return -1;
    }
    let ok = crate::tarea::con_grupo(|pgrp| {
        if *pgrp == pid {
            return false;
        }
        *pgrp = pid;
        true
    })
    .unwrap_or(false);
    if ok {
        pid as i64
    } else {
        -1
    }
}
