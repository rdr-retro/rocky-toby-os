//! Transformación de malla entera para animaciones Genie.
//!
//! Reimplementación `no_std` del modelo geométrico publicado por Harshil Shah,
//! inspirada inicialmente por la demo SwiftUI/Metal `genie-main` de Alex Widua
//! y Janum Trivedi. No usa Metal, SIMD ni punto flotante: Kira y las apps Toby
//! pueden compartir exactamente el mismo mapeo Q16.
//!
//! Modelo de embudo en dos fases: el borde cercano se desliza durante la mitad
//! inicial, la traslación global empieza al 40 %, y los laterales siguen una
//! curva cuadrática. Sigue la reconstrucción publicada por Harshil Shah del
//! movimiento visible de macOS; no pretende copiar la implementación privada
//! de WindowServer.

const UNIDAD: i64 = 65_535;
const ESCALA_Q16: i64 = 1 << 16;
const INICIO_TRASLACION: i64 = (UNIDAD * 2) / 5;

/// Rectángulo de pantalla en píxeles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Punto de pantalla en formato fijo Q16 (un píxel equivale a `1 << 16`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

/// Progreso normalizado a partir del reloj PIT de 100 Hz.
///
/// `duracion == 0` significa completar inmediatamente. El cálculo usa
/// `wrapping_sub` para tolerar el desbordamiento del contador.
pub fn progreso_ticks(ahora: u64, inicio: u64, duracion: u64) -> u16 {
    if duracion == 0 {
        return u16::MAX;
    }
    let transcurrido = ahora.wrapping_sub(inicio).min(duracion);
    ((transcurrido as u128 * UNIDAD as u128) / duracion as u128) as u16
}

/// Mapea un vértice de la superficie original al fotograma actual.
///
/// `u`, `v` y `progreso` son valores normalizados `0..=65535`; el resultado
/// son coordenadas Q16. `source` y `target` representan la posición y tamaño
/// completos de la superficie en los extremos de la animación.
pub fn map_point(source: Rect, target: Rect, u: u16, v: u16, progreso: u16) -> Point {
    let p = (progreso as i64).clamp(0, UNIDAD);
    if p == 0 {
        return Point {
            x: coordenada_x(source, u),
            y: coordenada_y(source, v),
        };
    }
    if p == UNIDAD {
        return Point {
            x: coordenada_x(target, u),
            y: coordenada_y(target, v),
        };
    }
    // La deformación lateral acaba al 50 %; la traslación arranca al 40 %.
    // Ese solapamiento es lo que evita que parezcan dos animaciones pegadas.
    let deslizamiento = (p * 2).min(UNIDAD);
    let traslacion = if p <= INICIO_TRASLACION {
        0
    } else {
        ((p - INICIO_TRASLACION) * UNIDAD) / (UNIDAD - INICIO_TRASLACION)
    };

    let source_top = coordenada_y(source, 0);
    let target_top = coordenada_y(target, 0);
    let source_bottom = coordenada_y(source, u16::MAX);
    let target_bottom = coordenada_y(target, u16::MAX);
    let desplazamiento_y = ((target_top - source_top) * traslacion) / UNIDAD;
    let top_y = source_top + desplazamiento_y;
    // El borde inferior alcanza el borde inferior del icono y se queda ahí.
    let bottom_y = (source_bottom + desplazamiento_y).min(target_bottom);
    let y = interpolar(top_y, bottom_y.max(top_y), v as i64);

    let source_left = coordenada_x(source, 0);
    let source_right = coordenada_x(source, u16::MAX);
    let target_left = coordenada_x(target, 0);
    let target_right = coordenada_x(target, u16::MAX);
    let bottom_left = interpolar(source_left, target_left, deslizamiento);
    let bottom_right = interpolar(source_right, target_right, deslizamiento);
    let x_left = lateral_embudo(y, source_top, target_top, source_left, bottom_left);
    let x_right = lateral_embudo(y, source_top, target_top, source_right, bottom_right);
    let x = interpolar(x_left, x_right, u as i64);

    Point { x, y }
}

fn lateral_embudo(y: i64, arriba_y: i64, abajo_y: i64, arriba_x: i64, abajo_x: i64) -> i64 {
    if y <= arriba_y {
        arriba_x
    } else if y >= abajo_y || abajo_y <= arriba_y {
        abajo_x
    } else {
        let progreso = ((y - arriba_y) * UNIDAD) / (abajo_y - arriba_y);
        interpolar(arriba_x, abajo_x, ease_in_out_cuadratico(progreso))
    }
}

