//! Rutinas comunes a todos los programas de usuario de rocky.
//!
//! Cada programa es un binario `no_std` independiente que incluye este modulo
//! con `#[path = "comun.rs"] mod comun;`. Aqui viven:
//!
//! * el punto de entrada `_start` (en ensamblador: alinea la pila y llama a
//!   `rocky_main`, que define cada programa);
//! * el `#[panic_handler]` (un fallo en un programa de usuario **no** debe
//!   tumbar el sistema: se pide `exit(1)`);
//! * las envolturas de las llamadas al sistema de rocky (`int 0x80`, con los
//!   mismos numeros que Linux x86_64).
//!
//! El ABI de rocky es el de Linux con `int 0x80`: numero en `rax`, argumentos
//! en `rdi`, `rsi`, `rdx` y resultado en `rax`. La puerta de interrupcion
//! conserva los registros del llamante; solo cambia `rax`.

#![allow(dead_code)]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

/// Numeros de llamada al sistema (los mismos que Linux x86_64).
pub const SYS_READ: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_BRK: u64 = 12;
pub const SYS_RT_SIGACTION: u64 = 13;
pub const SYS_RT_SIGRETURN: u64 = 15;
pub const SYS_IOCTL: u64 = 16;
pub const SYS_PIPE: u64 = 22;
pub const SYS_GETPID: u64 = 39;
pub const SYS_FORK: u64 = 57;
pub const SYS_EXECVE: u64 = 59;
pub const SYS_EXIT: u64 = 60;
pub const SYS_WAIT4: u64 = 61;
pub const SYS_KILL: u64 = 62;
pub const SYS_GETCWD: u64 = 79;
pub const SYS_CHDIR: u64 = 80;
pub const SYS_SETSID: u64 = 112;

// Fase 11: el filesystem desde el anillo 3.
pub const SYS_STAT: u64 = 4;
pub const SYS_FSTAT: u64 = 5;
pub const SYS_LSTAT: u64 = 6;
pub const SYS_ACCESS: u64 = 21;
pub const SYS_RENAME: u64 = 82;
pub const SYS_MKDIR: u64 = 83;
pub const SYS_RMDIR: u64 = 84;
pub const SYS_LINK: u64 = 86;
pub const SYS_UNLINK: u64 = 87;
pub const SYS_SYNC: u64 = 162;
pub const SYS_GETDENTS64: u64 = 217;

/// `O_RDONLY`.
pub const O_RDONLY: u64 = 0;
/// `O_WRONLY` (hay que combinarlo con `O_CREAT` para abrir un fichero nuevo).
pub const O_WRONLY: u64 = 1;
/// `O_RDWR`: lectura y escritura.
pub const O_RDWR: u64 = 2;
/// `O_TRUNC`: vacia el fichero al abrirlo para escritura.
pub const O_TRUNC: u64 = 0o1000;
/// `O_CREAT`: crea el fichero si no existe.
pub const O_CREAT: u64 = 0o100;

/// Bits de tipo de `st_mode` (`sys/stat.h`).
pub const S_IFMT: u32 = 0o170000;
/// `st_mode` de un fichero regular.
pub const S_IFREG: u32 = 0o100000;
/// `st_mode` de un directorio.
pub const S_IFDIR: u32 = 0o040000;

/// `d_type` de `linux_dirent64` para un fichero regular.
pub const DT_REG: u8 = 8;
/// `d_type` de `linux_dirent64` para un directorio.
pub const DT_DIR: u8 = 4;

/// `struct stat` de Linux en x86_64 (144 bytes).
///
/// rocky rellena hasta `st_ctime_ns` y deja la zona de `__unused` a cero; los
/// tiempos salen del reloj de la CMOS en el momento de la llamada.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_nlink: u64,
    pub st_mode: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub _pad0: u32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atime: i64,
    pub st_atime_ns: i64,
    pub st_mtime: i64,
    pub st_mtime_ns: i64,
    pub st_ctime: i64,
    pub st_ctime_ns: i64,
    pub _unused: [i64; 3],
}

