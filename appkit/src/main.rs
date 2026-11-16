//! **Toby AppKit** para Windows.
//!
//! Un editor visual del lenguaje Toby UI: a la izquierda el panel con los tipos
//! de boton del kit AQUA y con los ocho controles del lenguaje, en medio un
//! lienzo en blanco donde se colocan **en la posicion que se quiera**, y abajo
//! los botones *Generar .toby* y *Testear* (abre el intérprete real).
//!
//! Se trabaja con dos arrastres, los dos con el boton izquierdo pulsado:
//!
//!   * **colocar** un control: se pincha en una entrada del panel y, sin
//!     soltar, se lleva a la hoja (mientras el puntero esta dentro de la hoja
//!     el motor de verdad lo pinta, asi que se ve donde va a caer);
//!   * **mover** un control que ya estaba: se pincha encima y se arrastra.
//!
//! Lo importante de como esta hecho:
//!
//!   * La vista previa del lienzo la pinta **el motor real del lenguaje**
//!     (`App` -> parser -> validacion -> layout -> tema -> renderer), el mismo
//!     que compila el kernel. Cada vez que se toca el diseno se regenera el
//!     texto `.toby` y se vuelve a montar la `App`, asi que lo que se ve en el
//!     lienzo es exactamente lo que pintara el interprete: es WYSIWYG por
//!     construccion, no por parecerse.
//!   * El documento generado es un `<stack>` con las hojas en **posicion
//!     libre** (`x`/`y`), y usa `responsive="scale"` para que el motor adapte
//!     sus posiciones y tamanos cuando Kira cambia el tamano de la ventana.
//!   * No se copia NADA del motor: los ficheros de `kernel/src/toby_ui/` y los
//!     shims del interprete se incluyen con `#[path]`. Hay una unica fuente de
//!     verdad para el lenguaje.
//!   * *Testear* escribe el estado actual en un `.toby` temporal y lo abre en
//!     una ventana aparte del intérprete; al cerrarla, el temporal se limpia.
//!   * La ventana abre **maximizada** y F11 alterna a ventana normal, igual
//!     que en el interprete: la pantalla manda. La hoja del diseno respeta el
//!     tamano que declara el `<window>` del documento, asi que lo que se ve
//!     aqui es lo que vera el interprete a ese tamano.
//!
//! Aqui, como en el interprete, lo unico que se "ejecuta" de un documento son
//! los manejadores nativos que este programa registra.

#![allow(dead_code)]

extern crate alloc;
extern crate self as toby_appkit;

#[path = "../../kernel/src/libraries/toby_appkit/src/animation.rs"]
pub mod animation;
#[path = "../../kernel/src/libraries/toby_appkit/src/genie.rs"]
pub mod genie;

mod package;

#[path = "../../interprete de tobyui lang/src/host/bios.rs"]
mod bios;
#[path = "../../interprete de tobyui lang/src/host/fs.rs"]
mod fs;
#[path = "../../interprete de tobyui lang/src/host/fuente.rs"]
mod fuente;
#[path = "../../interprete de tobyui lang/src/host/fuente_sf.rs"]
mod fuente_sf;
#[path = "../../interprete de tobyui lang/src/host/gfx.rs"]
mod gfx;
#[path = "../../interprete de tobyui lang/src/host/input.rs"]
mod input;
#[path = "../../interprete de tobyui lang/src/host/mouse.rs"]
mod mouse;
#[path = "../../interprete de tobyui lang/src/host/pit.rs"]
mod pit;
#[path = "../../interprete de tobyui lang/src/host/png_escribir.rs"]
mod png_escribir;
#[path = "../../interprete de tobyui lang/src/host/sync.rs"]
mod sync;
#[path = "../../interprete de tobyui lang/src/host/tarea.rs"]
mod tarea;
#[path = "../../kernel/src/texto.rs"]
mod texto;
#[path = "../../interprete de tobyui lang/src/toby_ui.rs"]
mod toby_ui;
#[path = "../../interprete de tobyui lang/src/host/win32.rs"]
mod win32;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, Ordering};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use toby_ui::action::{Peticion, Registro, Resultado};
use toby_ui::app::App;
use toby_ui::event::Evento;
use toby_ui::layout::{self, Rect};
use toby_ui::renderer::{CanvasRenderer, Renderer};
use toby_ui::theme::{self, TEMA_POR_DEFECTO};

use win32::*;

// --- Medidas de la ventana del editor -------------------------------------

/// Ancho y alto de la ventana normal (la que vuelve F11 desde la maximizada).
const VENTANA_W: i32 = 1000;
const VENTANA_H: i32 = 660;
/// Ancho del panel lateral de componentes.
const PANEL_W: i32 = 200;
/// Alto de la barra de titulo y de la barra de acciones.
const BARRA_ALTA: i32 = 32;
const PIE_ALTA: i32 = 56;
/// Tamano de la hoja del diseno (el `<window>` del documento).
const HOJA_W: u32 = 480;
const HOJA_H: u32 = 300;
/// Alto de las filas del panel y los botones del editor.
const ALTO_BOTON: u32 = 32;

/// Colores propios del editor (los del motor se toman del tema).
const FONDO_ESCRITORIO: u32 = 0xDDE1E5;
const TEXTO_BARRA: u32 = 0xFFFFFF;
const TEXTO_HINT: u32 = 0xC7D8EE;
const SELECCION: u32 = 0xFF8A00;

/// Tamano actual del area de cliente de la ventana.
///
/// Lo fija `WM_SIZE` (llega al abrir y cada vez que cambia el tamano, F11
/// incluido). El valor inicial es el de la ventana normal, que es el que
/// usan la autoprueba y los modos sin ventana.
static VENTANA_ANCHO: AtomicI32 = AtomicI32::new(VENTANA_W);
static VENTANA_ALTO: AtomicI32 = AtomicI32::new(VENTANA_H);

/// Tamano actual del area de cliente.
fn tam_ventana() -> (i32, i32) {
    (
        VENTANA_ANCHO.load(Ordering::Relaxed),
        VENTANA_ALTO.load(Ordering::Relaxed),
    )
}

// --- El diseno que se esta montando ---------------------------------------

/// Un control colocado en el lienzo. Es la unica informacion que se guarda: el
/// texto `.toby` se regenera siempre desde aqui.
#[derive(Clone)]
struct Colocado {
    /// `id` del documento (`c1`, `c2`, ...).
    id: String,
    /// Etiqueta del elemento en el documento (`button`, `check`, ...).
    tipo: &'static str,
    /// Variante del kit (`VARIANTES`).
    variante: String,
    /// Texto principal del control, segun su tipo: `title` (boton, circular),
    /// `label` (check, radio), `items` (segmented, popup) o `placeholder`
    /// (field). Los sliders no llevan texto, llevan `valor`.
    texto: String,
    /// Valor del slider (`value`, 0..=100); los demas tipos no lo usan.
    valor: i32,
    /// Al teclear en un slider, indica si los dígitos reemplazan o continúan.
    valor_editando: bool,
    /// Alto en píxeles del control (tamaño CSS de AQUA).
    alto: u32,
    /// Posicion libre, en coordenadas de la hoja.
    x: i32,
    y: i32,
    /// `enabled` del documento.
    habilitado: bool,
    /// Accion declarada, si la tiene.
    accion: Option<String>,
}

/// El diseno completo: la hoja y sus controles.
struct Diseno {
    nombre: String,
    titulo: String,
    ancho: u32,
    alto: u32,
    controles: Vec<Colocado>,
    /// Contador para los `id` automaticos.
    siguiente: usize,
    /// Indice del control seleccionado.
    seleccion: Option<usize>,
}

/// Convierte el texto de un control en su `name`, como en HTML.
///
/// Reglas: minusculas, letras y digitos ASCII; todo lo demas (espacios,
/// acentos, signos) se vuelve un guion, y los guiones seguidos se juntan.
/// Si no queda nada (un slider, por ejemplo), se usa el tipo del control.
///
/// ```text
/// "Aceptar"      -> "aceptar"
/// "Guardar todo" -> "guardar-todo"
/// "Mediano"      -> "mediano"
/// "!"            -> "control"
/// ```
fn name_de(texto: &str, tipo: &str) -> String {
    let mut s = String::new();
    let mut guion = false;
    for c in texto.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            s.push(c);
            guion = false;
        } else if !guion && !s.is_empty() {
            s.push('-');
            guion = true;
        }
    }
    while s.ends_with('-') {
        s.pop();
    }
    if s.is_empty()
        || !s
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
    {
        String::from(tipo)
    } else {
        s
    }
}

/// Acciones que puede declarar un control del diseno, en orden de ciclo.
///
/// Son las que registra el interprete (`demo.pulsar`, `window.cerrar`); el
/// documento solo las NOMBRA.
static ACCIONES: &[&str] = &["demo.pulsar", "window.cerrar"];

impl Diseno {
    /// Diseno vacio: un lienzo en blanco.
    fn nuevo() -> Diseno {
        Diseno {
            nombre: String::from("diseno"),
            titulo: String::from("Diseno"),
            ancho: HOJA_W,
            alto: HOJA_H,
            controles: Vec::new(),
            siguiente: 1,
            seleccion: None,
        }
    }

    /// Anexa un control del panel y devuelve su indice.
    ///
    /// `texto` vacio deja el que trae el tipo por defecto (en los sliders se
    /// ignora: ellos llevan `valor`, que empieza en 0 y se teclea con digitos).
    fn anexar(
        &mut self,
        p: &'static TipoPanel,
        texto: &str,
        x: i32,
        y: i32,
        accion: Option<&str>,
    ) -> usize {
        let id = format!("c{}", self.siguiente);
        self.siguiente += 1;
        self.controles.push(Colocado {
            id,
            tipo: p.tipo,
            variante: String::from(p.variante),
            texto: if texto.is_empty() {
                String::from(p.texto)
            } else {
                String::from(texto)
            },
            valor: if p.tipo == "slider" || p.tipo == "slider-gota" {
                50
            } else {
                0
            },
            valor_editando: false,
            alto: layout::alto_css(p.tipo),
            x,
            y,
            habilitado: p.habilitado,
            accion: accion.map(String::from),
        });
        self.controles.len() - 1
    }

    /// Los `name` de todo el documento: la semilla de la app mas el de cada
    /// control, unico cada uno (los repetidos se numeran, como en HTML).
    ///
    /// Es la lista que escribe `documento()` y la que muestra el pie del
    /// editor: una sola fuente de verdad para el nombre de cada elemento.
    fn nombres(&self) -> Vec<String> {
        let mut nombres: Vec<String> = vec![self.nombre.clone()];
        for b in &self.controles {
            let base = name_de(&b.texto, b.tipo);
            let mut nombre = base.clone();
            let mut n = 2;
            while nombres.contains(&nombre) {
                nombre = format!("{}-{}", base, n);
                n += 1;
            }
            nombres.push(nombre);
        }
        nombres
    }

