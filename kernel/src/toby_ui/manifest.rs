//! Manifiesto de una aplicación Toby AppKit (`manifest.xml`).
//!
//! Una app empaquetada vive en una **carpeta `<nombre>.app`** que el sistema
//! reconoce como unidad. Dentro, `manifest.xml` declara su identidad y qué
//! recursos usa, siempre con el mismo espíritu que Toby UI: **sólo describe**.
//! No hay código, ni variables, ni expresiones, y el vocabulario es cerrado
//! (lo que no está en la tabla se rechaza con línea y motivo).
//!
//! Estructura del paquete:
//!
//! ```text
//! <app nombre="juegos" version="1.0">
//!   <identidad titulo="Juegos" icono="img/icono.png"/>
//!   <ui principal="ui/principal.toby" tema="aqua">
//!     <vista nombre="inicio" archivo="ui/principal.toby"/>
//!     <vista nombre="ajustes" archivo="ui/ajustes.toby"/>
//!   </ui>
//!   <codigo lenguaje="rust" fuente="code/src/main.rs" binario="code/app.elf"/>
//!   <capacidades>
//!     <capacidad nombre="ventana"/>
//!     <capacidad nombre="datos" ruta="datos"/>
//!   </capacidades>
//!   <inicio clase="sistema" automatico="true" prioridad="100"/>
//! </app>
//! ```
//!
//! El manifiesto **no** abre ficheros ni dibuja: sólo se lee y se valida. Quien
//! lo consume (el menú de inicio, el cargador de apps) llega después.
//!
//! Reutiliza el léxico del parser XML de Toby UI (`parser::analizar_xml`): el
//! manifiesto no tiene su propio analizador, sólo su propio esquema.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::document::Sintaxis;
use super::parser;

/// Extensión de una carpeta de aplicación.
pub const EXTENSION: &str = "app";
/// Nombre del fichero de manifiesto dentro del bundle.
pub const FICHERO: &str = "manifest.xml";
/// Tamaño máximo del manifiesto, en bytes (es un fichero pequeño).
pub const MAX_BYTES: usize = 8 * 1024;
/// Longitud máxima del nombre de la app (`[a-z_][a-z0-9_-]*`).
pub const MAX_NOMBRE: usize = 64;
/// Longitud máxima de una ruta de recurso dentro del bundle.
pub const MAX_RUTA: usize = 128;
/// Longitud máxima de la cadena de versión.
pub const MAX_VERSION: usize = 16;
/// Tema por defecto si el manifiesto no pide otro.
pub const TEMA_POR_DEFECTO: &str = "aqua";

/// Temas que el manifiesto puede pedir (lista cerrada).
pub const TEMAS: &[&str] = &["aqua"];

/// Capacidades que una app puede declarar (lista cerrada).
///
/// * `ventana` — puede abrir ventanas en el escritorio.
/// * `datos`   — puede leer y escribir su carpeta de datos (con `ruta`).
/// * `sonido`  — puede usar el altavoz interno.
pub const CAPACIDADES: &[&str] = &["ventana", "datos", "sonido"];

/// Clase de paquete. Las apps de sistema se instalan bajo `/system/apps`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClaseApp {
    Sistema,
    #[default]
    Usuario,
}

/// Política declarada de inicio automático.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inicio {
    pub clase: ClaseApp,
    pub automatico: bool,
    pub prioridad: i16,
}

/// Identidad visible de la app (lo que muestra el menú de inicio).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identidad {
    /// Título legible, p.ej. "Juegos".
    pub titulo: String,
    /// Ruta del icono dentro del bundle (PNG), si lo hay.
    pub icono: Option<String>,
}

/// La interfaz declarativa de la app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ui {
    /// Tipo de interfaz declarada por la app.
    pub modo: ModoUi,
    /// Documento Toby UI principal, dentro del bundle.
    /// Está vacío cuando `modo` es [`ModoUi::Texto`].
    pub principal: String,
    /// Tema del kit que se usa para pintarla.
    pub tema: String,
    /// Vistas Toby disponibles para las distintas pantallas/funciones.
    pub vistas: Vec<Vista>,
}

/// Interfaz de una app: Toby gráfica o consola de texto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModoUi {
    #[default]
    Toby,
    Texto,
}

/// Una pantalla o función declarativa de la aplicación.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vista {
    /// Identificador estable de la vista.
    pub nombre: String,
    /// Ruta relativa al bundle del documento Toby.
    pub archivo: String,
}

