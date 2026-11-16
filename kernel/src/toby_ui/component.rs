//! Definiciones de componentes y propiedades de Toby UI.
//!
//! Cada componente tiene un `ComponentDef` que declara qué propiedades acepta.
//! El parser y la validación leen estas definiciones para rechazar lo que no
//! está permitido. Añadir un componente = añadir una entrada en `COMPONENTES`.
//!
//! Esta es la tabla central del lenguaje: no hay propiedades "libres".

/// Los tipos de valor que puede tener una propiedad.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PropType {
    /// Entero (píxeles, valores discretos, etc).
    Int,
    /// Booleano: "true" o "false".
    Bool,
    /// Texto literal.
    String,
    /// Valor cerrado de un conjunto pequeño (lista en `valores`).
    Enum,
    /// Color en hexadecimal #RRGGBB.
    Color,
    /// Ruta a un PNG autorizado (recurso del sistema o de un bundle `.app`).
    Recurso,
    /// Referencia a un `id` declarado en el mismo documento.
    IdRef,
    /// Clave de una acción registrada en el runtime (`action="dominio.accion"`).
    ///
    /// Sólo es un NOMBRE: el documento nunca aporta código. Si la clave no
    /// existe en el registro de acciones, el fallo es controlado.
    Accion,
}

/// Una propiedad concreta de un componente.
#[derive(Clone, Copy, Debug)]
pub struct PropDef {
    /// Nombre exacto del atributo, p.ej. "title".
    pub nombre: &'static str,
    /// Tipo esperado.
    pub tipo: PropType,
    /// Si la propiedad es obligatoria.
    pub requerida: bool,
    /// Valores permitidos cuando `tipo` es `Enum` (vacío = no aplica).
    pub valores: &'static [&'static str],
}

/// Un componente completo del lenguaje.
#[derive(Clone, Copy, Debug)]
pub struct ComponentDef {
    /// Nombre exacto de la etiqueta, p.ej. "button".
    pub etiqueta: &'static str,
    /// Si es un contenedor (acepta hijos) o una hoja.
    pub contiene_hijos: bool,
    /// Si recibe eventos del usuario (ratón/teclado) y puede tener foco.
    pub interactivo: bool,
    /// Propiedades válidas para este componente.
    pub propiedades: &'static [PropDef],
}

/// Valores del atributo `align` (eje cruzado de `row`/`column`).
pub static VALORES_ALIGN: &[&str] = &["start", "center", "end", "stretch"];

/// Modos de presentación para una ventana de aplicación.
pub static VALORES_MODO_VENTANA: &[&str] = &["fullscreen", "window"];

/// Efectos de transición admitidos por la ventana raíz de una app Toby UI.
pub static VALORES_TRANSICION: &[&str] = &["none", "genie", "bounce"];

/// Valores del atributo `variant` (color del kit AQUA).
pub static VALORES_VARIANTE: &[&str] = &[
    "blue", "green", "red", "orange", "yellow", "magenta", "white",
];
/// Controles cuya hoja de estilos no define una variante blanca.
pub static VALORES_VARIANTE_COLOR: &[&str] =
    &["blue", "green", "red", "orange", "yellow", "magenta"];

/// Valores del atributo `fit` de `<image>`.
pub static VALORES_FIT: &[&str] = &["contain", "cover", "stretch"];
/// Modos admitidos por `<progress>`.
pub static VALORES_PROGRESS_MODE: &[&str] = &["indeterminate", "determinate"];
/// Modos de adaptación de los hijos posicionados de un `<stack>`.
pub static VALORES_RESPONSIVE: &[&str] = &["none", "scale"];

/// Número de opciones de un componente con `items="a|b|c"` (mínimo 1).
///
/// `segmented` y `popup` comparten esta convención: la lista es texto plano
/// separado por `|`, sin listas ni nodos hijos (el lenguaje sigue sin
/// estructuras de datos).
pub fn cuenta_items(items: &str) -> usize {
    let n = items.split('|').count();
    if n == 0 {
        1
    } else {
        n
    }
}

/// La opción `i` de una lista `items="a|b|c"` (fuera de rango = vacío).
pub fn item_en(items: &str, i: usize) -> &str {
    items.split('|').nth(i).unwrap_or("")
}

