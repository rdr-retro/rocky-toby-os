//! Convierte la colección BMP/PNG de `arrow/` al paquete binario KCUR v2 que
//! incrusta Kira. La decodificación de formatos de imagen ocurre solo aquí,
//! durante la construcción; el kernel consume cursores indexados y paletizados.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const FOTOGRAMAS_PENSANDO: usize = 12;
const TICKS_POR_FOTOGRAMA: u8 = 3;

struct Sprite {
    id: u8,
    ancho: u8,
    alto: u8,
    hotspot_x: u8,
    hotspot_y: u8,
    ticks_por_fotograma: u8,
    cuadros: Vec<Vec<u32>>,
}

struct Imagen {
    ancho: usize,
    alto: usize,
    pixeles: Vec<u32>, // AARRGGBB
}

/// Genera `kernel/assets/cursor_theme.bin` desde los originales de `arrow/`.
pub fn generar(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("arrow");
    let mut sprites = Vec::new();

    for (id, nombre, hx, hy) in [
        (0, "arrow.bmp", 0, 0),
        (1, "hand.bmp", 5, 0),
        (2, "ibeam.bmp", 5, 9),
        (3, "move.bmp", 10, 10),
        (4, "resize-ew.bmp", 10, 10),
        (5, "resize-ns.bmp", 10, 10),
        (6, "resize-nwse.bmp", 10, 10),
    ] {
        let path = dir.join(nombre);
        let bytes =
            fs::read(&path).map_err(|e| format!("no se pudo leer {}: {e}", path.display()))?;
        let image = bmp_a_argb(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        sprites.push(sprite_estatico(id, image, hx, hy)?);
    }

    let path = dir.join("spinning.png");
    let bytes = fs::read(&path).map_err(|e| format!("no se pudo leer {}: {e}", path.display()))?;
    let imagen_png = png::decodificar(&bytes)
        .map_err(|e| format!("{}: PNG invalido ({e:?})", path.display()))?;
    let imagen = Imagen {
        ancho: imagen_png.ancho,
        alto: imagen_png.alto,
        pixeles: imagen_png.pixeles,
    };
    validar_dimensiones(&imagen)?;
    let mut cuadros = Vec::with_capacity(FOTOGRAMAS_PENSANDO);
    for i in 0..FOTOGRAMAS_PENSANDO {
        cuadros.push(rotar(
            &imagen,
            i as f64 * core::f64::consts::TAU / FOTOGRAMAS_PENSANDO as f64,
        ));
    }
    sprites.push(Sprite {
        id: 7,
        ancho: imagen.ancho as u8,
        alto: imagen.alto as u8,
        hotspot_x: (imagen.ancho / 2) as u8,
        hotspot_y: (imagen.alto / 2) as u8,
        ticks_por_fotograma: TICKS_POR_FOTOGRAMA,
        cuadros,
    });

    let out = root.join("kernel/assets/cursor_theme.bin");
    let parent = out
        .parent()
        .ok_or_else(|| "la ruta del paquete no tiene carpeta padre".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("no se pudo crear {}: {e}", parent.display()))?;
    let paquete = empaquetar(&sprites)?;
    fs::write(&out, paquete).map_err(|e| format!("no se pudo escribir {}: {e}", out.display()))?;
    Ok(out)
}

fn sprite_estatico(id: u8, image: Imagen, hx: u8, hy: u8) -> Result<Sprite, String> {
    validar_dimensiones(&image)?;
    if hx as usize >= image.ancho || hy as usize >= image.alto {
        return Err(format!("hotspot fuera del sprite {id}"));
    }
    Ok(Sprite {
        id,
        ancho: image.ancho as u8,
        alto: image.alto as u8,
        hotspot_x: hx,
        hotspot_y: hy,
        ticks_por_fotograma: 0,
        cuadros: vec![image.pixeles],
    })
}

fn validar_dimensiones(image: &Imagen) -> Result<(), String> {
    if image.ancho == 0 || image.alto == 0 || image.ancho > 64 || image.alto > 64 {
        return Err(format!(
            "tamaño de cursor no admitido: {}x{}",
            image.ancho, image.alto
        ));
    }
    if image.pixeles.len() != image.ancho * image.alto {
        return Err("cantidad de píxeles incorrecta".to_string());
    }
    Ok(())
}

/// Lee BMP 24-bit y vuelve transparente el color de fondo conectado al borde.
/// El flood-fill conserva blancos encerrados, como el interior de la mano.
fn bmp_a_argb(bytes: &[u8]) -> Result<Imagen, String> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return Err("se esperaba BMP sin comprimir".to_string());
    }
    let offset = le_u32(bytes, 10)? as usize;
    let ancho = le_i32(bytes, 18)?;
    let alto_signed = le_i32(bytes, 22)?;
    if ancho <= 0 || alto_signed == 0 || le_u16(bytes, 28)? != 24 || le_u32(bytes, 30)? != 0 {
        return Err("solo se admite BMP 24-bit sin comprimir".to_string());
    }
    let ancho = ancho as usize;
    let alto = alto_signed.unsigned_abs() as usize;
    if ancho > 64 || alto > 64 {
        return Err(format!("tamaño BMP no admitido: {ancho}x{alto}"));
    }
    let stride = (ancho * 3 + 3) & !3;
    let bytes_p = stride
        .checked_mul(alto)
        .and_then(|n| offset.checked_add(n))
        .ok_or_else(|| "tamaño BMP desbordado".to_string())?;
    if bytes_p > bytes.len() {
        return Err("BMP truncado".to_string());
    }

    let mut rgb = vec![0u32; ancho * alto];
    for y in 0..alto {
        let fila = if alto_signed > 0 { alto - 1 - y } else { y };
        for x in 0..ancho {
            let p = offset + fila * stride + x * 3;
            rgb[y * ancho + x] =
                ((bytes[p + 2] as u32) << 16) | ((bytes[p + 1] as u32) << 8) | bytes[p] as u32;
        }
    }

    let fondo = rgb[ancho - 1];
    let mut fondo_exterior = vec![false; rgb.len()];
    let mut cola = Vec::new();
    for x in 0..ancho {
        agregar_fondo(x, fondo, &rgb, &mut fondo_exterior, &mut cola);
        agregar_fondo(
            (alto - 1) * ancho + x,
            fondo,
            &rgb,
            &mut fondo_exterior,
            &mut cola,
        );
    }
    for y in 0..alto {
        agregar_fondo(y * ancho, fondo, &rgb, &mut fondo_exterior, &mut cola);
        agregar_fondo(
            y * ancho + ancho - 1,
            fondo,
            &rgb,
            &mut fondo_exterior,
            &mut cola,
        );
    }
    while let Some(i) = cola.pop() {
        let x = i % ancho;
        let y = i / ancho;
        if x > 0 {
            agregar_fondo(i - 1, fondo, &rgb, &mut fondo_exterior, &mut cola);
        }
        if x + 1 < ancho {
            agregar_fondo(i + 1, fondo, &rgb, &mut fondo_exterior, &mut cola);
        }
        if y > 0 {
            agregar_fondo(i - ancho, fondo, &rgb, &mut fondo_exterior, &mut cola);
        }
        if y + 1 < alto {
            agregar_fondo(i + ancho, fondo, &rgb, &mut fondo_exterior, &mut cola);
        }
    }

    let pixeles = rgb
        .into_iter()
        .zip(fondo_exterior)
        .map(|(p, es_fondo)| if es_fondo { 0 } else { 0xFF00_0000 | p })
        .collect();
    Ok(Imagen {
        ancho,
        alto,
        pixeles,
    })
}