/// El código Rust opcional de la app; el ELF se genera fuera del kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Codigo {
    /// Lenguaje de implementación (por ahora sólo Rust).
    pub lenguaje: String,
    /// Fuente Rust principal dentro de `code/`, si está incluida.
    pub fuente: Option<String>,
    /// Binario ELF64 compilado para Rocky, si ya se construyó.
    pub binario: Option<String>,
}

/// Una capacidad declarada por la app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capacidad {
    /// Nombre de la capacidad (uno de [`CAPACIDADES`]).
    pub nombre: String,
    /// Carpeta de datos asociada (sólo para `datos`).
    pub ruta: Option<String>,
}

/// Un manifiesto ya validado y tipado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifiesto {
    /// Nombre de la app (`[a-z_][a-z0-9_-]*`).
    pub nombre: String,
    /// Versión declarada (p.ej. "1.0").
    pub version: String,
    /// Identidad visible.
    pub identidad: Identidad,
    /// Interfaz declarativa.
    pub ui: Ui,
    /// Código nativo, si la app lo trae.
    pub codigo: Option<Codigo>,
    /// Capacidades declaradas (puede estar vacía).
    pub capacidades: Vec<Capacidad>,
    /// Clasificación y preferencia de arranque. Si se omite, es app de usuario
    /// y no se inicia automáticamente.
    pub inicio: Inicio,
}

/// Error de manifiesto, con posición aproximada (línea del nodo).
#[derive(Clone, Debug)]
pub struct ManifestError {
    /// Línea (1-based) del elemento culpable.
    pub linea: usize,
    /// Descripción breve del problema.
    pub detalle: String,
}

impl core::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "línea {}: {}", self.linea, self.detalle)
    }
}

/// Resultado de leer un manifiesto: el tipado, o la lista de errores.
pub type ManifestResult = Result<Manifiesto, Vec<ManifestError>>;

/// Lee un manifiesto desde bytes: primero la forma (XML estricto) y luego el
/// esquema (vocabulario cerrado).
pub fn cargar(bytes: &[u8]) -> ManifestResult {
    let raiz = match parser::analizar_xml(bytes) {
        Ok(r) => r,
        Err(e) => {
            return Err(alloc::vec![ManifestError {
                linea: e.linea,
                detalle: e.detalle.to_string(),
            }])
        }
    };
    validar(&raiz)
}

/// Igual que [`cargar`], desde texto.
pub fn cargar_str(texto: &str) -> ManifestResult {
    cargar(texto.as_bytes())
}

