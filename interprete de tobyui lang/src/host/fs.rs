//! Shim de `fs` para el anfitrion de Windows.
//!
//! Toby UI solo admite recursos dentro de `/usr/share/tobyui/`; en rocky eso
//! vive en `rockydisk.img`. En el host no hay disco Estrato, asi que esta capa
//! traduce esa misma ruta virtual a los ficheros reales del kit AQUA que hay en
//! el repositorio (`../AQUA/`), manteniendo el MISMO contrato que `crate::fs`
//! del kernel: `montado`, `raiz`, `resolver`, `tamano` y `leer`.
//!
//! `themes/default/<fichero>` se corresponde con la raiz del kit AQUA
//! (`left_button_blue.png`, `body_button_green.png`, ...).

use alloc::vec::Vec;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Ficheros ya resueltos, en memoria, indexados por el registro que devuelve
/// `resolver` (el mismo "descriptor" opaco que usa el kernel).
static FICHEROS: OnceLock<Mutex<Vec<Vec<u8>>>> = OnceLock::new();

fn almacen() -> &'static Mutex<Vec<Vec<u8>>> {
    FICHEROS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Carpeta real que hace de `/usr/share/tobyui/`: el kit AQUA del repositorio.
fn raiz_real() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("AQUA")
}

/// Traduce una ruta virtual autorizada a un fichero real del kit.
fn mapear(ruta: &str) -> Option<PathBuf> {
    const RAIZ_APPS_SISTEMA: &str = "/system/apps/";
    if let Some(resto) = ruta.strip_prefix(RAIZ_APPS_SISTEMA) {
        if resto.is_empty()
            || resto.contains("..")
            || resto.contains('\\')
            || resto.starts_with('/')
        {
            return None;
        }
        return Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("system_apps")
                .join(resto),
        );
    }

    const RAIZ: &str = "/usr/share/tobyui/";
    let resto = ruta.strip_prefix(RAIZ)?;
    // El tema por defecto ocupa la raiz del kit AQUA.
    let resto = resto.strip_prefix("themes/default/").unwrap_or(resto);
    if resto.is_empty() || resto.contains("..") || resto.contains('\\') {
        return None;
    }
    Some(raiz_real().join(resto))
}

/// Siempre hay disco en el anfitrion.
pub fn montado() -> bool {
    true
}

/// Registro de la raiz (0), como en el kernel.
pub fn raiz() -> u32 {
    0
}

/// Resuelve una ruta y deja su contenido en memoria; devuelve su registro.
pub fn resolver(_cwd: u32, ruta: &str) -> Result<u32, ()> {
    let real = mapear(ruta).ok_or(())?;
    let datos = std::fs::read(&real).map_err(|_| ())?;
    let mut almacen = almacen().lock().map_err(|_| ())?;
    almacen.push(datos);
    Ok((almacen.len() - 1) as u32)
}

/// Tamano en bytes del fichero resuelto.
pub fn tamano(reg: u32) -> u32 {
    almacen()
        .lock()
        .ok()
        .and_then(|a| a.get(reg as usize).map(|d| d.len() as u32))
        .unwrap_or(0)
}

/// Copia hasta `buf.len()` bytes del fichero resuelto, desde `offset`.
pub fn leer(reg: u32, offset: usize, buf: &mut [u8]) -> usize {
    let almacen = match almacen().lock() {
        Ok(a) => a,
        Err(_) => return 0,
    };
    let datos = match almacen.get(reg as usize) {
        Some(d) => d,
        None => return 0,
    };
    if offset >= datos.len() {
        return 0;
    }
    let n = core::cmp::min(buf.len(), datos.len() - offset);
    buf[..n].copy_from_slice(&datos[offset..offset + n]);
    n
}
