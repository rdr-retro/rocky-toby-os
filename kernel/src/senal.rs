//! Señales: el `kernel/signal.c` de Linux 0.01, portado a rocky.
//!
//! Es la **Fase 7** del portado. Una señal es un aviso **asincrono** que el
//! nucleo deja pendiente sobre un proceso; al volver al anillo 3 (tras la
//! llamada al sistema o la excepcion en la que se encolo) el proceso o bien
//! **muere** (accion por defecto), o bien **ignora** la senal, o bien salta a un
//! **manejador** suyo que corre en anillo 3 y vuelve con `sigreturn`.
//!
//! Es el mismo modelo que Linux 0.01:
//!
//! | Linux 0.01 (`kernel/signal.c`)          | rocky                                      |
//! |-----------------------------------------|--------------------------------------------|
//! | `current->signal` (bitmap pendiente)     | [`Senales::pendientes`]                     |
//! | `current->blocked`                       | [`Senales::bloqueadas`]                     |
//! | `current->sigaction[signr-1]`            | [`Senales::acciones`]                       |
//! | `sys_kill`                                | [`kill`] + [`crate::tarea::enviar_senal`]   |
//! | `sys_signal` / `sys_sigaction`            | `sigaction` (syscall 13)                    |
//! | `sys_sigreturn`                           | [`sigreturn`] (syscall 15)                  |
//! | `do_signal`                               | [`comprobar`]                               |
//! | accion por defecto (`do_exit(1<<signr)`)  | [`terminar`] (`abortar`) / [`Def::Parar`]   |
//!
//! Diferencias conscientes con Linux 0.01:
//!
//! * rocky solo entrega **una senal a la vez**: mientras un manejador corre, no
//!   se entrega otra (Linux permite anidar). Es suficiente para la fase y evita
//!   tener que apilar varios contextos.
//! * No hay grupos de proceso: `kill(0, sig)` es "a mi mismo", no a un grupo.
//! * Las llamadas al sistema **bloqueantes** (`read` de tuberia, `wait4`) no se
//!   interrumpen con `EINTR`; solo `pause` despierta con una senal.

use crate::idt::InterruptStack;
use crate::println;
use crate::tarea;

// --- Numeros de senal (los mismos que Linux) --------------------------------

pub const SIGHUP: u32 = 1;
pub const SIGINT: u32 = 2;
pub const SIGQUIT: u32 = 3;
pub const SIGILL: u32 = 4;
pub const SIGTRAP: u32 = 5;
pub const SIGABRT: u32 = 6;
pub const SIGBUS: u32 = 7;
pub const SIGFPE: u32 = 8;
pub const SIGKILL: u32 = 9;
pub const SIGUSR1: u32 = 10;
pub const SIGSEGV: u32 = 11;
pub const SIGUSR2: u32 = 12;
pub const SIGPIPE: u32 = 13;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;
pub const SIGCHLD: u32 = 17;
pub const SIGCONT: u32 = 18;
pub const SIGSTOP: u32 = 19;
pub const SIGTSTP: u32 = 20;

/// Numero maximo de senal que rocky maneja (mapa de bits de 32).
const MAX_SENAL: u32 = 31;

/// Bit de `SIGKILL` en la mascara (no se puede bloquear ni capturar).
const SIGKILL_BIT: u32 = 1 << (SIGKILL - 1);

/// Banderas de `sa_flags` (los mismos valores que Linux 0.01).
pub const SA_ONESHOT: u64 = 0x8000_0000;
pub const SA_NOMASK: u64 = 0x4000_0000;

/// `struct sigaction` de rocky: manejador, restaurador y banderas.
///
/// Los valores especiales del manejador son los de Unix: `0` = accion por
/// defecto (`SIG_DFL`) y `1` = ignorar (`SIG_IGN`). El **restaurador** es el
/// codigo de anillo 3 al que el manejador regresa y que ejecuta la llamada
/// `sigreturn` (el `sa_restorer` de la `libc` de Linux).
#[derive(Clone, Copy)]
pub struct Accion {
    pub manejador: u64,
    pub restaurador: u64,
    pub banderas: u64,
}

impl Accion {
    /// Accion por defecto (`SIG_DFL`).
    pub const DEFECTO: Accion = Accion {
        manejador: 0,
        restaurador: 0,
        banderas: 0,
    };
}