/// Valida la raíz cruda contra el esquema del manifiesto.
pub fn validar(raiz: &Sintaxis) -> ManifestResult {
    let mut errores: Vec<ManifestError> = Vec::new();
    let mut ok = true;

    // `<rapp>` sólo se acepta como nombre antiguo de la raíz para poder leer
    // manifests previos; todo paquete nuevo se crea con `<app>`.
    if raiz.etiqueta != "app" && raiz.etiqueta != "rapp" {
        errores.push(ManifestError {
            linea: raiz.linea,
            detalle: format!(
                "la raíz del manifiesto debe ser <app>, no <{}>",
                raiz.etiqueta
            ),
        });
        return Err(errores);
    }

    // --- Atributos de la raíz: sólo `nombre` y `version`. ---
    let mut nombre = String::new();
    let mut version = String::new();
    for (k, v) in &raiz.atributos {
        match k.as_str() {
            "nombre" => {
                if nombre_valido(v) {
                    nombre = v.clone();
                } else {
                    errores.push(ManifestError {
                        linea: raiz.linea,
                        detalle: format!(
                            "nombre de app inválido: '{}' (se espera [a-z_][a-z0-9_-]* de hasta {} bytes)",
                            v, MAX_NOMBRE
                        ),
                    });
                    ok = false;
                }
            }
            "version" => {
                if version_valida(v) {
                    version = v.clone();
                } else {
                    errores.push(ManifestError {
                        linea: raiz.linea,
                        detalle: format!(
                            "versión inválida: '{}' (se espera algo como 1.0, hasta {} bytes)",
                            v, MAX_VERSION
                        ),
                    });
                    ok = false;
                }
            }
            otro => {
                errores.push(ManifestError {
                    linea: raiz.linea,
                    detalle: format!("atributo '{}' no permitido en <app>", otro),
                });
                ok = false;
            }
        }
    }
    if !raiz.atributos.iter().any(|(k, _)| k == "nombre") {
        errores.push(falta(raiz.linea, "nombre", "app"));
        ok = false;
    }
    if !raiz.atributos.iter().any(|(k, _)| k == "version") {
        errores.push(falta(raiz.linea, "version", "app"));
        ok = false;
    }

    // --- Hijos: identidad(1) ui(1) codigo(0/1) capacidades(0/1) inicio(0/1). ---
    let mut identidad: Option<Identidad> = None;
    let mut ui: Option<Ui> = None;
    let mut codigo: Option<Codigo> = None;
    let mut capacidades: Vec<Capacidad> = Vec::new();
    let mut inicio = Inicio::default();
    let mut visto_identidad = false;
    let mut visto_ui = false;
    let mut visto_codigo = false;
    let mut visto_capacidades = false;
    let mut visto_inicio = false;

    for h in &raiz.hijos {
        match h.etiqueta.as_str() {
            "identidad" => {
                if visto_identidad {
                    errores.push(duplicado(h.linea, "identidad"));
                    ok = false;
                }
                visto_identidad = true;
                if let Some(x) = validar_identidad(h, &mut errores) {
                    identidad = Some(x);
                } else {
                    ok = false;
                }
            }
            "ui" => {
                if visto_ui {
                    errores.push(duplicado(h.linea, "ui"));
                    ok = false;
                }
                visto_ui = true;
                if let Some(x) = validar_ui(h, &mut errores) {
                    ui = Some(x);
                } else {
                    ok = false;
                }
            }
            "codigo" => {
                if visto_codigo {
                    errores.push(duplicado(h.linea, "codigo"));
                    ok = false;
                }
                visto_codigo = true;
                if let Some(x) = validar_codigo(h, &mut errores) {
                    codigo = Some(x);
                } else {
                    ok = false;
                }
            }
            "capacidades" => {
                if visto_capacidades {
                    errores.push(duplicado(h.linea, "capacidades"));
                    ok = false;
                }
                visto_capacidades = true;
                let antes = errores.len();
                for c in validar_capacidades(h, &mut errores) {
                    capacidades.push(c);
                }
                if errores.len() > antes {
                    ok = false;
                }
            }
            "inicio" => {
                if visto_inicio {
                    errores.push(duplicado(h.linea, "inicio"));
                    ok = false;
                }
                visto_inicio = true;
                if let Some(x) = validar_inicio(h, &mut errores) {
                    inicio = x;
                } else {
                    ok = false;
                }
            }
            otro => {
                errores.push(ManifestError {
                    linea: h.linea,
                    detalle: format!("elemento desconocido en <app>: <{}>", otro),
                });
                ok = false;
            }
        }
    }

    if !visto_identidad {
        errores.push(ManifestError {
            linea: raiz.linea,
            detalle: String::from("falta el elemento <identidad> en <app>"),
        });
        ok = false;
    }
    if !visto_ui {
        errores.push(ManifestError {
            linea: raiz.linea,
            detalle: String::from("falta el elemento <ui> en <app>"),
        });
        ok = false;
    }

    if ok {
        Ok(Manifiesto {
            nombre,
            version,
            identidad: identidad.expect("validado"),
            ui: ui.expect("validado"),
            codigo,
            capacidades,
            inicio,
        })
    } else {
        Err(errores)
    }
}

/// `<inicio clase="usuario|sistema" automatico="true|false" prioridad="N"/>`.
fn validar_inicio(n: &Sintaxis, errores: &mut Vec<ManifestError>) -> Option<Inicio> {
    let mut ok = no_admite_hijos(n, errores);
    let mut inicio = Inicio::default();
    for (k, v) in &n.atributos {
        match k.as_str() {
            "clase" => match v.as_str() {
                "usuario" => inicio.clase = ClaseApp::Usuario,
                "sistema" => inicio.clase = ClaseApp::Sistema,
                _ => {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: String::from("'clase' en <inicio> debe ser 'usuario' o 'sistema'"),
                    });
                    ok = false;
                }
            },
            "automatico" => match v.as_str() {
                "true" => inicio.automatico = true,
                "false" => inicio.automatico = false,
                _ => {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: String::from("'automatico' en <inicio> debe ser 'true' o 'false'"),
                    });
                    ok = false;
                }
            },
            "prioridad" => match v.parse::<i16>() {
                Ok(p) if (-1000..=1000).contains(&p) => inicio.prioridad = p,
                _ => {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: String::from(
                            "'prioridad' en <inicio> debe ser un entero de -1000 a 1000",
                        ),
                    });
                    ok = false;
                }
            },
            otro => {
                errores.push(ManifestError {
                    linea: n.linea,
                    detalle: format!("atributo '{}' no permitido en <inicio>", otro),
                });
                ok = false;
            }
        }
    }
    if ok {
        Some(inicio)
    } else {
        None
    }
}

