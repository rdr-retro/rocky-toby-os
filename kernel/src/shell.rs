//! El interprete de ordenes de rocky.
//!
//! Los nombres de las ordenes siguen el estilo de Linux (`ls`, `cd`, `cat`,
//! `mkdir`, `rm`, `cp`, `mv`, `pwd`, ...). El sistema, en cambio, es propio: las
//! rutas usan `/` y el almacenamiento es el sistema de ficheros Estrato sobre un
//! disco ATA.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::asm;
use crate::cmos;
use crate::console;
use crate::fs::{self, Error, Tipo};
use crate::input;
use crate::pit;
use crate::rok;
use crate::sound;
use crate::vm;
use crate::{print, println};

const MAX_LINEA: usize = 256;

/// Fichero testigo que marca que el contenido de ejemplo ya se sembro.
const SEMILLA: &str = "/.semilla";
const ACCESO_DIRECTO_APPS: &str = "/home/escritorio/Apps.tobylink";
const DATOS_ACCESO_DIRECTO_APPS: &[u8] = include_bytes!("../../desktop/Apps.tobylink");

const MELODIA: [(u32, u32); 8] = [
    (262, 150),
    (294, 150),
    (330, 150),
    (349, 150),
    (392, 150),
    (440, 150),
    (494, 150),
    (523, 300),
];

/// Programa de ejemplo que se deja en la carpeta del usuario al formatear.
const EJEMPLO_HOLA: &str = r#"; hola.rka - tu primer programa para rocky.
; Ejecutalo tal cual con:  run hola.rka
    PUSHS "Como te llamas? "
    CALL print
    CALL readline
    STORE nombre
    PUSHS "Hola, "
    LOAD nombre
    ADD
    PUSHS "! Bienvenido a rocky."
    ADD
    CALL println
    PUSHI 0
    CALL exit
"#;

/// Profundidad de anidamiento de guiones (evita recursiones infinitas).
static PROFUNDIDAD: AtomicUsize = AtomicUsize::new(0);

/// Profundidad de anidamiento de programas (evita recursiones infinitas).
static PROGRAMA_PROF: AtomicUsize = AtomicUsize::new(0);

/// Estado de una sesion de consola.
pub struct Sesion {
    /// Registro del directorio de trabajo actual.
    pub cwd: u32,
}

pub fn main_loop() -> ! {
    let disco = crate::disk::init();
    crate::disk_srv::init();
    if !disco {
        println!();
        println!("rocky: no se ha detectado ningun disco.");
        println!("rocky: arrancando en modo reducido (solo ordenes internas).");
    } else if let Err(e) = fs::montar() {
        // Guarda anti-formateo: un volumen que no reconocemos solo se formatea
        // si es el disco de trabajo ATA. El volumen Estrato de un pendrive
        // arrancable no se toca nunca (podria llevar datos de otra persona).
        if e == Error::NoFormateado && !crate::disk::es_pendrive() {
            println!("rocky: volumen nuevo; formateando (Estrato)...");
            if let Err(e) = fs::formatear("ESTRATO") {
                println!("rocky: no se pudo formatear: {}", e.mensaje());
            } else {
                sembrar();
            }
        } else if e == Error::NoFormateado {
            println!("rocky: el volumen del pendrive no tiene formato Estrato; no se toca.");
        } else {
            println!("rocky: no se pudo montar el volumen: {}", e.mensaje());
        }
    }

    // La particion de logs no depende del volumen Estrato: se puede abrir aunque
    // no se haya podido montar el sistema de ficheros (y aun no montando el
    // disco, si no hay MBR, se queda apagada sola).
    if crate::fat32::init() {
        if let Some((n, max)) = crate::fat32::estado() {
            println!(
                "rocky: logs en COMANDOS.LOG (particion FAT32): {} bytes de {} KiB.",
                n,
                max / 1024
            );
        }
        crate::fat32::registrar("--- arranque de rocky ---");
    }

    let mut sesion = Sesion {
        cwd: if fs::montado() { fs::raiz() } else { 0 },
    };

    // Asegura la estructura de carpetas y migra los ficheros de sistema.
    preparar_arbol();

    // Arranca el sistema de servicios (registra los de serie y pone en marcha el
    // reloj, servicio `time`).
    crate::servicios::init();

    // El firmware deja el controlador PS/2 en un estado cualquiera (en esta
    // misma maquina, QEMU lo deja con el reloj del teclado deshabilitado). Se
    // prepara aqui, ya despues de las llamadas al BIOS del disco, para que el
    // interprete tenga teclado (y raton) con el video que sea.
    let entrada = crate::ps2::init();

    // rocky arranca con el servidor **Kira** activo: el compositor toma el
    // framebuffer (escritorio sin wallpaper, barra de menú y cursor) y los clientes de anillo 3 se
    // conectan a Kira por `/dev/kira`. `ESC` devuelve la consola de texto.
    let _ = crate::servicios::arrancar("kira");
    let apps_inicio = crate::inicio_apps::arrancar();

    // El banner se dibuja en la consola de texto, que queda oculta por Kira.
    banner();
    if apps_inicio > 0 {
        println!("  {} app(s) de inicio cargada(s).", apps_inicio);
    }

    entrada.imprimir();
    if fs::montado() {
        let _ = run_script(&mut sesion, "/inicio");
    }

    let mut linea = [0u8; MAX_LINEA];
    let mut prompt_terminal = false;
    loop {
        // Recoge las tareas que hayan terminado (p. ej. un servicio al pararlo).
        crate::tarea::recolectar();
        let terminal = crate::kira::comp::terminal_activa();
        if terminal {
            if !prompt_terminal {
                prompt(&sesion);
                prompt_terminal = true;
            }
        } else {
            prompt_terminal = false;
            if !crate::kira::comp::activo() {
                prompt(&sesion);
            }
        }
        let Some(n) = leer_linea_interactiva(&mut linea) else {
            // Kira acaba de abrir Terminal. Vuelve al principio para dibujar
            // el prompt dentro de la ventana, no en la consola VGA oculta.
            continue;
        };
        // Lo tecleado viene en CP437 (es lo que dibuja la fuente); se pasa a
        // texto para que el resto del interprete trabaje en UTF-8.
        let texto = console::texto_cp437(&linea[..n]);
        if crate::kira::comp::terminal_activa() && matches!(texto.trim(), "exit" | "quit" | "salir")
        {
            crate::kira::comp::cerrar_terminal();
            continue;
        }
        dispatch(&mut sesion, &texto);
    }
}

/// Ejecuta una sesión interactiva de la app Terminal sobre la consola de texto
/// ya inicializada. No vuelve a montar discos ni a arrancar servicios.
///
/// La sesión termina al escribir `exit`, `quit` o `salir`; el anfitrión de la
/// app restaura entonces el escritorio Kira.
pub fn terminal_loop() {
    let mut sesion = Sesion {
        cwd: if fs::montado() { fs::raiz() } else { 0 },
    };
    let mut linea = [0u8; MAX_LINEA];

    console::clear();
    println!("Rocky Terminal - modo texto");
    println!("Escribe 'help' para ver los comandos; 'exit' vuelve al escritorio.");
    println!();

    loop {
        crate::tarea::recolectar();
        prompt(&sesion);
        let n = leer_linea(&mut linea);
        let texto = console::texto_cp437(&linea[..n]);
        if matches!(texto.trim(), "exit" | "quit" | "salir") {
            println!("Volviendo al escritorio...");
            break;
        }
        dispatch(&mut sesion, &texto);
    }
}

/// Escribe el indicador de ordenes: `root@rocky:<ruta> $ `.
pub fn prompt(sesion: &Sesion) {
    let ruta = if fs::montado() {
        fs::ruta_de(sesion.cwd)
    } else {
        String::from("(sin disco)")
    };
    print!("root@rocky:{} $ ", ruta);
}

fn banner() {
    println!();
    println!("  rocky  -  sistema de ficheros Estrato");
    println!("  (un nucleo x86_64 en modo largo, escrito 100% en Rust)");
    println!();
    if fs::montado() {
        println!(
            "  volumen \"{}\": {} KiB, {} bloques libres",
            fs::etiqueta(),
            fs::total_bloques() * fs::BLOQUE as u64 / 1024,
            fs::bloques_libres()
        );
    }
    println!("  escribe 'help' para ver las ordenes.");
    println!();
}

