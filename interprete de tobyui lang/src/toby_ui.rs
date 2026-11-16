//! El motor Toby UI del kernel, incluido fichero a fichero con `#[path]`.
//!
//! No se copia ni se reescribe nada: son los MISMOS archivos que compila
//! `rocky` en `kernel/src/toby_ui/`. El anfitrion solo aporta lo que en el
//! kernel vive en la raiz del crate (`bios`, `gfx`, `fs`, `sync`, `mouse`,
//! `input`), que aqui son shims.
//!
//! `host.rs` (orden `toby` del shell) y `mod.rs` no se incluyen: el primero
//! esta atado al kernel (VESA, PS/2, Kira) y el segundo solo es la lista de
//! modulos que se reproduce aqui.

#![allow(dead_code)]

// Alias equivalente al de `mod.rs` del kernel: `resource.rs` usa
// `super::png_lib`. `png` es el descodificador real de rocky, no un stub.
pub use ::png as png_lib;

#[path = "../../kernel/src/toby_ui/action.rs"]
pub mod action;

#[path = "../../kernel/src/toby_ui/app.rs"]
pub mod app;

#[path = "../../kernel/src/toby_ui/component.rs"]
pub mod component;

#[path = "../../kernel/src/toby_ui/demo.rs"]
pub mod demo;

#[path = "../../kernel/src/toby_ui/document.rs"]
pub mod document;

#[path = "../../kernel/src/toby_ui/event.rs"]
pub mod event;

#[path = "../../kernel/src/toby_ui/kit.rs"]
pub mod kit;

#[path = "../../kernel/src/toby_ui/layout.rs"]
pub mod layout;

#[path = "../../kernel/src/toby_ui/manifest.rs"]
pub mod manifest;

#[path = "../../kernel/src/toby_ui/parser.rs"]
pub mod parser;

#[path = "../../kernel/src/toby_ui/renderer.rs"]
pub mod renderer;

#[path = "../../kernel/src/toby_ui/resource.rs"]
pub mod resource;

#[path = "../../kernel/src/toby_ui/theme.rs"]
pub mod theme;

#[path = "../../kernel/src/toby_ui/validation.rs"]
pub mod validation;

#[path = "../../kernel/src/toby_ui/valores.rs"]
pub mod valores;

#[allow(unused_imports)]
pub use toby_appkit::animation::{Animation, Easing, Effect, TransitionConfig};