/// `<identidad titulo="..." icono="..."/>`.
fn validar_identidad(n: &Sintaxis, errores: &mut Vec<ManifestError>) -> Option<Identidad> {
    let mut ok = no_admite_hijos(n, errores);
    let mut titulo: Option<String> = None;
    let mut icono: Option<String> = None;

    for (k, v) in &n.atributos {
        match k.as_str() {
            "titulo" => {
                if v.is_empty() {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: String::from("'titulo' no puede estar vacío"),
                    });
                    ok = false;
                } else {
                    titulo = Some(v.clone());
                }
            }
            "icono" => {
                if ruta_valida(v, Some(".png")) {
                    icono = Some(v.clone());
                } else {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!(
                            "icono inválido: '{}' (ruta relativa al bundle, terminada en .png)",
                            v
                        ),
                    });
                    ok = false;
                }
            }
            otro => {
                errores.push(ManifestError {
                    linea: n.linea,
                    detalle: format!("atributo '{}' no permitido en <identidad>", otro),
                });
                ok = false;
            }
        }
    }
    if titulo.is_none() {
        errores.push(falta(n.linea, "titulo", "identidad"));
        ok = false;
    }

    if ok {
        Some(Identidad {
            titulo: titulo.expect("validado"),
            icono,
        })
    } else {
        None
    }
}

/// `<ui principal="..." tema="..."><vista .../></ui>` o `<ui modo="texto"/>`.
fn validar_ui(n: &Sintaxis, errores: &mut Vec<ManifestError>) -> Option<Ui> {
    let mut ok = true;
    let mut modo = ModoUi::Toby;
    let mut principal: Option<String> = None;
    let mut tema = String::from(TEMA_POR_DEFECTO);
    let mut vistas: Vec<Vista> = Vec::new();

    for (k, v) in &n.atributos {
        match k.as_str() {
            "modo" => match v.as_str() {
                "toby" => modo = ModoUi::Toby,
                "texto" => modo = ModoUi::Texto,
                _ => {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!(
                            "modo de interfaz no permitido: '{}' (se admite toby o texto)",
                            v
                        ),
                    });
                    ok = false;
                }
            },
            "principal" => {
                if ruta_valida(v, Some(".toby")) {
                    principal = Some(v.clone());
                } else {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!(
                            "documento principal inválido: '{}' (ruta relativa al bundle, terminada en .toby)",
                            v
                        ),
                    });
                    ok = false;
                }
            }
            "tema" => {
                if TEMAS.contains(&v.as_str()) {
                    tema = v.clone();
                } else {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!(
                            "tema no permitido: '{}'; se admite: {}",
                            v,
                            listar(TEMAS)
                        ),
                    });
                    ok = false;
                }
            }
            otro => {
                errores.push(ManifestError {
                    linea: n.linea,
                    detalle: format!("atributo '{}' no permitido en <ui>", otro),
                });
                ok = false;
            }
        }
    }

    for h in &n.hijos {
        if h.etiqueta != "vista" {
            errores.push(ManifestError {
                linea: h.linea,
                detalle: format!("elemento desconocido en <ui>: <{}>", h.etiqueta),
            });
            ok = false;
            continue;
        }
        let mut nombre: Option<String> = None;
        let mut archivo: Option<String> = None;
        let mut vista_ok = no_admite_hijos(h, errores);
        for (k, v) in &h.atributos {
            match k.as_str() {
                "nombre" if nombre_valido(v) => nombre = Some(v.clone()),
                "nombre" => {
                    errores.push(ManifestError {
                        linea: h.linea,
                        detalle: format!("nombre de vista inválido: '{}'", v),
                    });
                    vista_ok = false;
                }
                "archivo" if ruta_valida(v, Some(".toby")) => archivo = Some(v.clone()),
                "archivo" => {
                    errores.push(ManifestError {
                        linea: h.linea,
                        detalle: format!("ruta de vista inválida: '{}' (se espera un .toby)", v),
                    });
                    vista_ok = false;
                }
                otro => {
                    errores.push(ManifestError {
                        linea: h.linea,
                        detalle: format!("atributo '{}' no permitido en <vista>", otro),
                    });
                    vista_ok = false;
                }
            }
        }
        if nombre.is_none() {
            errores.push(falta(h.linea, "nombre", "vista"));
            vista_ok = false;
        }
        if archivo.is_none() {
            errores.push(falta(h.linea, "archivo", "vista"));
            vista_ok = false;
        }
        if vista_ok {
            let nombre = nombre.expect("validado");
            let archivo = archivo.expect("validado");
            if vistas.iter().any(|v| v.nombre == nombre) {
                errores.push(duplicado(h.linea, "vista.nombre"));
                ok = false;
            } else if vistas.iter().any(|v| v.archivo == archivo) {
                errores.push(duplicado(h.linea, "vista.archivo"));
                ok = false;
            } else {
                vistas.push(Vista { nombre, archivo });
            }
        } else {
            ok = false;
        }
    }
    match modo {
        ModoUi::Toby => {
            if principal.is_none() {
                errores.push(falta(n.linea, "principal", "ui"));
                ok = false;
            }
            if let Some(principal) = principal.as_ref() {
                if vistas.is_empty() {
                    vistas.push(Vista {
                        nombre: String::from("principal"),
                        archivo: principal.clone(),
                    });
                } else if !vistas.iter().any(|v| v.archivo == *principal) {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: String::from(
                            "la vista principal debe aparecer en la lista de <vista>",
                        ),
                    });
                    ok = false;
                }
            }
        }
        ModoUi::Texto => {
            if principal.is_some() || !vistas.is_empty() {
                errores.push(ManifestError {
                    linea: n.linea,
                    detalle: String::from(
                        "<ui modo=\"texto\"> no admite principal ni elementos <vista>",
                    ),
                });
                ok = false;
            }
        }
    }

    if ok {
        Some(Ui {
            modo,
            principal: principal.unwrap_or_default(),
            tema,
            vistas,
        })
    } else {
        None
    }
}

