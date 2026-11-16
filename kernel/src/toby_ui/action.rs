//! Registro de acciones de Toby UI (Fase 7).
//!
//! Un documento Toby UI **nombra** acciones (`action="settings.save"`) pero
//! nunca aporta código: no hay `eval`, ni scripts, ni punteros, ni cadenas que
//! se interpreten. El código vive aquí, registrado por la aplicación anfitriona
//! en tiempo de ejecución como punteros a función normales.
//!
//! Despachar una clave que nadie ha registrado es un **error controlado**
//! ([`ErrorAccion::Desconocida`]), nunca una caída ni una ejecución implícita.
//!
//! Flujo típico:
//!
//! ```text
//! event::Interaccion::aplicar --> Cambios { click: Some(3) }
//!                            --> Registro::despachar_click --> manejador
//! ```
//!
//! El registro es genérico sobre el estado de la aplicación (`C`), así que este
//! módulo no inventa ninguna API del kernel: el anfitrión decide qué es `C`.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::component::clave_valida;
use super::event::{Cambios, Interaccion};

/// Datos que acompañan a una acción disparada desde el documento.
///
/// Es todo lo que ve un manejador sobre el documento: la clave declarada, el
/// nodo que la disparó, su `id` y su `name`. No hay acceso al árbol, ni a la
/// memoria, ni a nada que el anfitrión no haya decidido exponer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Peticion<'a> {
    /// Clave declarada en el documento, p.ej. `settings.save`.
    pub clave: &'a str,
    /// Índice del nodo culpable, en el preorden del documento.
    pub nodo: usize,
    /// `id` del nodo, si declara uno.
    pub id: Option<&'a str>,
    /// `name` del nodo (como en HTML), si declara uno.
    ///
    /// Es el nombre propio y único del elemento: permite que un mismo
    /// manejador atienda varios elementos sin depender del `id` técnico.
    pub nombre: Option<&'a str>,
}

/// Resultado de ejecutar una acción.
///
/// Es deliberadamente corto: un manejador no puede pedir cosas raras al
/// runtime. Si la aplicación necesita comunicar algo más, lo guarda en su
/// propio estado `C`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Resultado {
    /// La acción se ejecutó y no hay nada más que hacer.
    #[default]
    Hecho,
    /// La acción pide cerrar la ventana o la aplicación.
    Cerrar,
}

/// Firma de un manejador: recibe el estado del anfitrión y la petición.
///
/// Es un puntero a función, no un objeto con `vtable`: registrar una acción no
/// reserva memoria dinámica y no se puede "colar" código desde el documento.
pub type Manejador<C> = fn(&mut C, &Peticion<'_>) -> Resultado;

/// Motivo por el que una acción no se ha podido ejecutar.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ErrorAccion {
    /// La clave no tiene la forma admitida (ver `component::clave_valida`).
    ClaveInvalida(String),
    /// Nadie ha registrado esa clave.
    Desconocida(String),
    /// Ya había un manejador con esa clave; se conserva el primero.
    Duplicada(String),
}

impl ErrorAccion {
    /// Clave a la que se refiere el error.
    pub fn clave(&self) -> &str {
        match self {
            ErrorAccion::ClaveInvalida(c)
            | ErrorAccion::Desconocida(c)
            | ErrorAccion::Duplicada(c) => c,
        }
    }

    /// Descripción breve, en español, para mostrar al usuario.
    pub fn descripcion(&self) -> String {
        match self {
            ErrorAccion::ClaveInvalida(c) => {
                alloc::format!("clave de acción mal formada: '{}'", c)
            }
            ErrorAccion::Desconocida(c) => {
                alloc::format!("acción no registrada: '{}'", c)
            }
            ErrorAccion::Duplicada(c) => {
                alloc::format!("acción ya registrada: '{}'", c)
            }
        }
    }
}

impl core::fmt::Display for ErrorAccion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.descripcion())
    }
}

/// Tabla de acciones disponibles, construida por la aplicación.
///
/// Se usa `BTreeMap` (no `HashMap`) para que el orden de `claves()` sea
/// determinista: útil para volcar la lista de acciones o para las pruebas.
pub struct Registro<C> {
    acciones: BTreeMap<String, Manejador<C>>,
}

impl<C> Default for Registro<C> {
    fn default() -> Self {
        Registro::nuevo()
    }
}

impl<C> Registro<C> {
    /// Registro vacío. Despachar cualquier clave dará `Desconocida`.
    pub fn nuevo() -> Registro<C> {
        Registro {
            acciones: BTreeMap::new(),
        }
    }

    /// Registra un manejador bajo una clave declarativa.
    ///
    /// Rechaza claves mal formadas y claves repetidas (el primero gana).
    pub fn registrar(&mut self, clave: &str, manejador: Manejador<C>) -> Result<(), ErrorAccion> {
        if !clave_valida(clave) {
            return Err(ErrorAccion::ClaveInvalida(clave.to_string()));
        }
        if self.acciones.contains_key(clave) {
            return Err(ErrorAccion::Duplicada(clave.to_string()));
        }
        self.acciones.insert(clave.to_string(), manejador);
        Ok(())
    }