    /// El documento `.toby`: esto es lo que se genera.
    ///
    /// Las hojas van dentro de un `<stack>` con `x`/`y`, que es la posicion
    /// libre del lenguaje. Cada tipo escribe sus propias propiedades, con las
    /// reglas que el lenguaje exige (`title` obligatorio en `circular`,
    /// `items` en `segmented` y `popup`, `value/min/max` en los sliders).
    fn documento(&self) -> String {
        let mut s = String::new();
        s.push_str("<!-- Toby AppKit: diseno generado automaticamente. -->\n");
        s.push_str("<toby-app name=\"");
        s.push_str(&escapar_atributo_xml(&self.nombre));
        s.push_str("\">\n");
        s.push_str("  <window title=\"");
        s.push_str(&escapar_atributo_xml(&self.titulo));
        s.push_str(&format!(
            "\" width=\"{}\" height=\"{}\" align=\"stretch\">\n",
            self.ancho, self.alto
        ));
        s.push_str(&format!(
            "    <stack width=\"{}\" height=\"{}\" grow=\"1\" responsive=\"scale\">\n",
            self.ancho, self.alto
        ));
        // Los `name` deben ser unicos en el documento (lo exige el lenguaje);
        // es la misma lista que muestra el pie del editor.
        let nombres = self.nombres();
        for (i, b) in self.controles.iter().enumerate() {
            let texto_xml = escapar_atributo_xml(&b.texto);
            s.push_str("      <");
            s.push_str(b.tipo);
            s.push_str(&format!(" id=\"{}\"", b.id));
            s.push_str(&format!(" name=\"{}\"", nombres[i + 1]));
            // El campo no usa variantes: sus piezas son las blancas del kit.
            if b.tipo != "field" && !b.variante.is_empty() {
                s.push_str(&format!(" variant=\"{}\"", b.variante));
            }
            s.push_str(&format!(" height=\"{}\"", b.alto));
            if !b.habilitado {
                s.push_str(" enabled=\"false\"");
            }
            match b.tipo {
                // El lenguaje requiere title incluso si la etiqueta está vacía.
                "button" => s.push_str(&format!(" title=\"{}\"", texto_xml)),
                "circular" => s.push_str(&format!(" title=\"{}\"", texto_xml)),
                "check" => s.push_str(&format!(" label=\"{}\"", texto_xml)),
                "radio" => {
                    // Todas las bolas del diseno van en el mismo grupo, asi
                    // que se excluyen entre si (es lo que espera el motor).
                    s.push_str(&format!(" label=\"{}\" group=\"g1\"", texto_xml));
                }
                "segmented" | "popup" => s.push_str(&format!(" items=\"{}\"", texto_xml)),
                "field" => s.push_str(&format!(" placeholder=\"{}\"", texto_xml)),
                "slider" | "slider-gota" => {
                    s.push_str(&format!(" value=\"{}\" min=\"0\" max=\"100\"", b.valor));
                }
                _ => {}
            }
            if let Some(a) = &b.accion {
                s.push_str(&format!(" action=\"{}\"", a));
            }
            s.push_str(&format!(" x=\"{}\" y=\"{}\"/>\n", b.x, b.y));
        }
        s.push_str("    </stack>\n");
        s.push_str("  </window>\n");
        s.push_str("</toby-app>\n");
        s
    }
}

/// Escapa texto editable para insertarlo en un atributo XML. El parser Toby
/// decodifica estas entidades y conserva el valor original del control.
fn escapar_atributo_xml(texto: &str) -> String {
    let mut salida = String::with_capacity(texto.len());
    for caracter in texto.chars() {
        match caracter {
            '&' => salida.push_str("&amp;"),
            '<' => salida.push_str("&lt;"),
            '>' => salida.push_str("&gt;"),
            '"' => salida.push_str("&quot;"),
            '\'' => salida.push_str("&apos;"),
            otro => salida.push(otro),
        }
    }
    salida
}

// --- Panel lateral --------------------------------------------------------

/// Una entrada del panel lateral: algo que se puede arrastrar a la hoja.
struct TipoPanel {
    /// Etiqueta del elemento en el documento (`button`, `check`, ...).
    tipo: &'static str,
    /// Nombre que se muestra en el panel.
    etiqueta: &'static str,
    variante: &'static str,
    /// Texto por defecto (`title`/`label`/`items`/`placeholder`).
    texto: &'static str,
    habilitado: bool,
}

/// Los tipos de boton del kit AQUA, mas el apagado.
static PALETA: &[TipoPanel] = &[
    TipoPanel {
        tipo: "button",
        variante: "blue",
        etiqueta: "Azul",
        texto: "Boton",
        habilitado: true,
    },
    TipoPanel {
        tipo: "button",
        variante: "green",
        etiqueta: "Verde",
        texto: "Boton",
        habilitado: true,
    },
    TipoPanel {
        tipo: "button",
        variante: "red",
        etiqueta: "Rojo",
        texto: "Boton",
        habilitado: true,
    },
    TipoPanel {
        tipo: "button",
        variante: "orange",
        etiqueta: "Naranja",
        texto: "Boton",
        habilitado: true,
    },
    TipoPanel {
        tipo: "button",
        variante: "yellow",
        etiqueta: "Amarillo",
        texto: "Boton",
        habilitado: true,
    },
    TipoPanel {
        tipo: "button",
        variante: "magenta",
        etiqueta: "Magenta",
        texto: "Boton",
        habilitado: true,
    },
    TipoPanel {
        tipo: "button",
        variante: "white",
        etiqueta: "Blanco",
        texto: "Boton",
        habilitado: true,
    },
    TipoPanel {
        tipo: "button",
        variante: "blue",
        etiqueta: "Apagado",
        texto: "Boton",
        habilitado: false,
    },
];

/// Los ocho controles del lenguaje (F10), con su valor por defecto.
static CONTROLES: &[TipoPanel] = &[
    TipoPanel {
        tipo: "check",
        variante: "blue",
        etiqueta: "Check",
        texto: "Marcar",
        habilitado: true,
    },
    TipoPanel {
        tipo: "radio",
        variante: "blue",
        etiqueta: "Radio",
        texto: "Opcion",
        habilitado: true,
    },
    // El circular de AQUA es gris por defecto; sólo se colorea al declararlo.
    TipoPanel {
        tipo: "circular",
        variante: "",
        etiqueta: "Circular",
        texto: "C",
        habilitado: true,
    },
    TipoPanel {
        tipo: "segmented",
        variante: "blue",
        etiqueta: "Segmented",
        texto: "Uno|Dos|Tres",
        habilitado: true,
    },
    TipoPanel {
        tipo: "popup",
        variante: "blue",
        etiqueta: "Popup",
        texto: "Uno|Dos",
        habilitado: true,
    },
    TipoPanel {
        tipo: "field",
        variante: "blue",
        etiqueta: "Text field",
        texto: "Texto",
        habilitado: true,
    },
    TipoPanel {
        tipo: "slider",
        variante: "blue",
        etiqueta: "Slider",
        texto: "",
        habilitado: true,
    },
    TipoPanel {
        tipo: "slider-gota",
        variante: "blue",
        etiqueta: "Slider gota",
        texto: "",
        habilitado: true,
    },
];

/// La entrada `i` del panel: primero los botones, despues los controles.
fn entrada_paleta(i: usize) -> &'static TipoPanel {
    if i < PALETA.len() {
        &PALETA[i]
    } else {
        &CONTROLES[i - PALETA.len()]
    }
}

/// Cuantas entradas tiene el panel entero.
fn entradas_panel() -> usize {
    PALETA.len() + CONTROLES.len()
}

// --- Geometria del editor -------------------------------------------------

/// Rectangulo de la entrada `i` del panel lateral.
///
/// El grupo de los controles empieza mas abajo, dejando sitio a su rotulo.
fn rect_paleta(i: usize) -> Rect {
    let y = BARRA_ALTA + 34 + if i >= PALETA.len() { 20 } else { 0 };
    Rect {
        x: 12,
        y: y + (i as i32) * 32,
        w: (PANEL_W - 24) as u32,
        h: ALTO_BOTON,
    }
}

/// Zona de trabajo (todo menos el panel y las barras).
fn rect_area() -> Rect {
    let (w, h) = tam_ventana();
    Rect {
        x: PANEL_W,
        y: BARRA_ALTA,
        w: (w - PANEL_W).max(0) as u32,
        h: (h - BARRA_ALTA - PIE_ALTA).max(0) as u32,
    }
}

/// Rectangulos de los botones de la barra de acciones.
fn rect_generar() -> Rect {
    Rect {
        x: PANEL_W + 16,
        y: tam_ventana().1 - PIE_ALTA + 8,
        w: 136,
        h: ALTO_BOTON,
    }
}
fn rect_testear() -> Rect {
    let g = rect_generar();
    Rect {
        x: g.der() + 10,
        y: g.y,
        w: 92,
        h: ALTO_BOTON,
    }
}
fn rect_accion() -> Rect {
    let t = rect_testear();
    Rect {
        x: t.der() + 10,
        y: t.y,
        w: 208,
        h: ALTO_BOTON,
    }
}
fn rect_vaciar() -> Rect {
    let a = rect_accion();
    Rect {
        x: a.der() + 12,
        y: a.y,
        w: 80,
        h: ALTO_BOTON,
    }
}

/// Alturas presentes en las muestras CSS de AQUA y en `AQUA/index.html`.
const ALTOS_AQUA: &[u32] = &[18, 24, 26, 32, 36, 40, 48, 52, 62, 78, 96];

// --- Estado del editor ----------------------------------------------------

/// Contexto que ven los manejadores de acciones del documento.
///
/// La vista previa no despacha clics (en el editor, pinchar un boton lo
/// selecciona), asi que este estado solo existe para que el registro de
/// acciones tenga donde apuntar.
#[derive(Default)]
struct Ctx {
    ultimo: String,
}

/// `demo.pulsar`: en el editor solo se anota.
fn accion_anotar(c: &mut Ctx, p: &Peticion) -> Resultado {
    c.ultimo = format!("{} ({})", p.clave, p.id.unwrap_or("-"));
    Resultado::Hecho
}

/// El arrastre en curso.
///
/// Los dos gestos del editor empiezan igual (boton izquierdo pulsado), asi que
/// se distinguen por donde empezo el gesto: en el panel se *trae* un tipo
/// nuevo, encima de la hoja se *mueve* el boton que hay debajo.
#[derive(Clone, Copy)]
enum Arrastre {
    /// Se esta moviendo un boton que ya estaba en la hoja.
    Mover {
        /// Boton que se mueve.
        indice: usize,
        /// Desplazamiento del puntero dentro del boton, para que no salte.
        dx: i32,
        dy: i32,
    },
    /// Se esta trayendo un tipo del panel hacia la hoja.
    Traer {
        /// Entrada del panel de la que se viene.
        tipo: usize,
        /// Boton provisional ya colocado por el motor, si el puntero esta
        /// dentro de la hoja (fuera de ella se quita y el arrastre sigue).
        indice: Option<usize>,
    },
}

/// Todo el estado del editor.
struct Ensamblador {
    diseno: Diseno,
    /// Vista previa montada con el motor real (el documento regenerado).
    app: Option<App<Ctx>>,
    ctx: Ctx,
    /// Ultimo error del motor, si el documento generado no montase.
    error: Option<String>,
    /// Ultimo tipo cogido del panel (el que se resalta en naranja).
    tipo: Option<usize>,
    /// Arrastre en curso.
    arrastre: Option<Arrastre>,
    /// Lienzo de la ventana completa.
    lienzo: gfx::Canvas,
    /// Lienzo de la hoja (la vista previa).
    vista: gfx::Canvas,
    /// Miniaturas de los controles del panel, pintadas una vez con el motor.
    miniaturas: Vec<gfx::Canvas>,
    fuente: bios::Font,
    /// Mensaje de la barra inferior.
    estado: String,
    /// Fichero que escribe el boton *Generar*.
    salida: PathBuf,
}

impl Ensamblador {
    /// Editor recien abierto: lienzo en blanco.
    fn nuevo(salida: PathBuf) -> Ensamblador {
        let mut e = Ensamblador {
            diseno: Diseno::nuevo(),
            app: None,
            ctx: Ctx::default(),
            error: None,
            tipo: None,
            arrastre: None,
            lienzo: gfx::Canvas::nuevo(VENTANA_W, VENTANA_H),
            vista: gfx::Canvas::nuevo(HOJA_W as i32, HOJA_H as i32),
            miniaturas: CONTROLES.iter().map(miniatura_de).collect(),
            fuente: bios::get_font().expect("fuente incrustada"),
            estado: String::from("Arrastra un control del panel a la hoja."),
            salida,
        };
        e.reconstruir();
        e
    }