/// `<codigo lenguaje="rust" fuente="..." binario="..."/>` (opcional).
fn validar_codigo(n: &Sintaxis, errores: &mut Vec<ManifestError>) -> Option<Codigo> {
    let mut ok = no_admite_hijos(n, errores);
    let mut lenguaje: Option<String> = None;
    let mut fuente: Option<String> = None;
    let mut binario: Option<String> = None;

    for (k, v) in &n.atributos {
        match k.as_str() {
            "lenguaje" if v == "rust" => lenguaje = Some(v.clone()),
            "lenguaje" => {
                errores.push(ManifestError {
                    linea: n.linea,
                    detalle: format!("lenguaje nativo no admitido: '{}' (se admite rust)", v),
                });
                ok = false;
            }
            "fuente" => {
                if v.starts_with("code/") && ruta_valida(v, Some(".rs")) {
                    fuente = Some(v.clone());
                } else {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!("fuente Rust inválida: '{}' (ruta code/*.rs)", v),
                    });
                    ok = false;
                }
            }
            "binario" => {
                if v.starts_with("code/") && ruta_valida(v, Some(".elf")) {
                    binario = Some(v.clone());
                } else {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!("binario inválido: '{}' (ruta code/*.elf)", v),
                    });
                    ok = false;
                }
            }
            otro => {
                errores.push(ManifestError {
                    linea: n.linea,
                    detalle: format!("atributo '{}' no permitido en <codigo>", otro),
                });
                ok = false;
            }
        }
    }
    if lenguaje.is_none() {
        errores.push(falta(n.linea, "lenguaje", "codigo"));
        ok = false;
    }
    if fuente.is_none() && binario.is_none() {
        errores.push(ManifestError {
            linea: n.linea,
            detalle: String::from("<codigo> requiere una fuente Rust o un ELF compilado"),
        });
        ok = false;
    }

    if ok {
        Some(Codigo {
            lenguaje: lenguaje.expect("validado"),
            fuente,
            binario,
        })
    } else {
        None
    }
}

