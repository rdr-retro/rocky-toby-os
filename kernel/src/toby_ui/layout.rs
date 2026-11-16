//! Motor de layout de Toby UI (Fase 3).
//!
//! Determinista: mismas entradas → mismos rectángulos. No depende del tema
//! ni del renderer (las métricas intrínsecas de hoy son provisionales y
//! vivirán en `theme.rs` en la Fase 4).
//!
//! Dos pasadas sobre un mapa plano de nodos:
//!
//!   1. *Medida* (hacia arriba): cada nodo declara su tamaño deseado.
//!   2. *Colocación* (hacia abajo): cada contenedor reparte a sus hijos.
//!
//! Reglas de reparto en `column`/`window`/`toby-app` (vertical) y `row`
//! (horizontal): `spacing` entre hermanos, `padding` interior, `align` sobre
//! el eje cruzado y `grow` ponderado sobre el principal. `stack` superpone a
//! sus hijos; con `responsive="scale"` escala sus coordenadas y tamaños.
//!
//! **Posición libre:** una hoja (`button`, `image`) que declara `x` o `y` se
//! coloca en `origen_del_contenido + (x, y)` y queda fuera del flujo; el
//! contenedor que se ajusta a su contenido la incluye al medirse. Así un
//! `stack` con hojas posicionadas es un lienzo de posiciones libres.
//!
//! Todas las operaciones son enteras (`i32`/`u32`), sin punto flotante.

use alloc::vec::Vec;

use super::component::{self, ComponentDef, PROPIEDADES_ABSOLUTAS};
use super::document::{Nodo, ValidatedDocument};

/// Alto de la fuente bitmap 8x8 (la escala CSS se calcula por control).
pub const ALTO_LINEA: u32 = 8;
/// Alto de los botones de AppKit, independiente de los controles AQUA.
pub const ALTO_BOTON: u32 = 24;

/// Separación icono texto definida por AQUA como 0,42 × el alto.
pub fn hueco_marca(alto: u32) -> u32 {
    (alto.saturating_mul(42) + 50) / 100
}
/// Ancho del botón de flechas de un `<popup>` para un alto dado:
/// `--aqua-pop-h × 1,3` (31 px con el alto 24 de un control).
///
/// La reserva del layout y el pintado llaman AQUÍ, para que lo que se mide y
/// lo que se pinta sea lo mismo aunque el control lleve `height` propio.
pub fn ancho_paso(alto: u32) -> u32 {
    (alto.saturating_mul(13) + 5) / 10
}
/// Alto de cada fila del menú desplegado de un `<popup>`.
pub const ALTO_ITEM: u32 = 20;
/// Ancho por defecto de `<field>` y `<slider>` según AQUA.
pub const ANCHO_CONTROL: u32 = 240;
/// Tamaño intrínseco del indicador de progreso indeterminado de AppKit.
pub const ANCHO_PROGRESS: u32 = 180;
pub const ALTO_PROGRESS: u32 = 18;
/// La textura porcentual (`loader*.png`) conserva su altura nativa de 16 px.
pub const ALTO_PROGRESS_DETERMINADO: u32 = 16;

/// Tamaño vertical CSS por defecto de cada control AQUA.
pub fn alto_css(etiqueta: &str) -> u32 {
    match etiqueta {
        "button" => 62,
        "segmented" | "popup" => 32,
        _ => 26,
    }
}

/// Aproxima el ancho de la fuente del kit a partir de su tamaño CSS.
///
/// El kernel rasteriza glifos bitmap; el CSS de AQUA usa SF/Segoe UI. La
/// proporción media de 0,52 em conserva la medida de texto de la muestra y
/// permite que el layout responda al tamaño sin fijar caracteres a 8 px.
pub fn ancho_texto_css(texto: &str, alto_fuente: u32) -> u32 {
    let ancho_caracter = (alto_fuente.saturating_mul(52) + 50) / 100;
    (texto.chars().count() as u32).saturating_mul(ancho_caracter)
}

/// Ancho de la casilla de un `<check>` para un alto de `h` px.
///
/// El PNG nativo mide 635 × 649 (un pelo más alto que ancho); es la
/// proporción que fija `check.css`, redondeada a píxeles enteros.
pub fn ancho_check(h: u32) -> u32 {
    (h.saturating_mul(635) + 324) / 649
}

/// Ancho de la esfera de un `<radio>` o de un `<circular>` para un alto de
/// `h` px. El PNG nativo mide 682 × 653 (un poco más ancha que alta): es la
/// proporción de `radio.css` y `circular.css`, redondeada.
pub fn ancho_esfera(h: u32) -> u32 {
    (h.saturating_mul(682) + 326) / 653
}

/// Rectángulo entero en píxeles (coordenadas absolutas dentro de la app).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Rectángulo nulo.
    pub const VACIO: Rect = Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    /// ¿Contiene el punto? (los rectángulos nulos no contienen nada)
    pub fn contiene(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32
    }

    /// Borde derecho (exclusivo).
    pub fn der(&self) -> i32 {
        self.x + self.w as i32
    }

    /// Borde inferior (exclusivo).
    pub fn abajo(&self) -> i32 {
        self.y + self.h as i32
    }

    /// Intersección con un área de recorte.
    pub fn recortar(&self, area: Rect) -> Rect {
        let x1 = self.x.max(area.x);
        let y1 = self.y.max(area.y);
        let x2 = self.der().min(area.der());
        let y2 = self.abajo().min(area.abajo());
        if x2 <= x1 || y2 <= y1 {
            return Rect::VACIO;
        }
        Rect {
            x: x1,
            y: y1,
            w: (x2 - x1) as u32,
            h: (y2 - y1) as u32,
        }
    }
}

/// Alineación sobre el eje cruzado.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alineacion {
    /// Pegado al inicio.
    Inicio,
    /// Centrado.
    Centro,
    /// Pegado al final.
    Fin,
    /// Ocupa todo el ancho/alto disponible en el eje cruzado.
    Estirar,
}

impl Alineacion {
    /// Lee la alineación desde el nodo (`align`).
    pub fn del_nodo(n: &Nodo) -> Alineacion {
        match n.texto("align", "start") {
            "center" => Alineacion::Centro,
            "end" => Alineacion::Fin,
            "stretch" => Alineacion::Estirar,
            _ => Alineacion::Inicio,
        }
    }
}

/// Disponible máximo que un nodo puede ocupar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Restricciones {
    pub max_ancho: u32,
    pub max_alto: u32,
}

