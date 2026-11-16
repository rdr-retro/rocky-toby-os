//! Correspondencia entre texto Unicode y los glifos de la fuente de la VGA.
//!
//! La fuente de la VGA (la ROM, y la copia RAM que prepara [`crate::bios`])
//! indexa sus 256 glifos por byte, en orden CP437. El texto, en cambio, vive
//! en Unicode: los `.toby` son UTF-8 y el teclado entrega bytes CP437. Aquí
//! se traduce de un lado a otro, y este es el único sitio donde se hace: lo
//! usan la consola (modo texto), el motor Toby UI (`parser`, `renderer`,
//! `app`) y los anfitriones Windows, que incluyen este mismo fichero con
//! `#[path]`.
//!
//! Hay dos fuentes y, por tanto, dos lecturas del byte:
//!
//!   * la ROM de modo texto sólo trae los acentos de CP437 (`é ñ á Ç É Ä Ö Ü
//!     Ñ...`): ese camino usa sólo la tabla [`CP437`];
//!   * la copia RAM de `bios::get_font` añade encima los 23 acentos que la
//!     ROM no trae (`bios::COMPONER`: `À Á Â Ã È É ÊË Ì Í Î Ï Ò Ó ÔÕ Ù Ú Û Ý
//!     ã õ ý`), cada uno en su sitio Latin-1: [`glifo_de`] los traduce
//!     también, y es la lectura que usa la GUI.
//!
//! El único choque entre ambos mundos es el byte `0xFD`: en la ROM es `²` y
//! en la fuente RAM se compuso `ý` encima. Aquí ganan los compuestos (la ida
//! y vuelta byte→char→byte es estable), que es lo que pinta la GUI. El único
//! doble que no choca es `É`: la ROM lo trae en 0x90 y `COMPONER` lo anade
//! también en 0xC9, y aquí gana el nativo (0x90).

/// Descodifica el caracter UTF-8 que empieza en `b[i]`.
///
/// Devuelve el punto de codigo y cuantos bytes ocupa, o `None` si la secuencia
/// esta cortada o mal formada.
pub fn utf8_en(b: &[u8], i: usize) -> Option<(u32, usize)> {
    let x = *b.get(i)?;
    let largo = match x {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return None,
    };
    if i + largo > b.len() {
        return None;
    }
    let mascara = if largo == 1 { 0x7F } else { 0x7F >> largo };
    let mut c = (x & mascara) as u32;
    for k in 1..largo {
        let y = b[i + k];
        if y & 0xC0 != 0x80 {
            return None;
        }
        c = (c << 6) | (y & 0x3F) as u32;
    }
    Some((c, largo))
}

