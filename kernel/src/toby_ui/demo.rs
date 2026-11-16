//! Primera aplicación Toby UI completa (Fase 8).
//!
//! El documento vive en `demo.toby` (al lado de este fichero) y se incrusta con
//! `include_str!`, así que la demo no depende del sistema de ficheros. El mismo
//! fichero se podría sembrar en `rockydisk.img` como
//! `/usr/share/tobyui/demo.toby` y montarla con [`App::desde_fs`].
//!
//! Aquí está todo el "código" de la aplicación: tres manejadores y el estado
//! que tocan. El documento sólo los nombra.

use alloc::string::{String, ToString};

use super::action::{Peticion, Registro, Resultado};
use super::app::{App, ErrorApp};

/// El documento, incrustado desde el fichero real.
pub const DEMO: &str = include_str!("demo.toby");

/// Ruta que usaría la demo si se cargara del disco.
pub const RUTA: &str = "/usr/share/tobyui/demo.toby";

/// Estado propio de la aplicación. Es lo único que ven los manejadores.
#[derive(Clone, Debug, Default)]
pub struct Estado {
    /// Cuántas veces se ha pulsado «Guardar» desde el último reinicio.
    pub guardados: usize,
    /// `id` del nodo que disparó la última acción.
    pub ultimo: Option<String>,
    /// `name` de ese nodo: el nombre propio y único del elemento, como en
    /// HTML. Es lo que usaría una aplicación real para saber qué campo ha
    /// sido, sin depender del `id` técnico.
    pub ultimo_nombre: Option<String>,
    /// Si alguna acción pidió cerrar la aplicación.
    pub cerrado: bool,
}

/// Apunta el nodo que ha disparado, guardando `id` y `name`.
fn apuntar(e: &mut Estado, p: &Peticion<'_>) {
    e.ultimo = p.id.map(|s| s.to_string());
    e.ultimo_nombre = p.nombre.map(|s| s.to_string());
}

/// `action="settings.save"`: cuenta el guardado.
pub fn guardar(e: &mut Estado, p: &Peticion<'_>) -> Resultado {
    e.guardados += 1;
    apuntar(e, p);
    Resultado::Hecho
}

/// `action="settings.reset"`: vuelve a cero.
pub fn reiniciar(e: &mut Estado, p: &Peticion<'_>) -> Resultado {
    e.guardados = 0;
    apuntar(e, p);
    Resultado::Hecho
}

/// `action="window.close"`: pide cerrar la aplicación.
pub fn cerrar(e: &mut Estado, p: &Peticion<'_>) -> Resultado {
    e.cerrado = true;
    apuntar(e, p);
    Resultado::Cerrar
}

/// Las tres acciones que declara el documento.
pub fn registro() -> Registro<Estado> {
    let mut r = Registro::nuevo();
    // Claves fijas y bien formadas: si esto fallara sería un error de
    // programación, y las pruebas lo detectarían.
    r.registrar("settings.save", guardar)
        .expect("clave 'settings.save'");
    r.registrar("settings.reset", reiniciar)
        .expect("clave 'settings.reset'");
    r.registrar("window.close", cerrar)
        .expect("clave 'window.close'");
    r
}

/// Monta la demo con el documento incrustado.
pub fn aplicacion(ancho: u32, alto: u32) -> Result<App<Estado>, ErrorApp> {
    App::nueva(DEMO, ancho, alto, registro())
}

