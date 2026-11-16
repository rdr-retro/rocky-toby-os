//! Shim de `bios` para el anfitrion.
//!
//! En rocky `bios::Font` describe la fuente de la VGA que el kernel copia a
//! memoria. Aqui no hay BIOS: la fuente esta incrustada (se extrajo de
//! `vgaoem.fon` al generar `fuente.rs`), pero el `Font` que ve el motor del
//! lenguaje es exactamente el mismo: direccion, bytes por glifo y filas.

use crate::fuente;

/// Fuente bitmap que lee el `CanvasRenderer` de Toby UI.
///
/// `addr` apunta a los glifos (uno tras otro, un byte por fila, MSB a la
/// izquierda) y el glifo de un caracter esta en el indice de ese caracter.
pub struct Font {
    /// Direccion del primer byte de los glifos.
    pub addr: usize,
    /// Bytes que ocupa cada glifo (una fila por byte).
    pub bytes_per_char: usize,
    /// Filas (alto) de cada glifo.
    pub rows: usize,
}

/// Fuente incrustada del anfitrion.
pub fn get_font() -> Option<Font> {
    Some(Font {
        addr: fuente::GLIFOS.as_ptr() as usize,
        bytes_per_char: fuente::ALTO,
        rows: fuente::ALTO,
    })
}
