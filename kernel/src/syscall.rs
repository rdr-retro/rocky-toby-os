//! Llamadas al sistema: la puerta `int 0x80` del anillo 3 al anillo 0.
//!
//! Es la frontera propia de Linux 0.01, traducida a Rust. El programa de usuario
//! deja el numero de llamada en `rax` y los argumentos en `rdi`/`rsi`/`rdx`
//! (convencion de x86_64). La puerta `int 0x80` esta declarada en la IDT como
//! accesible desde el anillo 3 (`DPL = 3`), de modo que el programa puede
//! cruzarla; el manejador corre en anillo 0.
//!
//! Las llamadas que pueden **bloquear o terminar** la tarea (`fork`, `waitpid`,
//! `exit`) devuelven un marco que puede no ser el de entrada: el planificador
//! decide desde cual debe continuar el procesador (`interrupts.s` reajusta el
//! `rsp` con el valor de retorno).

use crate::idt::InterruptStack;

/// Lee `rdx` bytes en el puntero `rsi` desde el descriptor `rdi`. Numero de Linux
/// x86_64 (`read`).
pub const SYS_READ: u64 = 0;
/// Escribe `rdx` bytes del puntero `rsi` en el descriptor `rdi`.
pub const SYS_WRITE: u64 = 1;
/// Abre (o crea) el fichero `rdi` con las banderas `rsi`. Numero de Linux x86_64
/// (`open`).
pub const SYS_OPEN: u64 = 2;
/// Cierra el descriptor `rdi`. Numero de Linux x86_64 (`close`).
pub const SYS_CLOSE: u64 = 3;
/// Mueve la posicion del descriptor `rdi` a `rsi` segun `rdx`. Numero de Linux
/// x86_64 (`lseek`).
pub const SYS_LSEEK: u64 = 8;
/// Ajusta el fin del segmento de datos (heap) a `rdi` y devuelve el nuevo fin.
/// Numero de Linux x86_64 (`brk`).
pub const SYS_BRK: u64 = 12;
/// Instala una accion de senal: `(señal, act_ptr, old_ptr)`. Numero de Linux
/// x86_64 (`rt_sigaction`), pero con el `struct sigaction` **simplificado** de
/// rocky (manejador, restaurador y banderas: tres `u64`).
pub const SYS_SIGACTION: u64 = 13;
/// Cambia la mascara de senales bloqueadas: `(como, conjunto_ptr, viejo_ptr)`.
/// Numero de Linux x86_64 (`rt_sigprocmask`).
pub const SYS_SIGPROCMASK: u64 = 14;
/// Vuelve del manejador de una senal: restaura el contexto que guardo el
/// nucleo. Numero de Linux x86_64 (`rt_sigreturn`).
pub const SYS_SIGRETURN: u64 = 15;
/// Controla un dispositivo: en rocky, el terminal de la consola (`termios`,
/// grupo en primer plano, tamano de la ventana). Numero de Linux x86_64
/// (`ioctl`).
pub const SYS_IOCTL: u64 = 16;
/// Crea una tuberia anonima: deposita en el array de dos `int` que apunta `rdi`
/// los descriptores de lectura y de escritura. Numero de Linux x86_64 (`pipe`).
pub const SYS_PIPE: u64 = 22;
/// Duplica el descriptor `rdi`. Numero de Linux x86_64 (`dup`).
pub const SYS_DUP: u64 = 32;
/// Duplica `rdi` sobre `rsi`. Numero de Linux x86_64 (`dup2`).
pub const SYS_DUP2: u64 = 33;
/// Devuelve el PID de la tarea en curso.
pub const SYS_GETPID: u64 = 39;
/// Duplica el proceso: devuelve el PID del hijo (0 en el hijo). Numero de Linux
/// x86_64 (`fork`).
pub const SYS_FORK: u64 = 57;
/// Reemplaza la imagen del proceso actual por un ejecutable del disco:
/// `(ruta_ptr, argv, envp)`. Numero de Linux x86_64 (`execve`).
pub const SYS_EXECVE: u64 = 59;
/// Termina el proceso con el codigo de `rdi` y deja correr a otra tarea. Numero
/// de Linux x86_64 (`exit`).
pub const SYS_EXIT: u64 = 60;
/// Espera a que termine un hijo: `(pid, estado_ptr, opciones)`. Numero de
/// Linux x86_64 (`wait4`).
pub const SYS_WAIT4: u64 = 61;
/// Bloquea el proceso hasta que llegue una senal. Numero de Linux x86_64
/// (`pause`).
pub const SYS_PAUSE: u64 = 34;
/// Programa `SIGALRM` dentro de `rdi` segundos. Numero de Linux x86_64
/// (`alarm`).
pub const SYS_ALARM: u64 = 37;
/// Envia la senal `rsi` al proceso `rdi` (0 = al propio). Numero de Linux
/// x86_64 (`kill`).
pub const SYS_KILL: u64 = 62;
/// Crea una sesion (y un grupo de procesos) nuevos para el proceso en curso
/// (Fase 8). Numero de Linux x86_64 (`setsid`).
pub const SYS_SETSID: u64 = 112;
/// Copia la ruta del **directorio de trabajo** del proceso a un buffer suyo
/// (Fase 9). Numero de Linux x86_64 (`getcwd`).
pub const SYS_GETCWD: u64 = 79;
/// Cambia el **directorio de trabajo** del proceso (Fase 9). Numero de Linux
/// x86_64 (`chdir`).
pub const SYS_CHDIR: u64 = 80;

