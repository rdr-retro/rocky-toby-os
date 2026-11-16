//! Validación semántica de Toby UI.
//!
//! Toma el `Document` crudo del parser y produce un `ValidatedDocument` con el
//! árbol TIPADO. Si algo no encaja con la tabla de componentes, se rechaza
//! completo: no hay "mejor esfuerzo".
//!
//! Reglas de la Fase 2:
//!   * componente conocido
//!   * propiedades conocidas para ese componente
//!   * tipos correctos (int, bool, string, enum, color, idref)
//!   * propiedades obligatorias presentes
//!   * `id` únicos en todo el documento
//!   * `name` únicos también (el nombre propio del elemento, como en HTML)
//!   * jerarquía mínima: raíz `toby-app`, ésta exige al menos una `window`,
//!     y sólo los contenedores admiten hijos.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::component::{buscar, PropType};
use super::document::{Document, Nodo, PropValue, Sintaxis, ValidatedDocument};

/// Error de validación, con posición aproximada (línea del nodo).
#[derive(Clone, Debug)]
pub struct ValidationError {
    /// Línea (1-based) del nodo culpable, si se conoce.
    pub linea: usize,
    /// Descripción breve del problema.
    pub detalle: String,
}

impl core::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "línea {}: {}", self.linea, self.detalle)
    }
}

/// Resultado de validar: documento tipado o lista de errores.
pub type ValidationResult = Result<ValidatedDocument, Vec<ValidationError>>;

/// Valida un documento crudo contra la tabla de componentes.
pub fn validar(doc: &Document) -> ValidationResult {
    let mut errores: Vec<ValidationError> = Vec::new();
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let mut nombres: BTreeSet<String> = BTreeSet::new();

    // La raíz ya la forzó el parser a ser `toby-app`.
    let raiz = match validar_nodo(&doc.raiz, &mut errores, &mut ids, &mut nombres) {
        Some(n) => n,
        None => return Err(errores),
    };

    // Regla de jerarquía: la app debe abrir al menos una ventana.
    if !raiz.hijos.iter().any(|h| h.def.etiqueta == "window") {
        errores.push(ValidationError {
            linea: doc.raiz.linea,
            detalle: String::from("toby-app debe contener al menos un <window>"),
        });
    }

    if errores.is_empty() {
        // El nombre sale de la propiedad ya tipada de la raíz.
        let nombre = raiz
            .prop("name")
            .and_then(|v| v.texto())
            .unwrap_or("")
            .to_string();
        Ok(ValidatedDocument { nombre, raiz })
    } else {
        Err(errores)
    }
}

