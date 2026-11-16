//! Valores en tiempo de ejecución de los componentes de estado (Fase 10).
//!
//! Toby UI sigue sin tener variables ni memoria de programa: el documento
//! sólo declara. Lo que sí necesita una interfaz real es que, mientras la
//! ventana está abierta, el usuario marque un `check`, elija una opción de un
//! `segmented`, escriba en un `field` o mueva un `slider`. Ese estado vive
//! AQUÍ, en la aplicación, y no en el documento:
//!
//!   * el `.toby` declara el valor INICIAL (`checked="true"`, `value="12"`);
//!   * [`Valores`] guarda el valor ACTUAL, mientras dure la sesión.
//!
//! Es un vector plano por índice de nodo (preorden, igual que
//! [`super::layout::Disposicion`]): una celda por nodo, sin claves ni
//! consultas. Un nodo que no es de estado usa su celda de pasillo.

use alloc::string::String;
use alloc::vec::Vec;

use super::component;
use super::document::{Nodo, ValidatedDocument};

/// Rango (`min`, `max`) de un `<slider>`, garantizando `min < max`.
///
/// Un slider con `min >= max` no tendría valores posibles, así que se le
/// impone un paso mínimo de 1: el lenguaje no puede tumbar el pintado con un
/// par de números mal puestos.
pub fn rango_slider(n: &Nodo) -> (i64, i64) {
    let min = n.entero("min", 0);
    let max = n.entero("max", 100);
    if max > min {
        (min, max)
    } else {
        (min, min + 1)
    }
}

/// Valor actual de un componente, por índice de nodo.
#[derive(Clone, Debug, Default)]
pub struct Celda {
    /// `check`/`radio`: ¿está marcado?
    pub marcado: bool,
    /// `segmented`/`popup`: opción elegida; `slider`: valor dentro de su rango.
    pub entero: i64,
    /// `field`: contenido del texto.
    pub texto: String,
    /// `popup`: ¿está desplegado su menú?
    pub abierto: bool,
    /// `slider`: valor al empezar el arrastre, para saber si hubo cambio.
    pub trasiego: Option<i64>,
}

impl Celda {
    /// Rellena la celda con el valor inicial que declara el documento.
    fn inicial(n: &Nodo) -> Celda {
        match n.def.etiqueta {
            "check" | "radio" => Celda {
                marcado: n.booleano("checked", false),
                ..Celda::default()
            },
            "segmented" | "popup" => Celda {
                entero: component::acotar_item(n.texto("items", ""), n.entero("selected", 0)),
                ..Celda::default()
            },
            "slider" | "slider-gota" => {
                let (min, max) = rango_slider(n);
                // <input type="range"> nace en el punto medio si no declara
                // value; AQUA usa el mismo comportamiento que el HTML.
                let medio = ((min as i128 + max as i128) / 2) as i64;
                Celda {
                    entero: n.entero("value", medio).clamp(min, max),
                    ..Celda::default()
                }
            }
            "progress" => Celda {
                entero: n.entero("value", 0).clamp(0, 100),
                ..Celda::default()
            },
            "field" => Celda {
                texto: String::from(n.texto("value", "")),
                ..Celda::default()
            },
            _ => Celda::default(),
        }
    }
}

/// Los valores actuales de todos los nodos de un documento.
///
/// Se crea al montar la aplicación y se conserva mientras ésta vive; el
/// renderer y los eventos la leen y la escriben por índice de preorden.
#[derive(Clone, Debug, Default)]
pub struct Valores {
    celdas: Vec<Celda>,
}

impl Valores {
    /// Valores iniciales de un documento, tomados de sus propiedades.
    pub fn del_documento(doc: &ValidatedDocument) -> Valores {
        let pre = doc.preorden();
        let mut celdas = Vec::with_capacity(pre.len());
        for n in &pre {
            celdas.push(Celda::inicial(n));
        }
        Valores { celdas }
    }

    /// Número de celdas (uno por nodo del documento).
    pub fn len(&self) -> usize {
        self.celdas.len()
    }

    /// ¿No hay ningún nodo?
    pub fn vacio(&self) -> bool {
        self.celdas.is_empty()
    }

    /// Celda de un nodo (fuera de rango = celda vacía: nunca revienta).
    pub fn celda(&self, i: usize) -> &Celda {
        match self.celdas.get(i) {
            Some(c) => c,
            None => VACIA,
        }
    }