impl Restricciones {
    /// Sin tope útil para pruebas y medición interna.
    pub const SIN_TOPE: Restricciones = Restricciones {
        max_ancho: u32::MAX,
        max_alto: u32::MAX,
    };

    /// Tamaño máximo explícito.
    pub fn new(max_ancho: u32, max_alto: u32) -> Restricciones {
        Restricciones {
            max_ancho,
            max_alto,
        }
    }
}

/// Un nodo con su rectángulo calculado.
///
/// `nodos` está en preorden, el mismo orden que `ValidatedDocument::preorden()`.
#[derive(Clone, Debug)]
pub struct Colocado {
    /// Componente del nodo.
    pub def: &'static ComponentDef,
    /// Rectángulo absoluto ya calculado.
    pub rect: Rect,
    /// Índice del padre en `nodos` (ninguno en la raíz).
    pub indice_padre: Option<usize>,
    /// Índices de sus HIJOS DIRECTOS en `nodos`.
    ///
    /// No se usa un rango porque, en preorden, los hijos directos quedan
    /// separados por el subárbol del hermano anterior.
    pub hijos: Vec<usize>,
}

/// Resultado del layout: mapa plano índice → rectángulo.
#[derive(Clone, Debug, Default)]
pub struct Disposicion {
    pub nodos: Vec<Colocado>,
}

impl Disposicion {
    /// Rectángulo de un nodo por índice.
    pub fn rect(&self, i: usize) -> Rect {
        self.nodos.get(i).map(|c| c.rect).unwrap_or(Rect::VACIO)
    }

    /// Índice del nodo más profundo que contiene el punto.
    ///
    /// Se recorre al revés porque el orden de dibujo es el preorden: los
    /// últimos hermanos y los hijos cubren a lo anterior.
    pub fn indice_bajo(&self, x: i32, y: i32) -> Option<usize> {
        self.nodos.iter().rposition(|c| c.rect.contiene(x, y))
    }

    /// Número de nodos del mapa.
    pub fn num_nodos(&self) -> usize {
        self.nodos.len()
    }
}

/// Nodo del mapa plano durante el cálculo (con referencia al árbol).
struct Plano<'a> {
    nodo: &'a Nodo,
    rect: Rect,
    hijos: Vec<usize>,
    padre: Option<usize>,
}

/// Calcula la disposición de un documento validado.
pub fn calcular(doc: &ValidatedDocument, restr: &Restricciones) -> Disposicion {
    // 1. Aplana el árbol en preorden y anota los rangos de hijos.
    let mut plano: Vec<Plano> = Vec::new();
    aplanar(&doc.raiz, &mut plano, None);
    let n = plano.len();

    // 2. Medida (de abajo arriba).
    let mut deseado: Vec<(u32, u32)> = alloc::vec![(0, 0); n];
    for i in (0..n).rev() {
        let d = medir(&plano, i, &deseado, restr);
        deseado[i] = d;
    }

    // 3. Colocación (de arriba abajo) desde la raíz en (0, 0).
    let (rw, rh) = deseado[0];
    colocar(
        &mut plano,
        0,
        Rect {
            x: 0,
            y: 0,
            w: rw,
            h: rh,
        },
        &deseado,
    );

    Disposicion {
        nodos: plano
            .into_iter()
            .map(|p| Colocado {
                def: p.nodo.def,
                rect: p.rect,
                indice_padre: p.padre,
                hijos: p.hijos,
            })
            .collect(),
    }
}

/// Igual que `calcular`, con el tamaño máximo en una sola llamada.
pub fn calcular_en(doc: &ValidatedDocument, max_ancho: u32, max_alto: u32) -> Disposicion {
    calcular(doc, &Restricciones::new(max_ancho, max_alto))
}

fn aplanar<'a>(nodo: &'a Nodo, out: &mut Vec<Plano<'a>>, padre: Option<usize>) -> usize {
    let i = out.len();
    out.push(Plano {
        nodo,
        rect: Rect::VACIO,
        hijos: Vec::new(),
        padre,
    });
    let mut hijos: Vec<usize> = Vec::with_capacity(nodo.hijos.len());
    for h in &nodo.hijos {
        hijos.push(aplanar(h, out, Some(i)));
    }
    out[i].hijos = hijos;
    i
}

/// Tamaño entero no negativo de una propiedad (`0` si falta o es negativo).
fn entero_no_negativo(n: &Nodo, nombre: &str) -> u32 {
    n.entero(nombre, 0).max(0) as u32
}

pub(super) const ALTO_BANDA_TITULO_BOX: u32 = 14;

/// Padding horizontal del contenedor; los boxes titulados garantizan 16 px.
fn padding_contenedor(n: &Nodo) -> u32 {
    let minimo = if n.def.etiqueta == "box" && !n.texto("title", "").is_empty() {
        16
    } else {
        0
    };
    n.prop("padding")
        .and_then(|v| v.entero())
        .map(|v| v.max(minimo) as u32)
        .unwrap_or(if n.def.etiqueta == "box" { 16 } else { 0 })
}

/// El título queda en una banda propia, por encima del marco del box.
fn padding_superior_contenedor(n: &Nodo) -> u32 {
    let padding = padding_contenedor(n);
    if n.def.etiqueta == "box" && !n.texto("title", "").is_empty() {
        padding.saturating_add(ALTO_BANDA_TITULO_BOX)
    } else {
        padding
    }
}

/// Posición absoluta declarada de un nodo (`x`/`y`), si la tiene.
///
/// Basta con declarar una de las dos: la que falte vale cero. Un nodo con
/// posición absoluta se coloca respecto al origen del CONTENIDO de su
/// contenedor (dentro del `padding`) y no entra en el flujo de `row`/`column`.
pub fn posicion_absoluta(n: &Nodo) -> Option<(i32, i32)> {
    if !PROPIEDADES_ABSOLUTAS.iter().any(|p| n.prop(*p).is_some()) {
        return None;
    }
    Some((n.entero("x", 0) as i32, n.entero("y", 0) as i32))
}

