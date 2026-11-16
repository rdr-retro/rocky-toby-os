//! Adaptador del efecto Genie compartido de AppKit para las ventanas Kira.
//!
//! AppKit genera la malla y muestrea la superficie; Kira sólo aporta su clip
//! de esquinas redondeadas y el blend contra el fondo actual.

use crate::gfx::Canvas;
use crate::toby_ui::renderer;
use toby_appkit::genie::{self, Rect};

/// Dibuja una superficie con deformación Genie entre dos rectángulos.
///
/// `origen` y `destino` son las posiciones en pantalla de los extremos de la
/// animación; el contenido muestreado siempre es la superficie completa.
pub fn pintar(
    canvas: &Canvas,
    pixeles: &[u32],
    ancho: u32,
    alto: u32,
    origen: Rect,
    destino: Rect,
    progreso: u16,
) {
    let radio = renderer::RADIO_ESQUINA_VENTANA.min(ancho / 2).min(alto / 2);
    genie::rasterizar(
        pixeles,
        ancho,
        alto,
        canvas.width,
        canvas.height,
        origen,
        destino,
        progreso,
        |x, y, fuente_x, fuente_y, color| {
            let cobertura = cobertura_esquina(fuente_x, fuente_y, ancho, radio);
            if cobertura == 0 {
                canvas.put(x, y, color);
            } else if cobertura < 64 {
                canvas.put(x, y, mezclar_con_fondo(color, canvas.get(x, y), cobertura));
            }
        },
    );
}

fn cobertura_esquina(x: u32, y: u32, ancho: u32, radio: u32) -> u32 {
    if radio == 0 || y >= radio {
        return 0;
    }
    if x < radio {
        renderer::cobertura_exterior_esquina(y, x, radio)
    } else if x >= ancho - radio {
        renderer::cobertura_exterior_esquina(y, ancho - 1 - x, radio)
    } else {
        0
    }
}

fn mezclar_con_fondo(ventana: u32, fondo: u32, cobertura: u32) -> u32 {
    let canal = |desplazamiento: u32| {
        let frente = (ventana >> desplazamiento) & 0xFF;
        let atras = (fondo >> desplazamiento) & 0xFF;
        (frente * (64 - cobertura) + atras * cobertura + 32) / 64
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}
