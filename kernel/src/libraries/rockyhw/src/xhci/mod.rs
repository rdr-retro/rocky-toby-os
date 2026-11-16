//! Logica pura del controlador de host xHCI.
//!
//! ## Que hay aqui (y que no)
//!
//! Aqui esta **todo lo que se puede decidir sin hardware delante**: como se
//! leen y validan las capacidades de un controlador, como se decodifica el
//! estado de un puerto raiz, como es un TRB y como se vacila un anillo de
//! eventos.
//!
//! Lo que **no** hay aqui es el acceso al hardware. El driver de verdad
//! (en `kernel/src/drivers/usb/hcd/xhci`) usa estos modulos para decidir,
//! y usa `mmio` y `dma` de esta misma libreria para hablar con el
//! controlador. Aqui no se escribe ni un registro.
//!
//! ## Por que esta parte vive en la libreria
//!
//! Porque asi se puede probar en el anfitrion (`cargo test`) sin una
//! maquina virtual y sin un controlador USB de verdad. Un mapa de registros
//! de mentira implementa [`caps::Registro`] y una memoria de mentira
//! implementa [`ring::MemoriaTrb`]: el codigo que decide es el mismo que
//! luego corre contra el hardware.
//!
//! ## Referencia
//!
//! Intel, "eXtensible Host Controller Interface for Universal Serial Bus
//! (xHCI) Requirements Specification", Rev 1.2 (mayo de 2019).
//!
//! Codigo propio de rocky.

pub mod caps;
pub mod context;
pub mod port;
pub mod regs;
pub mod ring;

pub use caps::{Capacidades, ErrorCap, Protocolo, Protocolos, RangoPuertos, Registro};
pub use context::{
    ep_id, trb_pointer_con_dcs, DeviceContext, EndpointContext, InputContext, InputControlContext,
    SlotContext,
};
pub use port::{EstadoPuerto, Velocidad};
pub use ring::{AnilloComando, AnilloEvento, TipoEvento, TipoTrb, Trb};