/// Paso 1: tamaño deseado del nodo (hijos ya medidos).
fn medir(plano: &[Plano], i: usize, deseado: &[(u32, u32)], restr: &Restricciones) -> (u32, u32) {
    let n = plano[i].nodo;
    if !n.booleano("visible", true) {
        return (0, 0);
    }
    let (mut w, mut h) = match n.def.etiqueta {
        "toby-app" | "window" | "column" | "row" | "stack" => {
            medir_contenedor(n.def.etiqueta, plano, &plano[i].hijos, deseado, n)
        }
        "box" => {
            let (w, h) = medir_contenedor(n.def.etiqueta, plano, &plano[i].hijos, deseado, n);
            let ancho_titulo = ancho_texto_css(n.texto("title", ""), 10).saturating_add(28);
            (w.max(ancho_titulo), h.max(32))
        }
        "button" => medir_boton(n),
        "label" => medir_etiqueta(n),
        "check" | "radio" => medir_casilla(n),
        // La cara del circular es la esfera del radio (682 : 653).
        "circular" => {
            let h = alto_de(n);
            (ancho_esfera(h), h)
        }
        "segmented" => medir_segmentado(n),
        "popup" => medir_popup(n),
        "progress" => (
            ANCHO_PROGRESS,
            if n.texto("mode", "indeterminate") == "determinate" {
                ALTO_PROGRESS_DETERMINADO
            } else {
                ALTO_PROGRESS
            },
        ),
        "field" | "slider" | "slider-gota" => (ANCHO_CONTROL, alto_de(n)),
        _ => (0, 0),
    };
    // Tamaño fijo explícito: manda sobre la medida del contenido.
    if let Some(v) = n.prop("width").and_then(|v| v.entero()) {
        if v > 0 {
            w = v as u32;
        }
    }
    if let Some(v) = n.prop("height").and_then(|v| v.entero()) {
        if v > 0 {
            h = v as u32;
        }
    }
    // Por defecto la ventana es el viewport de la app. En modo `window`, en
    // cambio, conserva el tamaño declarado y la raíz la centra en el host.
    if n.def.etiqueta == "window"
        && !ventana_en_modo_ventana(n)
        && restr.max_ancho != u32::MAX
        && restr.max_alto != u32::MAX
    {
        w = restr.max_ancho;
        h = restr.max_alto;
    }
    // Una app con una ventana explícita conserva el viewport como raíz, pero
    // su ventana hija mantiene el tamaño declarado y se centra en ese espacio.
    if n.def.etiqueta == "toby-app"
        && restr.max_ancho != u32::MAX
        && restr.max_alto != u32::MAX
        && plano[i]
            .hijos
            .iter()
            .any(|&hijo| ventana_en_modo_ventana(plano[hijo].nodo))
    {
        w = restr.max_ancho;
        h = restr.max_alto;
    }
    // Un control nunca baja de su ancho usable, ni siquiera declarándolo:
    // una pista de 1 px no sería ni pintable ni usable, y el lenguaje no
    // puede tumbar la interfaz.
    w = w.max(ancho_minimo(n.def.etiqueta));
    // Un nodo nunca puede declarar más de lo disponible.
    (w.min(restr.max_ancho), h.min(restr.max_alto))
}

/// Ancho mínimo usable de una hoja (0 = sin suelo).
fn ancho_minimo(etiqueta: &str) -> u32 {
    match etiqueta {
        "field" => 40,
        "progress" => 2,
        "slider" | "slider-gota" => 32,
        _ => 0,
    }
}

fn ventana_en_modo_ventana(n: &Nodo) -> bool {
    n.def.etiqueta == "window" && n.texto("mode", "fullscreen") == "window"
}

fn medir_contenedor(
    etiqueta: &str,
    plano: &[Plano],
    hijos: &[usize],
    deseado: &[(u32, u32)],
    n: &Nodo,
) -> (u32, u32) {
    let padding = padding_contenedor(n);
    let padding_superior = padding_superior_contenedor(n);
    let padding_inferior = padding;
    let spacing = entero_no_negativo(n, "spacing");
    let mut ancho_max: u32 = 0;
    let mut alto_max: u32 = 0;
    let mut ancho_total: u32 = 0;
    let mut alto_total: u32 = 0;
    let mut cuenta: u32 = 0;
    // Extensión máxima de los hijos con posición absoluta: no ocupan flujo,
    // pero el contenedor tiene que ser capaz de contenerlos.
    let mut ext_ancho: u32 = 0;
    let mut ext_alto: u32 = 0;
    for &i in hijos {
        let (w, h) = deseado[i];
        if let Some((x, y)) = posicion_absoluta(plano[i].nodo) {
            ext_ancho = ext_ancho.max((x.max(0) as u32).saturating_add(w));
            ext_alto = ext_alto.max((y.max(0) as u32).saturating_add(h));
            continue;
        }
        ancho_max = ancho_max.max(w);
        alto_max = alto_max.max(h);
        ancho_total = ancho_total.saturating_add(w);
        alto_total = alto_total.saturating_add(h);
        cuenta += 1;
    }
    if cuenta > 1 {
        let extra = spacing.saturating_mul(cuenta - 1);
        ancho_total = ancho_total.saturating_add(extra);
        alto_total = alto_total.saturating_add(extra);
    }
    let base = match etiqueta {
        // `row` crece en horizontal y toma la altura máxima de sus hijos.
        "row" => (ancho_total, alto_max),
        // `stack` superpone: toma el mayor de los dos ejes.
        "stack" => (ancho_max, alto_max),
        // `column`, `window` y `toby-app`: apilado vertical.
        _ => (ancho_max, alto_total),
    };
    let base = (base.0.max(ext_ancho), base.1.max(ext_alto));
    let doble_horizontal = padding.saturating_mul(2);
    let padding_vertical = padding_superior.saturating_add(padding_inferior);
    (
        base.0.saturating_add(doble_horizontal),
        base.1.saturating_add(padding_vertical),
    )
}

fn medir_boton(n: &Nodo) -> (u32, u32) {
    let alto = alto_de(n);
    let titulo = n.texto("title", "");
    let alto_fuente = (alto.saturating_mul(28) + 50) / 100;
    // Dos tapas (258:409) más el padding CSS de 0,42em en ambos lados.
    let tapas = alto.saturating_mul(258).saturating_mul(2) / 409;
    let padding = (alto.saturating_mul(84) + 50) / 100;
    (
        tapas
            .saturating_add(padding)
            .saturating_add(ancho_texto_css(titulo, alto_fuente)),
        alto,
    )
}

/// Etiqueta de texto plano: no hereda piel, foco ni geometría de botón.
fn medir_etiqueta(n: &Nodo) -> (u32, u32) {
    let texto = n.texto("text", "");
    let tamano = n.entero("size", 14).clamp(8, 32) as u32;
    let mut ancho = 0;
    let mut lineas = 0u32;
    for linea in texto.split('\n') {
        ancho = ancho.max(ancho_texto_css(linea, tamano));
        lineas = lineas.saturating_add(1);
    }
    (
        ancho,
        tamano.saturating_add(2).saturating_mul(lineas.max(1)),
    )
}

