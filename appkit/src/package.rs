//! Herramientas de Toby AppKit para crear, validar y compilar bundles `.app`.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::toby_ui::{manifest, parser, validation};

const TARGET: &str = "x86_64-unknown-none";
const MAX_ELF: u64 = 16 * 1024 * 1024;

/// Crea una app nueva, sin sobrescribir nada que ya exista.
pub fn nuevo(destino: &Path) -> Result<PathBuf, String> {
    nuevo_con_clase(destino, "usuario")
}

/// Crea un bundle marcado como aplicación de sistema para instalarlo en
/// `/system/apps`.
pub fn nuevo_sistema(destino: &Path) -> Result<PathBuf, String> {
    nuevo_con_clase(destino, "sistema")
}

fn nuevo_con_clase(destino: &Path, clase: &str) -> Result<PathBuf, String> {
    let destino = ruta_app(destino);
    let nombre = nombre_app(&destino)?;
    if destino.exists() {
        return Err(format!("ya existe {}", destino.display()));
    }

    let padre = destino
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(padre).map_err(|e| format!("no se pudo crear el directorio: {e}"))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let etapa = padre.join(format!(".{nombre}.appkit-{}-{nonce}", std::process::id()));
    fs::create_dir(&etapa).map_err(|e| format!("no se pudo preparar el paquete: {e}"))?;

    let resultado = crear_contenido(&etapa, &nombre, clase).and_then(|_| {
        validar_con_nombre(&etapa, Some(&nombre))?;
        fs::rename(&etapa, &destino).map_err(|e| format!("no se pudo finalizar el paquete: {e}"))
    });
    if resultado.is_err() {
        let _ = fs::remove_dir_all(&etapa);
    }
    resultado.map(|_| destino)
}

/// Lee y valida el manifiesto, cada vista y los ficheros Rust declarados.
pub fn validar(ruta: &Path) -> Result<manifest::Manifiesto, String> {
    validar_con_nombre(ruta, None)
}

