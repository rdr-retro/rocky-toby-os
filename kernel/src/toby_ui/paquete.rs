//! Apertura y validación de bundles `.app` almacenados en Estrato.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::{espacio, fs};

use super::{manifest, parser, validation};

const MAX_CODIGO_FUENTE: usize = 1024 * 1024;
const MAX_BINARIO: usize = 16 * 1024 * 1024;

/// Abre un paquete instalado desde `/system/apps/` o `/home/apps/` y valida todas
/// sus vistas y los ficheros Rust declarados.
pub fn cargar(ruta: &str) -> Result<manifest::Manifiesto, String> {
    if !ruta_paquete_valida(ruta) {
        return Err(String::from(
            "la app debe estar en /system/apps/<nombre>.app o /home/apps/<nombre>.app",
        ));
    }
    let registro = fs::resolver(fs::raiz(), ruta)
        .map_err(|_| String::from("no se encontró la carpeta .app"))?;
    if fs::tipo(registro) != fs::Tipo::Directorio {
        return Err(String::from("la ruta .app no es una carpeta"));
    }

    let manifiesto_ruta = format!("{}/{}", ruta, manifest::FICHERO);
    let bytes = leer(&manifiesto_ruta, manifest::MAX_BYTES)?;
    let manifiesto = manifest::cargar(&bytes).map_err(|errores| {
        let mut resumen = String::from("manifest.xml inválido");
        for error in errores.iter().take(3) {
            resumen.push_str("; ");
            resumen.push_str(&error.to_string());
        }
        resumen
    })?;

    let (base, nombre_carpeta) = raiz_y_nombre(ruta).unwrap_or(("", ""));
    if nombre_carpeta != manifiesto.nombre {
        return Err(format!(
            "el nombre del bundle ({}) no coincide con manifest.xml ({})",
            nombre_carpeta, manifiesto.nombre
        ));
    }
    let clase_esperada = if base == espacio::APPS_SISTEMA {
        manifest::ClaseApp::Sistema
    } else {
        manifest::ClaseApp::Usuario
    };
    if manifiesto.inicio.clase != clase_esperada {
        return Err(String::from(
            "la clase declarada en <inicio> no coincide con la ubicación del bundle",
        ));
    }

    if manifiesto.ui.modo == manifest::ModoUi::Toby {
        for vista in &manifiesto.ui.vistas {
            let archivo = format!("{}/{}", ruta, vista.archivo);
            let bytes = leer(&archivo, parser::MAX_BYTES)
                .map_err(|e| format!("vista '{}': {}", vista.nombre, e))?;
            let doc = parser::analizar_bytes(&bytes)
                .map_err(|e| format!("vista '{}': {}", vista.nombre, e))?;
            validation::validar(&doc).map_err(|errores| {
                let detalle = errores
                    .first()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| String::from("documento no válido"));
                format!("vista '{}': {}", vista.nombre, detalle)
            })?;
        }
    }

    if let Some(codigo) = &manifiesto.codigo {
        if let Some(fuente) = &codigo.fuente {
            let archivo = format!("{}/{}", ruta, fuente);
            leer(&archivo, MAX_CODIGO_FUENTE)
                .map_err(|e| format!("fuente Rust '{}': {}", fuente, e))?;
        }
        if let Some(binario) = &codigo.binario {
            let archivo = format!("{}/{}", ruta, binario);
            let bytes = leer(&archivo, MAX_BINARIO)
                .map_err(|e| format!("binario Rust '{}': {}", binario, e))?;
            if bytes.len() < 4 || &bytes[..4] != b"\x7FELF" {
                return Err(format!("'{}' no es un ELF válido", binario));
            }
        }
    }

    Ok(manifiesto)
}

/// Resuelve el icono declarado por un bundle sin cargar sus vistas ni su
/// código. Finder puede consultar esta metadata ligera y precargar el PNG en
/// una tarea de fondo antes de que el compositor lo dibuje.
pub fn cargar_ruta_icono(ruta: &str) -> Option<String> {
    if !ruta_paquete_valida(ruta) || !fs::montado() {
        return None;
    }
    let manifiesto_ruta = format!("{}/{}", ruta, manifest::FICHERO);
    let registro = fs::resolver(fs::raiz(), &manifiesto_ruta).ok()?;
    if fs::tipo(registro) != fs::Tipo::Fichero {
        return None;
    }
    let tamano = fs::tamano(registro) as usize;
    if tamano == 0 || tamano > manifest::MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(tamano).ok()?;
    bytes.resize(tamano, 0);
    if fs::leer(registro, 0, &mut bytes) != tamano {
        return None;
    }
    let manifiesto = manifest::cargar(&bytes).ok()?;
    let icono = manifiesto.identidad.icono?;
    Some(format!("{}/{}", ruta, icono))
}

fn ruta_paquete_valida(ruta: &str) -> bool {
    let Some((_, nombre)) = raiz_y_nombre(ruta) else {
        return false;
    };
    if nombre.is_empty() || nombre.contains('/') || ruta.contains('\\') || ruta.contains("..") {
        return false;
    }
    let bytes = nombre.as_bytes();
    bytes.len() <= manifest::MAX_NOMBRE
        && (bytes[0].is_ascii_lowercase() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

fn raiz_y_nombre(ruta: &str) -> Option<(&'static str, &str)> {
    for base in [espacio::APPS_SISTEMA, espacio::APPS] {
        if let Some(nombre) = ruta
            .strip_prefix(base)
            .and_then(|resto| resto.strip_prefix('/'))
            .and_then(|resto| resto.strip_suffix(".app"))
        {
            return Some((base, nombre));
        }
    }
    None
}

fn leer(ruta: &str, max: usize) -> Result<Vec<u8>, String> {
    if !fs::montado() {
        return Err(String::from("el volumen Estrato no está montado"));
    }
    let registro =
        fs::resolver(fs::raiz(), ruta).map_err(|_| format!("no se encontró {}", ruta))?;
    if fs::tipo(registro) != fs::Tipo::Fichero {
        return Err(format!("{} no es un fichero", ruta));
    }
    let tamano = fs::tamano(registro) as usize;
    if tamano == 0 || tamano > max {
        return Err(format!("tamaño no válido para {}", ruta));
    }
    let mut bytes = vec![0; tamano];
    if fs::leer(registro, 0, &mut bytes) != tamano {
        return Err(format!("lectura incompleta de {}", ruta));
    }
    Ok(bytes)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn solo_admite_bundles_app_en_apps() {
        assert!(ruta_paquete_valida("/home/apps/reloj.app"));
        assert!(ruta_paquete_valida("/system/apps/gestor_inicio.app"));
        assert!(!ruta_paquete_valida("/home/escritorio/reloj.app"));
        assert!(!ruta_paquete_valida("/home/apps/../reloj.app"));
        assert!(!ruta_paquete_valida("/home/apps/reloj.rapp"));
        assert!(!ruta_paquete_valida("/home/apps/reloj.app/ui"));
    }
}