/// Estado de senales de una tarea (el `sigaction[]`/`signal`/`blocked` de Linux).
#[derive(Clone, Copy)]
pub struct Senales {
    /// Mapa de bits de senales **pendientes** (bit `n-1` = senal `n`).
    pub pendientes: u32,
    /// Mapa de bits de senales **bloqueadas**.
    pub bloqueadas: u32,
    /// Acciones por senal (indice `n-1`).
    pub acciones: [Accion; MAX_SENAL as usize],
    /// Senal cuyo manejador esta corriendo ahora mismo (0 = ninguna). Mientras
    /// no sea 0 no se entrega ninguna otra senal.
    pub actual: u32,
    /// Mascara de bloqueo a restaurar cuando el manejador haga `sigreturn`.
    pub mascara_previa: u32,
    /// Tick del reloj en que debe dispararse `SIGALRM` (0 = sin alarma).
    pub alarma: u64,
    /// La tarea esta bloqueada en `pause` esperando una senal.
    pub pause: bool,
    /// La tarea esta **parada** (SIGSTOP/SIGTSTP); la reanuda SIGCONT.
    pub parado: bool,
}

impl Senales {
    /// Estado limpio: sin pendientes, sin bloquear y todo a la accion por defecto.
    pub const fn nuevo() -> Senales {
        Senales {
            pendientes: 0,
            bloqueadas: 0,
            acciones: [Accion::DEFECTO; MAX_SENAL as usize],
            actual: 0,
            mascara_previa: 0,
            alarma: 0,
            pause: false,
            parado: false,
        }
    }

    /// Deja pendiente la senal `signr`.
    pub fn encolar(&mut self, signr: u32) {
        if (1..=MAX_SENAL).contains(&signr) {
            self.pendientes |= 1 << (signr - 1);
        }
    }

    /// Hay alguna senal pendiente que no este bloqueada (SIGKILL cuenta siempre).
    pub fn hay_desbloqueada(&self) -> bool {
        self.tomas().is_some()
    }

    /// Bitmap de senales que se entregarian ahora mismo.
    fn tomas(&self) -> Option<u32> {
        let m = (self.pendientes & !self.bloqueadas) | (self.pendientes & SIGKILL_BIT);
        if m == 0 {
            None
        } else {
            Some(m)
        }
    }

    /// Toma la senal pendiente de menor numero que no este bloqueada y la quita
    /// de pendientes (SIGKILL nunca se considera bloqueada).
    fn tomar(&mut self) -> Option<u32> {
        let m = self.tomas()?;
        let signr = m.trailing_zeros() + 1;
        self.pendientes &= !(1 << (signr - 1));
        Some(signr)
    }

    /// Accion instalada para `signr`.
    pub fn accion(&self, signr: u32) -> Accion {
        self.acciones[(signr - 1) as usize]
    }

    /// Instala una accion para `signr`.
    pub fn poner_accion(&mut self, signr: u32, a: Accion) {
        self.acciones[(signr - 1) as usize] = a;
    }

    /// Aplica `sigprocmask` (`how`: 0 = bloquear, 1 = desbloquear, 2 = fijar).
    pub fn aplicar_mascara(&mut self, how: u32, mascara: u32) {
        match how {
            0 => self.bloqueadas |= mascara,
            1 => self.bloqueadas &= !mascara,
            2 => self.bloqueadas = mascara,
            _ => {}
        }
        // SIGKILL no se puede bloquear nunca.
        self.bloqueadas &= !SIGKILL_BIT;
    }

    /// Estado del hijo tras `fork`: hereda las acciones y la mascara, pero no las
    /// senales pendientes (como Linux).
    pub fn heredar(&self) -> Senales {
        Senales {
            pendientes: 0,
            bloqueadas: self.bloqueadas,
            acciones: self.acciones,
            actual: 0,
            mascara_previa: 0,
            alarma: 0,
            pause: false,
            parado: false,
        }
    }

    /// Estado tras `execve`: se olvidan pendientes y alarmas y los manejadores
    /// vuelven a la accion por defecto (los "ignorar" se conservan, como Linux).
    pub fn reiniciar(&mut self) {
        self.pendientes = 0;
        self.actual = 0;
        self.mascara_previa = 0;
        self.alarma = 0;
        self.pause = false;
        self.parado = false;
        for a in self.acciones.iter_mut() {
            if a.manejador != 1 {
                *a = Accion::DEFECTO;
            }
        }
    }
}