fn agregar_fondo(i: usize, color: u32, pixeles: &[u32], fondo: &mut [bool], cola: &mut Vec<usize>) {
    if !fondo[i] && pixeles[i] == color {
        fondo[i] = true;
        cola.push(i);
    }
}

fn rotar(src: &Imagen, angulo: f64) -> Vec<u32> {
    let (w, h) = (src.ancho as i32, src.alto as i32);
    let (cx, cy) = ((w - 1) as f64 / 2.0, (h - 1) as f64 / 2.0);
    let (sin, cos) = angulo.sin_cos();
    let mut out = vec![0; src.pixeles.len()];
    for y in 0..h {
        for x in 0..w {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            out[y as usize * w as usize + x as usize] = muestrear_bilineal(src, sx, sy);
        }
    }
    out
}

fn muestrear_bilineal(src: &Imagen, x: f64, y: f64) -> u32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let mut suma_a = 0.0;
    let mut suma_r = 0.0;
    let mut suma_g = 0.0;
    let mut suma_b = 0.0;
    for (ox, oy, peso) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        let (px, py) = (x0 + ox, y0 + oy);
        if px < 0 || py < 0 || px >= src.ancho as i32 || py >= src.alto as i32 {
            continue;
        }
        let c = src.pixeles[py as usize * src.ancho + px as usize];
        let a = (c >> 24) as f64;
        suma_a += a * peso;
        suma_r += ((c >> 16) & 0xFF) as f64 * a * peso;
        suma_g += ((c >> 8) & 0xFF) as f64 * a * peso;
        suma_b += (c & 0xFF) as f64 * a * peso;
    }
    if suma_a <= 0.0 {
        return 0;
    }
    let a = suma_a.round().clamp(0.0, 255.0) as u32;
    let r = (suma_r / suma_a).round().clamp(0.0, 255.0) as u32;
    let g = (suma_g / suma_a).round().clamp(0.0, 255.0) as u32;
    let b = (suma_b / suma_a).round().clamp(0.0, 255.0) as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

