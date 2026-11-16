//! Descriptores de fichero: la capa "todo es un fichero" de Linux 0.01.
//!
//! Es la **Fase 4** del portado. Hasta ahora el nucleo leia y escribia ficheros
//! llamando directamente a [`crate::fs`] (con el inodo en la mano); un programa
//! de anillo 3 no tenia forma de quedarse con un fichero abierto ni de ir
//! leyendolo por trozos. Esta capa anade el modelo de Unix, calcado de Linux
//! 0.01:
//!
//! * una **tabla de ficheros abiertos**, global y de tamano fijo ([`NR_FILE`]),
//!   al estilo de `fs/file_table.c` (`struct file`: inodo, posicion y modo);
//! * una **tabla de descriptores por tarea** ([`NR_OPEN`] entradas), al estilo
//!   del `filp[]` de `struct task_struct`: cada entrada es un indice a la tabla
//!   global, o [`LIBRE`];
//! * los descriptores **0, 1 y 2** son la **consola** (teclado/pantalla), que no
//!   es un inodo: se representan con la variante [`Abierto::Consola`], no con una
//!   ranura de fichero.
//!
//! La tabla global cuenta **referencias**: `fork` copia la tabla de descriptores
//! del padre y **retiene** cada fichero (padre e hijo comparten la posicion, como
//! en Unix); `close` la suelta y libera la ranura cuando ya no la usa nadie.
//!
//! Las banderas de `open` y los `whence` de `lseek` son los de Linux 0.01
//! (`include/fcntl.h`).

use crate::sync::SyncCell;

/// Ficheros abiertos a la vez en todo el sistema (Linux 0.01: `NR_FILE`).
pub const NR_FILE: usize = 64;
/// Descriptores que puede tener una tarea (Linux 0.01: `NR_OPEN`).
pub const NR_OPEN: usize = 20;
/// Marca de "descriptor libre" en la tabla por tarea.
pub const LIBRE: u32 = u32::MAX;

// Banderas de `open` (Linux 0.01, `include/fcntl.h`).
pub const O_RDONLY: u64 = 0;
pub const O_WRONLY: u64 = 1;
pub const O_RDWR: u64 = 2;
pub const O_ACCMODE: u64 = 3;
pub const O_CREAT: u64 = 0o100;
pub const O_TRUNC: u64 = 0o1000;
pub const O_APPEND: u64 = 0o2000;

// Origenes de `lseek`.
pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;

/// Tipos de entrada de `linux_dirent64` (`d_type`), los `DT_*` de Linux.
pub const DT_DIR: u8 = 4;
pub const DT_REG: u8 = 8;

/// Un fichero abierto, o la consola.
#[derive(Clone, Copy)]
enum Abierto {
    /// El teclado y la pantalla (descriptores 0, 1 y 2).
    Consola,
    /// Un fichero del volumen, con su posicion de lectura/escritura.
    Archivo {
        registro: u32,
        pos: usize,
        leer: bool,
        escribir: bool,
    },
    /// Un extremo de una tuberia anonima (Fase 6). `id` es la tuberia en la
    /// tabla de [`crate::pipe`]; `escritura` distingue el extremo por el que se
    /// escribe del extremo por el que se lee. Los dos extremos de una tuberia
    /// comparten el buzon, no la posicion: por eso no hay aqui un `pos`.
    Tuberia { id: u32, escritura: bool },
    /// Una conexión con el servidor **Kira** (el servidor de ventanas del
    /// propio kernel). `id` es la conexión en [`crate::kira`]; el protocolo
    /// pasa por `read`/`write` sobre el descriptor, como en X11.
    Kira { id: u32 },
}

/// Una ranura de la tabla global: que hay abierto y cuantos descriptores la usan.
#[derive(Clone, Copy)]
struct Ranura {
    que: Abierto,
    /// Referencias vivas. `u32::MAX` marca las ranuras de consola, que duran
    /// siempre y no se liberan nunca.
    refs: u32,
}

/// La tabla global de ficheros abiertos.
static TABLA: SyncCell<[Option<Ranura>; NR_FILE]> = SyncCell::new([None; NR_FILE]);

