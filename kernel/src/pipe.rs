//! Tuberias anonimas: el `fs/pipe.c` de Linux 0.01, portado a rocky.
//!
//! Es la **Fase 6** del portado. Una tuberia es un **buzon circular de un
//! marco** (4 KiB) con dos extremos: uno de **lectura** y otro de **escritura**.
//! Los dos extremos viven en la misma tuberia, de modo que lo que se escribe por
//! uno se lee por el otro en orden FIFO. Es el mismo modelo que el `inode` de
//! tuberia de Linux 0.01 (`fs/pipe.c`): un buzon de una pagina, un puntero de
//! lectura (`head`) y un contador de bytes (`size`), y dos condiciones de
//! bloqueo:
//!
//! * un **lector** que llega con el buzon vacio se duerme mientras queden
//!   escritores; si ya no queda ninguno, la lectura termina en **fin de
//!   tuberia** (`0` bytes), como el `return read` de `read_pipe`;
//! * un **escritor** que llega con el buzon lleno se duerme hasta que alguien
//!   lea; si ya no queda ningun lector, la escritura no tiene sentido (`-1`),
//!   el equivalente sin `errno` del `EPIPE` de `write_pipe`.
//!
//! El nucleo no tiene `wait_queue` como Linux: como solo hay una CPU, cada
//! tarea bloqueada **apunta a la tuberia que espera** (`Tarea::esperando_tuberia`)
//! y quien cambia el estado (una lectura, una escritura o el cierre de un
//! extremo) recorre las tareas y despierta a las que esperaban a esa tuberia.
//! Es el `wake_up(&inode->i_wait)` de Linux 0.01, pero mirando el planificador
//! en vez de una cola de espera propia.
//!
//! La correspondencia con Linux 0.01 es directa:
//!
//! | Linux 0.01            | rocky                                   |
//! |-----------------------|-----------------------------------------|
//! | `get_pipe_inode`      | [`crear`] (reserva el marco del buzon)  |
//! | `read_pipe`           | [`leer`] (bloquea y cede el turno)      |
//! | `write_pipe`          | [`escribir`]                            |
//! | `PIPE_SIZE`/`PIPE_HEAD` | `Tuberia::size` / `Tuberia::head`     |
//! | `inode->i_count`      | `Tuberia::lectores`/`escritores`        |
//! | `wake_up`/`sleep_on`  | [`crate::tarea::despertar_tuberia`] / [`crate::tarea::bloquear_en_tuberia`] |
//!
//! Igual que `fs/pipe.c`, una lectura o una escritura **se completan del todo**
//! (o hasta el fin de tuberia): `read(fd, buf, 64)` no vuelve con 3 bytes si el
//! escritor sigue vivo, sino que espera a llenar 64 o a que se cierren todos los
//! escritores. Es la semantica de Linux 0.01, no la de las tuberias modernas.

use crate::pmm;
use crate::sync::SyncCell;

/// Capacidad del buzon: una pagina, como `PIPE_SIZE` en Linux 0.01.
pub const PIPE_BUF: usize = pmm::FRAME_SIZE;
/// Tuberias simultaneas en todo el sistema.
pub const NR_PIPE: usize = 16;

/// Una tuberia: el buzon circular y los dos contadores de extremos abiertos.
///
/// `buffer` es un marco fisico (la memoria del kernel se ve por identidad); el
/// buzon guarda `size` bytes validos a partir de `head`, de forma circular.
#[derive(Clone, Copy)]
struct Tuberia {
    /// Direccion del marco que hace de buzon (memoria fisica/identidad).
    buffer: usize,
    /// Posicion del proximo byte a leer (`PIPE_HEAD`).
    head: usize,
    /// Bytes validos en el buzon (`PIPE_SIZE`).
    size: usize,
    /// Extremos de lectura abiertos (`inode->i_count` del lado lector).
    lectores: u32,
    /// Extremos de escritura abiertos.
    escritores: u32,
}

/// La tabla de tuberias. El hueco `i` esta libre si es `None`; su identificador
/// publico es `i + 1` (el `0` queda libre para "ninguna").
static TUBERIAS: SyncCell<[Option<Tuberia>; NR_PIPE]> = SyncCell::new([None; NR_PIPE]);

/// Acceso crudo a la tabla (el kernel es monotarea: basta con que ningun acceso
/// se cuele a mitad de otro).
fn tabla() -> &'static mut [Option<Tuberia>; NR_PIPE] {
    unsafe { &mut *TUBERIAS.get() }
}

