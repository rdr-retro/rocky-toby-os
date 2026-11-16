//! Lector de PNG (RFC 2083).
//!
//! Decodifica un PNG a un mapa de pixeles `0xAARRGGBB` (alfa en 31..24) listo
//! para pintar con transparencia. La
//! descompresion de los `IDAT` la hace la libreria `inflate` (zlib/DEFLATE). Se
//! admiten los tipos de color habituales (gris, RGB, paleta, gris+alfa y RGBA) a
//! 8 y 16 bits por canal y sin entrelazar; el resto da error sin tumbar el
//! sistema.
//!
//! Es una libreria `no_std`: solo necesita `core` y el asignador de `alloc`.
//! Codigo propio de rocky.
#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// Errores al leer un PNG.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// No es un PNG o esta mal formado.
    Formato,
    /// Es un PNG valido pero de un tipo que no soportamos.
    Soporte,
    /// Los datos comprimidos estan corruptos.
    Datos,
}

/// Una imagen decodificada.
pub struct Imagen {
    pub ancho: usize,
    pub alto: usize,
    /// Pixeles en orden de filas, formato `0xAARRGGBB`.
    ///
    /// El alfa va en los bits 31..24 (255 = opaco). El RGB es directo, sin
    /// pre-multiplicar: quien pinta mezcla con el fondo segun el alfa.
    pub pixeles: Vec<u32>,
}

/// Cadena de 8 bytes con la que empieza todo PNG.
const FIRMA: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn be32(b: &[u8]) -> u32 {
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32
}

/// Bytes que ocupa un pixel segun el tipo de color y la profundidad.
fn bytes_por_pixel(color: u8, depth: u8) -> Option<usize> {
    Some(match (color, depth) {
        (0, 8) => 1,  // gris
        (0, 16) => 2, // gris 16 bits
        (2, 8) => 3,  // RGB
        (2, 16) => 6, // RGB 16 bits
        (3, 8) => 1,  // paleta
        (4, 8) => 2,  // gris + alfa
        (4, 16) => 4,
        (6, 8) => 4, // RGBA
        (6, 16) => 8,
        _ => return None,
    })
}

/// Decodifica un PNG completo.
pub fn decodificar(d: &[u8]) -> Result<Imagen, Error> {
    if d.len() < 8 || &d[0..8] != FIRMA {
        return Err(Error::Formato);
    }

    let mut pos = 8usize;
    let mut ancho = 0usize;
    let mut alto = 0usize;
    let mut profundidad = 0u8;
    let mut color = 0u8;
    let mut entrelazado = 0u8;
    let mut idat: Vec<u8> = Vec::new();
    let mut plte: Vec<u8> = Vec::new();
    let mut visto_ihdr = false;

    while pos + 8 <= d.len() {
        let len = be32(&d[pos..pos + 4]) as usize;
        let tipo = &d[pos + 4..pos + 8];
        let ini = pos + 8;
        let fin = ini.checked_add(len).ok_or(Error::Formato)?;
        if fin + 4 > d.len() {
            return Err(Error::Formato);
        }
        let cuerpo = &d[ini..fin];
        match tipo {
            b"IHDR" => {
                if len < 13 {
                    return Err(Error::Formato);
                }
                ancho = be32(&cuerpo[0..4]) as usize;
                alto = be32(&cuerpo[4..8]) as usize;
                profundidad = cuerpo[8];
                color = cuerpo[9];
                if cuerpo[10] != 0 || cuerpo[11] != 0 {
                    return Err(Error::Soporte); // metodo de compresion/filtro raros
                }
                entrelazado = cuerpo[12];
                visto_ihdr = true;
            }
            b"PLTE" => plte.extend_from_slice(cuerpo),
            b"IDAT" => idat.extend_from_slice(cuerpo),
            b"IEND" => break,
            _ => {}
        }
        pos = fin + 4; // tras el CRC
    }

    if !visto_ihdr || ancho == 0 || alto == 0 {
        return Err(Error::Formato);
    }
    if entrelazado != 0 {
        return Err(Error::Soporte); // entrelazado Adam7: no soportado
    }
    let bpp = bytes_por_pixel(color, profundidad).ok_or(Error::Soporte)?;
    if color == 3 && plte.len() < 3 {
        return Err(Error::Formato);
    }

    let crudo = inflate::zlib(&idat).map_err(|_| Error::Datos)?;
    let fila = ancho * bpp;
    if crudo.len() < (fila + 1) * alto {
        return Err(Error::Datos);
    }
    let filtrado = desfiltrar(&crudo, ancho, alto, bpp).ok_or(Error::Datos)?;

    let mut pixeles = vec![0u32; ancho * alto];
    a_rgb(
        &filtrado,
        ancho,
        alto,
        color,
        profundidad,
        bpp,
        &plte,
        &mut pixeles,
    )?;
    Ok(Imagen {
        ancho,
        alto,
        pixeles,
    })
}