    /// ¿Hay manejador para esta clave?
    pub fn contiene(&self, clave: &str) -> bool {
        self.acciones.contains_key(clave)
    }

    /// Claves registradas, en orden alfabético.
    pub fn claves(&self) -> Vec<&str> {
        self.acciones.keys().map(|k| k.as_str()).collect()
    }

    /// Cuántas acciones hay registradas.
    pub fn len(&self) -> usize {
        self.acciones.len()
    }

    /// ¿No hay ninguna acción registrada?
    pub fn vacio(&self) -> bool {
        self.acciones.is_empty()
    }

    /// Ejecuta una acción declarada.
    ///
    /// Devuelve `Err` si la clave está mal formada o si no existe: la
    /// aplicación decide cómo informar, pero el fallo queda contenido.
    pub fn despachar(
        &self,
        clave: &str,
        nodo: usize,
        id: Option<&str>,
        nombre: Option<&str>,
        ctx: &mut C,
    ) -> Result<Resultado, ErrorAccion> {
        if !clave_valida(clave) {
            return Err(ErrorAccion::ClaveInvalida(clave.to_string()));
        }
        let manejador = match self.acciones.get(clave) {
            Some(m) => *m,
            None => return Err(ErrorAccion::Desconocida(clave.to_string())),
        };
        Ok(manejador(
            ctx,
            &Peticion {
                clave,
                nodo,
                id,
                nombre,
            },
        ))
    }

    /// Ejecuta la acción que declare un nodo, si declara alguna.
    ///
    /// * `Ok(None)` — el nodo no declara `action`: no hay nada que hacer.
    /// * `Ok(Some(_))` — se ejecutó.
    /// * `Err(_)` — declaró una clave inexistente o mal formada.
    pub fn despachar_nodo(
        &self,
        it: &Interaccion<'_>,
        i: usize,
        ctx: &mut C,
    ) -> Result<Option<Resultado>, ErrorAccion> {
        match it.accion_de(i) {
            Some(clave) => self
                .despachar(clave, i, it.id_de(i), it.nombre_de(i), ctx)
                .map(Some),
            None => Ok(None),
        }
    }

