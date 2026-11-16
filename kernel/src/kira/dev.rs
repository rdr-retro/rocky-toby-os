//! IntegraciÃ³n con fd.rs para exponer /dev/kira como descriptor.

use alloc::vec;
use alloc::vec::Vec;

use crate::kira::proto::{put_u16, put_u32, put_u8, Endian};
use crate::kira::req;
use crate::kira::{self, conn::Conn};

const MAX_BUFFER_PROTOCOLO: usize = 65_536;

pub fn nueva_conexion() -> Option<u32> {
    if !kira::activo() {
        kira::iniciar(1024, 768);
    }
    let st = kira::estado_mut();
    let id = st
        .conns
        .iter()
        .position(|conn| conn.closed)
        .unwrap_or(st.conns.len());
    // Cada cliente recibe un byte alto distinto. Empezamos en 0x20 para
    // mantenernos fuera de los XID reservados del servidor (p. ej. root=1).
    if id > 0xDF {
        return None;
    }
    let base = ((0x20 + id as u32) & 0xFF) << 24;
    let mask = 0x00FFFFFFu32;
    let mut c = Conn::new(base, mask);
    c.focus = st.root_wid;
    if id == st.conns.len() {
        if st.conns.try_reserve(1).is_err() {
            return None;
        }
        st.conns.push(c);
    } else {
        st.conns[id] = c;
    }
    Some(id as u32)
}

/// Destruye todos los recursos de la conexión al cerrarse su último fd.
pub fn cerrar_conexion(cid: u32) {
    let cerrada = if let Some(conn) = kira::opcion()
        .as_mut()
        .and_then(|state| state.conns.get_mut(cid as usize))
    {
        if conn.closed {
            false
        } else {
            conn.closed = true;
            conn.req.clear();
            conn.evq.clear();
            conn.windows.clear();
            conn.gcs.clear();
            conn.surface_pixels = 0;
            true
        }
    } else {
        false
    };
    if cerrada {
        crate::kira::comp::repintar_ventanas();
    }
}

pub fn leer_kira(cid: u32, buf: &mut [u8]) -> i64 {
    let st = kira::opcion();
    if let Some(s) = st.as_mut() {
        if let Some(c) = s.conns.get_mut(cid as usize).filter(|c| !c.closed) {
            let n = c.evq.len().min(buf.len());
            if n == 0 {
                return 0;
            }
            buf[..n].copy_from_slice(&c.evq[..n]);
            c.evq.drain(..n);
            return n as i64;
        }
    }
    -1
}