/// Reserva las tres ranuras de la consola. Se llama una vez al arrancar.
pub fn init() {
    let t = unsafe { &mut *TABLA.get() };
    for s in t.iter_mut().take(3) {
        *s = Some(Ranura {
            que: Abierto::Consola,
            refs: u32::MAX,
        });
    }
}

/// Tabla de descriptores inicial de una tarea: 0, 1 y 2 a la consola.
pub fn fds_iniciales() -> [u32; NR_OPEN] {
    let mut f = [LIBRE; NR_OPEN];
    f[0] = 0;
    f[1] = 1;
    f[2] = 2;
    f
}

/// Devuelve el contenido de una ranura (una copia; no retiene el prestamo).
fn ranura(slot: u32) -> Option<Abierto> {
    if slot == LIBRE {
        return None;
    }
    let t = unsafe { &*TABLA.get() };
    t.get(slot as usize).and_then(|o| o.as_ref()).map(|r| r.que)
}

/// Actualiza la posicion de un fichero abierto.
fn fijar_pos(slot: u32, pos: usize) {
    let t = unsafe { &mut *TABLA.get() };
    if let Some(Some(r)) = t.get_mut(slot as usize) {
        if let Abierto::Archivo { pos: p, .. } = &mut r.que {
            *p = pos;
        }
    }
}

/// Suma una referencia a una ranura (lo usa `fork`).
pub fn retener(slot: u32) {
    let t = unsafe { &mut *TABLA.get() };
    if let Some(Some(r)) = t.get_mut(slot as usize) {
        if r.refs != u32::MAX {
            r.refs += 1;
        }
    }
}