/// La tuberia `id`, si sigue viva.
fn mostrar_mut(id: u32) -> Option<&'static mut Tuberia> {
    let i = (id as usize).checked_sub(1)?;
    tabla().get_mut(i).and_then(|s| s.as_mut())
}

/// Crea una tuberia vacia y devuelve su identificador (o `None` si no hay ni
/// marco ni hueco). Los contadores empiezan en cero: los sube [`abrir_extremo`]
/// a medida que la tabla de descriptores instala los dos extremos.
pub fn crear() -> Option<u32> {
    let buffer = pmm::alloc_frame()?;
    pmm::zero(buffer, PIPE_BUF);
    let t = tabla();
    for (i, s) in t.iter_mut().enumerate() {
        if s.is_none() {
            *s = Some(Tuberia {
                buffer,
                head: 0,
                size: 0,
                lectores: 0,
                escritores: 0,
            });
            return Some(i as u32 + 1);
        }
    }
    pmm::free_frame(buffer);
    None
}

/// Suma un extremo abierto (lo llama la capa de descriptores al instalar un
/// extremo, y solo entonces: `dup`/`fork` comparten la misma descripcion de
/// fichero y no cambian este contador).
pub fn abrir_extremo(id: u32, escritura: bool) {
    if let Some(t) = mostrar_mut(id) {
        if escritura {
            t.escritores += 1;
        } else {
            t.lectores += 1;
        }
    }
}

/// Resta un extremo. Si ya no queda ninguno, libera el buzon y la tuberia.
///
/// Despierta siempre a quien espere en la tuberia: cerrar el ultimo extremo de
/// escritura es lo que convierte una lectura bloqueada en **fin de tuberia**, y
/// cerrar el ultimo de lectura lo que hace fallar a un escritor.
pub fn soltar_extremo(id: u32, escritura: bool) {
    let mut libre = false;
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            if let Some(t) = mostrar_mut(id) {
                if escritura {
                    t.escritores = t.escritores.saturating_sub(1);
                } else {
                    t.lectores = t.lectores.saturating_sub(1);
                }
                libre = t.lectores == 0 && t.escritores == 0;
            }
        });
    }
    crate::tarea::despertar_tuberia(id);
    if libre {
        if let Some(i) = (id as usize).checked_sub(1) {
            if let Some(slot) = tabla().get_mut(i) {
                if let Some(t) = slot.take() {
                    pmm::free_frame(t.buffer);
                }
            }
        }
    }
}

/// Destruye una tuberia recien creada a la que todavia no se ha enganchado
/// ningun extremo: es la ruta de error de `pipe` cuando ya no queda sitio en la
/// tabla de ficheros abiertos. Libera el buzon.
pub fn destruir(id: u32) {
    if let Some(i) = (id as usize).checked_sub(1) {
        if let Some(slot) = tabla().get_mut(i) {
            if let Some(t) = slot.take() {
                pmm::free_frame(t.buffer);
            }
        }
    }
}

/// Resultado de un intento de transferencia que **no** puede bloquear.
enum Intento {
    /// Se transfirieron `n` bytes. En lectura, `0` significa fin de tuberia.
    Hecho(i64),
    /// No hay nada que hacer todavia: hay que dormir y reintentar.
    Esperar,
    /// El otro extremo cerro (escritura sin lectores).
    Roto,
}

/// Copia `n` bytes del buzon a `destino`, dando la vuelta al final del buzon.
fn copiar_desde(t: &Tuberia, n: usize, destino: &mut [u8]) {
    let primera = core::cmp::min(n, PIPE_BUF - t.head);
    unsafe {
        core::ptr::copy_nonoverlapping(
            (t.buffer + t.head) as *const u8,
            destino.as_mut_ptr(),
            primera,
        );
        if n > primera {
            core::ptr::copy_nonoverlapping(
                t.buffer as *const u8,
                destino.as_mut_ptr().add(primera),
                n - primera,
            );
        }
    }
}

/// Copia `n` bytes de `origen` al final del buzon, dando la vuelta si hace falta.
fn copiar_hacia(t: &Tuberia, n: usize, origen: &[u8]) {
    let cola = (t.head + t.size) % PIPE_BUF;
    let primera = core::cmp::min(n, PIPE_BUF - cola);
    unsafe {
        core::ptr::copy_nonoverlapping(origen.as_ptr(), (t.buffer + cola) as *mut u8, primera);
        if n > primera {
            core::ptr::copy_nonoverlapping(
                origen.as_ptr().add(primera),
                t.buffer as *mut u8,
                n - primera,
            );
        }
    }
}

