//! Kit de recursos PNG y piezas AQUA de Toby UI.
//!
//! Los botones del kit AQUA son **tres PNG** —`left_button_*`,
//! `body_button_*` y `right_button_*`— que se ensamblan en horizontal: los
//! extremos conservan su proporción nativa (258 × 409) y el cuerpo (30 × 409)
//! se estira, así el botón crece con el texto sin deformarse.
//!
//! Este módulo los carga del disco **una sola vez** (la primera vez que se usa
//! cada variante) y los guarda en una caché global, para que repintar por
//! hover/pulsado no vuelva a descomprimir PNGs.
//!
//! Todo pasa por `resource::cargar_png`, que limita los recursos al tema del
//! sistema, los fondos de pantalla o los bundles `.app` instalados.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::sync::SyncCell;

use super::resource::{self, Imagen};

/// Carpeta de recursos del tema por defecto dentro de `rockydisk.img`.
pub const RAIZ_TEMA: &str = "/usr/share/tobyui/themes/default/";

/// Variantes de color del kit AQUA (el índice es estable: el 0 es el azul).
pub static VARIANTES: [&str; 7] = [
    "blue", "green", "red", "orange", "yellow", "magenta", "white",
];

/// Índice de una variante (`""` o desconocida = azul del kit).
pub fn id_variante(variante: &str) -> usize {
    VARIANTES.iter().position(|v| *v == variante).unwrap_or(0)
}

/// Las tres piezas de un botón AQUA.
pub struct Piel {
    /// Extremo izquierdo (258 × 409 nativo).
    pub izq: Imagen,
    /// Cuerpo estirable (30 × 409 nativo).
    pub cuerpo: Imagen,
    /// Extremo derecho (258 × 409 nativo).
    pub der: Imagen,
}

struct Estado {
    /// Piel de cada variante (se carga la primera vez que se usa).
    botones: Vec<Option<Piel>>,
    botones_cargados: Vec<bool>,
    /// Las mismas pieles giradas para controles verticales, como el scroll bar.
    botones_verticales: Vec<Option<Piel>>,
    botones_verticales_cargados: Vec<bool>,
    /// Imágenes sueltas del documento, por ruta.
    /// La caja mantiene estable la dirección de la imagen aunque `imagenes`
    /// tenga que realocar su vector al cargar más recursos.
    imagenes: Vec<Box<(String, Option<Imagen>)>>,
    /// Piezas sueltas del kit, por nombre de fichero (`None` = no está en el kit).
    /// Cada entrada está en su propia caja porque `pieza` expone referencias
    /// estáticas que deben sobrevivir al crecimiento de este vector.
    piezas: Vec<Box<(String, Option<Imagen>)>>,
    /// Pieles compuestas a partir de piezas sueltas, cacheadas para no volver
    /// a clonar sus buffers de píxeles ni filtrarlos en cada repintado.
    pieles_compuestas: Vec<Box<(String, Piel)>>,
}

static CACHE: SyncCell<Option<Estado>> = SyncCell::new(None);

/// Estado de la caché, creado la primera vez.
fn estado() -> &'static mut Estado {
    unsafe {
        let slot = &mut *CACHE.get();
        if slot.is_none() {
            let mut botones = Vec::new();
            let mut botones_verticales = Vec::new();
            let mut botones_cargados = Vec::new();
            let mut botones_verticales_cargados = Vec::new();
            for _ in 0..VARIANTES.len() {
                botones.push(None);
                botones_verticales.push(None);
                botones_cargados.push(false);
                botones_verticales_cargados.push(false);
            }
            let mut imagenes = Vec::new();
            let mut piezas = Vec::new();
            let mut pieles_compuestas = Vec::new();
            // Las colecciones de AppKit son pequeñas y conocidas. Reservar al
            // inicio evita que un repintado tenga que realojar estos vectores.
            let _ = imagenes.try_reserve_exact(8);
            let _ = piezas.try_reserve_exact(64);
            let _ = pieles_compuestas.try_reserve_exact(8);
            *slot = Some(Estado {
                botones,
                botones_cargados,
                botones_verticales,
                botones_verticales_cargados,
                imagenes,
                piezas,
                pieles_compuestas,
            });
        }
        slot.as_mut().expect("kit inicializado")
    }
}

/// Accede a la caché sólo durante una sección crítica breve. La lectura de
/// disco, la decodificación PNG y las copias de píxeles se hacen fuera de ella:
/// abrir una app no debe impedir que Kira siga planificando y repintando.
fn con_estado<R>(f: impl FnOnce(&mut Estado) -> R) -> R {
    crate::tarea::critica(|| f(estado()))
}