    /// Carga el diseno de ejemplo (lo usan los modos sin ventana).
    fn cargar_ejemplo(&mut self) {
        self.diseno = diseno_ejemplo();
        self.reconstruir();
        self.info();
    }

    /// Vuelve a montar la vista previa desde el diseno actual.
    ///
    /// Es el paso que garantiza el WYSIWYG: se genera el `.toby` y se le da al
    /// mismo motor que usara el interprete.
    fn reconstruir(&mut self) {
        // La hoja previa pinta a el tamano que declara el diseno.
        if self.vista.width != self.diseno.ancho as i32
            || self.vista.height != self.diseno.alto as i32
        {
            self.vista = gfx::Canvas::nuevo(self.diseno.ancho as i32, self.diseno.alto as i32);
        }
        let src = self.diseno.documento();
        let mut registro: Registro<Ctx> = Registro::nuevo();
        for clave in ACCIONES {
            if let Err(e) = registro.registrar(clave, accion_anotar) {
                self.error = Some(e.descripcion());
                return;
            }
        }
        match App::nueva(&src, self.diseno.ancho, self.diseno.alto, registro) {
            Ok(app) => {
                self.app = Some(app);
                self.error = None;
            }
            Err(e) => self.error = Some(e.resumen()),
        }
    }

    /// Rectangulo de la hoja del diseno, centrada en la zona de trabajo.
    ///
    /// Su tamano es el que declara el documento (`window width/height`): lo
    /// que se ve en la hoja es exactamente lo que pintara el interprete, que
    /// la pinta a ese tamano.
    fn rect_hoja(&self) -> Rect {
        let a = rect_area();
        let alto_barras = toby_ui::renderer::ALTO_BARRA_VENTANA.saturating_mul(2);
        let alto_total = self.diseno.alto.saturating_add(alto_barras);
        Rect {
            x: a.x + (a.w as i32 - self.diseno.ancho as i32) / 2,
            y: a.y
                + (a.h as i32 - alto_total as i32) / 2
                + toby_ui::renderer::ALTO_BARRA_VENTANA as i32,
            w: self.diseno.ancho,
            h: self.diseno.alto,
        }
    }

    /// Ventana de previsualizacion: contenido entre la cabecera y el pie.
    fn rect_ventana(&self) -> Rect {
        let hoja = self.rect_hoja();
        Rect {
            x: hoja.x,
            y: hoja.y - toby_ui::renderer::ALTO_BARRA_VENTANA as i32,
            w: hoja.w,
            h: hoja
                .h
                .saturating_add(toby_ui::renderer::ALTO_BARRA_VENTANA.saturating_mul(2)),
        }
    }

    /// Rectangulo de un boton del diseno, segun el layout del motor.
    fn rect_de(&self, i: usize) -> Option<Rect> {
        let app = self.app.as_ref()?;
        let b = self.diseno.controles.get(i)?;
        let idx = app.documento().indice_de(&b.id)?;
        Some(app.disposicion().rect(idx))
    }

    /// Tamano de un boton segun el layout del motor.
    ///
    /// No se inventa ningun tamano: si el motor no da rectangulo (documento que
    /// no monta) se usa el minimo del kit para poder seguir trabajando.
    fn tamano(&self, i: usize) -> (i32, i32) {
        match self.rect_de(i) {
            Some(r) => (r.w as i32, r.h as i32),
            None => (72, ALTO_BOTON as i32),
        }
    }

    /// Encaja la posicion de un boton dentro de la hoja.
    fn encajar(&mut self, i: usize) {
        let (w, h) = self.tamano(i);
        let b = &mut self.diseno.controles[i];
        b.x = b.x.clamp(0, (self.diseno.ancho as i32 - w).max(0));
        b.y = b.y.clamp(0, (self.diseno.alto as i32 - h).max(0));
    }

    /// Coloca un control del tipo `tipo` centrado en un punto de la hoja.
    ///
    /// Devuelve el indice del control nuevo.
    fn insertar(&mut self, tipo: usize, px: i32, py: i32) -> usize {
        let p = entrada_paleta(tipo);
        let i = self.diseno.anexar(p, "", px, py, None);
        self.diseno.seleccion = Some(i);
        self.reconstruir();
        // Con el tamano real ya calculado por el motor, se centra en el punto.
        if let Some(r) = self.rect_de(i) {
            let b = &mut self.diseno.controles[i];
            b.x = px - r.w as i32 / 2;
            b.y = py - r.h as i32 / 2;
        }
        self.encajar(i);
        self.reconstruir();
        self.info();
        i
    }

    /// Coloca (o recoloca) un boton provisional centrado bajo el puntero.
    ///
    /// Es lo que hace que el arrastre se vea: el boton que sigue al raton es un
    /// boton de verdad, pintado por el motor, no una silueta dibujada aparte.
    fn llevar(&mut self, i: usize, px: i32, py: i32) {
        let (w, h) = self.tamano(i);
        let b = &mut self.diseno.controles[i];
        b.x = px - w / 2;
        b.y = py - h / 2;
        self.encajar(i);
        self.reconstruir();
        self.info();
    }

    /// Quita el boton seleccionado.
    fn quitar_seleccionado(&mut self) {
        if let Some(i) = self.diseno.seleccion.take() {
            if i < self.diseno.controles.len() {
                let b = self.diseno.controles.remove(i);
                self.estado = format!("Quitado {}.", b.id);
                self.reconstruir();
            }
        }
    }

    /// Cicla la accion del boton seleccionado: ninguna -> demo.pulsar ->
    /// window.cerrar -> ninguna.
    fn ciclar_accion(&mut self) {
        let i = match self.diseno.seleccion {
            Some(i) => i,
            None => {
                self.estado = String::from("Selecciona un control para darle accion.");
                return;
            }
        };
        let nueva = match self.diseno.controles[i].accion.as_deref() {
            None => Some(String::from(ACCIONES[0])),
            Some(a) if a == ACCIONES[0] => Some(String::from(ACCIONES[1])),
            _ => None,
        };
        self.diseno.controles[i].accion = nueva;
        self.reconstruir();
        self.info();
    }

    /// Cambia la altura del control seleccionado por el siguiente tamaño AQUA.
    fn cambiar_alto(&mut self, adelante: bool) {
        let i = match self.diseno.seleccion {
            Some(i) => i,
            None => {
                self.estado = String::from("Selecciona un control para cambiar su tamaño.");
                return;
            }
        };
        let alto = self.diseno.controles[i].alto;
        let pos = ALTOS_AQUA.iter().position(|h| *h == alto).unwrap_or(0);
        let nuevo = if adelante {
            ALTOS_AQUA[(pos + 1) % ALTOS_AQUA.len()]
        } else {
            ALTOS_AQUA[(pos + ALTOS_AQUA.len() - 1) % ALTOS_AQUA.len()]
        };
        self.diseno.controles[i].alto = nuevo;
        self.reconstruir();
        self.encajar(i);
        self.reconstruir();
        self.info();
    }

    /// Rota por las variantes realmente definidas en los CSS de AQUA.
    fn ciclar_variante(&mut self) {
        const CIRCULARES: &[&str] = &[
            "", "blue", "green", "red", "orange", "yellow", "magenta", "white",
        ];
        let i = match self.diseno.seleccion {
            Some(i) => i,
            None => {
                self.estado = String::from("Selecciona un control para cambiar su variante.");
                return;
            }
        };
        let tipo = self.diseno.controles[i].tipo;
        let variantes = match tipo {
            "button" => toby_ui::component::VALORES_VARIANTE,
            "circular" => CIRCULARES,
            "field" => {
                self.estado = String::from("Text field no tiene variantes de color en AQUA.");
                return;
            }
            _ => toby_ui::component::VALORES_VARIANTE_COLOR,
        };
        let actual = self.diseno.controles[i].variante.as_str();
        let pos = variantes.iter().position(|v| *v == actual).unwrap_or(0);
        self.diseno.controles[i].variante = String::from(variantes[(pos + 1) % variantes.len()]);
        self.reconstruir();
        self.info();
    }

    /// Vacia el lienzo.
    fn vaciar(&mut self) {
        if self.diseno.controles.is_empty() {
            return;
        }
        let n = self.diseno.controles.len();
        self.diseno.controles.clear();
        self.diseno.seleccion = None;
        self.estado = format!("Lienzo vaciado ({} controles fuera).", n);
        self.reconstruir();
    }

    /// Escribe el `.toby` en el fichero de salida.
    fn generar(&mut self) {
        let src = self.diseno.documento();
        match std::fs::write(&self.salida, src.as_bytes()) {
            Ok(()) => {
                self.estado = format!(
                    "Generado {}  ({} controles)",
                    nombre_de(&self.salida),
                    self.diseno.controles.len()
                );
                println!("--- {} ---", self.salida.display());
                print!("{}", src);
                println!("--- {} bytes ---", src.len());
                println!("Para verlo:  {}", comando_interprete(&self.salida));
            }
            Err(e) => self.estado = format!("ERROR al escribir: {}", e),
        }
    }