/// Ancho base de una opción AQUA (texto más padding CSS de 0,52em por lado).
pub fn ancho_item(items: &str, i: usize, alto: u32) -> u32 {
    let fuente = (alto.saturating_mul(42) + 50) / 100;
    let padding = (alto.saturating_mul(104) + 50) / 100;
    ancho_texto_css(component::item_en(items, i), fuente).saturating_add(padding)
}

/// `<check>` y `<radio>`: la marca a la izquierda y su etiqueta al lado.
fn medir_casilla(n: &Nodo) -> (u32, u32) {
    let alto = alto_de(n);
    let label = n.texto("label", "");
    // La casilla y la esfera no son cuadradas: cada una lleva su proporción
    // nativa (igual que las pinta el renderer).
    let marca = if n.def.etiqueta == "radio" {
        ancho_esfera(alto)
    } else {
        ancho_check(alto)
    };
    let ancho = marca
        .saturating_add(hueco_marca(alto))
        .saturating_add(ancho_texto_css(label, (alto.saturating_mul(62) + 50) / 100));
    (ancho, alto)
}

/// `<segmented>`: la suma de todas sus opciones más el marco.
fn medir_segmentado(n: &Nodo) -> (u32, u32) {
    let alto = alto_de(n);
    let items = n.texto("items", "");
    let cuenta = component::cuenta_items(items);
    let mut ancho = 0u32;
    for i in 0..cuenta {
        ancho = ancho.saturating_add(ancho_item(items, i, alto));
    }
    (ancho, alto)
}

/// `<popup>`: la opción más ancha más el hueco de la flecha.
fn medir_popup(n: &Nodo) -> (u32, u32) {
    let alto = alto_de(n);
    let items = n.texto("items", "");
    let mut ancho = 0u32;
    for i in 0..component::cuenta_items(items) {
        let texto = component::item_en(items, i);
        ancho = ancho.max(ancho_texto_css(texto, (alto.saturating_mul(42) + 50) / 100));
    }
    let cap = alto.saturating_mul(170) / 418;
    (
        ancho
            .saturating_add(cap.saturating_mul(2))
            .saturating_add(ancho_paso(alto)),
        alto,
    )
}

fn alto_de(n: &Nodo) -> u32 {
    n.prop("height")
        .and_then(|v| v.entero())
        .filter(|v| *v > 0)
        .map(|v| v.min(u32::MAX as i64) as u32)
        .unwrap_or_else(|| alto_css(n.def.etiqueta))
}

/// Paso 2: asigna rectángulos de arriba abajo.
fn colocar(plano: &mut [Plano], i: usize, rect: Rect, deseado: &[(u32, u32)]) {
    plano[i].rect = rect;
    if plano[i].hijos.is_empty() {
        return;
    }
    let n = plano[i].nodo;
    let hijos = plano[i].hijos.clone();
    let padding = padding_contenedor(n);
    let padding_superior = padding_superior_contenedor(n);
    let padding_vertical = padding_superior.saturating_add(padding);
    let doble_horizontal = padding.saturating_mul(2);
    let contenido = Rect {
        x: rect.x.saturating_add(padding as i32),
        y: rect.y.saturating_add(padding_superior as i32),
        w: rect.w.saturating_sub(doble_horizontal),
        h: rect.h.saturating_sub(padding_vertical),
    };
    let escala = if n.def.etiqueta == "stack" {
        escala_stack(n, rect, padding)
    } else {
        None
    };
    // Los hijos con `x`/`y` van en absoluto, respecto al contenido, y no
    // entran en el flujo. Un stack adaptable transforma el rectángulo lógico
    // aquí, de modo que pintura y aciertos de ratón usan las mismas medidas.
    let mut flujo: Vec<usize> = Vec::with_capacity(hijos.len());
    for hijo in hijos {
        match posicion_absoluta(plano[hijo].nodo) {
            Some((x, y)) => {
                if !plano[hijo].nodo.booleano("visible", true) {
                    continue;
                }
                let (dw, dh) = deseado[hijo];
                let (x, y, w, alto) = if let Some((factor, ox, oy)) = escala {
                    (
                        contenido.x + ox + escalar(x.max(0) as u32, factor) as i32,
                        contenido.y + oy + escalar(y.max(0) as u32, factor) as i32,
                        escalar(dw, factor),
                        escalar(dh, factor),
                    )
                } else {
                    (
                        contenido.x.saturating_add(x),
                        contenido.y.saturating_add(y),
                        dw,
                        dh,
                    )
                };
                colocar(plano, hijo, Rect { x, y, w, h: alto }, deseado);
            }
            None => flujo.push(hijo),
        }
    }
    match n.def.etiqueta {
        "row" => distribuir_en_fila(plano, flujo, contenido, deseado, n),
        "stack" => {
            // Solape en Z: cada hijo conserva su tamaño, todos en el mismo
            // origen (los que van después quedan encima).
            for h in flujo {
                if !plano[h].nodo.booleano("visible", true) {
                    continue;
                }
                let (dw, dh) = deseado[h];
                colocar(
                    plano,
                    h,
                    Rect {
                        x: contenido.x,
                        y: contenido.y,
                        w: dw,
                        h: dh,
                    },
                    deseado,
                );
            }
        }
        // `toby-app`, `window` y `column`: apilado vertical.
        _ => distribuir_en_columna(plano, flujo, contenido, deseado, n),
    }
}