/// Valida un nodo y todos sus hijos. Devuelve `None` si el nodo es inválido
/// (los errores se van acumulando en `errores`).
fn validar_nodo(
    s: &Sintaxis,
    errores: &mut Vec<ValidationError>,
    ids: &mut BTreeSet<String>,
    nombres: &mut BTreeSet<String>,
) -> Option<Nodo> {
    let mut ok = true;

    let def = match buscar(&s.etiqueta) {
        Some(d) => d,
        None => {
            errores.push(ValidationError {
                linea: s.linea,
                detalle: alloc::format!("componente desconocido: '{}'", s.etiqueta),
            });
            return None;
        }
    };

    // Propiedades conocidas y bien tipadas.
    let mut props: Vec<(String, PropValue)> = Vec::new();
    let mut id: Option<String> = None;
    for (nombre, valor) in &s.atributos {
        let pdef = def.propiedades.iter().find(|p| p.nombre == nombre.as_str());
        match pdef {
            None => {
                errores.push(ValidationError {
                    linea: s.linea,
                    detalle: alloc::format!(
                        "propiedad '{}' no permitida en <{}>",
                        nombre,
                        def.etiqueta
                    ),
                });
                ok = false;
            }
            Some(p) => {
                if nombre == "transition-duration"
                    && !valor
                        .parse::<u32>()
                        .map(|ms| (1..=10_000).contains(&ms))
                        .unwrap_or(false)
                {
                    errores.push(ValidationError {
                        linea: s.linea,
                        detalle: String::from(
                            "'transition-duration' debe estar entre 1 y 10000 ms",
                        ),
                    });
                    ok = false;
                }
                if nombre.as_str() == "id" {
                    // El id se comprueba además contra el resto del documento.
                    if ids.contains(valor) {
                        errores.push(ValidationError {
                            linea: s.linea,
                            detalle: alloc::format!("id repetido: '{}'", valor),
                        });
                        ok = false;
                    } else {
                        ids.insert(valor.clone());
                        id = Some(valor.clone());
                    }
                }
                // El `name` propio del elemento (como en HTML) también es
                // único en el documento: dos elementos no pueden compartirlo.
                // Y tiene forma de identificador, igual que un `id`.
                if nombre.as_str() == "name" {
                    if !es_identificador(valor) {
                        errores.push(ValidationError {
                            linea: s.linea,
                            detalle: alloc::format!(
                                "'{}' no es un 'name' válido en <{}>; se espera un \
                                 identificador como 'aceptar' o 'campo-nombre'",
                                valor,
                                def.etiqueta
                            ),
                        });
                        ok = false;
                    } else if nombres.contains(valor) {
                        errores.push(ValidationError {
                            linea: s.linea,
                            detalle: alloc::format!("name repetido: '{}'", valor),
                        });
                        ok = false;
                    } else {
                        nombres.insert(valor.clone());
                    }
                }
                match convertir(p.tipo, valor) {
                    Some(v) => {
                        // Los enums son listas cerradas: se rechaza lo que no
                        // esté declarado en `valores`.
                        if p.tipo == PropType::Enum
                            && !p.valores.is_empty()
                            && !p.valores.contains(&valor.as_str())
                        {
                            let mut msg = String::from("valor no permitido para '");
                            msg.push_str(nombre);
                            msg.push_str("' en <");
                            msg.push_str(def.etiqueta);
                            msg.push_str(">; se admite: ");
                            for (i, v) in p.valores.iter().enumerate() {
                                if i > 0 {
                                    msg.push_str(", ");
                                }
                                msg.push_str(v);
                            }
                            errores.push(ValidationError {
                                linea: s.linea,
                                detalle: msg,
                            });
                            ok = false;
                        } else {
                            props.push((nombre.clone(), v));
                        }
                    }
                    None => {
                        errores.push(ValidationError {
                            linea: s.linea,
                            detalle: alloc::format!(
                                "valor '{}' inválido para '{}' en <{}> (se esperaba {})",
                                valor,
                                nombre,
                                def.etiqueta,
                                describir_tipo(p.tipo)
                            ),
                        });
                        ok = false;
                    }
                }
            }
        }
    }

    // Propiedades obligatorias.
    for p in def.propiedades {
        if p.requerida && !s.atributos.iter().any(|(n, _)| n.as_str() == p.nombre) {
            errores.push(ValidationError {
                linea: s.linea,
                detalle: alloc::format!(
                    "falta la propiedad obligatoria '{}' en <{}>",
                    p.nombre,
                    def.etiqueta
                ),
            });
            ok = false;
        }
    }

    // Un progreso porcentual necesita un valor explícito y el modo ocupado no
    // admite uno: así no hay estados ambiguos ni porcentajes fuera de rango.
    if def.etiqueta == "progress" {
        let modo = s
            .atributos
            .iter()
            .find(|(nombre, _)| nombre == "mode")
            .map(|(_, valor)| valor.as_str())
            .unwrap_or("indeterminate");
        let valor = s
            .atributos
            .iter()
            .find(|(nombre, _)| nombre == "value")
            .map(|(_, valor)| valor.as_str());
        if modo == "determinate" && valor.is_none() {
            errores.push(ValidationError {
                linea: s.linea,
                detalle: String::from(
                    "<progress mode=\"determinate\"> requiere 'value' entre 0 y 100",
                ),
            });
            ok = false;
        } else if modo == "indeterminate" && valor.is_some() {
            errores.push(ValidationError {
                linea: s.linea,
                detalle: String::from("<progress mode=\"indeterminate\"> no admite 'value'"),
            });
            ok = false;
        }
        if let Some(valor) = valor {
            if let Ok(porcentaje) = valor.parse::<i64>() {
                if !(0..=100).contains(&porcentaje) {
                    errores.push(ValidationError {
                        linea: s.linea,
                        detalle: String::from("'value' en <progress> debe estar entre 0 y 100"),
                    });
                    ok = false;
                }
            }
        }
    }

    // Jerarquía: sólo los contenedores admiten hijos.
    if !def.contiene_hijos && !s.hijos.is_empty() {
        errores.push(ValidationError {
            linea: s.linea,
            detalle: alloc::format!("<{}> no admite hijos", def.etiqueta),
        });
        ok = false;
    }

    // Hijos.
    let mut hijos: Vec<Nodo> = Vec::new();
    for h in &s.hijos {
        if let Some(n) = validar_nodo(h, errores, ids, nombres) {
            hijos.push(n);
        } else {
            ok = false;
        }
    }

    if ok {
        Some(Nodo {
            def,
            linea: s.linea,
            id,
            props,
            hijos,
        })
    } else {
        None
    }
}

