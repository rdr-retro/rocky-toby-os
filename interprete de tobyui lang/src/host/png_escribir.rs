//! Escritor de PNG minimo, sin dependencias.
//!
//! Lo usa el modo `--png` del anfitrion para volcar la composicion a un fichero
//! y poder verla/verificarla sin abrir ventana. El motor del lenguaje no
//! necesita esto: es solo una utilidad del anfitrion.
//!
//! El flujo zlib se emite con bloques DEFLATE *almacenados* (sin comprimir):
//! valido y suficiente para una captura.

use alloc::vec::Vec;

/// CRC-32 (polinomio de PNG), sin tabla.
fn crc32(datos: &[u8]) -> u32 {
    let mut c: u32 = 0xFFFF_FFFF;
    for &b in datos {
        c ^= b as u32;
        for _ in 0..8 {
            let m = (c & 1).wrapping_neg();
            c = (c >> 1) ^ (0xEDB8_8320 & m);
        }
    }
    !c
}

/// Adler-32, el checksum que zlib pone al final.
fn adler32(datos: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &x in datos {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Flujo zlib con bloques DEFLATE almacenados.
fn zlib_almacenado(datos: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(datos.len() + datos.len() / 65535 * 5 + 16);
    // Cabecera zlib: CMF=0x78, FLG=0x01 (sin diccionario, nivel mas rapido).
    out.push(0x78);
    out.push(0x01);
    let mut i = 0usize;
    loop {
        let n = core::cmp::min(65535, datos.len() - i);
        let ultimo = i + n == datos.len();
        out.push(if ultimo { 1 } else { 0 }); // BFINAL + BTYPE=00 (almacenado)
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(!(n as u16)).to_le_bytes());
        out.extend_from_slice(&datos[i..i + n]);
        i += n;
        if ultimo {
            break;
        }
    }
    out.extend_from_slice(&adler32(datos).to_be_bytes());
    out
}

/// Anexa un chunk PNG (longitud, tipo, datos, CRC).
fn chunk(out: &mut Vec<u8>, tipo: &[u8; 4], datos: &[u8]) {
    out.extend_from_slice(&(datos.len() as u32).to_be_bytes());
    out.extend_from_slice(tipo);
    out.extend_from_slice(datos);
    let mut crc_in = Vec::with_capacity(4 + datos.len());
    crc_in.extend_from_slice(tipo);
    crc_in.extend_from_slice(datos);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

/// Escribe `pixeles` (`0x00RRGGBB`, opacos) como un PNG RGBA de `ancho`x`alto`.
pub fn escribir(ruta: &str, ancho: u32, alto: u32, pixeles: &[u32]) -> std::io::Result<()> {
    // Datos crudos: por fila, un byte de filtro (0 = ninguno) y RGBA.
    let mut crudo: Vec<u8> = Vec::with_capacity((alto * (1 + ancho * 4)) as usize);
    for y in 0..alto {
        crudo.push(0);
        for x in 0..ancho {
            let c = pixeles[(y * ancho + x) as usize];
            crudo.push((c >> 16) as u8);
            crudo.push((c >> 8) as u8);
            crudo.push(c as u8);
            crudo.push(255);
        }
    }

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    let mut ihdr: Vec<u8> = Vec::with_capacity(13);
    ihdr.extend_from_slice(&ancho.to_be_bytes());
    ihdr.extend_from_slice(&alto.to_be_bytes());
    ihdr.push(8); // profundidad de bit
    ihdr.push(6); // tipo de color: RGBA
    ihdr.push(0); // compresion
    ihdr.push(0); // filtro
    ihdr.push(0); // entrelazado
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_almacenado(&crudo));
    chunk(&mut out, b"IEND", &[]);

    std::fs::write(ruta, out)
}