fn distribuir_en_columna(
    plano: &mut [Plano],
    hijos: Vec<usize>,
    contenido: Rect,
    deseado: &[(u32, u32)],
    n: &Nodo,
) {
    let spacing = entero_no_negativo(n, "spacing");
    let alinear = Alineacion::del_nodo(n);
    let tamanos = repartir_eje(plano, &hijos, deseado, contenido.h, spacing, false);
    let mut y = contenido.y;
    for (hijo, alto_flexible) in tamanos {
        if !plano[hijo].nodo.booleano("visible", true) {
            continue;
        }
        let (dw, dh) = deseado[hijo];
        let ventana_flotante =
            n.def.etiqueta == "toby-app" && ventana_en_modo_ventana(plano[hijo].nodo);
        let alto_flexible = if ventana_flotante { dh } else { alto_flexible };
        let x = if ventana_flotante {
            contenido.x + mitad(contenido.w.saturating_sub(dw))
        } else {
            match alinear {
                Alineacion::Inicio | Alineacion::Estirar => contenido.x,
                Alineacion::Centro => contenido.x + mitad(contenido.w.saturating_sub(dw)),
                Alineacion::Fin => contenido.x + contenido.w.saturating_sub(dw) as i32,
            }
        };
        let y_pos = if ventana_flotante {
            contenido.y + mitad(contenido.h.saturating_sub(dh))
        } else {
            y
        };
        let ancho = if ventana_flotante {
            dw
        } else {
            match alinear {
                Alineacion::Estirar => contenido.w,
                _ => dw,
            }
        };
        colocar(
            plano,
            hijo,
            Rect {
                x,
                y: y_pos,
                w: ancho,
                h: alto_flexible,
            },
            deseado,
        );
        if !ventana_flotante {
            y += alto_flexible as i32 + spacing as i32;
        }
    }
}

fn distribuir_en_fila(
    plano: &mut [Plano],
    hijos: Vec<usize>,
    contenido: Rect,
    deseado: &[(u32, u32)],
    n: &Nodo,
) {
    let spacing = entero_no_negativo(n, "spacing");
    let alinear = Alineacion::del_nodo(n);
    let tamanos = repartir_eje(plano, &hijos, deseado, contenido.w, spacing, true);
    let mut x = contenido.x;
    for (hijo, ancho_flexible) in tamanos {
        if !plano[hijo].nodo.booleano("visible", true) {
            continue;
        }
        let (_, dh) = deseado[hijo];
        let y = match alinear {
            Alineacion::Inicio | Alineacion::Estirar => contenido.y,
            Alineacion::Centro => contenido.y + mitad(contenido.h.saturating_sub(dh)),
            Alineacion::Fin => contenido.y + contenido.h.saturating_sub(dh) as i32,
        };
        let alto = match alinear {
            Alineacion::Estirar => contenido.h,
            _ => dh,
        };
        colocar(
            plano,
            hijo,
            Rect {
                x,
                y,
                w: ancho_flexible,
                h: alto,
            },
            deseado,
        );
        x += ancho_flexible as i32 + spacing as i32;
    }
}

/// Reparte el eje principal con crecimiento ponderado y contracción cuando
/// el contenido no cabe. Los nodos sin `grow` mantienen su tamaño si sobra
/// espacio; si falta, participan en la contracción para que el layout siga
/// dentro de la ventana.
fn repartir_eje(
    plano: &[Plano],
    hijos: &[usize],
    deseado: &[(u32, u32)],
    disponible: u32,
    spacing: u32,
    horizontal: bool,
) -> Vec<(usize, u32)> {
    let visibles: Vec<usize> = hijos
        .iter()
        .copied()
        .filter(|&i| plano[i].nodo.booleano("visible", true))
        .collect();
    if visibles.is_empty() {
        return Vec::new();
    }
    let huecos = spacing.saturating_mul(visibles.len().saturating_sub(1) as u32);
    let espacio_hijos = disponible.saturating_sub(huecos);
    let base: Vec<u32> = visibles
        .iter()
        .map(|&i| {
            let (w, h) = deseado[i];
            if horizontal {
                w
            } else {
                h
            }
        })
        .collect();
    let total_base: u64 = base.iter().map(|&v| v as u64).sum();
    let mut resultado = base.clone();

    if total_base < espacio_hijos as u64 {
        let pesos: Vec<u64> = visibles
            .iter()
            .map(|&i| plano[i].nodo.entero("grow", 0).max(0) as u64)
            .collect();
        let total_peso: u64 = pesos.iter().fold(0u64, |suma, p| suma.saturating_add(*p));
        if total_peso != 0 {
            let extra = espacio_hijos as u64 - total_base;
            let mut usado = 0u64;
            let mut ultimo = None;
            for (i, peso) in pesos.iter().enumerate() {
                if *peso != 0 {
                    ultimo = Some(i);
                }
            }
            for i in 0..resultado.len() {
                if pesos[i] == 0 {
                    continue;
                }
                let parte = if Some(i) == ultimo {
                    extra - usado
                } else {
                    extra.saturating_mul(pesos[i]) / total_peso
                };
                resultado[i] = resultado[i].saturating_add(parte as u32);
                usado = usado.saturating_add(parte);
            }
        }
    } else if total_base > espacio_hijos as u64 {
        let mut usado = 0u32;
        let ultimo = resultado.len().saturating_sub(1);
        for (i, tamano) in resultado.iter_mut().enumerate() {
            *tamano = if i == ultimo {
                espacio_hijos.saturating_sub(usado)
            } else {
                (espacio_hijos as u64 * base[i] as u64 / total_base) as u32
            };
            usado = usado.saturating_add(*tamano);
        }
    }

    visibles.into_iter().zip(resultado).collect()
}

/// Factor de escala uniforme de un `<stack responsive="scale">` y los
/// márgenes que centran su lienzo lógico dentro del rectángulo disponible.
fn escala_stack(n: &Nodo, rect: Rect, padding: u32) -> Option<(u32, i32, i32)> {
    if n.texto("responsive", "none") != "scale" {
        return None;
    }
    let base_w = n.entero("width", 0).max(0) as u32;
    let base_h = n.entero("height", 0).max(0) as u32;
    let ancho = rect.w.saturating_sub(padding.saturating_mul(2));
    let alto = rect.h.saturating_sub(padding.saturating_mul(2));
    if base_w == 0 || base_h == 0 || ancho == 0 || alto == 0 {
        return None;
    }
    const UNIDAD: u32 = 1024;
    let factor = ((ancho as u64 * UNIDAD as u64 / base_w as u64)
        .min(alto as u64 * UNIDAD as u64 / base_h as u64))
    .clamp(1, u32::MAX as u64) as u32;
    let escalado_w = escalar(base_w, factor);
    let escalado_h = escalar(base_h, factor);
    Some((
        factor,
        (ancho.saturating_sub(escalado_w) / 2) as i32,
        (alto.saturating_sub(escalado_h) / 2) as i32,
    ))
}

fn escalar(valor: u32, factor: u32) -> u32 {
    ((valor as u64 * factor as u64) / 1024).min(u32::MAX as u64) as u32
}

