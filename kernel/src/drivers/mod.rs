//! Controladores de dispositivo (*drivers*) de rocky.
//!
//! rocky no porta Linux: cada controlador es codigo propio, pequeno y a medida
//! de lo que el sistema necesita. El arbol se organiza por **clase de
//! dispositivo** y, dentro de cada clase, por **fabricante**:
//!
//! ```text
//! drivers/
//!   gpu/          tarjetas graficas
//!     mod.rs      despacho por fabricante
//!     amd.rs      AMD/ATI (la RX 550 del PC real)
//!     intel.rs    Intel Haswell HD Graphics 4600 (perfil de pantalla VBE)
//!   usb/          pila USB
//!     mod.rs      inventario, tarea del HCD y orden `usb`
//!     pci.rs      seleccion por clase/subclase/prog_if
//!     dma.rs      asignacion, liberacion y validacion de DMA
//!     mmio.rs      acceso volatile y limites de BAR
//!     core/       enumeracion, descriptores y transferencias de control
//!     hcd/xhci/   controlador xHCI: anillos, puertos, HID boot y generico
//! ```
//!
//! El bus en si (enumeracion, BAR, espacio de configuracion) **no** es un
//! controlador: vive en [`crate::pci`], que solo *describe* los dispositivos.
//! Aqui esta lo que se *hace* con ellos.
//!
//! El soporte Intel Haswell mantiene el modo de pantalla preparado por el
//! firmware; el motor 3D nativo sigue pendiente de GTT, colas y gestión térmica.
//!
//! La pila USB cubre las fases 0 a 6 del plan: inventario de controladores de
//! host (solo lectura), xHCI en una tarea de fondo, enumeracion de dispositivos
//! en los puertos raiz y HID (protocolo boot y descriptor de informe). EHCI y
//! los hubs, y la validacion en hardware fisico, siguen pendientes.

pub mod gpu;
pub mod usb;

/// Arranca los controladores de dispositivo. La llama el nucleo **despues** de
/// barrer el bus PCI: los controladores necesitan el inventario de dispositivos.
///
/// Nota: esto corre **antes** de que exista el heap, asi que ningun
/// controlador puede reservar memoria dinamica todavia. El inventario USB es
/// un array fijo por eso.
pub fn init() {
    gpu::init();
    usb::init();
}
