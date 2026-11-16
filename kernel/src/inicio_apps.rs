//! Descubrimiento, orden y política de inicio de aplicaciones `.app`.
//!
//! Kira y los controladores gráficos son servicios del sistema y se arrancan
//! antes de este módulo. Después se cargan las apps declaradas como automáticas:
//! primero las de `/system/apps`, luego `/home/apps`, por prioridad descendente.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::fs::{self, Tipo};
use crate::toby_ui::action::{Peticion, Registro, Resultado};
use crate::toby_ui::manifest::{self, ClaseApp, Manifiesto};

const CONFIG: &str = "/system/config/startup.conf";
const MAX_CONFIG: usize = 16 * 1024;
const MAX_APPS_AUTOSTART: usize = 32;

#[derive(Clone, Debug)]
pub struct Aplicacion {
    pub nombre: String,
    pub titulo: String,
    pub clase: ClaseApp,
    pub automatico: bool,
    pub prioridad: i16,
    pub binario: Option<String>,
}

/// Devuelve las apps válidas en el mismo orden en que se cargarían al inicio.
pub fn listar() -> Vec<Aplicacion> {
    let mut apps = Vec::new();
    if !fs::montado() {
        return apps;
    }
    descubrir_raiz(crate::espacio::APPS_SISTEMA, &mut apps);
    descubrir_raiz(crate::espacio::APPS, &mut apps);
    aplicar_config(&mut apps);
    apps.sort_by(|a, b| {
        orden_clase(a.clase)
            .cmp(&orden_clase(b.clase))
            .then_with(|| b.prioridad.cmp(&a.prioridad))
            .then_with(|| a.nombre.cmp(&b.nombre))
    });
    apps
}

fn orden_clase(clase: ClaseApp) -> u8 {
    match clase {
        ClaseApp::Sistema => 0,
        ClaseApp::Usuario => 1,
    }
}

fn descubrir_raiz(ruta: &str, salida: &mut Vec<Aplicacion>) {
    let Ok(dir) = fs::resolver(fs::raiz(), ruta) else {
        return;
    };
    fs::listar(dir, |e| {
        if e.tipo != Tipo::Directorio || !e.nombre.ends_with(".app") {
            return;
        }
        let paquete = format!("{}/{}", ruta, e.nombre);
        let Ok(manifiesto) = crate::toby_ui::paquete::cargar(&paquete) else {
            return;
        };
        salida.push(desde_manifiesto(paquete, manifiesto));
    });
}

fn desde_manifiesto(ruta: String, manifiesto: Manifiesto) -> Aplicacion {
    let binario = manifiesto
        .codigo
        .as_ref()
        .and_then(|codigo| codigo.binario.as_ref())
        .map(|b| format!("{}/{}", ruta, b));
    Aplicacion {
        nombre: manifiesto.nombre,
        titulo: manifiesto.identidad.titulo,
        clase: manifiesto.inicio.clase,
        automatico: manifiesto.inicio.automatico,
        prioridad: manifiesto.inicio.prioridad,
        binario,
    }
}

/// Estado de la interfaz gráfica del gestor de inicio.
pub struct EstadoGestor {
    apps: Vec<Aplicacion>,
}

/// Crea una interfaz Toby UI con una fila accionable por app instalada. Al
/// cambiar una política se guarda en Estrato y la vista se cierra para que al
/// volver a abrirla muestre el nuevo estado.
pub fn interfaz_gestor(ancho: u32, alto: u32) -> (String, EstadoGestor, Registro<EstadoGestor>) {
    let apps = listar();
    let mut xml = format!(
        "<toby-app name=\"gestor_inicio\"><window title=\"Gestor de inicio\" width=\"{}\" height=\"{}\" align=\"stretch\"><column padding=\"16\" spacing=\"8\">",
        ancho.min(720).max(320), alto.min(650).max(300)
    );
    xml.push_str("<button id=\"encabezado\" title=\"Inicio automatico — click para alternar\" variant=\"blue\" enabled=\"false\"/>");
    for (indice, app) in apps.iter().take(8).enumerate() {
        let clase = match app.clase {
            ClaseApp::Sistema => "Sistema",
            ClaseApp::Usuario => "Usuario",
        };
        let activo = if app.automatico { "sí" } else { "no" };
        let variant = if app.automatico { "green" } else { "white" };
        xml.push_str("<row spacing=\"6\">");
        let titulo = escapar_xml(&format!(
            "{} · {} · inicio {} · prioridad {}",
            clase, app.nombre, activo, app.prioridad
        ));
        xml.push_str(&format!(
            "<button id=\"t{}\" title=\"{}\" variant=\"{}\" action=\"inicio.toggle\"/>",
            indice, titulo, variant
        ));
        xml.push_str(&format!(
            "<button id=\"p{}\" title=\"+\" variant=\"white\" width=\"42\" action=\"inicio.mas\"/>",
            indice
        ));
        xml.push_str(&format!(
            "<button id=\"m{}\" title=\"-\" variant=\"white\" width=\"42\" action=\"inicio.menos\"/></row>",
            indice
        ));
    }
    if apps.is_empty() {
        xml.push_str("<button id=\"vacio\" title=\"No hay otras apps instaladas\" variant=\"white\" enabled=\"false\"/>");
    }
    xml.push_str("<button id=\"pie\" title=\"Sistema y gráficos Kira se preparan antes que las apps\" variant=\"white\" enabled=\"false\"/></column></window></toby-app>");

    let mut registro = Registro::nuevo();
    registro
        .registrar("inicio.toggle", alternar_inicio)
        .expect("clave de acción del gestor válida");
    registro
        .registrar("inicio.mas", subir_prioridad)
        .expect("clave de prioridad válida");
    registro
        .registrar("inicio.menos", bajar_prioridad)
        .expect("clave de prioridad válida");
    (xml, EstadoGestor { apps }, registro)
}