/// Resta una referencia; libera la ranura cuando llega a cero.
///
/// Si la ranura era un extremo de tuberia, al soltarse el ultimo descriptor que
/// la usaba se cierra ese extremo: es lo que convierte las lecturas bloqueadas
/// en fin de tuberia (se cierra el ultimo escritor) o los escritores en `-1`.
fn soltar(slot: u32) {
    // El prestamo de `TABLA` se cierra **antes** de avisar al subsistema
    // (tuberia o kira). Asi la regla de propiedad es explicita: TABLA nunca se
    // mantiene prestada a traves de una llamada a otro global (Fase 4).
    let (libera, cerrada) = {
        let t = unsafe { &mut *TABLA.get() };
        let i = slot as usize;
        let mut cerrada: Option<Abierto> = None;
        let libera = match t.get_mut(i) {
            Some(Some(r)) if r.refs != u32::MAX => {
                r.refs -= 1;
                if r.refs == 0 {
                    cerrada = Some(r.que);
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if libera {
            t[i] = None;
        }
        (libera, cerrada)
    };
    if libera {
        match cerrada {
            Some(Abierto::Tuberia { id, escritura }) => crate::pipe::soltar_extremo(id, escritura),
            Some(Abierto::Kira { id }) => crate::kira::dev::cerrar_conexion(id),
            _ => {}
        }
    }
}

/// Reserva una ranura global para un fichero nuevo.
fn reservar(registro: u32, pos: usize, leer: bool, escribir: bool) -> Option<u32> {
    let t = unsafe { &mut *TABLA.get() };
    for (i, s) in t.iter_mut().enumerate() {
        if s.is_none() {
            *s = Some(Ranura {
                que: Abierto::Archivo {
                    registro,
                    pos,
                    leer,
                    escribir,
                },
                refs: 1,
            });
            return Some(i as u32);
        }
    }
    None
}

/// Reserva una ranura global para una conexión Kira.
pub fn reservar_kira(id: u32) -> Option<u32> {
    let t = unsafe { &mut *TABLA.get() };
    for (i, s) in t.iter_mut().enumerate() {
        if s.is_none() {
            *s = Some(Ranura {
                que: Abierto::Kira { id },
                refs: 1,
            });
            return Some(i as u32);
        }
    }
    None
}

/// Reserva una ranura global para un extremo de tuberia. Suma el extremo a la
/// tuberia: mientras la ranura viva, ese extremo cuenta como abierto.
fn reservar_tuberia(id: u32, escritura: bool) -> Option<u32> {
    let t = unsafe { &mut *TABLA.get() };
    for (i, s) in t.iter_mut().enumerate() {
        if s.is_none() {
            *s = Some(Ranura {
                que: Abierto::Tuberia { id, escritura },
                refs: 1,
            });
            crate::pipe::abrir_extremo(id, escritura);
            return Some(i as u32);
        }
    }
    None
}

/// Resuelve el descriptor `fd` de la tarea en curso a una ranura global.
fn fd_de(fd: u64) -> Option<u32> {
    if fd >= NR_OPEN as u64 {
        return None;
    }
    match crate::tarea::con_fds(|fds| fds[fd as usize]) {
        Some(s) if s != LIBRE => Some(s),
        _ => None,
    }
}

/// `open`: abre (o crea) un fichero y devuelve su descriptor, o `-1`.
pub fn abrir(cwd: u32, ruta: &str, banderas: u64) -> i64 {
    use crate::fs::{self, Tipo};

    // Dispositivo virtual Kira: `/dev/kira` abre una conexión con el servidor de
    // ventanas. No necesita volumen montado.
    if ruta == "/dev/kira" {
        let cid = match crate::kira::dev::nueva_conexion() {
            Some(cid) => cid,
            None => return -1,
        };
        let slot = match reservar_kira(cid) {
            Some(s) => s,
            None => {
                crate::kira::dev::cerrar_conexion(cid);
                return -1;
            }
        };
        let fd = crate::tarea::con_fds(|fds| {
            for (i, d) in fds.iter_mut().enumerate() {
                if *d == LIBRE {
                    *d = slot;
                    return Some(i as i64);
                }
            }
            None
        });
        match fd {
            Some(Some(i)) => i,
            _ => {
                soltar(slot);
                -1
            }
        }
    } else {
        if !fs::montado() {
            return -1;
        }
        let crear = banderas & O_CREAT != 0;
        let reg = match fs::resolver(cwd, ruta) {
            Ok(r) => r,
            Err(_) if crear => match fs::crear(cwd, ruta, Tipo::Fichero) {
                Ok(r) => r,
                Err(_) => return -1,
            },
            Err(_) => return -1,
        };
        let t = fs::tipo(reg);
        let acc = banderas & O_ACCMODE;
        if t == Tipo::Directorio {
            // Un directorio solo se abre para **lectura**: es el descriptor sobre el
            // que gira `getdents64` (el `opendir` de Linux, que es simplemente un
            // `open` con `O_RDONLY`). Por el propio no se leen ni se escriben bytes,
            // y `O_TRUNC` no tiene sentido.
            if acc != O_RDONLY || banderas & O_TRUNC != 0 {
                return -1;
            }
        }
        let leer = acc == O_RDONLY || acc == O_RDWR;
        let escribir = acc == O_WRONLY || acc == O_RDWR;
        if t != Tipo::Directorio && banderas & O_TRUNC != 0 && escribir {
            fs::truncar(reg);
        }
        let pos = if banderas & O_APPEND != 0 {
            fs::tamano(reg) as usize
        } else {
            0
        };

        let slot = match reservar(reg, pos, leer, escribir) {
            Some(s) => s,
            None => return -1, // tabla de ficheros llena
        };
        // Un descriptor libre en la tarea en curso.
        let fd = crate::tarea::con_fds(|fds| {
            for (i, d) in fds.iter_mut().enumerate() {
                if *d == LIBRE {
                    *d = slot;
                    return Some(i as i64);
                }
            }
            None
        });
        match fd {
            Some(Some(i)) => i,
            _ => {
                soltar(slot); // no habia descriptor libre
                -1
            }
        }
    } // fin del else de rutas normales
}

/// `pipe`: crea una tuberia anonima y deposita en la ventana del proceso (en el
/// array de dos `int` que apunta `ptr`) primero el descriptor de **lectura** y
/// despues el de **escritura**. Devuelve `0` o `-1`.
///
/// Es el `sys_pipe` de Linux 0.01: crea el "inodo" de tuberia, monta sus dos
/// `struct file` (lectura y escritura) y los instala en dos descriptores libres
/// del proceso actual.
pub fn crear_pipe(ptr: u64) -> i64 {
    // 1. El destino tiene que ser de la ventana del proceso y escribible (y, si
    //    venia de `fork`, hay que romper antes la copia-al-escribir).
    if !crate::proceso::poblar_rango_escritura(ptr, 8) {
        return -1;
    }
    // 2. El buzon y la tuberia.
    let id = match crate::pipe::crear() {
        Some(id) => id,
        None => return -1,
    };
    // 3. Los dos extremos en la tabla global de ficheros abiertos.
    let s_lee = match reservar_tuberia(id, false) {
        Some(s) => s,
        None => {
            crate::pipe::destruir(id);
            return -1;
        }
    };
    let s_esc = match reservar_tuberia(id, true) {
        Some(s) => s,
        None => {
            soltar(s_lee); // cierra el extremo de lectura (y libera la tuberia)
            return -1;
        }
    };
    // 4. Dos descriptores libres en la tarea en curso.
    let fds = crate::tarea::con_fds(|f| {
        let mut libres = [usize::MAX; 2];
        let mut k = 0;
        for (i, d) in f.iter().enumerate() {
            if *d == LIBRE {
                libres[k] = i;
                k += 1;
                if k == 2 {
                    break;
                }
            }
        }
        if k == 2 {
            f[libres[0]] = s_lee;
            f[libres[1]] = s_esc;
            Some((libres[0], libres[1]))
        } else {
            None
        }
    });
    let (fd_lee, fd_esc) = match fds {
        Some(Some(x)) => x,
        _ => {
            soltar(s_lee);
            soltar(s_esc);
            return -1;
        }
    };
    // 5. Los descriptores, a la ventana del proceso (ya asegurada en el paso 1).
    unsafe {
        core::ptr::write_volatile(ptr as *mut i32, fd_lee as i32);
        core::ptr::write_volatile((ptr + 4) as *mut i32, fd_esc as i32);
    }
    0
}

/// `close`: cierra un descriptor. Devuelve `0` o `-1`.
pub fn cerrar(fd: u64) -> i64 {
    if fd >= NR_OPEN as u64 {
        return -1;
    }
    let slot = crate::tarea::con_fds(|fds| {
        let d = fds[fd as usize];
        if d == LIBRE {
            None
        } else {
            fds[fd as usize] = LIBRE;
            Some(d)
        }
    });
    match slot {
        Some(Some(s)) => {
            soltar(s);
            0
        }
        _ => -1,
    }
}

/// `read`: lee hasta `buf.len()` bytes. Devuelve los bytes leidos o `-1`.
///
/// En la consola (descriptor 0) la lectura la gobierna el terminal ([`crate::tty`],
/// Fase 8): modo canonico, eco y edicion los decide su `termios`. Bloquea
/// **cediendo el turno** mientras no hay nada; el teclado es por sondeo, asi que
/// funciona aunque la puerta de la llamada al sistema tenga las interrupciones
/// tapadas.
pub fn leer(fd: u64, buf: &mut [u8]) -> i64 {
    let slot = match fd_de(fd) {
        Some(s) => s,
        None => return -1,
    };
    match ranura(slot) {
        Some(Abierto::Consola) => crate::tty::leer(buf),
        Some(Abierto::Archivo {
            registro,
            pos,
            leer,
            ..
        }) => {
            if !leer {
                return -1; // abierto solo para escritura
            }
            if crate::fs::tipo(registro) == crate::fs::Tipo::Directorio {
                // Un directorio no se lee a trozos: su contenido se recorre con
                // `getdents64`. Devolver los bloques crudos enmascaría el
                // formato de Estrato y no seria util para nadie.
                return -1;
            }
            let n = crate::fs::leer(registro, pos, buf);
            fijar_pos(slot, pos + n);
            n as i64
        }
        Some(Abierto::Tuberia { id, escritura }) => {
            if escritura {
                return -1; // extremo de escritura: por ahi no se lee
            }
            // Puede bloquear: el nucleo cede el turno hasta que haya datos o se
            // cierren todos los escritores (fin de tuberia, `0`).
            crate::pipe::leer(id, buf)
        }
        Some(Abierto::Kira { id }) => crate::kira::dev::leer_kira(id, buf),
        None => -1,
    }
}

/// `write`: escribe `datos`. Devuelve los bytes escritos o `-1`.
pub fn escribir(fd: u64, datos: &[u8]) -> i64 {
    let slot = match fd_de(fd) {
        Some(s) => s,
        None => return -1,
    };
    match ranura(slot) {
        Some(Abierto::Consola) => {
            crate::tty::escribir(datos);
            datos.len() as i64
        }
        Some(Abierto::Archivo {
            registro,
            pos,
            escribir,
            ..
        }) => {
            if !escribir {
                return -1; // abierto solo para lectura
            }
            match crate::fs::escribir(registro, pos, datos) {
                Ok(n) => {
                    fijar_pos(slot, pos + n);
                    n as i64
                }
                Err(_) => -1,
            }
        }
        Some(Abierto::Tuberia { id, escritura }) => {
            if !escritura {
                return -1; // extremo de lectura: por ahi no se escribe
            }
            // Puede bloquear si el buzon esta lleno; devuelve `-1` si no queda
            // ningun lector.
            crate::pipe::escribir(id, datos)
        }
        Some(Abierto::Kira { id }) => crate::kira::dev::escribir_kira(id, datos),
        None => -1,
    }
}

/// `ioctl`: controla un dispositivo. En rocky el unico que lo atiende es el
/// **terminal** de la consola (los descriptores 0, 1 y 2); un fichero o una
/// tuberia devuelven `-1` (el `ENOTTY`).
///
/// Los mandos son el `termios` ([`crate::tty::TCGETS`]/[`TCSETS`](crate::tty::TCSETS))
/// y el grupo de procesos en primer plano ([`TIOCSPGRP`](crate::tty::TIOCSPGRP)).
pub fn ioctl(fd: u64, request: u64, arg: u64) -> i64 {
    let slot = match fd_de(fd) {
        Some(s) => s,
        None => return -1,
    };
    match ranura(slot) {
        Some(Abierto::Consola) => crate::tty::ioctl(request, arg),
        _ => -1,
    }
}

/// `lseek`: mueve la posicion de un fichero. Devuelve la nueva posicion o `-1`.
pub fn buscar(fd: u64, offset: i64, whence: u64) -> i64 {
    let slot = match fd_de(fd) {
        Some(s) => s,
        None => return -1,
    };
    match ranura(slot) {
        Some(Abierto::Archivo { registro, pos, .. }) => {
            let base = match whence {
                SEEK_CUR => pos as i64,
                SEEK_END => crate::fs::tamano(registro) as i64,
                _ => 0,
            };
            let nuevo = base + offset;
            if nuevo < 0 {
                return -1;
            }
            fijar_pos(slot, nuevo as usize);
            nuevo
        }
        // La consola no tiene posicion.
        _ => -1,
    }
}

/// Inodo (registro) al que apunta un descriptor abierto, para `fstat`.
///
/// Devuelve `None` si `fd` no esta en la tabla de la tarea en curso o si no es
/// un fichero o directorio del volumen (la consola y las tuberias no tienen
/// inodo).
pub fn registro_de(fd: u64) -> Option<u32> {
    let slot = fd_de(fd)?;
    match ranura(slot) {
        Some(Abierto::Archivo { registro, .. }) => Some(registro),
        _ => None,
    }
}

/// `getdents64`: rellena `buf` con entradas `linux_dirent64` del directorio
/// abierto en `fd`, empezando por su posicion actual.
///
/// La posicion del descriptor es el indice de entrada (el "cookie" de Linux):
/// cada llamada continua donde se quedo la anterior, y al llegar al final
/// devuelve `0` (EOF). Devuelve el numero de bytes escritos, o `-1` si `fd` no
/// es un directorio abierto para lectura, o si `buf` no llega ni para una
/// entrada (el `EINVAL` de Linux, para no devolver un EOF ambiguo).
pub fn getdents64(fd: u64, buf: &mut [u8]) -> i64 {
    let slot = match fd_de(fd) {
        Some(s) => s,
        None => return -1,
    };
    let (registro, pos, leer) = match ranura(slot) {
        Some(Abierto::Archivo {
            registro,
            pos,
            leer,
            ..
        }) => (registro, pos, leer),
        _ => return -1, // ni la consola ni las tuberias son directorios
    };
    if !leer || crate::fs::tipo(registro) != crate::fs::Tipo::Directorio {
        return -1;
    }

    // `fs::listar` recorre entero el directorio (y ya salta "." y ".."), asi
    // que el "indice de entrada" es lo unico estado que hay que guardar: no se
    // cachean bloques ni nombres, que es lo que le costaria la vida a un
    // sistema sin memoria suficiente.
    let mut indice: usize = 0;
    let mut escrito: usize = 0;
    let mut lleno = false;
    crate::fs::listar(registro, |e| {
        if lleno {
            return;
        }
        if indice < pos {
            indice += 1;
            return;
        }
        // struct linux_dirent64: ino(8) off(8) reclen(2) type(1) nombre\0.
        let nombre = e.nombre.as_bytes();
        let reclen = (19 + nombre.len() + 7) & !7; // alineado a 8, como Linux
        if escrito + reclen > buf.len() {
            lleno = true;
            return;
        }
        let o = escrito;
        buf[o..o + 8].copy_from_slice(&(e.registro as u64).to_le_bytes());
        buf[o + 8..o + 16].copy_from_slice(&((indice + 1) as u64).to_le_bytes());
        buf[o + 16..o + 18].copy_from_slice(&(reclen as u16).to_le_bytes());
        buf[o + 18] = match e.tipo {
            crate::fs::Tipo::Directorio => DT_DIR,
            crate::fs::Tipo::Fichero => DT_REG,
        };
        buf[o + 19..o + 19 + nombre.len()].copy_from_slice(nombre);
        buf[o + 19 + nombre.len()] = 0;
        escrito += reclen;
        indice += 1;
    });
    // Si no pudo caber ni la primera entrada, `indice` no ha avanzado: el
    // descriptor se queda donde estaba y se devuelve el error.
    fijar_pos(slot, indice);

    if escrito == 0 {
        if lleno {
            return -1;
        }
        return 0; // fin del directorio
    }
    escrito as i64
}

/// `dup`: duplica un descriptor (los dos comparten posicion).
pub fn dup(fd: u64) -> i64 {
    let slot = match fd_de(fd) {
        Some(s) => s,
        None => return -1,
    };
    let nuevo = crate::tarea::con_fds(|fds| {
        for (i, d) in fds.iter_mut().enumerate() {
            if *d == LIBRE {
                *d = slot;
                return Some(i as i64);
            }
        }
        None
    });
    match nuevo {
        Some(Some(i)) => {
            retener(slot);
            i
        }
        _ => -1,
    }
}

/// `dup2`: como `dup`, pero a un descriptor concreto (cerrandolo si hacia falta).
pub fn dup2(old: u64, new: u64) -> i64 {
    if new >= NR_OPEN as u64 {
        return -1;
    }
    let slot = match fd_de(old) {
        Some(s) => s,
        None => return -1,
    };
    if old == new {
        return new as i64;
    }
    let viejo = crate::tarea::con_fds(|fds| {
        let v = fds[new as usize];
        fds[new as usize] = slot;
        v
    });
    if let Some(v) = viejo {
        if v != LIBRE {
            soltar(v);
        }
    }
    retener(slot);
    new as i64
}

/// Cierra todos los descriptores de una tarea (lo llama el planificador al
/// recogerla).
pub fn cerrar_todos(fds: &mut [u32; NR_OPEN]) {
    for d in fds.iter_mut() {
        if *d != LIBRE {
            soltar(*d);
            *d = LIBRE;
        }
    }
}
