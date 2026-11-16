//! Paginacion de 4 niveles (PML4) para rocky.
//!
//! El sector de arranque deja los primeros 4 GiB mapeados por identidad con
//! paginas de 2 MiB (para el kernel y el framebuffer de video). Aqui anadimos
//! una **ventana dinamica** en la mitad alta de la direccion virtual
//! (`0xFFFF_8000_0000_0000`), con paginas de 4 KiB, que es donde vive el heap
//! del kernel. La ventana crece bajo demanda asignando marcos al gestor fisico.
//!
//! Diseño propio de rocky.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::pmm;

/// Base de la ventana dinamica del kernel (PML4 indice 256).
pub const HEAP_BASE: usize = 0xFFFF_8000_0000_0000;
/// Direccion fisica de la PML4 que creo el bootloader.
const PML4_PHYS: usize = 0x1000;
/// Indice de PML4 de la ventana.
const HEAP_PML4_INDEX: usize = (HEAP_BASE >> 39) & 0x1FF;

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
/// Bit de usuario: la pagina (y todos los niveles por encima) es accesible desde
/// el anillo 3.
pub const USER: u64 = 1 << 2;
/// Bit **software** de copia-al-escribir (uno de los bits "disponibles" de la
/// entrada, que la CPU no interpreta). Marca una pagina compartida por dos
/// procesos tras `fork`: es de solo lectura y el primer fallo de escritura la
/// copia.
pub const COW: u64 = 1 << 9;
/// Bit de **cache deshabilitada** (PCD): la pagina no pasa por la cache. Es lo
/// que corresponde a la memoria de un dispositivo (por ejemplo los registros de
/// la GPU): sin esto el procesador podria servir una lectura de la cache en vez
/// de ir al registro, o agrupar escrituras.
pub const PCD: u64 = 1 << 4;
/// Bit de direccion fisica dentro de una entrada de pagina.
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;
/// Bit PAT de una entrada PDE que mapea una pagina grande de 2 MiB.
const HUGE_PAT: u64 = 1 << 12;
const HUGE_PAGE_SIZE: usize = 2 * 1024 * 1024;
const PAT_MSR: u32 = 0x277;
static PAT_WC_CONFIGURADA: AtomicBool = AtomicBool::new(false);

/// La PML4 que dejo montada el sector de arranque (memoria baja, identidad).
pub const PML4_NUCLEO: usize = PML4_PHYS;

#[inline]
fn entry_ptr(table: usize, index: usize) -> *mut u64 {
    (table + index * 8) as *mut u64
}

#[inline]
fn read_entry(table: usize, index: usize) -> u64 {
    unsafe { core::ptr::read_volatile(entry_ptr(table, index)) }
}

#[inline]
fn write_entry(table: usize, index: usize, value: u64) {
    unsafe { core::ptr::write_volatile(entry_ptr(table, index), value) }
}

fn is_present(entry: u64) -> bool {
    entry & PRESENT != 0
}

fn next_table(table: usize, index: usize) -> Option<usize> {
    let entry = read_entry(table, index);
    if is_present(entry) {
        Some((entry & ADDR_MASK) as usize)
    } else {
        None
    }
}

/// Devuelve la tabla siguiente, creandola (con un marco nuevo) si no existe.
///
/// `flags` se anade solo al **crear** la tabla: si ya existia se respeta tal
/// cual. Asi, mapear una pagina de usuario nunca convierte en accesible para el
/// anillo 3 una tabla que ya era del nucleo (por ejemplo la de la identidad).
fn next_table_create_flags(table: usize, index: usize, flags: u64) -> usize {
    if let Some(next) = next_table(table, index) {
        // Si apuntase a una pagina grande (PS=1) no podemos descender.
        let entry = read_entry(table, index);
        if entry & (1 << 7) != 0 {
            panic!("paging: entrada apunta a una pagina grande");
        }
        return next;
    }
    let frame = pmm::alloc_frame().expect("paging: sin marcos para tablas");
    pmm::zero(frame, pmm::FRAME_SIZE);
    write_entry(table, index, frame as u64 | PRESENT | WRITABLE | flags);
    frame
}

fn next_table_create(table: usize, index: usize) -> usize {
    next_table_create_flags(table, index, 0)
}