/// Convierte un valor textual a su tipo, o `None` si no encaja.
fn convertir(tipo: PropType, valor: &str) -> Option<PropValue> {
    match tipo {
        PropType::Int => valor.parse::<i64>().ok().map(PropValue::Int),
        PropType::Bool => match valor {
            "true" => Some(PropValue::Bool(true)),
            "false" => Some(PropValue::Bool(false)),
            _ => None,
        },
        PropType::String => Some(PropValue::Str(valor.to_string())),
        PropType::Enum => Some(PropValue::Enum(valor.to_string())),
        PropType::Color => parse_color(valor).map(PropValue::Color),
        // Una ruta de recurso: sólo se admiten PNG dentro de la carpeta
        // autorizada (tema del sistema o bundle `.app`). Que exista se comprueba
        // al cargarlo (fallo visible, nunca una caída).
        PropType::Recurso => {
            if super::resource::autorizada(valor) {
                Some(PropValue::Str(valor.to_string()))
            } else {
                None
            }
        }
        // Una clave de acción sólo tiene que estar BIEN FORMADA aquí; que
        // exista un manejador se comprueba al despacharla (Fase 7).
        PropType::Accion => {
            if super::component::clave_valida(valor) {
                Some(PropValue::Str(valor.to_string()))
            } else {
                None
            }
        }
        // Un idref se comprueba aparte (unicidad); aquí basta con la forma.
        PropType::IdRef => {
            if es_identificador(valor) {
                Some(PropValue::Str(valor.to_string()))
            } else {
                None
            }
        }
    }
}

/// `#RRGGBB` → 0x00RRGGBB.
fn parse_color(valor: &str) -> Option<u32> {
    let hex = valor.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// Forma de identificador: `[a-zA-Z][a-zA-Z0-9_.-]*`.
fn es_identificador(v: &str) -> bool {
    let b = v.as_bytes();
    if b.is_empty() {
        return false;
    }
    if !b[0].is_ascii_alphabetic() {
        return false;
    }
    b[1..]
        .iter()
        .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'.' || *c == b'-')
}