fn alternar_inicio(estado: &mut EstadoGestor, peticion: &Peticion<'_>) -> Resultado {
    let Some(id) = peticion.id else {
        return Resultado::Hecho;
    };
    let Some(indice) = id.strip_prefix('t').and_then(|s| s.parse::<usize>().ok()) else {
        return Resultado::Hecho;
    };
    let Some(app) = estado.apps.get(indice) else {
        return Resultado::Hecho;
    };
    let automatico = !app.automatico;
    match fijar(
        &id_calificado(app.clase, &app.nombre),
        automatico,
        app.prioridad,
    ) {
        Ok(()) => Resultado::Cerrar,
        Err(_) => Resultado::Hecho,
    }
}

fn subir_prioridad(estado: &mut EstadoGestor, peticion: &Peticion<'_>) -> Resultado {
    cambiar_prioridad(estado, peticion, 10)
}

fn bajar_prioridad(estado: &mut EstadoGestor, peticion: &Peticion<'_>) -> Resultado {
    cambiar_prioridad(estado, peticion, -10)
}

fn cambiar_prioridad(estado: &mut EstadoGestor, peticion: &Peticion<'_>, delta: i16) -> Resultado {
    let Some(id) = peticion.id else {
        return Resultado::Hecho;
    };
    let Some(indice) = id
        .strip_prefix('p')
        .or_else(|| id.strip_prefix('m'))
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return Resultado::Hecho;
    };
    let Some(app) = estado.apps.get(indice) else {
        return Resultado::Hecho;
    };
    let clase = app.clase;
    let nombre = app.nombre.clone();
    let automatico = app.automatico;
    let prioridad = app.prioridad.saturating_add(delta).clamp(-1000, 1000);
    match fijar(&id_calificado(clase, &nombre), automatico, prioridad) {
        Ok(()) => Resultado::Cerrar,
        Err(_) => Resultado::Hecho,
    }
}

fn id_calificado(clase: ClaseApp, nombre: &str) -> String {
    format!(
        "{}/{}",
        if clase == ClaseApp::Sistema {
            "sistema"
        } else {
            "usuario"
        },
        nombre
    )
}

fn escapar_xml(texto: &str) -> String {
    let mut salida = String::new();
    for c in texto.chars() {
        match c {
            '&' => salida.push_str("&amp;"),
            '"' => salida.push_str("&quot;"),
            '<' => salida.push_str("&lt;"),
            '>' => salida.push_str("&gt;"),
            '\'' => salida.push_str("&apos;"),
            _ => salida.push(c),
        }
    }
    salida
}