/// Prepara la ventana del heap en la mitad alta.
pub fn init() {
    // Creamos la PDPT de la ventana si aun no existe.
    let _ = next_table_create(PML4_PHYS, HEAP_PML4_INDEX);
}

/// Mapea la pagina que contiene `virt` al marco fisico `phys`.
pub fn map(virt: usize, phys: usize) -> Result<(), ()> {
    map_in(PML4_NUCLEO, virt, phys, PRESENT | WRITABLE)
}

/// Mapea `virt` -> `phys` en la PML4 `pml4` con los bits `flags`.
///
/// Es la version general de [`map`]: sirve para mapear en la tabla de otro
/// espacio de direcciones (la de un proceso) y para anadir el bit de usuario.
pub fn map_in(pml4: usize, virt: usize, phys: usize, flags: u64) -> Result<(), ()> {
    let i4 = (virt >> 39) & 0x1FF;
    let i3 = (virt >> 30) & 0x1FF;
    let i2 = (virt >> 21) & 0x1FF;
    let i1 = (virt >> 12) & 0x1FF;

    // Los niveles intermedios nuevos heredan los bits de `flags` (en particular
    // el de usuario); los que ya existian no se tocan.
    let pdpt = next_table_create_flags(pml4, i4, flags & USER);
    let pd = next_table_create_flags(pdpt, i3, flags & USER);
    let pt = next_table_create_flags(pd, i2, flags & USER);

    write_entry(pt, i1, phys as u64 | flags);
    flush(virt);
    Ok(())
}

/// Carga la PML4 `pml4` en CR3 (cambia el espacio de direcciones activo).
/// Evita recargar CR3 si ya es el espacio activo: en el planificador muchas
/// tareas del nucleo comparten PML4, y una escritura redundante invalidaria la
/// TLB sin aportar ningun cambio.
pub fn activar(pml4: usize) {
    unsafe {
        let actual: u64;
        asm!("mov {}, cr3", out(reg) actual, options(nostack, preserves_flags));
        if actual as usize != pml4 {
            asm!("mov cr3, {}", in(reg) pml4 as u64, options(nostack, preserves_flags));
        }
    }
}

/// Quita el mapeo de la pagina que contiene `virt`.
#[allow(dead_code)] // API completa de paginacion; aun no hay quien la llame.
pub fn unmap(virt: usize) {
    let i4 = (virt >> 39) & 0x1FF;
    let i3 = (virt >> 30) & 0x1FF;
    let i2 = (virt >> 21) & 0x1FF;
    let i1 = (virt >> 12) & 0x1FF;

    if let Some(pdpt) = next_table(PML4_PHYS, i4) {
        if let Some(pd) = next_table(pdpt, i3) {
            if let Some(pt) = next_table(pd, i2) {
                write_entry(pt, i1, 0);
                flush(virt);
            }
        }
    }
}

/// Marca como **no cacheable** (bit PCD) la pagina de 2 MiB que contiene `virt`.
///
/// Es lo que se debe hacer con la memoria de un dispositivo (MMIO): sin esto el
/// procesador podria servir una lectura de la cache en vez de ir al registro, o
/// agrupar escrituras. Solo actua si esa direccion ya es una **pagina grande**
/// presente (como las del mapa de identidad que dejo el bootloader); si no,
/// devuelve `false` sin tocar nada.
pub fn sin_cache(virt: usize) -> bool {
    let i4 = (virt >> 39) & 0x1FF;
    let i3 = (virt >> 30) & 0x1FF;
    let i2 = (virt >> 21) & 0x1FF;

    let pdpt = match next_table(PML4_NUCLEO, i4) {
        Some(t) => t,
        None => return false,
    };
    let pd = match next_table(pdpt, i3) {
        Some(t) => t,
        None => return false,
    };

    let entry = read_entry(pd, i2);
    // Solo tiene sentido sobre una pagina grande (PS=1) presente: si no fuera
    // asi, la direccion no viene del mapa de identidad y no la tocamos.
    if !is_present(entry) || entry & (1 << 7) == 0 {
        return false;
    }
    if entry & PCD == 0 {
        write_entry(pd, i2, entry | PCD);
        flush(virt);
    }
    true
}