    /// Abre el documento actual en una ventana independiente del intérprete
    /// real. El documento temporal se conserva hasta que se cierre la ventana.
    fn testear(&mut self) {
        if let Some(error) = &self.error {
            self.estado = format!("No se puede probar el documento: {}", error);
            return;
        }
        let interprete = match ruta_interprete() {
            Some(p) => p,
            None => {
                self.estado = String::from("No encuentro el intérprete; compílalo primero.");
                return;
            }
        };
        let temporal = match escribir_preview_temporal(&self.diseno.documento()) {
            Ok(p) => p,
            Err(e) => {
                self.estado = format!("No se pudo preparar la vista previa: {}", e);
                return;
            }
        };
        let mut comando = Command::new(&interprete);
        comando.arg(&temporal);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // El intérprete sólo necesita su ventana gráfica, sin otra consola.
            comando.creation_flags(0x0800_0000);
        }
        match comando.spawn() {
            Ok(mut hijo) => {
                let limpiar = temporal.clone();
                std::thread::spawn(move || {
                    let _ = hijo.wait();
                    let _ = std::fs::remove_file(limpiar);
                });
                self.estado = String::from("Vista previa abierta en el intérprete real.");
            }
            Err(e) => {
                let _ = std::fs::remove_file(temporal);
                self.estado = format!("No se pudo abrir el intérprete: {}", e);
            }
        }
    }

    /// Texto de la segunda linea del pie (que hay seleccionado).
    fn info(&mut self) {
        let i = match self.diseno.seleccion {
            Some(i) => i,
            None => {
                self.estado = String::from("Nada seleccionado.");
                return;
            }
        };
        let b = &self.diseno.controles[i];
        let texto = if b.tipo == "slider" || b.tipo == "slider-gota" {
            format!("valor={}", b.valor)
        } else {
            format!("'{}'", b.texto)
        };
        self.estado = format!(
            "sel {}  {}  {}  x={} y={} h={}  variante={}  {}{}",
            b.id,
            b.tipo,
            texto,
            b.x,
            b.y,
            b.alto,
            if b.tipo == "field" {
                "—"
            } else {
                &b.variante
            },
            if b.habilitado {
                "habilitado"
            } else {
                "apagado"
            },
            match &b.accion {
                Some(a) => format!("  accion={}", a),
                None => String::new(),
            }
        );
    }

    /// Rotulo del pie: el `name=` del control seleccionado.
    ///
    /// Es el mismo `name` que se escribe en el documento (la lista de
    /// `Diseno::nombres`), asi que lo que se ve aqui es lo que se declara.
    fn rotulo_nombre(&self) -> String {
        let i = match self.diseno.seleccion {
            Some(i) => i,
            None => return String::from("nada seleccionado"),
        };
        let nombres = self.diseno.nombres();
        match nombres.get(i + 1) {
            Some(n) => format!("name={}", n),
            None => String::from("nada seleccionado"),
        }
    }

    /// Un clic de raton en la ventana: empieza un arrastre.
    ///
    /// Colocar un boton es **arrastrar**: se coge el tipo en el panel y se suelta
    /// dentro de la hoja. Un clic suelto no coloca nada.
    fn clic(&mut self, x: i32, y: i32) {
        // Panel lateral: coger un tipo para traerlo a la hoja.
        for i in 0..entradas_panel() {
            if rect_paleta(i).contiene(x, y) {
                self.tipo = Some(i);
                self.arrastre = Some(Arrastre::Traer {
                    tipo: i,
                    indice: None,
                });
                self.estado = format!("Tipo {}: arrastralo a la hoja.", entrada_paleta(i).etiqueta);
                return;
            }
        }
        // Barra de acciones.
        if rect_generar().contiene(x, y) {
            self.generar();
            return;
        }
        if rect_testear().contiene(x, y) {
            self.testear();
            return;
        }
        if rect_accion().contiene(x, y) {
            self.ciclar_accion();
            return;
        }
        if rect_vaciar().contiene(x, y) {
            self.vaciar();
            return;
        }
        // Hoja: encima de un boton, seleccionarlo y empezar a moverlo.
        let hoja = self.rect_hoja();
        if !hoja.contiene(x, y) {
            return;
        }
        let (lx, ly) = (x - hoja.x, y - hoja.y);
        match self.boton_bajo(lx, ly) {
            Some(i) => {
                self.diseno.seleccion = Some(i);
                if let Some(r) = self.rect_de(i) {
                    self.arrastre = Some(Arrastre::Mover {
                        indice: i,
                        dx: lx - r.x,
                        dy: ly - r.y,
                    });
                }
                self.info();
            }
            None => {
                self.diseno.seleccion = None;
                self.estado = String::from("Pincha encima de un control para moverlo.");
            }
        }
    }

    /// El control colocado que esta bajo un punto de la hoja.
    ///
    /// Usa el acierto del propio motor (`Disposicion::indice_bajo`): el mismo
    /// que usara el interprete para saber que pieza se pulsa.
    fn boton_bajo(&self, lx: i32, ly: i32) -> Option<usize> {
        let app = self.app.as_ref()?;
        let i = app.disposicion().indice_bajo(lx, ly)?;
        let preorden = app.documento().preorden();
        let nodo = preorden.get(i)?;
        // Solo se mueven los controles colocados: `window` y `stack` (y
        // cualquier otra pieza sin `id` del diseno) no se tocan.
        let id = nodo.id.as_deref()?;
        self.diseno.controles.iter().position(|b| b.id == id)
    }

    /// Movimiento del raton: el arrastre en curso, o el resaltado de la vista
    /// previa (que se lo pide al motor, como hace el interprete).
    fn mover(&mut self, x: i32, y: i32) -> bool {
        let hoja = self.rect_hoja();
        match self.arrastre {
            Some(Arrastre::Mover { indice, dx, dy }) => {
                if let Some(b) = self.diseno.controles.get_mut(indice) {
                    b.x = x - hoja.x - dx;
                    b.y = y - hoja.y - dy;
                }
                self.encajar(indice);
                self.reconstruir();
                self.info();
                return true;
            }
            // Trayendo un tipo del panel: mientras el puntero esta dentro de la
            // hoja el boton esta puesto de verdad (lo pinta el motor, que es el
            // propio WYSIWYG); si sale de la hoja se quita y espera fuera.
            Some(Arrastre::Traer { tipo, indice }) => {
                let dentro = hoja.contiene(x, y);
                let (lx, ly) = (x - hoja.x, y - hoja.y);
                match (dentro, indice) {
                    (true, None) => {
                        let i = self.insertar(tipo, lx, ly);
                        self.arrastre = Some(Arrastre::Traer {
                            tipo,
                            indice: Some(i),
                        });
                    }
                    (true, Some(i)) => self.llevar(i, lx, ly),
                    (false, Some(i)) => {
                        // Siempre es el ultimo boton, asi que quitarlo no mueve
                        // los indices de los demas.
                        self.diseno.controles.remove(i);
                        self.diseno.seleccion = None;
                        self.arrastre = Some(Arrastre::Traer { tipo, indice: None });
                        self.reconstruir();
                    }
                    (false, None) => {}
                }
                return true;
            }
            None => {}
        }
        // Sin arrastre: se le pasa el puntero al motor (resaltado, como en el
        // interprete). No se despachan clics: aqui pinchar es seleccionar.
        if hoja.contiene(x, y) {
            if let Some(app) = &mut self.app {
                let ev = Evento::RatonMueve {
                    x: x - hoja.x,
                    y: y - hoja.y,
                    dx: 0,
                    dy: 0,
                };
                app.procesar(&ev, &mut self.ctx);
            }
        }
        false
    }

    /// Suelta el boton del raton: se acaba el arrastre.
    ///
    /// Lo que se suelta dentro de la hoja se queda; lo que se suelta fuera (o un
    /// clic que nunca llego a la hoja) no coloca nada.
    fn soltar(&mut self) {
        if let Some(Arrastre::Traer { tipo, indice }) = self.arrastre {
            match indice {
                Some(i) => {
                    self.diseno.seleccion = Some(i);
                    self.info();
                }
                None => {
                    self.estado = format!(
                        "Tipo {}: suelta dentro de la hoja.",
                        entrada_paleta(tipo).etiqueta
                    );
                }
            }
        }
        self.arrastre = None;
    }

    /// Un carácter tecleado: edita el texto del control seleccionado.
    ///
    /// Cada tipo guarda lo suyo: `title`, `label`, `items` o `placeholder`.
    /// En un slider los digitos fijan el `value` (0..=100).
    fn teclear(&mut self, ch: u32) {
        let i = match self.diseno.seleccion {
            Some(i) => i,
            None => return,
        };
        let tipo = self.diseno.controles[i].tipo;
        if tipo == "slider" || tipo == "slider-gota" {
            let b = &mut self.diseno.controles[i];
            match ch {
                0x08 => {
                    b.valor = (b.valor / 10).max(0);
                    b.valor_editando = true;
                }
                _ => match char::from_u32(ch).and_then(|c| c.to_digit(10)) {
                    Some(d) => {
                        b.valor = if b.valor_editando {
                            (b.valor * 10 + d as i32).clamp(0, 100)
                        } else {
                            d as i32
                        };
                        b.valor_editando = true;
                    }
                    None => return,
                },
            }
            self.reconstruir();
            self.info();
            return;
        }
        if ch == 0x08 {
            // Retroceso: borra el ultimo carácter.
            let mut t = self.diseno.controles[i].texto.clone();
            t.pop();
            self.diseno.controles[i].texto = t;
        } else if let Some(c) = char::from_u32(ch) {
            // Todo ASCII imprimible es válido; los reservados se escapan al
            // serializar el documento Toby.
            if c.is_ascii_graphic() || c == ' ' {
                self.diseno.controles[i].texto.push(c);
            } else {
                return;
            }
        } else {
            return;
        }
        self.reconstruir();
        self.encajar(i);
        self.reconstruir();
        self.info();
    }

    /// Teclas de edicion (flechas para afinar, Supr para quitar).
    fn tecla(&mut self, vk: WPARAM) {
        if vk == VK_DELETE {
            self.quitar_seleccionado();
            return;
        }
        if vk == VK_F2 {
            self.ciclar_variante();
            return;
        }
        if vk == VK_PRIOR || vk == VK_NEXT {
            self.cambiar_alto(vk == VK_PRIOR);
            return;
        }
        let (dx, dy) = match vk {
            VK_LEFT => (-1, 0),
            VK_RIGHT => (1, 0),
            VK_UP => (0, -1),
            VK_DOWN => (0, 1),
            _ => return,
        };
        if let Some(i) = self.diseno.seleccion {
            let b = &mut self.diseno.controles[i];
            b.x += dx;
            b.y += dy;
            self.encajar(i);
            self.reconstruir();
            self.info();
        }
    }

    /// Pinta la ventana completa en el lienzo.
    fn pintar(&mut self) {
        // 1) La vista previa la pinta el MOTOR sobre su propio lienzo.
        if let Some(app) = &self.app {
            if let Some(fuente_suave) = fuente_sf::sistema() {
                app.pintar_con_fuente_suave(&mut self.vista, Some(&self.fuente), &fuente_suave);
            } else {
                app.pintar(&mut self.vista, Some(&self.fuente));
            }
        }
        let seleccion = self.diseno.seleccion.and_then(|i| self.rect_de(i));
        let hoja = self.rect_hoja();
        let rotulo = self.rotulo_nombre();
        let (vw, vh) = tam_ventana();

        // 2) El chrome del editor, sobre el lienzo de la ventana. El renderer
        //    toma prestado `self.lienzo`, asi que vive en su propio bloque.
        {
            let estado = self.estado.clone();
            let error = self.error.clone();
            let salida = self.salida.clone();
            let tipo = self.tipo;
            let diseno = &self.diseno;
            let mut r = CanvasRenderer::nuevo(&mut self.lienzo, Some(&self.fuente));

            // Fondo y barras.
            r.limpiar(FONDO_ESCRITORIO);
            let barra = Rect {
                x: 0,
                y: 0,
                w: vw as u32,
                h: BARRA_ALTA as u32,
            };
            r.relleno(barra, TEMA_POR_DEFECTO.acentos.azul);
            r.texto(12, 12, "Toby AppKit", TEXTO_BARRA, 1);
            r.texto(
                232,
                12,
                "arrastra  ·  F2: color  ·  PgUp/PgDn: tamaño  ·  Testear: ventana real",
                TEXTO_HINT,
                1,
            );
            let pista = "F11: pantalla completa";
            r.texto(
                vw - gfx::text_width(pista, 1) - 12,
                12,
                pista,
                TEXTO_HINT,
                1,
            );
            let pie = Rect {
                x: 0,
                y: vh - PIE_ALTA,
                w: vw as u32,
                h: PIE_ALTA as u32,
            };
            r.relleno(pie, TEMA_POR_DEFECTO.paleta.panel);
            r.borde(pie, TEMA_POR_DEFECTO.paleta.borde);
            // El hueco del pie a la izquierda de los botones: ahi se ve el
            // `name` del control seleccionado (el mismo del documento).
            r.texto(12, vh - PIE_ALTA + 8, &rotulo, 0x1F2328, 1);

            // Panel lateral con los tipos de boton.
            let panel = Rect {
                x: 0,
                y: BARRA_ALTA,
                w: PANEL_W as u32,
                h: (vh - BARRA_ALTA - PIE_ALTA).max(0) as u32,
            };
            r.relleno(panel, TEMA_POR_DEFECTO.paleta.panel);
            r.borde(panel, TEMA_POR_DEFECTO.paleta.borde);
            r.texto(12, BARRA_ALTA + 12, "Tipos de boton", 0x1F2328, 1);
            for (i, t) in PALETA.iter().enumerate() {
                let rect = rect_paleta(i);
                if t.habilitado {
                    pintar_boton_editor(&mut r, rect, t.variante, t.etiqueta);
                } else {
                    // Igual que el motor: un boton `enabled="false"` no usa piel.
                    r.relleno(rect, TEMA_POR_DEFECTO.acentos.gris);
                    let ancho = gfx::text_width(t.etiqueta, 1);
                    r.texto(
                        rect.x + (rect.w as i32 - ancho) / 2,
                        rect.y + 8,
                        t.etiqueta,
                        theme::aclarar(TEMA_POR_DEFECTO.paleta.texto, 55),
                        1,
                    );
                }
                if tipo == Some(i) {
                    r.borde(
                        Rect {
                            x: rect.x - 2,
                            y: rect.y - 2,
                            w: rect.w + 4,
                            h: rect.h + 4,
                        },
                        SELECCION,
                    );
                }
            }
            // Los controles del lenguaje: sus miniaturas (pintadas por el
            // motor al abrir) se pegan despues, igual que la hoja.
            r.texto(
                12,
                rect_paleta(PALETA.len()).y - 16,
                "Controles",
                0x1F2328,
                1,
            );
            for i in 0..CONTROLES.len() {
                if tipo == Some(PALETA.len() + i) {
                    let rect = rect_paleta(PALETA.len() + i);
                    r.borde(
                        Rect {
                            x: rect.x - 2,
                            y: rect.y - 2,
                            w: rect.w + 4,
                            h: rect.h + 4,
                        },
                        SELECCION,
                    );
                }
            }

            // Barra de acciones.
            pintar_boton_editor(&mut r, rect_generar(), "blue", "Generar .toby");
            pintar_boton_editor(&mut r, rect_testear(), "green", "Testear");
            let etiqueta_accion = match diseno
                .seleccion
                .and_then(|i| diseno.controles[i].accion.clone())
            {
                Some(a) => format!("Accion: {}", a),
                None => String::from("Accion: ninguna"),
            };
            pintar_boton_editor(&mut r, rect_accion(), "orange", &etiqueta_accion);
            pintar_boton_editor(&mut r, rect_vaciar(), "red", "Vaciar");

            let v = rect_vaciar();
            r.texto(v.der() + 16, rect_generar().y + 8, &estado, 0x1F2328, 1);
            if let Some(e) = error {
                r.texto(PANEL_W + 16, vh - 22, &format!("motor: {}", e), 0xB01B1B, 1);
            } else {
                r.texto(
                    PANEL_W + 16,
                    vh - 22,
                    &format!("salida: {}", salida.display()),
                    0x5A6470,
                    1,
                );
            }
        }

        // 2b) Las miniaturas de los controles del panel, tal cual las pinto el
        //     motor: son la promesa de lo que se coloca.
        for i in 0..CONTROLES.len() {
            let rect = rect_paleta(PALETA.len() + i);
            blit(&mut self.lienzo, &self.miniaturas[i], rect.x, rect.y);
        }

        // 3) La hoja: la vista previa del motor, tal cual.
        blit(&mut self.lienzo, &self.vista, hoja.x, hoja.y);
        // Marco Kira compartido: barra AQUA, controles macOS y esquinas con el
        // mismo radio que el compositor. No se añade sombra.
        {
            let ventana = self.rect_ventana();
            let titulo = self.diseno.titulo.clone();
            let fuente = &self.fuente;
            let mut r = CanvasRenderer::nuevo(&mut self.lienzo, Some(fuente));
            toby_ui::renderer::pintar_cabecera_ventana(
                &mut r,
                ventana,
                FONDO_ESCRITORIO,
                TEMA_POR_DEFECTO.paleta.borde,
                &titulo,
            );
            toby_ui::renderer::pintar_pie_ventana(&mut r, ventana, TEMA_POR_DEFECTO.paleta.borde);
        }

        // 4) El marco de seleccion, encima de la hoja.
        if let Some(s) = seleccion {
            let marco = Rect {
                x: s.x + hoja.x - 2,
                y: s.y + hoja.y - 2,
                w: s.w + 4,
                h: s.h + 4,
            };
            marco_de(&self.lienzo, marco, SELECCION);
        }
    }
}

