//! Despacho de requests Kira/X11.

use alloc::vec::Vec;

use crate::gfx::Canvas;
use crate::kira::conn::{Conn, Gc};
use crate::kira::event::expose_event;
use crate::kira::proto::{get_u16, get_u32};
use crate::kira::wm::{create_window, ensure_root, find_window};

/// Despacha un request sobre la conexion `conn`.
///
/// `root_wid`/`screen_w`/`screen_h` son la geometria de la ventana raiz del
/// servidor. Se pasan explicitamente para que `ensure_root` no tenga que volver
/// a tomar `&mut KiraState`: el llamante (`escribir_kira`) ya mantiene prestado
/// el estado y un segundo `estado_mut()` seria un alias mutable (Fase 4).
pub fn dispatch(
    conn: &mut Conn,
    data: &[u8],
    root_wid: u32,
    screen_w: i32,
    screen_h: i32,
) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let opcode = data[0];
    match opcode {
        1 => {
            // CreateWindow
            if data.len() < 20 {
                return Vec::new();
            }
            ensure_root(conn, root_wid, screen_w, screen_h);
            let parent = get_u32(data, 4, conn.endian);
            let wid = get_u32(data, 8, conn.endian);
            if find_window(conn, parent).is_none() {
                return Vec::new();
            }
            let x = get_u16(data, 12, conn.endian) as i16;
            let y = get_u16(data, 14, conn.endian) as i16;
            let w = get_u16(data, 16, conn.endian);
            let h = get_u16(data, 18, conn.endian);
            create_window(conn, parent, wid, x, y, w, h);
            Vec::new()
        }
        4 => {
            // DestroyWindow
            if data.len() < 8 {
                return Vec::new();
            }
            crate::kira::wm::destroy_window(conn, get_u32(data, 4, conn.endian));
            Vec::new()
        }
        8 | 9 => {
            // MapWindow / MapRaised: mapea, lo sube y genera Expose.
            if data.len() < 8 {
                return Vec::new();
            }
            let wid = get_u32(data, 4, conn.endian);
            if let Some(idx) = find_window(conn, wid) {
                conn.windows[idx].mapped = true;
                conn.windows[idx].z_order = crate::kira::wm::orden_mapeo();
                let w = conn.windows[idx].w;
                let h = conn.windows[idx].h;
                let seq = conn.seq;
                let endian = conn.endian;
                return expose_event(seq, wid, 0, 0, w, h, 0, endian);
            }
            Vec::new()
        }
        10 => {
            // UnmapWindow
            if data.len() >= 8 {
                let wid = get_u32(data, 4, conn.endian);
                if let Some(idx) = find_window(conn, wid) {
                    conn.windows[idx].mapped = false;
                }
            }
            Vec::new()
        }
        55 | 56 => {
            // CreateGC / ChangeGC: foreground (bit 2) y background (bit 3).
            let (id_off, mask_off, values_off) = if opcode == 55 {
                (8, 12, 16)
            } else {
                (4, 8, 12)
            };
            if data.len() < values_off {
                return Vec::new();
            }
            let id = get_u32(data, id_off, conn.endian);
            if opcode == 55 {
                if conn.gcs.iter().any(|gc| gc.id == id)
                    || conn.gcs.len() >= 256
                    || conn.gcs.try_reserve(1).is_err()
                {
                    return Vec::new();
                }
                conn.gcs.push(Gc {
                    id,
                    fg: 0x000000,
                    bg: 0xFFFFFF,
                });
            }
            let mask = get_u32(data, mask_off, conn.endian);
            if let Some(gc) = conn.gcs.iter_mut().find(|gc| gc.id == id) {
                aplicar_valores_gc(gc, mask, &data[values_off..], conn.endian);
            }
            Vec::new()
        }
        60 => {
            // FreeGC
            if data.len() >= 8 {
                let id = get_u32(data, 4, conn.endian);
                if let Some(idx) = conn.gcs.iter().position(|gc| gc.id == id) {
                    conn.gcs.remove(idx);
                }
            }
            Vec::new()
        }
        61 => {
            // ClearArea (el fondo inicial de Kira es blanco).
            if data.len() < 16 {
                return Vec::new();
            }
            let wid = get_u32(data, 4, conn.endian);
            let x = get_u16(data, 8, conn.endian) as i16 as i32;
            let y = get_u16(data, 10, conn.endian) as i16 as i32;
            if let Some(idx) = find_window(conn, wid) {
                let win = &conn.windows[idx];
                let w = if get_u16(data, 12, conn.endian) == 0 {
                    win.w as i32 - x
                } else {
                    get_u16(data, 12, conn.endian) as i32
                };
                let h = if get_u16(data, 14, conn.endian) == 0 {
                    win.h as i32 - y
                } else {
                    get_u16(data, 14, conn.endian) as i32
                };
                pintar_rect(conn, wid, x, y, w, h, 0xFFFFFF);
            }
            Vec::new()
        }
        64 => {
            // PolyPoint, tanto coordenadas absolutas como relativas.
            if data.len() < 12 {
                return Vec::new();
            }
            let wid = get_u32(data, 4, conn.endian);
            let color = color_gc(conn, get_u32(data, 8, conn.endian));
            let endian = conn.endian;
            let mut x = 0i32;
            let mut y = 0i32;
            let mut primero = true;
            con_superficie(conn, wid, |c| {
                for punto in data[12..].chunks_exact(4) {
                    let px = get_u16(punto, 0, endian) as i16 as i32;
                    let py = get_u16(punto, 2, endian) as i16 as i32;
                    if data[1] == 1 && !primero {
                        x += px;
                        y += py;
                    } else {
                        x = px;
                        y = py;
                    }
                    c.put(x, y, color);
                    primero = false;
                }
            });
            Vec::new()
        }
        65 => {
            // PolyLine: segmentos consecutivos, con clip de superficie.
            if data.len() < 12 {
                return Vec::new();
            }
            let wid = get_u32(data, 4, conn.endian);
            let color = color_gc(conn, get_u32(data, 8, conn.endian));
            let relativo = data[1] == 1;
            let endian = conn.endian;
            con_superficie(conn, wid, |c| {
                let mut puntos = data[12..].chunks_exact(4);
                if let Some(punto) = puntos.next() {
                    let mut x = get_u16(punto, 0, endian) as i16 as i32;
                    let mut y = get_u16(punto, 2, endian) as i16 as i32;
                    for punto in puntos {
                        let px = get_u16(punto, 0, endian) as i16 as i32;
                        let py = get_u16(punto, 2, endian) as i16 as i32;
                        let (nx, ny) = if relativo { (x + px, y + py) } else { (px, py) };
                        c.line(x, y, nx, ny, color);
                        x = nx;
                        y = ny;
                    }
                }
            });
            Vec::new()
        }
        67 => {
            // PolyRectangle
            if data.len() < 12 {
                return Vec::new();
            }
            let wid = get_u32(data, 4, conn.endian);
            let color = color_gc(conn, get_u32(data, 8, conn.endian));
            let endian = conn.endian;
            con_superficie(conn, wid, |c| {
                for rect in data[12..].chunks_exact(8) {
                    let x = get_u16(rect, 0, endian) as i16 as i32;
                    let y = get_u16(rect, 2, endian) as i16 as i32;
                    let w = get_u16(rect, 4, endian) as i32;
                    let h = get_u16(rect, 6, endian) as i32;
                    if w > 0 && h > 0 {
                        c.hline(x, x + w - 1, y, color);
                        c.hline(x, x + w - 1, y + h - 1, color);
                        c.vline(x, y, y + h - 1, color);
                        c.vline(x + w - 1, y, y + h - 1, color);
                    }
                }
            });
            Vec::new()
        }
        70 => {
            // PolyFillRectangle: varias formas en una sola request X11.
            if data.len() < 12 {
                return Vec::new();
            }
            let wid = get_u32(data, 4, conn.endian);
            let color = color_gc(conn, get_u32(data, 8, conn.endian));
            let endian = conn.endian;
            con_superficie(conn, wid, |c| {
                for rect in data[12..].chunks_exact(8) {
                    let x = get_u16(rect, 0, endian) as i16 as i32;
                    let y = get_u16(rect, 2, endian) as i16 as i32;
                    let w = get_u16(rect, 4, endian) as i32;
                    let h = get_u16(rect, 6, endian) as i32;
                    c.fill_rect(x, y, w, h, color);
                }
            });
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn aplicar_valores_gc(gc: &mut Gc, mask: u32, values: &[u8], endian: crate::kira::proto::Endian) {
    let mut off = 0usize;
    for bit in 0..32 {
        if mask & (1 << bit) == 0 {
            continue;
        }
        if off + 4 > values.len() {
            return;
        }
        let value = get_u32(values, off, endian);
        match bit {
            2 => gc.fg = value & 0xFFFFFF,
            3 => gc.bg = value & 0xFFFFFF,
            _ => {}
        }
        off += 4;
    }
}

fn color_gc(conn: &Conn, id: u32) -> u32 {
    conn.gcs
        .iter()
        .find(|gc| gc.id == id)
        .map(|gc| gc.fg)
        .unwrap_or(0)
}

fn con_superficie<F: FnOnce(&Canvas)>(conn: &mut Conn, wid: u32, pintar: F) {
    let Some(win) = conn.windows.iter_mut().find(|win| win.id == wid) else {
        return;
    };
    if win.pixels.is_empty() {
        return;
    }
    let superficie = Canvas::from_buffer(
        win.pixels.as_mut_ptr(),
        win.w as i32,
        win.h as i32,
        win.w as usize,
    );
    pintar(&superficie);
}

fn pintar_rect(conn: &mut Conn, wid: u32, x: i32, y: i32, w: i32, h: i32, color: u32) {
    con_superficie(conn, wid, |c| c.fill_rect(x, y, w, h, color));
}
