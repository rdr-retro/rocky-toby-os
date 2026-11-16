//! Anfitrión de aplicaciones Toby UI (la orden `toby`).
//!
//! Monta un documento `.toby` o la vista principal de un bundle `.app`.
//! Las apps a pantalla completa usan un modo VESA propio; `mode="window"` se
//! presenta como una ventana superpuesta en Kira sin cambiar el modo de vídeo.
//!
//! Aquí es donde el lenguaje `.toby` "compone una aplicación": el tema ensambla
//! los botones a partir de las **tres piezas PNG** del kit AQUA, así que este
//! anfitrión es también la prueba visible de que Kira/TobyUI soporta PNG.
//!
//! Las apps a pantalla completa se abren desde el intérprete en modo texto;
//! las apps en modo ventana se alojan directamente en el escritorio Kira.

use crate::bios;
use crate::fs;
use crate::gfx::{self, Canvas};
use crate::ps2;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::app::App;
use super::demo;
use super::event::{Evento, Tecla};

/// Ruta por defecto si la orden no indica ninguna.
pub const RUTA_POR_DEFECTO: &str = "/usr/share/tobyui/demo.toby";

/// Monta y muestra la aplicación indicada. Devuelve un mensaje legible de error.
pub fn ver(ruta: &str) -> Result<(), String> {
    if !fs::montado() {
        return Err("no hay ningun disco montado".to_string());
    }
    let ruta = if ruta.is_empty() {
        RUTA_POR_DEFECTO
    } else {
        ruta
    };
    let manifiesto = if ruta.ends_with(".app") {
        Some(super::paquete::cargar(ruta)?)
    } else {
        None
    };
    // Una app con interfaz de texto recibe la consola del kernel; no se carga
    // parser, layout ni renderer Toby y no se crea ninguna ventana en Kira.
    if manifiesto
        .as_ref()
        .is_some_and(|m| m.ui.modo == super::manifest::ModoUi::Texto)
    {
        crate::kira::comp::cerrar_ventana_app();
        crate::kira::comp::ceder();
        crate::shell::terminal_loop();
        crate::kira::comp::recuperar();
        return Ok(());
    }
    let es_terminal = ruta == "terminal.app" || ruta.ends_with("/terminal.app");
    let es_textedit = ruta == "textedit.app" || ruta.ends_with("/textedit.app");
    if (es_terminal || es_textedit) && !crate::kira::comp::activo() {
        // Estas apps pertenecen al escritorio de Kira incluso si se lanzan
        // desde la consola VGA: se abren como ventanas, no a pantalla completa.
        crate::kira::comp::arrancar();
    }
    if (es_terminal || es_textedit) && !crate::kira::comp::activo() {
        return Err(
            "Kira no pudo activar el modo gráfico; la app no se abrirá a pantalla completa."
                .to_string(),
        );
    }
    if es_terminal && crate::kira::comp::enfocar_terminal() {
        return Ok(());
    }
    let gestor_inicio = ruta == "/system/apps/gestor_inicio.app";
    let ruta_app = String::from(ruta);
    let icono_app = manifiesto
        .as_ref()
        .and_then(|m| {
            m.identidad
                .icono
                .as_ref()
                .map(|icono| format!("{}/{}", ruta, icono))
        })
        .unwrap_or_else(|| String::from("/system/apps/toby_desktop.app/img/app.png"));
    let (ruta_doc, titulo) = if ruta.ends_with(".app") {
        let manifiesto = manifiesto.expect("app validada arriba");
        (
            format!("{}/{}", ruta, manifiesto.ui.principal),
            manifiesto.identidad.titulo,
        )
    } else {
        (String::from(ruta), String::from(ruta))
    };
    let es_finder = ruta.ends_with("/toby_finder.app");

    let elegido = gfx::find_mode(1024, 768)
        .or_else(|| gfx::find_mode(800, 600))
        .or_else(gfx::find_first_demo_mode);
    let (modo, mi) = match elegido {
        Some(v) => v,
        None => {
            return Err("este equipo no ofrece ningun modo VESA de 32 bpp.".to_string());
        }
    };

    // La aplicación se monta ANTES de cambiar de modo: si el documento tiene un
    // fallo, el aviso se queda en modo texto y no se parpadea la pantalla.
    if gestor_inicio {
        crate::kira::comp::ceder();
        let (documento, ctx, registro) = crate::inicio_apps::interfaz_gestor(mi.width, mi.height);
        let app = match App::nueva(&documento, mi.width, mi.height, registro) {
            Ok(a) => a,
            Err(e) => {
                crate::kira::comp::recuperar();
                return Err(e.resumen());
            }
        };
        return ejecutar(app, ctx, &titulo, modo, &mi);
    }

    let kira_activo = crate::kira::comp::activo();
    let (ancho_host, alto_host) = crate::kira::comp::tamano()
        .filter(|_| kira_activo)
        .map(|(w, h)| (w as u32, h as u32))
        .unwrap_or((mi.width, mi.height));
    let app = match App::desde_fs(&ruta_doc, ancho_host, alto_host, demo::registro()) {
        Ok(a) => a,
        Err(e) => {
            crate::kira::comp::recuperar();
            return Err(e.resumen());
        }
    };
    let titulo_ventana = if ruta.ends_with(".app") {
        titulo.clone()
    } else {
        String::from(app.nombre())
    };
    if kira_activo {
        if let Some((ancho, alto)) = app.tamano_ventana() {
            let mut app = app;
            app.cambiar_tamano(ancho, alto);
            return ejecutar_ventana(
                app,
                demo::Estado::default(),
                ancho,
                alto,
                es_finder,
                es_terminal,
                ruta_app,
                titulo_ventana,
                icono_app,
            );
        }
    }
    crate::kira::comp::ceder();
    let mut app = app;
    app.cambiar_tamano(mi.width, mi.height);
    ejecutar(app, demo::Estado::default(), &titulo, modo, &mi)
}