fn connection_setup_reply(
    xid_base: u32,
    xid_mask: u32,
    root: u32,
    screen_w: i32,
    screen_h: i32,
) -> Vec<u8> {
    // Todos los datos llegan como parametros: el llamante (`escribir_kira`) ya
    // mantiene prestado `&mut KiraState`, y volver a leer KIRA aqui crearia un
    // alias mutable al mismo estado (Fase 4).
    let (sw, sh) = (screen_w as u16, screen_h as u16);
    let vendor = b"Kira";
    let vendor_pad = (vendor.len() + 3) & !3;
    let mut v: Vec<u8> = vec![0; 128 + vendor_pad];
    let mut off = 0;
    put_u8(&mut v, off, 1);
    off += 1;
    put_u8(&mut v, off, 0);
    off += 1;
    put_u16(&mut v, off, 11, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, 0, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, 0, Endian::Lsb);
    off += 2;
    put_u32(&mut v, off, 11, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, xid_base, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, xid_mask, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    put_u16(&mut v, off, vendor.len() as u16, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, 16384, Endian::Lsb);
    off += 2;
    put_u8(&mut v, off, 1);
    off += 1;
    put_u8(&mut v, off, 1);
    off += 1;
    put_u8(&mut v, off, 0);
    off += 1;
    put_u8(&mut v, off, 0);
    off += 1;
    put_u8(&mut v, off, 32);
    off += 1;
    put_u8(&mut v, off, 32);
    off += 1;
    put_u8(&mut v, off, 8);
    off += 1;
    put_u8(&mut v, off, 255);
    off += 1;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    for (i, b) in vendor.iter().enumerate() {
        v[off + i] = *b;
    }
    off += vendor_pad;
    put_u8(&mut v, off, 24);
    off += 1;
    put_u8(&mut v, off, 32);
    off += 1;
    put_u8(&mut v, off, 32);
    off += 1;
    put_u8(&mut v, off, 0);
    off += 1;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4; // format pad: 8 bytes total
    put_u32(&mut v, off, root, Endian::Lsb);
    off += 4;
    put_u16(&mut v, off, sw, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, sh, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, sw * 10, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, sh * 10, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, 1, Endian::Lsb);
    off += 2;
    put_u32(&mut v, off, 0x23, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0xFFFFFF, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    put_u16(&mut v, off, 0, Endian::Lsb);
    off += 2;
    put_u16(&mut v, off, 0, Endian::Lsb);
    off += 2;
    put_u8(&mut v, off, 24);
    off += 1;
    put_u8(&mut v, off, 0);
    off += 1;
    put_u16(&mut v, off, 1, Endian::Lsb);
    off += 2;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0x23, Endian::Lsb);
    off += 4;
    put_u8(&mut v, off, 4);
    off += 1;
    put_u8(&mut v, off, 8);
    off += 1;
    put_u16(&mut v, off, 256, Endian::Lsb);
    off += 2;
    put_u32(&mut v, off, 0xFF0000, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0x00FF00, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0x0000FF, Endian::Lsb);
    off += 4;
    put_u32(&mut v, off, 0, Endian::Lsb);
    off += 4;
    let add = (off as u16).wrapping_sub(8) / 4;
    put_u16(&mut v, 6, add, Endian::Lsb);
    v.truncate(off);
    v
}

pub fn escribir_kira(cid: u32, datos: &[u8]) -> i64 {
    let resultado = {
        let st = kira::estado_mut();
        // La geometria de la raiz se copia **antes** de prestar `c` de `st`:
        // `req::dispatch` la necesita y no puede volver a leer KIRA sin crear un
        // segundo `&mut KiraState` (Fase 4).
        let root_wid = st.root_wid;
        let screen_w = st.screen_w;
        let screen_h = st.screen_h;
        if let Some(c) = st.conns.get_mut(cid as usize).filter(|c| !c.closed) {
            if datos.len() > MAX_BUFFER_PROTOCOLO
                || c.req.len().saturating_add(datos.len()) > MAX_BUFFER_PROTOCOLO
                || c.req.try_reserve(datos.len()).is_err()
            {
                return -1;
            }
            c.req.extend_from_slice(datos);
            if !c.setup_done && c.req.len() >= 12 {
                let order = c.req[0];
                c.endian = if order == b'l' {
                    Endian::Lsb
                } else {
                    Endian::Msb
                };
                c.req.clear();
                let (xid_base, xid_mask) = (c.xid_base, c.xid_mask);
                let rep = connection_setup_reply(xid_base, xid_mask, root_wid, screen_w, screen_h);
                if c.evq.len().saturating_add(rep.len()) > MAX_BUFFER_PROTOCOLO
                    || c.evq.try_reserve(rep.len()).is_err()
                {
                    return -1;
                }
                c.evq.extend_from_slice(&rep);
                c.setup_done = true;
                c.seq = 0;
                return datos.len() as i64;
            }
            loop {
                if c.req.len() < 4 {
                    break;
                }
                let len = (crate::kira::proto::get_u16(&c.req, 2, c.endian) as usize) * 4;
                if len < 4 {
                    c.req.clear();
                    return -1;
                }
                if len > MAX_BUFFER_PROTOCOLO {
                    c.req.clear();
                    return -1;
                }
                if c.req.len() < len {
                    break;
                }
                let mut reqb = Vec::new();
                if reqb.try_reserve_exact(len).is_err() {
                    c.req.clear();
                    return -1;
                }
                reqb.extend_from_slice(&c.req[..len]);
                c.req.drain(..len);
                let resp = req::dispatch(c, &reqb, root_wid, screen_w, screen_h);
                if !resp.is_empty() {
                    if c.evq.len().saturating_add(resp.len()) <= MAX_BUFFER_PROTOCOLO
                        && c.evq.try_reserve(resp.len()).is_ok()
                    {
                        c.evq.extend_from_slice(&resp);
                    }
                }
                c.seq = c.seq.wrapping_add(1);
            }
            datos.len() as i64
        } else {
            -1
        }
    };
    if resultado >= 0 {
        crate::kira::comp::repintar_ventanas();
    }
    resultado
}
