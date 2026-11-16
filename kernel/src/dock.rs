//! Dock del escritorio: componente independiente que Kira mantiene visible.
//! Las apps fijadas permanecen; los iconos de apps abiertas se añaden mientras
//! exista al menos una ventana de esa app.

use crate::gfx::{self, Canvas};
use crate::toby_ui::layout::Rect;
use crate::toby_ui::renderer::{CanvasRenderer, Renderer};
use core::sync::atomic::{AtomicBool, Ordering};

const ALTO_MAXIMO: i32 = 64;
const MARGEN_LATERAL: i32 = 16;
const MARGEN_INFERIOR: i32 = 0;
const RELLENO: i32 = 8;
const SEPARACION: i32 = 8;
const LADO_ICONO: i32 = 48;
pub const ICONO_APP_GENERICO: &str = "/system/apps/toby_desktop.app/img/app.png";
const INDICADOR_APP_ABIERTA: &str = "/system/apps/dock.app/img/arrow up.png";
const PAPELERA_RUTA: &str = "/home/papelera";
const PAPELERA_VACIA: &str = "/system/apps/dock.app/img/trash_empty.png";
const PAPELERA_LLENA: &str = "/system/apps/dock.app/img/trash_full.png";

static PAPELERA_TIENE_ELEMENTOS: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct AplicacionAbierta<'a> {
    pub ruta: &'a str,
    pub icono: &'a str,
    /// Presente para una ventana minimizada: su superficie se conserva como
    /// captura y se reduce al pintar la miniatura en el Dock.
    pub miniatura: Option<VentanaMiniatura<'a>>,
}

#[derive(Clone, Copy)]
pub struct VentanaMiniatura<'a> {
    pub id: u32,
    pub pixeles: &'a [u32],
    pub ancho: u32,
    pub alto: u32,
    /// Oculta la captura durante la minimización; la celda ya está reservada.
    pub mostrar: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjetivoDock<'a> {
    pub ruta: &'a str,
    /// Una miniatura vuelve a abrir exactamente esa ventana; `None` activa la app.
    pub ventana_id: Option<u32>,
}

struct AplicacionAnclada {
    ruta: &'static str,
    icono: &'static str,
}

#[derive(Clone, Copy)]
struct Entrada<'a> {
    ruta: &'a str,
    icono: &'a str,
    abierta: bool,
    papelera: bool,
    miniatura: Option<VentanaMiniatura<'a>>,
    ventana_id: Option<u32>,
}

// Toby Finder permanece anclado aunque se cierre. Las demás apps sólo ocupan
// una celda mientras tengan alguna ventana abierta.
const APLICACIONES: &[AplicacionAnclada] = &[AplicacionAnclada {
    ruta: "/system/apps/toby_finder.app",
    icono: "/system/apps/toby_finder.app/img/toby_finder.png",
}];