/// Pinta un boton del editor con la piel de una variante del kit.
fn pintar_boton_editor<R: Renderer>(r: &mut R, rect: Rect, variante: &str, texto: &str) {
    match toby_ui::kit::piel(variante) {
        Some(piel) => theme::pintar_piel(r, piel, rect),
        None => r.relleno(rect, theme::acento_de(&TEMA_POR_DEFECTO, variante)),
    }
    let ancho = gfx::text_width(texto, 1);
    r.texto(
        rect.x + (rect.w as i32 - ancho) / 2,
        rect.y + 8,
        texto,
        theme::texto_de(&TEMA_POR_DEFECTO, variante),
        1,
    );
}

/// Miniatura de un control del panel, pintada por el motor de verdad.
///
/// Es un documento `.toby` de una sola pieza dentro de una ventana del tamano
/// de la entrada del panel: lo que se ve en el panel es exactamente lo que se
/// va a colocar en la hoja.
fn miniatura_de(p: &'static TipoPanel) -> gfx::Canvas {
    const ANCHO: u32 = (PANEL_W - 24) as u32;
    const ALTO: u32 = ALTO_BOTON;
    // La miniatura muestra el alto CSS real, centrado en la fila del panel.
    let alto = layout::alto_css(p.tipo);
    let y = (ALTO as i32 - alto as i32) / 2;
    let mut pieza = format!("      <{} x=\"6\" y=\"{}\"", p.tipo, y);
    // El campo no usa variantes: sus piezas son las blancas del kit.
    if p.tipo != "field" && !p.variante.is_empty() {
        pieza.push_str(&format!(" variant=\"{}\"", p.variante));
    }
    pieza.push_str(&format!(" height=\"{}\"", alto));
    match p.tipo {
        "button" | "circular" => pieza.push_str(&format!(" title=\"{}\"", p.texto)),
        "check" | "radio" => pieza.push_str(&format!(" label=\"{}\"", p.texto)),
        "segmented" | "popup" => pieza.push_str(&format!(" items=\"{}\"", p.texto)),
        "field" => pieza.push_str(&format!(" placeholder=\"{}\"", p.texto)),
        "slider" | "slider-gota" => pieza.push_str(" value=\"50\" min=\"0\" max=\"100\""),
        _ => {}
    }
    pieza.push_str(" />\n");
    let src = format!(
        "<toby-app name=\"mini\">\n  <window title=\"mini\" width=\"{ANCHO}\" height=\"{ALTO}\">\n    <stack width=\"{ANCHO}\" height=\"{ALTO}\">\n{pieza}    </stack>\n  </window>\n</toby-app>\n"
    );
    let mut lienzo = gfx::Canvas::nuevo(ANCHO as i32, ALTO as i32);
    let mut registro: Registro<Ctx> = Registro::nuevo();
    for clave in ACCIONES {
        let _ = registro.registrar(clave, accion_anotar);
    }
    if let Ok(app) = App::nueva(&src, ANCHO, ALTO, registro) {
        let fuente = bios::get_font().expect("fuente incrustada");
        if let Some(fuente_suave) = fuente_sf::sistema() {
            app.pintar_con_fuente_suave(&mut lienzo, Some(&fuente), &fuente_suave);
        } else {
            app.pintar(&mut lienzo, Some(&fuente));
        }
    }
    lienzo
}

/// Copia la vista previa sobre el lienzo de la ventana, en (`dx`, `dy`).
fn blit(destino: &mut gfx::Canvas, origen: &gfx::Canvas, dx: i32, dy: i32) {
    for y in 0..origen.height {
        for x in 0..origen.width {
            destino.put(dx + x, dy + y, origen.get(x, y));
        }
    }
}

/// Marco de 1 px con asas en las esquinas.
fn marco_de(lienzo: &gfx::Canvas, r: Rect, color: u32) {
    if r.w < 8 || r.h < 8 {
        return;
    }
    lienzo.hline(r.x, r.der() - 1, r.y, color);
    lienzo.hline(r.x, r.der() - 1, r.abajo() - 1, color);
    lienzo.vline(r.x, r.y, r.abajo() - 1, color);
    lienzo.vline(r.der() - 1, r.y, r.abajo() - 1, color);
    for (hx, hy) in [
        (r.x, r.y),
        (r.der() - 3, r.y),
        (r.x, r.abajo() - 3),
        (r.der() - 3, r.abajo() - 3),
    ] {
        lienzo.fill_rect(hx, hy, 3, 3, color);
    }
}

// --- Diseno de ejemplo ----------------------------------------------------

/// Diseno de muestra: el que usan los modos `--generar` y `--png`.
fn diseno_ejemplo() -> Diseno {
    let mut d = Diseno::nuevo();
    d.alto = 340;
    // Botones.
    for (p, texto, x, accion) in [
        (&PALETA[0], "Aceptar", 16, Some("demo.pulsar")),
        (&PALETA[1], "Guardar", 132, Some("demo.pulsar")),
        (&PALETA[2], "Cerrar", 248, Some("window.cerrar")),
        (&PALETA[7], "Apagado", 364, None),
    ] {
        let i = d.anexar(p, texto, x, 24, accion);
        d.controles[i].alto = 36;
    }
    // Controles.
    d.anexar(&CONTROLES[0], "", 40, 84, None); // check
    d.anexar(&CONTROLES[1], "Sí", 40, 118, None); // radio
    d.anexar(&CONTROLES[1], "No", 40, 148, None); // radio
    d.anexar(&CONTROLES[2], "", 280, 84, None); // circular gris, por defecto
    d.anexar(&CONTROLES[3], "Uno|Dos|Tres", 40, 186, None); // segmented
    d.anexar(&CONTROLES[4], "Rojo|Verde|Azul", 260, 186, None); // popup
    d.anexar(&CONTROLES[5], "Escribe aquí", 40, 230, None); // field
    let slider = d.anexar(&CONTROLES[6], "", 40, 270, None);
    d.controles[slider].valor = 30;
    let gota = d.anexar(&CONTROLES[7], "", 40, 310, None);
    d.controles[gota].valor = 70;
    d.seleccion = Some(1);
    d
}

/// Nombre de fichero de una ruta.
fn nombre_de(p: &Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => String::from("?"),
    }
}

/// Escribe un `.toby` único fuera del repositorio para la ventana de prueba.
fn escribir_preview_temporal(documento: &str) -> std::io::Result<PathBuf> {
    let marca = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let ruta = std::env::temp_dir().join(format!(
        "tobyui-preview-{}-{}.toby",
        std::process::id(),
        marca
    ));
    std::fs::write(&ruta, documento.as_bytes())?;
    Ok(ruta)
}

/// Intérprete del mismo checkout; permite ejecutar AppKit en debug o
/// release sin fijarse a una sola configuración de compilación.
fn ruta_interprete() -> Option<PathBuf> {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("interprete de tobyui lang")
        .join("target");
    ["release", "debug"]
        .iter()
        .map(|perfil| raiz.join(perfil).join("interprete-tobyui.exe"))
        .find(|p| p.is_file())
}

/// Comando para abrir un documento con el interprete.
fn comando_interprete(doc: &Path) -> String {
    let exe = ruta_interprete().unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("interprete de tobyui lang")
            .join("target")
            .join("release")
            .join("interprete-tobyui.exe")
    });
    format!("\"{}\" \"{}\"", exe.display(), doc.display())
}

// --- Ventana Win32 --------------------------------------------------------

/// Estado vivo del editor.
static SESION: OnceLock<Mutex<Ensamblador>> = OnceLock::new();

/// Coordenadas de raton empaquetadas en el `lParam`.
fn punto(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

/// Ejecuta algo sobre el estado del editor.
fn con_estado(f: impl FnOnce(&mut Ensamblador)) {
    let sesion = match SESION.get() {
        Some(s) => s,
        None => return,
    };
    if let Ok(mut guard) = sesion.lock() {
        f(&mut guard);
    }
}

/// Pinta el editor y lo vuelca al area de cliente.
fn pintar_ventana(hwnd: HWND) {
    con_estado(|e| e.pintar());
    let sesion = match SESION.get() {
        Some(s) => s,
        None => return,
    };
    let guard = match sesion.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let lienzo = &guard.lienzo;
    unsafe {
        let mut ps: PAINTSTRUCT = core::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);

        let mut bmi: BITMAPINFO = core::mem::zeroed();
        bmi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as DWORD;
        bmi.bmiHeader.biWidth = lienzo.width;
        bmi.bmiHeader.biHeight = -lienzo.height; // origen arriba (top-down)
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        StretchDIBits(
            hdc,
            0,
            0,
            lienzo.width,
            lienzo.height,
            0,
            0,
            lienzo.width,
            lienzo.height,
            lienzo.pixeles().as_ptr() as *const c_void,
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        EndPaint(hwnd, &ps);
    }
}

/// Pide repintado de la ventana.
fn repintar(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, core::ptr::null(), 0) };
}

