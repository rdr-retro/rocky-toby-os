//! Fuentes instaladas en el sistema para Kira.
//!
//! Los originales TTF/OTF viven en `/usr/share/fonts`. El build instala cachés
//! RKF1 para el renderer bitmap y RKA3 con métricas, cobertura gris y kerning para la barra
//! de menús; el kernel no necesita incorporar un parser TrueType.

use alloc::vec;
use alloc::vec::Vec;

use crate::bios::Font;
use crate::sync::SyncCell;

const ANCHO: usize = 8;
const ALTO: usize = 16;
const GLIFOS: usize = 256;
const CABECERA: &[u8; 8] = b"RKF1\x08\x10\x00\x01";
const CABECERA_TITULO_ACERCA: &[u8; 4] = b"RKT1";
const TAMANO_CABECERA_TITULO_ACERCA: usize = 12;
const CABECERA_TEXTEDIT: &[u8; 8] = b"RKA4\x14\x18\x00\x01";
const ANCHO_TEXTEDIT: usize = 20;
const ALTO_TEXTEDIT: usize = 24;
// Un byte empaqueta los píxeles de cada fila; no es un byte por píxel.
const BYTES_POR_FILA: usize = (ANCHO + 7) / 8;
const TAMANO_TABLA_AVANCES: usize = GLIFOS;
const TAMANO_TABLA_KERNING: usize = GLIFOS * GLIFOS;
const TAMANO_ARCHIVO: usize = 8 + BYTES_POR_FILA * ALTO * GLIFOS;
const TAMANO_ARCHIVO_TEXTEDIT: usize = 8 + GLIFOS + ANCHO_TEXTEDIT * ALTO_TEXTEDIT * GLIFOS;

static INTENTADA: SyncCell<bool> = SyncCell::new(false);
static DATOS: SyncCell<Option<Vec<u8>>> = SyncCell::new(None);
static FUENTE: SyncCell<Option<Font>> = SyncCell::new(None);
static DATOS_SUAVES: SyncCell<Option<Vec<u8>>> = SyncCell::new(None);
static FUENTE_SUAVE: SyncCell<Option<crate::toby_ui::renderer::FuenteSuave>> = SyncCell::new(None);
static INTENTADA_TEXTEDIT: SyncCell<bool> = SyncCell::new(false);
static DATOS_TEXTEDIT: SyncCell<Option<Vec<u8>>> = SyncCell::new(None);
static FUENTE_TEXTEDIT: SyncCell<Option<crate::toby_ui::renderer::FuenteSuave>> =
    SyncCell::new(None);
static INTENTADA_ACERCA: SyncCell<bool> = SyncCell::new(false);
static DATOS_ACERCA: SyncCell<Option<Vec<u8>>> = SyncCell::new(None);
static FUENTE_ACERCA: SyncCell<Option<crate::toby_ui::renderer::FuenteSuave>> = SyncCell::new(None);
static INTENTADA_TITULO_ACERCA: SyncCell<bool> = SyncCell::new(false);
static DATOS_TITULO_ACERCA: SyncCell<Option<Vec<u8>>> = SyncCell::new(None);
static TITULO_ACERCA: SyncCell<Option<TituloAcerca>> = SyncCell::new(None);
static DATOS_TERMINAL: SyncCell<Option<Vec<u8>>> = SyncCell::new(None);
static FUENTE_TERMINAL: SyncCell<Option<Font>> = SyncCell::new(None);
static DATOS_TERMINAL_SUAVES: SyncCell<Option<Vec<u8>>> = SyncCell::new(None);
static FUENTE_TERMINAL_SUAVE: SyncCell<Option<crate::toby_ui::renderer::FuenteSuave>> =
    SyncCell::new(None);

#[derive(Clone, Copy)]
pub struct TituloAcerca {
    pub rocky: crate::toby_ui::renderer::MascaraSuave,
    pub ampersand: crate::toby_ui::renderer::MascaraSuave,
    pub toby_os: crate::toby_ui::renderer::MascaraSuave,
}