/// Intenta leer del buzon sin dormir (`read_pipe` sin el `sleep_on`).
fn intentar_leer(t: &mut Tuberia, buf: &mut [u8]) -> Intento {
    if t.size == 0 {
        // Vacio: fin de tuberia solo si ya no queda ningun escritor.
        return if t.escritores == 0 {
            Intento::Hecho(0)
        } else {
            Intento::Esperar
        };
    }
    let n = core::cmp::min(buf.len(), t.size);
    copiar_desde(t, n, buf);
    t.head = (t.head + n) % PIPE_BUF;
    t.size -= n;
    Intento::Hecho(n as i64)
}

/// Intenta escribir en el buzon sin dormir (`write_pipe` sin el `sleep_on`).
fn intentar_escribir(t: &mut Tuberia, datos: &[u8]) -> Intento {
    if t.lectores == 0 {
        return Intento::Roto; // nadie va a leerlo nunca
    }
    let hueco = PIPE_BUF - t.size;
    if hueco == 0 {
        return Intento::Esperar; // buzon lleno
    }
    let n = core::cmp::min(datos.len(), hueco);
    copiar_hacia(t, n, datos);
    t.size += n;
    Intento::Hecho(n as i64)
}

/// `read` sobre el extremo de lectura de la tuberia `id` (Linux: `read_pipe`).
///
/// Completa `buf` (o termina en fin de tuberia) durmiendo el turno cuando hace
/// falta. Devuelve los bytes leidos; `0` es fin de tuberia.
pub fn leer(id: u32, buf: &mut [u8]) -> i64 {
    if buf.is_empty() {
        return 0;
    }
    let mut leidos = 0usize;
    while leidos < buf.len() {
        // El intento y el dormir van en la **misma seccion critica**: si no, un
        // escritor podria depositar datos entre el intento fallido y el `sleep`
        // y el lector se dormiria para siempre (testigo perdido).
        let paso = unsafe {
            crate::cpu::sin_interrupciones(|| {
                let r = match mostrar_mut(id) {
                    Some(t) => intentar_leer(t, &mut buf[leidos..]),
                    None => Intento::Hecho(0), // la tuberia ya no existe: fin
                };
                if let Intento::Esperar = r {
                    crate::tarea::bloquear_en_tuberia(id);
                }
                r
            })
        };
        match paso {
            Intento::Hecho(0) => break, // fin de tuberia
            Intento::Hecho(n) => {
                leidos += n as usize;
                // Hay hueco nuevo: despertar a los escritores que esperaban.
                crate::tarea::despertar_tuberia(id);
            }
            Intento::Esperar => unsafe { crate::tarea::ceder() },
            Intento::Roto => break,
        }
    }
    leidos as i64
}

/// `write` sobre el extremo de escritura de la tuberia `id` (Linux: `write_pipe`).
///
/// Completa `datos` durmiendo cuando el buzon se llena. Devuelve los bytes
/// escritos; `-1` si no queda ningun lector (el `EPIPE` de Linux, sin `errno`).
pub fn escribir(id: u32, datos: &[u8]) -> i64 {
    if datos.is_empty() {
        return 0;
    }
    let mut escritos = 0usize;
    while escritos < datos.len() {
        let paso = unsafe {
            crate::cpu::sin_interrupciones(|| {
                let r = match mostrar_mut(id) {
                    Some(t) => intentar_escribir(t, &datos[escritos..]),
                    None => Intento::Roto,
                };
                if let Intento::Esperar = r {
                    crate::tarea::bloquear_en_tuberia(id);
                }
                r
            })
        };
        match paso {
            Intento::Hecho(n) => {
                escritos += n as usize;
                // Hay datos nuevos: despertar a los lectores que esperaban.
                crate::tarea::despertar_tuberia(id);
            }
            Intento::Esperar => unsafe { crate::tarea::ceder() },
            Intento::Roto => {
                // Sin lectores: se devuelve lo ya escrito o `-1`. Linux 0.01
                // devolvia lo escrito; aqui se prefiere distinguir el fallo.
                return if escritos == 0 { -1 } else { escritos as i64 };
            }
        }
    }
    escritos as i64
}

/// Cuantas tuberias hay vivas ahora mismo (para `free`, diagnostico...).
pub fn cuantas() -> usize {
    tabla().iter().filter(|s| s.is_some()).count()
}
