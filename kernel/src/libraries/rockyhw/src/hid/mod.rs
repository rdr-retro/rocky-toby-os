//! HID por el **protocolo boot**: interpretes de informe y traduccion de usos
//! a codigos de barrido del juego 1.
//!
//! Aqui solo hay logica pura: no toca puertos, ni memoria de DMA, ni colas del
//! kernel. Se puede probar en el anfitrion (`cargo test`), que es lo que pide
//! el plan para todo lo que decide que tecla se ha pulsado.
//!
//! ## Por que se traduce a codigos de barrido
//!
//! rocky ya tiene una sola capa de traduccion de teclas: la distribucion
//! española, las teclas muertas y el CP437 viven en el teclado PS/2 del kernel
//! (`input.rs`), que trabaja en el juego de barrido 1. Un teclado USB no manda
//! codigos de barrido: manda **usos HID** (una tabla de posiciones fisicas).
//! Para no tener dos distribuciones que se desincronicen, aqui se pasa de uso
//! HID a la **posicion** del juego 1, y el resto del camino es el mismo que el
//! del PS/2.
//!
//! Referencia: "HID Usage Tables", seccion 10 (Keyboard/Keypad), y la matriz de
//! traduccion de Linux (`drivers/hid/hid-input.c`, `hid_keyboard`), que es la
//! misma que usa QEMU para su teclado USB emulado.

pub mod boot;
pub mod descriptor;
pub mod informe;
pub mod teclado;