fn ease_in_out_cuadratico(progreso: i64) -> i64 {
    let p = progreso.clamp(0, UNIDAD);
    if p < UNIDAD / 2 {
        (2 * p * p) / UNIDAD
    } else {
        let restante = UNIDAD - p;
        UNIDAD - ((2 * restante * restante) / UNIDAD)
    }
}

fn interpolar(inicio: i64, fin: i64, progreso: i64) -> i64 {
    if progreso == 0 {
        inicio
    } else if progreso >= UNIDAD {
        fin
    } else {
        inicio + ((fin - inicio) * progreso) / UNIDAD
    }
}

fn coordenada_x(rect: Rect, u: u16) -> i64 {
    ((rect.x as i64) << 16) + ((rect.w as i64 * u as i64 * ESCALA_Q16) / UNIDAD)
}

fn coordenada_y(rect: Rect, v: u16) -> i64 {
    ((rect.y as i64) << 16) + ((rect.h as i64 * v as i64 * ESCALA_Q16) / UNIDAD)
}

/// Rasteriza una superficie de píxeles con la malla Genie.
///
/// `pintar` recibe posición de destino, coordenadas de textura y color. El
/// callback es responsable del clip/blend propio del anfitrión. No se reserva
/// memoria: sólo se usan dos filas de dos vértices en la pila.
#[allow(clippy::too_many_arguments)]
pub fn rasterizar<F>(
    pixeles: &[u32],
    ancho: u32,
    alto: u32,
    lienzo_ancho: i32,
    lienzo_alto: i32,
    origen: Rect,
    destino: Rect,
    progreso: u16,
    mut pintar: F,
) where
    F: FnMut(i32, i32, u32, u32, u32),
{
    const FILAS: usize = 50;
    if ancho == 0 || alto == 0 || lienzo_ancho <= 0 || lienzo_alto <= 0 {
        return;
    }
    let Some(total) = (ancho as usize).checked_mul(alto as usize) else {
        return;
    };
    if pixeles.len() < total {
        return;
    }

    let mut fila_arriba = [RasterVertice::default(); 2];
    let mut fila_abajo = [RasterVertice::default(); 2];
    for fila in 0..FILAS {
        crear_fila_malla(
            &mut fila_arriba,
            fila,
            FILAS,
            ancho,
            alto,
            origen,
            destino,
            progreso,
        );
        crear_fila_malla(
            &mut fila_abajo,
            fila + 1,
            FILAS,
            ancho,
            alto,
            origen,
            destino,
            progreso,
        );
        rasterizar_triangulo(
            pixeles,
            ancho,
            alto,
            lienzo_ancho,
            lienzo_alto,
            fila_arriba[0],
            fila_arriba[1],
            fila_abajo[1],
            &mut pintar,
        );
        rasterizar_triangulo(
            pixeles,
            ancho,
            alto,
            lienzo_ancho,
            lienzo_alto,
            fila_arriba[0],
            fila_abajo[1],
            fila_abajo[0],
            &mut pintar,
        );
        core::mem::swap(&mut fila_arriba, &mut fila_abajo);
    }
}

#[derive(Clone, Copy, Default)]
struct RasterVertice {
    x: i32,
    y: i32,
    sx: i64,
    sy: i64,
}

fn crear_fila_malla(
    fila: &mut [RasterVertice; 2],
    indice_y: usize,
    total_filas: usize,
    ancho: u32,
    alto: u32,
    origen: Rect,
    destino: Rect,
    progreso: u16,
) {
    let v = (indice_y * u16::MAX as usize / total_filas) as u16;
    for (indice_x, vertice) in fila.iter_mut().enumerate() {
        let u = (indice_x * u16::MAX as usize) as u16;
        let Point { x, y } = map_point(origen, destino, u, v, progreso);
        *vertice = RasterVertice {
            x: redondear_q16(x),
            y: redondear_q16(y),
            sx: coordenada_textura(ancho, u),
            sy: coordenada_textura(alto, v),
        };
    }
}

fn coordenada_textura(tamano: u32, normalizada: u16) -> i64 {
    // Rect define bordes (`x..x+width`), no centros de píxel. Usar el mismo
    // intervalo para la textura hace que el fotograma inicial sea una copia
    // exacta y el muestreo final se acota al último texel.
    ((tamano as i64 * normalizada as i64) << 16) / u16::MAX as i64
}

