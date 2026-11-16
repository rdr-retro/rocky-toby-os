//! Gestor de memoria fisica: reparte la RAM en marcos (frames) de 4 KiB.
//!
//! El mapa de memoria lo entrega el firmware por `INT 15h, EAX=E820`. Sobre el
//! construimos un mapa de bits propio: un bit por marco (0 = libre, 1 = usado).
//! Solo gestionamos la memoria a partir de 3 MiB, porque por debajo se reserva
//! la BIOS, el kernel reubicado y los datos bajos del arranque.
//!
//! Diseño propio de rocky, sin parecido con ningun gestor existente.

use alloc::vec::Vec;

use crate::bios::{self, Regs};
use crate::sync::SyncCell;

/// Tamano de marco.
pub const FRAME_SIZE: usize = 4096;
/// Numero maximo de marcos que soportamos (1 GiB de RAM).
///
/// No se puede subir a voluntad: el mapa de bits que cuelga de aqui vive en
/// `.bss`. Con `MAX_FRAMES = 1 GiB` el mapa ocupa 32 KiB; el linker comprueba
/// que el kernel y todo su `.bss` quepan antes de la pila de arranque.
const MAX_FRAMES: usize = 256 * 1024;
/// Palabras del mapa de bits (1 bit por marco).
const BITMAP_WORDS: usize = MAX_FRAMES / 64;
/// Todo por debajo de 3 MiB se considera ocupado (BIOS, kernel, trampolin y
/// margen para los datos estaticos del kernel reubicado).
const LOW_RESERVED: usize = 0x30_0000;
/// Pila de arranque: es la que usa para siempre la tarea del nucleo (el
/// interprete de ordenes y el escritorio), asi que se reserva entera. Su tope
/// tiene que coincidir con el `mov rsp, ...` de `start.s`; vive en [3, 4) MiB,
/// despues de la imagen del kernel y su `.bss`.
pub const STACK_TOP: usize = 0x40_0000;
/// Tamano reservado para esa pila.
const STACK_BYTES: usize = 0x10_0000;

/// Buffer en memoria baja donde el BIOS escribe cada entrada E820.
const E820_ADDR: usize = 0x7480;
const E820_SEG: u16 = (E820_ADDR >> 4) as u16; // 0x748

const E820_SIGNATURE: u32 = 0x534D_4150; // "SMAP"

struct Pmm {
    /// Un bit por marco: 1 = usado.
    used_map: [u64; BITMAP_WORDS],
    /// Marcos gestionados (hasta el final de la RAM).
    total: usize,
    used: usize,
    /// Pista para búsqueda next-fit (Fase 6).
    hint: usize,
}

static PMM: SyncCell<Pmm> = SyncCell::new(Pmm {
    used_map: [0; BITMAP_WORDS],
    total: 0,
    used: 0,
    hint: 0,
});

/// Cuenta de referencias por marco, para la copia-al-escribir de las paginas de
/// usuario. Va en su propio `static` (y no dentro de `Pmm`) para que el mapa de
/// bits de `Pmm` siga siendo todo ceros y acabe en `.bss`: metido en una
/// estructura con un `Vec`, el mapa de 32 KiB pasaria a `.data` y no cabria en
/// la ventana de carga del arranque. Se reserva en [`init_refs`], despues de
/// `heap::init`.
static REFS: SyncCell<Vec<u8>> = SyncCell::new(Vec::new());

#[inline]
fn rd64(addr: usize) -> u64 {
    unsafe { core::ptr::read_volatile(addr as *const u64) }
}

#[inline]
fn rd32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

impl Pmm {
    #[inline]
    fn is_used(&self, frame: usize) -> bool {
        self.used_map[frame / 64] & (1u64 << (frame % 64)) != 0
    }

    #[inline]
    fn set_used(&mut self, frame: usize, used: bool) {
        let bit = 1u64 << (frame % 64);
        let word = &mut self.used_map[frame / 64];
        let was = *word & bit != 0;
        if used && !was {
            *word |= bit;
            self.used += 1;
        } else if !used && was {
            *word &= !bit;
            self.used -= 1;
        }
    }