/// Deshace el filtrado por filas de PNG (tipos 0 a 4).
fn desfiltrar(datos: &[u8], ancho: usize, alto: usize, bpp: usize) -> Option<Vec<u8>> {
    let fila = ancho * bpp;
    let mut out = vec![0u8; alto * fila];
    let mut pos = 0usize;
    for y in 0..alto {
        if pos >= datos.len() {
            return None;
        }
        let f = datos[pos];
        pos += 1;
        if pos + fila > datos.len() {
            return None;
        }
        let src = &datos[pos..pos + fila];
        pos += fila;
        let base = y * fila;
        for x in 0..fila {
            let raw = src[x];
            let a = if x >= bpp { out[base + x - bpp] } else { 0 };
            let b = if y > 0 { out[base - fila + x] } else { 0 };
            let c = if x >= bpp && y > 0 {
                out[base - fila + x - bpp]
            } else {
                0
            };
            let v = match f {
                0 => raw,
                1 => raw.wrapping_add(a),
                2 => raw.wrapping_add(b),
                3 => raw.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => raw.wrapping_add(paeth(a, b, c)),
                _ => return None,
            };
            out[base + x] = v;
        }
    }
    Some(out)
}

/// Predictor de Paeth (el filtro 4 de PNG).
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Convierte los pixeles ya desfiltrados a `0xAARRGGBB`, conservando el alfa.
///
/// El RGB se deja directo (sin componer sobre negro) para que quien pinte
/// pueda mezclarlo con el fondo: asi las esquinas redondeadas y las sombras
/// suaves de las piezas AQUA quedan transparentes de verdad.
fn a_rgb(
    f: &[u8],
    ancho: usize,
    alto: usize,
    color: u8,
    depth: u8,
    bpp: usize,
    plte: &[u8],
    out: &mut [u32],
) -> Result<(), Error> {
    for y in 0..alto {
        for x in 0..ancho {
            let p = &f[(y * ancho + x) * bpp..];
            let (r, g, b, a): (u8, u8, u8, u8) = match (color, depth) {
                (0, 8) => (p[0], p[0], p[0], 255),
                (0, 16) => (p[0], p[0], p[0], 255),
                (2, 8) => (p[0], p[1], p[2], 255),
                (2, 16) => (p[0], p[2], p[4], 255),
                (3, 8) => {
                    let i = p[0] as usize * 3;
                    if i + 2 >= plte.len() {
                        return Err(Error::Datos);
                    }
                    (plte[i], plte[i + 1], plte[i + 2], 255)
                }
                (4, 8) => (p[0], p[0], p[0], p[1]),
                (4, 16) => (p[0], p[0], p[0], p[2]),
                (6, 8) => (p[0], p[1], p[2], p[3]),
                (6, 16) => (p[0], p[2], p[4], p[6]),
                _ => return Err(Error::Soporte),
            };
            out[y * ancho + x] =
                ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        }
    }
    Ok(())
}
