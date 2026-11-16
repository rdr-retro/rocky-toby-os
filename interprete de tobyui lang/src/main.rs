//! Anfitrion de Toby UI para Windows: abre una ventana y pinta un documento
//! `.toby` con el motor REAL del lenguaje (parser, validacion, layout, tema,
//! renderer), el mismo que corre en el kernel rocky.
//!
//! El programa es deliberadamente pequeno:
//!
//!   1. Lee un documento `.toby` de disco (`documentos/ejemplo.toby` por defecto).
//!   2. Registra las acciones nativas que ese documento nombra.
//!   3. Monta la `App`, que valida y dispone el documento.
//!   4. Abre una ventana Win32 **maximizada** (pantalla completa; F11 alterna
//!      con la ventana normal de 480x300) y entrega raton/teclado a
//!      `App::procesar`. El area de cliente manda: el layout se recalcula para
//!      el tamano real de la ventana y el lienzo va detras.
//!   5. Pinta en un lienzo en RAM y lo vuelca a la ventana con `StretchDIBits`.
//!
//! Nada de lo que dice Toby UI se ejecuta como codigo: el documento solo
//! describe que hay y a que accion nativa llama cada boton.

// El crate reproduce la superficie de API que el motor del lenguaje espera
// (Canvas, Font, fs, ...), asi que sobran items aunque no se usen todos.
#![allow(dead_code)]

// El crate reutiliza `alloc` (los ficheros del kernel usan `alloc::...`).
extern crate alloc;

#[path = "host/bios.rs"]
mod bios;
#[path = "host/fs.rs"]
mod fs;
#[path = "host/fuente.rs"]
mod fuente;
#[path = "host/fuente_sf.rs"]
mod fuente_sf;
#[path = "host/gfx.rs"]
mod gfx;
#[path = "host/input.rs"]
mod input;
#[path = "host/mouse.rs"]
mod mouse;
#[path = "host/pit.rs"]
mod pit;
#[path = "host/png_escribir.rs"]
mod png_escribir;
#[path = "host/sync.rs"]
mod sync;
#[path = "host/tarea.rs"]
mod tarea;

// La tabla Unicode↔glifos vive en el nucleo y se comparte tal cual (mismo
// criterio que `toby_ui`: un solo fichero, incluido con `#[path]`).
#[path = "../../kernel/src/texto.rs"]
mod texto;

mod toby_ui;
#[path = "host/win32.rs"]
mod win32;

use alloc::format;
use alloc::string::String;
use core::ffi::c_void;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use toby_ui::action::{Peticion, Registro, Resultado};
use toby_ui::app::App;
use toby_ui::event::{Boton, Evento, Tecla};

use win32::*;

/// Area de la aplicacion por defecto: es la que da el host al arrancar y la
/// que usa el modo `--png`. La `<window width=... height=...>` del documento
/// vale como tamano MINIMO: si el host da mas sitio (pantalla completa) la
/// ventana lo ocupa entero y el layout se recalcula para ese area.
const ANCHO: u32 = 480;
const ALTO: u32 = 300;

/// Estado propio del anfitrion: lo unico que ven los manejadores de acciones.
#[derive(Default)]
struct Acciones {
    /// Cuantas veces se ha pulsado un boton.
    clics: usize,
    /// Ultima accion disparada (clave y `id`), para el resumen.
    ultimo: String,
}

/// `demo.pulsar`: la accion de los botones de color.
fn accion_pulsar(c: &mut Acciones, p: &Peticion) -> Resultado {
    c.clics += 1;
    c.ultimo = format!("{} ({})", p.clave, p.id.unwrap_or("-"));
    println!(
        "  clic #{}  ->  accion='{}'  id='{}'  name='{}'",
        c.clics,
        p.clave,
        p.id.unwrap_or("-"),
        p.nombre.unwrap_or("-")
    );
    Resultado::Hecho
}

/// `window.cerrar`: pide cerrar la ventana.
fn accion_cerrar(_c: &mut Acciones, p: &Peticion) -> Resultado {
    println!(
        "  accion='{}'  name='{}'  ->  cierra la ventana",
        p.clave,
        p.nombre.unwrap_or("-")
    );
    Resultado::Cerrar
}

/// Todo lo que hay que conservar entre mensajes de la ventana.
struct Sesion {
    app: App<Acciones>,
    ctx: Acciones,
    lienzo: gfx::Canvas,
    fuente: bios::Font,
}