fn describir_tipo(t: PropType) -> &'static str {
    match t {
        PropType::Int => "un entero",
        PropType::Bool => "'true' o 'false'",
        PropType::String => "texto",
        PropType::Enum => "un valor de la lista permitida",
        PropType::Color => "un color #RRGGBB",
        PropType::Recurso => "una ruta PNG autorizada (/usr/share/tobyui/…)",
        PropType::IdRef => "un identificador válido",
        PropType::Accion => "una clave de acción (dominio.accion)",
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::toby_ui::parser::analizar;

    #[test]
    fn documento_valido() {
        let d = analizar(
            "<toby-app name=\"demo\">\
               <window title=\"Hola\" width=\"640\" height=\"400\">\
                 <column spacing=\"8\">\
                   <button id=\"ok\" title=\"Aceptar\"/>\
                 </column>\
               </window>\
             </toby-app>",
        )
        .expect("parser");
        let v = validar(&d).expect("valido");
        assert_eq!(v.nombre, "demo");
        assert_eq!(v.raiz.def.etiqueta, "toby-app");
        assert_eq!(v.raiz.hijos.len(), 1);
        assert_eq!(v.num_nodos(), 4);
        assert!(v.por_id("ok").is_some());
    }

    #[test]
    fn duracion_de_transicion_se_acota_a_10_segundos() {
        for duracion in ["0", "10001"] {
            let src = alloc::format!(
                "<toby-app name=\"x\"><window width=\"1\" height=\"1\" \
                 transition=\"genie\" transition-duration=\"{}\"/></toby-app>",
                duracion
            );
            let d = analizar(&src).expect("parser");
            let e = validar(&d).expect_err("duracion fuera de rango");
            assert!(
                e.iter().any(|x| x.detalle.contains("transition-duration")),
                "errores: {:?}",
                e
            );
        }
    }

    #[test]
    fn componente_desconocido() {
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\"><marquee/></window></toby-app>",
        )
        .expect("parser");
        let e = validar(&d).expect_err("debe fallar");
        assert!(e[0].detalle.contains("desconocido"));
    }

    #[test]
    fn propiedad_desconocida() {
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\" colour=\"red\"/></toby-app>",
        )
        .expect("parser");
        let e = validar(&d).expect_err("debe fallar");
        assert!(e[0].detalle.contains("no permitida"));
    }

    #[test]
    fn tipo_mal() {
        let d = analizar("<toby-app name=\"x\"><window width=\"ancho\" height=\"1\"/></toby-app>")
            .expect("parser");
        assert!(validar(&d).is_err());
    }

    #[test]
    fn obligatoria_faltante() {
        let d = analizar("<toby-app name=\"x\"><window width=\"10\"/></toby-app>").expect("parser");
        let e = validar(&d).expect_err("debe fallar");
        assert!(e[0].detalle.contains("height"));
    }

    #[test]
    fn progress_admite_modos_indeterminado_y_porcentual() {
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"200\" height=\"40\">\
               <progress id=\"busy\" mode=\"indeterminate\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        let v = validar(&d).expect("progreso indeterminado válido");
        assert_eq!(
            v.por_id("busy")
                .expect("progress")
                .texto("mode", "indeterminate"),
            "indeterminate"
        );

        let porcentual = analizar(
            "<toby-app name=\"x\"><window width=\"200\" height=\"40\">\
               <progress id=\"load\" mode=\"determinate\" value=\"65\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        let v = validar(&porcentual).expect("progreso porcentual válido");
        assert_eq!(v.por_id("load").expect("progress").entero("value", -1), 65);

        for xml in [
            "<toby-app name=\"x\"><window width=\"200\" height=\"40\"><progress mode=\"determinate\"/></window></toby-app>",
            "<toby-app name=\"x\"><window width=\"200\" height=\"40\"><progress mode=\"determinate\" value=\"101\"/></window></toby-app>",
            "<toby-app name=\"x\"><window width=\"200\" height=\"40\"><progress mode=\"determinate\" value=\"-1\"/></window></toby-app>",
            "<toby-app name=\"x\"><window width=\"200\" height=\"40\"><progress value=\"50\"/></window></toby-app>",
        ] {
            let documento = analizar(xml).expect("parser");
            assert!(validar(&documento).is_err());
        }
    }

    #[test]
    fn id_repetido() {
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
               <button id=\"a\" title=\"1\"/><button id=\"a\" title=\"2\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        let e = validar(&d).expect_err("debe fallar");
        assert!(e.iter().any(|x| x.detalle.contains("repetido")));
    }

    #[test]
    fn sin_ventana() {
        let d = analizar("<toby-app name=\"x\"><column/></toby-app>").expect("parser");
        let e = validar(&d).expect_err("debe fallar");
        assert!(e[0].detalle.contains("window"));
    }

    #[test]
    fn accion_mal_formada() {
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
               <button title=\"a\" action=\"Guardar!\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        let e = validar(&d).expect_err("debe fallar");
        assert!(e[0].detalle.contains("clave de acción"), "{}", e[0].detalle);

        // Y una bien formada pasa y se lee como texto.
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
               <button id=\"b\" title=\"a\" action=\"settings.save\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        let v = validar(&d).expect("valido");
        let b = v.por_id("b").expect("botón");
        assert_eq!(b.texto("action", ""), "settings.save");
    }

    #[test]
    fn hoja_con_hijos() {
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
               <button title=\"a\"><box/></button>\
             </window></toby-app>",
        )
        .expect("parser");
        assert!(validar(&d).is_err());
    }

    #[test]
    fn name_unico_en_el_documento() {
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
               <button name=\"ok\" title=\"1\"/><button name=\"ok\" title=\"2\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        let e = validar(&d).expect_err("debe fallar");
        assert!(
            e.iter().any(|x| x.detalle.contains("name repetido")),
            "{:?}",
            e
        );

        // Dos elementos de TIPOS distintos tampoco pueden compartir nombre.
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
               <button name=\"ok\" title=\"1\"/><check name=\"ok\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        assert!(validar(&d).is_err());

        // El nombre de la app también ocupa el mismo espacio de nombres.
        let d = analizar(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
               <button name=\"x\" title=\"1\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        assert!(validar(&d).is_err());
    }

    #[test]
    fn name_correcto_se_lee_en_el_nodo() {
        let d = analizar(
            "<toby-app name=\"demo\"><window width=\"1\" height=\"1\">\
               <button id=\"b\" name=\"aceptar\" title=\"Aceptar\"/>\
               <button id=\"c\" title=\"Sin nombre\"/>\
             </window></toby-app>",
        )
        .expect("parser");
        let v = validar(&d).expect("valido");
        assert_eq!(v.por_id("b").expect("botón").nombre(), Some("aceptar"));
        assert_eq!(v.por_id("c").expect("botón").nombre(), None);
        // La raíz también es un elemento con su nombre.
        assert_eq!(v.raiz.nombre(), Some("demo"));
    }

    #[test]
    fn name_rechaza_valores_que_no_son_identificador() {
        // Forma: `[a-zA-Z][a-zA-Z0-9_.-]*`, igual que un `id`.
        for malo in ["\"\"", "\"1arranca\"", "\"con espacio\"", "\"con-ñ\""] {
            let src = alloc::format!(
                "<toby-app name=\"x\"><window width=\"1\" height=\"1\">\
                   <button name={} title=\"a\"/>\
                 </window></toby-app>",
                malo
            );
            let d = analizar(&src).expect("parser");
            let e = validar(&d).expect_err("debe fallar");
            assert!(
                e.iter().any(|x| x.detalle.contains("nombre")),
                "name={} debió rechazarse: {:?}",
                malo,
                e
            );
        }
    }

    #[test]
    fn todos_los_tipos_admiten_name() {
        // Como en HTML, cada tipo de elemento tiene su nombre.
        for c in super::super::component::COMPONENTES {
            assert!(
                c.propiedades.iter().any(|p| p.nombre == "name"),
                "<{}> no admite 'name'",
                c.etiqueta
            );
        }
    }
}
