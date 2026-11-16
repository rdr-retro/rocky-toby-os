//! Servidor Kira Window Server mínimo para rocky.
//!
//! El servidor Kira se comunica con clientes de anillo 3 a través de un
//! descriptor de tipo `FdKind::Kira`. El cliente habla protocolo Kira wire
//! (compatible con X11 core mínimo).
#![allow(dead_code)] // scaffolding: campos y funciones para uso futuro del servidor

use crate::sync::SyncCell;
use alloc::vec::Vec;

pub mod comp;
pub mod conn;
pub mod desktop;
pub mod dev;
pub mod event;
pub mod genie;
pub mod input;
pub mod proto;
pub mod req;
pub mod wm;

static KIRA: SyncCell<Option<KiraState>> = SyncCell::new(None);

#[derive(Clone, Debug)]
pub struct KiraState {
    pub screen_w: i32,
    pub screen_h: i32,
    pub root_wid: u32,
    pub conns: Vec<conn::Conn>,
}

pub fn opcion() -> &'static mut Option<KiraState> {
    unsafe { &mut *KIRA.get() }
}

pub fn estado_mut() -> &'static mut KiraState {
    opcion().as_mut().expect("kira no inicializado")
}

pub fn activo() -> bool {
    unsafe { (&*KIRA.get()).is_some() }
}

pub fn iniciar(sw: i32, sh: i32) {
    if activo() {
        return;
    }
    let root = 0x00000001;
    *opcion() = Some(KiraState {
        screen_w: sw,
        screen_h: sh,
        root_wid: root,
        conns: Vec::new(),
    });
}

/// Ajusta la geometria del servidor a la del modo de video realmente activo.
///
/// La llama el compositor al reanudar: el modo pedido con `gui <ancho>x<alto>`
/// puede no existir y el BIOS acabar ofreciendo otro, y el servidor tiene que
/// reflejar el que esta de verdad en pantalla.
pub fn ajustar_pantalla(sw: i32, sh: i32) {
    if let Some(k) = opcion().as_mut() {
        k.screen_w = sw;
        k.screen_h = sh;
    }
}

/// Para el servidor: descarta el estado (y con el, todas las conexiones).
pub fn parar() {
    *opcion() = None;
}