/// Nombre legible de una senal.
pub fn nombre(signr: u32) -> &'static str {
    match signr {
        SIGHUP => "SIGHUP",
        SIGINT => "SIGINT",
        SIGQUIT => "SIGQUIT",
        SIGILL => "SIGILL",
        SIGTRAP => "SIGTRAP",
        SIGABRT => "SIGABRT",
        SIGBUS => "SIGBUS",
        SIGFPE => "SIGFPE",
        SIGKILL => "SIGKILL",
        SIGUSR1 => "SIGUSR1",
        SIGSEGV => "SIGSEGV",
        SIGUSR2 => "SIGUSR2",
        SIGPIPE => "SIGPIPE",
        SIGALRM => "SIGALRM",
        SIGTERM => "SIGTERM",
        SIGCHLD => "SIGCHLD",
        SIGCONT => "SIGCONT",
        SIGSTOP => "SIGSTOP",
        SIGTSTP => "SIGTSTP",
        _ => "SIG?",
    }
}

/// Traduce una excepcion del procesador a la senal que genera (el `traps.c` de
/// Linux). Solo para las que un programa de anillo 3 puede provocar.
pub fn desde_excepcion(vector: u64) -> Option<u32> {
    Some(match vector {
        0 => SIGFPE,        // #DE
        1 | 3 => SIGTRAP,   // #DB, #BP
        4 | 5 => SIGSEGV,   // #OF, #BR
        6 => SIGILL,        // #UD
        7 => SIGSEGV,       // #NM
        13 | 14 => SIGSEGV, // #GP, #PF
        16 => SIGFPE,       // #MF
        17 => SIGBUS,       // #AC
        _ => return None,
    })
}

/// Indica si el proceso en curso tiene instalado un manejador **entregable** para
/// `signr` (no es accion por defecto, no es "ignorar" y no esta bloqueada).
pub fn capturada(signr: u32) -> bool {
    tarea::con_senales(|s| {
        let h = s.accion(signr).manejador;
        h != 0 && h != 1 && (s.bloqueadas & (1 << (signr - 1))) == 0
    })
    .unwrap_or(false)
}

/// Deja pendiente `signr` en el proceso en curso (lo usa la ruta de excepcion).
pub fn encolar_actual(signr: u32) {
    let _ = tarea::con_senales(|s| s.encolar(signr));
}

/// Resultado de la accion por defecto de una senal.
enum Def {
    /// Ignorar (SIGCHLD, SIGCONT).
    Ignorar,
    /// Parar el proceso (SIGSTOP, SIGTSTP): lo reanuda SIGCONT.
    Parar,
    /// Terminar el proceso.
    Terminar,
}

fn accion_por_defecto(signr: u32) -> Def {
    match signr {
        SIGCHLD | SIGCONT => Def::Ignorar,
        SIGSTOP | SIGTSTP => Def::Parar,
        _ => Def::Terminar,
    }
}

/// Numero de palabras de contexto de usuario que guarda el marco de senal.
const MAX_SENAL_PALABRAS: usize = 18;

