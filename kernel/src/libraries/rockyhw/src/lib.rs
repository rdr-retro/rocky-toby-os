//! Primitivas de hardware de rocky: bus PCI, reserva de memoria para DMA y
//! acceso acotado a la memoria de un dispositivo (MMIO).
//!
//! Es una libreria `no_std` (solo `core`) porque vive dentro del kernel, pero
//! **no depende de nada del kernel**: aqui solo hay logica pura y accesos por
//! puntero. Asi se puede compilar y probar en el anfitrion (`cargo test`) sin
//! hardware USB, que es lo que pide el plan para las primitivas de la fase 1.
//!
//! El barrido del bus, el acceso al espacio de configuracion y el gestor de
//! marcos siguen en el kernel (`pci.rs` y `pmm.rs`): esta libreria solo
//! describe y valida.
//!
//! Codigo propio de rocky.

#![no_std]

pub mod cola;
pub mod dma;
pub mod hid;
pub mod mmio;
pub mod pci;
pub mod xhci;
