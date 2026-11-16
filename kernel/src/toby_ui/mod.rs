//! Toby UI: lenguaje declarativo para interfaces gráficas de Rocky OS.
//!
//! Este módulo implementa el lenguaje de aplicaciones `.toby`: declara
//! estructura, distribucion, apariencia, recursos y referencias a acciones
//! registradas. No ejecuta codigo arbitrario (lo interpreta el anfitrion
//! [`host`](crate::toby_ui::host) sobre el servidor de video de rocky).
//!
//! Fases:
//!   * Fase 2 (hecha): parser XML-like con límites + modelo tipado + validación.
//!   * Fase 3 (hecha): layout.rs (row/column/stack/box, spacing/padding/align).
//!   * Fase 4 (hecha): resource.rs + theme.rs (nine-slice, paleta AQUA).
//!   * Fase 5 (hecha): renderer.rs (trait Renderer + CanvasRenderer).
//!   * Fase 6 (hecha): event.rs (ratón/teclado/foco/hover/pressed/focused).
//!   * Fase 7 (hecha): action.rs (registro de acciones seguro).
//!   * Fase 8 (hecha): app.rs + demo.toby (aplicación completa de punta a punta).
//!   * Fase 9.1 (hecha): manifest.rs (`manifest.xml` de una app `.app`).
//!   * Fase 10 (hecha): controles de estado (check, radio, circular,
//!     segmented, popup, field, slider, slider-gota) + `valores.rs`.
//!   * Fase 10.5 (hecha): texto UTF-8 en los valores (etiquetas, títulos,
//!     items y campos con acentos): el parser descodifica UTF-8 y
//!     `crate::texto` traduce cada carácter a su glifo CP437 de la fuente,
//!     con los 23 acentos que compone `bios::COMPONER`.
//!
//! Flujo de datos:
//!
//! ```text
//! texto .toby --> parser::analizar --> Document (crudo)
//!                                 --> validation::validar --> ValidatedDocument (tipado)
//!                                                            --> layout::calcular --> Disposicion
//!                                                            --> renderer::pintar --> framebuffer
//!                                                            <-- event::sondear / event::Interaccion
//!                                                            --> action::Registro (sólo nombres)
//!
//! `app::App` encadena todo lo anterior y es lo que usa un anfitrión; `demo`
//! es la primera aplicación completa escrita en el lenguaje.
//! ```
//!
//! El parser no conoce la semántica y el validador no conoce la sintaxis:
//! añadir un componente sólo toca `component.rs` (más el renderer en su caso).

// Módulo en construcción (fases 2-8): todavía no hay ningún consumidor
// externo, así que se calla el aviso de "no usado".
#![allow(dead_code)]

pub mod action;
pub mod app;
pub mod component;
pub mod demo;
pub mod document;
pub mod event;
pub mod host;
pub mod kit;
pub mod layout;
pub mod manifest;
pub mod paquete;
pub mod parser;
pub mod renderer;
pub mod resource;
pub mod theme;
pub mod validation;
pub mod valores;

// El decodificador PNG es una dependencia externa del kernel (`png`); se
// expone con un alias para que `resource.rs` no dependa de su nombre exacto
// ni de si vive en la raíz del crate.
pub(crate) use png as png_lib;

// Re-exportes de conveniencia. Se marcan con allow porque el módulo todavía
// no se usa desde fuera del propio módulo (el anfitrión llega con la app).
#[allow(unused_imports)]
pub use action::{ErrorAccion, Manejador, Peticion, Registro, Resultado};
#[allow(unused_imports)]
pub use app::{App, ErrorApp, Paso};
#[allow(unused_imports)]
pub use component::{buscar, clave_valida, ComponentDef, PropDef, PropType, COMPONENTES};
#[allow(unused_imports)]
pub use demo::aplicacion;
#[allow(unused_imports)]
pub use document::{Document, Nodo, PropValue, Sintaxis, ValidatedDocument};
#[allow(unused_imports)]
pub use event::{sondear, Boton, Cambios, Entrada, Evento, Interaccion, Sesion, Tecla};
#[allow(unused_imports)]
pub use layout::{calcular, calcular_en, Colocado, Disposicion, Rect, Restricciones};
#[allow(unused_imports)]
pub use manifest::{cargar as cargar_manifiesto, ManifestError, Manifiesto};
#[allow(unused_imports)]
pub use parser::{analizar, analizar_bytes, analizar_xml, ParserError};
#[allow(unused_imports)]
pub use renderer::{pintar, pintar_con_estado, CanvasRenderer, Renderer};
#[allow(unused_imports)]
pub use resource::{autorizada, autorizada_doc, cargar_png, leer_doc, ErrorResource, Imagen};
#[allow(unused_imports)]
pub use theme::{acento_de, texto_de, Estado, NineSlice, Tema, TEMA_POR_DEFECTO};
#[allow(unused_imports)]
pub use toby_appkit::animation::{Animation, Easing, Effect, TransitionConfig};
#[allow(unused_imports)]
pub use validation::{validar, ValidationError, ValidationResult};