fn aplicar_config(apps: &mut [Aplicacion]) {
    let Ok(reg) = fs::resolver(fs::raiz(), CONFIG) else {
        return;
    };
    if fs::tipo(reg) != Tipo::Fichero {
        return;
    }
    let tam = fs::tamano(reg) as usize;
    if tam == 0 || tam > MAX_CONFIG {
        return;
    }
    let mut bytes = alloc::vec![0; tam];
    let n = fs::leer(reg, 0, &mut bytes);
    let Ok(texto) = core::str::from_utf8(&bytes[..n]) else {
        return;
    };
    for linea in texto.lines() {
        let Some((clave, valor)) = linea.split_once('=') else {
            continue;
        };
        let Some((auto, prioridad)) = valor.split_once(',') else {
            continue;
        };
        let Some((clase, nombre)) = clave.split_once('/') else {
            continue;
        };
        let automatico = match auto {
            "1" => true,
            "0" => false,
            _ => continue,
        };
        let Ok(prioridad) = prioridad.parse::<i16>() else {
            continue;
        };
        if !(-1000..=1000).contains(&prioridad) {
            continue;
        }
        let clase = match clase {
            "sistema" => ClaseApp::Sistema,
            "usuario" => ClaseApp::Usuario,
            _ => continue,
        };
        if let Some(app) = apps
            .iter_mut()
            .find(|app| app.clase == clase && app.nombre == nombre)
        {
            app.automatico = automatico;
            app.prioridad = prioridad;
        }
    }
}

/// Persiste la política de una app. Las opciones se guardan separadas del
/// manifiesto para que el gestor no tenga que reescribir metadatos del paquete.
pub fn fijar(
    nombre_calificado: &str,
    automatico: bool,
    prioridad: i16,
) -> Result<(), &'static str> {
    if !(-1000..=1000).contains(&prioridad) {
        return Err("prioridad fuera del rango -1000..1000");
    }
    let (clase, nombre) =
        analizar_id(nombre_calificado).ok_or("usa sistema/<app> o usuario/<app>")?;
    let mut apps = listar();
    let Some(app) = apps
        .iter_mut()
        .find(|app| app.clase == clase && app.nombre == nombre)
    else {
        return Err("no existe una app válida con ese nombre y clase");
    };
    app.automatico = automatico;
    app.prioridad = prioridad;

    let mut contenido = String::new();
    for app in apps {
        let clase = match app.clase {
            ClaseApp::Sistema => "sistema",
            ClaseApp::Usuario => "usuario",
        };
        contenido.push_str(clase);
        contenido.push('/');
        contenido.push_str(&app.nombre);
        contenido.push('=');
        contenido.push(if app.automatico { '1' } else { '0' });
        contenido.push(',');
        contenido.push_str(&app.prioridad.to_string());
        contenido.push('\n');
    }
    guardar_config(contenido.as_bytes())
}

fn analizar_id(id: &str) -> Option<(ClaseApp, &str)> {
    let (clase, nombre) = id.split_once('/')?;
    let clase = match clase {
        "sistema" => ClaseApp::Sistema,
        "usuario" => ClaseApp::Usuario,
        _ => return None,
    };
    let bytes = nombre.as_bytes();
    if bytes.is_empty()
        || bytes.len() > manifest::MAX_NOMBRE
        || !(bytes[0].is_ascii_lowercase() || bytes[0] == b'_')
        || !bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
    {
        return None;
    }
    Some((clase, nombre))
}

fn guardar_config(contenido: &[u8]) -> Result<(), &'static str> {
    let raiz = fs::raiz();
    asegurar_dir(raiz, "/system")?;
    asegurar_dir(raiz, "/system/config")?;
    let reg = match fs::resolver(raiz, CONFIG) {
        Ok(reg) if fs::tipo(reg) == Tipo::Fichero => {
            fs::truncar(reg);
            reg
        }
        Ok(_) => return Err("startup.conf no es un fichero"),
        Err(_) => {
            fs::crear(raiz, CONFIG, Tipo::Fichero).map_err(|_| "no se pudo crear startup.conf")?
        }
    };
    fs::escribir(reg, 0, contenido).map_err(|_| "no se pudo guardar startup.conf")?;
    Ok(())
}

fn asegurar_dir(raiz: u32, ruta: &str) -> Result<(), &'static str> {
    match fs::resolver(raiz, ruta) {
        Ok(reg) if fs::tipo(reg) == Tipo::Directorio => Ok(()),
        Ok(_) => Err("la ruta de configuración no es un directorio"),
        Err(_) => fs::crear(raiz, ruta, Tipo::Directorio)
            .map(|_| ())
            .map_err(|_| "no se pudo crear la carpeta de configuración"),
    }
}

