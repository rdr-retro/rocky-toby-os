//! Recursos de Toby UI (Fase 4).
//!
//! Carga PNG y documentos Toby desde Estrato (`fs.rs`), sólo desde recursos del
//! sistema o dentro de un bundle `.app` instalado. El lenguaje no permite
//! rutas libres: un documento que pida algo fuera de esas raíces se rechaza.
//!
//! Rutas autorizadas: recursos Toby UI, fondos de `/usr/share/backgrounds/`
//! y recursos dentro de bundles `.app` instalados.

use alloc::vec;
use alloc::vec::Vec;

/// Carpeta de recursos del tema del sistema (sin barra final).
pub const RAIZ_RECURSOS: &str = "/usr/share/tobyui/";
/// Carpeta de fondos del sistema.
pub const RAIZ_FONDOS: &str = "/usr/share/backgrounds/";
/// Carpeta de aplicaciones empaquetadas.
pub const RAIZ_APPS: &str = "/home/apps/";
/// Carpeta donde se instalan los bundles del sistema.
pub const RAIZ_APPS_SISTEMA: &str = "/system/apps/";

/// Tamaño máximo razonable de un PNG de tema (evita agotar el heap).
pub const MAX_BYTES: usize = 512 * 1024;
/// Fondos fotográficos: límite mayor que el de piezas pequeñas de interfaz.
pub const MAX_FONDO_BYTES: usize = 2 * 1024 * 1024;
/// Tamaño máximo de lado de una imagen.
///
/// El kit AQUA trae piezas de hasta ~1000 px (tics, flechas); 1024 las admite.
pub const MAX_LADO: usize = 1024;
/// Dimensiones/píxeles máximos de fondos de pantalla del sistema.
const MAX_FONDO_LADO: usize = 1500;
const MAX_FONDO_PIXELES: usize = 1_600_000;
/// Tamaño máximo de un documento `.toby` (el mismo tope que el parser).
pub const MAX_DOC: usize = super::parser::MAX_BYTES;

/// Error de carga de un recurso.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorResource {
    /// La ruta no está dentro de la carpeta autorizada (o intenta salir).
    RutaNoAutorizada,
    /// El sistema de ficheros no está montado.
    NoMontado,
    /// El fichero no existe.
    NoExiste,
    /// El fichero existe pero es demasiado grande.
    DemasiadoGrande,
    /// No es un PNG válido o está corrupto.
    Formato,
}

impl ErrorResource {
    /// Descripción breve del error.
    pub fn descripcion(&self) -> &'static str {
        match self {
            ErrorResource::RutaNoAutorizada => "ruta de recurso no autorizada",
            ErrorResource::NoMontado => "sistema de ficheros no montado",
            ErrorResource::NoExiste => "recurso no encontrado",
            ErrorResource::DemasiadoGrande => "recurso demasiado grande",
            ErrorResource::Formato => "formato de recurso no válido",
        }
    }
}

/// Una imagen ya decodificada (píxeles `0xAARRGGBB` en filas).
#[derive(Clone, Debug)]
pub struct Imagen {
    pub ancho: u32,
    pub alto: u32,
    pub pixeles: Vec<u32>,
}

/// ¿Está la ruta dentro de la carpeta autorizada, sin trucos de navegación?
///
/// Reglas: prefijo exacto, sin `..`, sin `//`, sin segmentos ocultos y
/// extensión `.png`.
pub fn autorizada(ruta: &str) -> bool {
    autorizada_ext(ruta, ".png")
}

/// Igual que [`autorizada`], pero para documentos del lenguaje (`.toby`).
pub fn autorizada_doc(ruta: &str) -> bool {
    autorizada_ext(ruta, ".toby")
}

