//! Heap del kernel: asignador dinamico de memoria para rocky.
//!
//! Interponemos una lista enlazada de bloques libres sobre la ventana virtual
//! de [`crate::paging`]. Cuando una peticion no cabe, se mapean marcos nuevos
//! ([`crate::pmm`]) y se anaden a la lista. Los bloques contiguos se fusionan al
//! liberar para luchar contra la fragmentacion.
//!
//! Fase 6: *slabs* por clases de tamaño (8,16,32,64,128,256,512,1024,2048)
//! para objetos frecuentes; ruta de bloques grandes intacta.
//!
//! Con esto el kernel puede usar `Vec`, `String`, `Box`, etc. Diseño propio.

use core::alloc::{GlobalAlloc, Layout};
use core::{mem, ptr};

use crate::cpu::{sin_interrupciones, IrqOff};
use crate::paging::{self, HEAP_BASE};
use crate::pmm::{self, FRAME_SIZE};
use crate::sync::SyncCell;

/// Paginas que se mapean de golpe al arrancar (1 MiB).
const INITIAL_PAGES: usize = 256;
/// Paginas minimas que se añaden cuando el heap se queda corto (1 MiB).
const GROW_PAGES: usize = 256;

/// Clases de slab (Fase 6)
const SLAB_SIZES: [usize; 9] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048];

#[repr(C)]
struct SlabObj {
    next: *mut SlabObj,
}

struct Slab {
    free: *mut SlabObj,
    obj_size: usize,
}

struct HeapState {
    free_head: *mut Node,
    /// Primer byte virtual aun sin mapear.
    mapped_end: usize,
    slabs: [Slab; 9],
}

/// Cabecera de un bloque libre. Vive al principio del propio bloque.
#[repr(C)]
struct Node {
    size: usize,
    next: *mut Node,
}

static STATE: SyncCell<HeapState> = SyncCell::new(HeapState {
    free_head: ptr::null_mut(),
    mapped_end: HEAP_BASE,
    slabs: [
        Slab {
            free: ptr::null_mut(),
            obj_size: 8,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 16,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 32,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 64,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 128,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 256,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 512,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 1024,
        },
        Slab {
            free: ptr::null_mut(),
            obj_size: 2048,
        },
    ],
});

/// El asignador global de rocky.
pub struct Heap;

#[inline]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[inline]
fn node_header() -> usize {
    mem::size_of::<Node>()
}

/// Inserta un bloque libre en la lista ordenada por direccion, fusionando con
/// los bloques contiguos que lo rodeen.
unsafe fn add_free_region(state: &mut HeapState, addr: usize, size: usize) {
    if size < node_header() {
        return;
    }
    let node = addr as *mut Node;
    (*node).size = size;
    (*node).next = ptr::null_mut();

    let mut prev: *mut Node = ptr::null_mut();
    let mut cur = state.free_head;
    while !cur.is_null() && (cur as usize) < addr {
        prev = cur;
        cur = (*cur).next;
    }

    // Fusiona con el bloque siguiente si son contiguos.
    if !cur.is_null() && addr + size == cur as usize {
        (*node).size += (*cur).size;
        (*node).next = (*cur).next;
    } else {
        (*node).next = cur;
    }

    // Enlaza con el anterior y fusiona si son contiguos.
    if prev.is_null() {
        state.free_head = node;
    } else if (prev as usize) + (*prev).size == addr {
        (*prev).size += (*node).size;
        (*prev).next = (*node).next;
    } else {
        (*prev).next = node;
    }
}

/// Intenta servir una peticion de `size` bytes alineados a `align` sin crecer.
unsafe fn try_alloc(state: &mut HeapState, size: usize, align: usize) -> Option<*mut u8> {
    let mut prev: *mut Node = ptr::null_mut();
    let mut cur = state.free_head;
    while !cur.is_null() {
        let block_start = cur as usize;
        let block_size = (*cur).size;
        let block_end = block_start + block_size;

        let data = align_up(block_start + node_header(), align);
        let data_end = data + size;

        if data_end <= block_end {
            let remainder = block_end - data_end;
            let next = (*cur).next;

            if remainder >= node_header() + mem::align_of::<Node>() {
                // Divide: el bloque empieza igual, y dejamos libre la cola.
                let tail = data_end as *mut Node;
                (*tail).size = remainder;
                (*tail).next = next;

                if data == block_start {
                    // El bloque entero se consume: la cola lo reemplaza.
                    if prev.is_null() {
                        state.free_head = tail;
                    } else {
                        (*prev).next = tail;
                    }
                } else {
                    (*cur).size = data - block_start;
                    (*cur).next = tail;
                }
            } else {
                // Se consume el bloque entero tal cual.
                if prev.is_null() {
                    state.free_head = next;
                } else {
                    (*prev).next = next;
                }
            }
            return Some(data as *mut u8);
        }

        prev = cur;
        cur = (*cur).next;
    }
    None
}