/// `<capacidades><capacidad .../></capacidades>`.
fn validar_capacidades(n: &Sintaxis, errores: &mut Vec<ManifestError>) -> Vec<Capacidad> {
    let mut out: Vec<Capacidad> = Vec::new();

    if !n.atributos.is_empty() {
        errores.push(ManifestError {
            linea: n.linea,
            detalle: String::from("<capacidades> no admite atributos"),
        });
    }

    for h in &n.hijos {
        if h.etiqueta != "capacidad" {
            errores.push(ManifestError {
                linea: h.linea,
                detalle: format!("<capacidades> sólo admite <capacidad>, no <{}>", h.etiqueta),
            });
            continue;
        }
        if let Some(c) = validar_capacidad(h, errores) {
            if out.iter().any(|x| x.nombre == c.nombre) {
                errores.push(ManifestError {
                    linea: h.linea,
                    detalle: format!("capacidad repetida: '{}'", c.nombre),
                });
            } else {
                out.push(c);
            }
        }
    }
    out
}

/// `<capacidad nombre="..." ruta="..."/>`.
fn validar_capacidad(n: &Sintaxis, errores: &mut Vec<ManifestError>) -> Option<Capacidad> {
    let mut ok = no_admite_hijos(n, errores);
    let mut nombre: Option<String> = None;
    let mut ruta: Option<String> = None;

    for (k, v) in &n.atributos {
        match k.as_str() {
            "nombre" => {
                if CAPACIDADES.contains(&v.as_str()) {
                    nombre = Some(v.clone());
                } else {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!(
                            "capacidad no permitida: '{}'; se admite: {}",
                            v,
                            listar(CAPACIDADES)
                        ),
                    });
                    ok = false;
                }
            }
            "ruta" => {
                if ruta_valida(v, None) {
                    ruta = Some(v.clone());
                } else {
                    errores.push(ManifestError {
                        linea: n.linea,
                        detalle: format!("ruta de datos inválida: '{}'", v),
                    });
                    ok = false;
                }
            }
            otro => {
                errores.push(ManifestError {
                    linea: n.linea,
                    detalle: format!("atributo '{}' no permitido en <capacidad>", otro),
                });
                ok = false;
            }
        }
    }

    if nombre.is_none() {
        errores.push(falta(n.linea, "nombre", "capacidad"));
        ok = false;
    }
    // Sólo `datos` lleva carpeta; cualquier otra capacidad con `ruta` es un error
    // (declararla donde no aplica suele ser un despiste, no una intención).
    if ruta.is_some() && nombre.as_deref() != Some("datos") {
        errores.push(ManifestError {
            linea: n.linea,
            detalle: String::from("sólo la capacidad 'datos' admite 'ruta'"),
        });
        ok = false;
    }

    if ok {
        Some(Capacidad {
            nombre: nombre.expect("validado"),
            ruta,
        })
    } else {
        None
    }
}

/// Un elemento hoja (sin hijos); deja el error si los tiene.
fn no_admite_hijos(n: &Sintaxis, errores: &mut Vec<ManifestError>) -> bool {
    if n.hijos.is_empty() {
        true
    } else {
        errores.push(ManifestError {
            linea: n.linea,
            detalle: format!("<{}> no admite hijos", n.etiqueta),
        });
        false
    }
}

/// Nombre de app: `[a-z_][a-z0-9_-]*`, hasta [`MAX_NOMBRE`] bytes.
fn nombre_valido(v: &str) -> bool {
    let b = v.as_bytes();
    if b.is_empty() || b.len() > MAX_NOMBRE {
        return false;
    }
    if !(b[0].is_ascii_lowercase() || b[0] == b'_') {
        return false;
    }
    b[1..]
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

/// Versión: uno o más números separados por puntos (`1`, `1.0`, `1.2.3`).
fn version_valida(v: &str) -> bool {
    if v.is_empty() || v.len() > MAX_VERSION {
        return false;
    }
    v.split('.')
        .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

/// Ruta relativa al bundle, sin escapar de él.
///
/// Reglas: no vacía, cabe en [`MAX_RUTA`], no empieza por `/`, no usa `\`, sin
/// segmentos vacíos, `.` ni `..`, y (si se pide) termina en la extensión dada.
fn ruta_valida(v: &str, ext: Option<&str>) -> bool {
    if v.is_empty() || v.len() > MAX_RUTA {
        return false;
    }
    if v.starts_with('/') || v.contains('\\') {
        return false;
    }
    for seg in v.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return false;
        }
    }
    match ext {
        Some(e) => v.ends_with(e),
        None => true,
    }
}