/// Pinta anclajes y apps abiertas. Los duplicados de una app con varias
/// ventanas se reúnen en un solo icono.
pub fn pintar(
    c: &mut Canvas,
    abiertas: &[Option<AplicacionAbierta<'_>>],
    rebote: Option<(&str, i32)>,
) {
    let cantidad = cantidad_iconos(abiertas);
    let papelera_llena = PAPELERA_TIENE_ELEMENTOS.load(Ordering::Relaxed);
    let marco = rect_dock(c.width, c.height, cantidad);
    if marco.w < 3 || marco.h < 3 {
        return;
    }
    let azul = gfx::rgb(25, 91, 190);
    let borde = gfx::rgb(164, 211, 255);

    c.fill_rect(marco.x, marco.y, marco.w as i32, marco.h as i32, azul);
    // Borde de un píxel arriba y en los laterales; queda abierto abajo.
    c.fill_rect(marco.x, marco.y, marco.w as i32, 1, borde);
    c.fill_rect(marco.x, marco.y, 1, marco.h as i32, borde);
    c.fill_rect(marco.der() - 1, marco.y, 1, marco.h as i32, borde);

    let (ancho_pantalla, alto_pantalla) = (c.width, c.height);
    let mut r = CanvasRenderer::nuevo(c, None);
    for indice in 0..cantidad {
        let (Some(entrada), Some(mut rect)) = (
            entrada(indice, abiertas),
            rect_icono(indice, cantidad, ancho_pantalla, alto_pantalla),
        ) else {
            continue;
        };
        if let Some((_, offset_y)) = rebote.filter(|(ruta, _)| *ruta == entrada.ruta) {
            rect.y = rect.y.saturating_add(offset_y);
        }
        if entrada.miniatura.is_none() {
            let imagen = if entrada.papelera {
                crate::toby_ui::kit::imagen_recurso(if papelera_llena {
                    PAPELERA_LLENA
                } else {
                    PAPELERA_VACIA
                })
            } else {
                crate::toby_ui::kit::imagen_recurso(entrada.icono)
                    .or_else(|| crate::toby_ui::kit::imagen_recurso(ICONO_APP_GENERICO))
            };
            if let Some(imagen) = imagen {
                r.imagen(rect, imagen);
            }
        }
    }
    drop(r);
    for indice in 0..cantidad {
        let (Some(entrada), Some(rect)) = (
            entrada(indice, abiertas),
            rect_icono(indice, cantidad, ancho_pantalla, alto_pantalla),
        ) else {
            continue;
        };
        if let Some(miniatura) = entrada.miniatura.filter(|miniatura| miniatura.mostrar) {
            pintar_miniatura(c, rect, miniatura);
        }
    }
    if let Some(flecha) = crate::toby_ui::kit::imagen_recurso(INDICADOR_APP_ABIERTA) {
        let dock = rect_dock(ancho_pantalla, alto_pantalla, cantidad);
        let mut r = CanvasRenderer::nuevo(c, None);
        for indice in 0..cantidad {
            let (Some(entrada), Some(rect)) = (
                entrada(indice, abiertas),
                rect_icono(indice, cantidad, ancho_pantalla, alto_pantalla),
            ) else {
                continue;
            };
            if entrada.abierta && entrada.miniatura.is_none() {
                // La flecha reemplaza la rayita y queda centrada bajo el icono,
                // aprovechando sólo el espacio disponible dentro del Dock.
                let espacio_abajo = dock.y + dock.h as i32 - rect.abajo() as i32;
                let alto = 6.min(espacio_abajo - 2).max(0);
                let ancho = (alto * 9 / 6).min(rect.w as i32);
                if ancho > 0 && alto > 0 {
                    let x = rect.x + (rect.w as i32 - ancho) / 2;
                    let y = rect.abajo() as i32 - 1;
                    r.imagen(
                        Rect {
                            x,
                            y,
                            w: ancho as u32,
                            h: alto as u32,
                        },
                        flecha,
                    );
                }
            }
        }
    }
}

/// Devuelve qué icono del Dock contiene el punto, si lo hay.
pub fn indice_icono_dock(
    x: i32,
    y: i32,
    ancho_pantalla: i32,
    alto_pantalla: i32,
    abiertas: &[Option<AplicacionAbierta<'_>>],
) -> Option<usize> {
    let cantidad = cantidad_iconos(abiertas);
    (0..cantidad).find(|&indice| {
        rect_icono(indice, cantidad, ancho_pantalla, alto_pantalla)
            .is_some_and(|rect| rect.contiene(x, y))
    })
}

/// Impide comenzar una selección sobre el marco o el espacio interior del Dock.
pub fn dentro_dock(
    x: i32,
    y: i32,
    ancho_pantalla: i32,
    alto_pantalla: i32,
    abiertas: &[Option<AplicacionAbierta<'_>>],
) -> bool {
    rect_dock(ancho_pantalla, alto_pantalla, cantidad_iconos(abiertas)).contiene(x, y)
}

/// Alto que ocupa el Dock, útil para maximizar ventanas sin cubrirlo.
pub fn alto_reservado(alto_pantalla: i32) -> i32 {
    rect_dock(1, alto_pantalla, APLICACIONES.len() + 1).h as i32
}

/// Precarga las dos variantes fuera de la sección crítica del compositor.
pub fn precalentar_papelera() {
    let _ = crate::toby_ui::kit::imagen_recurso(PAPELERA_VACIA);
    let _ = crate::toby_ui::kit::imagen_recurso(PAPELERA_LLENA);
}

/// Comprueba el contenido de `/home/papelera` desde una tarea normal. Devuelve
/// `true` sólo cuando cambia el estado, para que Kira repinte únicamente entonces.
pub fn actualizar_estado_papelera() -> bool {
    let llena = if crate::fs::montado() {
        crate::fs::resolver(crate::fs::raiz(), PAPELERA_RUTA)
            .ok()
            .filter(|&registro| crate::fs::tipo(registro) == crate::fs::Tipo::Directorio)
            .is_some_and(|registro| {
                let mut tiene_elementos = false;
                crate::fs::listar_hasta(registro, |_| {
                    tiene_elementos = true;
                    false
                });
                tiene_elementos
            })
    } else {
        false
    };
    PAPELERA_TIENE_ELEMENTOS.swap(llena, Ordering::Relaxed) != llena
}

pub fn objetivo_dock<'a>(
    indice: usize,
    abiertas: &'a [Option<AplicacionAbierta<'a>>],
) -> Option<ObjetivoDock<'a>> {
    entrada(indice, abiertas)
        .filter(|app| !app.papelera)
        .map(|app| ObjetivoDock {
            ruta: app.ruta,
            ventana_id: app.ventana_id,
        })
}