/// Mapea `pages` marcos nuevos y los añade a la lista de libres.
unsafe fn grow(state: &mut HeapState, pages: usize) -> bool {
    let start = state.mapped_end;
    let mut mapped = 0usize;
    for _ in 0..pages {
        let frame = match pmm::alloc_frame() {
            Some(f) => f,
            None => break,
        };
        if paging::map(state.mapped_end, frame).is_err() {
            pmm::free_frame(frame);
            break;
        }
        state.mapped_end += FRAME_SIZE;
        mapped += 1;
    }
    if mapped == 0 {
        return false;
    }
    crate::metricas::heap_grow();
    add_free_region(state, start, mapped * FRAME_SIZE);
    true
}

#[inline]
fn slab_class(size: usize) -> Option<usize> {
    for (i, &s) in SLAB_SIZES.iter().enumerate() {
        if size <= s {
            return Some(i);
        }
    }
    None
}

unsafe fn slab_grow(state: &mut HeapState, class: usize) {
    // Mapear un marco y partirlo en objetos de esa clase
    let frame = match pmm::alloc_frame() {
        Some(f) => f,
        None => return,
    };
    if paging::map(state.mapped_end, frame).is_err() {
        pmm::free_frame(frame);
        return;
    }
    let base = state.mapped_end;
    state.mapped_end += FRAME_SIZE;
    let obj_size = state.slabs[class].obj_size.max(mem::size_of::<SlabObj>());
    let mut off = 0usize;
    while off + obj_size <= FRAME_SIZE {
        let obj = (base + off) as *mut SlabObj;
        (*obj).next = state.slabs[class].free;
        state.slabs[class].free = obj;
        off += obj_size;
    }
}

/// Prepara el heap. Debe llamarse despues de [`crate::paging::init`].
pub fn init() {
    let state = unsafe { &mut *STATE.get() };
    state.free_head = ptr::null_mut();
    state.mapped_end = HEAP_BASE;
    // Inicializar slabs
    for i in 0..SLAB_SIZES.len() {
        state.slabs[i] = Slab {
            free: ptr::null_mut(),
            obj_size: SLAB_SIZES[i],
        };
    }
    unsafe {
        grow(state, INITIAL_PAGES);
        // Precalentar slabs pequeños
        for i in 0..4 {
            slab_grow(state, i);
        }
    }
}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _irq = IrqOff::medir();
        let state = &mut *STATE.get();
        let size_req = layout.size();
        if let Some(class) = slab_class(size_req) {
            let obj_size = state.slabs[class].obj_size;
            if obj_size >= layout.align() {
                let freep = state.slabs[class].free;
                if !freep.is_null() {
                    state.slabs[class].free = (*freep).next;
                    crate::metricas::alloc(size_req);
                    return freep as *mut u8;
                }
                slab_grow(state, class);
                let freep2 = state.slabs[class].free;
                if !freep2.is_null() {
                    state.slabs[class].free = (*freep2).next;
                    crate::metricas::alloc(size_req);
                    return freep2 as *mut u8;
                }
            }
        }
        let size = align_up(size_req.max(node_header()), mem::align_of::<Node>());
        let align = layout.align().max(mem::align_of::<Node>());
        loop {
            if let Some(ptr) = try_alloc(state, size, align) {
                crate::metricas::alloc(size_req);
                return ptr;
            }
            let need = (size + node_header() + FRAME_SIZE - 1) / FRAME_SIZE;
            if !grow(state, need.max(GROW_PAGES)) {
                return ptr::null_mut();
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _irq = IrqOff::medir();
        let size_req = layout.size();
        if let Some(class) = slab_class(size_req) {
            let _obj_size = SLAB_SIZES[class];
            if (ptr as usize) >= HEAP_BASE {
                let obj = ptr as *mut SlabObj;
                (*obj).next = STATE.get().as_mut().unwrap().slabs[class].free;
                STATE.get().as_mut().unwrap().slabs[class].free = obj;
                crate::metricas::dealloc();
                return;
            }
        }
        let state = &mut *STATE.get();
        let size = align_up(size_req.max(node_header()), mem::align_of::<Node>());
        add_free_region(state, ptr as usize, size);
        crate::metricas::dealloc();
    }
}

/// Bytes ya mapeados en la ventana del heap.
pub fn mapped_bytes() -> usize {
    let state = unsafe { &*STATE.get() };
    state.mapped_end - HEAP_BASE
}

/// Bytes libres dentro del heap (recorre la lista de bloques libres).
pub fn free_bytes() -> usize {
    unsafe {
        sin_interrupciones(|| {
            let state = &*STATE.get();
            let mut free = 0usize;
            let mut cur = state.free_head;
            while !cur.is_null() {
                free += (*cur).size;
                cur = (*cur).next;
            }
            free
        })
    }
}