/// Mitad redondeada hacia arriba, sin depender de `div_ceil`.
fn mitad(v: u32) -> i32 {
    (v as i32 + 1) / 2
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::toby_ui::parser::analizar;
    use crate::toby_ui::validation::validar;

    fn preparar(src: &str) -> ValidatedDocument {
        let d = analizar(src).expect("parser");
        validar(&d).expect("validación")
    }

    /// Tamaño intrínseco de botón usado para comprobar las reglas de flujo.
    fn boton(x: i32, y: i32, titulo: &str, alto: u32) -> Rect {
        let caps = alto.saturating_mul(258).saturating_mul(2) / 409;
        let padding = (alto.saturating_mul(84) + 50) / 100;
        let fuente = (alto.saturating_mul(28) + 50) / 100;
        Rect {
            x,
            y,
            w: caps + padding + ancho_texto_css(titulo, fuente),
            h: alto,
        }
    }

    const APP: &str = "<toby-app name=\"demo\">";

    #[test]
    fn columna_basica() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window title=\"Hola\" width=\"320\" height=\"240\">",
            "<column>",
            "<button id=\"a\" title=\"Aceptar\"/>",
            "<button id=\"b\" title=\"Cancelar\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        // 0 app, 1 window, 2 column, 3 a, 4 b
        assert_eq!(
            disp.rect(0),
            Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 240
            }
        );
        assert_eq!(
            disp.rect(1),
            Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 240
            }
        );
        assert_eq!(disp.rect(3), boton(0, 0, "Aceptar", 62));
        assert_eq!(disp.rect(4), boton(0, 62, "Cancelar", 62));
        let _ = APP;
    }

    #[test]
    fn progress_tiene_medida_intrinseca_aqua() {
        let doc = preparar(
            "<toby-app name=\"demo\"><window width=\"240\" height=\"40\">\
               <progress id=\"cargando\"/>\
             </window></toby-app>",
        );
        let disp = calcular_en(&doc, 240, 40);
        assert_eq!(
            disp.rect(2),
            Rect {
                x: 0,
                y: 0,
                w: ANCHO_PROGRESS,
                h: ALTO_PROGRESS,
            }
        );
    }

    #[test]
    fn progress_determinado_conserva_alto_nativo_de_la_textura() {
        let doc = preparar(
            "<toby-app name=\"demo\"><window width=\"240\" height=\"40\">\
               <progress id=\"instalacion\" mode=\"determinate\" value=\"68\"/>\
             </window></toby-app>",
        );
        let disp = calcular_en(&doc, 240, 40);
        assert_eq!(
            disp.rect(2),
            Rect {
                x: 0,
                y: 0,
                w: ANCHO_PROGRESS,
                h: ALTO_PROGRESS_DETERMINADO,
            }
        );
    }

    #[test]
    fn padding_y_spacing() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column spacing=\"4\" padding=\"10\">",
            "<button id=\"a\" title=\"Aceptar\"/>",
            "<button id=\"b\" title=\"Cancelar\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(disp.rect(3), boton(10, 10, "Aceptar", 62));
        // 10 + 62 (alto del botón) + 4 (spacing) = 76.
        assert_eq!(disp.rect(4), boton(10, 76, "Cancelar", 62));
    }

    #[test]
    fn las_cajas_anidadas_reservan_padding_y_se_miden_con_sus_hijos() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\"><window width=\"400\" height=\"300\">",
            "<column><box id=\"externa\" title=\"Cuenta\" spacing=\"8\">",
            "<button id=\"perfil\" title=\"Perfil\"/>",
            "<box id=\"interna\" title=\"Privacidad\">",
            "<button id=\"permisos\" title=\"Permisos\"/>",
            "</box></box></column></window></toby-app>"
        ));
        let disp = calcular_en(&doc, 400, 300);
        let externa = disp.rect(3);
        let perfil = disp.rect(4);
        let interna = disp.rect(5);
        let permisos = disp.rect(6);

        assert_eq!(perfil.x, externa.x + 16);
        assert_eq!(perfil.y, externa.y + 30);
        assert_eq!(interna.x, externa.x + 16);
        assert_eq!(interna.y, perfil.abajo() + 8);
        assert_eq!(permisos.x, interna.x + 16);
        assert_eq!(permisos.y, interna.y + 30);
        assert!(interna.abajo() <= externa.abajo() - 16);
    }

    #[test]
    fn align_centro() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column width=\"200\" align=\"center\">",
            "<button id=\"a\" title=\"Aceptar\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        let esperado = boton(0, 0, "Aceptar", 62);
        assert_eq!(
            disp.rect(3),
            Rect {
                x: (200 - esperado.w as i32 + 1) / 2,
                ..esperado
            }
        );
    }

    #[test]
    fn align_estirar() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column width=\"200\" align=\"stretch\">",
            "<button id=\"a\" title=\"Aceptar\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(
            disp.rect(3),
            Rect {
                x: 0,
                y: 0,
                w: 200,
                h: 62
            }
        );
    }

    #[test]
    fn grow_reparte_el_espacio_vertical_sobrante() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\" align=\"stretch\">",
            "<column padding=\"10\" spacing=\"10\" align=\"stretch\" grow=\"1\">",
            "<button id=\"a\" title=\"A\"/>",
            "<button id=\"b\" title=\"B\" grow=\"1\"/>",
            "</column></window></toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(
            disp.rect(2),
            Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 240
            }
        );
        assert_eq!(
            disp.rect(3),
            Rect {
                x: 10,
                y: 10,
                w: 300,
                h: 62
            }
        );
        assert_eq!(
            disp.rect(4),
            Rect {
                x: 10,
                y: 82,
                w: 300,
                h: 148
            }
        );
    }

    #[test]
    fn stack_responsive_escala_y_centra_los_elementos_posicionados() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"200\" height=\"120\" align=\"stretch\">",
            "<stack width=\"200\" height=\"120\" grow=\"1\" responsive=\"scale\">",
            "<button id=\"a\" title=\"OK\" height=\"20\" x=\"20\" y=\"10\"/>",
            "</stack></window></toby-app>"
        ));
        let disp = calcular_en(&doc, 400, 300);
        let boton = disp.rect(3);
        assert_eq!(
            disp.rect(2),
            Rect {
                x: 0,
                y: 0,
                w: 400,
                h: 300
            }
        );
        assert_eq!(boton.x, 40);
        assert_eq!(boton.y, 50);
        assert_eq!(boton.h, 40);
        assert_eq!(boton.w, super::medir_boton(&doc.por_id("a").unwrap()).0 * 2);
    }

    #[test]
    fn fila_basica() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<row spacing=\"8\">",
            "<button id=\"a\" title=\"A\"/>",
            "<button id=\"b\" title=\"BB\"/>",
            "</row>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        let a = boton(0, 0, "A", 62);
        let bb = boton(a.w as i32 + 8, 0, "BB", 62);
        assert_eq!(disp.rect(3), a);
        assert_eq!(disp.rect(4), bb);
    }

    #[test]
    fn fila_align_fin() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<row height=\"40\" align=\"end\">",
            "<button id=\"a\" title=\"A\"/>",
            "</row>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        let esperado = boton(0, 0, "A", 62);
        assert_eq!(disp.rect(3), Rect { y: 0, ..esperado });
    }

    #[test]
    fn stack_superpone() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<stack width=\"100\" height=\"60\">",
            "<button id=\"a\" title=\"A\" height=\"24\"/>",
            "<button id=\"b\" title=\"B\" height=\"24\"/>",
            "</stack>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(disp.rect(3), boton(0, 0, "A", 24));
        assert_eq!(disp.rect(4), boton(0, 0, "B", 24));
        // El segundo hermano está encima: es el que "gana" el punto.
        assert_eq!(disp.indice_bajo(10, 10), Some(4));
    }

    #[test]
    fn invisible_no_ocupa() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column>",
            "<button id=\"a\" title=\"A\" visible=\"false\"/>",
            "<button id=\"b\" title=\"B\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(disp.rect(3), Rect::VACIO);
        // El segundo empieza en el origen: el invisible no deja hueco.
        assert_eq!(disp.rect(4), boton(0, 0, "B", 62));
    }

    #[test]
    fn restricciones_limitan() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"640\" height=\"480\">",
            "<column/>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(
            disp.rect(1),
            Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 240
            }
        );
    }

    #[test]
    fn la_ventana_llena_el_area_del_host() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"480\" height=\"300\">",
            "<column padding=\"20\"/>",
            "</window>",
            "</toby-app>"
        ));
        // El host manda: la ventana declara 480x300 pero ocupa la pantalla
        // entera (1024x768) y la raiz, que es el area de la aplicacion, tambien.
        let disp = calcular_en(&doc, 1024, 768);
        assert_eq!(
            disp.rect(0),
            Rect {
                x: 0,
                y: 0,
                w: 1024,
                h: 768
            }
        );
        assert_eq!(
            disp.rect(1),
            Rect {
                x: 0,
                y: 0,
                w: 1024,
                h: 768
            }
        );
        // Los contenedores NO se estiran: dentro, la columna sigue midiendo lo
        // que declara su contenido (20 de padding por los cuatro lados).
        assert_eq!(
            disp.rect(2),
            Rect {
                x: 0,
                y: 0,
                w: 40,
                h: 40
            }
        );
        // El host mas pequeno que lo declarado acota: manda el area real.
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(
            disp.rect(1),
            Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 240
            }
        );
        // Sin host (medida interna) sigue ganando lo declarado por el documento.
        let disp = calcular_en(&doc, u32::MAX, u32::MAX);
        assert_eq!(
            disp.rect(1),
            Rect {
                x: 0,
                y: 0,
                w: 480,
                h: 300
            }
        );
    }

    #[test]
    fn ventana_modo_window_se_centra_y_conserva_su_tamano() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"640\" height=\"420\" mode=\"window\">",
            "<column padding=\"34\"><button id=\"a\" title=\"Abrir\"/></column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 1024, 768);
        assert_eq!(
            disp.rect(0),
            Rect {
                x: 0,
                y: 0,
                w: 1024,
                h: 768
            }
        );
        assert_eq!(
            disp.rect(1),
            Rect {
                x: 192,
                y: 174,
                w: 640,
                h: 420
            }
        );
        assert_eq!(disp.rect(3).x, 226);
        assert_eq!(disp.rect(3).y, 208);
    }

    #[test]
    fn determinista() {
        let f = |src: &str| {
            let doc = preparar(src);
            calcular_en(&doc, 320, 240)
                .nodos
                .iter()
                .map(|c| c.rect)
                .collect::<alloc::vec::Vec<Rect>>()
        };
        let src = concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column spacing=\"6\" padding=\"12\" align=\"center\">",
            "<button id=\"a\" title=\"Aceptar\"/>",
            "<row spacing=\"3\">",
            "<button id=\"b\" title=\"Cancelar\"/>",
            "<button id=\"c\" title=\"OK\"/>",
            "</row>",
            "</column>",
            "</window>",
            "</toby-app>"
        );
        assert_eq!(f(src), f(src));
    }

    #[test]
    fn preorden_coincide_con_layout() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column><button id=\"a\" title=\"A\"/></column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        let pre = doc.preorden();
        assert_eq!(pre.len(), disp.num_nodos());
        for (i, n) in pre.iter().enumerate() {
            assert_eq!(n.def.etiqueta, disp.nodos[i].def.etiqueta);
        }
    }

    #[test]
    fn indice_bajo_devuelve_el_mas_profundo() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column>",
            "<button id=\"a\" title=\"A\" height=\"24\"/>",
            "<button id=\"b\" title=\"B\" height=\"24\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(disp.indice_bajo(5, 5), Some(3));
        assert_eq!(disp.indice_bajo(5, 30), Some(4));
        // Punto fuera de la ventana.
        assert_eq!(disp.indice_bajo(1000, 1000), None);
    }

    #[test]
    fn posicion_absoluta_en_lienzo() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            // Un `stack` con hojas posicionadas es un lienzo libre.
            "<stack width=\"320\" height=\"240\">",
            "<button id=\"a\" title=\"A\" x=\"40\" y=\"20\"/>",
            "<button id=\"b\" title=\"B\" x=\"200\" y=\"180\"/>",
            "</stack>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(disp.rect(3), boton(40, 20, "A", 62));
        assert_eq!(disp.rect(4), boton(200, 180, "B", 62));
        // El acierto de ratón usa los rectángulos absolutos.
        assert_eq!(disp.indice_bajo(50, 30), Some(3));
        assert_eq!(disp.indice_bajo(210, 190), Some(4));
        assert_eq!(disp.indice_bajo(100, 100), Some(2)); // el propio stack
    }

    #[test]
    fn absoluta_no_consume_flujo() {
        // En una columna, las hojas con `x`/`y` no empujan a las del flujo.
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column padding=\"10\">",
            "<button id=\"abs\" title=\"Libre\" x=\"100\" y=\"50\"/>",
            "<button id=\"flujo\" title=\"Flujo\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        // Origen del contenido = (10, 10); el absoluto suma su (x, y).
        assert_eq!(disp.rect(3), boton(110, 60, "Libre", 62));
        // El flujo no se desplaza: sigue siendo el primer hijo en flujo.
        assert_eq!(disp.rect(4), boton(10, 10, "Flujo", 62));
    }

    #[test]
    fn absoluta_agranda_el_contenedor_medido() {
        // Sin tamaño explícito, el contenedor crece hasta cubrir al absoluto.
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<stack>",
            "<button id=\"a\" title=\"A\" x=\"100\" y=\"50\"/>",
            "</stack>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        let control = boton(100, 50, "A", 62);
        assert_eq!(
            disp.rect(2),
            Rect {
                x: 0,
                y: 0,
                w: control.x as u32 + control.w,
                h: control.y as u32 + control.h
            }
        );
        assert_eq!(disp.rect(3), control);
    }

    #[test]
    fn solo_y_basta_para_ser_absoluta() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<stack width=\"320\" height=\"240\">",
            "<button id=\"a\" title=\"A\" y=\"12\"/>",
            "<button id=\"b\" title=\"B\"/>",
            "</stack>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 320, 240);
        assert_eq!(disp.rect(3), boton(0, 12, "A", 62));
        // La que no declara nada sigue en el origen (superpuesta).
        assert_eq!(disp.rect(4), boton(0, 0, "B", 62));
    }

    // --- F10: las ocho hojas nuevas miden como sus CSS del kit AQUA ---

    /// Las ocho, colocado cada una con `x`/`y` para que no interfieran.
    const CONTROLES: &str = concat!(
        "<toby-app name=\"demo\">",
        "<window width=\"480\" height=\"400\">",
        "<stack width=\"480\" height=\"400\">",
        "<check id=\"c\" x=\"0\" y=\"0\" label=\"Recordar\"/>",
        "<radio id=\"r\" x=\"0\" y=\"40\" label=\"Opcion\"/>",
        "<circular id=\"k\" x=\"0\" y=\"80\" title=\"1\"/>",
        "<segmented id=\"s\" x=\"0\" y=\"120\" items=\"Pequeño|Mediano|Grande\"/>",
        "<popup id=\"p\" x=\"0\" y=\"160\" items=\"Rojo|Verde|Azul\"/>",
        "<field id=\"f\" x=\"0\" y=\"200\"/>",
        "<slider id=\"l\" x=\"0\" y=\"240\"/>",
        "<slider-gota id=\"g\" x=\"0\" y=\"280\"/>",
        "</stack>",
        "</window>",
        "</toby-app>"
    );

    #[test]
    fn medidas_de_los_ocho_controles() {
        let doc = preparar(CONTROLES);
        let disp = calcular_en(&doc, 480, 400);
        // check/radio: dimensiones AQUA predeterminadas de 26 px, con la
        // proporción de sus piezas, el hueco CSS y el texto escalado.
        let alto_marca = 26;
        let alto_texto = (alto_marca * 62 + 50) / 100;
        assert_eq!(
            disp.rect(3),
            Rect {
                x: 0,
                y: 0,
                w: ancho_check(alto_marca)
                    + hueco_marca(alto_marca)
                    + ancho_texto_css("Recordar", alto_texto),
                h: alto_marca,
            }
        );
        assert_eq!(ancho_check(18), 18, "18 x 635/649 sigue siendo 18");
        // radio: la esfera (682:653), un pelo más ancha que alta.
        assert_eq!(
            disp.rect(4).w,
            ancho_esfera(alto_marca)
                + hueco_marca(alto_marca)
                + ancho_texto_css("Opcion", alto_texto)
        );
        assert_eq!(disp.rect(4).h, alto_marca);
        assert_eq!(ancho_esfera(18), 19);
        // circular: la cara del kit, esfera de 32.
        assert_eq!((disp.rect(5).w, disp.rect(5).h), (ancho_esfera(26), 26));
        // segmented: 32 px, con fuentes y márgenes a escala.
        let items = "Pequeño|Mediano|Grande";
        let suma: u32 = (0..component::cuenta_items(items))
            .map(|i| ancho_item(items, i, 32))
            .sum();
        assert_eq!(disp.rect(6).w, suma);
        assert_eq!(disp.rect(6).h, 32);
        // popup: texto más extremos nativos y botón de 1,3 × la altura.
        let popup_items = "Rojo|Verde|Azul";
        let fuente_popup = (32 * 42 + 50) / 100;
        let mas_ancha = popup_items
            .split('|')
            .map(|it| ancho_texto_css(it, fuente_popup))
            .max()
            .unwrap_or(0);
        let cap = 32 * 170 / 418;
        assert_eq!(ancho_paso(32), 42);
        assert_eq!(disp.rect(7).w, mas_ancha + cap * 2 + ancho_paso(32));
        assert_eq!(disp.rect(7).h, 32);
        // field y sliders comparten los 240 × 26 px del CSS.
        assert_eq!((disp.rect(8).w, disp.rect(8).h), (ANCHO_CONTROL, 26));
        assert_eq!((disp.rect(9).w, disp.rect(9).h), (ANCHO_CONTROL, 26));
        assert_eq!((disp.rect(10).w, disp.rect(10).h), (ANCHO_CONTROL, 26));
    }

    #[test]
    fn los_controles_con_width_propio_lo_respetan() {
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"480\" height=\"240\">",
            "<stack width=\"480\" height=\"240\">",
            "<field id=\"f\" x=\"0\" y=\"0\" width=\"200\"/>",
            "<slider id=\"l\" x=\"0\" y=\"40\" width=\"300\"/>",
            "</stack>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 480, 240);
        assert_eq!(disp.rect(3).w, 200);
        assert_eq!(disp.rect(4).w, 300);
        // Un ancho ridículo se acota al mínimo usable de cada control.
        let doc = preparar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"480\" height=\"240\">",
            "<stack width=\"480\" height=\"240\">",
            "<field id=\"f\" x=\"0\" y=\"0\" width=\"1\"/>",
            "<slider id=\"l\" x=\"0\" y=\"40\" width=\"1\"/>",
            "</stack>",
            "</window>",
            "</toby-app>"
        ));
        let disp = calcular_en(&doc, 480, 240);
        assert_eq!(disp.rect(3).w, 40, "el field no baja de 40");
        assert_eq!(disp.rect(4).w, 32, "el slider no baja de 32");
    }
}
