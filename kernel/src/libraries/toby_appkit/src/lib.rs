//! SDK Rust mínimo para aplicaciones nativas de Toby AppKit en Rocky.
//!
//! `toby-appkit` ofrece la ABI `int 0x80` del sistema, salida de texto y la
//! macro [`app_entry!`] que genera el punto de entrada `_start` de un ELF
//! `no_std`. Las interfaces visuales siguen descritas en archivos `.toby` del
//! bundle; este SDK es la capa de código nativo de la aplicación.

#![no_std]

/// Relojes, curvas temporales y configuracion reutilizable de transiciones.
pub mod animation;
/// Matemática reutilizable del efecto Genie para AppKit y el compositor Kira.
pub mod genie;

use core::arch::asm;
use core::fmt::{self, Write};

/// Número de llamada `write` de la ABI Rocky (compatible con Linux x86_64).
pub const SYS_WRITE: u64 = 1;
/// Número de llamada `exit` de la ABI Rocky (compatible con Linux x86_64).
pub const SYS_EXIT: u64 = 60;

/// Operaciones básicas sobre el sistema de ficheros del usuario.
pub mod fs {
    use core::ffi::CStr;

    /// Abre un fichero en modo lectura.
    pub const O_RDONLY: u64 = 0;
    /// Abre un fichero en modo escritura.
    pub const O_WRONLY: u64 = 1;
    /// Abre un fichero para lectura y escritura.
    pub const O_RDWR: u64 = 2;
    /// Crea el fichero si todavía no existe.
    pub const O_CREAT: u64 = 0o100;
    /// Trunca el fichero al abrirlo para escritura.
    pub const O_TRUNC: u64 = 0o1000;

    /// Abre una ruta UTF-8 terminada en NUL; devuelve descriptor o error negativo.
    pub fn open(ruta: &CStr, flags: u64) -> isize {
        unsafe { super::llamada3(2, ruta.as_ptr() as u64, flags, 0) as isize }
    }

    /// Lee bytes desde un descriptor abierto.
    pub fn read(fd: u64, buffer: &mut [u8]) -> isize {
        unsafe { super::llamada3(0, fd, buffer.as_mut_ptr() as u64, buffer.len() as u64) as isize }
    }

    /// Cierra un descriptor.
    pub fn close(fd: u64) -> isize {
        unsafe { super::llamada3(3, fd, 0, 0) as isize }
    }

    /// Crea un directorio.
    pub fn mkdir(ruta: &CStr, modo: u64) -> isize {
        unsafe { super::llamada3(83, ruta.as_ptr() as u64, modo, 0) as isize }
    }

    /// Elimina un fichero.
    pub fn unlink(ruta: &CStr) -> isize {
        unsafe { super::llamada3(87, ruta.as_ptr() as u64, 0, 0) as isize }
    }

    /// Renombra un fichero o directorio.
    pub fn rename(origen: &CStr, destino: &CStr) -> isize {
        unsafe { super::llamada3(82, origen.as_ptr() as u64, destino.as_ptr() as u64, 0) as isize }
    }
}

/// Escribe bytes en un descriptor; devuelve los bytes escritos o un error negativo.
pub fn write(fd: u64, bytes: &[u8]) -> isize {
    let mut escritos = 0usize;
    while escritos < bytes.len() {
        let resultado = unsafe {
            llamada3(
                SYS_WRITE,
                fd,
                bytes[escritos..].as_ptr() as u64,
                (bytes.len() - escritos) as u64,
            ) as isize
        };
        if resultado <= 0 {
            return if escritos == 0 {
                resultado
            } else {
                escritos as isize
            };
        }
        escritos += resultado as usize;
    }
    escritos as isize
}

/// Escribe argumentos formateados en la salida estándar.
pub fn print_args(args: fmt::Arguments<'_>) {
    let _ = fmt::write(&mut Salida, args);
}

/// Escribe argumentos formateados y un salto de línea en la salida estándar.
pub fn println_args(args: fmt::Arguments<'_>) {
    print_args(args);
    let _ = write(1, b"\n");
}

/// Finaliza el proceso con el código indicado.
pub fn exit(codigo: i32) -> ! {
    unsafe {
        llamada3(SYS_EXIT, codigo as u64, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

struct Salida;

impl Write for Salida {
    fn write_str(&mut self, texto: &str) -> fmt::Result {
        if write(1, texto.as_bytes()) == texto.len() as isize {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

#[inline(always)]
unsafe fn llamada3(numero: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let resultado: u64;
    asm!(
        "int 0x80",
        inlateout("rax") numero => resultado,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        options(nostack),
    );
    resultado
}

/// Imprime texto formateado sin añadir una nueva línea.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::print_args(core::format_args!($($arg)*));
    }};
}

/// Imprime texto formateado seguido de una nueva línea.
#[macro_export]
macro_rules! println {
    () => {{ $crate::println_args(core::format_args!("")); }};
    ($($arg:tt)*) => {{
        $crate::println_args(core::format_args!($($arg)*));
    }};
}

/// Genera el punto de entrada ELF y el manejador de pánico de una app `no_std`.
///
/// El bloque principal pasado a la macro debe ser una función sin argumentos
/// y sin valor de retorno, por ejemplo `app_entry!(mi_app);`.
#[macro_export]
macro_rules! app_entry {
    ($entrada:path) => {
        core::arch::global_asm!(
            ".global _start",
            ".type _start, @function",
            "_start:",
            "  mov r12, [rsp]",
            "  and rsp, -16",
            "  mov rdi, r12",
            "  call rocky_main",
            "  ud2",
        );

        #[no_mangle]
        pub extern "C" fn rocky_main(_argc: usize) -> ! {
            $entrada();
            $crate::exit(0);
        }

        #[panic_handler]
        fn toby_appkit_panico(_: &core::panic::PanicInfo<'_>) -> ! {
            $crate::exit(1)
        }
    };
}