/// Carga las tres piezas de una variante desde el disco.
fn cargar_piel(color: &str) -> Option<Piel> {
    let izq =
        resource::cargar_png(&alloc::format!("{}left_button_{}.png", RAIZ_TEMA, color)).ok()?;
    let cuerpo =
        resource::cargar_png(&alloc::format!("{}body_button_{}.png", RAIZ_TEMA, color)).ok()?;
    let der =
        resource::cargar_png(&alloc::format!("{}right_button_{}.png", RAIZ_TEMA, color)).ok()?;
    Some(Piel { izq, cuerpo, der })
}

fn girar_imagen_90(img: &Imagen) -> Option<Imagen> {
    let cantidad = (img.ancho as usize).checked_mul(img.alto as usize)?;
    if img.ancho == 0 || img.alto == 0 || img.pixeles.len() < cantidad {
        return None;
    }
    let mut pixeles = Vec::new();
    pixeles.try_reserve_exact(cantidad).ok()?;
    pixeles.resize(cantidad, 0);
    for y in 0..img.alto {
        for x in 0..img.ancho {
            let destino_x = img.alto - 1 - y;
            let destino_y = x;
            let origen = y as usize * img.ancho as usize + x as usize;
            let destino = destino_y as usize * img.alto as usize + destino_x as usize;
            pixeles[destino] = img.pixeles[origen];
        }
    }
    Some(Imagen {
        ancho: img.alto,
        alto: img.ancho,
        pixeles,
    })
}

/// Piel de la variante pedida, cargándola del disco la primera vez.
///
/// Devuelve `None` si el kit no está en el disco (entonces el tema cae a su
/// relleno sólido de siempre).
pub fn piel(variante: &str) -> Option<&'static Piel> {
    let id = id_variante(variante);
    let (intentada, cacheada) = con_estado(|est| {
        let cacheada = est.botones[id]
            .as_ref()
            .map(|p| unsafe { &*(p as *const Piel) });
        (est.botones_cargados[id], cacheada)
    });
    if intentada {
        return cacheada;
    }
    let cargada = cargar_piel(VARIANTES[id]);
    con_estado(|est| {
        if !est.botones_cargados[id] {
            est.botones[id] = cargada;
            est.botones_cargados[id] = true;
        }
        est.botones[id]
            .as_ref()
            .map(|p| unsafe { &*(p as *const Piel) })
    })
}

/// Piel AppKit azul rotada para dibujar el mismo botón en orientación vertical.
/// Los PNG se transforman una sola vez y quedan cacheados en el kit.
pub fn piel_vertical(variante: &str) -> Option<&'static Piel> {
    let id = id_variante(variante);
    let (intentada, cacheada) = con_estado(|est| {
        let cacheada = est.botones_verticales[id]
            .as_ref()
            .map(|p| unsafe { &*(p as *const Piel) });
        (est.botones_verticales_cargados[id], cacheada)
    });
    if intentada {
        return cacheada;
    }
    let vertical = piel(variante).and_then(|p| {
        Some(Piel {
            izq: girar_imagen_90(&p.izq)?,
            cuerpo: girar_imagen_90(&p.cuerpo)?,
            der: girar_imagen_90(&p.der)?,
        })
    });
    con_estado(|est| {
        if !est.botones_verticales_cargados[id] {
            est.botones_verticales[id] = vertical;
            est.botones_verticales_cargados[id] = true;
        }
        est.botones_verticales[id]
            .as_ref()
            .map(|p| unsafe { &*(p as *const Piel) })
    })
}

/// Imagen suelta de un recurso (`<image src="...">`), cacheada por ruta.
pub fn imagen_recurso(ruta: &str) -> Option<&'static Imagen> {
    if let Some(cacheada) = con_estado(|est| {
        est.imagenes
            .iter()
            .find(|entrada| entrada.0 == ruta)
            .map(|entrada| {
                entrada
                    .1
                    .as_ref()
                    .map(|img| unsafe { &*(img as *const Imagen) })
            })
    }) {
        return cacheada;
    }
    let img = resource::cargar_png(ruta).ok();
    let nueva = Box::new((String::from(ruta), img));
    con_estado(|est| {
        if let Some(entrada) = est.imagenes.iter().find(|entrada| entrada.0 == ruta) {
            return entrada
                .1
                .as_ref()
                .map(|img| unsafe { &*(img as *const Imagen) });
        }
        est.imagenes.push(nueva);
        let entrada = est.imagenes.last().expect("recién añadida");
        entrada
            .1
            .as_ref()
            .map(|img| unsafe { &*(img as *const Imagen) })
    })
}