/// Reglas de ruta: raíz segura (sistema o bundle app), sin traversal ni
/// segmentos ocultos, y con la extensión pedida.
fn autorizada_ext(ruta: &str, ext: &str) -> bool {
    let resto = if let Some(resto) = ruta.strip_prefix(RAIZ_RECURSOS) {
        resto
    } else if ext == ".png" && ruta.starts_with(RAIZ_FONDOS) {
        ruta.strip_prefix(RAIZ_FONDOS).unwrap_or("")
    } else if let Some(resto) = ruta
        .strip_prefix(RAIZ_APPS)
        .or_else(|| ruta.strip_prefix(RAIZ_APPS_SISTEMA))
    {
        let Some((bundle, dentro)) = resto.split_once('/') else {
            return false;
        };
        let Some(nombre) = bundle.strip_suffix(".app") else {
            return false;
        };
        if !nombre_bundle_valido(nombre) {
            return false;
        }
        dentro
    } else {
        return false;
    };

    !resto.is_empty()
        && resto.ends_with(ext)
        && !ruta.contains('\\')
        && resto.split('/').all(|segmento| {
            !segmento.is_empty()
                && segmento != "."
                && segmento != ".."
                && !segmento.starts_with('.')
        })
}

fn nombre_bundle_valido(nombre: &str) -> bool {
    let bytes = nombre.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= super::manifest::MAX_NOMBRE
        && (bytes[0].is_ascii_lowercase() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

/// Lee un documento `.toby` de la carpeta autorizada, en crudo.
///
/// Se devuelven bytes, no texto: el parser los acepta tal cual
/// (`parser::analizar_bytes`), así que un fichero con codificación rara se
/// rechaza en el parser y no aquí.
pub fn leer_doc(ruta: &str) -> Result<Vec<u8>, ErrorResource> {
    if !autorizada_doc(ruta) {
        return Err(ErrorResource::RutaNoAutorizada);
    }
    if !crate::fs::montado() {
        return Err(ErrorResource::NoMontado);
    }
    let reg = crate::fs::resolver(crate::fs::raiz(), ruta).map_err(|_| ErrorResource::NoExiste)?;
    let tam = crate::fs::tamano(reg) as usize;
    if tam == 0 {
        return Err(ErrorResource::NoExiste);
    }
    if tam > MAX_DOC {
        return Err(ErrorResource::DemasiadoGrande);
    }
    let mut buf: Vec<u8> = vec![0u8; tam];
    let leidos = crate::fs::leer(reg, 0, &mut buf);
    if leidos != tam {
        return Err(ErrorResource::Formato);
    }
    Ok(buf)
}

/// Carga y decodifica un PNG desde la carpeta autorizada.
pub fn cargar_png(ruta: &str) -> Result<Imagen, ErrorResource> {
    if !autorizada(ruta) {
        return Err(ErrorResource::RutaNoAutorizada);
    }
    if !crate::fs::montado() {
        return Err(ErrorResource::NoMontado);
    }
    let reg = crate::fs::resolver(crate::fs::raiz(), ruta).map_err(|_| ErrorResource::NoExiste)?;
    let tam = crate::fs::tamano(reg) as usize;
    if tam == 0 {
        return Err(ErrorResource::NoExiste);
    }
    let es_fondo = ruta.starts_with(RAIZ_FONDOS);
    let (max_bytes, max_lado, max_pixeles) = if es_fondo {
        (MAX_FONDO_BYTES, MAX_FONDO_LADO, MAX_FONDO_PIXELES)
    } else {
        (MAX_BYTES, MAX_LADO, MAX_LADO * MAX_LADO)
    };
    if tam > max_bytes {
        return Err(ErrorResource::DemasiadoGrande);
    }
    let mut buf: Vec<u8> = vec![0u8; tam];
    let leidos = crate::fs::leer(reg, 0, &mut buf);
    if leidos != tam {
        return Err(ErrorResource::Formato);
    }
    let Some((ancho, alto)) = dimensiones_png(&buf) else {
        return Err(ErrorResource::Formato);
    };
    if ancho == 0
        || alto == 0
        || ancho > max_lado
        || alto > max_lado
        || ancho
            .checked_mul(alto)
            .map_or(true, |pixeles| pixeles > max_pixeles)
    {
        return Err(ErrorResource::DemasiadoGrande);
    }
    let img = super::png_lib::decodificar(&buf).map_err(|_| ErrorResource::Formato)?;
    if img.ancho != ancho || img.alto != alto {
        return Err(ErrorResource::DemasiadoGrande);
    }
    Ok(Imagen {
        ancho: img.ancho as u32,
        alto: img.alto as u32,
        pixeles: img.pixeles,
    })
}

/// Lee el IHDR antes de decodificar para que un PNG con dimensiones enormes no
/// fuerce asignaciones descontroladas dentro del decodificador.
fn dimensiones_png(datos: &[u8]) -> Option<(usize, usize)> {
    if datos.len() < 24
        || &datos[..8] != b"\x89PNG\r\n\x1a\n"
        || datos[8..12] != [0, 0, 0, 13]
        || &datos[12..16] != b"IHDR"
    {
        return None;
    }
    let ancho = u32::from_be_bytes(datos[16..20].try_into().ok()?) as usize;
    let alto = u32::from_be_bytes(datos[20..24].try_into().ok()?) as usize;
    Some((ancho, alto))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn rutas_autorizadas() {
        assert!(autorizada("/usr/share/tobyui/aqua/boton.png"));
        assert!(autorizada("/usr/share/tobyui/botones/left.png"));
        assert!(autorizada("/usr/share/backgrounds/flower.png"));
        assert!(autorizada(
            "/usr/share/backgrounds/flower intense clean.png"
        ));
    }

    #[test]
    fn rutas_rechazadas() {
        assert!(!autorizada("/etc/passwd"));
        assert!(!autorizada("/usr/share/tobyui/../../etc/passwd"));
        assert!(!autorizada("/usr/share/tobyui//x.png"));
        assert!(!autorizada("/usr/share/tobyui/"));
        assert!(!autorizada("/usr/share/tobyui/a.png.exe"));
        assert!(!autorizada("/usr/share/tobyui\\x.png"));
        assert!(!autorizada_doc("/usr/share/backgrounds/nota.toby"));
        assert!(!autorizada("/usr/share/backgrounds/../etc/passwd.png"));
        assert!(!autorizada(""));
    }

    #[test]
    fn dimensiones_png_se_validan_antes_de_decodificar() {
        let mut cabecera = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        cabecera.extend_from_slice(&1448u32.to_be_bytes());
        cabecera.extend_from_slice(&1086u32.to_be_bytes());
        assert_eq!(dimensiones_png(&cabecera), Some((1448, 1086)));
        assert!(1448usize * 1086 <= MAX_FONDO_PIXELES);
        assert_eq!(dimensiones_png(b"not png"), None);
    }

    #[test]
    fn documentos_autorizados_solo_toby() {
        assert!(autorizada_doc("/usr/share/tobyui/demo.toby"));
        assert!(autorizada_doc("/usr/share/tobyui/apps/reloj.toby"));
        // Un PNG no es un documento y viceversa.
        assert!(!autorizada_doc("/usr/share/tobyui/x.png"));
        assert!(!autorizada("/usr/share/tobyui/demo.toby"));
        assert!(!autorizada_doc("/etc/x.toby"));
        assert!(!autorizada_doc("/usr/share/tobyui/../x.toby"));
        assert!(!autorizada_doc("/usr/share/tobyui/a.toby.bak"));
        assert!(!autorizada_doc("/usr/share/tobyui/"));
        assert!(autorizada_doc("/home/apps/reloj.app/ui/principal.toby"));
        assert!(autorizada_doc(
            "/system/apps/gestor_inicio.app/ui/main.toby"
        ));
        assert!(!autorizada_doc("/home/apps/reloj.rapp/ui/principal.toby"));
        assert!(!autorizada_doc("/home/apps/reloj.app/../escape.toby"));
        assert!(!autorizada_doc("/home/apps/Reloj.app/ui/principal.toby"));
        assert!(autorizada("/home/apps/reloj.app/img/icon.png"));
    }
}