/// Fija el tamano actual del area de cliente y agranda el lienzo del editor.
///
/// Lo llama `WM_SIZE`, que llega al abrir y con cada cambio de tamano (F11
/// incluido): la ventana manda y el lienzo crece con ella.
fn redimensionar(w: i32, h: i32) {
    if w <= 0 || h <= 0 {
        return;
    }
    VENTANA_ANCHO.store(w, Ordering::Relaxed);
    VENTANA_ALTO.store(h, Ordering::Relaxed);
    con_estado(|e| e.lienzo = gfx::Canvas::nuevo(w, h));
}

/// F11: alterna entre maximizada y ventana normal.
fn alternar_maximizada(hwnd: HWND) {
    unsafe {
        if IsZoomed(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else {
            ShowWindow(hwnd, SW_MAXIMIZE);
        }
    }
}

/// Procedimiento de la ventana.
unsafe extern "system" fn procedimiento(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // El editor se pinta entero cada vez: no dejamos que Windows borre.
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            // La ventana manda: el lienzo se adapta al area de cliente.
            let w = (lparam & 0xFFFF) as u16 as i32;
            let h = ((lparam >> 16) & 0xFFFF) as u16 as i32;
            redimensionar(w, h);
            repintar(hwnd);
            0
        }
        WM_PAINT => {
            pintar_ventana(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = punto(lparam);
            con_estado(|e| {
                e.mover(x, y);
            });
            repintar(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = punto(lparam);
            // El raton es de esta ventana hasta que se suelte: asi el arrastre
            // sigue aunque el puntero se salga del area de cliente.
            SetCapture(hwnd);
            con_estado(|e| e.clic(x, y));
            repintar(hwnd);
            0
        }
        WM_LBUTTONUP => {
            // Primero se acaba el arrastre y luego se suelta el raton: asi el
            // WM_CAPTURECHANGED que provoca ReleaseCapture ya no encuentra nada
            // que terminar.
            con_estado(|e| e.soltar());
            ReleaseCapture();
            repintar(hwnd);
            0
        }
        // Si otra ventana se lleva el raton (Alt+Tab, por ejemplo), el arrastre
        // se da por terminado: mejor dejarlo a medias que creer que sigue.
        WM_CAPTURECHANGED => {
            con_estado(|e| e.soltar());
            repintar(hwnd);
            0
        }
        WM_CHAR => {
            let ch = wparam as u32;
            con_estado(|e| e.teclear(ch));
            repintar(hwnd);
            0
        }
        WM_KEYDOWN => {
            if wparam == VK_F11 {
                // Pantalla completa y ventana normal.
                alternar_maximizada(hwnd);
            } else if wparam != VK_TAB {
                con_estado(|e| e.tecla(wparam));
            }
            repintar(hwnd);
            0
        }
        WM_CLOSE | WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Crea la ventana y bombea mensajes hasta que se cierre.
fn ventana() -> Result<(), String> {
    unsafe {
        let hinst = GetModuleHandleW(core::ptr::null());
        let clase = win32::ancho("EnsambladorTobyUIVentana");
        let titulo = win32::ancho("Toby AppKit");

        let mut wc: WNDCLASSW = core::mem::zeroed();
        wc.lpfnWndProc = Some(procedimiento);
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(core::ptr::null_mut(), IDC_ARROW as *const u16);
        wc.lpszClassName = clase.as_ptr();
        if RegisterClassW(&wc) == 0 {
            return Err(String::from("RegisterClassW fallo"));
        }

        let estilo = WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX;
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: VENTANA_W,
            bottom: VENTANA_H,
        };
        AdjustWindowRect(&mut rc, estilo, 0);
        let ancho_v = rc.right - rc.left;
        let alto_v = rc.bottom - rc.top;
        let x = ((GetSystemMetrics(SM_CXSCREEN) - ancho_v) / 2).max(0);
        let y = ((GetSystemMetrics(SM_CYSCREEN) - alto_v) / 2).max(0);

        let hwnd = CreateWindowExW(
            0,
            clase.as_ptr(),
            titulo.as_ptr(),
            estilo,
            x,
            y,
            ancho_v,
            alto_v,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            hinst,
            core::ptr::null_mut(),
        );
        if hwnd.is_null() {
            return Err(String::from("CreateWindowExW fallo"));
        }

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        // Maximizada desde el principio: la pantalla manda (F11 alterna a
        // ventana normal). El orden importa: maximizar antes de enfocar
        // des-maximiza al recibir el foco (ya visto en el interprete).
        ShowWindow(hwnd, SW_MAXIMIZE);

        let mut msg: MSG = core::mem::zeroed();
        while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

// --- Autoprueba de la logica del editor -----------------------------------

/// Un pixel del lienzo del editor.
///
/// Sirve para comprobar en la autoprueba que algo **sale pintado** y no solo
/// que este en el modelo: en un entorno sin pantalla, mirar el bufer es la
/// unica forma de ver que el motor ha pintado lo que se espera.
fn pixel_del_lienzo(e: &Ensamblador, x: i32, y: i32) -> u32 {
    let fila = y.max(0) as usize * e.lienzo.stride;
    e.lienzo.pixeles()[fila + x.max(0) as usize]
}

/// Ejercita la logica del editor sin ventana y la compara con el layout que
/// calcula el motor. Devuelve cuantos fallos ha encontrado.
///
/// Es la unica forma de comprobar la parte interactiva en un entorno sin
/// pantalla: se simulan clics, arrastres y teclas llamando a los mismos
/// metodos que llama el procedimiento de ventana.
fn prueba() -> i32 {
    let mut ed = Ensamblador::nuevo(PathBuf::from("prueba.toby"));
    let hoja = ed.rect_hoja();
    let mut fallos = 0;

    macro_rules! comprobar {
        ($cond:expr, $texto:expr) => {
            if $cond {
                println!("  ok  {}", $texto);
            } else {
                println!("  MAL {}", $texto);
                fallos += 1;
            }
        };
    }

    println!("--- autoprueba de Toby AppKit ---");
    let src_adaptativo = concat!(
        "<toby-app name=\"adaptativo\">",
        "<window title=\"Adaptativo\" width=\"480\" height=\"300\" align=\"stretch\">",
        "<stack width=\"480\" height=\"300\" grow=\"1\" responsive=\"scale\">",
        "<button id=\"accion\" title=\"Aceptar\" height=\"24\" x=\"20\" y=\"10\"/>",
        "</stack></window></toby-app>"
    );
    let mut adaptativa = App::nueva(src_adaptativo, 480, 300, Registro::<Ctx>::nuevo())
        .expect("documento adaptable");
    let original = adaptativa.disposicion().rect(3);
    adaptativa.cambiar_tamano(960, 600);
    let maximizado = adaptativa.disposicion().rect(3);
    comprobar!(
        maximizado.x == original.x * 2
            && maximizado.y == original.y * 2
            && maximizado.w == original.w * 2
            && maximizado.h == original.h * 2,
        "el layout compartido adapta posición y tamaño al maximizar"
    );
    adaptativa.cambiar_tamano(480, 300);
    comprobar!(
        adaptativa.disposicion().rect(3) == original,
        "el layout vuelve exactamente al tamaño original al restaurar"
    );
    let mut finder = App::nueva(
        include_str!("../../system_apps/toby_finder.app/ui/main.toby"),
        1024,
        768,
        Registro::<Ctx>::nuevo(),
    )
    .expect("Toby Finder adaptable");
    let indice_contenido = finder
        .documento()
        .indice_de("contenido")
        .expect("control de contenido del Finder");
    let ancho_finder_normal = finder.disposicion().rect(indice_contenido).w;
    let redimensionado = finder.cambiar_tamano_ventana(1024, 700);
    comprobar!(
        redimensionado && finder.disposicion().rect(indice_contenido).w > ancho_finder_normal,
        "Toby Finder redistribuye el espacio al maximizar en Kira"
    );

    comprobar!(
        ed.diseno.controles.is_empty(),
        "arranca con el lienzo en blanco"
    );
    comprobar!(ed.app.is_some(), "el documento vacio monta en el motor");
    ed.pintar();
    let ventana = ed.rect_ventana();
    comprobar!(
        pixel_del_lienzo(&ed, ventana.x + 1, ventana.y + 1) == FONDO_ESCRITORIO,
        "la esquina superior de la ventana tiene el radio compartido"
    );
    comprobar!(
        pixel_del_lienzo(&ed, ventana.x, ventana.abajo() - 1) == TEMA_POR_DEFECTO.paleta.borde,
        "la esquina inferior de la ventana termina recta"
    );
    comprobar!(
        ventana.abajo() == ed.rect_hoja().abajo() + toby_ui::renderer::ALTO_BARRA_VENTANA as i32,
        "el pie inferior queda justo debajo del contenido"
    );
    comprobar!(
        {
            let pixel = pixel_del_lienzo(
                &ed,
                ventana.x + ventana.w as i32 / 2,
                ventana.abajo() - toby_ui::renderer::ALTO_BARRA_VENTANA as i32 / 2,
            );
            pixel != TEMA_POR_DEFECTO.paleta.fondo && pixel != TEMA_POR_DEFECTO.paleta.borde
        },
        "el pie inferior muestra la franja gris degradada"
    );
    comprobar!(
        {
            let pixel = pixel_del_lienzo(&ed, ventana.x + 46, ventana.y + 14);
            ((pixel >> 8) & 0xFF) > ((pixel >> 16) & 0xFF) && ((pixel >> 8) & 0xFF) > (pixel & 0xFF)
        },
        "la barra muestra un botón verde AQUA tipo píldora"
    );
    comprobar!(
        {
            let pixel = pixel_del_lienzo(&ed, ventana.x + ventana.w as i32 - 20, ventana.y + 14);
            let rojo = (pixel >> 16) & 0xFF;
            let verde = (pixel >> 8) & 0xFF;
            let azul = pixel & 0xFF;
            rojo > 0xAA
                && verde > 0xAA
                && azul > 0xAA
                && rojo.abs_diff(verde) < 0x40
                && verde.abs_diff(azul) < 0x40
        },
        "la barra muestra a la derecha una píldora blanca AQUA pequeña"
    );
    comprobar!(
        pixel_del_lienzo(&ed, ventana.x + ventana.w as i32 + 2, ventana.y + 14) == FONDO_ESCRITORIO,
        "el marco de la ventana no proyecta sombra"
    );

    // Un clic suelto en la hoja ya no coloca nada: se coloca arrastrando.
    ed.clic(hoja.x + 100, hoja.y + 80);
    comprobar!(
        ed.diseno.controles.is_empty(),
        "pinchar en la hoja no inserta"
    );

    // Arrastrar el tipo "Verde" desde el panel hasta (100, 80).
    let p1 = rect_paleta(1);
    ed.clic(p1.x + 5, p1.y + 5);
    comprobar!(ed.tipo == Some(1), "el clic en el panel coge el tipo Verde");
    comprobar!(
        matches!(ed.arrastre, Some(Arrastre::Traer { indice: None, .. })),
        "el clic en el panel empieza a traer el tipo"
    );
    comprobar!(
        ed.diseno.controles.is_empty(),
        "mientras no llega a la hoja no hay control"
    );

    // Salirse de la hoja lo quita, y volver a entrar lo vuelve a poner.
    ed.mover(hoja.x - 40, hoja.y + 80);
    comprobar!(
        ed.diseno.controles.is_empty(),
        "fuera de la hoja no se coloca nada"
    );
    ed.mover(hoja.x + 100, hoja.y + 80);
    comprobar!(
        ed.diseno.controles.len() == 1,
        "al entrar en la hoja aparece el control"
    );
    comprobar!(
        matches!(
            ed.arrastre,
            Some(Arrastre::Traer {
                indice: Some(0),
                ..
            })
        ),
        "el control que se trae es el provisional"
    );
    ed.soltar();
    comprobar!(ed.arrastre.is_none(), "soltar termina el arrastre");
    comprobar!(
        ed.diseno.seleccion == Some(0),
        "el control colocado queda seleccionado"
    );
    comprobar!(
        ed.diseno.controles[0].variante == "green",
        "el control traido usa la variante elegida en el panel"
    );
    let r = ed.rect_de(0).expect("el motor da rectangulo");
    comprobar!(
        (r.x + r.w as i32 / 2 - 100).abs() <= 1 && (r.y + r.h as i32 / 2 - 80).abs() <= 1,
        &format!(
            "el control queda centrado donde se solto ({}, {})",
            r.x, r.y
        )
    );
    comprobar!(
        r.x == ed.diseno.controles[0].x && r.y == ed.diseno.controles[0].y,
        "la posicion del modelo es exactamente la que pinta el motor"
    );

    // Traer un segundo boton, de otra variante y en otro sitio.
    let p2 = rect_paleta(2);
    ed.clic(p2.x + 5, p2.y + 5);
    ed.mover(hoja.x + 300, hoja.y + 200);
    ed.soltar();
    comprobar!(
        ed.diseno.controles.len() == 2,
        "se pueden colocar varios controles"
    );
    comprobar!(
        ed.diseno.seleccion == Some(1),
        "el ultimo colocado queda seleccionado"
    );

    // Traer un tercero y soltarlo fuera de la hoja: no queda nada.
    ed.clic(p2.x + 5, p2.y + 5);
    ed.mover(hoja.x + 300, hoja.y + 250);
    comprobar!(
        ed.diseno.controles.len() == 3,
        "el provisional se ve mientras se arrastra"
    );
    // Y sale pintado de verdad: se mira el mismo pixel del lienzo con el
    // boton provisional puesto y sin el.
    ed.pintar();
    let con = pixel_del_lienzo(&ed, hoja.x + 300, hoja.y + 250);
    ed.mover(hoja.x - 30, hoja.y - 30);
    comprobar!(
        ed.diseno.controles.len() == 2,
        "al salir de la hoja el provisional se quita"
    );
    ed.pintar();
    let sin = pixel_del_lienzo(&ed, hoja.x + 300, hoja.y + 250);
    comprobar!(
        con != sin,
        "lo que se arrastra sale pintado en el lienzo (WYSIWYG)"
    );
    ed.soltar();
    comprobar!(
        ed.diseno.controles.len() == 2,
        "soltar fuera de la hoja no coloca nada"
    );

    // Arrastrar el primero hasta (200, 150).
    let r0 = ed.rect_de(0).expect("rect del primero");
    ed.clic(
        hoja.x + r0.x + r0.w as i32 / 2,
        hoja.y + r0.y + r0.h as i32 / 2,
    );
    comprobar!(
        matches!(ed.arrastre, Some(Arrastre::Mover { indice: 0, .. })),
        "pinchar encima de un control empieza a moverlo"
    );
    comprobar!(
        ed.diseno.seleccion == Some(0),
        "ese clic selecciona el control de debajo"
    );
    ed.mover(hoja.x + 200, hoja.y + 150);
    ed.soltar();
    comprobar!(ed.arrastre.is_none(), "soltar termina el arrastre");
    let r0b = ed.rect_de(0).expect("rect tras arrastrar");
    comprobar!(
        (r0b.x + r0b.w as i32 / 2 - 200).abs() <= 1 && (r0b.y + r0b.h as i32 / 2 - 150).abs() <= 1,
        &format!("el control sigue al puntero ({}, {})", r0b.x, r0b.y)
    );

    // Los controles no se pueden sacar de la hoja.
    ed.clic(hoja.x + r0b.x + 2, hoja.y + r0b.y + 2);
    ed.mover(hoja.x - 400, hoja.y - 400);
    ed.soltar();
    let r0c = ed.rect_de(0).expect("rect pegado arriba-izquierda");
    comprobar!(
        r0c.x == 0 && r0c.y == 0,
        "el arrastre se recorta en la esquina (0, 0)"
    );
    ed.clic(hoja.x + 2, hoja.y + 2);
    ed.mover(hoja.x + 9999, hoja.y + 9999);
    ed.soltar();
    let r0d = ed.rect_de(0).expect("rect pegado abajo-derecha");
    comprobar!(
        r0d.x == ed.diseno.ancho as i32 - r0d.w as i32
            && r0d.y == ed.diseno.alto as i32 - r0d.h as i32,
        "el arrastre se recorta en la esquina opuesta"
    );

    // Flechas: afinar posicion pixel a pixel.
    let antes = ed.rect_de(0).expect("rect antes de las flechas");
    ed.tecla(VK_LEFT);
    ed.tecla(VK_UP);
    let despues = ed.rect_de(0).expect("rect despues de las flechas");
    comprobar!(
        despues.x == antes.x - 1 && despues.y == antes.y - 1,
        "las flechas mueven el boton un pixel"
    );

    // Teclear cambia el texto (y con el, el ancho que mide el motor).
    let ancho_antes = ed.rect_de(0).expect("rect").w;
    for c in "OK!".chars() {
        ed.teclear(c as u32);
    }
    comprobar!(
        ed.diseno.controles[0].texto.ends_with("OK!"),
        &format!(
            "el teclado anade al texto ('{}')",
            ed.diseno.controles[0].texto
        )
    );
    let ancho_despues = ed.rect_de(0).expect("rect").w;
    comprobar!(
        ancho_despues > ancho_antes,
        "el control se ensancha con el texto nuevo"
    );
    ed.teclear(0x08);
    comprobar!(
        ed.diseno.controles[0].texto.ends_with("OK"),
        "el retroceso borra el ultimo caracter"
    );
    // Los ASCII reservados se escriben y el documento se puede volver a leer.
    for c in ['<', '>', '&', '"', '\''] {
        ed.teclear(c as u32);
    }
    comprobar!(
        ed.diseno.controles[0].texto.ends_with("<>&\"'"),
        "los caracteres ASCII reservados se pueden escribir"
    );
    let doc_ascii = ed.diseno.documento();
    comprobar!(
        doc_ascii.contains("&lt;&gt;&amp;&quot;&apos;"),
        "los caracteres ASCII se escapan al generar Toby UI"
    );
    comprobar!(
        App::nueva(
            &doc_ascii,
            ed.diseno.ancho,
            ed.diseno.alto,
            Registro::<Ctx>::nuevo()
        )
        .is_ok(),
        "la vista con ASCII reservado vuelve a cargar en el motor"
    );

    // Accion: ninguna -> demo.pulsar -> window.cerrar -> ninguna.
    ed.ciclar_accion();
    comprobar!(
        ed.diseno.controles[0].accion.as_deref() == Some("demo.pulsar"),
        "el ciclo de accion pone demo.pulsar"
    );
    ed.ciclar_accion();
    comprobar!(
        ed.diseno.controles[0].accion.as_deref() == Some("window.cerrar"),
        "el ciclo de accion pasa a window.cerrar"
    );
    ed.ciclar_accion();
    comprobar!(
        ed.diseno.controles[0].accion.is_none(),
        "el ciclo de accion vuelve a ninguna"
    );

    // El documento generado se vuelve a leer sin errores y con los mismos
    // sitios: es la garantia de que el editor no miente.
    let src = ed.diseno.documento();
    let mut registro: Registro<Ctx> = Registro::nuevo();
    for clave in ACCIONES {
        let _ = registro.registrar(clave, accion_anotar);
    }
    match App::nueva(&src, ed.diseno.ancho, ed.diseno.alto, registro) {
        Ok(app) => {
            let mut iguales = true;
            for b in ed.diseno.controles.iter() {
                match app.documento().indice_de(&b.id) {
                    Some(idx) => {
                        let rr = app.disposicion().rect(idx);
                        if rr.x != b.x || rr.y != b.y {
                            iguales = false;
                        }
                    }
                    None => iguales = false,
                }
            }
            comprobar!(iguales, "al releer el .toby cada control cae en su sitio");
        }
        Err(e) => {
            println!("  MAL el .toby generado no monta: {}", e.resumen());
            fallos += 1;
        }
    }

    // Los ocho controles del lenguaje: se cogen del panel, se sueltan en la
    // hoja y el documento que sale monta en el motor con su elemento.
    ed.vaciar();
    for (k, p) in CONTROLES.iter().enumerate() {
        let idx = PALETA.len() + k;
        let rect = rect_paleta(idx);
        ed.clic(rect.x + 4, rect.y + 4);
        comprobar!(
            matches!(ed.arrastre, Some(Arrastre::Traer { tipo, .. }) if tipo == idx),
            format!("la entrada '{}' del panel empieza un arrastre", p.etiqueta)
        );
        ed.mover(hoja.x + 150, hoja.y + 120);
        ed.soltar();
        comprobar!(
            ed.diseno.controles.len() == k + 1 && ed.error.is_none(),
            format!("colocar '{}' monta en el motor", p.tipo)
        );
        comprobar!(
            ed.diseno.controles[k].tipo == p.tipo,
            format!("lo colocado es un '{}'", p.tipo)
        );
        comprobar!(
            ed.diseno.documento().contains(&format!("<{} id=", p.tipo)),
            format!("el .toby lleva el elemento <{}>", p.tipo)
        );
    }

    // Cada tipo escribe sus propias propiedades.
    ed.vaciar();
    ed.insertar(PALETA.len(), 40, 40); // check
    comprobar!(
        ed.diseno.documento().contains("label=\"Marcar\""),
        "el check escribe su etiqueta"
    );
    ed.insertar(PALETA.len() + 1, 40, 80); // radio
    comprobar!(
        ed.diseno.documento().contains("group=\"g1\""),
        "el radio escribe su grupo (asi se excluyen entre si)"
    );
    ed.insertar(PALETA.len() + 3, 40, 120); // segmented
    comprobar!(
        ed.diseno.documento().contains("items=\"Uno|Dos|Tres\""),
        "el segmented escribe sus opciones"
    );

    // El campo no tiene variantes en el lenguaje: el documento no las escribe.
    ed.vaciar();
    ed.insertar(PALETA.len() + 5, 40, 40); // field
    let src_campo = ed.diseno.documento();
    comprobar!(
        src_campo.contains("placeholder=\"Texto\"") && !src_campo.contains("variant"),
        "el campo escribe su marcador y no lleva variante"
    );

    // El slider se teclea con digitos; el .toby lleva value/min/max.
    ed.vaciar();
    let i = ed.insertar(PALETA.len() + 6, 200, 200); // slider
    for c in "75".chars() {
        ed.teclear(c as u32);
    }
    comprobar!(
        ed.diseno.controles[i].valor == 75,
        "teclear fija el valor del slider"
    );
    comprobar!(
        ed.diseno
            .documento()
            .contains("value=\"75\" min=\"0\" max=\"100\""),
        "el slider escribe value/min/max"
    );
    ed.teclear(0x08);
    comprobar!(
        ed.diseno.controles[i].valor == 7,
        "el retroceso baja el valor del slider"
    );

    // Como en HTML, cada control lleva su `name`: propio, derivado de su
    // texto y unico en el documento.
    ed.vaciar();
    ed.insertar(0, 40, 40); // boton "Boton"
    ed.insertar(0, 40, 80); // otro boton "Boton"
    ed.insertar(PALETA.len(), 40, 120); // check "Marcar"
    let src_nombres = ed.diseno.documento();
    comprobar!(
        src_nombres.contains("name=\"boton\"") && src_nombres.contains("name=\"boton-2\""),
        "dos controles con el mismo texto se numeran (boton, boton-2)"
    );
    comprobar!(
        src_nombres.contains("name=\"marcar\""),
        "el name sale del texto del control"
    );

    // El name se regenera al editar el texto.
    ed.diseno.seleccion = Some(0);
    while !ed.diseno.controles[0].texto.is_empty() {
        ed.teclear(0x08);
    }
    for c in "Aceptar".chars() {
        ed.teclear(c as u32);
    }
    comprobar!(
        ed.diseno.documento().contains("name=\"aceptar\"")
            && ed.diseno.documento().contains("name=\"boton\""),
        "cambiar el texto cambia el name del control"
    );

    // El slider no tiene texto: su name es el tipo.
    ed.vaciar();
    ed.insertar(PALETA.len() + 6, 40, 40); // slider
    comprobar!(
        ed.diseno.documento().contains("name=\"slider\""),
        "un control sin texto usa su tipo como name"
    );

    // El name de la app tambien ocupa ese espacio: no se puede repetir con el
    // de un control (el lenguaje lo rechazaria al montar).
    ed.vaciar();
    ed.diseno.nombre = String::from("aceptar");
    let ib = ed.insertar(0, 40, 40); // boton "Boton"
    while !ed.diseno.controles[ib].texto.is_empty() {
        ed.teclear(0x08);
    }
    for c in "Aceptar".chars() {
        ed.teclear(c as u32);
    }
    comprobar!(
        ed.diseno.documento().contains("name=\"aceptar-2\""),
        "el name choca con el de la app y se numerado"
    );
    comprobar!(
        ed.error.is_none(),
        "el documento con ese name monta en el motor"
    );
    ed.diseno.nombre = String::from("diseno");

    // El name del seleccionado se ve en el pie del editor: el rotulo dice el
    // name del documento y sus pixeles estan pintados en el hueco libre del
    // pie, a la izquierda de los botones de la barra de acciones.
    ed.vaciar();
    ed.insertar(0, 60, 60); // boton "Boton" -> name="boton"
    ed.tipo = None;
    ed.diseno.seleccion = Some(0);
    comprobar!(
        ed.rotulo_nombre() == "name=boton",
        "el pie rotula el name del control seleccionado"
    );
    comprobar!(
        ed.diseno.nombres().len() == ed.diseno.controles.len() + 1
            && ed.diseno.nombres()[1] == "boton",
        "la lista de names del editor es la del documento"
    );
    ed.pintar();
    let pie_y = tam_ventana().1 - PIE_ALTA + 8;
    let fondo_pie = pixel_del_lienzo(&ed, 4, pie_y + 4);
    let mut pintados = 0;
    for yy in pie_y..pie_y + 10 {
        for xx in 12..216 {
            if pixel_del_lienzo(&ed, xx, yy) != fondo_pie {
                pintados += 1;
            }
        }
    }
    comprobar!(
        pintados > 60,
        &format!(
            "el name del control esta pintado en el pie ({} px)",
            pintados
        )
    );
    ed.diseno.seleccion = None;
    comprobar!(
        ed.rotulo_nombre() == "nada seleccionado",
        "sin seleccion el pie lo dice"
    );

    // El panel: cada entrada sale pintada y las miniaturas no se desbordan.
    // (Se quita el resaltado de seleccion para que el borde naranja no cuente.)
    ed.tipo = None;
    ed.pintar();
    let fondo = pixel_del_lienzo(&ed, 4, BARRA_ALTA + 4);
    for i in 0..entradas_panel() {
        let rect = rect_paleta(i);
        let mut distintos = 0;
        for yy in rect.y..rect.abajo() {
            for xx in rect.x..rect.der() {
                if pixel_del_lienzo(&ed, xx, yy) != fondo {
                    distintos += 1;
                }
            }
        }
        comprobar!(
            distintos > 40,
            format!("la entrada {} del panel sale pintada ({} px)", i, distintos)
        );
    }
    let mut limpio = true;
    for i in 0..entradas_panel() {
        let rect = rect_paleta(i);
        for yy in rect.abajo()..(rect.y + 32).min(tam_ventana().1 - PIE_ALTA) {
            for xx in rect.x..rect.der() {
                if pixel_del_lienzo(&ed, xx, yy) != fondo {
                    limpio = false;
                }
            }
        }
    }
    comprobar!(limpio, "entre entradas del panel no se pinta de mas");

    // Las ocho miniaturas de los controles son distintas entre si.
    let mut distintas = true;
    for a in 0..ed.miniaturas.len() {
        for b in a + 1..ed.miniaturas.len() {
            if ed.miniaturas[a].pixeles() == ed.miniaturas[b].pixeles() {
                distintas = false;
            }
        }
    }
    comprobar!(distintas, "las ocho miniaturas del panel son distintas");

    // Supr quita el seleccionado; vaciar deja el lienzo en blanco.
    ed.insertar(0, 240, 240); // un boton mas: quedan dos controles
    ed.diseno.seleccion = Some(0);
    ed.tecla(VK_DELETE);
    comprobar!(
        ed.diseno.controles.len() == 1,
        "Supr quita el control seleccionado"
    );
    ed.vaciar();
    comprobar!(
        ed.diseno.controles.is_empty(),
        "Vaciar deja el lienzo en blanco"
    );

    // Generar escribe el fichero.
    let salida = std::env::temp_dir().join("toby_appkit_autoprueba.toby");
    ed.diseno.anexar(&PALETA[0], "Hola", 10, 10, None);
    ed.salida = salida.clone();
    ed.reconstruir();
    ed.generar();
    match std::fs::read_to_string(&salida) {
        Ok(t) => comprobar!(
            t.contains("<toby-app name=\"diseno\">") && t.contains("x=\"10\" y=\"10\""),
            "Generar escribe el .toby en el fichero de salida"
        ),
        Err(e) => {
            println!("  MAL no se pudo leer lo generado: {}", e);
            fallos += 1;
        }
    }

    let test = rect_testear();
    comprobar!(
        test.x >= rect_generar().der() && test.der() <= rect_accion().x,
        "Testear ocupa su propio botón, entre Generar y Acción"
    );
    let circular = {
        let mut d = Diseno::nuevo();
        d.anexar(&CONTROLES[2], "", 0, 0, None);
        d.documento()
    };
    comprobar!(
        circular.contains("<circular")
            && circular.contains("title=\"C\"")
            && !circular.contains("variant=\"\""),
        "el circular de la muestra conserva el gris predeterminado de AQUA"
    );
    match escribir_preview_temporal(&ed.diseno.documento()) {
        Ok(temporal) => {
            let coincide = std::fs::read_to_string(&temporal)
                .map(|texto| texto == ed.diseno.documento())
                .unwrap_or(false);
            let _ = std::fs::remove_file(&temporal);
            comprobar!(
                coincide,
                "Testear prepara un .toby temporal idéntico al diseño actual"
            );
        }
        Err(e) => {
            println!("  MAL Testear no pudo preparar el documento: {}", e);
            fallos += 1;
        }
    }

    // Los atajos permiten recorrer tamaños y colores de las muestras AQUA.
    let indice = ed.insertar(0, 180, 130);
    ed.tecla(VK_F2);
    comprobar!(
        ed.diseno.controles[indice].variante == "green",
        "F2 cambia a la siguiente variante AQUA"
    );
    ed.tecla(VK_PRIOR);
    comprobar!(
        ed.diseno.controles[indice].alto == 78,
        "Page Up aplica el siguiente tamaño AQUA"
    );
    comprobar!(
        ed.diseno
            .documento()
            .contains("variant=\"green\" height=\"78\""),
        "variante y tamaño se escriben en el .toby que se previsualiza"
    );
    ed.vaciar();

    println!("--- fallos: {} ---", fallos);
    fallos
}

// --- Entrada --------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "nuevo" {
        match package::nuevo(Path::new(&args[2])) {
            Ok(ruta) => println!("Toby AppKit: paquete creado en {}", ruta.display()),
            Err(e) => {
                eprintln!("Toby AppKit: no se pudo crear la app: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    if args.len() >= 3 && args[1] == "nuevo-sistema" {
        match package::nuevo_sistema(Path::new(&args[2])) {
            Ok(ruta) => println!("Toby AppKit: app de sistema creada en {}", ruta.display()),
            Err(e) => {
                eprintln!("Toby AppKit: no se pudo crear la app de sistema: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    if args.len() >= 3 && args[1] == "validar" {
        match package::validar(Path::new(&args[2])) {
            Ok(m) => println!("OK: {} ({})", m.identidad.titulo, m.nombre),
            Err(e) => {
                eprintln!("Toby AppKit: paquete inválido: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    if args.len() >= 3 && args[1] == "compilar" {
        match package::compilar(Path::new(&args[2])) {
            Ok(ruta) => println!("OK: binario Rust generado en {}", ruta.display()),
            Err(e) => {
                eprintln!("Toby AppKit: error compilando la app: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    let mut salida = Path::new(env!("CARGO_MANIFEST_DIR")).join("salida.toby");
    let mut png: Option<String> = None;
    let mut generar: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--png" => {
                i += 1;
                if i < args.len() {
                    png = Some(args[i].clone());
                }
            }
            "--generar" => {
                i += 1;
                if i < args.len() {
                    generar = Some(args[i].clone());
                }
            }
            "--salida" => {
                i += 1;
                if i < args.len() {
                    salida = PathBuf::from(&args[i]);
                }
            }
            otro if !otro.starts_with("--") => salida = PathBuf::from(otro),
            _ => {}
        }
        i += 1;
    }

    println!("Toby AppKit  -  editor y kit de aplicaciones Rocky");
    println!("salida    : {}", salida.display());

    // Modo de autoprueba: ejercita la logica del editor sin ventana.
    if args.iter().any(|a| a == "--prueba") {
        let fallos = prueba();
        std::process::exit(if fallos == 0 { 0 } else { 1 });
    }

    // Modo sin ventana 1: generar el .toby del diseno de ejemplo.
    if let Some(ruta) = generar {
        let d = diseno_ejemplo();
        let src = d.documento();
        println!("{}", src);
        match std::fs::write(&ruta, src.as_bytes()) {
            Ok(()) => println!("generado  : {} ({} controles)", ruta, d.controles.len()),
            Err(e) => eprintln!("no se pudo escribir '{}': {}", ruta, e),
        }
        println!("para verlo: {}", comando_interprete(Path::new(&ruta)));
        return;
    }

    let mut editor = Ensamblador::nuevo(salida);

    // Modo sin ventana 2: volcar la interfaz del editor a un PNG.
    if let Some(ruta) = png {
        editor.cargar_ejemplo();
        editor.pintar();
        match png_escribir::escribir(
            &ruta,
            VENTANA_W as u32,
            VENTANA_H as u32,
            editor.lienzo.pixeles(),
        ) {
            Ok(()) => println!("PNG       : {} ({}x{})", ruta, VENTANA_W, VENTANA_H),
            Err(e) => eprintln!("no se pudo escribir el PNG: {}", e),
        }
        println!("{}", editor.diseno.documento());
        return;
    }

    println!("uso       : arrastra una entrada del panel hasta la hoja para");
    println!("            colocarla, y pincha encima de un control para moverlo.");
    println!("            Generar .toby escribe el documento (y lo imprime aqui).");
    println!("            sin ventana: --png f.png | --generar d.toby | --prueba");
    println!("            paquetes    : nuevo <nombre.app> | nuevo-sistema <nombre.app>");
    println!("                           validar <nombre.app> | compilar <nombre.app>");
    println!(
        "panel     : {} tipos de boton y {} controles",
        PALETA.len(),
        CONTROLES.len()
    );
    println!(
        "hoja      : la que declara el <window> del diseno ({}x{})",
        HOJA_W, HOJA_H
    );
    println!("F11       : alterna pantalla completa y ventana normal");
    println!("cerrar    : boton de cerrar de la ventana");

    SESION.set(Mutex::new(editor)).ok();
    if let Err(e) = ventana() {
        eprintln!("error creando la ventana: {}", e);
    }
}