/// Correspondencia entre los glifos de CP437 y su caracter Unicode.
///
/// Solo el rango Latin-1, que es todo lo que hace falta para el español: los
/// dibujos de caja y los simbolos griegos de CP437 no se escriben nunca desde
/// texto UTF-8 (la fuente los sigue teniendo, pero por otras vias).
pub const CP437: [(u8, u32); 53] = [
    (0x80, 0x00C7), // Ç
    (0x81, 0x00FC), // ü
    (0x82, 0x00E9), // é
    (0x83, 0x00E2), // â
    (0x84, 0x00E4), // ä
    (0x85, 0x00E0), // à
    (0x86, 0x00E5), // å
    (0x87, 0x00E7), // ç
    (0x88, 0x00EA), // ê
    (0x89, 0x00EB), // ë
    (0x8A, 0x00E8), // è
    (0x8B, 0x00EF), // ï
    (0x8C, 0x00EE), // î
    (0x8D, 0x00EC), // ì
    (0x8E, 0x00C4), // Ä
    (0x8F, 0x00C5), // Å
    (0x90, 0x00C9), // É
    (0x91, 0x00E6), // æ
    (0x92, 0x00C6), // Æ
    (0x93, 0x00F4), // ô
    (0x94, 0x00F6), // ö
    (0x95, 0x00F2), // ò
    (0x96, 0x00FB), // û
    (0x97, 0x00F9), // ù
    (0x98, 0x00FF), // ÿ
    (0x99, 0x00D6), // Ö
    (0x9A, 0x00DC), // Ü
    (0x9B, 0x00A2), // ¢
    (0x9C, 0x00A3), // £
    (0x9D, 0x00A5), // ¥
    (0xA0, 0x00E1), // á
    (0xA1, 0x00ED), // í
    (0xA2, 0x00F3), // ó
    (0xA3, 0x00FA), // ú
    (0xA4, 0x00F1), // ñ
    (0xA5, 0x00D1), // Ñ
    (0xA6, 0x00AA), // ª
    (0xA7, 0x00BA), // º
    (0xA8, 0x00BF), // ¿
    (0xAA, 0x00AC), // ¬
    (0xAB, 0x00BD), // ½
    (0xAC, 0x00BC), // ¼
    (0xAD, 0x00A1), // ¡
    (0xAE, 0x00AB), // «
    (0xAF, 0x00BB), // »
    (0xE1, 0x00DF), // ß
    (0xE6, 0x00B5), // µ
    (0xF1, 0x00B1), // ±
    (0xF6, 0x00F7), // ÷
    (0xF8, 0x00B0), // °
    (0xFA, 0x00B7), // ·
    (0xFD, 0x00B2), // ²
    (0xFF, 0x00A0), // espacio duro
];

/// Los 23 acentos que `bios::COMPONER` compone en la copia RAM de la fuente.
///
/// Son códigos Latin-1, y cada uno ocupa el slot que coincide con su código:
/// por eso la traducción es la identidad. **Si se cambia un destino en
/// `bios::COMPONER` hay que cambiarlo también aquí** (el de la ROM no los
/// trae, así que el orden de la tabla decide).
const COMPONER: [u32; 23] = [
    0x00C0, 0x00C1, 0x00C2, 0x00C3, // À Á Â Ã
    0x00C8, 0x00C9, 0x00CA, 0x00CB, // È É Ê Ë
    0x00CC, 0x00CD, 0x00CE, 0x00CF, // Ì Í Î Ï
    0x00D2, 0x00D3, 0x00D4, 0x00D5, // Ò Ó Ô Õ
    0x00D9, 0x00DA, 0x00DB, 0x00DD, // Ù Ú Û Ý
    0x00E3, 0x00F5, 0x00FD, // ã õ ý
];

/// Glifo (byte CP437) del caracter `c` en la fuente **preparada** de la GUI.
///
/// Es la lectura completa: ASCII, la tabla [`CP437`] y los acentos compuestos
/// de `bios::COMPONER`. Lo que no tiene glifo (`€`, `ñ` raro, ...) sale como
/// `?`, igual que en la consola.
pub fn glifo_de(c: char) -> u8 {
    let u = c as u32;
    if u < 0x80 {
        return u as u8;
    }
    // Primero la tabla: si la ROM ya traía el caracter (É está en 0x90), se
    // usa su glifo nativo, que es el de siempre.
    if let Some((g, _)) = CP437.iter().find(|(_, x)| *x == u) {
        // Salvo que la composición le haya pisado el slot: en la ROM 0xFD es
        // ², pero en la fuente de la GUI se compuso ý encima, así que ² ya no
        // tiene sitio.
        if !COMPONER.contains(&(*g as u32)) {
            return *g;
        }
    }
    // Lo que la ROM no trae, en su sitio Latin-1 (bios::COMPONER).
    if COMPONER.contains(&u) {
        return u as u8;
    }
    b'?'
}