/// Lee una linea con eco y edicion basica (retroceso).
///
/// Si un proceso de anillo 3 tiene **tomada la consola** (esta dentro de un
/// `read` sobre el terminal, [`crate::tty::ocupado`]), el interprete no le roba
/// el teclado: se aparta durmiendo hasta que la suelte. Es la version minima de
/// dar el terminal al proceso en primer plano.
///
/// Con la **pantalla de Kira** puesta (escritorio y cursor) tampoco sondea: el
/// teclado es de Kira y es ella la que atiende el ESC que sale de ese modo.
fn leer_linea(buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    loop {
        if crate::kira::comp::activo() {
            if crate::kira::comp::terminal_activa() {
                if let Some(orden) = crate::kira::comp::terminal_tomar_orden() {
                    let n = orden.len().min(buf.len());
                    buf[..n].copy_from_slice(&orden[..n]);
                    return n;
                }
            }
            crate::tarea::dormir(1);
            continue;
        }
        if crate::tty::ocupado() {
            crate::tarea::dormir(1);
            continue;
        }
        let tecla = match input::poll() {
            Some(k) => k,
            None => {
                crate::tarea::dormir(1);
                continue;
            }
        };
        match tecla {
            b'\n' => {
                println!();
                return len;
            }
            0x08 => {
                if len > 0 {
                    len -= 1;
                    console::put(0x08);
                    console::put(b' ');
                    console::put(0x08);
                }
            }
            0x09 => {}
            c if (0x20..0x7F).contains(&c) || c >= 0x80 => {
                if len < buf.len() {
                    buf[len] = c;
                    len += 1;
                    console::put(c);
                }
            }
            _ => {}
        }
    }
}

/// Lee para el bucle principal y avisa si aparece una Terminal mientras Kira
/// ya estaba mostrando el escritorio. La siguiente iteración dibuja el prompt
/// en esa ventana antes de aceptar la primera orden.
fn leer_linea_interactiva(buf: &mut [u8]) -> Option<usize> {
    let mut len = 0usize;
    let mut terminal_al_entrar = crate::kira::comp::terminal_activa();
    loop {
        if crate::kira::comp::activo() {
            let terminal = crate::kira::comp::terminal_activa();
            if terminal && !terminal_al_entrar {
                return None;
            }
            terminal_al_entrar = terminal;
            if terminal {
                if let Some(orden) = crate::kira::comp::terminal_tomar_orden() {
                    let n = orden.len().min(buf.len());
                    buf[..n].copy_from_slice(&orden[..n]);
                    return Some(n);
                }
            }
            crate::tarea::dormir(1);
            continue;
        }
        terminal_al_entrar = false;
        if crate::tty::ocupado() {
            crate::tarea::dormir(1);
            continue;
        }
        let tecla = match input::poll() {
            Some(k) => k,
            None => {
                crate::tarea::dormir(1);
                continue;
            }
        };
        match tecla {
            b'\n' => {
                println!();
                return Some(len);
            }
            0x08 => {
                if len > 0 {
                    len -= 1;
                    console::put(0x08);
                    console::put(b' ');
                    console::put(0x08);
                }
            }
            0x09 => {}
            c if (0x20..0x7F).contains(&c) || c >= 0x80 => {
                if len < buf.len() {
                    buf[len] = c;
                    len += 1;
                    console::put(c);
                }
            }
            _ => {}
        }
    }
}

/// Crea la estructura y los ficheros del sistema en un volumen recien formateado.
///
/// El arbol definitivo (el espacio del usuario cuelga de `/home`; las apps viven
/// SOLO en `/home/apps`; los recursos del tema, en `/usr/share/tobyui`):
///
/// ```text
/// /
/// |-- bienvenida          (fichero de texto de bienvenida)
/// |-- inicio              (guion que se ejecuta al arrancar)
/// |-- bin/                programas del sistema
/// |-- usr/share/tobyui/   recursos del tema del sistema
/// `-- home/               espacio del usuario (monousuario)
///     |-- apps/           apps <nombre>.app
///     `-- escritorio/     carpeta de trabajo del usuario
/// ```
fn sembrar() {
    let raiz = fs::raiz();
    asegurar_dir(raiz, crate::espacio::USUARIO);
    asegurar_dir(raiz, crate::espacio::ESCRITORIO);
    asegurar_dir(raiz, crate::espacio::APPS);
    asegurar_dir(raiz, "/system");
    asegurar_dir(raiz, crate::espacio::APPS_SISTEMA);
    asegurar_dir(raiz, "/system/config");
    asegurar_dir(raiz, "/usr");
    asegurar_dir(raiz, "/usr/share");
    asegurar_dir(raiz, crate::espacio::RECURSOS);
    asegurar_acceso_directo_apps(raiz);

    if let Ok(reg) = fs::crear(raiz, "/bienvenida", Tipo::Fichero) {
        let texto = "\
Bienvenido a rocky.

Este volumen usa Estrato, el sistema de ficheros propio de rocky.
Aqui los ficheros viven en un disco ATA de verdad: lo que escribas
se conserva al apagar y volver a arrancar.

Tu carpeta de usuario esta en /home. Prueba estas ordenes
(al estilo de Linux):
  ls                ver el contenido del directorio actual
  mkdir docs        crear un directorio
  write nota        crear un fichero (termina con una linea que sea .)
  cat nota          mostrar su contenido
  run hola.rka      ejecuta un programa del lenguaje de rocky
  toby [ruta]       abre una vista .toby o el principal de una carpeta .app
  visor             dibuja una imagen a pantalla completa

Al arrancar, rocky levanta el servidor grafico **Kira** (menu y escritorio sin wallpaper
del raton). ESC devuelve el interprete de ordenes en modo texto; para volver a
ver la pantalla de Kira, usa 'servicio restart kira'.

Escribe 'help' para la lista completa.
";
        let _ = fs::escribir(reg, 0, texto.as_bytes());
    }
    if let Ok(reg) = fs::crear(raiz, "/inicio", Tipo::Fichero) {
        let _ = fs::escribir(
            reg,
            0,
            b"# guion de arranque de rocky\necho Un placer verte por aqui. Escribe 'help'.\n",
        );
    }
    if let Ok(reg) = fs::crear(raiz, "/home/escritorio/hola.rka", Tipo::Fichero) {
        let _ = fs::escribir(reg, 0, EJEMPLO_HOLA.as_bytes());
    }
    // El tema del sistema: la demo de Toby UI tambien vive en el disco, en la
    // carpeta autorizada, de modo que una app se puede montar con
    // `App::desde_fs` (y no solo con el documento incrustado).
    if let Ok(reg) = fs::crear(raiz, crate::espacio::RECURSOS_DEMO, Tipo::Fichero) {
        let _ = fs::escribir(reg, 0, crate::toby_ui::demo::DEMO.as_bytes());
    }
    // Testigo: deja constancia de que el contenido de ejemplo ya se sembro (lo
    // consulta `preparar_arbol` para sembrarlo una sola vez en cada volumen).
    if let Ok(reg) = fs::crear(raiz, SEMILLA, Tipo::Fichero) {
        let _ = fs::escribir(reg, 0, b"rocky\n");
    }
}

/// Asegura la estructura en cada arranque (idempotente) y, si el volumen viene
/// de una version anterior, devuelve los ficheros de sistema de `/rocky` a la
/// raiz y elimina esa carpeta.
fn preparar_arbol() {
    if !fs::montado() {
        return;
    }
    let raiz = fs::raiz();
    // Un volumen creado por el anfitrion (p. ej. la imagen del disco generada al
    // construir) no trae los ficheros de ejemplo: se siembran una sola vez.
    if fs::resolver(raiz, SEMILLA).is_err() {
        sembrar();
    }
    // El espacio del usuario y el tema del sistema se aseguran en cada arranque
    // (por si el volumen viene de una version anterior sin `apps/` o sin
    // `/usr/share/tobyui`).
    asegurar_dir(raiz, crate::espacio::USUARIO);
    asegurar_dir(raiz, crate::espacio::ESCRITORIO);
    asegurar_dir(raiz, crate::espacio::APPS);
    asegurar_dir(raiz, "/system");
    asegurar_dir(raiz, crate::espacio::APPS_SISTEMA);
    asegurar_dir(raiz, "/system/config");
    asegurar_dir(raiz, "/usr");
    asegurar_dir(raiz, "/usr/share");
    asegurar_dir(raiz, crate::espacio::RECURSOS);
    asegurar_acceso_directo_apps(raiz);
    if fs::resolver(raiz, crate::espacio::RECURSOS_DEMO).is_err() {
        if let Ok(reg) = fs::crear(raiz, crate::espacio::RECURSOS_DEMO, Tipo::Fichero) {
            let _ = fs::escribir(reg, 0, crate::toby_ui::demo::DEMO.as_bytes());
        }
    }
    mover(raiz, "/rocky/bienvenida", "/bienvenida");
    mover(raiz, "/rocky/inicio", "/inicio");
    if let Ok(r) = fs::resolver(raiz, "/rocky") {
        if fs::tipo(r) == Tipo::Directorio && fs::es_vacio(r) {
            let _ = fs::borrar(raiz, "/rocky");
        }
    }
}

/// Crea un directorio si no existe todavia.
///
/// `fs::crear` no crea los padres, asi que las rutas anidadas se van asegurando
/// nivel a nivel (p. ej. `/usr`, `/usr/share`, `/usr/share/tobyui`).
fn asegurar_dir(raiz: u32, ruta: &str) {
    if fs::resolver(raiz, ruta).is_err() {
        let _ = fs::crear(raiz, ruta, Tipo::Directorio);
    }
}

/// Instala el acceso directo de ejemplo si el usuario aun no lo tiene.
fn asegurar_acceso_directo_apps(raiz: u32) {
    if fs::resolver(raiz, ACCESO_DIRECTO_APPS).is_err() {
        if let Ok(reg) = fs::crear(raiz, ACCESO_DIRECTO_APPS, Tipo::Fichero) {
            let _ = fs::escribir(reg, 0, DATOS_ACCESO_DIRECTO_APPS);
        }
    }
}