    /// Marca como libres los marcos completamente contenidos en `[base, base+len)`.
    fn free_range(&mut self, base: u64, len: u64) {
        if len == 0 {
            return;
        }
        let start = align_up(base as usize, FRAME_SIZE) / FRAME_SIZE;
        let end = ((base + len) as usize) / FRAME_SIZE;
        let end = end.min(self.total);
        for frame in start..end {
            self.set_used(frame, false);
        }
    }
}

/// Recorre el mapa E820 y construye el estado del gestor.
pub fn init() {
    let pmm = unsafe { &mut *PMM.get() };

    // Estado inicial: todo ocupado; iremos liberando lo que el firmware declare
    // como disponible.
    for word in pmm.used_map.iter_mut() {
        *word = u64::MAX;
    }
    pmm.total = 0;
    pmm.used = 0;
    pmm.hint = 0;

    // Primera pasada: tope de la RAM y comprobacion de que el mapa E820 sirve.
    //
    // El bucle va acotado y ademas corta si el firmware repite el mismo `ebx`:
    // hay BIOS que no terminan el recorrido (no dejan EBX a cero ni activan CF)
    // y en un PC real eso deja la maquina colgada dentro de `INT 15h`.
    let mut top: u64 = 0;
    let mut usable_seen = false;
    let mut entradas: u32 = 0;
    let mut ebx: u32 = 0;
    for _ in 0..256 {
        let r = bios::int15(Regs {
            eax: 0xE820,
            ebx,
            ecx: 24,
            edx: E820_SIGNATURE,
            es: E820_SEG,
            edi: 0,
            ..Default::default()
        });
        if r.carry || r.eax != E820_SIGNATURE {
            break;
        }
        entradas += 1;
        let base = rd64(E820_ADDR);
        let len = rd64(E820_ADDR + 8);
        let kind = rd32(E820_ADDR + 16);
        // Solo la RAM disponible define el tope util; las regiones reservadas
        // altas (APIC, firmware...) no cuentan como memoria nuestra.
        if kind == 1 {
            top = top.max(base + len);
            usable_seen = true;
        }
        if r.ebx == 0 || r.ebx == ebx {
            break;
        }
        ebx = r.ebx;
    }

    // Plan B para BIOS sin un E820 utilizable: tamano de la memoria extendida a
    // partir de 1 MiB (`INT 15h, AH=88h`). En ese caso no nos fiamos del E820
    // para liberar rangos y damos por libre todo lo que hay hasta `top`.
    if top < LOW_RESERVED as u64 + (8 << 20) {
        let r = bios::int15(Regs {
            eax: 0x8800,
            ..Default::default()
        });
        if !r.carry {
            let kb = (r.eax & 0xFFFF) as u64;
            if kb > 0 {
                top = top.max(LOW_RESERVED as u64 + kb * 1024);
            }
        }
        usable_seen = false;
    }
    if top == 0 {
        top = 32 << 20; // ultimo recurso: 32 MiB
    }

    crate::println!(
        "rocky: e820: {} entradas, tope de RAM = {} MiB (mapa {}utilizable).",
        entradas,
        (top >> 20) as usize,
        if usable_seen { "" } else { "no " }
    );

    pmm.total = ((top as usize) / FRAME_SIZE).min(MAX_FRAMES);
    // Todos los marcos gestionados empiezan ocupados; liberaremos los rangos
    // disponibles a continuacion.
    pmm.used = pmm.total;

    if usable_seen {
        // Segunda pasada: liberar los rangos disponibles que dio el firmware.
        let mut ebx: u32 = 0;
        for _ in 0..256 {
            let r = bios::int15(Regs {
                eax: 0xE820,
                ebx,
                ecx: 24,
                edx: E820_SIGNATURE,
                es: E820_SEG,
                edi: 0,
                ..Default::default()
            });
            if r.carry || r.eax != E820_SIGNATURE {
                break;
            }
            let base = rd64(E820_ADDR);
            let len = rd64(E820_ADDR + 8);
            let kind = rd32(E820_ADDR + 16);
            if kind == 1 {
                pmm.free_range(base, len);
            }
            if r.ebx == 0 || r.ebx == ebx {
                break;
            }
            ebx = r.ebx;
        }
    } else {
        pmm.free_range(
            LOW_RESERVED as u64,
            (top as u64).saturating_sub(LOW_RESERVED as u64),
        );
    }

    // Reservamos de nuevo la memoria baja (BIOS, kernel, trampolin, VGA...).
    let low_frames = LOW_RESERVED / FRAME_SIZE;
    for frame in 0..low_frames.min(pmm.total) {
        pmm.set_used(frame, true);
    }

    // Y la pila de arranque, que la tarea del nucleo usa toda la vida (por eso
    // no puede acabar repartida como memoria libre).
    let stack_start = (STACK_TOP - STACK_BYTES) / FRAME_SIZE;
    let stack_end = (STACK_TOP / FRAME_SIZE).min(pmm.total);
    for frame in stack_start..stack_end {
        pmm.set_used(frame, true);
    }
}

