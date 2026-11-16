//! Estado de una conexión Kira con un cliente.

use alloc::vec::Vec;

use crate::kira::proto::Endian;

#[derive(Clone, Debug)]
pub struct Gc {
    pub id: u32,
    pub fg: u32,
    pub bg: u32,
}

#[derive(Clone, Debug)]
pub struct Window {
    pub id: u32,
    pub parent: u32,
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
    pub mapped: bool,
    pub event_mask: u32,
    /// Superficie RGB del cliente (sin cabecera ni marco del gestor).
    pub pixels: Vec<u32>,
    /// Orden global de mapeo: las ventanas más recientes quedan encima.
    pub z_order: u64,
}

#[derive(Clone, Debug)]
pub struct Conn {
    pub endian: Endian,
    pub xid_base: u32,
    pub xid_mask: u32,
    pub next_xid: u32,
    pub seq: u16,
    pub req: Vec<u8>,
    pub evq: Vec<u8>,
    pub windows: Vec<Window>,
    pub gcs: Vec<Gc>,
    /// Presupuesto de superficies para que una conexión no monopolice el heap.
    pub surface_pixels: usize,
    pub focus: u32,
    pub setup_done: bool,
    /// La conexión puede reutilizar su slot cuando se cierra el último fd.
    pub closed: bool,
}

impl Conn {
    pub fn new(xid_base: u32, xid_mask: u32) -> Self {
        Conn {
            endian: Endian::Lsb,
            xid_base,
            xid_mask,
            next_xid: 1,
            seq: 0,
            req: Vec::new(),
            evq: Vec::new(),
            windows: Vec::new(),
            gcs: Vec::new(),
            surface_pixels: 0,
            focus: 0,
            setup_done: false,
            closed: false,
        }
    }

    pub fn next_xid(&mut self) -> u32 {
        let id = self.xid_base | ((self.next_xid & self.xid_mask) as u32);
        self.next_xid += 1;
        id
    }
}