/// Acota una opción seleccionada al número de opciones disponibles.
pub fn acotar_item(items: &str, seleccion: i64) -> i64 {
    let ultimo = cuenta_items(items) as i64 - 1;
    seleccion.clamp(0, ultimo.max(0))
}

/// Propiedades que colocan una HOJA en absoluto dentro de su contenedor.
///
/// Un nodo que declara `x` o `y` (`button` e `image`) se coloca en
/// `origen_del_contenido + (x, y)` y **no participa del flujo** de
/// `row`/`column`; el contenedor que se ajusta a su contenido lo tiene en
/// cuenta para medirse. Sin `x` ni `y` todo se comporta como hasta ahora.
pub static PROPIEDADES_ABSOLUTAS: &[&str] = &["x", "y"];

/// La propiedad `name`: el NOMBRE propio del elemento, como en HTML.
///
/// Todos los tipos la admiten y es **única en el documento** (lo comprueba
/// `validation`, igual que con `id`). Es opcional y no pinta nada: sirve para
/// identificar el elemento en el listado del intérprete y en las peticiones
/// de acción, de forma que un mismo manejador pueda distinguir de dónde
/// viene sin depender de un `id` técnico.
pub const PROP_NOMBRE: PropDef = PropDef {
    nombre: "name",
    tipo: PropType::String,
    requerida: false,
    valores: &[],
};

