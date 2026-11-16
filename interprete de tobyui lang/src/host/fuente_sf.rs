//! Caché SF Pro Display Medium compartida por los anfitriones de escritorio.
//!
//! Kira carga la caché desde `/usr/share/fonts`; AppKit y el intérprete de
//! Windows usan estos mismos bytes para que la vista previa sea WYSIWYG.

use crate::toby_ui::renderer::FuenteSuave;

const CACHE: &[u8] = include_bytes!("../../../Fonts/SFPRODISPLAYMEDIUM.rka");
const ANCHO: usize = 12;
const ALTO: usize = 24;
const CABECERA: &[u8; 8] = b"RKA3\x0C\x18\x00\x01";
const GLIFOS: usize = 256;
const TAMANO_TABLA_KERNING: usize = GLIFOS * GLIFOS;
const TAMANO_GRAFICOS: usize = 8 + GLIFOS + TAMANO_TABLA_KERNING + ANCHO * ALTO * GLIFOS;

/// Vista estática de la caché de SF Pro en el formato RKA3 del kernel.
pub fn sistema() -> Option<FuenteSuave> {
    if CACHE.len() != TAMANO_GRAFICOS || CACHE.get(..CABECERA.len())? != CABECERA {
        return None;
    }
    let base = CACHE.as_ptr() as usize;
    Some(FuenteSuave {
        addr: base + 8 + GLIFOS + TAMANO_TABLA_KERNING,
        avances_addr: base + 8,
        kerning_addr: base + 8 + GLIFOS,
        ancho: ANCHO,
        alto: ALTO,
        alto_base: 16,
        bytes_por_glifo: ANCHO * ALTO,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn cache_sf_pro_usa_la_resolucion_alta() {
        let fuente = sistema().expect("caché SF Pro completa");
        assert_eq!((fuente.ancho, fuente.alto, fuente.alto_base), (12, 24, 16));
        assert_eq!(fuente.bytes_por_glifo, 12 * 24);
    }
}