/// Registra `mode="window"` como superficie persistente de Kira. El
/// compositor mantiene su propio ciclo de entrada y sigue atendiendo el
/// escritorio mientras la app está abierta.
fn ejecutar_ventana(
    app: App<demo::Estado>,
    ctx: demo::Estado,
    ancho: u32,
    alto: u32,
    finder: bool,
    terminal: bool,
    ruta_app: String,
    titulo_app: String,
    icono_app: String,
) -> Result<(), String> {
    let Some(len) = (ancho as usize).checked_mul(alto as usize) else {
        return Err("el tamaño de la ventana Toby no es válido".to_string());
    };
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(len)
        .map_err(|_| "no hay memoria para la ventana Toby".to_string())?;
    pixels.resize(len, 0);

    let abierta = if finder {
        let ruta_carga = ruta_app.clone();
        let abierta = crate::kira::comp::abrir_ventana_finder(
            app, ctx, pixels, ancho, alto, ruta_app, titulo_app, icono_app,
        );
        if abierta.is_some() {
            crate::kira::comp::cargar_finder_inicial(&ruta_carga, ancho, alto);
        }
        abierta
    } else if terminal {
        crate::kira::comp::abrir_ventana_terminal(
            app, ctx, pixels, ancho, alto, ruta_app, titulo_app, icono_app,
        )
    } else {
        crate::kira::comp::abrir_ventana_app(
            app, ctx, pixels, ancho, alto, ruta_app, titulo_app, icono_app,
        )
    };
    abierta.ok_or_else(|| "no se pudo crear la ventana en Kira".to_string())?;
    Ok(())
}

fn ejecutar<C>(
    mut app: App<C>,
    mut ctx: C,
    titulo: &str,
    modo: u16,
    mi: &gfx::ModeInfo,
) -> Result<(), String> {
    if gfx::set_mode(modo).is_none() {
        crate::kira::comp::recuperar();
        return Err("no se pudo activar el modo grafico.".to_string());
    }

    // El firmware acaba de cambiar de modo de vídeo y puede haber dejado el 8042
    // otra vez a su gusto: se prepara el teclado y el ratón DESPUÉS del BIOS.
    let _ = ps2::init();
    let fuente = bios::get_font();
    let mut lienzo = Canvas::new(mi);
    let mut primero = true;
    let animar_progreso = app.tiene_progreso_animado();
    let mut ultimo_tick_progreso = crate::pit::ticks();

    crate::println!();
    crate::println!(
        "toby: {}  -  modo 0x{:04X} {}x{}x{} bpp  -  ESC vuelve al escritorio",
        titulo,
        modo,
        mi.width,
        mi.height,
        mi.bpp
    );

    loop {
        let eventos = app.sondear();
        let salir = eventos
            .iter()
            .any(|e| matches!(e, Evento::Tecla(Tecla::Escape)));
        let movido = eventos
            .iter()
            .any(|e| matches!(e, Evento::RatonMueve { .. }));
        let paso = app.atender_eventos(&eventos, &mut ctx);
        let ahora = crate::pit::ticks();
        let progreso_vencido = animar_progreso && ahora.wrapping_sub(ultimo_tick_progreso) >= 3;
        if primero || movido || paso.repintar() || progreso_vencido {
            if progreso_vencido && !primero && !movido && !paso.repintar() {
                app.pintar_progreso_animado(&mut lienzo, fuente.as_ref(), ahora);
            } else {
                app.pintar_en(&mut lienzo, fuente.as_ref(), ahora);
            }
            if progreso_vencido {
                ultimo_tick_progreso = ahora;
            }
            primero = false;
        }
        if salir || app.cerrada() {
            break;
        }
        core::hint::spin_loop();
    }

    bios::set_text_mode();
    crate::println!();
    crate::kira::comp::recuperar();
    Ok(())
}