/// Todos los componentes permitidos en Toby UI (tabla central).
///
/// Añadir un componente nuevo = añadir una entrada aquí y, si necesita
/// lógica propia, su caso en `layout.rs` y `renderer.rs`. El parser y el
/// validador no cambian.
pub static COMPONENTES: &[ComponentDef] = &[
    ComponentDef {
        etiqueta: "toby-app",
        contiene_hijos: true,
        interactivo: false,
        propiedades: &[PropDef {
            nombre: "name",
            tipo: PropType::String,
            requerida: true,
            valores: &[],
        }],
    },
    ComponentDef {
        etiqueta: "window",
        contiene_hijos: true,
        interactivo: false,
        propiedades: &[
            PROP_NOMBRE,
            PropDef {
                nombre: "title",
                tipo: PropType::String,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "mode",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_MODO_VENTANA,
            },
            PropDef {
                nombre: "transition",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_TRANSICION,
            },
            PropDef {
                nombre: "transition-duration",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "resizable",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            // Alineación de los hijos dentro de la ventana.
            PropDef {
                nombre: "align",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_ALIGN,
            },
            PropDef {
                nombre: "grow",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
        ],
    },
    ComponentDef {
        etiqueta: "column",
        contiene_hijos: true,
        interactivo: false,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "spacing",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "padding",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "align",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_ALIGN,
            },
            PropDef {
                nombre: "grow",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
        ],
    },
    ComponentDef {
        etiqueta: "row",
        contiene_hijos: true,
        interactivo: false,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "spacing",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "padding",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "align",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_ALIGN,
            },
            PropDef {
                nombre: "grow",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
        ],
    },
    ComponentDef {
        etiqueta: "stack",
        contiene_hijos: true,
        interactivo: false,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "padding",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "grow",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "responsive",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_RESPONSIVE,
            },
        ],
    },
    ComponentDef {
        etiqueta: "box",
        contiene_hijos: true,
        interactivo: false,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "title",
                tipo: PropType::String,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "background",
                tipo: PropType::Color,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "bordered",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "spacing",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "padding",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "align",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_ALIGN,
            },
            PropDef {
                nombre: "grow",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
        ],
    },
    ComponentDef {
        etiqueta: "button",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "title",
                tipo: PropType::String,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE,
            },
        ],
    },
    ComponentDef {
        etiqueta: "progress",
        contiene_hijos: false,
        interactivo: false,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "mode",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_PROGRESS_MODE,
            },
            PropDef {
                nombre: "value",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
        ],
    },
    ComponentDef {
        etiqueta: "image",
        contiene_hijos: false,
        interactivo: false,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "src",
                tipo: PropType::Recurso,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "fit",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_FIT,
            },
        ],
    },
    ComponentDef {
        etiqueta: "label",
        contiene_hijos: false,
        interactivo: false,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "text",
                tipo: PropType::String,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "size",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "bold",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "align",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_ALIGN,
            },
        ],
    },
    // --- Controles (Fase 10) ----------------------------------------------
    //
    // Todos son HOJAS con posición libre, como `button`, y todos pueden tener
    // `action`. Lo que los distingue es que guardan un valor en tiempo de
    // ejecución (`app::Valores`): el marcado de un `check`, el índice de un
    // `segmented`, el texto de un `field`... El documento sólo declara el
    // valor INICIAL.
    ComponentDef {
        etiqueta: "check",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "label",
                tipo: PropType::String,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "checked",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE_COLOR,
            },
        ],
    },
    ComponentDef {
        etiqueta: "radio",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "label",
                tipo: PropType::String,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "checked",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            // Los radios con el MISMO `group` son mutuamente excluyentes.
            PropDef {
                nombre: "group",
                tipo: PropType::String,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE_COLOR,
            },
        ],
    },
    ComponentDef {
        etiqueta: "circular",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "title",
                tipo: PropType::String,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE,
            },
        ],
    },
    ComponentDef {
        etiqueta: "segmented",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            // Opciones separadas por `|`: `items="Pequeño|Mediano|Grande"`.
            PropDef {
                nombre: "items",
                tipo: PropType::String,
                requerida: true,
                valores: &[],
            },
            // Índice de la opción elegida (0-based), acotado al número de items.
            PropDef {
                nombre: "selected",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE_COLOR,
            },
        ],
    },
    ComponentDef {
        etiqueta: "popup",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            // Opciones separadas por `|`, igual que `segmented`.
            PropDef {
                nombre: "items",
                tipo: PropType::String,
                requerida: true,
                valores: &[],
            },
            PropDef {
                nombre: "selected",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE_COLOR,
            },
        ],
    },
    ComponentDef {
        etiqueta: "field",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            // Texto inicial (el contenido en tiempo de ejecución vive en la app).
            PropDef {
                nombre: "value",
                tipo: PropType::String,
                requerida: false,
                valores: &[],
            },
            // Texto tenue mientras el campo está vacío.
            PropDef {
                nombre: "placeholder",
                tipo: PropType::String,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            // Dispara al pulsar Intro con el campo enfocado.
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
        ],
    },
    ComponentDef {
        etiqueta: "slider",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "value",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "min",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "max",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            // Dispara al SOLTAR el ratón, si el valor cambió durante el arrastre.
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE_COLOR,
            },
        ],
    },
    // Igual que `slider`, pero con el pulgar de gota del kit AQUA.
    ComponentDef {
        etiqueta: "slider-gota",
        contiene_hijos: false,
        interactivo: true,
        propiedades: &[
            PropDef {
                nombre: "id",
                tipo: PropType::IdRef,
                requerida: false,
                valores: &[],
            },
            PROP_NOMBRE,
            PropDef {
                nombre: "visible",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "x",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "y",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "enabled",
                tipo: PropType::Bool,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "value",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "min",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "max",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "width",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "height",
                tipo: PropType::Int,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "action",
                tipo: PropType::Accion,
                requerida: false,
                valores: &[],
            },
            PropDef {
                nombre: "variant",
                tipo: PropType::Enum,
                requerida: false,
                valores: VALORES_VARIANTE_COLOR,
            },
        ],
    },
];

/// Busca la definición de un componente por etiqueta.
pub fn buscar(etiqueta: &str) -> Option<&'static ComponentDef> {
    // Stepper Control es el nombre nuevo del control de opciones. Conserva
    // `<popup>` como etiqueta histórica para no romper documentos existentes.
    let etiqueta = if etiqueta == "stepper" {
        "popup"
    } else {
        etiqueta
    };
    COMPONENTES.iter().find(|c| c.etiqueta == etiqueta)
}

/// Longitud máxima de una clave de acción, en bytes.
pub const MAX_CLAVE: usize = 64;

/// Número máximo de segmentos de una clave (`a.b.c`).
const MAX_SEGMENTOS: usize = 3;

