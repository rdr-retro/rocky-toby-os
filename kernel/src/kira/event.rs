//! Generación de eventos Kira/X11.

use alloc::vec;
use alloc::vec::Vec;

use crate::kira::proto::{put_u16, put_u32, put_u8, Endian};

pub fn expose_event(
    conn_seq: u16,
    window: u32,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    count: u16,
    endian: Endian,
) -> Vec<u8> {
    let mut ev = vec![0u8; 32];
    put_u8(&mut ev, 0, 12);
    put_u8(&mut ev, 1, 0);
    put_u16(&mut ev, 2, conn_seq, endian);
    put_u32(&mut ev, 4, window, endian);
    put_u16(&mut ev, 8, x, endian);
    put_u16(&mut ev, 10, y, endian);
    put_u16(&mut ev, 12, w, endian);
    put_u16(&mut ev, 14, h, endian);
    put_u16(&mut ev, 16, count, endian);
    ev
}