/// Caracter Unicode del glifo `b`, camino de vuelta de [`glifo_de`].
///
/// Es lo que usa el campo de texto: el teclado entrega bytes CP437 y el texto
/// se guarda en Unicode. Los compuestos vuelven a su Latin-1 (0xC1 = `Á`), y
/// el resto sigue la tabla o, si no está, `b as char`.
pub fn char_de_glifo(b: u8) -> char {
    if COMPONER.contains(&(b as u32)) {
        return char::from_u32(b as u32).unwrap_or('?');
    }
    match CP437.iter().find(|(g, _)| *g == b) {
        Some((_, u)) => char::from_u32(*u).unwrap_or('?'),
        None => b as char, // ASCII y todo lo que no esta en la tabla
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_es_identidad() {
        assert_eq!(glifo_de('A'), b'A');
        assert_eq!(glifo_de(' '), b' ');
        assert_eq!(char_de_glifo(b'a'), 'a');
    }

    #[test]
    fn los_acentos_de_cp437_en_su_sitio() {
        assert_eq!(glifo_de('ñ'), 0xA4);
        assert_eq!(glifo_de('Ñ'), 0xA5);
        assert_eq!(glifo_de('é'), 0x82);
        assert_eq!(glifo_de('É'), 0x90);
        assert_eq!(glifo_de('á'), 0xA0);
        assert_eq!(glifo_de('í'), 0xA1);
        assert_eq!(glifo_de('ó'), 0xA2);
        assert_eq!(glifo_de('ú'), 0xA3);
        assert_eq!(glifo_de('ü'), 0x81);
        assert_eq!(glifo_de('¿'), 0xA8);
        assert_eq!(glifo_de('¡'), 0xAD);
        assert_eq!(glifo_de('Ç'), 0x80);
        assert_eq!(glifo_de('Ä'), 0x8E);
        assert_eq!(glifo_de('Ö'), 0x99);
        assert_eq!(glifo_de('Ü'), 0x9A);
    }

    #[test]
    fn los_23_compuestos_tienen_glifo() {
        for u in COMPONER {
            let c = char::from_u32(u).expect("codigo Latin-1 valido");
            let g = glifo_de(c);
            assert_ne!(g, b'?', "{c} debe tener glifo");
            // Si la ROM ya lo traía (É en 0x90) se usa su glifo nativo; si no,
            // el slot Latin-1 donde bios::COMPONER lo compone.
            let nativo = CP437.iter().find(|(_, x)| *x == u).map(|(g, _)| *g);
            assert_eq!(g, nativo.unwrap_or(u as u8), "glifo de {c}");
            assert_eq!(char_de_glifo(g), c, "y volver de él");
        }
    }

    #[test]
    fn el_slot_pisado_por_la_composicion_ya_no_es_2() {
        // 0xFD es ² en la ROM, pero bios::COMPONER compuso ý encima: en la
        // fuente de la GUI ese byte dibuja ý, así que ² ya no tiene sitio y
        // la ida y vuelta (byte → char → byte) queda estable.
        assert_eq!(glifo_de('²'), b'?');
        assert_eq!(glifo_de('ý'), 0xFD);
        assert_eq!(char_de_glifo(0xFD), 'ý');
    }

    #[test]
    fn un_texto_espanol_completo_no_cae_en_el_interrogante() {
        for c in "ñÑéÉáíóúüÜ¿¡Ççàâäåèêëîïòôõûùÿßµ±÷°·«»ªº¬½¼¢£¥á".chars()
        {
            assert_ne!(glifo_de(c), b'?', "este no tenía glifo: {c}");
        }
    }

    #[test]
    fn lo_que_no_tiene_glifo_sale_de_pregunta() {
        assert_eq!(glifo_de('€'), b'?');
        assert_eq!(glifo_de('—'), b'?');
    }

    #[test]
    fn el_descodificador_utf8_deja_un_solo_caracter() {
        assert_eq!(utf8_en("ñ".as_bytes(), 0), Some((0xF1, 2)));
        assert_eq!(utf8_en("Pequeño".as_bytes(), 5), Some((0xF1, 2)));
        assert_eq!(utf8_en("ñ".as_bytes(), 1), None, "continuación suelta");
        assert_eq!(utf8_en(&[0xC3], 0), None, "secuencia cortada");
        assert_eq!(utf8_en(&[0x80], 0), None, "sin byte inicial");
    }
}