    /// Celda de un nodo, para escribir en ella.
    pub fn celda_mut(&mut self, i: usize) -> &mut Celda {
        while self.celdas.len() <= i {
            self.celdas.push(Celda::default());
        }
        &mut self.celdas[i]
    }

    /// ¿Está marcado ese `check`/`radio`?
    pub fn marcado(&self, i: usize) -> bool {
        self.celda(i).marcado
    }

    /// Opción o valor entero de ese nodo.
    pub fn entero(&self, i: usize) -> i64 {
        self.celda(i).entero
    }

    /// Texto de ese `field`.
    pub fn texto(&self, i: usize) -> &str {
        &self.celda(i).texto
    }

    /// ¿Está desplegado ese `popup`?
    pub fn abierto(&self, i: usize) -> bool {
        self.celda(i).abierto
    }
}

/// Celda devuelta fuera de rango (inmutable, para no allocationar).
static VACIA: &Celda = &Celda {
    marcado: false,
    entero: 0,
    texto: String::new(),
    abierto: false,
    trasiego: None,
};

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::toby_ui::parser::analizar;
    use crate::toby_ui::validation::validar;

    const DOC: &str = concat!(
        "<toby-app name=\"v\">",
        "<window width=\"400\" height=\"300\">",
        "<stack width=\"400\" height=\"300\">",
        "<check id=\"c1\" x=\"10\" y=\"10\" label=\"Sí\" checked=\"true\"/>",
        "<radio id=\"r1\" x=\"10\" y=\"40\" label=\"A\" group=\"g\" checked=\"true\"/>",
        "<segmented id=\"s1\" x=\"10\" y=\"70\" items=\"Pequeño|Mediano|Grande\" selected=\"2\"/>",
        "<slider id=\"l1\" x=\"10\" y=\"100\" value=\"30\" min=\"0\" max=\"50\"/>",
        "<field id=\"t1\" x=\"10\" y=\"130\" value=\"hola\" width=\"140\"/>",
        "<popup id=\"p1\" x=\"10\" y=\"160\" items=\"Rojo|Verde\" selected=\"1\"/>",
        "</stack>",
        "</window>",
        "</toby-app>"
    );

    fn valores(src: &str) -> (ValidatedDocument, Valores) {
        let d = analizar(src).expect("parser");
        let v = validar(&d).expect("validación");
        let val = Valores::del_documento(&v);
        (v, val)
    }

    #[test]
    fn toma_los_valores_iniciales_del_documento() {
        let (doc, val) = valores(DOC);
        let id = |s: &str| doc.indice_de(s).expect("id");
        assert!(val.marcado(id("c1")), "el check nace marcado");
        assert!(val.marcado(id("r1")), "el radio nace marcado");
        assert_eq!(val.entero(id("s1")), 2, "el segmented nace en «Grande»");
        assert_eq!(val.entero(id("l1")), 30, "el slider nace en 30");
        assert_eq!(val.texto(id("t1")), "hola", "el field nace con su value");
        assert_eq!(val.entero(id("p1")), 1, "el popup nace en «Verde»");
        assert!(!val.abierto(id("p1")), "el popup nace cerrado");
    }

    #[test]
    fn una_seleccion_fuera_de_rango_se_acota() {
        let src = DOC.replace("selected=\"2\"", "selected=\"99\"");
        let (doc, val) = valores(&src);
        let i = doc.indice_de("s1").expect("id");
        assert_eq!(val.entero(i), 2, "sólo hay 3 opciones (0..2)");
    }

    #[test]
    fn un_slider_fuera_de_rango_se_acota_a_su_minimo() {
        let src = DOC.replace("value=\"30\"", "value=\"-5\"");
        let (doc, val) = valores(&src);
        let i = doc.indice_de("l1").expect("id");
        assert_eq!(val.entero(i), 0, "no puede bajar de min");
    }

    #[test]
    fn slider_sin_value_nace_en_el_punto_medio_del_rango() {
        let src = DOC.replace("value=\"30\" min=\"0\" max=\"50\"", "min=\"0\" max=\"100\"");
        let (doc, val) = valores(&src);
        let i = doc.indice_de("l1").expect("id");
        assert_eq!(val.entero(i), 50);
    }

    #[test]
    fn la_celda_fuera_de_rango_es_vacia() {
        let (_doc, val) = valores(DOC);
        assert!(!val.marcado(999));
        assert_eq!(val.entero(999), 0);
        assert_eq!(val.texto(999), "");
        assert!(!val.abierto(999));
    }
}
