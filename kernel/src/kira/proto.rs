//! Helpers para codificar/decodificar Kira/X11 wire protocol.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Endian {
    Lsb,
    Msb,
}

pub fn put_u8(out: &mut [u8], off: usize, v: u8) {
    out[off] = v;
}

pub fn u16_lsb(b: &[u8], off: usize) -> u16 {
    ((b[off + 1] as u16) << 8) | (b[off] as u16)
}

pub fn u16_msb(b: &[u8], off: usize) -> u16 {
    ((b[off] as u16) << 8) | (b[off + 1] as u16)
}

pub fn u32_lsb(b: &[u8], off: usize) -> u32 {
    ((b[off + 3] as u32) << 24)
        | ((b[off + 2] as u32) << 16)
        | ((b[off + 1] as u32) << 8)
        | (b[off] as u32)
}

pub fn u32_msb(b: &[u8], off: usize) -> u32 {
    ((b[off] as u32) << 24)
        | ((b[off + 1] as u32) << 16)
        | ((b[off + 2] as u32) << 8)
        | (b[off + 3] as u32)
}

pub fn put_u16_lsb(out: &mut [u8], off: usize, v: u16) {
    out[off] = (v & 0xFF) as u8;
    out[off + 1] = ((v >> 8) & 0xFF) as u8;
}

pub fn put_u16_msb(out: &mut [u8], off: usize, v: u16) {
    out[off] = ((v >> 8) & 0xFF) as u8;
    out[off + 1] = (v & 0xFF) as u8;
}

pub fn put_u32_lsb(out: &mut [u8], off: usize, v: u32) {
    out[off] = (v & 0xFF) as u8;
    out[off + 1] = ((v >> 8) & 0xFF) as u8;
    out[off + 2] = ((v >> 16) & 0xFF) as u8;
    out[off + 3] = ((v >> 24) & 0xFF) as u8;
}

pub fn put_u32_msb(out: &mut [u8], off: usize, v: u32) {
    out[off] = ((v >> 24) & 0xFF) as u8;
    out[off + 1] = ((v >> 16) & 0xFF) as u8;
    out[off + 2] = ((v >> 8) & 0xFF) as u8;
    out[off + 3] = (v & 0xFF) as u8;
}

pub fn get_u16(buf: &[u8], off: usize, e: Endian) -> u16 {
    match e {
        Endian::Lsb => u16_lsb(buf, off),
        Endian::Msb => u16_msb(buf, off),
    }
}

pub fn get_u32(buf: &[u8], off: usize, e: Endian) -> u32 {
    match e {
        Endian::Lsb => u32_lsb(buf, off),
        Endian::Msb => u32_msb(buf, off),
    }
}

pub fn put_u16(out: &mut [u8], off: usize, v: u16, e: Endian) {
    match e {
        Endian::Lsb => put_u16_lsb(out, off, v),
        Endian::Msb => put_u16_msb(out, off, v),
    }
}

pub fn put_u32(out: &mut [u8], off: usize, v: u32, e: Endian) {
    match e {
        Endian::Lsb => put_u32_lsb(out, off, v),
        Endian::Msb => put_u32_msb(out, off, v),
    }
}