// --- Fase 11: el filesystem desde el anillo 3. ------------------------------
//
// Linux 0.01 expone `fs/` a los programas con `sys_mkdir`, `sys_rmdir`,
// `sys_unlink`, `sys_link`, `sys_rename` y `sys_stat`; el shell de rocky lo
// hacia todo en anillo 0. Estos son los mismos numeros de Linux x86_64.

/// `stat`: metadatos del fichero `rdi` en el `struct stat` que apunta `rsi`.
pub const SYS_STAT: u64 = 4;
/// `fstat`: lo mismo, pero por descriptor ya abierto.
pub const SYS_FSTAT: u64 = 5;
/// `lstat`: como `stat` sin seguir el ultimo enlace. rocky no tiene enlaces
/// simbolicos, asi que hoy es identico a `stat`.
pub const SYS_LSTAT: u64 = 6;
/// `access`: comprueba que la ruta `rdi` existe (con los modos `rsi`).
pub const SYS_ACCESS: u64 = 21;
/// `rename`: mueve (o renombra) `rdi` a `rsi`.
pub const SYS_RENAME: u64 = 82;
/// `mkdir`: crea el directorio `rdi`. El modo `rsi` se conserva en el inodo,
/// pero rocky es monousuario y todavia no lo audita.
pub const SYS_MKDIR: u64 = 83;
/// `rmdir`: borra el directorio `rdi`, que debe estar vacio.
pub const SYS_RMDIR: u64 = 84;
/// `link`: crea la entrada `rsi` apuntando al **mismo** inodo que `rdi`.
pub const SYS_LINK: u64 = 86;
/// `unlink`: borra el fichero `rdi` (los directorios van con `rmdir`).
pub const SYS_UNLINK: u64 = 87;
/// `sync`: empuja al disco lo que quede pendiente.
pub const SYS_SYNC: u64 = 162;
/// `getdents64`: vuelca en `rsi` las entradas del directorio abierto en `rdi`.
pub const SYS_GETDENTS64: u64 = 217;