/// Configura PAT[4] como write-combining y aplica ese tipo a las paginas
/// grandes que cubren el framebuffer lineal. WC permite que la CPU agrupe las
/// escrituras secuenciales de pixels antes de enviarlas al dispositivo.
///
/// Solo modifica el mapeo identidad de 2 MiB que prepara el bootloader y
/// rechaza direcciones fuera de los primeros 4 GiB o paginas ya marcadas PCD.
/// En cualquier caso no compatible devuelve `false`, conservando el modo de
/// cache previo.
pub fn framebuffer_write_combining(base: usize, bytes: usize) -> bool {
    const LIMITE_IDENTIDAD: u64 = 1u64 << 32;
    const PCD: u64 = 1 << 4;
    const PAGE_SIZE: u64 = 1 << 7;
    const PAT_ENTRY4: u64 = 1u64 << 32;

    let Some(end) = (base as u64).checked_add(bytes as u64) else {
        return false;
    };
    if bytes == 0 || end > LIMITE_IDENTIDAD {
        return false;
    }

    let first = base & !(HUGE_PAGE_SIZE - 1);
    let last = ((end - 1) as usize) & !(HUGE_PAGE_SIZE - 1);

    // Valida toda la ventana antes de tocar PAT o las tablas: si un BIOS dio
    // una direccion inesperada, no dejamos el framebuffer a medio cambiar.
    let mut page = first;
    loop {
        let Some(entry_ptr) = huge_identity_entry(page) else {
            return false;
        };
        let entry = unsafe { core::ptr::read_volatile(entry_ptr) };
        if entry & PRESENT == 0
            || entry & PAGE_SIZE == 0
            || entry & PCD != 0
            || entry & 0x000F_FFFF_FFE0_0000 != page as u64
        {
            return false;
        }
        if page == last {
            break;
        }
        page += HUGE_PAGE_SIZE;
    }

    if !PAT_WC_CONFIGURADA.load(Ordering::Acquire) {
        // PAT es una capacidad de CPU, no una suposicion del modo VESA.
        if core::arch::x86_64::__cpuid(1).edx & (1 << 16) == 0 {
            return false;
        }

        unsafe {
            let actual = leer_msr(PAT_MSR);
            let nuevo = (actual & !(0xFFu64 << 32)) | PAT_ENTRY4;
            if nuevo != actual {
                // PAT[4] no se usa en los mapeos actuales. Vaciar las caches
                // antes y despues mantiene correcta la transicion en hardware
                // que haya cacheado previamente el framebuffer.
                asm!("wbinvd", options(nostack));
                escribir_msr(PAT_MSR, nuevo);
                asm!("wbinvd", options(nostack));
            }
            PAT_WC_CONFIGURADA.store(true, Ordering::Release);
        }
    }

    page = first;
    loop {
        if let Some(entry) = huge_identity_entry(page) {
            unsafe { core::ptr::write_volatile(entry, core::ptr::read_volatile(entry) | HUGE_PAT) };
            flush(page);
        }
        if page == last {
            break;
        }
        page += HUGE_PAGE_SIZE;
    }
    true
}

/// Devuelve la entrada PDE que contiene `virt` en el mapa identidad del boot.
fn huge_identity_entry(virt: usize) -> Option<*mut u64> {
    let i4 = (virt >> 39) & 0x1FF;
    let i3 = (virt >> 30) & 0x1FF;
    let i2 = (virt >> 21) & 0x1FF;
    let pdpt = next_table(PML4_NUCLEO, i4)?;
    let pd = next_table(pdpt, i3)?;
    Some(entry_ptr(pd, i2))
}

unsafe fn leer_msr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nomem, nostack, preserves_flags)
    );
    ((hi as u64) << 32) | lo as u64
}

unsafe fn escribir_msr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nostack, preserves_flags)
    );
}

/// Invalida la entrada de la TLB para `virt`.
///
/// Necesario siempre que se cambie una entrada de pagina de una direccion que ya
/// estaba mapeada (por ejemplo, al reemplazar la imagen de un proceso en
/// `execve`): sin esto, el procesador podria seguir usando la traduccion vieja.
pub fn flush(virt: usize) {
    unsafe {
        asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags));
    }
}