fn redondear_q16(valor: i64) -> i32 {
    let redondeado = if valor >= 0 {
        (valor + (1 << 15)) >> 16
    } else {
        -(((-valor) + (1 << 15)) >> 16)
    };
    redondeado.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn arista(a: RasterVertice, b: RasterVertice, x: i32, y: i32) -> i64 {
    (b.x as i64 - a.x as i64) * (y as i64 - a.y as i64)
        - (b.y as i64 - a.y as i64) * (x as i64 - a.x as i64)
}

#[allow(clippy::too_many_arguments)]
fn rasterizar_triangulo<F>(
    pixeles: &[u32],
    ancho: u32,
    alto: u32,
    lienzo_ancho: i32,
    lienzo_alto: i32,
    a: RasterVertice,
    b: RasterVertice,
    c: RasterVertice,
    pintar: &mut F,
) where
    F: FnMut(i32, i32, u32, u32, u32),
{
    let area = arista(a, b, c.x, c.y);
    if area == 0 {
        return;
    }
    let sentido = area.signum();
    let min_x = a.x.min(b.x).min(c.x).max(0);
    let min_y = a.y.min(b.y).min(c.y).max(0);
    let max_x = a.x.max(b.x).max(c.x).min(lienzo_ancho - 1);
    let max_y = a.y.max(b.y).max(c.y).min(lienzo_alto - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let wa = arista(b, c, x, y);
            let wb = arista(c, a, x, y);
            let wc = arista(a, b, x, y);
            if wa * sentido < 0 || wb * sentido < 0 || wc * sentido < 0 {
                continue;
            }
            let sx = (wa * a.sx + wb * b.sx + wc * c.sx) / area;
            let sy = (wa * a.sy + wb * b.sy + wc * c.sy) / area;
            let fuente_x = ((sx >> 16).clamp(0, ancho as i64 - 1)) as u32;
            let fuente_y = ((sy >> 16).clamp(0, alto as i64 - 1)) as u32;
            let color = pixeles[fuente_y as usize * ancho as usize + fuente_x as usize];
            pintar(x, y, fuente_x, fuente_y, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: u16 = u16::MAX;

    #[test]
    fn endpoints_are_exact() {
        let window = Rect {
            x: 41,
            y: 27,
            w: 640,
            h: 420,
        };
        let dock = Rect {
            x: 380,
            y: 550,
            w: 48,
            h: 48,
        };
        for (u, v) in [(0, 0), (FULL, 0), (0, FULL), (FULL, FULL), (32768, 32768)] {
            let start = map_point(window, dock, u, v, 0);
            let end = map_point(window, dock, u, v, FULL);
            assert_eq!(start.x, coordenada_x(window, u));
            assert_eq!(start.y, coordenada_y(window, v));
            assert_eq!(end.x, coordenada_x(dock, u));
            assert_eq!(end.y, coordenada_y(dock, v));
        }
    }

    #[test]
    fn progress_is_clamped_and_wrap_safe() {
        assert_eq!(progreso_ticks(100, 100, 40), 0);
        assert_eq!(progreso_ticks(120, 100, 40), 32_767);
        assert_eq!(progreso_ticks(140, 100, 40), FULL);
        assert_eq!(progreso_ticks(0, u64::MAX - 9, 20), 32_767);
        assert_eq!(progreso_ticks(10, 20, 0), FULL);
    }

    #[test]
    fn lower_edge_shears_before_full_translation() {
        let window = Rect {
            x: 30,
            y: 50,
            w: 400,
            h: 300,
        };
        let dock = Rect {
            x: 300,
            y: 500,
            w: 48,
            h: 48,
        };
        let start = map_point(window, dock, 0, FULL, 0);
        let half = map_point(window, dock, 0, FULL, 32_768);
        assert!(half.x - start.x > (200 << 16));
        assert!(half.y < (dock.y as i64) << 16);
    }

    #[test]
    fn transform_stays_finite_and_near_the_travel_corridor() {
        let window = Rect {
            x: 30,
            y: 50,
            w: 800,
            h: 520,
        };
        let dock = Rect {
            x: 470,
            y: 700,
            w: 48,
            h: 48,
        };
        for step in 0..=32 {
            let p = (step * FULL as usize / 32) as u16;
            for u in (0..=FULL).step_by(2048) {
                for v in (0..=FULL).step_by(2048) {
                    let point = map_point(window, dock, u, v, p);
                    assert!(point.x > -1_000_000 && point.x < 100_000_000);
                    assert!(point.y > -1_000_000 && point.y < 100_000_000);
                }
            }
        }
    }

    #[test]
    fn rasterizer_uses_caller_surface_without_allocating_or_knowing_canvas() {
        let source_pixels = [0x11, 0x22, 0x33, 0x44];
        let mut output = [0; 4];
        rasterizar(
            &source_pixels,
            2,
            2,
            2,
            2,
            Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            0,
            |x, y, _sx, _sy, color| output[y as usize * 2 + x as usize] = color,
        );
        assert_eq!(output, source_pixels);
    }
}