/// Entrega la primera senal pendiente al `marco` que esta a punto de volver al
/// anillo 3. Devuelve el marco desde el que debe continuar la tarea: el mismo
/// (accion ignorada, o manejador preparado) o el de otra tarea (si el proceso
/// muere o se para).
///
/// Este es el punto unico de entrega: lo llama [`crate::idt::isr_dispatch`] al
/// final de **toda** interrupcion, de modo que una senal se vea en cuanto la
/// tarea vuelve a correr (como el `ret_from_sys_call` de Linux).
pub fn comprobar(marco: *mut InterruptStack) -> *mut InterruptStack {
    loop {
        // Solo se entregan senales al anillo 3.
        if unsafe { (*marco).cs & 3 } != 3 {
            return marco;
        }
        // Toma la siguiente senal entregable; si ya hay un manejador corriendo,
        // no se entrega otra.
        let signr = match tarea::con_senales(|s| if s.actual != 0 { None } else { s.tomar() }) {
            Some(Some(s)) => s,
            _ => return marco,
        };

        let accion = tarea::con_senales(|s| s.accion(signr)).unwrap_or(Accion::DEFECTO);
        let manejador = accion.manejador;

        // SIGKILL no se puede capturar, ignorar ni bloquear.
        if signr == SIGKILL {
            return terminar(marco, signr);
        }
        if manejador == 1 {
            continue; // SIG_IGN
        }
        if manejador == 0 {
            match accion_por_defecto(signr) {
                Def::Ignorar => continue,
                Def::Parar => {
                    crate::tarea::parar_actual();
                    return crate::tarea::planificar(marco);
                }
                Def::Terminar => return terminar(marco, signr),
            }
        }

        // Manejador del usuario: se salva su contexto en su pila y se le salta.
        if !construir_marco(marco, signr, manejador, accion.restaurador) {
            // Sin sitio en la pila: se aplica la accion por defecto.
            return terminar(marco, signr);
        }
        let _ = tarea::con_senales(|s| {
            s.actual = signr;
            s.mascara_previa = s.bloqueadas;
            if accion.banderas & SA_NOMASK == 0 {
                s.bloqueadas |= 1 << (signr - 1);
            }
            if accion.banderas & SA_ONESHOT != 0 {
                s.poner_accion(signr, Accion::DEFECTO);
            }
        });
        return marco;
    }
}

/// Accion por defecto "terminar": informa y deja al proceso morir con el codigo
/// convencional de "muerto por senal" (`128 + señal`).
fn terminar(marco: *mut InterruptStack, signr: u32) -> *mut InterruptStack {
    let pid = crate::tarea::pid_actual().unwrap_or(0);
    println!(
        "rocky: PID {} terminado por la senal {} ({})",
        pid,
        signr,
        nombre(signr)
    );
    crate::tarea::abortar(marco, 128 + signr as i64)
}

/// Salva el contexto de anillo 3 en su pila y redirige la tarea al manejador.
///
/// El marco en la pila del usuario, de menor a mayor direccion, es:
///
/// ```text
///   [ restaurador ]                     <- 1 palabra (a donde vuelve el manejador)
///   [ r15 r14 r13 r12 r11 r10 r9 r8 ]   \
///   [ rdi rsi rbp rbx rdx rcx rax  ]    |  contexto salvado (18 palabras)
///   [ rip rflags rsp               ]   /
/// ```
///
/// El manejador arranca con `rsp` apuntando al restaurador: al hacer `ret` salta
/// a el y `rsp` queda apuntando al contexto, que `sigreturn` restaura.
fn construir_marco(
    marco: *mut InterruptStack,
    signr: u32,
    manejador: u64,
    restaurador: u64,
) -> bool {
    let f = unsafe { &mut *marco };
    let palabras = (MAX_SENAL_PALABRAS + 1) as u64; // contexto + direccion de vuelta
    let base = f.rsp.wrapping_sub(palabras * 8);
    if !crate::proceso::poblar_rango_escritura(base, palabras * 8) {
        return false;
    }
    let w: [u64; MAX_SENAL_PALABRAS] = [
        f.r15, f.r14, f.r13, f.r12, f.r11, f.r10, f.r9, f.r8, f.rdi, f.rsi, f.rbp, f.rbx, f.rdx,
        f.rcx, f.rax, f.rip, f.rflags, f.rsp,
    ];
    unsafe {
        let p = base as *mut u64;
        core::ptr::write_volatile(p, restaurador);
        for (i, v) in w.iter().enumerate() {
            core::ptr::write_volatile(p.add(1 + i), *v);
        }
        f.rip = manejador;
        f.rsp = base;
        f.rdi = signr as u64;
    }
    true
}