    /// Ejecuta la acción del nodo sobre el que se acaba de completar un clic.
    ///
    /// Es el puente directo desde `event::Interaccion::aplicar`: se le pasa el
    /// `Cambios` que devolvió y sólo actúa si hubo clic.
    pub fn despachar_click(
        &self,
        it: &Interaccion<'_>,
        cambios: &Cambios,
        ctx: &mut C,
    ) -> Result<Option<Resultado>, ErrorAccion> {
        match cambios.click {
            Some(i) => self.despachar_nodo(it, i, ctx),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::toby_ui::document::ValidatedDocument;
    use crate::toby_ui::event::{Boton, Evento};
    use crate::toby_ui::{layout, parser::analizar, validation::validar};

    /// Estado de ejemplo del anfitrión.
    #[derive(Default)]
    struct Estado {
        guardados: usize,
        ultimo_id: Option<String>,
        ultimo_nombre: Option<String>,
        cerrado: bool,
    }

    fn guardar(e: &mut Estado, p: &Peticion<'_>) -> Resultado {
        e.guardados += 1;
        e.ultimo_id = p.id.map(|s| s.to_string());
        e.ultimo_nombre = p.nombre.map(|s| s.to_string());
        Resultado::Hecho
    }

    fn cerrar(e: &mut Estado, p: &Peticion<'_>) -> Resultado {
        e.cerrado = true;
        e.ultimo_id = p.id.map(|s| s.to_string());
        e.ultimo_nombre = p.nombre.map(|s| s.to_string());
        Resultado::Cerrar
    }

    fn registro() -> Registro<Estado> {
        let mut r = Registro::nuevo();
        r.registrar("settings.save", guardar).expect("clave válida");
        r.registrar("window.close", cerrar).expect("clave válida");
        r
    }

    const DOC: &str = concat!(
        "<toby-app name=\"demo\">",
        "<window width=\"320\" height=\"240\">",
        "<column padding=\"10\" spacing=\"4\">",
        "<button id=\"guardar\" name=\"guardar-btn\" title=\"Guardar\" height=\"24\" action=\"settings.save\"/>",
        "<button id=\"otro\" title=\"Otro\" height=\"24\" action=\"settings.save\"/>",
        "<button id=\"salir\" title=\"Salir\" height=\"24\" action=\"window.close\"/>",
        "<button id=\"mudo\" title=\"Mudo\" height=\"24\"/>",
        "</column>",
        "</window>",
        "</toby-app>"
    );

    fn montar(src: &str) -> (ValidatedDocument, layout::Disposicion) {
        let d = analizar(src).expect("parser");
        let v = validar(&d).expect("validación");
        let disp = layout::calcular_en(&v, 320, 240);
        (v, disp)
    }

    #[test]
    fn registrar_rechaza_claves_mal_formadas_y_repetidas() {
        let mut r: Registro<Estado> = Registro::nuevo();
        assert_eq!(
            r.registrar("Save!", guardar),
            Err(ErrorAccion::ClaveInvalida("Save!".into()))
        );
        assert!(r.vacio());
        assert!(r.registrar("a.b", guardar).is_ok());
        assert_eq!(
            r.registrar("a.b", cerrar),
            Err(ErrorAccion::Duplicada("a.b".into()))
        );
        assert_eq!(r.len(), 1);
        // El primero registrado sigue siendo el que se ejecuta.
        let mut e = Estado::default();
        r.despachar("a.b", 0, None, None, &mut e).expect("existe");
        assert_eq!(e.guardados, 1);
        assert!(!e.cerrado);
    }

    #[test]
    fn despachar_clave_desconocida_es_error_controlado() {
        let r = registro();
        let mut e = Estado::default();
        let err = r
            .despachar("settings.borrar", 0, None, None, &mut e)
            .expect_err("no existe");
        assert_eq!(err, ErrorAccion::Desconocida("settings.borrar".into()));
        assert!(err.descripcion().contains("no registrada"));
        // El estado del anfitrión queda intacto.
        assert_eq!(e.guardados, 0);
        assert!(!e.cerrado);

        let err = r
            .despachar("Mal", 0, None, None, &mut e)
            .expect_err("mal formada");
        assert!(matches!(err, ErrorAccion::ClaveInvalida(_)));
    }

    #[test]
    fn despachar_ejecuta_el_manejador_y_devuelve_su_resultado() {
        let r = registro();
        let mut e = Estado::default();
        let res = r
            .despachar(
                "window.close",
                7,
                Some("salir"),
                Some("boton-salir"),
                &mut e,
            )
            .expect("existe");
        assert_eq!(res, Resultado::Cerrar);
        assert!(e.cerrado);
        assert_eq!(e.ultimo_id.as_deref(), Some("salir"));
        assert_eq!(e.ultimo_nombre.as_deref(), Some("boton-salir"));
    }

    #[test]
    fn las_claves_se_listan_en_orden() {
        let mut r = registro();
        r.registrar("alpha.uno", guardar).expect("válida");
        assert_eq!(
            r.claves(),
            alloc::vec!["alpha.uno", "settings.save", "window.close"]
        );
        assert!(r.contiene("window.close"));
        assert!(!r.contiene("window.open"));
    }

    #[test]
    fn un_clic_dispara_la_accion_del_nodo_pulsado() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        let r = registro();
        let mut e = Estado::default();

        // Índices: 0 app, 1 window, 2 column, 3 guardar, 4 otro, 5 salir, 6 mudo.
        // Botón "guardar" en (10,10,~80,24).
        let _ = it.aplicar(&Evento::RatonMueve {
            x: 20,
            y: 20,
            dx: 0,
            dy: 0,
        });
        let c1 = it.aplicar(&Evento::RatonAbajo {
            x: 20,
            y: 20,
            boton: Boton::Izquierdo,
        });
        assert!(c1.click.is_none());
        let c2 = it.aplicar(&Evento::RatonArriba {
            x: 20,
            y: 20,
            boton: Boton::Izquierdo,
        });
        assert_eq!(c2.click, Some(3));

        let res = r.despachar_click(&it, &c2, &mut e).expect("registrada");
        assert_eq!(res, Some(Resultado::Hecho));
        assert_eq!(e.guardados, 1);
        assert_eq!(e.ultimo_id.as_deref(), Some("guardar"));
        // El `name` del elemento viaja con la petición, como en HTML.
        assert_eq!(e.ultimo_nombre.as_deref(), Some("guardar-btn"));

        // Un clic en el botón sin acción no despacha nada.
        let c3 = it.aplicar(&Evento::RatonAbajo {
            x: 20,
            y: 100,
            boton: Boton::Izquierdo,
        });
        let _ = c3;
        let c4 = it.aplicar(&Evento::RatonArriba {
            x: 20,
            y: 100,
            boton: Boton::Izquierdo,
        });
        let res = r.despachar_click(&it, &c4, &mut e).expect("sin acción");
        assert_eq!(res, None);
        assert_eq!(e.guardados, 1);
    }

    #[test]
    fn un_documento_con_accion_inexistente_falla_al_despachar_no_al_cargar() {
        let src = concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column><button id=\"x\" title=\"X\" action=\"nadie.responde\"/></column>",
            "</window>",
            "</toby-app>"
        );
        // Cargar es válido: la forma de la clave es correcta.
        let (v, d) = montar(src);
        let it = Interaccion::nueva(&v, &d);
        let r = registro();
        let mut e = Estado::default();
        let err = r.despachar_nodo(&it, 3, &mut e).expect_err("no registrada");
        assert_eq!(err.clave(), "nadie.responde");
    }

    #[test]
    fn cambiar_de_documento_no_arrastra_acciones() {
        // El registro es independiente del documento: se puede reutilizar con
        // otro árbol (útil cuando la app recarga una vista).
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        let r = registro();
        let mut e = Estado::default();
        it.enfocar(Some(5));
        assert!(r.despachar_nodo(&it, 5, &mut e).expect("existe") == Some(Resultado::Cerrar));
        assert!(e.cerrado);
    }
}