/// Precarga las pieles e imágenes propias de la ventana Finder. Debe llamarse
/// desde una tarea normal antes de publicar/repintar la ventana, no desde el
/// compositor: la primera lectura y decodificación PNG puede tardar bastante.
pub fn precalentar_finder() {
    let _ = piel("green");
    let _ = piel("white");
    let _ = piel("blue");
    // La esfera azul solo se solicita al entrar en hover; la blanca se usa
    // en reposo. Ambas deben estar residentes antes de recibir movimiento.
    let _ = esfera("white");
    let _ = esfera("blue");
    let _ = piel_vertical("blue");
    let _ = indicador_izquierda();
    let _ = indicador_derecha();
    for indice in 0..4 {
        let _ = icono_vista_finder(indice);
    }
    let _ = segmentado("blue");
    let _ = segmentado("white");
    let _ = shooter();
}

// --- Piezas sueltas del kit (Fase 10: controles) --------------------------
//
// El kit AQUA exportó los controles como imágenes sueltas y con dos
// peculiaridades que se centralizan AQUÍ para que el resto del código hable
// sólo de colores:
//   * el blanco del segmentado se llama `withe` (error del exportador);
//   * no todos los colores traen todas las piezas (no hay check blanco ni
//     pulgar de gota blanco: se caen al azul del kit).
// Todas pasan por `resource::cargar_png`, así que sólo se admiten ficheros
// de `/usr/share/tobyui/` (en rocky, dentro de `rockydisk.img`).

/// Color del kit para una variante (vacío o desconocido = azul).
fn color_de(variante: &str) -> &'static str {
    match VARIANTES.iter().position(|v| *v == variante) {
        Some(i) => VARIANTES[i],
        None => VARIANTES[0],
    }
}

/// Pieza suelta del kit por nombre de fichero, cacheada (se carga una vez).
///
/// `None` = el kit no la trae; el que pinta decide su respaldo.
pub fn pieza(nombre: &str) -> Option<&'static Imagen> {
    if let Some(cacheada) = con_estado(|est| {
        est.piezas
            .iter()
            .find(|entrada| entrada.0 == nombre)
            .map(|entrada| {
                entrada
                    .1
                    .as_ref()
                    .map(|img| unsafe { &*(img as *const Imagen) })
            })
    }) {
        return cacheada;
    }
    let cargada = resource::cargar_png(&format!("{}{}", RAIZ_TEMA, nombre)).ok();
    let nueva = Box::new((String::from(nombre), cargada));
    con_estado(|est| {
        if let Some(entrada) = est.piezas.iter().find(|entrada| entrada.0 == nombre) {
            return entrada
                .1
                .as_ref()
                .map(|img| unsafe { &*(img as *const Imagen) });
        }
        est.piezas.push(nueva);
        est.piezas
            .last()
            .expect("recién añadida")
            .1
            .as_ref()
            .map(|img| unsafe { &*(img as *const Imagen) })
    })
}

/// Tres piezas ensamblables del kit, cacheadas por nombre.
fn tres_piezas(izq: &str, cuerpo: &str, der: &str) -> Option<&'static Piel> {
    let clave = format!("{}\0{}\0{}", izq, cuerpo, der);
    if let Some(cacheada) = con_estado(|est| {
        est.pieles_compuestas
            .iter()
            .find(|entrada| entrada.0 == clave)
            .map(|entrada| unsafe { &*(&entrada.1 as *const Piel) })
    }) {
        return Some(cacheada);
    }
    let piel = Piel {
        izq: pieza(izq)?.clone(),
        cuerpo: pieza(cuerpo)?.clone(),
        der: pieza(der)?.clone(),
    };
    let nueva = Box::new((clave.clone(), piel));
    con_estado(|est| {
        if let Some(entrada) = est
            .pieles_compuestas
            .iter()
            .find(|entrada| entrada.0 == clave)
        {
            return Some(unsafe { &*(&entrada.1 as *const Piel) });
        }
        est.pieles_compuestas.push(nueva);
        let entrada = est.pieles_compuestas.last().expect("piel recién añadida");
        Some(unsafe { &*(&entrada.1 as *const Piel) })
    })
}

/// Casilla de un `<check>`: `<color>_check.png`.
///
/// El kit no exportó el check blanco: se usa el azul del kit (el CSS de AQUA
/// tampoco define `.aqua-check--white`).
pub fn casilla(variante: &str) -> Option<&'static Imagen> {
    pieza(&format!("{}_check.png", color_activo(variante)))
}