/// La sesion viva. El bucle de mensajes es de un solo hilo, pero la ventana
/// reclama el estado desde un callback (`procedimiento`), asi que va en un
/// `Mutex` para no recurrir a `static mut`.
static SESION: OnceLock<Mutex<Sesion>> = OnceLock::new();

/// Coordenadas de raton empaquetadas en el `lParam` de un mensaje.
fn punto(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

/// Ajusta el lienzo y la disposicion al area de cliente real de la ventana.
///
/// La ventana arranca MAXIMIZADA (pantalla completa) y F11 la devuelve a su
/// tamano normal; en los dos casos (y al redimensionar) el motor vuelve a
/// disponer el documento para el area que el host da, que es lo que manda. El
/// lienzo se recrea del mismo tamano que esa area, asi que el volcado a
/// pantalla y las coordenadas del raton siguen siendo 1:1.
fn adaptar_area(hwnd: HWND) {
    let sesion = match SESION.get() {
        Some(s) => s,
        None => return,
    };
    let mut rc: RECT = unsafe { core::mem::zeroed() };
    if unsafe { GetClientRect(hwnd, &mut rc) } == 0 {
        return;
    }
    let ancho = rc.right - rc.left;
    let alto = rc.bottom - rc.top;
    if ancho <= 0 || alto <= 0 {
        return;
    }
    let mut guard = match sesion.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Sesion { app, lienzo, .. } = &mut *guard;
    if lienzo.width == ancho && lienzo.height == alto {
        return;
    }
    app.cambiar_tamano(ancho as u32, alto as u32);
    *lienzo = gfx::Canvas::nuevo(ancho, alto);
}

/// Entrega un evento a la aplicacion y pide repintado si algo cambio.
fn aplicar(hwnd: HWND, ev: Evento) {
    let sesion = match SESION.get() {
        Some(s) => s,
        None => return,
    };
    let mut guard = match sesion.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let (repintar, cerrar) = {
        let Sesion { app, ctx, .. } = &mut *guard;
        let paso = app.procesar(&ev, ctx);
        (paso.repintar(), app.cerrada())
    };
    drop(guard);
    if cerrar {
        unsafe { PostQuitMessage(0) };
    } else if repintar {
        unsafe { InvalidateRect(hwnd, core::ptr::null(), 0) };
    }
}

/// Pinta la aplicacion en el lienzo y lo vuelca al area de cliente.
fn pintar(hwnd: HWND) {
    let sesion = match SESION.get() {
        Some(s) => s,
        None => return,
    };
    let mut guard = match sesion.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    unsafe {
        let mut ps: PAINTSTRUCT = core::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let Sesion {
            app,
            lienzo,
            fuente,
            ..
        } = &mut *guard;
        if let Some(fuente_suave) = fuente_sf::sistema() {
            app.pintar_con_fuente_suave(lienzo, Some(&*fuente), &fuente_suave);
        } else {
            app.pintar(lienzo, Some(&*fuente));
        }

        let mut bmi: BITMAPINFO = core::mem::zeroed();
        bmi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as DWORD;
        bmi.bmiHeader.biWidth = lienzo.width;
        bmi.bmiHeader.biHeight = -lienzo.height; // origen arriba (top-down)
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        StretchDIBits(
            hdc,
            0,
            0,
            lienzo.width,
            lienzo.height,
            0,
            0,
            lienzo.width,
            lienzo.height,
            lienzo.pixeles().as_ptr() as *const c_void,
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        EndPaint(hwnd, &ps);
    }
}

/// Procedimiento de la ventana.
unsafe extern "system" fn procedimiento(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Doble bufer: no dejamos que Windows borre el fondo.
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            pintar(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = punto(lparam);
            aplicar(hwnd, Evento::RatonMueve { x, y, dx: 0, dy: 0 });
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = punto(lparam);
            aplicar(
                hwnd,
                Evento::RatonAbajo {
                    x,
                    y,
                    boton: Boton::Izquierdo,
                },
            );
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = punto(lparam);
            aplicar(
                hwnd,
                Evento::RatonArriba {
                    x,
                    y,
                    boton: Boton::Izquierdo,
                },
            );
            0
        }
        WM_KEYDOWN => {
            match wparam {
                VK_ESCAPE => PostQuitMessage(0),
                // F11 alterna entre pantalla completa (maximizada) y la
                // ventana normal: el motor vuelve a disponer el documento
                // para el area que resulte (`WM_SIZE`).
                VK_F11 => {
                    let maximizada = unsafe { IsZoomed(hwnd) } != 0;
                    unsafe { ShowWindow(hwnd, if maximizada { SW_RESTORE } else { SW_MAXIMIZE }) };
                }
                VK_TAB => aplicar(hwnd, Evento::Tecla(Tecla::Tabulador)),
                _ => {}
            }
            0
        }
        // El area de cliente ha cambiado (arranque, F11, redimensionado):
        // lienzo y layout se van detras del tamano real de la ventana.
        WM_SIZE => {
            adaptar_area(hwnd);
            unsafe { InvalidateRect(hwnd, core::ptr::null(), 0) };
            0
        }
        // El host entrega Unicode; el motor trabaja con bytes CP437 (como el
        // teclado del kernel). Lo que la fuente no dibuja sale como '?'.
        // Tab va por WM_KEYDOWN (el 0x09 de aqui lo doble) y la tecla muerta
        // llega como 0.
        WM_CHAR => {
            if wparam != 0 && wparam != 0x09 {
                let c = char::from_u32(wparam as u32).unwrap_or('?');
                let b = texto::glifo_de(c);
                aplicar(hwnd, Evento::Tecla(Tecla::del_byte(b)));
            }
            0
        }
        WM_CLOSE | WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Crea la ventana (maximizada, con F11 para volver al tamano normal) y
/// bombea mensajes hasta que se cierre.
fn ventana(ancho: u32, alto: u32) -> Result<(), String> {
    unsafe {
        let hinst = GetModuleHandleW(core::ptr::null());
        let clase = win32::ancho("TobyInterpreteVentana");
        let titulo = win32::ancho("Toby UI  -  composicion de botones");

        let mut wc: WNDCLASSW = core::mem::zeroed();
        wc.lpfnWndProc = Some(procedimiento);
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(core::ptr::null_mut(), IDC_ARROW as *const u16);
        wc.lpszClassName = clase.as_ptr();
        if RegisterClassW(&wc) == 0 {
            return Err(String::from("RegisterClassW fallo"));
        }

        // Ventana de tamano fijo: area de cliente = ancho x alto (es el tamano
        // al que vuelve F11; al arrancar se maximiza en `ShowWindow`).
        let estilo = WS_OVERLAPPEDWINDOW & !WS_THICKFRAME & !WS_MAXIMIZEBOX;
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: ancho as i32,
            bottom: alto as i32,
        };
        AdjustWindowRect(&mut rc, estilo, 0);

        // Tamaño exterior real (borde + barra de titulo) y posicion centrada en
        // la pantalla, para que la ventana quede bien colocada al restaurar.
        let ancho_v = rc.right - rc.left;
        let alto_v = rc.bottom - rc.top;
        let x = ((GetSystemMetrics(SM_CXSCREEN) - ancho_v) / 2).max(0);
        let y = ((GetSystemMetrics(SM_CYSCREEN) - alto_v) / 2).max(0);

        let hwnd = CreateWindowExW(
            0,
            clase.as_ptr(),
            titulo.as_ptr(),
            estilo,
            x,
            y,
            ancho_v,
            alto_v,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            hinst,
            core::ptr::null_mut(),
        );
        if hwnd.is_null() {
            return Err(String::from("CreateWindowExW fallo"));
        }

        // La ventana queda arriba y con foco ANTES de maximizar: hacerlo al
        // reves (SetWindowPos con SWP_SHOWWINDOW sobre una ventana ya
        // maximizada) la devuelve a su tamano normal y NO arranca en pantalla
        // completa.
        SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);

        // Arranca en PANTALLA COMPLETA (maximizada): el area de cliente pasa a
        // ser la de la pantalla y el motor dispone el documento para ese sitio.
        // F11 vuelve al tamano declarado (`SW_RESTORE`).
        ShowWindow(hwnd, SW_MAXIMIZE);
        UpdateWindow(hwnd);

        // Autocomprobacion del arranque, con el area real y el estado real de
        // la ventana en el propio proceso (no depende de quien mire fuera).
        let mut rc: RECT = core::mem::zeroed();
        if GetClientRect(hwnd, &mut rc) != 0 {
            println!(
                "pantalla  : {}x{}  maximizada={}  (F11 alterna con la ventana de {}x{})",
                rc.right - rc.left,
                rc.bottom - rc.top,
                if IsZoomed(hwnd) != 0 { "si" } else { "NO" },
                ancho,
                alto
            );
        }

        let mut msg: MSG = core::mem::zeroed();
        while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Ruta del documento por defecto: `documentos/ejemplo.toby`.
fn ruta_por_defecto() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("documentos")
        .join("ejemplo.toby")
        .to_string_lossy()
        .into_owned()
}

/// Vuelca por consola como esta compuesta la escena (indice, componente, `id`,
/// `name` y rectangulo de cada nodo), en orden de pintado.
fn resumen(app: &App<Acciones>) {
    println!("composicion ({} nodos):", app.documento().preorden().len());
    for (i, n) in app.documento().preorden().iter().enumerate() {
        let r = app.disposicion().rect(i);
        println!(
            "  {i:>2}  {:<12} {:<13} {:<13} ({:>3},{:>3})  {}x{}",
            n.def.etiqueta,
            n.id.as_deref().unwrap_or("-"),
            n.nombre().unwrap_or("-"),
            r.x,
            r.y,
            r.w,
            r.h
        );
    }
}

fn main() {
    // Argumentos: [documento.toby] [--png salida.png] [--ancho N --alto M]
    // `--ancho`/`--alto` fijan el area que da el host (por defecto 480x300):
    // en modo `--png` es el tamano del lienzo, y con ventana es el tamano al
    // que se vuelve con F11.
    let args: Vec<String> = std::env::args().collect();
    let mut doc: Option<String> = None;
    let mut png: Option<String> = None;
    let mut ancho: u32 = ANCHO;
    let mut alto: u32 = ALTO;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--png" => {
                i += 1;
                if i < args.len() {
                    png = Some(args[i].clone());
                }
            }
            "--ancho" => {
                i += 1;
                if i < args.len() {
                    ancho = args[i].parse().unwrap_or(ancho).max(1);
                }
            }
            "--alto" => {
                i += 1;
                if i < args.len() {
                    alto = args[i].parse().unwrap_or(alto).max(1);
                }
            }
            _ => doc = Some(args[i].clone()),
        }
        i += 1;
    }
    let ruta = doc.unwrap_or_else(ruta_por_defecto);

    println!("Toby UI  -  interprete de tobyui lang (anfitrion de Windows)");

    // 1) El documento, leido de disco.
    let src = match std::fs::read_to_string(&ruta) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("no se pudo leer '{}': {}", ruta, e);
            return;
        }
    };
    println!("documento : {} ({} bytes)", ruta, src.len());

    // 2) Acciones nativas que el documento puede nombrar.
    let mut registro: Registro<Acciones> = Registro::nuevo();
    registro
        .registrar("demo.pulsar", accion_pulsar)
        .expect("registrar demo.pulsar");
    registro
        .registrar("window.cerrar", accion_cerrar)
        .expect("registrar window.cerrar");

    // 3) Monta la aplicacion (parser -> validacion -> layout).
    let app = match App::nueva(&src, ancho, alto, registro) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("documento invalido: {}", e.resumen());
            return;
        }
    };
    println!("acciones  : {}", app.registro().claves().join(", "));
    println!(
        "area      : {}x{} (area que da el host al arrancar)",
        ancho, alto
    );

    // 4) Fuente y lienzo.
    let fuente = bios::get_font().expect("fuente incrustada");
    let mut lienzo = gfx::Canvas::nuevo(ancho as i32, alto as i32);

    // Modo sin ventana: pinta una vez, guarda un PNG y termina.
    if let Some(salida) = png {
        if let Some(fuente_suave) = fuente_sf::sistema() {
            app.pintar_con_fuente_suave(&mut lienzo, Some(&fuente), &fuente_suave);
        } else {
            app.pintar(&mut lienzo, Some(&fuente));
        }
        match png_escribir::escribir(&salida, ancho, alto, lienzo.pixeles()) {
            Ok(()) => println!("PNG       : {} ({}x{})", salida, ancho, alto),
            Err(e) => eprintln!("no se pudo escribir el PNG: {}", e),
        }
        resumen(&app);
        return;
    }

    println!("cerrar    : boton 'Cerrar' o tecla ESC   |   pantalla completa: F11");

    // 5) Sesion global y ventana.
    SESION
        .set(Mutex::new(Sesion {
            app,
            ctx: Acciones::default(),
            lienzo,
            fuente,
        }))
        .ok();
    if let Err(e) = ventana(ancho, alto) {
        eprintln!("error creando la ventana: {}", e);
    }
}
