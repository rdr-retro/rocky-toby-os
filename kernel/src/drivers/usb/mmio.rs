//! Acceso a los registros de un controlador USB.
//!
//! Es la capa fina entre el bus PCI y el driver del controlador: comprueba que
//! el BAR de registros es utilizable y devuelve una ventana acotada, en la que
//! cada acceso comprueba su rango antes de tocar el dispositivo.
//!
//! El tope del mapa por identidad (4 GiB) se aplica aqui: un BAR de 64 bits
//! por encima de ese limite no se puede leer sin ampliar la paginacion, y
//! preferimos saberlo ahora y no cuando el controlador este a medio
//! inicializar.
//!
//! Codigo propio de rocky.

use crate::pci::Bar;
use rockyhw::dma::LIMITE_4GIB;

/// La ventana de registros de un dispositivo, con sus limites.
pub use rockyhw::mmio::Mmio;

/// Por que no se puede abrir la ventana de registros.
pub use rockyhw::mmio::ErrorMmio;

/// Barrera de orden de escrituras y lecturas.
pub use rockyhw::mmio::mfence;

/// Tira de la cache un rango que puede ocupar varias lineas.
pub use rockyhw::mmio::clflush_rango;

/// Abre la ventana de registros de un BAR ya medido.
///
/// Falla si el BAR es de espacio de E/S (sus registros no son memoria) o si la
/// ventana queda por encima del mapa por identidad del arranque.
pub fn ventana(bar: &Bar) -> Result<Mmio, ErrorMmio> {
    Mmio::de_bar_en_mapa(bar, LIMITE_4GIB)
}
