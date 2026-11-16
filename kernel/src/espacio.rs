//! El espacio del usuario y el del sistema: las rutas canonicas de rocky.
//!
//! rocky es **monousuario** (`whoami` responde `root`, no hay `uid`), asi que
//! su espacio del usuario cuelga entero de `/home`. Este modulo es la unica
//! fuente de verdad de esas rutas: el sembrado del disco, el espacio del usuario
//! y el futuro cargador de apps las toman de aqui en vez de repetir cadenas.
//!
//! ```text
//! /
//! |-- bin/               programas del sistema (ELF que trae el anfitrion)
//! |-- system/apps/       aplicaciones del sistema (`.app`)
//! |-- usr/share/fonts/   fuentes TTF/OTF y caches bitmap para Kira
//! |-- usr/share/backgrounds/ fondos del sistema (recursos AQUA)
//! |-- usr/share/tobyui/  recursos del tema del sistema (PNG y .toby)
//! `-- home/              ESPACIO DEL USUARIO
//!     |-- apps/          aplicaciones <nombre>.app del usuario
//!     |-- papelera/      archivos apartados de forma temporal
//!     `-- escritorio/    carpeta de trabajo del usuario
//! ```
//!
//! Reglas:
//!   * Las apps viven en [`APPS`] (usuario) o [`APPS_SISTEMA`] (sistema), no en
//!     el escritorio ni en la raiz. Cada una incluye su `manifest.xml`.
//!   * [`RECURSOS`] es del **sistema** (el tema): no es de ninguna app. Los
//!     recursos propios de una app salen de su propio `img/`.

#![allow(dead_code)]

/// Programas del sistema, que instala el anfitrion (no el usuario).
pub const BIN: &str = "/bin";

/// Raiz del espacio del usuario.
pub const USUARIO: &str = "/home";

/// Carpeta de trabajo del usuario (sus documentos).
pub const ESCRITORIO: &str = "/home/escritorio";

/// Carpeta donde viven las apps del usuario. Cada app es `<nombre>.app/`.
pub const APPS: &str = "/home/apps";

/// Papelera del usuario; su contenido determina el icono vacío/lleno del Dock.
pub const PAPELERA: &str = "/home/papelera";

/// Carpeta reservada para aplicaciones instaladas como parte del sistema.
pub const APPS_SISTEMA: &str = "/system/apps";

/// Recursos del tema del sistema (PNG de piezas y documentos base).
pub const RECURSOS: &str = "/usr/share/tobyui";

/// Carpeta de fuentes instaladas en el sistema.
pub const FUENTES: &str = "/usr/share/fonts";

/// Imágenes de fondo instaladas desde los recursos AQUA.
pub const FONDOS: &str = "/usr/share/backgrounds";

/// Cache bitmap de SF Pro Regular usada por defecto en Kira.
pub const FUENTE_KIRA: &str = "/usr/share/fonts/sfprodisplayregular.rkf";

/// Cache SF Pro Display Medium suavizada para la barra y el texto proporcional de Kira.
/// Se reutiliza la cache existente para no duplicar su consumo de memoria en el kernel.
pub const FUENTE_KIRA_SUAVE: &str = "/usr/share/fonts/sfprodisplaymedium.rka";

/// Cache SF Pro Display Regular suavizada para los datos de «Acerca de».
pub const FUENTE_ACERCA: &str = "/usr/share/fonts/sfprodisplayregular.rka";

/// Máscaras Times a tamaño final para el título grande de «Acerca de».
pub const FUENTE_ACERCA_TITULO: &str = "/usr/share/fonts/about-title.rka";

/// Alternativa serif si SF Pro no está instalada o su cache está dañada.
pub const FUENTE_KIRA_ALTERNATIVA: &str = "/usr/share/fonts/times.rkf";

/// Caché Times antialias de alta resolución para el editor TextEdit.
pub const FUENTE_TEXTEDIT: &str = "/usr/share/fonts/times-textedit.rka";

/// Fuente monoespaciada Cascadia Code para la Terminal de Kira.
pub const FUENTE_TERMINAL: &str = "/usr/share/fonts/cascadiacode-variablefont_wght.rkf";

/// Cache antialias de Cascadia Code usada al pintar texto de Terminal.
pub const FUENTE_TERMINAL_SUAVE: &str = "/usr/share/fonts/cascadiacode-variablefont_wght.rka";

/// Documento de la demo de Toby UI sembrado en el disco, dentro de [`RECURSOS`].
pub const RECURSOS_DEMO: &str = "/usr/share/tobyui/demo.toby";