/// ¿Tiene esta clave la forma de una acción declarable?
///
/// Forma admitida: de 1 a 3 segmentos separados por puntos; cada segmento
/// empieza por `a-z` o `_` y sigue con `a-z`, `0-9`, `_` o `-`. Ejemplos
/// válidos: `save`, `settings.save`, `window.close`, `lista.add-item`.
/// Ejemplos rechazados: ``, `Save`, `save `, `.save`, `save.`, `a..b`, `a/b`.
///
/// Es una comprobación puramente léxica: que la acción EXISTA lo decide el
/// registro en tiempo de ejecución (Fase 7).
pub fn clave_valida(clave: &str) -> bool {
    if clave.is_empty() || clave.len() > MAX_CLAVE {
        return false;
    }
    let mut segmentos = 0;
    for seg in clave.split('.') {
        segmentos += 1;
        if segmentos > MAX_SEGMENTOS || !segmento_valido(seg) {
            return false;
        }
    }
    true
}

/// Un segmento suelto de una clave: `[a-z_][a-z0-9_-]*`.
fn segmento_valido(seg: &str) -> bool {
    let b = seg.as_bytes();
    if b.is_empty() {
        return false;
    }
    if !(b[0].is_ascii_lowercase() || b[0] == b'_') {
        return false;
    }
    b[1..]
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn claves_de_accion_bien_formadas() {
        assert!(clave_valida("save"));
        assert!(clave_valida("settings.save"));
        assert!(clave_valida("window.close"));
        assert!(clave_valida("a.b.c"));
        assert!(clave_valida("lista.add-item"));
        assert!(clave_valida("_privada"));
        assert!(clave_valida("d0.accion1"));
    }

    #[test]
    fn claves_de_accion_mal_formadas() {
        assert!(!clave_valida(""));
        assert!(!clave_valida("Save")); // mayúsculas
        assert!(!clave_valida("save ")); // espacio
        assert!(!clave_valida(" save"));
        assert!(!clave_valida(".save")); // segmento vacío
        assert!(!clave_valida("save."));
        assert!(!clave_valida("a..b"));
        assert!(!clave_valida("a/b"));
        assert!(!clave_valida("a.b.c.d")); // demasiados segmentos
        assert!(!clave_valida("1accion")); // empieza por dígito
        assert!(!clave_valida("con ñ"));
        assert!(!clave_valida(&"x".repeat(MAX_CLAVE + 1)));
        assert!(clave_valida(&"x".repeat(MAX_CLAVE)));
    }

    #[test]
    fn la_tabla_tiene_claves_completas() {
        // Todo componente debe declarar al menos una propiedad y no repetir
        // nombres; `action` sólo aparece donde tiene sentido.
        for c in COMPONENTES {
            let mut vistos: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
            for p in c.propiedades {
                assert!(!p.nombre.is_empty(), "{}: propiedad sin nombre", c.etiqueta);
                assert!(
                    !vistos.contains(&p.nombre),
                    "{}: '{}' repetida",
                    c.etiqueta,
                    p.nombre
                );
                vistos.push(p.nombre);
                if p.tipo == PropType::Enum {
                    assert!(!p.valores.is_empty(), "{}: enum sin valores", p.nombre);
                }
            }
            assert!(vistos.contains(&"id") || c.etiqueta == "toby-app" || c.etiqueta == "window");
        }
    }

    #[test]
    fn variantes_coinciden_con_las_que_define_aqua_css() {
        let variantes = |etiqueta: &str| {
            COMPONENTES
                .iter()
                .find(|c| c.etiqueta == etiqueta)
                .and_then(|c| c.propiedades.iter().find(|p| p.nombre == "variant"))
                .map(|p| p.valores)
                .expect("variant declarado")
        };
        // CSS define la piel blanca sólo para button y circular.
        assert_eq!(variantes("button"), VALORES_VARIANTE);
        assert_eq!(variantes("circular"), VALORES_VARIANTE);
        for etiqueta in [
            "check",
            "radio",
            "segmented",
            "popup",
            "slider",
            "slider-gota",
        ] {
            assert_eq!(variantes(etiqueta), VALORES_VARIANTE_COLOR, "<{etiqueta}>");
        }
    }

    #[test]
    fn stepper_control_acepta_la_etiqueta_nueva_y_conserva_popup() {
        let stepper = buscar("stepper").expect("Stepper Control debe estar disponible");
        let popup = buscar("popup").expect("la etiqueta anterior debe seguir disponible");
        assert!(core::ptr::eq(stepper, popup));
        assert_eq!(stepper.etiqueta, "popup");
    }
}