/// Intenta cargar una vez la cache SF Pro instalada en el volumen del sistema.
/// Si no está disponible, Kira conserva la fuente del BIOS.
pub fn inicializar() {
    if unsafe { *INTENTADA.get() } || !crate::fs::montado() {
        return;
    }

    let datos = cargar_cache(crate::espacio::FUENTE_KIRA)
        .or_else(|| cargar_cache(crate::espacio::FUENTE_KIRA_ALTERNATIVA));
    let datos_suaves = cargar_cache_suave(crate::espacio::FUENTE_KIRA_SUAVE);
    let datos_terminal = cargar_cache(crate::espacio::FUENTE_TERMINAL);
    let datos_terminal_suaves = cargar_cache_suave(crate::espacio::FUENTE_TERMINAL_SUAVE);
    unsafe {
        if let Some(datos) = datos {
            let addr = datos.as_ptr() as usize;
            *DATOS.get() = Some(datos);
            *FUENTE.get() = Some(Font {
                addr,
                bytes_per_char: ALTO,
                rows: ALTO,
            });
        }
        if let Some((datos, ancho, alto)) = datos_suaves {
            let addr = datos.as_ptr() as usize;
            *DATOS_SUAVES.get() = Some(datos);
            *FUENTE_SUAVE.get() = Some(crate::toby_ui::renderer::FuenteSuave {
                addr: addr + TAMANO_TABLA_AVANCES + TAMANO_TABLA_KERNING,
                avances_addr: addr,
                kerning_addr: addr + TAMANO_TABLA_AVANCES,
                ancho,
                alto,
                alto_base: 16,
                bytes_por_glifo: ancho * alto,
            });
        }
        if let Some(datos) = datos_terminal {
            let addr = datos.as_ptr() as usize;
            *DATOS_TERMINAL.get() = Some(datos);
            *FUENTE_TERMINAL.get() = Some(Font {
                addr,
                bytes_per_char: ALTO,
                rows: ALTO,
            });
        }
        if let Some((datos, ancho, alto)) = datos_terminal_suaves {
            let addr = datos.as_ptr() as usize;
            *DATOS_TERMINAL_SUAVES.get() = Some(datos);
            *FUENTE_TERMINAL_SUAVE.get() = Some(crate::toby_ui::renderer::FuenteSuave {
                addr: addr + TAMANO_TABLA_AVANCES + TAMANO_TABLA_KERNING,
                avances_addr: addr,
                kerning_addr: addr + TAMANO_TABLA_AVANCES,
                ancho,
                alto,
                alto_base: 16,
                bytes_por_glifo: ancho * alto,
            });
        }
        *INTENTADA.get() = true;
    }
}

/// Fuente que usa el compositor: cache del sistema si existe; BIOS si no.
pub fn para_kira() -> Option<Font> {
    inicializar();
    unsafe { *FUENTE.get() }.or_else(crate::bios::get_font)
}

/// Caché antialias, avances proporcionales y kerning para la barra de menús.
pub fn para_barra_menu() -> Option<crate::toby_ui::renderer::FuenteSuave> {
    inicializar();
    unsafe { *FUENTE_SUAVE.get() }
}

/// Caché antialias SF Pro Display Regular para la información de «Acerca de».
/// Se carga solo cuando se abre esa ventana, sin cambiar el peso del topbar.
pub fn para_acerca() -> Option<crate::toby_ui::renderer::FuenteSuave> {
    if !unsafe { *INTENTADA_ACERCA.get() } {
        if !crate::fs::montado() {
            return None;
        }
        let datos = cargar_cache_suave(crate::espacio::FUENTE_ACERCA);
        unsafe {
            if let Some((datos, ancho, alto)) = datos {
                let addr = datos.as_ptr() as usize;
                *DATOS_ACERCA.get() = Some(datos);
                *FUENTE_ACERCA.get() = Some(crate::toby_ui::renderer::FuenteSuave {
                    addr: addr + TAMANO_TABLA_AVANCES + TAMANO_TABLA_KERNING,
                    avances_addr: addr,
                    kerning_addr: addr + TAMANO_TABLA_AVANCES,
                    ancho,
                    alto,
                    alto_base: 16,
                    bytes_por_glifo: ancho * alto,
                });
            }
            *INTENTADA_ACERCA.get() = true;
        }
    }
    unsafe { *FUENTE_ACERCA.get() }
}

/// Máscaras Times renderizadas al tamaño de pantalla del título de Acerca de.
/// Se cargan una sola vez y ocupan solo unas decenas de KiB.
pub fn titulo_acerca() -> Option<TituloAcerca> {
    if !unsafe { *INTENTADA_TITULO_ACERCA.get() } {
        if !crate::fs::montado() {
            return None;
        }
        if let Some((datos, alto, anchos)) =
            cargar_mascaras_titulo(crate::espacio::FUENTE_ACERCA_TITULO)
        {
            let addr = datos.as_ptr() as usize;
            let rocky_addr = addr;
            let ampersand_addr = rocky_addr + anchos[0] * alto;
            let toby_os_addr = ampersand_addr + anchos[1] * alto;
            unsafe {
                *DATOS_TITULO_ACERCA.get() = Some(datos);
                *TITULO_ACERCA.get() = Some(TituloAcerca {
                    rocky: crate::toby_ui::renderer::MascaraSuave {
                        addr: rocky_addr,
                        ancho: anchos[0] as u32,
                        alto: alto as u32,
                    },
                    ampersand: crate::toby_ui::renderer::MascaraSuave {
                        addr: ampersand_addr,
                        ancho: anchos[1] as u32,
                        alto: alto as u32,
                    },
                    toby_os: crate::toby_ui::renderer::MascaraSuave {
                        addr: toby_os_addr,
                        ancho: anchos[2] as u32,
                        alto: alto as u32,
                    },
                });
            }
        }
        unsafe {
            *INTENTADA_TITULO_ACERCA.get() = true;
        }
    }
    unsafe { *TITULO_ACERCA.get() }
}