/// Reserva `count` marcos fisicos contiguos. Devuelve la direccion fisica.
/// Fase 6: búsqueda next-fit con pista para reducir el recorrido desde el inicio.
///
/// Es un caso particular de [`alloc_frames_aligned`]: sin alineacion extra y
/// sin tope fisico.
pub fn alloc_frames(count: usize) -> Option<usize> {
    alloc_frames_aligned(count, FRAME_SIZE, u64::MAX)
}

/// Reserva `count` marcos fisicos contiguos con dos requisitos extra.
///
/// - **Alineacion**: la direccion base tiene que ser multiplo de `align`, que
///   tiene que ser un multiplo de [`FRAME_SIZE`]. Lo pide cualquier estructura
///   de un controlador que el dispositivo tiene que poder leer tal cual (anillos
///   de xHCI, colas de transferencias...).
/// - **Tope fisico**: la ventana completa (base + tamano) tiene que caber por
///   debajo de `limite_fisico`, que es un tope **exclusivo**. Con
///   `1 << 32` la region queda por debajo de 4 GiB, que es lo que cubre el
///   mapa por identidad que prepara el arranque.
///
/// La busqueda es la misma de next-fit con pista: primero desde la pista y,
/// si no hay sitio, desde el principio. Lo unico que cambia es que solo valen
/// las posiciones alineadas.
pub fn alloc_frames_aligned(count: usize, align: usize, limite_fisico: u64) -> Option<usize> {
    let pmm = unsafe { &mut *PMM.get() };
    if count == 0 || count > pmm.total {
        return None;
    }
    if align < FRAME_SIZE || !align.is_power_of_two() {
        return None;
    }
    let bytes = count.checked_mul(FRAME_SIZE)?;

    // Si la base tiene que estar alineada a `align`, el indice de marco tiene
    // que ser multiplo de `align / FRAME_SIZE` (porque `align` es multiplo del
    // tamano de marco, la cuenta coincide).
    let paso = align / FRAME_SIZE;

    let start = match buscar_tirada(pmm, pmm.hint, pmm.total, count, paso) {
        Some(s) => Some(s),
        // Wrap-around: se vuelve a empezar desde el principio.
        None if pmm.hint > 0 => buscar_tirada(pmm, 0, pmm.hint, count, paso),
        None => None,
    }?;

    // La ventana completa tiene que caber por debajo del tope fisico.
    let fin = (start as u64) * FRAME_SIZE as u64 + bytes as u64;
    if fin > limite_fisico {
        return None;
    }

    for f in start..start + count {
        pmm.set_used(f, true);
    }
    pmm.hint = (start + count).min(pmm.total);
    Some(start * FRAME_SIZE)
}