/// Atiende una llamada y devuelve el marco desde el que debe continuar la tarea.
///
/// Normalmente es el mismo marco (con `rax` ya reescrito con el resultado); en
/// `exit` es el marco de otra tarea.
pub fn dispatch(stack: *mut InterruptStack) -> *mut InterruptStack {
    crate::metricas::syscall();
    // Se leen los campos con el puntero crudo (el marco lo escribio el
    // procesador) para no tener un `&mut` vivo mientras se llama al
    // planificador, que puede cambiar de tarea.
    let rax = unsafe { (*stack).rax };
    match rax {
        SYS_EXIT => {
            let codigo = unsafe { (*stack).rdi } as i64;
            crate::tarea::salir(codigo);
            crate::tarea::planificar(stack)
        }
        SYS_READ => {
            let (fd, buf, len) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            let r = leer(fd, buf, len);
            unsafe { (*stack).rax = r };
            stack
        }
        SYS_WRITE => {
            let (fd, buf, len) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            let r = escribir(fd, buf, len);
            unsafe { (*stack).rax = r };
            stack
        }
        SYS_OPEN => {
            let (ruta_ptr, banderas) = unsafe { ((*stack).rdi, (*stack).rsi) };
            let r = match leer_cadena(ruta_ptr) {
                Some(ruta) => crate::fd::abrir(crate::tarea::cwd_actual(), &ruta, banderas),
                None => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_CLOSE => {
            let fd = unsafe { (*stack).rdi };
            unsafe { (*stack).rax = crate::fd::cerrar(fd) as u64 };
            stack
        }
        SYS_LSEEK => {
            let (fd, offset, whence) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            unsafe { (*stack).rax = crate::fd::buscar(fd, offset as i64, whence) as u64 };
            stack
        }
        // --- Fase 11: filesystem desde anillo 3 ----------------------------
        SYS_STAT | SYS_LSTAT => {
            let (ruta_ptr, buf) = unsafe { ((*stack).rdi, (*stack).rsi) };
            unsafe { (*stack).rax = stat_ruta(ruta_ptr, buf) as u64 };
            stack
        }
        SYS_FSTAT => {
            let (fd, buf) = unsafe { ((*stack).rdi, (*stack).rsi) };
            unsafe { (*stack).rax = stat_fd(fd, buf) as u64 };
            stack
        }
        SYS_ACCESS => {
            let (ruta_ptr, _modos) = unsafe { ((*stack).rdi, (*stack).rsi) };
            let r: i64 = match leer_cadena(ruta_ptr) {
                Some(r) if crate::fs::montado() => {
                    if crate::fs::resolver(crate::tarea::cwd_actual(), &r).is_ok() {
                        0
                    } else {
                        -1
                    }
                }
                _ => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_GETDENTS64 => {
            let (fd, dirp, tam) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            unsafe { (*stack).rax = getdents64(fd, dirp, tam) as u64 };
            stack
        }
        SYS_RENAME => {
            let (viejo, nuevo) = unsafe { ((*stack).rdi, (*stack).rsi) };
            let r = match dos_cadenas(viejo, nuevo) {
                Some((v, n)) => mapear(crate::fs::mover(crate::tarea::cwd_actual(), &v, &n)),
                None => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_MKDIR => {
            let (ruta_ptr, _modo) = unsafe { ((*stack).rdi, (*stack).rsi) };
            let r = match leer_cadena(ruta_ptr) {
                Some(r) => mapear(crate::fs::crear(
                    crate::tarea::cwd_actual(),
                    &r,
                    crate::fs::Tipo::Directorio,
                )),
                None => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_RMDIR => {
            let ruta_ptr = unsafe { (*stack).rdi };
            let r = match leer_cadena(ruta_ptr) {
                // Un `rmdir` sobre un fichero es un error (`ENOTDIR`), no un
                // borrado accidental.
                Some(r) if es_dir(&r) => mapear(crate::fs::borrar(crate::tarea::cwd_actual(), &r)),
                _ => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_UNLINK => {
            let ruta_ptr = unsafe { (*stack).rdi };
            let r = match leer_cadena(ruta_ptr) {
                // Lo contrario: `unlink` sobre un directorio es `EISDIR` y hay
                // que usar `rmdir`.
                Some(r) if es_fichero(&r) => {
                    mapear(crate::fs::borrar(crate::tarea::cwd_actual(), &r))
                }
                _ => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_LINK => {
            let (origen, destino) = unsafe { ((*stack).rdi, (*stack).rsi) };
            let r = match dos_cadenas(origen, destino) {
                Some((o, d)) => mapear(crate::fs::enlazar(crate::tarea::cwd_actual(), &o, &d)),
                None => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_SYNC => {
            // El volumen se escribe a disco en cada operacion; `sync` es el
            // empujon final (el `sys_sync` de Linux, que volcaba la tabla de
            // buffers). Devuelve siempre 0: no hay errores que reportar.
            if crate::fs::montado() {
                crate::disk::sincronizar();
            }
            unsafe { (*stack).rax = 0 };
            stack
        }
        SYS_BRK => {
            let addr = unsafe { (*stack).rdi };
            unsafe { (*stack).rax = crate::tarea::ajustar_brk(addr) as u64 };
            stack
        }
        SYS_PIPE => {
            let ptr = unsafe { (*stack).rdi };
            unsafe { (*stack).rax = crate::fd::crear_pipe(ptr) as u64 };
            stack
        }
        SYS_DUP => {
            let fd = unsafe { (*stack).rdi };
            unsafe { (*stack).rax = crate::fd::dup(fd) as u64 };
            stack
        }
        SYS_DUP2 => {
            let (old, new) = unsafe { ((*stack).rdi, (*stack).rsi) };
            unsafe { (*stack).rax = crate::fd::dup2(old, new) as u64 };
            stack
        }
        SYS_GETPID => {
            unsafe { (*stack).rax = crate::tarea::actual_id() };
            stack
        }
        SYS_FORK => {
            let hijo = crate::tarea::bifurcar(stack);
            unsafe { (*stack).rax = hijo };
            stack
        }
        SYS_EXECVE => {
            let (ruta_ptr, argv, envp) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            let r = ejecutar(stack, ruta_ptr, argv, envp);
            // Si `execve` fue bien, `ejecutar` ya reescribio el marco para volver
            // al programa nuevo (con `rax` ya a 0). Si fallo, se devuelve el
            // error al programa que llamo.
            if r != 0 {
                unsafe { (*stack).rax = r };
            }
            stack
        }
        SYS_WAIT4 => {
            // `pid` es un pid_t de 32 bits: -1 significa "cualquier hijo".
            let pid = unsafe { (*stack).rdi as i32 as i64 };
            let estado_ptr = unsafe { (*stack).rsi };
            match crate::tarea::esperar(pid) {
                Some((hijo, codigo)) => {
                    // El estado de salida se copia a la memoria del usuario solo
                    // si el puntero cae dentro de su ventana (la pagina se trae
                    // bajo demanda, y se rompe la copia si venia de `fork`).
                    if estado_ptr != 0 && crate::proceso::poblar_rango_escritura(estado_ptr, 4) {
                        unsafe { core::ptr::write_volatile(estado_ptr as *mut i32, codigo as i32) };
                    }
                    unsafe { (*stack).rax = hijo };
                }
                None => unsafe { (*stack).rax = (-1i64) as u64 }, // sin hijos que esperar
            }
            stack
        }
        SYS_SIGACTION => {
            let (signum, act, old) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            unsafe { (*stack).rax = crate::senal::sigaction(signum, act, old) as u64 };
            stack
        }
        SYS_SIGPROCMASK => {
            let (how, set, old) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            unsafe { (*stack).rax = crate::senal::sigprocmask(how, set, old) as u64 };
            stack
        }
        SYS_IOCTL => {
            let (fd, request, arg) = unsafe { ((*stack).rdi, (*stack).rsi, (*stack).rdx) };
            unsafe { (*stack).rax = crate::fd::ioctl(fd, request, arg) as u64 };
            stack
        }
        SYS_SETSID => {
            unsafe { (*stack).rax = crate::tty::setsid() as u64 };
            stack
        }
        SYS_CHDIR => {
            let ruta_ptr = unsafe { (*stack).rdi };
            let r = match leer_cadena(ruta_ptr) {
                Some(ruta) => cambiar_directorio(&ruta),
                None => -1,
            };
            unsafe { (*stack).rax = r as u64 };
            stack
        }
        SYS_GETCWD => {
            let (buf, tam) = unsafe { ((*stack).rdi, (*stack).rsi) };
            unsafe { (*stack).rax = leer_directorio(buf, tam) as u64 };
            stack
        }
        SYS_SIGRETURN => {
            // Vuelve del manejador: se restaura el contexto guardado. Si la
            // pila del usuario no se puede leer no hay a donde volver, y el
            // proceso muere.
            if crate::senal::sigreturn(stack) {
                stack
            } else {
                crate::tarea::salir(128 + crate::senal::SIGSEGV as i64);
                crate::tarea::planificar(stack)
            }
        }
        SYS_KILL => {
            let pid = unsafe { (*stack).rdi as i32 as i64 };
            let senal = unsafe { (*stack).rsi };
            unsafe { (*stack).rax = crate::senal::kill(pid, senal) as u64 };
            stack
        }
        SYS_ALARM => {
            let segundos = unsafe { (*stack).rdi };
            unsafe { (*stack).rax = crate::senal::alarm(segundos) };
            stack
        }
        SYS_PAUSE => {
            // Se duerme hasta que llegue una senal; la entrega la hara
            // `senal::comprobar` al volver a anillo 3.
            crate::tarea::pausar_hasta_senal();
            unsafe { (*stack).rax = 0 };
            stack
        }
        _ => {
            unsafe { (*stack).rax = (-1i64) as u64 }; // llamada desconocida
            stack
        }
    }
}

/// `read`: copia hasta `len` bytes desde el descriptor `fd` a la ventana del
/// proceso.
///
/// El puntero `buf` tiene que caer dentro de la ventana de usuario (el anillo 3
/// no puede pedir que se escriba en memoria del nucleo). El descriptor decide de
/// donde sale: la consola (teclado) o un fichero del volumen.
fn leer(fd: u64, buf: u64, len: u64) -> u64 {
    if len == 0 {
        return 0;
    }
    // Las paginas del proceso pueden no estar todavia en memoria (se traen bajo
    // demanda) y pueden estar compartidas tras `fork`: el nucleo las asegura
    // escribibles antes de depositar en ellas los datos leidos.
    if !crate::proceso::poblar_rango_escritura(buf, len) {
        return (-1i64) as u64; // direccion fuera de la ventana del proceso
    }
    let datos = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, len as usize) };
    crate::fd::leer(fd, datos) as u64
}

/// `write`: copia `len` bytes desde la ventana del proceso al descriptor `fd`.
///
/// Lo mismo que `read`: el puntero tiene que ser de la ventana del proceso. El
/// descriptor decide donde acaba el texto: la consola o un fichero.
fn escribir(fd: u64, buf: u64, len: u64) -> u64 {
    if len == 0 {
        return 0;
    }
    if !crate::proceso::poblar_rango(buf, len) {
        return (-1i64) as u64; // direccion fuera de la ventana del proceso
    }
    let datos = unsafe { core::slice::from_raw_parts(buf as *const u8, len as usize) };
    crate::fd::escribir(fd, datos) as u64
}

/// `execve`: lee el ejecutable `ruta` del sistema de ficheros y reemplaza la
/// imagen del proceso actual (todavia sin `open`: el nucleo lo lee directamente
/// de Estrato, con permiso de anillo 0).
///
/// `argv` y `envp` son punteros a vectores terminados en NULL en la ventana del
/// proceso. Sus cadenas se copian a memoria del nucleo **antes** de liberar la
/// imagen vieja, y de ahi a la pila del programa nuevo.
///
/// Devuelve `0` si fue bien o `-1` (no se usa `errno` todavia).
fn ejecutar(stack: *mut InterruptStack, ruta_ptr: u64, argv_ptr: u64, envp_ptr: u64) -> u64 {
    use alloc::vec;

    // 1. La ruta y los argumentos vienen de la ventana del proceso: se copian
    //    con comprobacion (antes de que se libere su imagen).
    let ruta = match leer_cadena(ruta_ptr) {
        Some(r) => r,
        None => return (-1i64) as u64,
    };
    let argv = match leer_vector(argv_ptr) {
        Some(v) => v,
        None => return (-1i64) as u64,
    };
    let envp = match leer_vector(envp_ptr) {
        Some(v) => v,
        None => return (-1i64) as u64,
    };

    // 2. Se abre el ejecutable **por la capa de descriptores** (como `do_execve`
    //    en Linux, que hace `open` y luego `read`). Asi `execve` estrena la
    //    Fase 4 en lugar de leer el inodo a mano. La ruta relativa se resuelve
    //    desde el directorio de trabajo del proceso (Fase 9). El descriptor se
    //    cierra antes de tocar la imagen: no debe sobrevivir al programa nuevo.
    let dfd = crate::fd::abrir(crate::tarea::cwd_actual(), &ruta, crate::fd::O_RDONLY);
    if dfd < 0 {
        return (-1i64) as u64;
    }
    let dfd = dfd as u64;
    // El tamano sale del propio descriptor (`lseek` al final), sin volver a
    // resolver la ruta.
    let tam = crate::fd::buscar(dfd, 0, crate::fd::SEEK_END);
    if tam <= 0 || tam > crate::proceso::CODIGO_MAX as i64 {
        crate::fd::cerrar(dfd);
        return (-1i64) as u64;
    }
    crate::fd::buscar(dfd, 0, crate::fd::SEEK_SET);

    // 3. Se lee a memoria del nucleo (varias paginas, hasta `CODIGO_MAX`).
    let mut datos = vec![0u8; tam as usize];
    let n = crate::fd::leer(dfd, &mut datos);
    crate::fd::cerrar(dfd);
    if n < 0 {
        return (-1i64) as u64;
    }
    datos.truncate(n as usize);

    // 4. Se reemplaza la imagen de la tarea actual.
    crate::tarea::ejecutar(stack, &datos, &argv, &envp) as u64
}

/// `chdir`: resuelve `ruta` desde el directorio de trabajo del proceso y, si es
/// un directorio, pasa a ser el nuevo directorio de trabajo. Devuelve `0` o `-1`.
fn cambiar_directorio(ruta: &str) -> i64 {
    let cwd = crate::tarea::cwd_actual();
    match crate::fs::resolver(cwd, ruta) {
        Ok(reg) if crate::fs::tipo(reg) == crate::fs::Tipo::Directorio => {
            if crate::tarea::fijar_cwd(reg) {
                0
            } else {
                -1
            }
        }
        _ => -1,
    }
}

// --- Fase 11: filesystem desde anillo 3 ------------------------------------

/// Rocky todavia no tiene `errno`: los errores del filesystem se devuelven todos
/// como `-1`, igual que el resto de llamadas desde la Fase 4.
fn mapear<T>(r: Result<T, crate::fs::Error>) -> i64 {
    if r.is_ok() {
        0
    } else {
        -1
    }
}

/// ¿Existe la ruta y es un directorio? (`rmdir` rechaza los ficheros).
fn es_dir(ruta: &str) -> bool {
    if !crate::fs::montado() {
        return false;
    }
    match crate::fs::resolver(crate::tarea::cwd_actual(), ruta) {
        Ok(reg) => crate::fs::tipo(reg) == crate::fs::Tipo::Directorio,
        Err(_) => false,
    }
}

/// ¿Existe la ruta y es un fichero? (`unlink` rechaza los directorios).
fn es_fichero(ruta: &str) -> bool {
    if !crate::fs::montado() {
        return false;
    }
    match crate::fs::resolver(crate::tarea::cwd_actual(), ruta) {
        Ok(reg) => crate::fs::tipo(reg) == crate::fs::Tipo::Fichero,
        Err(_) => false,
    }
}

/// Copia dos rutas de la ventana del proceso (para `rename` y `link`).
fn dos_cadenas(a: u64, b: u64) -> Option<(alloc::string::String, alloc::string::String)> {
    let x = leer_cadena(a)?;
    let y = leer_cadena(b)?;
    Some((x, y))
}

// Campos del `struct stat` de Linux en x86_64 (desplazamientos en bytes).
// rocky llena hasta `ST_CTIME + 8` y deja el resto (zona de `__unused`) a cero.
const ST_DEV: usize = 0;
const ST_INO: usize = 8;
const ST_NLINK: usize = 16;
const ST_MODE: usize = 24;
const ST_SIZE: usize = 48;
const ST_BLKSIZE: usize = 56;
const ST_BLOCKS: usize = 64;
const ST_ATIME: usize = 72;
const ST_MTIME: usize = 88;
const ST_CTIME: usize = 104;
/// Tamano del `struct stat` de Linux en x86_64.
const STAT_TAM: usize = 144;

/// `stat`: copia a la ventana del proceso el `struct stat` de la ruta.
fn stat_ruta(ruta_ptr: u64, buf: u64) -> i64 {
    if !crate::fs::montado() {
        return -1;
    }
    let ruta = match leer_cadena(ruta_ptr) {
        Some(r) => r,
        None => return -1,
    };
    match crate::fs::resolver(crate::tarea::cwd_actual(), &ruta) {
        Ok(reg) => stat_escribir(reg, buf),
        Err(_) => -1,
    }
}

/// `fstat`: lo mismo, pero saliendo del descriptor ya abierto (que resuelve el
/// inodo sin tener que volver a recorrer la ruta).
fn stat_fd(fd: u64, buf: u64) -> i64 {
    match crate::fd::registro_de(fd) {
        Some(reg) => stat_escribir(reg, buf),
        None => -1,
    }
}

/// Rellena el `struct stat` de Linux en x86_64 para el registro `reg`.
///
/// El buffer se trae bajo demanda y se asegura escribible antes de tocarlo: el
/// anillo 3 puede apuntar a una pagina que todavia no este cargada, o a una
/// copia-al-escribir heredada de `fork`. Los tiempos salen del reloj de la CMOS
/// en el momento de la llamada, porque los inodos de Estrato todavia no guardan
/// `atime`/`ctime`; el `mtime` se conserva en el inodo.
fn stat_escribir(reg: u32, buf: u64) -> i64 {
    if !crate::proceso::poblar_rango_escritura(buf, STAT_TAM as u64) {
        return -1;
    }
    let tam = crate::fs::tamano(reg) as i64;
    let ahora = tiempo_unix();
    let mut b = [0u8; STAT_TAM];
    b[ST_DEV..ST_DEV + 8].copy_from_slice(&0u64.to_le_bytes()); // un solo volumen
    b[ST_INO..ST_INO + 8].copy_from_slice(&(reg as u64).to_le_bytes());
    let nlink = crate::fs::enlaces(reg).max(1) as u64;
    b[ST_NLINK..ST_NLINK + 8].copy_from_slice(&nlink.to_le_bytes());
    b[ST_MODE..ST_MODE + 4].copy_from_slice(&crate::fs::modo(reg).to_le_bytes());
    // rocky es monousuario: no hay `st_uid` ni `st_gid` que reportar.
    b[ST_SIZE..ST_SIZE + 8].copy_from_slice(&tam.to_le_bytes());
    b[ST_BLKSIZE..ST_BLKSIZE + 8].copy_from_slice(&512i64.to_le_bytes());
    b[ST_BLOCKS..ST_BLOCKS + 8].copy_from_slice(&((tam + 511) / 512).to_le_bytes());
    b[ST_ATIME..ST_ATIME + 8].copy_from_slice(&ahora.to_le_bytes());
    b[ST_MTIME..ST_MTIME + 8].copy_from_slice(&ahora.to_le_bytes());
    b[ST_CTIME..ST_CTIME + 8].copy_from_slice(&ahora.to_le_bytes());

    unsafe {
        let p = buf as *mut u8;
        for (i, &x) in b.iter().enumerate() {
            core::ptr::write_volatile(p.add(i), x);
        }
    }
    0
}

/// Segundos transcurridos desde el 1 de enero de 1970, segun el reloj de la
/// CMOS.
///
/// Es el `mktime` de Linux 0.01 (`kernel/mktime.c`) escrito en Rust: cuenta los
/// dias ano a ano y mes a mes, con los bisiestos bien mirados.
fn tiempo_unix() -> i64 {
    const DIA: i64 = 86_400;
    let (hora, min, seg) = crate::cmos::time();
    let (dia, mes, anio) = crate::cmos::date();
    if anio < 1970 {
        return 0;
    }
    fn bisiesto(a: u16) -> bool {
        (a % 4 == 0 && a % 100 != 0) || a % 400 == 0
    }
    let mut res: i64 = 0;
    for a in 1970..anio {
        res += 365 * DIA;
        if bisiesto(a) {
            res += DIA;
        }
    }
    // Dias acumulados antes de cada mes (posicion 0 = enero).
    let acum: [i64; 12] = if bisiesto(anio) {
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335]
    } else {
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
    };
    let m = (mes as usize).clamp(1, 12) - 1;
    res += acum[m] * DIA;
    res += (dia.max(1) as i64 - 1) * DIA;
    res += hora as i64 * 3600 + min as i64 * 60 + seg as i64;
    res
}

/// `getdents64`: vuelca en la ventana del proceso las entradas del directorio
/// que apunta `fd`. Devuelve los bytes escritos, `0` al llegar al final o `-1`.
fn getdents64(fd: u64, dirp: u64, tam: u64) -> i64 {
    if tam == 0 || !crate::proceso::poblar_rango_escritura(dirp, tam) {
        return -1;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(dirp as *mut u8, tam as usize) };
    crate::fd::getdents64(fd, buf)
}

/// `getcwd`: copia la ruta del directorio de trabajo **con NUL** al buffer del
/// proceso. Devuelve la longitud **sin** el NUL, o `-1` si no cabe o el puntero
/// no es de la ventana del proceso.
///
/// rocky todavia no usa `errno`, y devolver la longitud (como `read`) es mas
/// comodo que devolver el puntero, que es lo que hace Linux.
fn leer_directorio(buf: u64, tam: u64) -> i64 {
    if tam == 0 {
        return -1;
    }
    let ruta = crate::fs::ruta_de(crate::tarea::cwd_actual());
    let bytes = ruta.as_bytes();
    let n = bytes.len();
    if (n + 1) as u64 > tam {
        return -1;
    }
    // Las paginas del proceso pueden estar sin traer (bajo demanda) o compartidas
    // tras `fork`: el nucleo las asegura escribibles antes de depositar la ruta.
    if !crate::proceso::poblar_rango_escritura(buf, (n + 1) as u64) {
        return -1;
    }
    unsafe {
        let p = buf as *mut u8;
        for (i, &b) in bytes.iter().enumerate() {
            core::ptr::write_volatile(p.add(i), b);
        }
        core::ptr::write_volatile(p.add(n), 0);
    }
    n as i64
}

/// Copia una cadena terminada en NUL de la ventana del proceso.
///
/// Devuelve `None` si el puntero no cae dentro de la ventana o si no hay NUL en
/// los primeros 256 bytes (el limite de una ruta de rocky).
fn leer_cadena(ptr: u64) -> Option<alloc::string::String> {
    if ptr == 0 || !crate::proceso::puntero_valido(ptr, 1) {
        return None;
    }
    let mut bytes: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    // Las paginas se traen bajo demanda: solo hace falta asegurar cada pagina
    // nueva que se cruza.
    let mut pagina = usize::MAX;
    for i in 0..256u64 {
        let dir = ptr + i;
        if !crate::proceso::puntero_valido(dir, 1) {
            return None;
        }
        let pag = (dir as usize) & !(crate::pmm::FRAME_SIZE - 1);
        if pag != pagina {
            if !crate::proceso::poblar(dir) {
                return None;
            }
            pagina = pag;
        }
        let b = unsafe { core::ptr::read_volatile(dir as *const u8) };
        if b == 0 {
            return Some(alloc::string::String::from_utf8_lossy(&bytes).into_owned());
        }
        bytes.push(b);
    }
    None
}

/// Copia un vector de cadenas de la ventana del proceso (como `argv`/`envp`:
/// punteros terminados en NULL). Un puntero NULL es un vector vacio.
///
/// Devuelve `None` si algun puntero se sale de la ventana o si el vector no
/// termina en NULL dentro de un limite razonable.
fn leer_vector(ptr: u64) -> Option<alloc::vec::Vec<alloc::vec::Vec<u8>>> {
    let mut v: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    if ptr == 0 {
        return Some(v); // NULL: sin argumentos
    }
    for i in 0..64u64 {
        let dir = ptr + i * 8;
        if !crate::proceso::poblar_rango(dir, 8) {
            return None;
        }
        let p = unsafe { core::ptr::read_volatile(dir as *const u64) };
        if p == 0 {
            return Some(v);
        }
        let s = leer_cadena(p)?;
        v.push(s.into_bytes());
    }
    None
}