/// Lanza los ELF declarados para inicio automático, respetando el orden de
/// `listar()`. Las apps puramente Toby se cargan cuando el usuario las abre.
pub fn arrancar() -> usize {
    let mut lanzadas = 0;
    for app in listar()
        .into_iter()
        .filter(|app| app.automatico)
        .take(MAX_APPS_AUTOSTART)
    {
        let Some(ruta) = app.binario else {
            continue;
        };
        let Ok(reg) = fs::resolver(fs::raiz(), &ruta) else {
            continue;
        };
        if fs::tipo(reg) != Tipo::Fichero {
            continue;
        }
        let tam = fs::tamano(reg) as usize;
        if tam < 4 || tam > 16 * 1024 * 1024 {
            continue;
        }
        let mut elf = alloc::vec![0; tam];
        let n = fs::leer(reg, 0, &mut elf);
        elf.truncate(n);
        if elf.get(..4) != Some(b"\x7fELF") {
            continue;
        }
        if crate::proceso::lanzar_elf(&elf, &app.nombre).is_some() {
            lanzadas += 1;
        }
    }
    lanzadas
}

/// Consola administrativa del inicio automático.
pub fn comando(argumentos: &str) {
    let mut partes = argumentos.split_whitespace();
    match partes.next().unwrap_or("") {
        "" | "lista" | "list" => {
            crate::println!("\nCLASE    APP                 INICIO  PRIORIDAD  TÍTULO");
            crate::println!(
                "-------- ------------------- ------- ---------- ----------------------"
            );
            for app in listar() {
                let clase = if app.clase == ClaseApp::Sistema {
                    "sistema"
                } else {
                    "usuario"
                };
                crate::println!(
                    "{:<8} {:<19} {:<7} {:<10} {}",
                    clase,
                    app.nombre,
                    if app.automatico { "sí" } else { "no" },
                    app.prioridad,
                    app.titulo
                );
            }
            crate::println!("\nUso: inicio activar|desactivar <sistema|usuario>/<app> [prioridad]");
        }
        accion @ ("activar" | "desactivar") => {
            let Some(id) = partes.next() else {
                crate::println!("Uso: inicio {} <sistema|usuario>/<app> [prioridad]", accion);
                return;
            };
            let prioridad = partes
                .next()
                .and_then(|p| p.parse::<i16>().ok())
                .or_else(|| {
                    listar()
                        .into_iter()
                        .find(|app| {
                            id == format!(
                                "{}/{}",
                                if app.clase == ClaseApp::Sistema {
                                    "sistema"
                                } else {
                                    "usuario"
                                },
                                app.nombre
                            )
                        })
                        .map(|app| app.prioridad)
                })
                .unwrap_or(0);
            match fijar(id, accion == "activar", prioridad) {
                Ok(()) => crate::println!("inicio: política guardada para {}", id),
                Err(e) => crate::println!("inicio: {}", e),
            }
        }
        "prioridad" => {
            let (Some(id), Some(valor)) = (partes.next(), partes.next()) else {
                crate::println!("Uso: inicio prioridad <sistema|usuario>/<app> <entero>");
                return;
            };
            let Some(prioridad) = valor.parse::<i16>().ok() else {
                crate::println!("inicio: la prioridad debe ser un entero");
                return;
            };
            let Some(app) = listar().into_iter().find(|app| {
                id == format!(
                    "{}/{}",
                    if app.clase == ClaseApp::Sistema {
                        "sistema"
                    } else {
                        "usuario"
                    },
                    app.nombre
                )
            }) else {
                crate::println!("inicio: app no encontrada");
                return;
            };
            match fijar(id, app.automatico, prioridad) {
                Ok(()) => crate::println!("inicio: prioridad {} guardada para {}", prioridad, id),
                Err(e) => crate::println!("inicio: {}", e),
            }
        }
        otro => crate::println!(
            "inicio: acción desconocida '{}'; usa 'inicio' para ver opciones",
            otro
        ),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn las_apps_de_sistema_se_ordenan_antes_que_las_de_usuario() {
        assert!(orden_clase(ClaseApp::Sistema) < orden_clase(ClaseApp::Usuario));
    }

    #[test]
    fn ids_del_gestor_no_aceptan_rutas() {
        assert_eq!(id_calificado(ClaseApp::Sistema, "kira"), "sistema/kira");
        assert_eq!(
            id_calificado(ClaseApp::Usuario, "editor_1"),
            "usuario/editor_1"
        );
        assert_eq!(escapar_xml("A&B\""), "A&amp;B&quot;");
    }

    #[test]
    fn la_interfaz_del_gestor_es_un_documento_toby_valido() {
        let (xml, _estado, registro) = interfaz_gestor(640, 480);
        assert_eq!(
            registro.claves(),
            alloc::vec!["inicio.mas", "inicio.menos", "inicio.toggle"]
        );
        crate::toby_ui::app::App::nueva(&xml, 640, 480, registro)
            .expect("el gestor debe montar con o sin apps instaladas");
    }
}