/// Mueve un fichero de `origen` a `destino` (lee, crea, escribe y borra).
fn mover(raiz: u32, origen: &str, destino: &str) {
    if fs::resolver(raiz, destino).is_ok() {
        return;
    }
    let reg = match fs::resolver(raiz, origen) {
        Ok(r) if fs::tipo(r) == Tipo::Fichero => r,
        _ => return,
    };
    let tam = fs::tamano(reg) as usize;
    let mut buf = vec![0u8; tam];
    let n = fs::leer(reg, 0, &mut buf);
    if let Ok(dst) = fs::crear(raiz, destino, Tipo::Fichero) {
        if fs::escribir(dst, 0, &buf[..n]).is_ok() {
            let _ = fs::borrar(raiz, origen);
        }
    }
}

pub fn dispatch(sesion: &mut Sesion, entrada: &str) {
    let linea = entrada.trim();
    if linea.is_empty() || linea.starts_with('#') {
        return;
    }
    crate::fat32::registrar(linea);
    let (orden, arg) = match linea.find(char::is_whitespace) {
        Some(i) => (&linea[..i], linea[i..].trim()),
        None => (linea, ""),
    };

    match orden {
        "help" | "?" | "--help" => help(),
        "uname" => uname(arg),
        "whoami" => println!("root"),
        "clear" => console::clear(),
        "ls" => ls(sesion, arg),
        "tree" => tree(sesion, arg),
        "cat" => cat(sesion, arg),
        "write" => write_file(sesion, arg, false),
        "append" => write_file(sesion, arg, true),
        "rm" => rm(sesion, arg),
        "rmdir" => rmdir(sesion, arg),
        "mkdir" => mkdir(sesion, arg),
        "cp" => cp(sesion, arg),
        "mv" => mv(sesion, arg),
        "ln" => ln(sesion, arg),
        "stat" => stat(sesion, arg),
        "cd" => cd(sesion, arg),
        "pwd" => pwd(sesion),
        "mkfs" => mkfs(sesion, arg),
        "sh" | "source" => {
            let _ = run_script(sesion, arg);
        }
        "run" => run(sesion, arg),
        "ps" | "tareas" => crate::tarea::imprimir(),
        "usuario" | "anillo3" => {
            let id = lanzar_desde_disco(sesion, "/bin/usuario", "usuario");
            if id >= 0 {
                println!("usuario: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "fork" | "hijos" => {
            let id = lanzar_desde_disco(sesion, "/bin/fork", "fork");
            if id >= 0 {
                println!("fork: proceso padre en anillo 3 lanzado (PID {}).", id);
            }
        }
        "exec" => {
            let id = lanzar_desde_disco(sesion, "/bin/exec", "exec");
            if id >= 0 {
                println!("exec: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "fd" | "ficheros" => {
            let id = lanzar_desde_disco(sesion, "/bin/fd", "fd");
            if id >= 0 {
                println!("fd: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "pipe" | "tuberia" | "tuberias" => {
            let id = lanzar_desde_disco(sesion, "/bin/pipe", "pipe");
            if id >= 0 {
                println!("pipe: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "senal" | "senales" => {
            let id = lanzar_desde_disco(sesion, "/bin/senal", "senal");
            if id >= 0 {
                println!("senal: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "tty" | "terminal" => {
            let t = crate::tty::termios();
            println!(
                "tty: consola ICANON={} ECHO={} ISIG={}, grupo en primer plano={}",
                (t.c_lflag & crate::tty::ICANON) != 0,
                (t.c_lflag & crate::tty::ECHO) != 0,
                (t.c_lflag & crate::tty::ISIG) != 0,
                crate::tty::pgrp_frente()
            );
            let id = lanzar_desde_disco(sesion, "/bin/tty", "tty");
            if id >= 0 {
                println!("tty: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "cwd" | "directorio" => {
            let id = lanzar_desde_disco(sesion, "/bin/cwd", "cwd");
            if id >= 0 {
                println!("cwd: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "fs3" | "filesystem" => {
            let id = lanzar_desde_disco(sesion, "/bin/fs3", "fs3");
            if id >= 0 {
                println!("fs3: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "kira" | "kira_test" => {
            let id = lanzar_desde_disco(sesion, "/bin/kira_test", "kira_test");
            if id >= 0 {
                println!("kira_test: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "kill" => kill(arg),
        "memoria" | "mem" => {
            let id = lanzar_desde_disco(sesion, "/bin/memoria", "memoria");
            if id >= 0 {
                println!("memoria: proceso en anillo 3 lanzado (PID {}).", id);
            }
        }
        "fallo" | "excepcion" => {
            let id = lanzar_desde_disco(sesion, "/bin/excepcion", "fallo");
            if id >= 0 {
                println!(
                    "fallo: PID {} lanzado en anillo 3; ejecutara una instruccion invalida (#UD).",
                    id
                );
            }
        }
        "panico" | "panic" => panico(arg),
        "servicio" | "servicios" | "services" => servicio(arg),
        "inicio" | "apps-inicio" => crate::inicio_apps::comando(arg),
        "asm" => asm_cmd(sesion, arg),
        "free" => free(),
        "metricas" | "metrics" | "marea" => crate::metricas::imprimir(),
        "test_plan" | "test-sched" | "test_planificador" => crate::tarea::test_plan_shell(),
        "df" => df(),
        "date" => date(),
        "uptime" => uptime(),
        "echo" => println!("{}", arg),
        "beep" => beep(arg),
        "melody" => melody(),
        "modes" => crate::gfx::print_modes(),
        "lspci" | "pci" => crate::pci::listar(),
        "usb" => crate::drivers::usb::comando(arg),
        "gpu" => crate::drivers::gpu::mostrar(),
        "cursor" => crate::drivers::gpu::cursor(arg),
        "log" | "logs" => mostrar_log(),
        "demo" => demo(arg),
        "gui" | "escritorio" | "desktop" => gui(arg),
        "imagen" | "img" | "view" => {
            if arg.is_empty() {
                println!("Uso: imagen <fichero.png>");
            } else if let Err(e) = crate::visor::ver(arg) {
                println!("imagen: {}", e);
            }
        }
        "gestor-inicio" | "inicio-grafico" => {
            if let Err(e) = crate::toby_ui::host::ver("/system/apps/gestor_inicio.app") {
                println!("gestor-inicio: {}", e);
            }
        }
        "toby" | "app" | "aplicacion" => {
            if let Err(e) = crate::toby_ui::host::ver(arg) {
                println!("toby: {}", e);
            }
        }
        "poweroff" | "halt" | "shutdown" => {
            println!("Apagando rocky...");
            let _ = crate::cpu::shutdown();
        }
        otra => println!("Orden desconocida: {}. Escribe 'help'.", otra),
    }
}

/// Orden `log`: cuenta lo que hay escrito en la particion de logs.
///
/// El fichero (`COMANDOS.LOG`) es de solo escritura desde rocky: el volcado es
/// para leerlo luego en Windows. Aqui solo se ensena el uso.
fn mostrar_log() {
    match crate::fat32::estado() {
        Some((n, max)) => {
            println!(
                "COMANDOS.LOG: {} bytes escritos de {} KiB disponibles ({}%) en la particion FAT32 de logs.",
                n,
                max / 1024,
                (n as u64 * 100 / max as u64)
            );
            println!("  (abre el disco desde Windows y busca COMANDOS.LOG)");
        }
        None => {
            println!("log: este disco no tiene particion de logs (tipo 0x0C).")
        }
    }
}

/// Orden `panico`: provoca fallos controlados para probar la ruta de emergencia.
///
/// Los subcomandos son deliberadamente ruidosos: cualquiera de ellos (salvo
/// `fallo`, que es un proceso de usuario) detiene el sistema con la pantalla
/// azul. Se usan desde las pruebas automaticas y desde un PC real.
fn panico(arg: &str) {
    match arg.split_whitespace().next().unwrap_or("") {
        "kernel" | "rust" => crate::panico::prueba_panico(),
        "ud" | "ud2" => crate::panico::prueba_ud(),
        "gp" => crate::panico::prueba_gp(),
        "pf" => crate::panico::prueba_pf(),
        "df" => crate::panico::prueba_df(),
        "fb" | "grafico" => crate::panico::prueba_framebuffer(),
        _ => {
            println!("Uso: panico <kernel|ud|gp|pf|df|fb>");
            println!("  kernel  provoca un panic! de Rust (ruta del #[panic_handler])");
            println!("  ud      ejecuta una instruccion invalida (excepcion #UD)");
            println!("  gp      accede a una direccion no canonica (excepcion #GP)");
            println!("  pf      accede a una direccion no mapeada (excepcion #PF, con CR2)");
            println!("  df      desborda la pila para provocar un doble fallo (#DF)");
            println!("  fb      pasa a un modo VESA y provoca el panico en framebuffer");
            println!("Tambien: la orden 'fallo' lanza un proceso de anillo 3 que provoca #UD");
            println!("(esa excepcion solo termina el proceso, no detiene el sistema).");
        }
    }
}

/// `kill <pid> <senal>`: envia una senal a un proceso de usuario (Fase 7).
fn kill(arg: &str) {
    let mut it = arg.split_whitespace();
    let pid = match it.next().and_then(|s| s.parse::<u64>().ok()) {
        Some(p) => p,
        None => {
            println!("uso: kill <pid> <senal>");
            println!("senales: 2 SIGINT, 9 SIGKILL, 15 SIGTERM, 18 SIGCONT, 19 SIGSTOP");
            return;
        }
    };
    let senal = match it.next().and_then(parse_senal) {
        Some(s) => s,
        None => {
            println!("kill: falta la senal (o no la conozco)");
            return;
        }
    };
    if crate::tarea::enviar_senal(pid, senal) {
        println!(
            "kill: senal {} ({}) enviada al PID {}.",
            senal,
            crate::senal::nombre(senal),
            pid
        );
    } else {
        println!("kill: no existe el PID {}.", pid);
    }
}

/// Convierte el nombre o el numero de una senal (para la orden `kill`).
fn parse_senal(s: &str) -> Option<u32> {
    match s {
        "SIGINT" | "INT" => Some(crate::senal::SIGINT),
        "SIGKILL" | "KILL" => Some(crate::senal::SIGKILL),
        "SIGTERM" | "TERM" => Some(crate::senal::SIGTERM),
        "SIGCONT" | "CONT" => Some(crate::senal::SIGCONT),
        "SIGSTOP" | "STOP" => Some(crate::senal::SIGSTOP),
        "SIGUSR1" | "USR1" => Some(crate::senal::SIGUSR1),
        "SIGUSR2" | "USR2" => Some(crate::senal::SIGUSR2),
        "SIGALRM" | "ALRM" => Some(crate::senal::SIGALRM),
        "SIGCHLD" | "CHLD" => Some(crate::senal::SIGCHLD),
        otro => otro.parse::<u32>().ok(),
    }
}

fn help() {
    println!();
    println!("Ordenes de rocky (estilo Linux):");
    println!("  help                 esta lista");
    println!("  uname [-a]           informacion del sistema");
    println!("  whoami               usuario actual");
    println!("  clear                limpia la pantalla");
    println!("  ls [-l] [ruta]       contenido de un directorio");
    println!("  tree [ruta]          arbol de directorios");
    println!("  cat <fich>...        muestra ficheros");
    println!("  write <fich>         crea/sobrescribe (acaba con una linea .)");
    println!("  append <fich>        anade al final (acaba con una linea .)");
    println!("  rm <ruta>...         borra ficheros o directorios vacios");
    println!("  rmdir <dir>...       borra directorios vacios");
    println!("  mkdir <dir>...       crea directorios");
    println!("  cp <origen> <dest>   copia un fichero");
    println!("  mv <ruta> <destino>  mueve o renombra (entre directorios)");
    println!("  ln <origen> <dest>   enlace duro: dos nombres, un mismo inodo");
    println!("  stat <ruta>          metadatos: inodo, modo, enlaces y tamano");
    println!("  cd [dir]             cambia de directorio");
    println!("  pwd                  directorio de trabajo");
    println!("  mkfs [etiqueta]      formatea el volumen (borra todo)");
    println!("  sh <guion>           ejecuta un fichero como guion");
    println!("  run <prog> [args]    ejecuta un programa .rok (o un fuente .rka)");
    println!("  ps                   lista las tareas en curso");
    println!("  usuario              lanza el programa de ejemplo en anillo 3");
    println!("  fork                 lanza un proceso que se bifurca y espera al hijo");
    println!("  exec                 lanza un proceso que se reemplaza con /bin/hola (execve)");
    println!(
        "  fd                   lanza un proceso que abre/lee/cierra un fichero (descriptores)"
    );
    println!("  pipe                 lanza un proceso que crea una tuberia y escribe/lee por ella");
    println!("  senal                proceso en anillo 3 que prueba las senales (Fase 7)");
    println!("  kill <pid> <senal>   envia una senal a un proceso (2/9/15/18/19/...)");
    println!("                       nombres: INT|KILL|TERM|CONT|STOP|USR1|USR2|ALRM|CHLD");
    println!("  tty                  proceso en anillo 3 que prueba el terminal (Fase 8):");
    println!("                       ioctl/termios, setsid y lectura canonica de una linea");
    println!("  cwd                  proceso en anillo 3 que prueba el directorio de trabajo");
    println!("                       (Fase 9): chdir, getcwd y open con rutas relativas");
    println!("  fs3                  proceso en anillo 3 que usa el filesystem (Fase 11):");
    println!("                       mkdir, stat, link, rename, getdents64, unlink y rmdir");
    println!("  memoria              lanza un proceso que prueba brk y copia-al-escribir (MV)");
    println!("  fallo                lanza un proceso que provoca una excepcion (#UD) en anillo 3");
    println!("  panico <caso>        provoca un fallo fatal controlado (pantalla azul)");
    println!("                       casos: kernel|ud|gp|pf|df|fb");
    println!("  servicio [accion]    servicios (por defecto, la lista)");
    println!("                       acciones: start|stop|status|restart <nombre>");
    println!("  inicio [accion]      gestiona apps automaticas y prioridad de arranque");
    println!("  gestor-inicio        abre el gestor visual de apps de inicio");
    println!("  asm <fuente> [sal]   ensambla un fuente .rka a un ejecutable .rok");
    println!("  free                 memoria fisica y heap");
    println!("  metricas             contadores del kernel: tarea, syscalls, heap, disco");
    println!("  test_plan            comprueba invariantes del planificador (Fase 1)");
    println!("  df                   informacion del volumen");
    println!("  date                 fecha y hora");
    println!("  uptime               tiempo encendido");
    println!("  echo <texto>         imprime texto");
    println!("  beep [Hz] [ms]       tono con el altavoz interno");
    println!("  melody               melodia de ejemplo");
    println!("  modes                modos VESA disponibles");
    println!("  lspci                dispositivos del bus PCI");
    println!(
        "  usb                  inventario de los controladores de host USB y estado de la pila"
    );
    println!("  gpu                  tarjeta grafica: identificacion, BARs y registros");
    println!("  cursor               cursor por hardware de la GPU ([X Y] | off | info)");
    println!("  log                  uso de la particion FAT32 de logs (COMANDOS.LOG)");
    println!("  demo [WxH]           demo grafica 2D");
    println!("  gui [WxH]            escritorio de Kira a esa resolucion (ESC sale)");
    println!("                       gui info muestra los modos; gui off lo apaga");
    println!("                       gui puntero [1..8] escala el cursor del escritorio");
    println!(
        "                       gui cursor <arrow|hand|text|move|resize-*|busy> prueba estados"
    );
    println!("                       gui prueba pinta una imagen reescalada de ejemplo");
    println!("  imagen <fich.png>    muestra un PNG a pantalla completa (ESC sale)");
    println!("  toby [ruta]         abre un .toby o el principal de un bundle .app (ESC sale)");
    println!("  kira                 lanza el cliente de prueba del servidor Kira");
    println!("  poweroff             apaga la maquina");
    println!();
    println!("rocky arranca con Kira activo (barra de menu y escritorio sin wallpaper;");
    println!("cursor). ESC devuelve la consola de texto; 'servicio restart kira'");
    println!("vuelve a mostrar la pantalla de Kira.");
    println!();
}

fn uname(arg: &str) {
    if arg.split_whitespace().any(|p| p == "-a" || p == "--all") {
        println!("rocky 0.3 x86_64");
        println!("Nucleo monousuario y multitarea, x86_64 en modo largo, 100% Rust.");
        println!("Memoria: gestor de marcos propio + heap con paginacion dinamica.");
        println!("Almacenamiento: disco ATA (PIO) con el sistema de ficheros Estrato.");
        println!("No es un clon de nada: el diseño es propio de rocky.");
    } else {
        println!("rocky");
    }
}

fn sin_disco() -> bool {
    if !fs::montado() {
        println!("(no hay ningun volumen montado)");
        true
    } else {
        false
    }
}

fn ls(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let mut largo = false;
    let mut ruta = ".";
    for parte in arg.split_whitespace() {
        if parte.starts_with('-') {
            if parte.contains('l') {
                largo = true;
            }
        } else {
            ruta = parte;
        }
    }

    let reg = match fs::resolver(sesion.cwd, ruta) {
        Ok(r) => r,
        Err(e) => {
            println!("ls: {}: {}", ruta, e.mensaje());
            return;
        }
    };

    if fs::tipo(reg) == Tipo::Fichero {
        if largo {
            println!("-rw-r--r--  {:>10}  {}", fs::tamano(reg), ruta);
        } else {
            println!("{}", ruta);
        }
        return;
    }

    if largo {
        let mut ficheros = 0;
        let mut dirs = 0;
        let mut bytes = 0u64;
        fs::listar(reg, |e| match e.tipo {
            Tipo::Directorio => {
                dirs += 1;
                println!("drwxr-xr-x  {:>10}  {}/", "-", e.nombre);
            }
            Tipo::Fichero => {
                ficheros += 1;
                bytes += e.tam as u64;
                println!("-rw-r--r--  {:>10}  {}", e.tam, e.nombre);
            }
        });
        println!();
        println!(
            "  {} fichero(s), {} directorio(s), {} bytes",
            ficheros, dirs, bytes
        );
    } else {
        let mut nombres: Vec<String> = Vec::new();
        fs::listar(reg, |e| {
            let mut n = e.nombre.clone();
            if e.tipo == Tipo::Directorio {
                n.push('/');
            }
            nombres.push(n);
        });
        if nombres.is_empty() {
            return;
        }
        let ancho = 20;
        let columnas = 4;
        for (i, n) in nombres.iter().enumerate() {
            print!("{:<width$}", n, width = ancho);
            if (i + 1) % columnas == 0 {
                println!();
            }
        }
        if nombres.len() % columnas != 0 {
            println!();
        }
    }
}

fn cat(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    if arg.is_empty() {
        println!("Uso: cat <fichero>...");
        return;
    }
    for ruta in arg.split_whitespace() {
        match fs::resolver(sesion.cwd, ruta) {
            Ok(reg) => {
                if fs::tipo(reg) == Tipo::Directorio {
                    println!("cat: {}: es un directorio", ruta);
                    continue;
                }
                let tam = fs::tamano(reg) as usize;
                let mut buf = vec![0u8; tam];
                let n = fs::leer(reg, 0, &mut buf);
                // Los ficheros que escribe rocky van en UTF-8; los que no lo
                // sean (por ejemplo, textos en CP437 de antes) salen tal cual.
                match core::str::from_utf8(&buf[..n]) {
                    Ok(s) => print!("{}", s),
                    Err(_) => console::write_bytes(&buf[..n]),
                }
                if n == 0 || buf[n - 1] != b'\n' {
                    println!();
                }
            }
            Err(e) => println!("cat: {}: {}", ruta, e.mensaje()),
        }
    }
}

/// `write`/`append`: lee lineas hasta una que sea exactamente `.`.
fn write_file(sesion: &Sesion, arg: &str, anexar: bool) {
    if sin_disco() {
        return;
    }
    if arg.is_empty() {
        println!("Uso: {} <fichero>", if anexar { "append" } else { "write" });
        return;
    }
    let reg = match fs::resolver(sesion.cwd, arg) {
        Ok(r) => {
            if fs::tipo(r) == Tipo::Directorio {
                println!("{}: es un directorio.", arg);
                return;
            }
            if !anexar {
                fs::truncar(r);
            }
            r
        }
        Err(Error::NoExiste) => match fs::crear(sesion.cwd, arg, Tipo::Fichero) {
            Ok(r) => r,
            Err(e) => {
                println!("{}: {}", arg, e.mensaje());
                return;
            }
        },
        Err(e) => {
            println!("{}: {}", arg, e.mensaje());
            return;
        }
    };

    println!("(escribe el contenido; termina con una linea que sea solo un punto)");
    let mut contenido = String::new();
    let mut linea = [0u8; MAX_LINEA];
    loop {
        print!("| ");
        let n = leer_linea(&mut linea);
        let texto = console::texto_cp437(&linea[..n]);
        if texto == "." {
            break;
        }
        contenido.push_str(&texto);
        contenido.push('\n');
    }

    let offset = if anexar { fs::tamano(reg) as usize } else { 0 };
    match fs::escribir(reg, offset, contenido.as_bytes()) {
        Ok(n) => println!("Escritos {} bytes en {}.", n, fs::ruta_de(reg)),
        Err(e) => println!("No se pudo escribir: {}", e.mensaje()),
    }
}

fn mkdir(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    if arg.is_empty() {
        println!("Uso: mkdir <directorio>...");
        return;
    }
    for d in arg.split_whitespace() {
        if let Err(e) = fs::crear(sesion.cwd, d, Tipo::Directorio) {
            println!("mkdir: {}: {}", d, e.mensaje());
        }
    }
}

fn rm(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    if arg.is_empty() {
        println!("Uso: rm <ruta>...");
        return;
    }
    for ruta in arg.split_whitespace() {
        if let Ok(r) = fs::resolver(sesion.cwd, ruta) {
            if r == sesion.cwd {
                println!("rm: no se puede borrar el directorio de trabajo");
                continue;
            }
        }
        if let Err(e) = fs::borrar(sesion.cwd, ruta) {
            println!("rm: {}: {}", ruta, e.mensaje());
        }
    }
}

fn rmdir(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    if arg.is_empty() {
        println!("Uso: rmdir <directorio>...");
        return;
    }
    for ruta in arg.split_whitespace() {
        match fs::resolver(sesion.cwd, ruta) {
            Ok(r) => {
                if fs::tipo(r) != Tipo::Directorio {
                    println!("rmdir: {}: no es un directorio", ruta);
                    continue;
                }
                if r == sesion.cwd {
                    println!("rmdir: no se puede borrar el directorio de trabajo");
                    continue;
                }
            }
            Err(e) => {
                println!("rmdir: {}: {}", ruta, e.mensaje());
                continue;
            }
        }
        if let Err(e) = fs::borrar(sesion.cwd, ruta) {
            println!("rmdir: {}: {}", ruta, e.mensaje());
        }
    }
}

fn cp(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let partes: Vec<&str> = arg.split_whitespace().collect();
    if partes.len() != 2 {
        println!("Uso: cp <origen> <destino>");
        return;
    }
    let (origen, destino) = (partes[0], partes[1]);

    let src = match fs::resolver(sesion.cwd, origen) {
        Ok(r) if fs::tipo(r) == Tipo::Fichero => r,
        Ok(_) => {
            println!("cp: {}: es un directorio", origen);
            return;
        }
        Err(e) => {
            println!("cp: {}: {}", origen, e.mensaje());
            return;
        }
    };

    // Si el destino es un directorio, copia dentro con el mismo nombre.
    let destino_final = match fs::resolver(sesion.cwd, destino) {
        Ok(d) if fs::tipo(d) == Tipo::Directorio => {
            let base = fs::nombre_de(fs::padre(src), src).unwrap_or_else(|| String::from("copia"));
            if let Err(e) = fs::crear(d, &base, Tipo::Fichero) {
                println!("cp: {}/{}: {}", destino, base, e.mensaje());
                return;
            }
            match fs::resolver(d, &base) {
                Ok(r) => r,
                Err(e) => {
                    println!("cp: {}: {}", destino, e.mensaje());
                    return;
                }
            }
        }
        Ok(_) => {
            println!("cp: {}: ya existe", destino);
            return;
        }
        Err(Error::NoExiste) => match fs::crear(sesion.cwd, destino, Tipo::Fichero) {
            Ok(r) => r,
            Err(e) => {
                println!("cp: {}: {}", destino, e.mensaje());
                return;
            }
        },
        Err(e) => {
            println!("cp: {}: {}", destino, e.mensaje());
            return;
        }
    };

    let tam = fs::tamano(src) as usize;
    let mut buf = vec![0u8; tam];
    let n = fs::leer(src, 0, &mut buf);
    if let Err(e) = fs::escribir(destino_final, 0, &buf[..n]) {
        println!("cp: error al copiar: {}", e.mensaje());
    }
}

fn mv(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let partes: Vec<&str> = arg.split_whitespace().collect();
    if partes.len() != 2 {
        println!("Uso: mv <ruta> <destino>");
        return;
    }
    // `fs::mover` admite rutas con `/`, asi que ahora se puede cambiar de
    // directorio ademas de renombrar (Fase 11).
    if let Err(e) = fs::mover(sesion.cwd, partes[0], partes[1]) {
        println!("mv: {}: {}", partes[0], e.mensaje());
    }
}

/// `ln`: enlace duro, el `sys_link` de Linux 0.01 (Fase 11). Los dos nombres
/// apuntan al **mismo** inodo: el contenido se conserva mientras quede un nombre.
fn ln(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let partes: Vec<&str> = arg.split_whitespace().collect();
    if partes.len() != 2 {
        println!("Uso: ln <origen> <destino>");
        return;
    }
    if let Err(e) = fs::enlazar(sesion.cwd, partes[0], partes[1]) {
        println!("ln: {}: {}", partes[0], e.mensaje());
    }
}

/// `stat`: metadatos de un registro (Fase 11), los mismos campos que devuelve
/// la llamada `stat` del anillo 3.
fn stat(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let ruta = arg.trim();
    if ruta.is_empty() {
        println!("Uso: stat <ruta>");
        return;
    }
    let reg = match fs::resolver(sesion.cwd, ruta) {
        Ok(r) => r,
        Err(e) => {
            println!("stat: {}: {}", ruta, e.mensaje());
            return;
        }
    };
    match fs::tipo(reg) {
        Tipo::Directorio => println!("  tipo:    directorio"),
        Tipo::Fichero => println!("  tipo:    fichero regular"),
    }
    println!("  inodo:   {}", reg);
    println!("  modo:    {:06o}", fs::modo(reg));
    println!("  enlaces: {}", fs::enlaces(reg).max(1));
    println!("  tamano:  {} bytes", fs::tamano(reg));
    println!("  ruta:    {}", fs::ruta_de(reg));
}

fn cd(sesion: &mut Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let destino = arg.trim();
    if destino.is_empty() || destino == "~" || destino == "/" {
        sesion.cwd = fs::raiz();
        crate::tarea::fijar_cwd(sesion.cwd);
        return;
    }
    match fs::resolver(sesion.cwd, destino) {
        Ok(r) if fs::tipo(r) == Tipo::Directorio => {
            sesion.cwd = r;
            // El `cd` del interprete cambia tambien el directorio de la tarea, de
            // modo que los procesos que lance despues lo **hereden** (Fase 9).
            crate::tarea::fijar_cwd(r);
        }
        Ok(_) => println!("cd: {}: no es un directorio", destino),
        Err(e) => println!("cd: {}: {}", destino, e.mensaje()),
    }
}

fn pwd(sesion: &Sesion) {
    if sin_disco() {
        return;
    }
    println!("{}", fs::ruta_de(sesion.cwd));
}

fn tree(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let ruta = if arg.is_empty() { "." } else { arg };
    let reg = match fs::resolver(sesion.cwd, ruta) {
        Ok(r) => r,
        Err(e) => {
            println!("tree: {}: {}", ruta, e.mensaje());
            return;
        }
    };
    println!("{}", fs::ruta_de(reg));
    tree_rec(reg, 1);
}

fn tree_rec(dir: u32, nivel: usize) {
    let mut dirs: Vec<(String, u32)> = Vec::new();
    let mut fich: Vec<(String, u32)> = Vec::new();
    fs::listar(dir, |e| match e.tipo {
        Tipo::Directorio => dirs.push((e.nombre.clone(), e.registro)),
        Tipo::Fichero => fich.push((e.nombre.clone(), e.tam)),
    });
    for (nombre, _) in fich.iter() {
        for _ in 0..nivel {
            print!("    ");
        }
        println!("{}", nombre);
    }
    for (nombre, reg) in dirs.iter() {
        for _ in 0..nivel {
            print!("    ");
        }
        println!("{}/", nombre);
        tree_rec(*reg, nivel + 1);
    }
}

fn mkfs(sesion: &mut Sesion, arg: &str) {
    if !crate::disk::presente() {
        println!("No hay ningun disco que formatear.");
        return;
    }
    let etiqueta = arg
        .split_whitespace()
        .find(|p| !p.starts_with('-'))
        .unwrap_or("ESTRATO");
    println!("Formateando el volumen (se borrara todo)...");
    match fs::formatear(etiqueta) {
        Ok(_) => {
            sembrar();
            sesion.cwd = fs::raiz();
            crate::tarea::fijar_cwd(sesion.cwd);
            println!("Volumen \"{}\" formateado.", etiqueta);
        }
        Err(e) => println!("No se pudo formatear: {}", e.mensaje()),
    }
}

fn free() {
    println!();
    println!("Memoria fisica (marcos de 4 KiB):");
    let total = crate::pmm::total_bytes() / 1024;
    let libre = crate::pmm::free_bytes() / 1024;
    println!("  gestionada: {:>8} KiB", total);
    println!("  libre:      {:>8} KiB", libre);
    println!("  marcos:     {:>8} libres", crate::pmm::free_frame_count());
    println!();
    println!("Heap del kernel (ventana dinamica paginada):");
    println!(
        "  mapeado:    {:>8} KiB",
        crate::heap::mapped_bytes() / 1024
    );
    println!("  libre:      {:>8} KiB", crate::heap::free_bytes() / 1024);
    println!();
    println!("Tuberias anonimas (Fase 6):");
    println!("  abiertas:   {:>8}", crate::pipe::cuantas());
    println!();
}

fn df() {
    if sin_disco() {
        return;
    }
    let total = fs::total_bloques() * fs::BLOQUE as u64;
    let libres = fs::bloques_libres() * fs::BLOQUE as u64;
    println!();
    println!("Fichero     Tipo        Tamano      Libre");
    println!("----------  ----------  ----------  ----------");
    println!(
        "Estrato     {}       {} KiB   {} KiB",
        fs::etiqueta(),
        total / 1024,
        libres / 1024
    );
    println!("Registros libres: {}", fs::registros_libres());
    println!();
}

fn date() {
    let (d, m, a) = cmos::date();
    let (h, mi, s) = cmos::time();
    println!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", a, m, d, h, mi, s);
}

fn uptime() {
    let (h, m, s) = cmos::time();
    println!("{:02}:{:02}:{:02} up {} s", h, m, s, pit::seconds());
}

/// Gestiona los servicios: `servicio [start|stop|status|restart|list] [nombre]`.
///
/// Sin accion (o con `list`) muestra la tabla. El estado se escribe en ingles
/// (`active` / `inactive`).
fn servicio(arg: &str) {
    let mut partes = arg.split_whitespace();
    match partes.next().unwrap_or("") {
        "" | "list" | "lista" | "ls" => listar_servicios(),
        "start" | "arrancar" | "iniciar" => match partes.next() {
            Some(n) => match crate::servicios::arrancar(n) {
                Ok(()) => println!("servicio {}: {}", n, estado_servicio(n)),
                Err(e) => println!("servicio: {}", e),
            },
            None => println!("Uso: servicio start <nombre>"),
        },
        "stop" | "parar" | "detener" => match partes.next() {
            Some(n) => match crate::servicios::parar(n) {
                Ok(()) => println!("servicio {}: {}", n, estado_servicio(n)),
                Err(e) => println!("servicio: {}", e),
            },
            None => println!("Uso: servicio stop <nombre>"),
        },
        "status" | "estado" => match partes.next() {
            Some(n) => {
                if crate::servicios::estado_etiqueta(n).is_some() {
                    println!("servicio {}: {}", n, estado_servicio(n));
                } else {
                    println!("servicio: no existe el servicio '{}'", n);
                }
            }
            None => println!("Uso: servicio status <nombre>"),
        },
        "restart" | "reiniciar" => match partes.next() {
            Some(n) => {
                let _ = crate::servicios::parar(n);
                match crate::servicios::arrancar(n) {
                    Ok(()) => println!("servicio {}: {}", n, estado_servicio(n)),
                    Err(e) => println!("servicio: {}", e),
                }
            }
            None => println!("Uso: servicio restart <nombre>"),
        },
        otra => println!(
            "servicio: accion desconocida '{}'. Prueba 'servicio' para ver la lista.",
            otra
        ),
    }
}

/// Estado de un servicio en ingles, o `?` si no existe.
fn estado_servicio(nombre: &str) -> &'static str {
    crate::servicios::estado_etiqueta(nombre).unwrap_or("?")
}

/// Imprime la tabla de servicios con su estado (`active` / `inactive`).
fn listar_servicios() {
    println!();
    println!("  SERVICIO   ESTADO     DESCRIPCION");
    println!("  ---------  ---------  --------------------------------------------");
    for (nombre, estado, desc) in crate::servicios::listar() {
        println!("  {:<9}  {:<9}  {}", nombre, estado, desc);
    }
    println!();
}

fn beep(arg: &str) {
    let mut partes = arg.split_whitespace();
    let freq: u32 = partes.next().and_then(|s| s.parse().ok()).unwrap_or(880);
    let ms: u32 = partes.next().and_then(|s| s.parse().ok()).unwrap_or(150);
    if !(20..=20_000).contains(&freq) {
        println!("beep: frecuencia fuera de rango (20-20000 Hz).");
        return;
    }
    if ms > 60_000 {
        println!("beep: duracion maxima 60000 ms.");
        return;
    }
    sound::beep(freq, ms);
}

fn melody() {
    println!("Reproduciendo melodia...");
    sound::play(&MELODIA);
    println!("Fin.");
}

fn demo(arg: &str) {
    let mut request = None;
    if let Ok(s) = core::str::from_utf8(arg.as_bytes()) {
        let mut partes = s.split(|c: char| c == 'x' || c == 'X' || c.is_whitespace());
        let a = partes.next();
        let b = partes.next();
        if let (Some(a), Some(b)) = (a, b) {
            if let (Ok(w), Ok(h)) = (a.parse::<u32>(), b.parse::<u32>()) {
                request = Some((w, h));
            }
        }
    }
    if request.is_none() && !arg.trim().is_empty() {
        println!("Sintaxis: demo [ancho x alto]   (p.ej. demo 1024x768)");
        return;
    }
    crate::demo::run(request);
}

/// Orden `gui`: el escritorio de Kira a la resolucion pedida.
///
///   gui                 (re)pon el escritorio, con la resolucion actual
///   gui 1920x1080       el escritorio a esa resolucion, si se puede poner
///   gui info            el modo activo y los modos disponibles
///   gui off             apaga el escritorio y vuelve al modo texto
///   gui puntero [1..8]  escala el cursor del escritorio (filtro del sistema);
///                       sin numero solo dice la escala puesta, y con ella
///                       vuelve a poner el escritorio si estaba oculto
///   gui prueba          pinta una imagen generada, reescalada, de ejemplo
///
/// Detras esta el servicio `kira`: `gui` solo fija la resolucion que quiere el
/// compositor y le pide que vuelva a tomar la pantalla. El modo lo pone primero
/// el driver AMD (fase 14c); si la tarjeta no es de las suyas, se busca el modo
/// VESA y, si tampoco existe, el compositor cae en su cascada (1024x768,
/// 800x600, el primero).
/// La escala del puntero y la resolucion quedan apuntadas como preferencias
/// ([`crate::kira::comp::fijar_escala_puntero`]), asi que sobreviven a que el
/// escritorio se oculte (ESC, `gui off`) y se recupere.
fn gui(arg: &str) {
    let a = arg.trim();
    match a {
        "" => {
            let _ = crate::servicios::arrancar("kira");
            crate::kira::comp::reajustar();
            let (w, h) = pantalla_activa();
            println!("gui: escritorio de Kira activo ({}x{}).", w, h);
        }
        "off" | "stop" | "apagar" => {
            let _ = crate::servicios::parar("kira");
            println!(
                "gui: escritorio apagado, modo texto. ('servicio start kira' lo vuelve a poner)"
            );
        }
        "info" | "modes" | "modos" => {
            if crate::kira::comp::activo() {
                let (w, h) = pantalla_activa();
                crate::pantalla::aviso_serie(&format!("gui: escritorio activo a {}x{}.", w, h));
                println!("gui: escritorio activo a {}x{}.", w, h);
            } else {
                crate::pantalla::aviso_serie("gui: escritorio apagado (modo texto).");
                println!("gui: escritorio apagado (modo texto).");
            }
            if let Some((pw, ph)) = crate::kira::comp::resolucion() {
                crate::pantalla::aviso_serie(&format!("gui: resolucion pedida: {}x{}.", pw, ph));
                println!("gui: resolucion pedida: {}x{}.", pw, ph);
            }
            crate::gfx::print_modes();
        }
        // Escala el cursor del escritorio con el mismo filtro del sistema
        // (ver `cursor::Superposicion::pintar_escalado`).
        s if s.starts_with("puntero") || s.starts_with("raton") || s.starts_with("cursor") => {
            let resto = s
                .trim_start_matches("puntero")
                .trim_start_matches("raton")
                .trim_start_matches("cursor")
                .trim();
            let estado_nombre = resto.strip_prefix("estado ").unwrap_or(resto).trim();
            if let Some(estado) = crate::cursor::Estado::desde_nombre(estado_nombre) {
                crate::kira::comp::fijar_estado_cursor(estado);
                if !crate::kira::comp::activo() {
                    let _ = crate::servicios::arrancar("kira");
                    crate::kira::comp::reajustar();
                } else {
                    crate::kira::comp::repintar_puntero();
                }
                let nombre = match estado {
                    crate::cursor::Estado::Flecha => "arrow",
                    crate::cursor::Estado::Mano => "hand",
                    crate::cursor::Estado::Texto => "ibeam",
                    crate::cursor::Estado::Mover => "move",
                    crate::cursor::Estado::RedimensionarHorizontal => "resize-ew",
                    crate::cursor::Estado::RedimensionarVertical => "resize-ns",
                    crate::cursor::Estado::RedimensionarDiagonal => "resize-nwse",
                    crate::cursor::Estado::Pensando => "spinning",
                };
                let aviso = format!("gui: cursor '{}' activo.", nombre);
                crate::pantalla::aviso_serie(&aviso);
                println!("{}", aviso);
                return;
            }
            let (esc, cambio) = if resto.is_empty() {
                (crate::kira::comp::escala_puntero(), false)
            } else {
                match resto.parse::<i32>() {
                    Ok(n) => (crate::kira::comp::fijar_escala_puntero(n), true),
                    Err(_) => {
                        crate::pantalla::aviso_serie("Sintaxis: gui puntero [1..8]");
                        println!("Sintaxis: gui puntero [1..8]");
                        return;
                    }
                }
            };
            // El cambio no se veria si el escritorio esta oculto, que es el
            // caso normal: para teclear hay que salir de Kira con ESC. Se
            // vuelve a tomar la pantalla igual que hace `gui` a secas, para
            // que la escala nueva se vea al instante. Sin numero es solo una
            // consulta y no toca la pantalla.
            if cambio {
                if !crate::kira::comp::activo() {
                    let _ = crate::servicios::arrancar("kira");
                    crate::kira::comp::reajustar();
                }
                crate::kira::comp::repintar_puntero();
            }
            let aviso = format!(
                "gui: cursor {:?}, escala {}x ({} px).",
                crate::kira::comp::estado_cursor(),
                esc,
                crate::cursor::sprite().ancho * esc
            );
            crate::pantalla::aviso_serie(&aviso);
            println!("{}", aviso);
        }
        // Imagen de prueba: un degradado generado en memoria y escalado con el
        // mismo filtro del sistema, para ver en pantalla como suaviza los bordes.
        "prueba" | "test" | "demo" => {
            gui_prueba_imagen();
        }
        _ => match parsear_resolucion(a) {
            Some((w, h)) => {
                crate::kira::comp::fijar_resolucion(w, h);
                let _ = crate::servicios::arrancar("kira");
                crate::kira::comp::reajustar();
                let (aw, ah) = pantalla_activa();
                if aw < 0 {
                    println!(
                        "gui: no hay modo util para {}x{}; sigue el modo texto.",
                        w, h
                    );
                } else if (aw, ah) == (w as i32, h as i32) {
                    crate::pantalla::aviso_serie(&format!(
                        "gui: escritorio de Kira a {}x{}.",
                        aw, ah
                    ));
                    println!("gui: escritorio de Kira a {}x{}.", aw, ah);
                } else {
                    crate::pantalla::aviso_serie(&format!(
                        "gui: el BIOS no tiene {}x{}; puesto el escritorio a {}x{}.",
                        w, h, aw, ah
                    ));
                    println!(
                        "gui: el BIOS no tiene {}x{}; puesto el escritorio a {}x{}.",
                        w, h, aw, ah
                    );
                }
            }
            None => {
                crate::pantalla::aviso_serie(
                    "Sintaxis: gui [ancho x alto]   (p.ej. gui 1920x1080)",
                );
                println!("Sintaxis: gui [ancho x alto]   (p.ej. gui 1920x1080)");
            }
        },
    }
}

/// Imagen de prueba del escalador de Kira: genera un cuadro con transparencia
/// en memoria y lo pinta en el escritorio reescalado, para ver en pantalla como
/// el filtro (caja al reducir, bilineal al ampliar, alfa premultiplicado) deja
/// los bordes suaves en vez de a escalones. No toca nada del sistema: es solo
/// una demostracion visual del mismo muestreo que usan las pieles de Toby UI.
fn gui_prueba_imagen() {
    // Como el resto de subordenes de `gui`, activa el escritorio antes: si no
    // esta en graficos, se pone (con la resolucion actual pedida) y luego se
    // pinta la imagen de una sola vez. Asi `gui prueba` sirve aun empezando
    // desde el modo texto.
    if !crate::kira::comp::activo() {
        let _ = crate::servicios::arrancar("kira");
        crate::kira::comp::reajustar();
    }
    if !crate::kira::comp::activo() {
        crate::pantalla::aviso_serie(
            "gui: prueba de imagen: no hay escritorio de Kira disponible.",
        );
        println!("gui: prueba de imagen: no hay escritorio de Kira disponible.");
        return;
    }
    // Cuadro fuente pequeno (48x48) con un circulo opaco y el resto
    // transparente, mas una cruz de referencia: al ampliarlo mucho se ve el
    // relleno suave del borde circular.
    const N: i32 = 48;
    let mut px = alloc::vec![0u32; (N * N) as usize];
    let c = N / 2;
    let radio = (N / 2) * (N / 2);
    for y in 0..N {
        for x in 0..N {
            let dx = x - c;
            let dy = y - c;
            let d2 = dx * dx + dy * dy;
            let idx = (y * N + x) as usize;
            if d2 <= radio {
                // Relleno: degradado rojo->verde por bandas de 16 px de la
                // fuente; al ampliar 8x cada banda ocupa 128 px de pantalla y
                // el reborde bilineal funde una con otra. (banda va de 0 a 15.)
                let banda = (d2 * 16 / radio).min(15) as u32;
                px[idx] = 0xFF00_0000 | (banda << 20) | ((15 - banda) << 4);
            } else if x == c || y == c {
                px[idx] = 0xFFFF_FFFF; // cruz blanca opaca (referencia 1:1)
            } else {
                px[idx] = 0; // transparente
            }
        }
    }
    // Ampliar 8x (384x384) arriba a la derecha, lejos del puntero.
    let (dw, dh) = (N * 8, N * 8);
    let x = crate::pantalla::modo()
        .map(|m| m.ancho as i32 - dw - 24)
        .unwrap_or(24);
    let y = 24;
    if crate::kira::comp::pintar_imagen_escalada(&px, N, N, x, y, dw, dh) {
        let aviso = format!(
            "gui: imagen de prueba {}x{} -> {}x{} pintada en ({},{}).",
            N, N, dw, dh, x, y
        );
        crate::pantalla::aviso_serie(&aviso);
        println!("{}", aviso);
    } else {
        crate::pantalla::aviso_serie("gui: la imagen de prueba no se pudo pintar.");
        println!("gui: la imagen de prueba no se pudo pintar.");
    }
}

/// Parsea `1920x1080` (tambien con espacios o mayuscula X).
fn parsear_resolucion(s: &str) -> Option<(u32, u32)> {
    let mut partes = s.split(|c: char| c == 'x' || c == 'X' || c.is_whitespace());
    let a = partes.next()?;
    let b = partes.next()?;
    let w = a.parse::<u32>().ok()?;
    let h = b.parse::<u32>().ok()?;
    if w < 320 || h < 200 || w > 8192 || h > 8192 {
        return None;
    }
    Some((w, h))
}

/// `(ancho, alto)` del modo grafico activo, o `(-1, -1)` si estamos en texto.
fn pantalla_activa() -> (i32, i32) {
    match crate::pantalla::modo() {
        Some(m) => (m.ancho as i32, m.alto as i32),
        None => (-1, -1),
    }
}

/// Ejecuta un fichero de texto como guion, linea a linea.
fn run_script(sesion: &mut Sesion, ruta: &str) -> Result<(), Error> {
    if !fs::montado() {
        return Err(Error::NoExiste);
    }
    let reg = fs::resolver(sesion.cwd, ruta)?;
    if fs::tipo(reg) == Tipo::Directorio {
        return Err(Error::EsDirectorio);
    }
    if PROFUNDIDAD.fetch_add(1, Ordering::Relaxed) > 16 {
        PROFUNDIDAD.fetch_sub(1, Ordering::Relaxed);
        println!("(demasiados guiones anidados)");
        return Ok(());
    }
    let tam = fs::tamano(reg) as usize;
    let mut buf = vec![0u8; tam];
    let n = fs::leer(reg, 0, &mut buf);
    if let Ok(texto) = core::str::from_utf8(&buf[..n]) {
        for linea in texto.lines() {
            dispatch(sesion, linea);
        }
    }
    PROFUNDIDAD.fetch_sub(1, Ordering::Relaxed);
    Ok(())
}

fn lanzar_desde_disco(sesion: &Sesion, ruta: &str, nombre: &str) -> i64 {
    let reg = match fs::resolver(sesion.cwd, ruta) {
        Ok(r) => r,
        Err(e) => {
            println!("{}: {}: {}", nombre, ruta, e.mensaje());
            return -1;
        }
    };
    if fs::tipo(reg) != Tipo::Fichero {
        println!("{}: {}: no es un fichero", nombre, ruta);
        return -1;
    }
    let tam = fs::tamano(reg) as usize;
    let mut datos = vec![0u8; tam];
    let n = fs::leer(reg, 0, &mut datos);
    datos.truncate(n);
    match crate::proceso::lanzar_elf(&datos, nombre) {
        Some(id) => id as i64,
        None => {
            println!("{}: {}: no se pudo cargar el ejecutable", nombre, ruta);
            -1
        }
    }
}

/// Un programa ya cargado en memoria, listo para ejecutarse (en linea o en su
/// propia tarea).
struct Cargado {
    prog: rok::Programa,
    cwd: u32,
    args: Vec<String>,
}

/// Carga y valida un programa `.rok` (o fuente `.rka` a ensamblar al vuelo).
///
/// Los errores se informan aqui, en la Terminal de quien lo lanza, no en la
/// tarea del programa.
fn cargar_programa(sesion: &Sesion, arg: &str) -> Option<Cargado> {
    if sin_disco() {
        return None;
    }
    let partes: Vec<&str> = arg.split_whitespace().collect();
    if partes.is_empty() {
        println!("Uso: run <programa> [argumentos...]");
        return None;
    }
    let ruta = partes[0];
    let args: Vec<String> = partes[1..].iter().map(|s| s.to_string()).collect();

    let reg = match fs::resolver(sesion.cwd, ruta) {
        Ok(r) => r,
        Err(e) => {
            println!("run: {}: {}", ruta, e.mensaje());
            return None;
        }
    };
    if fs::tipo(reg) == Tipo::Directorio {
        println!("run: {}: es un directorio", ruta);
        return None;
    }

    let tam = fs::tamano(reg) as usize;
    let mut datos = vec![0u8; tam];
    let n = fs::leer(reg, 0, &mut datos);
    datos.truncate(n);

    let prog = if rok::Programa::es_rok(&datos) {
        match rok::Programa::cargar(&datos) {
            Ok(p) => p,
            Err(e) => {
                println!("run: {}: {}", ruta, e);
                return None;
            }
        }
    } else {
        let fuente = String::from_utf8_lossy(&datos);
        match asm::ensamblar(&fuente) {
            Ok(p) => p,
            Err(e) => {
                println!("run: {}", e);
                return None;
            }
        }
    };

    Some(Cargado {
        prog,
        cwd: sesion.cwd,
        args,
    })
}

/// `run <programa> [argumentos...]`: carga y ejecuta un programa `.rok`.
///
/// Si el fichero no es un ejecutable `.rok` sino un fuente de texto (`.rka`),
/// se ensambla en memoria y se ejecuta al vuelo. Asi se puede empezar a probar
/// sin compilar a mano. El programa corre en linea (bloquea al interprete hasta
/// que acaba).
fn run(sesion: &mut Sesion, arg: &str) {
    let paquete = match cargar_programa(sesion, arg) {
        Some(p) => p,
        None => return,
    };
    ejecutar_en_linea(sesion, paquete);
}

/// Ejecuta un programa en la tarea que llama (bloquea hasta que acaba).
fn ejecutar_en_linea(sesion: &mut Sesion, paquete: Cargado) {
    if PROGRAMA_PROF.fetch_add(1, Ordering::Relaxed) >= 16 {
        PROGRAMA_PROF.fetch_sub(1, Ordering::Relaxed);
        println!("run: demasiados programas anidados");
        return;
    }
    let mut ejecutar = |cmd: &str| dispatch(sesion, cmd);
    let codigo = vm::ejecutar(&paquete.prog, paquete.cwd, &paquete.args, &mut ejecutar);
    PROGRAMA_PROF.fetch_sub(1, Ordering::Relaxed);

    if codigo != 0 {
        println!("(el programa termino con codigo {})", codigo);
    }
}

/// `asm <fuente> [salida]`: ensambla un fuente de texto a un ejecutable `.rok`.
fn asm_cmd(sesion: &Sesion, arg: &str) {
    if sin_disco() {
        return;
    }
    let partes: Vec<&str> = arg.split_whitespace().collect();
    if partes.is_empty() || partes.len() > 2 {
        println!("Uso: asm <fuente.rka> [salida.rok]");
        return;
    }
    let origen = partes[0];
    let destino = if partes.len() == 2 {
        partes[1].to_string()
    } else {
        cambiar_extension(origen, rok::EXTENSION)
    };

    let reg = match fs::resolver(sesion.cwd, origen) {
        Ok(r) => r,
        Err(e) => {
            println!("asm: {}: {}", origen, e.mensaje());
            return;
        }
    };
    if fs::tipo(reg) != Tipo::Fichero {
        println!("asm: {}: no es un fichero", origen);
        return;
    }

    let tam = fs::tamano(reg) as usize;
    let mut datos = vec![0u8; tam];
    let n = fs::leer(reg, 0, &mut datos);
    datos.truncate(n);
    let fuente = String::from_utf8_lossy(&datos);

    let prog = match asm::ensamblar(&fuente) {
        Ok(p) => p,
        Err(e) => {
            println!("asm: {}", e);
            return;
        }
    };

    let binario = prog.serializar();
    match guardar(sesion, &destino, &binario) {
        Ok(()) => println!(
            "asm: {} -> {} ({} bytes de codigo, {} textos)",
            origen,
            destino,
            prog.codigo.len(),
            prog.textos.len()
        ),
        Err(e) => println!("asm: {}: {}", destino, e.mensaje()),
    }
}

/// Crea o sobrescribe un fichero con unos bytes.
fn guardar(sesion: &Sesion, ruta: &str, datos: &[u8]) -> Result<(), Error> {
    let reg = match fs::resolver(sesion.cwd, ruta) {
        Ok(r) => {
            if fs::tipo(r) != Tipo::Fichero {
                return Err(Error::EsDirectorio);
            }
            fs::truncar(r);
            r
        }
        Err(Error::NoExiste) => fs::crear(sesion.cwd, ruta, Tipo::Fichero)?,
        Err(e) => return Err(e),
    };
    fs::escribir(reg, 0, datos).map(|_| ())
}

/// Cambia la extension de una ruta (solo la parte del nombre del fichero).
fn cambiar_extension(ruta: &str, ext: &str) -> String {
    let barra = ruta.rfind('/').map_or(0, |p| p + 1);
    match ruta.rfind('.') {
        Some(p) if p >= barra => format!("{}.{}", &ruta[..p], ext),
        _ => format!("{}.{}", ruta, ext),
    }
}