/// Rectángulo de pantalla del icono asociado a una ruta del Dock.
/// Kira lo usa como ancla de las animaciones Genie de minimizar/restaurar.
pub fn rect_icono_ruta(
    ruta: &str,
    ancho_pantalla: i32,
    alto_pantalla: i32,
    abiertas: &[Option<AplicacionAbierta<'_>>],
) -> Option<Rect> {
    let cantidad = cantidad_iconos(abiertas);
    (0..cantidad).find_map(|indice| {
        let coincide = entrada(indice, abiertas)
            .is_some_and(|app| !app.papelera && app.ruta == ruta && app.miniatura.is_none());
        coincide.then(|| rect_icono(indice, cantidad, ancho_pantalla, alto_pantalla))?
    })
}

/// Rectángulo de la miniatura correspondiente a una ventana concreta.
pub fn rect_icono_ventana(
    id: u32,
    ancho_pantalla: i32,
    alto_pantalla: i32,
    abiertas: &[Option<AplicacionAbierta<'_>>],
) -> Option<Rect> {
    let cantidad = cantidad_iconos(abiertas);
    (0..cantidad).find_map(|indice| {
        let coincide = entrada(indice, abiertas)
            .is_some_and(|app| app.miniatura.is_some_and(|miniatura| miniatura.id == id));
        coincide.then(|| rect_icono(indice, cantidad, ancho_pantalla, alto_pantalla))?
    })
}

fn cantidad_iconos(abiertas: &[Option<AplicacionAbierta<'_>>]) -> usize {
    APLICACIONES.len() + cantidad_dinamicas(abiertas) + 1
}

fn cantidad_dinamicas(abiertas: &[Option<AplicacionAbierta<'_>>]) -> usize {
    abiertas
        .iter()
        .enumerate()
        .filter(|(indice, entrada)| {
            let Some(app) = entrada else { return false };
            if app.miniatura.is_some() {
                true
            } else {
                !esta_anclada(app.ruta)
                    && !abiertas[..*indice]
                        .iter()
                        .flatten()
                        .any(|previa| previa.miniatura.is_none() && previa.ruta == app.ruta)
            }
        })
        .count()
}

fn entrada<'a>(
    indice: usize,
    abiertas: &'a [Option<AplicacionAbierta<'a>>],
) -> Option<Entrada<'a>> {
    if let Some(anclada) = APLICACIONES.get(indice) {
        let abierta = abiertas
            .iter()
            .flatten()
            .any(|app| app.ruta == anclada.ruta);
        return Some(Entrada {
            ruta: anclada.ruta,
            icono: anclada.icono,
            abierta,
            papelera: false,
            miniatura: None,
            ventana_id: None,
        });
    }

    let destino = indice.checked_sub(APLICACIONES.len())?;
    let dinamicas = cantidad_dinamicas(abiertas);
    if destino == dinamicas {
        return Some(Entrada {
            ruta: PAPELERA_RUTA,
            icono: if PAPELERA_TIENE_ELEMENTOS.load(Ordering::Relaxed) {
                PAPELERA_LLENA
            } else {
                PAPELERA_VACIA
            },
            abierta: false,
            papelera: true,
            miniatura: None,
            ventana_id: None,
        });
    }
    if destino > dinamicas {
        return None;
    }
    let mut posicion = 0;
    for (indice_abierta, app) in abiertas.iter().enumerate() {
        let Some(app) = app else { continue };
        if app.miniatura.is_none()
            && (esta_anclada(app.ruta)
                || abiertas[..indice_abierta]
                    .iter()
                    .flatten()
                    .any(|previa| previa.miniatura.is_none() && previa.ruta == app.ruta))
        {
            continue;
        }
        if posicion == destino {
            return Some(Entrada {
                ruta: app.ruta,
                icono: app.icono,
                abierta: true,
                papelera: false,
                miniatura: app.miniatura,
                ventana_id: app.miniatura.map(|miniatura| miniatura.id),
            });
        }
        posicion += 1;
    }
    None
}

fn esta_anclada(ruta: &str) -> bool {
    APLICACIONES.iter().any(|app| app.ruta == ruta)
}

fn rect_dock(ancho_pantalla: i32, alto_pantalla: i32, cantidad: usize) -> Rect {
    let cantidad = cantidad as i32;
    let separaciones = cantidad.saturating_sub(1) * SEPARACION;
    let ancho_contenido = cantidad * LADO_ICONO + separaciones + RELLENO * 2;
    let disponible = ancho_pantalla.saturating_sub(MARGEN_LATERAL * 2).max(0);
    let ancho = ancho_contenido.min(disponible);
    let alto = ALTO_MAXIMO.min(alto_pantalla.saturating_sub(MARGEN_INFERIOR));
    Rect {
        x: (ancho_pantalla - ancho) / 2,
        y: alto_pantalla - MARGEN_INFERIOR - alto,
        w: ancho.max(0) as u32,
        h: alto.max(0) as u32,
    }
}

fn rect_icono(
    indice: usize,
    cantidad: usize,
    ancho_pantalla: i32,
    alto_pantalla: i32,
) -> Option<Rect> {
    if indice >= cantidad || cantidad == 0 {
        return None;
    }

    let dock = rect_dock(ancho_pantalla, alto_pantalla, cantidad);
    let ancho_interior = (dock.w as i32 - RELLENO * 2).max(0);
    // Al reducirse el Dock para una pantalla estrecha, primero se elimina el
    // espacio entre iconos y después se compacta cada icono para que quepa.
    let cantidad_i32 = cantidad as i32;
    let separacion = if cantidad > 1 {
        SEPARACION.min(ancho_interior.saturating_sub(cantidad_i32).max(0) / (cantidad_i32 - 1))
    } else {
        0
    };
    let ancho_celdas = ancho_interior - separacion * (cantidad_i32 - 1);
    let celda = ancho_celdas / cantidad_i32;
    let lado = LADO_ICONO.min(celda).min((dock.h as i32 - RELLENO).max(0));
    if lado <= 0 {
        return None;
    }

    let x_celda = dock.x + RELLENO + indice as i32 * (celda + separacion);
    Some(Rect {
        x: x_celda + (celda - lado) / 2,
        y: dock.y + (dock.h as i32 - lado) / 2,
        w: lado as u32,
        h: lado as u32,
    })
}

fn pintar_miniatura(c: &Canvas, rect: Rect, miniatura: VentanaMiniatura<'_>) {
    let ancho = miniatura.ancho as usize;
    let alto = miniatura.alto as usize;
    let Some(necesarios) = ancho.checked_mul(alto) else {
        return;
    };
    if ancho == 0 || alto == 0 || miniatura.pixeles.len() < necesarios || rect.w < 8 || rect.h < 8 {
        return;
    }

    let (destino_ancho, destino_alto) =
        if ancho as u64 * rect.h as u64 > alto as u64 * rect.w as u64 {
            let alto = ((rect.w as u64 * alto as u64) / ancho as u64).max(1) as u32;
            (rect.w, alto)
        } else {
            let ancho = ((rect.h as u64 * ancho as u64) / alto as u64).max(1) as u32;
            (ancho, rect.h)
        };
    let x0 = rect.x + (rect.w as i32 - destino_ancho as i32) / 2;
    let y0 = rect.y + (rect.h as i32 - destino_alto as i32) / 2;
    let radio = crate::toby_ui::renderer::RADIO_ESQUINA_VENTANA
        .min(miniatura.ancho / 2)
        .min(miniatura.alto / 2);
    // Las esquinas transparentes se componen sobre el Dock azul, sin marco ni
    // placa propia: la captura queda integrada en el fondo como en macOS.
    let mate = gfx::rgb(25, 91, 190);
    for dy in 0..destino_alto {
        let sy = ((dy as u64 * alto as u64) / destino_alto as u64) as usize;
        for dx in 0..destino_ancho {
            let sx = ((dx as u64 * ancho as u64) / destino_ancho as u64) as usize;
            let columna_esquina = if sx < radio as usize {
                Some(sx as u32)
            } else if sx >= ancho.saturating_sub(radio as usize) {
                Some((ancho - 1 - sx) as u32)
            } else {
                None
            };
            let exterior = if sy < radio as usize {
                columna_esquina.map_or(0, |columna| {
                    crate::toby_ui::renderer::cobertura_exterior_esquina(sy as u32, columna, radio)
                })
            } else {
                0
            };
            if exterior == 64 {
                continue;
            }
            let pixel = miniatura.pixeles[sy * ancho + sx];
            c.put(
                x0 + dx as i32,
                y0 + dy as i32,
                if exterior == 0 {
                    pixel
                } else {
                    mezclar_exterior_miniatura(pixel, mate, exterior)
                },
            );
        }
    }
}

/// La superficie de Kira guarda la cobertura exterior de las esquinas contra
/// negro. Al mostrarla reducida, recompone esa fracción sobre el azul del Dock
/// para conservar el borde transparente sin un halo oscuro.
fn mezclar_exterior_miniatura(pixel: u32, mate: u32, exterior: u32) -> u32 {
    let canal = |desplazamiento: u32| {
        let frente = (pixel >> desplazamiento) & 0xFF;
        let fondo = (mate >> desplazamiento) & 0xFF;
        (frente + (fondo * exterior + 32) / 64).min(0xFF)
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}