impl Stat {
    /// Un `struct stat` todo a cero, listo para usarlo como salida de `stat` o
    /// `fstat`.
    pub fn vacio() -> Stat {
        unsafe { core::mem::MaybeUninit::zeroed().assume_init() }
    }
}

/// Senales.
pub const SIGUSR1: u64 = 10;
pub const SIGTERM: u64 = 15;

/// `ioctl` del terminal (Fase 8).
pub const TCGETS: u64 = 0x5401;
pub const TCSETS: u64 = 0x5402;
pub const TIOCGPGRP: u64 = 0x540F;
pub const TIOCSPGRP: u64 = 0x5410;
pub const ICANON: u32 = 0o000002;
pub const ECHO: u32 = 0o000010;

// Punto de entrada: guarda el `argc` que el nucleo deja en el tope de la pila,
// alinea la pila como manda el ABI de System V y llama a `rocky_main`.
global_asm!(
    ".global _start",
    ".type _start, @function",
    "_start:",
    "  mov r12, [rsp]",
    "  and rsp, -16",
    "  mov rdi, r12",
    "  call rocky_main",
    "  ud2",
);

#[panic_handler]
fn panico(_: &PanicInfo) -> ! {
    salir(1)
}

/// Llama a `int 0x80` con tres argumentos.
#[inline(always)]
pub unsafe fn sys3(n: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    asm!(
        "int 0x80",
        inlateout("rax") n => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        options(nostack),
    );
    ret
}

#[inline(always)]
pub fn escribir(fd: u64, buf: &[u8]) -> i64 {
    unsafe { sys3(SYS_WRITE, fd, buf.as_ptr() as u64, buf.len() as u64) as i64 }
}

#[inline(always)]
pub fn leer(fd: u64, buf: &mut [u8]) -> i64 {
    unsafe { sys3(SYS_READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64) as i64 }
}

#[inline(always)]
pub fn abrir(ruta: *const u8, banderas: u64) -> i64 {
    unsafe { sys3(SYS_OPEN, ruta as u64, banderas, 0) as i64 }
}

#[inline(always)]
pub fn cerrar(fd: u64) -> i64 {
    unsafe { sys3(SYS_CLOSE, fd, 0, 0) as i64 }
}

#[inline(always)]
pub fn fork() -> i64 {
    unsafe { sys3(SYS_FORK, 0, 0, 0) as i64 }
}

#[inline(always)]
pub fn execve(ruta: *const u8, argv: *const u64, envp: *const u64) -> i64 {
    unsafe { sys3(SYS_EXECVE, ruta as u64, argv as u64, envp as u64) as i64 }
}