/// Restaura el contexto que salvo [`construir_marco`] (llamada `sigreturn`).
///
/// Devuelve `false` si la pila del usuario no es accesible (entonces la llama
/// `sigreturn` termina el proceso).
pub fn sigreturn(marco: *mut InterruptStack) -> bool {
    let base = unsafe { (*marco).rsp };
    if !crate::proceso::poblar_rango(base, MAX_SENAL_PALABRAS as u64 * 8) {
        return false;
    }
    let mut w = [0u64; MAX_SENAL_PALABRAS];
    unsafe {
        let p = base as *const u64;
        for (i, v) in w.iter_mut().enumerate() {
            *v = core::ptr::read_volatile(p.add(i));
        }
        (*marco).r15 = w[0];
        (*marco).r14 = w[1];
        (*marco).r13 = w[2];
        (*marco).r12 = w[3];
        (*marco).r11 = w[4];
        (*marco).r10 = w[5];
        (*marco).r9 = w[6];
        (*marco).r8 = w[7];
        (*marco).rdi = w[8];
        (*marco).rsi = w[9];
        (*marco).rbp = w[10];
        (*marco).rbx = w[11];
        (*marco).rdx = w[12];
        (*marco).rcx = w[13];
        (*marco).rax = w[14];
        (*marco).rip = w[15];
        (*marco).rflags = w[16];
        (*marco).rsp = w[17];
    }
    // Se sale del manejador: se restaura la mascara y se deja entregar otra senal.
    let _ = tarea::con_senales(|s| {
        s.actual = 0;
        s.bloqueadas = s.mascara_previa;
        s.mascara_previa = 0;
    });
    true
}

// --- Llamadas al sistema ----------------------------------------------------

/// `sigaction(signum, act, oldact)`: instala (y devuelve la anterior) la accion
/// de una senal. `act` y `oldact` son punteros a un `struct sigaction` de 3
/// palabras (`manejador`, `restaurador`, `banderas`).
pub fn sigaction(signum: u64, act: u64, old: u64) -> i64 {
    let signr = signum as u32;
    if signr < 1 || signr > MAX_SENAL || signr == SIGKILL || signr == SIGSTOP {
        return -1;
    }
    let nueva = if act != 0 {
        if !crate::proceso::poblar_rango(act, 24) {
            return -1;
        }
        Some(unsafe {
            Accion {
                manejador: core::ptr::read_volatile(act as *const u64),
                restaurador: core::ptr::read_volatile((act as *const u64).add(1)),
                banderas: core::ptr::read_volatile((act as *const u64).add(2)),
            }
        })
    } else {
        None
    };
    let vieja = tarea::con_senales(|s| {
        let v = s.accion(signr);
        if let Some(a) = nueva {
            s.poner_accion(signr, a);
        }
        v
    })
    .unwrap_or(Accion::DEFECTO);
    if old != 0 {
        if !crate::proceso::poblar_rango_escritura(old, 24) {
            return -1;
        }
        unsafe {
            core::ptr::write_volatile(old as *mut u64, vieja.manejador);
            core::ptr::write_volatile((old as *mut u64).add(1), vieja.restaurador);
            core::ptr::write_volatile((old as *mut u64).add(2), vieja.banderas);
        }
    }
    0
}

/// `sigprocmask(how, set, oldset)`: consulta o cambia la mascara de bloqueo.
pub fn sigprocmask(how: u64, set: u64, old: u64) -> i64 {
    let nueva = if set != 0 {
        if !crate::proceso::poblar_rango(set, 8) {
            return -1;
        }
        unsafe { core::ptr::read_volatile(set as *const u64) as u32 }
    } else {
        0
    };
    let vieja = tarea::con_senales(|s| {
        let v = s.bloqueadas;
        if set != 0 {
            s.aplicar_mascara(how as u32, nueva);
        }
        v
    })
    .unwrap_or(0);
    if old != 0 {
        if !crate::proceso::poblar_rango_escritura(old, 8) {
            return -1;
        }
        unsafe { core::ptr::write_volatile(old as *mut u64, vieja as u64) };
    }
    0
}

/// `kill(pid, sig)`: deja pendiente `sig` en otro proceso (o en el propio si
/// `pid == 0`). Devuelve `0` o `-1`.
pub fn kill(pid: i64, signum: u64) -> i64 {
    let signr = signum as u32;
    if signr < 1 || signr > MAX_SENAL {
        return -1;
    }
    let destino = if pid == 0 {
        crate::tarea::pid_actual().unwrap_or(0) as i64
    } else {
        pid
    };
    if destino <= 0 {
        return -1;
    }
    if crate::tarea::enviar_senal(destino as u64, signr) {
        0
    } else {
        -1
    }
}

/// `alarm(seconds)`: programa `SIGALRM` dentro de `seconds` segundos; devuelve
/// los segundos que quedaban de la alarma anterior.
pub fn alarm(segundos: u64) -> u64 {
    tarea::poner_alarma(segundos)
}