/// Busca una tirada de `count` marcos libres que empiece en un marco con
/// indice multiplo de `paso`.
///
/// Solo mira el rango `[desde, hasta)`. Devuelve el indice del primer marco
/// de la tirada.
fn buscar_tirada(
    pmm: &Pmm,
    desde: usize,
    hasta: usize,
    count: usize,
    paso: usize,
) -> Option<usize> {
    let mut frame = desde;
    while frame < hasta {
        // Solo cuentan las posiciones alineadas.
        let resto = frame % paso;
        if resto != 0 {
            frame += paso - resto;
            continue;
        }
        if pmm.is_used(frame) {
            frame += 1;
            continue;
        }
        // Comprueba si desde aqui hay `count` marcos libres seguidos.
        let mut run = 0usize;
        let mut f = frame;
        while f < hasta && run < count {
            if pmm.is_used(f) {
                break;
            }
            run += 1;
            f += 1;
        }
        if run == count {
            return Some(frame);
        }
        // Aqui no cabe: se sigue buscando mas adelante.
        frame += 1;
    }
    None
}

/// Reserva un unico marco fisico.
pub fn alloc_frame() -> Option<usize> {
    alloc_frames(1)
}

/// Libera `count` marcos contiguos a partir de `addr`.
pub fn free_frames(addr: usize, count: usize) {
    let pmm = unsafe { &mut *PMM.get() };
    let start = addr / FRAME_SIZE;
    for frame in start..(start + count).min(pmm.total) {
        pmm.set_used(frame, false);
    }
    // Ajustar hint hacia atrás para encontrar huecos más rápido (Fase 6)
    if start < pmm.hint {
        pmm.hint = start;
    }
}

/// Libera un unico marco.
pub fn free_frame(addr: usize) {
    free_frames(addr, 1);
}

/// Reserva la tabla de referencias (una entrada por marco). Se llama una sola
/// vez, despues de montar el heap: es el momento en que ya se puede pedir
/// memoria dinamica para la propia tabla.
pub fn init_refs() {
    let total = unsafe { &*PMM.get() }.total;
    unsafe { *REFS.get() = alloc::vec![0u8; total] };
}

/// Suma una referencia a un marco (lo usa la copia-al-escribir al compartir una
/// pagina entre el padre y el hijo de `fork`).
pub fn frame_retain(addr: usize) {
    let refs = unsafe { &mut *REFS.get() };
    let f = addr / FRAME_SIZE;
    if f < refs.len() {
        refs[f] = refs[f].saturating_add(1);
    }
}

/// Referencias vivas de un marco (0 si no esta en la tabla).
pub fn frame_refs(addr: usize) -> u8 {
    let refs = unsafe { &*REFS.get() };
    let f = addr / FRAME_SIZE;
    if f < refs.len() {
        refs[f]
    } else {
        0
    }
}

/// Suelta una referencia a una pagina de usuario y libera el marco solo cuando
/// ya no queda ninguna. Para marcos sin referencias registradas se comporta como
/// [`free_frame`].
///
/// Es la contrapartida de [`frame_retain`]: mientras dos procesos compartan una
/// pagina (copia-al-escribir) el marco no vuelve al gestor.
pub fn frame_release(addr: usize) {
    let pmm = unsafe { &mut *PMM.get() };
    let refs = unsafe { &mut *REFS.get() };
    let f = addr / FRAME_SIZE;
    if f < refs.len() && refs[f] > 0 {
        refs[f] -= 1;
        if refs[f] == 0 {
            pmm.set_used(f, false);
        }
    } else {
        pmm.set_used(f, false);
    }
}

/// Pone a cero `count` bytes a partir de `addr` (memoria fisica/identidad).
pub fn zero(addr: usize, count: usize) {
    unsafe {
        core::ptr::write_bytes(addr as *mut u8, 0, count);
    }
}

/// Bytes de RAM gestionados.
pub fn total_bytes() -> usize {
    let pmm = unsafe { &*PMM.get() };
    pmm.total * FRAME_SIZE
}

/// Bytes de RAM libres.
pub fn free_bytes() -> usize {
    let pmm = unsafe { &*PMM.get() };
    (pmm.total - pmm.used) * FRAME_SIZE
}

/// Marcos libres.
pub fn free_frame_count() -> usize {
    let pmm = unsafe { &*PMM.get() };
    pmm.total - pmm.used
}