/// Esfera de un `<radio>`: `<color>_radio.png`.
///
/// El radio tampoco tiene variante blanca en el CSS (sí la tiene el
/// `circular`), así que el blanco cae al azul del kit.
pub fn esfera(variante: &str) -> Option<&'static Imagen> {
    pieza(&format!("{}_radio.png", color_de(variante)))
}

/// Esfera de color de un control ACTIVO: la esfera de un `<radio>` marcado o
/// el pulgar de un `<slider>`; con la caída al azul del kit (arriba).
pub fn esfera_activa(variante: &str) -> Option<&'static Imagen> {
    pieza(&format!("{}_radio.png", color_activo(variante)))
}

/// Casilla o esfera GRISES del kit (`gray_check.png` / `gray_radio.png`).
///
/// Son la del `checked="false"` del HTML y, de paso, la del `enabled="false"`
/// (el CSS apaga con `grayscale`, que es lo más parecido que hay en piezas).
pub fn gris(esfera: bool) -> Option<&'static Imagen> {
    if esfera {
        pieza("gray_radio.png")
    } else {
        pieza("gray_check.png")
    }
}

/// Color ACTIVO de un control comparado (`checked`/`selected`).
///
/// El CSS de AQUA sólo define la variante blanca para el botón y para el
/// `circular`: en check, radio, segmented, popup y slider el blanco cae al
/// azul del kit. Desconocido = azul, como siempre.
pub fn color_activo(variante: &str) -> &'static str {
    let c = color_de(variante);
    if c == "white" {
        "blue"
    } else {
        c
    }
}

/// Punto interior de un radio marcado (`dot.png`).
pub fn punto() -> Option<&'static Imagen> {
    pieza("dot.png")
}

/// Marca de un check marcado (`check_v.png`).
pub fn marca() -> Option<&'static Imagen> {
    pieza("check_v.png")
}

/// Pulgar de gota de un `<slider-gota>`: `slider_thumb_<color>.png`.
pub fn gota(variante: &str) -> Option<&'static Imagen> {
    let color = color_de(variante);
    if color == "blue" || color == "white" {
        // El azul sale sin sufijo y el blanco no existe: el mismo pulgar.
        return pieza("slider_thumb.png");
    }
    pieza(&format!("slider_thumb_{}.png", color))
}

/// Flecha de bajada de un `<popup>` (`arrow_down.png`).
pub fn flecha() -> Option<&'static Imagen> {
    pieza("arrow_down.png")
}

/// Flecha de subida de un `<popup>` (`arrow up.png`, con espacio: ese es el
/// nombre con el que la exportó el kit).
pub fn flecha_arriba() -> Option<&'static Imagen> {
    pieza("arrow up.png")
}

/// Indicador izquierdo del navegador de Finder (`indicator_left.png`).
pub fn indicador_izquierda() -> Option<&'static Imagen> {
    pieza("indicator_left.png")
}

/// Indicador derecho del navegador de Finder (`indicator_right.png`).
pub fn indicador_derecha() -> Option<&'static Imagen> {
    pieza("indicator_right.png")
}

/// Iconos PNG nativos del selector de vistas de Finder: iconos, lista,
/// columnas y galería, en ese orden.
pub fn icono_vista_finder(indice: usize) -> Option<&'static Imagen> {
    let nombre = match indice {
        0 => "icon_view.png",
        1 => "list_view.png",
        2 => "column_view.png",
        3 => "gallery_view.png",
        _ => return None,
    };
    pieza(nombre)
}

/// Tirador de esquina inferior derecha para el pie gris de las ventanas.
pub fn shooter() -> Option<&'static Imagen> {
    pieza("shooter.png")
}

/// Carril gris de un `<slider>` o del fondo de un `<field>`.
///
/// Tres piezas como el botón. Los nombres del kit traen un espacio antes de
/// los extremos (`graybar_ left.png`): aquí se escribe tal cual están.
pub fn carril() -> Option<&'static Piel> {
    tres_piezas(
        "graybar_ left.png",
        "graybar_body.png",
        "graybar_ right.png",
    )
}

/// Marco de un `<segmented>`: `segmented_<color>_{left,body,right}.png`.
pub fn segmentado(variante: &str) -> Option<&'static Piel> {
    // El blanco del kit se exportó como `withe` (sic).
    let color = if color_de(variante) == "white" {
        "withe"
    } else {
        color_de(variante)
    };
    tres_piezas(
        &format!("segmented_{}_left.png", color),
        &format!("segmented_{}_body.png", color),
        &format!("segmented_{}_right.png", color),
    )
}