fn validar_con_nombre(
    ruta: &Path,
    nombre_temporal: Option<&str>,
) -> Result<manifest::Manifiesto, String> {
    let raiz = fs::canonicalize(ruta)
        .map_err(|e| format!("no se encuentra el bundle {}: {e}", ruta.display()))?;
    if !raiz.is_dir() {
        return Err("el bundle .app debe ser una carpeta".into());
    }
    let nombre_carpeta = match nombre_temporal {
        Some(nombre) => nombre.to_string(),
        None => nombre_app(&raiz)?,
    };
    let bytes = fs::read(raiz.join(manifest::FICHERO))
        .map_err(|e| format!("no se pudo leer manifest.xml: {e}"))?;
    if bytes.len() > manifest::MAX_BYTES {
        return Err("manifest.xml supera el tamaño máximo".into());
    }
    let manifiesto = manifest::cargar(&bytes).map_err(|errores| {
        errores
            .iter()
            .take(3)
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    if nombre_carpeta != manifiesto.nombre {
        return Err(format!(
            "la carpeta {} no coincide con el nombre '{}' del manifiesto",
            nombre_carpeta, manifiesto.nombre
        ));
    }

    if let Some(icono) = &manifiesto.identidad.icono {
        leer_relativo(&raiz, icono, 512 * 1024)?;
    }
    for vista in &manifiesto.ui.vistas {
        let bytes = leer_relativo(&raiz, &vista.archivo, parser::MAX_BYTES)
            .map_err(|e| format!("vista '{}': {e}", vista.nombre))?;
        let documento =
            parser::analizar_bytes(&bytes).map_err(|e| format!("vista '{}': {e}", vista.nombre))?;
        validation::validar(&documento).map_err(|errores| {
            errores
                .first()
                .map(ToString::to_string)
                .unwrap_or_else(|| "documento Toby no válido".into())
        })?;
    }
    if let Some(codigo) = &manifiesto.codigo {
        if let Some(fuente) = &codigo.fuente {
            leer_relativo(&raiz, fuente, 1024 * 1024).map_err(|e| format!("fuente Rust: {e}"))?;
        }
        if let Some(binario) = &codigo.binario {
            let elf = leer_relativo(&raiz, binario, MAX_ELF as usize)
                .map_err(|e| format!("binario Rust: {e}"))?;
            if elf.len() < 4 || &elf[..4] != b"\x7FELF" {
                return Err(format!("{binario} no tiene cabecera ELF"));
            }
        }
    }
    Ok(manifiesto)
}

/// Compila el ejecutable Rust del bundle con el SDK y enlazador del kernel.
pub fn compilar(ruta: &Path) -> Result<PathBuf, String> {
    let manifiesto = validar(ruta)?;
    let codigo = manifiesto
        .codigo
        .as_ref()
        .ok_or_else(|| "manifest.xml no declara <codigo lenguaje=\"rust\">".to_string())?;
    if codigo.fuente.is_none() {
        return Err("el manifiesto no declara el archivo fuente Rust".into());
    }

    let raiz = fs::canonicalize(ruta).map_err(|e| e.to_string())?;
    let binario_rel = codigo
        .binario
        .clone()
        .unwrap_or_else(|| "code/app.elf".into());
    let binario = raiz.join(&binario_rel);
    let crate_code = raiz.join("code");
    let cargo = crate_code.join("Cargo.toml");
    if !cargo.is_file() {
        return Err("falta code/Cargo.toml; crea el paquete con AppKit nuevo".into());
    }

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "no se pudo resolver el directorio del kernel".to_string())?
        .to_path_buf();
    let sdk = repo.join("kernel/src/libraries/toby_appkit");
    let linker = sdk.join("rocky.ld");
    let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/applications")
        .join(&manifiesto.nombre);
    let rustflags = [
        "-Crelocation-model=pic".to_string(),
        "-Cpanic=abort".to_string(),
        format!("-Clink-arg=-T{}", linker.display()),
        "-Clink-arg=-static".to_string(),
    ]
    .join("\u{1f}");

    let estado = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(TARGET)
        .arg("--manifest-path")
        .arg(&cargo)
        .arg("--target-dir")
        .arg(&target_dir)
        .current_dir(&crate_code)
        .env("CARGO_ENCODED_RUSTFLAGS", rustflags)
        .env_remove("RUSTFLAGS")
        .status()
        .map_err(|e| format!("no se pudo iniciar cargo: {e}"))?;
    if !estado.success() {
        return Err(format!("cargo terminó con {estado}"));
    }

    let elf = target_dir
        .join(TARGET)
        .join("release")
        .join(&manifiesto.nombre);
    let metadatos = fs::metadata(&elf).map_err(|e| format!("no se generó el ELF: {e}"))?;
    if metadatos.len() < 4 || metadatos.len() > MAX_ELF {
        return Err(format!("tamaño ELF no admitido: {} bytes", metadatos.len()));
    }
    fs::create_dir_all(
        binario
            .parent()
            .ok_or_else(|| "ruta de salida binaria no válida".to_string())?,
    )
    .map_err(|e| format!("no se pudo crear code/: {e}"))?;
    fs::copy(&elf, &binario).map_err(|e| format!("no se pudo copiar el ELF al bundle: {e}"))?;

    if codigo.binario.is_none() {
        anadir_binario_manifest(&raiz.join(manifest::FICHERO), &binario_rel)?;
    }
    validar(&raiz)?;
    Ok(binario)
}

fn crear_contenido(etapa: &Path, nombre: &str, clase: &str) -> Result<(), String> {
    fs::create_dir_all(etapa.join("ui")).map_err(|e| e.to_string())?;
    fs::create_dir_all(etapa.join("code/src")).map_err(|e| e.to_string())?;

    let titulo = titulo_desde_nombre(nombre);
    let manifiesto = format!(
        "<app nombre=\"{nombre}\" version=\"1.0\">\n  <identidad titulo=\"{titulo}\"/>\n  <ui principal=\"ui/main.toby\" tema=\"aqua\">\n    <vista nombre=\"principal\" archivo=\"ui/main.toby\"/>\n  </ui>\n  <codigo lenguaje=\"rust\" fuente=\"code/src/main.rs\"/>\n  <inicio clase=\"{clase}\" automatico=\"false\" prioridad=\"0\"/>\n  <capacidades><capacidad nombre=\"ventana\"/></capacidades>\n</app>\n"
    );
    fs::write(etapa.join(manifest::FICHERO), manifiesto).map_err(|e| e.to_string())?;
    fs::write(
        etapa.join("ui/main.toby"),
        format!(
            "<toby-app name=\"app-{nombre}\">\n  <window title=\"{titulo}\" width=\"480\" height=\"300\">\n    <stack width=\"480\" height=\"300\">\n      <button id=\"saludo\" name=\"saludo\" variant=\"blue\" title=\"Hola desde Toby AppKit\" x=\"150\" y=\"120\"/>\n    </stack>\n  </window>\n</toby-app>\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        etapa.join("code/src/main.rs"),
        "#![no_std]\n#![no_main]\n\nuse toby_appkit::{app_entry, println};\n\nfn mi_app() {\n    println!(\"Hola desde Toby AppKit\");\n}\n\napp_entry!(mi_app);\n",
    )
    .map_err(|e| e.to_string())?;

    let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "no se pudo resolver el SDK".to_string())?
        .join("kernel/src/libraries/toby_appkit");
    let sdk_relativo = ruta_relativa(&etapa.join("code"), &sdk)?;
    let cargo = format!(
        "[package]\nname = \"{nombre}\"\nversion = \"0.1.0\"\nedition = \"2021\"\nautobins = false\n\n[[bin]]\nname = \"{nombre}\"\npath = \"src/main.rs\"\n\n[dependencies]\ntoby-appkit = {{ path = \"{}\" }}\n\n[workspace]\n\n[profile.release]\npanic = \"abort\"\nopt-level = 2\n",
        sdk_relativo.to_string_lossy().replace('\\', "/")
    );
    fs::write(etapa.join("code/Cargo.toml"), cargo).map_err(|e| e.to_string())?;
    Ok(())
}

fn leer_relativo(raiz: &Path, relativo: &str, max: usize) -> Result<Vec<u8>, String> {
    if Path::new(relativo)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("ruta relativa no segura: {relativo}"));
    }
    let ruta = fs::canonicalize(raiz.join(relativo))
        .map_err(|e| format!("no se encuentra {relativo}: {e}"))?;
    if !ruta.starts_with(raiz) {
        return Err(format!("{relativo} sale del bundle"));
    }
    let meta = fs::metadata(&ruta).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > max as u64 {
        return Err(format!("tamaño o tipo de fichero no válido: {relativo}"));
    }
    fs::read(ruta).map_err(|e| e.to_string())
}