/// Monta la demo leyendo el documento de la carpeta autorizada.
pub fn aplicacion_desde_fs(ancho: u32, alto: u32) -> Result<App<Estado>, ErrorApp> {
    App::desde_fs(RUTA, ancho, alto, registro())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::toby_ui::event::{Boton, Evento};

    #[test]
    fn el_documento_incrustado_monta_entero() {
        let a = aplicacion(560, 200).expect("la demo debe montar");
        assert_eq!(a.nombre(), "demo");
        // toby-app + window + column + 2 filas + 9 botones.
        assert_eq!(a.documento().num_nodos(), 14);
        assert_eq!(a.registro().len(), 3);

        // Todos los ids declarados existen y tienen rectángulo.
        for id in [
            "guardar",
            "reiniciar",
            "salir",
            "azul",
            "verde",
            "magenta",
            "amarillo",
            "blanco",
            "apagado",
        ] {
            let i = a
                .documento()
                .indice_de(id)
                .unwrap_or_else(|| panic!("falta '{}'", id));
            let r = a.disposicion().rect(i);
            assert!(r.w > 0 && r.h > 0, "'{}' sin tamaño", id);
        }
        // Todos los elementos con `name` lo llevan una sola vez (el
        // validador ya lo exige; aqui se ve que la demo lo usa de verdad).
        let mut vistos = alloc::vec::Vec::new();
        for n in a.documento().preorden() {
            if let Some(nom) = n.nombre() {
                assert!(!vistos.contains(&nom), "name '{}' repetido", nom);
                vistos.push(nom);
            }
        }
        assert_eq!(vistos.len(), 10, "la demo debe nombrar sus diez botones");
        // La ventana es la que pide el documento.
        assert_eq!(a.disposicion().rect(1).w, 560);
        assert_eq!(a.disposicion().rect(1).h, 200);
    }

    #[test]
    fn el_recorrido_completo_funciona() {
        let mut a = aplicacion(560, 200).expect("monta");
        let mut e = Estado::default();

        // 1. Guardar.
        let i = a.documento().indice_de("guardar").expect("id");
        let r = a.disposicion().rect(i);
        let (x, y) = (r.x + 4, r.y + 4);
        a.procesar(
            &Evento::RatonAbajo {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        let paso = a.procesar(
            &Evento::RatonArriba {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        assert_eq!(paso.accion, Some(Resultado::Hecho));
        assert_eq!(e.guardados, 1);
        assert_eq!(e.ultimo.as_deref(), Some("guardar"));
        // El `name` propio del elemento viaja con la peticion, como en HTML.
        assert_eq!(e.ultimo_nombre.as_deref(), Some("btn-guardar"));

        // 2. Reiniciar.
        let i = a.documento().indice_de("reiniciar").expect("id");
        let r = a.disposicion().rect(i);
        let (x, y) = (r.x + 4, r.y + 4);
        a.procesar(
            &Evento::RatonAbajo {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        a.procesar(
            &Evento::RatonArriba {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        assert_eq!(e.guardados, 0);
        assert_eq!(e.ultimo.as_deref(), Some("reiniciar"));

        // 3. Salir: pide cerrar.
        let i = a.documento().indice_de("salir").expect("id");
        let r = a.disposicion().rect(i);
        let (x, y) = (r.x + 4, r.y + 4);
        a.procesar(
            &Evento::RatonAbajo {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        let paso = a.procesar(
            &Evento::RatonArriba {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        assert_eq!(paso.accion, Some(Resultado::Cerrar));
        assert!(e.cerrado && a.cerrada());
    }

    #[test]
    fn los_botones_de_muestrario_no_hacen_nada() {
        let mut a = aplicacion(560, 200).expect("monta");
        let mut e = Estado::default();
        for id in ["azul", "verde", "magenta", "amarillo", "blanco"] {
            let i = a.documento().indice_de(id).expect("id");
            let r = a.disposicion().rect(i);
            let (x, y) = (r.x + 4, r.y + 4);
            a.procesar(
                &Evento::RatonAbajo {
                    x,
                    y,
                    boton: Boton::Izquierdo,
                },
                &mut e,
            );
            let paso = a.procesar(
                &Evento::RatonArriba {
                    x,
                    y,
                    boton: Boton::Izquierdo,
                },
                &mut e,
            );
            assert!(paso.accion.is_none() && paso.error.is_none(), "{}", id);
        }
        assert_eq!(e.guardados, 0);
        assert!(e.ultimo.is_none());
    }

    #[test]
    fn el_boton_deshabilitado_no_recibe_el_clic() {
        let mut a = aplicacion(560, 200).expect("monta");
        let mut e = Estado::default();
        let i = a.documento().indice_de("apagado").expect("id");
        let r = a.disposicion().rect(i);
        let (x, y) = (r.x + 4, r.y + 4);
        // Ni siquiera es diana.
        assert_ne!(a.hover(), Some(i));
        a.procesar(&Evento::RatonMueve { x, y, dx: 0, dy: 0 }, &mut e);
        assert_eq!(a.hover(), None);
        a.procesar(
            &Evento::RatonAbajo {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        let paso = a.procesar(
            &Evento::RatonArriba {
                x,
                y,
                boton: Boton::Izquierdo,
            },
            &mut e,
        );
        assert!(paso.cambios.click.is_none());
    }

    #[test]
    fn el_tabulador_recorre_solo_los_habilitados() {
        let mut a = aplicacion(560, 200).expect("monta");
        let mut e = Estado::default();
        let mut vistos = alloc::vec::Vec::new();
        for _ in 0..9 {
            a.procesar(
                &Evento::Tecla(crate::toby_ui::event::Tecla::Tabulador),
                &mut e,
            );
            vistos.push(a.foco().expect("foco"));
        }
        // Ocho interactivos (los nueve menos el deshabilitado) y vuelta al principio.
        assert_eq!(vistos.len(), 9);
        assert_eq!(vistos[8], vistos[0]);
        let apagado = a.documento().indice_de("apagado").expect("id");
        assert!(!vistos.contains(&apagado));
        // Todos los demás, sí.
        for id in [
            "guardar",
            "reiniciar",
            "salir",
            "azul",
            "verde",
            "magenta",
            "amarillo",
            "blanco",
        ] {
            let i = a.documento().indice_de(id).expect("id");
            assert!(vistos.contains(&i), "'{}' no recibió el foco", id);
        }
    }
}