pub fn salir(codigo: u64) -> ! {
    unsafe {
        sys3(SYS_EXIT, codigo, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

#[inline(always)]
pub fn wait4(pid: i64, estado: *mut u32) -> i64 {
    unsafe { sys3(SYS_WAIT4, pid as u64, estado as u64, 0) as i64 }
}

#[inline(always)]
pub fn pipe(fds: *mut u32) -> i64 {
    unsafe { sys3(SYS_PIPE, fds as u64, 0, 0) as i64 }
}

#[inline(always)]
pub fn brk(dir: u64) -> u64 {
    unsafe { sys3(SYS_BRK, dir, 0, 0) }
}

#[inline(always)]
pub fn getpid() -> i64 {
    unsafe { sys3(SYS_GETPID, 0, 0, 0) as i64 }
}

#[inline(always)]
pub fn kill(pid: i64, sig: u64) -> i64 {
    unsafe { sys3(SYS_KILL, pid as u64, sig, 0) as i64 }
}

#[inline(always)]
pub fn chdir(ruta: *const u8) -> i64 {
    unsafe { sys3(SYS_CHDIR, ruta as u64, 0, 0) as i64 }
}

#[inline(always)]
pub fn getcwd(buf: *mut u8, tam: u64) -> i64 {
    unsafe { sys3(SYS_GETCWD, buf as u64, tam, 0) as i64 }
}

#[inline(always)]
pub fn setsid() -> i64 {
    unsafe { sys3(SYS_SETSID, 0, 0, 0) as i64 }
}

#[inline(always)]
pub fn ioctl(fd: u64, req: u64, arg: u64) -> i64 {
    unsafe { sys3(SYS_IOCTL, fd, req, arg) as i64 }
}

#[inline(always)]
pub fn sigaction(sig: u64, act: *const u64) -> i64 {
    unsafe { sys3(SYS_RT_SIGACTION, sig, act as u64, 0) as i64 }
}

#[inline(always)]
pub fn sigreturn() -> i64 {
    unsafe { sys3(SYS_RT_SIGRETURN, 0, 0, 0) as i64 }
}

// --- Fase 11: el filesystem desde el anillo 3. ------------------------------

#[inline(always)]
pub fn stat(ruta: *const u8, buf: *mut Stat) -> i64 {
    unsafe { sys3(SYS_STAT, ruta as u64, buf as u64, 0) as i64 }
}

#[inline(always)]
pub fn fstat(fd: u64, buf: *mut Stat) -> i64 {
    unsafe { sys3(SYS_FSTAT, fd, buf as u64, 0) as i64 }
}

#[inline(always)]
pub fn access(ruta: *const u8, modos: u64) -> i64 {
    unsafe { sys3(SYS_ACCESS, ruta as u64, modos, 0) as i64 }
}

#[inline(always)]
pub fn mkdir(ruta: *const u8, modo: u64) -> i64 {
    unsafe { sys3(SYS_MKDIR, ruta as u64, modo, 0) as i64 }
}

#[inline(always)]
pub fn rmdir(ruta: *const u8) -> i64 {
    unsafe { sys3(SYS_RMDIR, ruta as u64, 0, 0) as i64 }
}

#[inline(always)]
pub fn unlink(ruta: *const u8) -> i64 {
    unsafe { sys3(SYS_UNLINK, ruta as u64, 0, 0) as i64 }
}

#[inline(always)]
pub fn link(origen: *const u8, destino: *const u8) -> i64 {
    unsafe { sys3(SYS_LINK, origen as u64, destino as u64, 0) as i64 }
}

#[inline(always)]
pub fn rename(viejo: *const u8, nuevo: *const u8) -> i64 {
    unsafe { sys3(SYS_RENAME, viejo as u64, nuevo as u64, 0) as i64 }
}

#[inline(always)]
pub fn sync() -> i64 {
    unsafe { sys3(SYS_SYNC, 0, 0, 0) as i64 }
}

/// `getdents64`: recoge en `buf` las entradas del directorio abierto en `fd`.
/// Devuelve los bytes escritos, `0` al final del directorio o `-1`.
#[inline(always)]
pub fn getdents64(fd: u64, buf: &mut [u8]) -> i64 {
    unsafe {
        sys3(
            SYS_GETDENTS64,
            fd,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        ) as i64
    }
}

/// Lee una entrada `linux_dirent64` del buffer que devolvio [`getdents64`].
///
/// Devuelve `(inodo, tipo, nombre)` y deja `*off` apuntando a la entrada
/// siguiente; `None` cuando el buffer ya esta recorrido.
pub fn entrada64<'a>(buf: &'a [u8], off: &mut usize) -> Option<(u64, u8, &'a [u8])> {
    if *off + 19 > buf.len() {
        return None;
    }
    let d_ino = u64::from_le_bytes(buf[*off..*off + 8].try_into().ok()?);
    let d_reclen = u16::from_le_bytes(buf[*off + 16..*off + 18].try_into().ok()?) as usize;
    let d_type = buf[*off + 18];
    if d_reclen < 19 || *off + d_reclen > buf.len() {
        return None;
    }
    let n0 = *off + 19;
    let mut n1 = n0;
    while n1 < *off + d_reclen && buf[n1] != 0 {
        n1 += 1;
    }
    let nombre = &buf[n0..n1];
    *off += d_reclen;
    Some((d_ino, d_type, nombre))
}

/// ¿Contiene el buffer de [`getdents64`] una entrada con ese nombre?
pub fn contiene(buf: &[u8], nombre: &[u8]) -> bool {
    let mut off = 0usize;
    while let Some((_, _, n)) = entrada64(buf, &mut off) {
        if n == nombre {
            return true;
        }
    }
    false
}