/// Lista los valores permitidos para un mensaje de error.
fn listar(valores: &[&str]) -> String {
    let mut s = String::new();
    for (i, v) in valores.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(v);
    }
    s
}

/// Error de atributo obligatorio que falta.
fn falta(linea: usize, attr: &str, etiqueta: &str) -> ManifestError {
    ManifestError {
        linea,
        detalle: format!("falta el atributo obligatorio '{}' en <{}>", attr, etiqueta),
    }
}

/// Error de elemento repetido.
fn duplicado(linea: usize, etiqueta: &str) -> ManifestError {
    ManifestError {
        linea,
        detalle: format!("elemento repetido: <{}>", etiqueta),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const VALIDO: &str = "<rapp nombre=\"juegos\" version=\"1.0\">\
         <identidad titulo=\"Juegos\" icono=\"img/icono.png\"/>\
         <ui principal=\"ui/principal.toby\"/>\
         <capacidades>\
           <capacidad nombre=\"ventana\"/>\
           <capacidad nombre=\"datos\" ruta=\"datos\"/>\
         </capacidades>\
       </rapp>";

    #[test]
    fn manifiesto_minimo_valido() {
        let m = cargar_str(VALIDO).expect("válido");
        assert_eq!(m.nombre, "juegos");
        assert_eq!(m.version, "1.0");
        assert_eq!(m.identidad.titulo, "Juegos");
        assert_eq!(m.identidad.icono.as_deref(), Some("img/icono.png"));
        assert_eq!(m.ui.principal, "ui/principal.toby");
        assert_eq!(m.ui.modo, ModoUi::Toby);
        assert_eq!(m.ui.tema, TEMA_POR_DEFECTO);
        assert!(m.codigo.is_none());
        assert_eq!(m.capacidades.len(), 2);
        assert_eq!(m.inicio.clase, ClaseApp::Usuario);
        assert!(!m.inicio.automatico);
        assert_eq!(m.inicio.prioridad, 0);
    }

    #[test]
    fn interfaz_solo_texto_no_exige_documento_grafico() {
        let manifiesto = cargar_str(
            "<app nombre=\"terminal\" version=\"1.0\"><identidad titulo=\"Terminal\"/><ui modo=\"texto\"/><codigo lenguaje=\"rust\" fuente=\"code/src/main.rs\"/></app>",
        )
        .expect("manifiesto de consola válido");
        assert_eq!(manifiesto.ui.modo, ModoUi::Texto);
        assert!(manifiesto.ui.principal.is_empty());
        assert!(manifiesto.ui.vistas.is_empty());
    }

    #[test]
    fn interfaz_texto_rechaza_vistas_graficas() {
        let error = cargar_str(
            "<app nombre=\"terminal\" version=\"1.0\"><identidad titulo=\"Terminal\"/><ui modo=\"texto\"><vista nombre=\"principal\" archivo=\"main.toby\"/></ui></app>",
        )
        .expect_err("una app de texto no debe declarar vistas Toby");
        assert!(error.iter().any(|e| e.detalle.contains("modo=\"texto\"")));
    }

    #[test]
    fn comentarios_y_espacios_permitidos() {
        let texto = "<!-- app -->\n<rapp nombre=\"reloj\" version=\"2\">\n\
             <identidad titulo=\"Reloj\"/>\n<ui principal=\"ui/reloj.toby\"/>\n</rapp>";
        let m = cargar_str(texto).expect("válido");
        assert_eq!(m.nombre, "reloj");
    }

    #[test]
    fn raiz_incorrecta() {
        let e =
            cargar_str("<bundle nombre=\"x\" version=\"1\"></bundle>").expect_err("debe fallar");
        assert!(e[0].detalle.contains("app"), "{}", e[0].detalle);
    }

    #[test]
    fn falta_nombre_o_version() {
        let e = cargar_str(
            "<rapp version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/></rapp>",
        )
        .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("nombre")));
        let e = cargar_str(
            "<rapp nombre=\"x\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/></rapp>",
        )
        .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("version")));
    }

    #[test]
    fn nombre_y_version_mal_formados() {
        assert!(cargar_str("<rapp nombre=\"Juegos\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/></rapp>").is_err());
        assert!(cargar_str("<rapp nombre=\"x\" version=\"uno\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/></rapp>").is_err());
        // Uno y varios segmentos son válidos.
        assert!(cargar_str("<rapp nombre=\"a\" version=\"1.2.3\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/></rapp>").is_ok());
    }

    #[test]
    fn elemento_desconocido() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/><extra/></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("extra")));
    }

    #[test]
    fn atributo_desconocido() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\" color=\"rojo\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("color")));
    }

    #[test]
    fn faltan_identidad_o_ui() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"></rapp>").expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("identidad")));
        assert!(e.iter().any(|x| x.detalle.contains("ui")));
    }

    #[test]
    fn duplicados_rechazados() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/><ui principal=\"b.toby\"/></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("repetido")));
    }

    #[test]
    fn rutas_que_escapan_del_bundle() {
        assert!(cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"../x.toby\"/></rapp>").is_err());
        assert!(cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"/etc/x.toby\"/></rapp>").is_err());
        assert!(cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"ui\\\\x.toby\"/></rapp>").is_err());
        assert!(cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"ui/./x.toby\"/></rapp>").is_err());
    }

    #[test]
    fn ui_exige_extension_toby() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"ui/principal.txt\"/></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("principal")));
    }

    #[test]
    fn tema_cerrado() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\" tema=\"oscuro\"/></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("tema")));
        assert!(cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\" tema=\"aqua\"/></rapp>").is_ok());
    }

    #[test]
    fn capacidades_cerradas() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/><capacidades><capacidad nombre=\"red\"/></capacidades></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("red")));
    }

    #[test]
    fn ruta_solo_para_datos() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/><capacidades><capacidad nombre=\"ventana\" ruta=\"v\"/></capacidades></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("datos")));
    }

    #[test]
    fn codigo_opcional() {
        let m = cargar_str("<app nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/><codigo lenguaje=\"rust\" fuente=\"code/main.rs\" binario=\"code/x.elf\"/></app>")
            .expect("válido");
        let codigo = m.codigo.expect("código opcional presente");
        assert_eq!(codigo.lenguaje, "rust");
        assert_eq!(codigo.fuente.as_deref(), Some("code/main.rs"));
        assert_eq!(codigo.binario.as_deref(), Some("code/x.elf"));
    }

    #[test]
    fn clase_y_prioridad_de_inicio_se_validan() {
        let m = cargar_str(
            "<app nombre=\"gestor\" version=\"1.0\"><identidad titulo=\"Gestor\"/>\
             <ui principal=\"main.toby\"/><inicio clase=\"sistema\" automatico=\"true\" prioridad=\"90\"/></app>",
        )
        .expect("política válida");
        assert_eq!(m.inicio.clase, ClaseApp::Sistema);
        assert!(m.inicio.automatico);
        assert_eq!(m.inicio.prioridad, 90);
        assert!(cargar_str(
            "<app nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/><inicio automatico=\"yes\"/></app>"
        )
        .is_err());
        assert!(cargar_str(
            "<app nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/><inicio prioridad=\"5000\"/></app>"
        )
        .is_err());
    }

    #[test]
    fn app_con_varias_vistas() {
        let m = cargar_str(
            "<app nombre=\"editor\" version=\"1.0\"><identidad titulo=\"Editor\"/>\
             <ui principal=\"ui/inicio.toby\"><vista nombre=\"inicio\" archivo=\"ui/inicio.toby\"/>\
             <vista nombre=\"ajustes\" archivo=\"ui/ajustes.toby\"/></ui></app>",
        )
        .expect("paquete válido");
        assert_eq!(m.ui.vistas.len(), 2);
        assert_eq!(m.ui.vistas[1].nombre, "ajustes");
    }

    #[test]
    fn fuente_rust_escapando_code_rechazada() {
        let e = cargar_str(
            "<app nombre=\"x\" version=\"1\"><identidad titulo=\"x\"/><ui principal=\"a.toby\"/>\
             <codigo lenguaje=\"rust\" fuente=\"code/../evil.rs\"/></app>",
        )
        .expect_err("no debe salir del bundle");
        assert!(e.iter().any(|x| x.detalle.contains("fuente Rust")));
    }

    #[test]
    fn hojas_sin_hijos() {
        let e = cargar_str("<rapp nombre=\"x\" version=\"1\"><identidad titulo=\"x\"><algo/></identidad><ui principal=\"a.toby\"/></rapp>")
            .expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("no admite hijos")));
    }
}
