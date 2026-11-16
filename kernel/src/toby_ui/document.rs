//! Modelo de documento de Toby UI.
//!
//! Hay DOS representaciones, a propósito:
//!
//!   * `Document` / `Sintaxis`  · lo que devuelve el parser, en crudo (cadenas).
//!   * `ValidatedDocument` / `Nodo` · el árbol TIPADO que consume el layout,
//!     el tema y el renderer. Sólo se llega aquí pasando por `validation`.
//!
//! El documento original nunca se ejecuta: sólo se describe.

use alloc::string::String;
use alloc::vec::Vec;

use super::component::ComponentDef;

/// Valor de una propiedad ya interpretado (tipado).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PropValue {
    Int(i64),
    Bool(bool),
    Str(String),
    /// Valor de un conjunto cerrado (p.ej. "start").
    Enum(String),
    /// Color #RRGGBB ya convertido.
    Color(u32),
}

impl PropValue {
    /// Nombre del tipo, para mensajes de error.
    pub fn tipo(&self) -> &'static str {
        match self {
            PropValue::Int(_) => "entero",
            PropValue::Bool(_) => "booleano",
            PropValue::Str(_) => "texto",
            PropValue::Enum(_) => "valor de lista",
            PropValue::Color(_) => "color #RRGGBB",
        }
    }

    pub fn entero(&self) -> Option<i64> {
        match self {
            PropValue::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn booleano(&self) -> Option<bool> {
        match self {
            PropValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn texto(&self) -> Option<&str> {
        match self {
            PropValue::Str(v) | PropValue::Enum(v) => Some(v.as_str()),
            _ => None,
        }
    }
}

/// Nodo crudo: lo que produce el parser, sin tocar.
#[derive(Debug)]
pub struct Sintaxis {
    /// Etiqueta tal cual aparece, p.ej. "button".
    pub etiqueta: String,
    /// Línea (1-based) donde empieza la etiqueta, para los mensajes de error.
    pub linea: usize,
    /// Atributos en crudo (nombre, valor textual).
    pub atributos: Vec<(String, String)>,
    /// Hijos, en orden.
    pub hijos: Vec<Sintaxis>,
}

/// Documento crudo completo.
#[derive(Debug)]
pub struct Document {
    /// Nombre de la aplicación (atributo `name` del `toby-app`).
    ///
    /// El parser no lo interpreta: sale del árbol de atributos al validar
    /// (`validation::validar` lo copia al `ValidatedDocument`).
    pub nombre: String,
    /// Raíz del árbol. Siempre `toby-app`.
    pub raiz: Sintaxis,
}

/// Nodo ya validado y tipado.
#[derive(Debug)]
pub struct Nodo {
    /// Definición del componente (tabla central de `component.rs`).
    pub def: &'static ComponentDef,
    /// Línea de origen, para diagnósticos.
    pub linea: usize,
    /// Identificador declarado, si lo había.
    pub id: Option<String>,
    /// Propiedades ya convertidas a su tipo.
    pub props: Vec<(String, PropValue)>,
    /// Hijos validados.
    pub hijos: Vec<Nodo>,
}

impl Nodo {
    /// Lee una propiedad por nombre.
    pub fn prop(&self, nombre: &str) -> Option<&PropValue> {
        self.props
            .iter()
            .find(|(k, _)| k.as_str() == nombre)
            .map(|(_, v)| v)
    }

    /// El `name` declarado en el elemento (como en HTML), si lo tiene.
    ///
    /// Es una propiedad normal (`PROP_NOMBRE`), así que no ocupa campo
    /// propio: se lee de la lista ya convertida.
    pub fn nombre(&self) -> Option<&str> {
        self.prop("name").and_then(|v| v.texto())
    }

    /// Entero con valor por defecto.
    pub fn entero(&self, nombre: &str, defecto: i64) -> i64 {
        self.prop(nombre)
            .and_then(|v| v.entero())
            .unwrap_or(defecto)
    }

    /// Booleano con valor por defecto.
    pub fn booleano(&self, nombre: &str, defecto: bool) -> bool {
        self.prop(nombre)
            .and_then(|v| v.booleano())
            .unwrap_or(defecto)
    }

    /// Texto con valor por defecto.
    pub fn texto<'a>(&'a self, nombre: &str, defecto: &'a str) -> &'a str {
        self.prop(nombre).and_then(|v| v.texto()).unwrap_or(defecto)
    }
}

/// Documento validado: la única forma de llegar a layout y render.
#[derive(Debug)]
pub struct ValidatedDocument {
    /// Nombre de la aplicación.
    pub nombre: String,
    /// Raíz (siempre el `toby-app`).
    pub raiz: Nodo,
}

impl ValidatedDocument {
    /// Busca un nodo por `id` (recorrido en profundidad, preorden).
    pub fn por_id(&self, id: &str) -> Option<&Nodo> {
        buscar_id(&self.raiz, id)
    }

    /// Número total de nodos del árbol.
    pub fn num_nodos(&self) -> usize {
        contar(&self.raiz)
    }

    /// Nodos del árbol en recorrido en profundidad (preorden).
    ///
    /// El orden es el mismo que el de `layout::Disposicion::nodos`: el índice
    /// de un nodo aquí coincide con su índice allí. Útil para pasar del mapa
    /// plano de rectángulos de vuelta a las propiedades del nodo.
    pub fn preorden(&self) -> Vec<&Nodo> {
        let mut out = Vec::new();
        apilar(&self.raiz, &mut out);
        out
    }

    /// Índice en preorden del nodo con ese `id`, si existe.
    ///
    /// Es el índice que usan a la vez `layout::Disposicion`,
    /// `event::Interaccion` y `action::Peticion`, así que permite pasar de un
    /// `id` declarado en el documento a su rectángulo, su estado o su acción.
    pub fn indice_de(&self, id: &str) -> Option<usize> {
        let mut siguiente = 0;
        indice_id(&self.raiz, id, &mut siguiente)
    }

    /// Nodo en la posición `i` del recorrido en profundidad, si existe.
    ///
    /// El mismo orden que [`preorden`], pero sin construir la lista entera:
    /// sólo se recorre contando. Lo usan los gestos de `app`, que necesitan
    /// las propiedades de un nodo concreto en cada evento.
    pub fn nodo_en(&self, i: usize) -> Option<&Nodo> {
        let mut cont = 0usize;
        contar_en(&self.raiz, i, &mut cont)
    }
}

fn indice_id(nodo: &Nodo, id: &str, siguiente: &mut usize) -> Option<usize> {
    let indice = *siguiente;
    if nodo.id.as_deref() == Some(id) {
        return Some(indice);
    }
    *siguiente = siguiente.saturating_add(1);
    for hijo in &nodo.hijos {
        if let Some(indice) = indice_id(hijo, id, siguiente) {
            return Some(indice);
        }
    }
    None
}

fn contar_en<'a>(nodo: &'a Nodo, i: usize, cont: &mut usize) -> Option<&'a Nodo> {
    if *cont == i {
        return Some(nodo);
    }
    *cont += 1;
    for h in &nodo.hijos {
        if let Some(n) = contar_en(h, i, cont) {
            return Some(n);
        }
    }
    None
}

fn apilar<'a>(n: &'a Nodo, out: &mut Vec<&'a Nodo>) {
    out.push(n);
    for h in &n.hijos {
        apilar(h, out);
    }
}

fn buscar_id<'a>(nodo: &'a Nodo, id: &str) -> Option<&'a Nodo> {
    if nodo.id.as_deref() == Some(id) {
        return Some(nodo);
    }
    for hijo in &nodo.hijos {
        if let Some(n) = buscar_id(hijo, id) {
            return Some(n);
        }
    }
    None
}

fn contar(nodo: &Nodo) -> usize {
    1 + nodo.hijos.iter().map(contar).sum::<usize>()
}