fn empaquetar(sprites: &[Sprite]) -> Result<Vec<u8>, String> {
    if sprites.len() > u8::MAX as usize {
        return Err("demasiados cursores".to_string());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"KCUR");
    out.push(2); // índices de 2 bpp / 8 bpp más paleta ARGB
    out.push(sprites.len() as u8);
    for s in sprites {
        let pixeles_totales = (s.ancho as usize)
            .checked_mul(s.alto as usize)
            .and_then(|n| n.checked_mul(s.cuadros.len()))
            .ok_or_else(|| format!("tamaño desbordado para cursor {}", s.id))?;
        if s.cuadros
            .iter()
            .any(|c| c.len() != s.ancho as usize * s.alto as usize)
        {
            return Err(format!(
                "fotograma de tamaño incorrecto para cursor {}",
                s.id
            ));
        }
        let paleta = crear_paleta(s);
        if paleta.is_empty() || paleta.len() > 256 {
            return Err(format!("paleta no válida para cursor {}", s.id));
        }
        let bits = if paleta.len() <= 4 { 2 } else { 8 };
        out.extend_from_slice(&[
            s.id,
            s.ancho,
            s.alto,
            s.hotspot_x,
            s.hotspot_y,
            s.cuadros.len() as u8,
            s.ticks_por_fotograma,
            bits,
        ]);
        out.extend_from_slice(&(paleta.len() as u16).to_le_bytes());
        for color in &paleta {
            out.extend_from_slice(&color.to_le_bytes());
        }

        let payload_inicio = out.len();
        if bits == 2 {
            out.resize(payload_inicio + pixeles_totales.div_ceil(4), 0);
        } else {
            out.reserve(pixeles_totales);
        }
        let mut indice_global = 0usize;
        for cuadro in &s.cuadros {
            for px in cuadro {
                let indice = indice_color(*px, &paleta) as u8;
                if bits == 2 {
                    let byte = payload_inicio + indice_global / 4;
                    out[byte] |= indice << ((indice_global % 4) * 2);
                } else {
                    out.push(indice);
                }
                indice_global += 1;
            }
        }
    }
    Ok(out)
}

/// Conserva exactamente los colores de cursores pequeños y reduce la rueda
/// animada a una paleta determinista de hasta 256 colores.
fn crear_paleta(sprite: &Sprite) -> Vec<u32> {
    let mut frecuencias = BTreeMap::<u32, u32>::new();
    for color in sprite.cuadros.iter().flatten() {
        let canonico = if color >> 24 == 0 { 0 } else { *color };
        *frecuencias.entry(canonico).or_default() += 1;
    }
    if frecuencias.len() <= 4 {
        return frecuencias.into_keys().collect();
    }

    let tiene_transparencia = frecuencias.remove(&0).is_some();
    let max_opacos = 256 - usize::from(tiene_transparencia);
    let puntos = frecuencias
        .into_iter()
        .map(|(argb, peso)| PuntoColor {
            canales: [
                ((argb >> 16) & 0xff) as u8,
                ((argb >> 8) & 0xff) as u8,
                (argb & 0xff) as u8,
                (argb >> 24) as u8,
            ],
            peso,
        })
        .collect::<Vec<_>>();

    let mut paleta = Vec::with_capacity(256);
    if tiene_transparencia {
        paleta.push(0);
    }
    if puntos.len() <= max_opacos {
        paleta.extend(puntos.into_iter().map(PuntoColor::argb));
        return paleta;
    }
    paleta.extend(cuantizar_mediana(puntos, max_opacos));
    paleta
}

#[derive(Clone)]
struct PuntoColor {
    canales: [u8; 4], // RGBA
    peso: u32,
}

impl PuntoColor {
    fn argb(self) -> u32 {
        ((self.canales[3] as u32) << 24)
            | ((self.canales[0] as u32) << 16)
            | ((self.canales[1] as u32) << 8)
            | self.canales[2] as u32
    }
}