/// Fuente Times antialias del editor. Se carga bajo demanda para no aumentar
/// el uso de memoria de Kira mientras TextEdit no se esté usando.
pub fn para_textedit() -> Option<crate::toby_ui::renderer::FuenteSuave> {
    if !unsafe { *INTENTADA_TEXTEDIT.get() } {
        if !crate::fs::montado() {
            return None;
        }
        let datos = cargar_cache_textedit(crate::espacio::FUENTE_TEXTEDIT);
        unsafe {
            if let Some(datos) = datos {
                let addr = datos.as_ptr() as usize;
                *DATOS_TEXTEDIT.get() = Some(datos);
                *FUENTE_TEXTEDIT.get() = Some(crate::toby_ui::renderer::FuenteSuave {
                    addr: addr + GLIFOS,
                    avances_addr: addr,
                    // Cero indica que la caché compacta no trae tabla de kerning.
                    kerning_addr: 0,
                    ancho: ANCHO_TEXTEDIT,
                    alto: ALTO_TEXTEDIT,
                    alto_base: ALTO_TEXTEDIT,
                    bytes_por_glifo: ANCHO_TEXTEDIT * ALTO_TEXTEDIT,
                });
            }
            *INTENTADA_TEXTEDIT.get() = true;
        }
    }
    unsafe { *FUENTE_TEXTEDIT.get() }
}

/// Fuente bitmap monoespaciada Cascadia Code; si falta, usa la fuente de Kira.
pub fn para_terminal() -> Option<Font> {
    inicializar();
    unsafe { *FUENTE_TERMINAL.get() }.or_else(para_kira)
}

/// Cache antialias monoespaciada de Cascadia Code para la Terminal.
pub fn para_terminal_suave() -> Option<crate::toby_ui::renderer::FuenteSuave> {
    inicializar();
    unsafe { *FUENTE_TERMINAL_SUAVE.get() }
}

fn cargar_cache(ruta: &str) -> Option<Vec<u8>> {
    let reg = crate::fs::resolver(crate::fs::raiz(), ruta).ok()?;
    let tamano = crate::fs::tamano(reg) as usize;
    if tamano != TAMANO_ARCHIVO {
        return None;
    }
    let mut archivo = vec![0; tamano];
    if crate::fs::leer(reg, 0, &mut archivo) != tamano {
        return None;
    }
    Some(parsear(&archivo)?.to_vec())
}

fn cargar_cache_suave(ruta: &str) -> Option<(Vec<u8>, usize, usize)> {
    let reg = crate::fs::resolver(crate::fs::raiz(), ruta).ok()?;
    let tamano = crate::fs::tamano(reg) as usize;
    let mut cabecera = [0; 8];
    if crate::fs::leer(reg, 0, &mut cabecera) != cabecera.len() {
        return None;
    }
    let (ancho, alto, esperado) = dimensiones_cache_suave(&cabecera, tamano)?;
    let mut archivo = vec![0; tamano];
    if crate::fs::leer(reg, 0, &mut archivo) != tamano {
        return None;
    }
    let (ancho_validado, alto_validado, _) = parsear_suave(&archivo)?;
    if ancho_validado != ancho || alto_validado != alto || archivo.len() != esperado {
        return None;
    }
    // Quita la cabecera en el mismo buffer: las cachés se conservan en memoria,
    // pero evitamos duplicarla temporalmente durante cada carga.
    archivo.copy_within(cabecera.len().., 0);
    archivo.truncate(esperado - cabecera.len());
    Some((archivo, ancho, alto))
}

fn cargar_mascaras_titulo(ruta: &str) -> Option<(Vec<u8>, usize, [usize; 3])> {
    let reg = crate::fs::resolver(crate::fs::raiz(), ruta).ok()?;
    let tamano = crate::fs::tamano(reg) as usize;
    let mut cabecera = [0; TAMANO_CABECERA_TITULO_ACERCA];
    if crate::fs::leer(reg, 0, &mut cabecera) != cabecera.len()
        || cabecera.get(..4)? != CABECERA_TITULO_ACERCA
    {
        return None;
    }
    let palabra =
        |inicio: usize| u16::from_le_bytes([cabecera[inicio], cabecera[inicio + 1]]) as usize;
    let alto = palabra(4);
    let anchos = [palabra(6), palabra(8), palabra(10)];
    if alto == 0 || alto > 96 || anchos.iter().any(|ancho| *ancho == 0 || *ancho > 512) {
        return None;
    }
    let pixeles = anchos.iter().try_fold(0usize, |total, ancho| {
        total.checked_add(ancho.checked_mul(alto)?)
    })?;
    if tamano != TAMANO_CABECERA_TITULO_ACERCA.checked_add(pixeles)? {
        return None;
    }
    let mut archivo = vec![0; tamano];
    if crate::fs::leer(reg, 0, &mut archivo) != tamano {
        return None;
    }
    archivo.copy_within(TAMANO_CABECERA_TITULO_ACERCA.., 0);
    archivo.truncate(pixeles);
    Some((archivo, alto, anchos))
}

