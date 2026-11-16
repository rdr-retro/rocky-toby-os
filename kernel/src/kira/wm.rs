//! Gestor de ventanas interno del servidor kira.

use alloc::vec::Vec;

use crate::kira::conn::{Conn, Window};
use core::sync::atomic::{AtomicU64, Ordering};

const MAX_VENTANAS_POR_CLIENTE: usize = 128;
const MAX_PIXELES_POR_VENTANA: usize = 1_048_576;
const MAX_PIXELES_POR_CLIENTE: usize = 4_194_304;
static ORDEN_MAPEO: AtomicU64 = AtomicU64::new(1);

/// Asegura que la ventana raiz existe en la conexion `conn`.
///
/// Recibe la geometria del servidor como **parametros** (no la lee de KIRA):
/// quien la llama (`req::dispatch`) ya tiene prestado `&mut Conn`, y un
/// `estado_mut()` aqui volveria a tomar `&mut KiraState` **a la vez** que el que
/// mantiene `escribir_kira`, que es justo el alias mutable que la Fase 4 elimina.
pub fn ensure_root(conn: &mut Conn, root: u32, screen_w: i32, screen_h: i32) {
    // Asegurar que root existe
    if !conn.windows.iter().any(|w| w.id == root) {
        conn.windows.push(Window {
            id: root,
            parent: 0,
            x: 0,
            y: 0,
            w: screen_w as u16,
            h: screen_h as u16,
            mapped: true,
            event_mask: 0,
            pixels: Vec::new(),
            z_order: 0,
        });
    }
}

pub fn find_window(conn: &Conn, id: u32) -> Option<usize> {
    conn.windows.iter().position(|w| w.id == id)
}

pub fn create_window(conn: &mut Conn, parent: u32, wid: u32, x: i16, y: i16, w: u16, h: u16) {
    if w == 0
        || h == 0
        || conn.windows.len() >= MAX_VENTANAS_POR_CLIENTE
        || conn.windows.iter().any(|ventana| ventana.id == wid)
        || conn.windows.try_reserve(1).is_err()
    {
        return;
    }
    let Some(pixeles) = (w as usize).checked_mul(h as usize) else {
        return;
    };
    if pixeles > MAX_PIXELES_POR_VENTANA
        || conn.surface_pixels.saturating_add(pixeles) > MAX_PIXELES_POR_CLIENTE
    {
        return;
    }
    let mut superficie = Vec::new();
    if superficie.try_reserve_exact(pixeles).is_err() {
        return;
    }
    superficie.resize(pixeles, 0xFFFFFF);
    conn.surface_pixels += pixeles;
    conn.windows.push(Window {
        id: wid,
        parent,
        x,
        y,
        w,
        h,
        mapped: false,
        event_mask: 0,
        pixels: superficie,
        z_order: 0,
    });
}

/// Destruye una ventana y todos sus descendientes, liberando sus superficies.
pub fn destroy_window(conn: &mut Conn, wid: u32) {
    let Some(root) = find_window(conn, wid) else {
        return;
    };
    if conn.windows[root].parent == 0 {
        return;
    }
    let mut borrar = Vec::new();
    if borrar.try_reserve_exact(conn.windows.len()).is_err() {
        return;
    }
    borrar.push(wid);
    loop {
        let anterior = borrar.len();
        for ventana in &conn.windows {
            if borrar.contains(&ventana.parent) && !borrar.contains(&ventana.id) {
                borrar.push(ventana.id);
            }
        }
        if borrar.len() == anterior {
            break;
        }
    }
    let mut liberados = 0usize;
    conn.windows.retain(|ventana| {
        if borrar.contains(&ventana.id) {
            liberados = liberados.saturating_add(ventana.pixels.len());
            false
        } else {
            true
        }
    });
    conn.surface_pixels = conn.surface_pixels.saturating_sub(liberados);
}

pub fn orden_mapeo() -> u64 {
    ORDEN_MAPEO.fetch_add(1, Ordering::Relaxed)
}