fn cuantizar_mediana(puntos: Vec<PuntoColor>, max_colores: usize) -> Vec<u32> {
    let mut cajas = vec![puntos];
    while cajas.len() < max_colores {
        let mut mejor: Option<(usize, usize, u32)> = None;
        for (i, caja) in cajas.iter().enumerate() {
            if caja.len() < 2 {
                continue;
            }
            let mut mejor_canal = 0;
            let mut mejor_rango = 0u32;
            for canal in 0..4 {
                let (mut min, mut max) = (u8::MAX, u8::MIN);
                for punto in caja {
                    min = min.min(punto.canales[canal]);
                    max = max.max(punto.canales[canal]);
                }
                let rango = (max - min) as u32;
                if rango > mejor_rango {
                    mejor_rango = rango;
                    mejor_canal = canal;
                }
            }
            let peso = caja.iter().map(|p| p.peso).sum::<u32>();
            let prioridad = mejor_rango * peso;
            if prioridad > 0 && mejor.is_none_or(|(_, _, actual)| prioridad > actual) {
                mejor = Some((i, mejor_canal, prioridad));
            }
        }
        let Some((indice, canal, _)) = mejor else {
            break;
        };
        let mut caja = cajas.swap_remove(indice);
        caja.sort_by_key(|p| p.canales[canal]);
        let peso_total = caja.iter().map(|p| p.peso).sum::<u32>();
        let mut acumulado = 0;
        let mut corte = 1;
        for (i, punto) in caja.iter().enumerate().take(caja.len() - 1) {
            acumulado += punto.peso;
            corte = i + 1;
            if acumulado * 2 >= peso_total {
                break;
            }
        }
        let derecha = caja.split_off(corte);
        cajas.push(caja);
        cajas.push(derecha);
    }

    cajas
        .into_iter()
        .map(|caja| {
            let total = caja.iter().map(|p| p.peso).sum::<u32>();
            let mut suma = [0u64; 4];
            for punto in &caja {
                for (canal, valor) in punto.canales.iter().enumerate() {
                    suma[canal] += *valor as u64 * punto.peso as u64;
                }
            }
            PuntoColor {
                canales: std::array::from_fn(|i| {
                    ((suma[i] + total as u64 / 2) / total as u64) as u8
                }),
                peso: 1,
            }
            .argb()
        })
        .collect()
}

fn indice_color(color: u32, paleta: &[u32]) -> usize {
    if color >> 24 == 0 {
        if let Some(i) = paleta.iter().position(|p| *p == 0) {
            return i;
        }
    }
    let (r, g, b, a) = (
        ((color >> 16) & 0xff) as i32,
        ((color >> 8) & 0xff) as i32,
        (color & 0xff) as i32,
        (color >> 24) as i32,
    );
    paleta
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (pr, pg, pb, pa) = (
                ((p >> 16) & 0xff) as i32,
                ((p >> 8) & 0xff) as i32,
                (p & 0xff) as i32,
                (p >> 24) as i32,
            );
            let (dr, dg, db, da) = (r - pr, g - pg, b - pb, a - pa);
            (i, dr * dr + dg * dg + db * db + 2 * da * da)
        })
        .min_by_key(|(_, distancia)| *distancia)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn le_u16(bytes: &[u8], off: usize) -> Result<u16, String> {
    let b = bytes
        .get(off..off + 2)
        .ok_or_else(|| "cabecera truncada".to_string())?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn le_u32(bytes: &[u8], off: usize) -> Result<u32, String> {
    let b = bytes
        .get(off..off + 4)
        .ok_or_else(|| "cabecera truncada".to_string())?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn le_i32(bytes: &[u8], off: usize) -> Result<i32, String> {
    le_u32(bytes, off).map(|v| v as i32)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn genera_los_siete_estados_y_los_doce_fotogramas_de_espera() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let path = generar(root).unwrap();
        let bytes = fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"KCUR");
        assert_eq!(bytes[4], 2);
        assert_eq!(bytes[5], 8);
        assert!(
            bytes.len() < 8 * 1024,
            "el tema comprimido no es compacto: {} bytes",
            bytes.len()
        );
        let mut pos = 6;
        let mut pensando = None;
        for _ in 0..bytes[5] {
            let id = bytes[pos];
            let (w, h, frames, bits) = (
                bytes[pos + 1] as usize,
                bytes[pos + 2] as usize,
                bytes[pos + 5],
                bytes[pos + 7],
            );
            if id == 7 {
                pensando = Some(frames);
                assert_eq!((w, h), (18, 18));
                assert_eq!(bits, 8);
                let colores = u16::from_le_bytes([bytes[pos + 8], bytes[pos + 9]]) as usize;
                assert!(colores <= 256);
                let primero = pos + 10 + colores * 4;
                let tamano_cuadro = w * h;
                assert_ne!(
                    &bytes[primero..primero + tamano_cuadro],
                    &bytes[primero + tamano_cuadro..primero + 2 * tamano_cuadro],
                    "spinning.png debe generar cuadros realmente rotados"
                );
            }
            let colores = u16::from_le_bytes([bytes[pos + 8], bytes[pos + 9]]) as usize;
            let payload = if bits == 2 {
                (w * h * frames as usize).div_ceil(4)
            } else {
                w * h * frames as usize
            };
            pos += 10 + colores * 4 + payload;
            if id < 7 {
                assert_eq!(
                    bits, 2,
                    "el cursor estático {id} debería ser lossless a 2 bpp"
                );
            }
        }
        assert_eq!(pensando, Some(FOTOGRAMAS_PENSANDO as u8));
        assert_eq!(pos, bytes.len());
    }
}