fn ruta_app(ruta: &Path) -> PathBuf {
    if ruta
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("app"))
    {
        ruta.to_path_buf()
    } else {
        let mut salida = ruta.to_path_buf();
        let nombre = ruta.file_name().unwrap_or_default().to_string_lossy();
        salida.set_file_name(format!("{nombre}.app"));
        salida
    }
}

fn nombre_app(ruta: &Path) -> Result<String, String> {
    let carpeta = ruta
        .file_name()
        .ok_or_else(|| "indica una carpeta llamada <nombre>.app".to_string())?
        .to_string_lossy();
    let nombre = carpeta
        .strip_suffix(".app")
        .ok_or_else(|| "la aplicación debe terminar en .app".to_string())?;
    let b = nombre.as_bytes();
    if b.is_empty()
        || b.len() > manifest::MAX_NOMBRE
        || !(b[0].is_ascii_lowercase() || b[0] == b'_')
        || !b[1..]
            .iter()
            .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || *x == b'_' || *x == b'-')
    {
        return Err("usa un nombre [a-z_][a-z0-9_-]* para la carpeta .app".into());
    }
    Ok(nombre.to_string())
}

fn titulo_desde_nombre(nombre: &str) -> String {
    nombre
        .split(['_', '-'])
        .filter(|s| !s.is_empty())
        .map(|parte| {
            let mut chars = parte.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ruta_relativa(origen: &Path, destino: &Path) -> Result<PathBuf, String> {
    let origen = fs::canonicalize(origen).map_err(|e| e.to_string())?;
    let destino = fs::canonicalize(destino).map_err(|e| e.to_string())?;
    let a: Vec<_> = origen.components().collect();
    let b: Vec<_> = destino.components().collect();
    let mut comun = 0;
    while comun < a.len() && comun < b.len() && a[comun] == b[comun] {
        comun += 1;
    }
    if comun == 0 {
        return Err("el bundle y Toby AppKit deben estar en la misma unidad".into());
    }
    let mut relativo = PathBuf::new();
    for _ in comun..a.len() {
        relativo.push("..");
    }
    for parte in &b[comun..] {
        relativo.push(parte.as_os_str());
    }
    Ok(relativo)
}

fn anadir_binario_manifest(ruta: &Path, binario: &str) -> Result<(), String> {
    let mut texto = fs::read_to_string(ruta).map_err(|e| e.to_string())?;
    let inicio = texto
        .find("<codigo ")
        .ok_or_else(|| "falta <codigo> en manifest.xml".to_string())?;
    let fin = inicio
        + texto[inicio..]
            .find("/>")
            .ok_or_else(|| "<codigo> debe ser autocerrado".to_string())?;
    if texto[inicio..fin].contains("binario=") {
        return Ok(());
    }
    texto.insert_str(fin, &format!(" binario=\"{binario}\""));
    fs::write(ruta, texto).map_err(|e| format!("no se pudo actualizar manifest.xml: {e}"))
}