fn cargar_cache_textedit(ruta: &str) -> Option<Vec<u8>> {
    let reg = crate::fs::resolver(crate::fs::raiz(), ruta).ok()?;
    let tamano = crate::fs::tamano(reg) as usize;
    if tamano != TAMANO_ARCHIVO_TEXTEDIT {
        return None;
    }
    let mut archivo = vec![0; tamano];
    if crate::fs::leer(reg, 0, &mut archivo) != tamano
        || archivo.get(..CABECERA_TEXTEDIT.len())? != CABECERA_TEXTEDIT
    {
        return None;
    }
    archivo.copy_within(CABECERA_TEXTEDIT.len().., 0);
    archivo.truncate(TAMANO_ARCHIVO_TEXTEDIT - CABECERA_TEXTEDIT.len());
    Some(archivo)
}

fn parsear(archivo: &[u8]) -> Option<&[u8]> {
    if archivo.len() != TAMANO_ARCHIVO || archivo.get(..CABECERA.len())? != CABECERA {
        return None;
    }
    archivo.get(CABECERA.len()..)
}

fn dimensiones_cache_suave(cabecera: &[u8], longitud: usize) -> Option<(usize, usize, usize)> {
    if cabecera.get(..4)? != b"RKA3" || cabecera.get(6..8)? != [0, 1] {
        return None;
    }
    let ancho = cabecera[4] as usize;
    let alto = cabecera[5] as usize;
    if ancho == 0 || alto == 0 || ancho > 32 || alto > 48 {
        return None;
    }
    let esperado = 8usize
        .checked_add(TAMANO_TABLA_AVANCES)?
        .checked_add(TAMANO_TABLA_KERNING)?
        .checked_add(ancho.checked_mul(alto)?.checked_mul(GLIFOS)?)?;
    (longitud == esperado).then_some((ancho, alto, esperado))
}

fn parsear_suave(archivo: &[u8]) -> Option<(usize, usize, &[u8])> {
    let cabecera: &[u8; 8] = archivo.get(..8)?.try_into().ok()?;
    let (ancho, alto, _) = dimensiones_cache_suave(cabecera, archivo.len())?;
    Some((ancho, alto, archivo.get(8..)?))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn valida_cache_y_devuelve_glifos() {
        let mut archivo = vec![0; TAMANO_ARCHIVO];
        archivo[..CABECERA.len()].copy_from_slice(CABECERA);
        archivo[CABECERA.len()] = 0xA5;
        let glifos = parsear(&archivo).unwrap();
        assert_eq!(glifos.len(), 4096);
        assert_eq!(glifos[0], 0xA5);
    }

    #[test]
    fn rechaza_cache_truncada_o_de_otro_formato() {
        assert!(parsear(&[0; TAMANO_ARCHIVO]).is_none());
        let mut truncada = vec![0; TAMANO_ARCHIVO - 1];
        truncada[..CABECERA.len()].copy_from_slice(CABECERA);
        assert!(parsear(&truncada).is_none());
    }

    #[test]
    fn valida_cache_antialias_de_resolucion_variable() {
        for (ancho, alto) in [(8, 16), (12, 24)] {
            let mut archivo =
                vec![0; 8 + TAMANO_TABLA_AVANCES + TAMANO_TABLA_KERNING + ancho * alto * GLIFOS];
            archivo[..4].copy_from_slice(b"RKA3");
            archivo[4] = ancho as u8;
            archivo[5] = alto as u8;
            archivo[6..8].copy_from_slice(&(GLIFOS as u16).to_le_bytes());
            archivo[8 + TAMANO_TABLA_AVANCES + TAMANO_TABLA_KERNING] = 127;
            let (ancho_leido, alto_leido, glifos) = parsear_suave(&archivo).unwrap();
            assert_eq!((ancho_leido, alto_leido), (ancho, alto));
            assert_eq!(
                glifos.len(),
                TAMANO_TABLA_AVANCES + TAMANO_TABLA_KERNING + ancho * alto * GLIFOS
            );
            assert_eq!(glifos[TAMANO_TABLA_AVANCES + TAMANO_TABLA_KERNING], 127);
        }
    }
}
