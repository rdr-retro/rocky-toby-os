//! Compositor de Kira: lo que hay en pantalla con el servidor activo.
//!
//! Cuando el servicio `kira` esta activo, Kira toma el **framebuffer** y dibuja
//! el escritorio de Toby, su wallpaper, barra de menú, ventanas y cursor. El intérprete de órdenes sigue trabajando
//! en su consola **VGA de texto**, que queda tapada mientras esta la pantalla
//! de Kira; se sale de ella pulsando **ESC**, que devuelve el equipo al modo
//! texto (el servicio `kira` sigue activo, solo se oculta su escritorio). Para
//! volver a verlo esta `servicio restart kira`.
//!
//! Cualquier otro subsistema que necesite el video (una aplicacion `.toby`, la
//! demo o el visor de imagenes) lo cede con [`ceder`] antes de poner su modo
//! VESA; cuando vuelve el modo texto y nadie tiene el video, el compositor
//! recupera la pantalla solo, sin que el interprete haga nada.
//!
//! Codigo propio de rocky.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use crate::bios;
use crate::cursor::{self, Superposicion};
use crate::fuentes;
use crate::gfx::{self, Canvas};
use crate::kira::conn::Window;
use crate::sync::SyncCell;
use crate::toby_ui::app::App;
use crate::toby_ui::demo;
use crate::toby_ui::layout::Rect;
use crate::toby_ui::renderer::{self, CanvasRenderer, Renderer};
use crate::{input, mouse, tarea};

/// Azul de las cabeceras de ventana (no es el fondo del escritorio).
const AZUL_CABECERA: u32 = gfx::rgb(34, 62, 168);
/// El escritorio no carga ni dibuja wallpaper; se limpia a negro.
const FONDO_LIMPIO: u32 = gfx::rgb(0, 0, 0);
/// Alto de la barra de menú fija del escritorio.
const ALTO_MENU: i32 = renderer::ALTO_BARRA_VENTANA as i32;
/// Finder reserva una segunda fila AppKit para sus controles de navegación.
const ALTO_CABECERA_FINDER: u32 = 56;
/// Zona interactiva del tirador de tamaño del pie de ventana.
const LADO_TIRADOR_REDIMENSION: i32 = 20;
const TAMANO_MIN_VENTANA: (u32, u32) = (200, 120);
const TAMANO_MIN_FINDER: (u32, u32) = (320, 160);
/// Margen de reserva para que cada movimiento del tirador no realoque ambas
/// superficies al crecer uno o unos pocos píxeles. El tope sigue siendo el
/// área de pantalla, de modo que el margen nunca permite ventanas gigantes.
const MARGEN_RESERVA_REDIMENSION: usize = 16 * 1024;
/// Cuatro segmentos vacíos: el selector es sólo iconográfico.
const SEGMENTOS_VISTA_FINDER: &str = "|||";
const ANCHO_SEGMENTADO_FINDER: u32 = 120;
const ALTO_CABECERA_LISTA_FINDER: i32 = 24;
/// Fila compacta: icono de 16 px y 1 px de margen vertical.
/// La misma métrica gobierna pintura, hit-test y capacidad del scrollbar.
const ALTO_FILA_LISTA_FINDER: i32 = 18;
const COLOR_SELECCION_LISTA_FINDER: u32 = gfx::rgb(55, 125, 232);
const ALTO_FILA_COLUMNA_FINDER: i32 = 28;
const ALTO_FILA_GALERIA_FINDER: i32 = 40;
const ANCHO_COLUMNA_FINDER: i32 = 176;
const ANCHO_VIRTUAL_LISTA_FINDER: u32 = 760;
/// Grosor transversal del carril y del pulgar AppKit del Finder.
const GROSOR_SCROLL_FINDER: u32 = 10;
/// Botón gráfico de Toby en la esquina izquierda de la barra.
const BOTON_MENU_X: i32 = 16;
const BOTON_MENU_Y: i32 = 4;
const BOTON_MENU_ANCHO: i32 = 20;
const BOTON_MENU_ALTO: i32 = 20;
const MENU_TEXTO_X: i32 = BOTON_MENU_X + BOTON_MENU_ANCHO + 8;
const MENU_TEXTO_ALTO: i32 = 16;
const MENU_TEXTO_Y: i32 = (ALTO_MENU - MENU_TEXTO_ALTO) / 2;
const MENU_TOBY_X: i32 = 8;
const MENU_TOBY_Y: i32 = ALTO_MENU;
const MENU_TOBY_ANCHO: u32 = 278;
const MENU_TOBY_ALTO: u32 = 300;
const MENU_ITEM_MARGEN: i32 = 5;
const MENU_ITEM_ALTO: i32 = 19;
const MENU_SEPARADOR_ALTO: i32 = 9;
const FILA_ACERCA_MENU: u8 = 0;
const FILA_SUSPENDER_MENU: u8 = 12;
const FILA_REINICIAR_MENU: u8 = 13;
const FILA_APAGAR_MENU: u8 = 14;
const RUTA_ACERCA: &str = "@system/about-toby-rocky-os";
const ACERCA_DOC: &str = concat!(
    "<toby-app name=\"acerca\"><window title=\"Acerca de Toby &amp; Rocky OS\" ",
    "width=\"560\" height=\"320\" mode=\"window\">",
    "<column width=\"560\" height=\"320\"/></window></toby-app>"
);

#[derive(Clone, Copy)]
enum FilaMenuToby {
    Opcion(&'static str),
    Separador,
}

const FILAS_MENU_TOBY: [FilaMenuToby; 17] = [
    FilaMenuToby::Opcion("Acerca de Toby & Rocky OS"),
    FilaMenuToby::Opcion("Actualización de software…"),
    FilaMenuToby::Opcion("Software de Toby & Rocky OS…"),
    FilaMenuToby::Separador,
    FilaMenuToby::Opcion("Preferencias del sistema…"),
    FilaMenuToby::Opcion("Dock"),
    FilaMenuToby::Opcion("Ubicación"),
    FilaMenuToby::Separador,
    FilaMenuToby::Opcion("Elementos recientes"),
    FilaMenuToby::Separador,
    FilaMenuToby::Opcion("Forzar salida…"),
    FilaMenuToby::Separador,
    FilaMenuToby::Opcion("Suspender"),
    FilaMenuToby::Opcion("Reiniciar…"),
    FilaMenuToby::Opcion("Apagar equipo…"),
    FilaMenuToby::Separador,
    FilaMenuToby::Opcion("Cerrar sesión de Toby & Rocky OS…"),
];

#[derive(Clone, Copy)]
enum AccionEnergia {
    Suspender,
    Reiniciar,
    Apagar,
}
const ICONO_TOBY: &str = "/system/apps/toby_desktop.app/img/toby.png";
const ICONO_TOBY_PULSADO: &str = "/system/apps/toby_desktop.app/img/tobypressed.png";
const FONDO_ESCRITORIO: &str = "/usr/share/backgrounds/flower.png";
/// Limita el consumo de superficies de las apps que comparten el framebuffer.
const MAX_VENTANAS_APP: usize = 8;
/// Duración Genie predeterminada de AppKit convertida al reloj PIT de 100 Hz.
const DURACION_GENIE_TICKS: u64 =
    (toby_appkit::animation::GENIE_DURATION_MS as u64 * 100 + 999) / 1000;
/// Duración del rebote del icono mientras se inicia una app cerrada.
const DURACION_REBOTE_DOCK_TICKS: u64 =
    (toby_appkit::animation::BOUNCE_REPEAT_PERIOD_MS as u64 * 100 + 999) / 1000;
const ALTURA_REBOTE_DOCK_PX: u16 = 18;
/// Refresco de controles animados a ~33 fps; la interfaz no consulta ni
/// recompone superficies entre estos ticks si no hay otros cambios.
const INTERVALO_PROGRESS_TICKS: u64 = 3;

/// Tecla ESC: sale de la pantalla de Kira y vuelve al modo texto.
const ESC: u8 = 0x1B;

/// Ctrl+/− es un atajo del escritorio: se procesa globalmente, aunque una
/// ventana tenga el foco, y no se entrega como texto a la app activa.
fn atajo_zoom_escritorio(tecla: u8, control: bool) -> Option<bool> {
    if !control {
        return None;
    }
    match tecla {
        b'+' | b'*' | b'=' => Some(true),
        b'-' | b'_' => Some(false),
        _ => None,
    }
}

/// Estado del compositor.
struct Comp {
    /// Kira tiene ahora mismo el modo de video puesto.
    activo: bool,
    /// Otro subsistema (TobyUI, la demo, el visor) tiene el video: mientras
    /// este marcado, el compositor no vuelve a tomar la pantalla.
    cedido: bool,
    /// Lienzo sobre el framebuffer (`None` si no hay modo activo).
    canvas: Option<Canvas>,
    /// Buffer trasero persistente: evita mostrar el escritorio a medio pintar.
    backbuffer: Vec<u32>,
    /// Elementos reales de /home/escritorio y su selección/orden.
    escritorio: crate::kira::desktop::Escritorio,
    arrastrando_escritorio: bool,
    arrastre_escritorio_modificado: bool,
    /// Ventanas Toby, ordenadas de atrás hacia delante (la última tiene el foco).
    ventanas_app: Vec<VentanaApp>,
    siguiente_id_ventana: u32,
    /// Último tick en que se avanzó el progreso animado de la ventana visible.
    ultimo_tick_progreso: u64,
    /// Rebote temporal del icono de una app cerrada que está iniciándose.
    rebote_dock: Option<ReboteDock>,
    /// Posicion del cursor del raton.
    mx: i32,
    my: i32,
    /// Botones del ratón del ciclo anterior (bit 0 = izquierdo).
    botones: u8,
    /// El gesto actual empezó dentro de la ventana de la app: no debe
    /// activar elementos del escritorio al salir de ella.
    ventana_captura: bool,
    /// El botón gráfico de Toby está presionado.
    boton_menu_pulsado: bool,
    /// Menú desplegable de Toby abierto.
    menu_toby_abierto: bool,
    menu_toby_acerca: bool,
    /// Pulsación iniciada sobre el botón Toby o una opción de su menú.
    menu_boton_armado: bool,
    menu_item_armado: Option<u8>,
    /// Índice de la app del Dock sobre la que comenzó la pulsación.
    dock_app_armada: Option<DockAppArmada>,
    /// Punto inicial del rectángulo de selección mientras se arrastra.
    seleccion: Option<(i32, i32)>,
    /// Fotograma base durante el marco de selección, para repintar solo las
    /// tiras que cambian en vez de recomponer y mezclar toda el área cada vez.
    marco_base: Vec<u32>,
    /// Estado actual del cursor (flecha, mano, texto, ocupado, etc.).
    cursor_estado: cursor::Estado,
    cursor_frame: usize,
    cursor_escala: i32,
    /// Una app del Dock se está cargando en una tarea aparte para que el
    /// compositor pueda seguir animando el cursor ocupado.
    cargando_app: bool,
    /// Metadatos e iconos de apps de Finder se resuelven fuera del compositor.
    resolviendo_iconos_finder: bool,
    ruta_carga_app: Option<String>,
    cursor_estado_antes_carga: Option<cursor::Estado>,
    /// Puntero dibujado encima, con el trozo de fondo tapado guardado.
    sup: Superposicion,
}

struct VentanaApp {
    id: u32,
    /// Identidad del bundle/documento para enfocar su icono del Dock.
    ruta: String,
    /// Nombre visible procedente de `manifest.xml` (o del documento Toby).
    titulo: String,
    icono: String,
    /// System Preferences usa el mismo render SF Pro antialias que AppKit.
    fuente_sistema: bool,
    /// Algunas ventanas de sistema son fijas y no muestran pie ni tirador.
    permite_redimension: bool,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    pixels: Vec<u32>,
    app: App<demo::Estado>,
    ctx: demo::Estado,
    finder: Option<FinderWindow>,
    terminal: Option<TerminalWindow>,
    textedit: Option<TextEditWindow>,
    /// Ventana informativa del sistema, sin controles ni contenido adicional.
    acerca: bool,
    arrastrando: bool,
    captura_raton: bool,
    ancla_arrastre: (i32, i32),
    /// Geometría y punto de inicio mientras se arrastra el tirador inferior.
    ancla_redimension: Option<(i32, i32, u32, u32)>,
    /// Evita repintar la superficie salvo al entrar o salir de los controles.
    hover_control: bool,
    /// Botón AppKit de la cabecera bajo el puntero (Finder).
    hover_boton_cabecera: Option<usize>,
    /// Control de cabecera pulsado, pendiente de soltar sobre el mismo.
    control_armado: Option<renderer::ControlVentana>,
    /// Botón específico de la app pulsado en la cabecera AppKit.
    boton_cabecera_armado: Option<usize>,
    /// Segmento AppKit de Finder pulsado, pendiente de soltar en el mismo.
    segmento_cabecera_armado: Option<usize>,
    /// Geometría anterior, guardada mientras la ventana está maximizada.
    geometria_normal: Option<(i32, i32, u32, u32)>,
    minimizada: bool,
    genie: Option<AnimacionGenie>,
    /// Fondo capturado para mover la ventana sin recomponer el escritorio.
    /// Se crea solo al empezar a moverla; redimensionar no necesita una copia
    /// completa adicional de la superficie.
    fondo: Vec<u32>,
    /// Evita reintentar una reserva fallida en cada paquete del mismo arrastre.
    fondo_intento: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct DockAppArmada {
    ruta: String,
    ventana_id: Option<u32>,
}

#[derive(Clone, Copy)]
struct AnimacionGenie {
    origen: toby_appkit::genie::Rect,
    destino: toby_appkit::genie::Rect,
    reloj: toby_appkit::animation::Animation,
    ultimo_tick: u64,
    accion: AccionGenie,
}

#[derive(Clone)]
struct ReboteDock {
    ruta: String,
    /// Icono resuelto desde manifest.xml y precargado mientras se abre la app.
    icono: String,
    reloj: toby_appkit::animation::Bounce,
    ultimo_tick: u64,
}

#[derive(Clone, Copy)]
enum AccionGenie {
    Minimizar,
    Restaurar,
}

/// Consola ASCII propia de la Terminal. La entrada y la salida permanecen en
/// Kira; no se cambia al modo VGA de texto.
struct TerminalWindow {
    salida: Vec<u8>,
    entrada: Vec<u8>,
    ordenes: VecDeque<Vec<u8>>,
    sucia: bool,
}

/// Editor sencillo de texto alojado en una ventana de Kira.
struct TextEditWindow {
    /// Bytes CP437 recibidos del teclado; no hay formato ni acciones de disco.
    contenido: Vec<u8>,
    sucia: bool,
}

impl TextEditWindow {
    const MAX_CONTENIDO: usize = 64 * 1024;

    fn nuevo() -> Self {
        let mut contenido = Vec::new();
        let _ = contenido.try_reserve(1024);
        Self {
            contenido,
            sucia: true,
        }
    }

    fn tecla(&mut self, byte: u8) -> bool {
        match byte {
            0x08 => {
                if self.contenido.pop().is_some() {
                    self.sucia = true;
                    true
                } else {
                    false
                }
            }
            b'\r' => false,
            b'\n' | 0x20..=0xFF => {
                if self.contenido.len() >= Self::MAX_CONTENIDO
                    || self.contenido.try_reserve(1).is_err()
                {
                    return false;
                }
                self.contenido.push(byte);
                self.sucia = true;
                true
            }
            _ => false,
        }
    }
}

fn ventana_contiene(ventana: &VentanaApp, x: i32, y: i32) -> bool {
    !ventana.minimizada
        && ventana.genie.is_none()
        && x >= ventana.x
        && x < ventana.x + ventana.w as i32
        && y >= ventana.y
        && y < ventana.y + ventana.h as i32
}

fn sobre_tirador_redimension(ancho: u32, alto: u32, x: i32, y: i32) -> bool {
    let (ancho, alto) = (ancho as i32, alto as i32);
    x >= ancho - LADO_TIRADOR_REDIMENSION
        && y >= alto - LADO_TIRADOR_REDIMENSION
        && x < ancho
        && y < alto
}

fn dimension_redimensionada(inicial: u32, delta: i32, minimo: u32, maximo: u32) -> u32 {
    (inicial as i64 + delta as i64).clamp(minimo as i64, maximo.max(minimo) as i64) as u32
}

/// Índice de la ventana visible más alta bajo el puntero.
fn indice_ventana_bajo(ventanas: &[VentanaApp], x: i32, y: i32) -> Option<usize> {
    ventanas
        .iter()
        .rposition(|ventana| ventana_contiene(ventana, x, y))
}

/// Eleva una ventana a primer plano sin cambiar el orden relativo de las demás.
fn elevar_ventana(ventanas: &mut Vec<VentanaApp>, indice: usize) -> bool {
    if indice + 1 == ventanas.len() {
        return false;
    }
    let ventana = ventanas.remove(indice);
    ventanas.push(ventana);
    true
}

fn aplicaciones_abiertas_dock(
    ventanas: &[VentanaApp],
) -> [Option<crate::dock::AplicacionAbierta<'_>>; MAX_VENTANAS_APP + 1] {
    let mut aplicaciones = [None; MAX_VENTANAS_APP + 1];
    let mut cantidad = 0;
    for (indice, ventana) in ventanas.iter().enumerate() {
        if ventana.acerca || ventana.minimizada || ventana.genie.is_some() {
            continue;
        }
        if ventanas[..indice].iter().any(|anterior| {
            !anterior.acerca
                && !anterior.minimizada
                && anterior.genie.is_none()
                && anterior.ruta == ventana.ruta
        }) {
            continue;
        }
        if cantidad >= MAX_VENTANAS_APP {
            break;
        }
        aplicaciones[cantidad] = Some(crate::dock::AplicacionAbierta {
            ruta: &ventana.ruta,
            icono: &ventana.icono,
            miniatura: None,
        });
        cantidad += 1;
    }
    for ventana in ventanas
        .iter()
        .filter(|ventana| ventana.minimizada || ventana.genie.is_some())
    {
        if cantidad >= MAX_VENTANAS_APP {
            break;
        }
        aplicaciones[cantidad] = Some(crate::dock::AplicacionAbierta {
            ruta: &ventana.ruta,
            icono: &ventana.icono,
            miniatura: Some(crate::dock::VentanaMiniatura {
                id: ventana.id,
                pixeles: &ventana.pixels,
                ancho: ventana.w,
                alto: ventana.h,
                mostrar: ventana.minimizada
                    || matches!(
                        ventana.genie.map(|animacion| animacion.accion),
                        Some(AccionGenie::Restaurar)
                    ),
            }),
        });
        cantidad += 1;
    }
    aplicaciones
}

fn aplicaciones_abiertas_dock_con_carga<'a>(
    ventanas: &'a [VentanaApp],
    carga: Option<&'a ReboteDock>,
) -> [Option<crate::dock::AplicacionAbierta<'a>>; MAX_VENTANAS_APP + 1] {
    let mut aplicaciones = aplicaciones_abiertas_dock(ventanas);
    if let Some(carga) =
        carga.filter(|carga| !ventanas.iter().any(|ventana| ventana.ruta == carga.ruta))
    {
        aplicaciones[MAX_VENTANAS_APP] = Some(crate::dock::AplicacionAbierta {
            ruta: &carga.ruta,
            icono: &carga.icono,
            miniatura: None,
        });
    }
    aplicaciones
}

impl TerminalWindow {
    const MAX_SALIDA: usize = 64 * 1024;
    const MAX_ENTRADA: usize = 255;

    fn nueva() -> Self {
        Self {
            salida: b"Rocky Terminal - Cascadia Code\r\nType help for commands. Type exit to close this window.\r\n\r\n".to_vec(),
            entrada: Vec::new(),
            ordenes: VecDeque::new(),
            sucia: true,
        }
    }

    fn escribir(&mut self, byte: u8) {
        match byte {
            b'\r' => return,
            0x08 => {
                self.salida.pop();
            }
            _ => self.salida.push(byte),
        }
        self.limitar_salida();
        self.sucia = true;
    }

    fn limitar_salida(&mut self) {
        if self.salida.len() > Self::MAX_SALIDA {
            let exceso = self.salida.len() - Self::MAX_SALIDA;
            let recorte = self.salida[exceso..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(exceso, |n| exceso + n + 1);
            self.salida.drain(..recorte);
        }
    }

    fn tecla(&mut self, byte: u8) -> bool {
        match byte {
            b'\n' => {
                self.salida.extend_from_slice(&self.entrada);
                self.salida.push(b'\n');
                self.limitar_salida();
                self.ordenes.push_back(core::mem::take(&mut self.entrada));
            }
            0x08 => {
                self.entrada.pop();
            }
            0x03 => {
                self.salida.extend_from_slice(b"^C\r\n");
                self.entrada.clear();
            }
            0x20..=0x7E | 0x80..=0xFF if self.entrada.len() < Self::MAX_ENTRADA => {
                self.entrada.push(byte);
            }
            _ => return false,
        }
        self.sucia = true;
        true
    }
}

/// Estado propio de Finder. La lista y el pintor de iconos son el mismo
/// `Escritorio` compartido que usa Toby Desktop; aquí sólo cambia la ruta y
/// el área/página donde se presenta.
#[derive(Clone, Copy)]
enum ObjetivoScrollFinder {
    ListaVertical,
    ListaHorizontal,
    ColumnaVertical(usize),
    ColumnasHorizontal,
}

#[derive(Clone, Copy)]
struct ArrastreScrollFinder {
    objetivo: ObjetivoScrollFinder,
    agarre: i32,
}

struct GeometriaScrollFinder {
    pista: Rect,
    pulgar: Rect,
    desplazamiento_maximo: usize,
    desplazamiento: usize,
    paso: usize,
    horizontal: bool,
}
struct FinderWindow {
    archivos: crate::kira::desktop::Escritorio,
    cargando: bool,
    /// En vista de columnas, cada entrada contiene el directorio del nivel
    /// anterior. La primera columna sigue siendo `archivos`.
    columnas: Vec<crate::kira::desktop::Escritorio>,
    /// Primer elemento visible de cada columna secundaria.
    inicios_columnas: Vec<usize>,
    historial: Vec<String>,
    historial_siguiente: Vec<String>,
    /// Página de iconos; en lista, galería y columna se usa `inicio_elementos`
    /// para que la barra pueda desplazarse fila a fila.
    pagina: usize,
    inicio_elementos: usize,
    desplazamiento_x_lista: usize,
    columna_inicio: usize,
    arrastre_scroll: Option<ArrastreScrollFinder>,
    /// Índice único seleccionado en el segmented de vistas (0..4).
    vista: usize,
    ultimo_click: Option<(usize, u64)>,
    aplicacion_pendiente: Option<String>,
    rectangulo: Option<(i32, i32, i32, i32)>,
    marco_base: Vec<u32>,
}

impl FinderWindow {
    fn nuevo() -> Self {
        Self {
            archivos: crate::kira::desktop::Escritorio::navegador("/"),
            cargando: true,
            columnas: Vec::new(),
            inicios_columnas: Vec::new(),
            historial: Vec::new(),
            historial_siguiente: Vec::new(),
            pagina: 0,
            inicio_elementos: 0,
            desplazamiento_x_lista: 0,
            columna_inicio: 0,
            arrastre_scroll: None,
            vista: 0,
            ultimo_click: None,
            aplicacion_pendiente: None,
            rectangulo: None,
            marco_base: Vec::new(),
        }
    }
}

/// Fondo ya ajustado al modo activo. El costoso filtrado del PNG ocurre una
/// vez al cambiar la resolución; los repintados del escritorio solo blitean
/// este buffer.
struct FondoCache {
    ancho: i32,
    alto: i32,
    pixeles: Vec<u32>,
}

static FONDO_CACHE: SyncCell<FondoCache> = SyncCell::new(FondoCache {
    ancho: 0,
    alto: 0,
    pixeles: Vec::new(),
});

static COMP: SyncCell<Comp> = SyncCell::new(Comp {
    activo: false,
    cedido: false,
    canvas: None,
    backbuffer: Vec::new(),
    escritorio: crate::kira::desktop::Escritorio::nuevo(),
    arrastrando_escritorio: false,
    arrastre_escritorio_modificado: false,
    ventanas_app: Vec::new(),
    siguiente_id_ventana: 1,
    ultimo_tick_progreso: 0,
    rebote_dock: None,
    // Posicion por defecto: arriba a la izquierda, separada del borde. Con el
    // video apagado no se pinta, pero asi no arranca en la esquina exacta.
    mx: cursor::MARGEN_INICIAL,
    my: cursor::MARGEN_INICIAL,
    botones: 0,
    ventana_captura: false,
    boton_menu_pulsado: false,
    menu_toby_abierto: false,
    menu_toby_acerca: false,
    menu_boton_armado: false,
    menu_item_armado: None,
    dock_app_armada: None,
    seleccion: None,
    marco_base: Vec::new(),
    cursor_estado: cursor::Estado::Flecha,
    cursor_frame: usize::MAX,
    cursor_escala: 0,
    cargando_app: false,
    resolviendo_iconos_finder: false,
    ruta_carga_app: None,
    cursor_estado_antes_carga: None,
    sup: Superposicion::new(),
});

/// Resolucion pedida para el escritorio (`gui <ancho>x<alto>`), si la hay.
///
/// `None` = la resolucion por defecto del compositor. Es un `static` y no un
/// campo de `Comp` para poder fijarla desde el interprete sin tocar el heap.
static RESOLUCION: SyncCell<Option<(u32, u32)>> = SyncCell::new(None);

/// Escala del puntero del raton, elegida con `gui puntero` (1 = tamano natural).
///
/// Es un `static` como [`RESOLUCION`] para que sea una **preferencia** y no un
/// estado del escritorio: asi sobrevive a los `ocultar` (la salida con ESC, el
/// `ceder` de otro subsistema y el `reajustar` de `gui`). Si viviera en `Comp`
/// se pondria a 1 cada vez que el interprete toma el teclado, que es justo
/// cuando se escribe `gui puntero`, y el cambio no se veria nunca.
static ESCALA: SyncCell<i32> = SyncCell::new(1);

fn comp() -> &'static mut Comp {
    // Seguro: cada acceso va dentro de una seccion critica de tareas
    // (`tarea::critica`), que impide que el planificador cambie de tarea a
    // mitad de un repintado.
    unsafe { &mut *COMP.get() }
}

/// Escala del puntero, leida sin entrar en una seccion critica nueva.
///
/// Solo se usa desde codigo que ya va dentro de una (las mismas zonas que usan
/// [`comp`]), donde leer la escala es seguro por el mismo motivo.
fn escala_actual() -> i32 {
    unsafe { *ESCALA.get() }
}

/// Pide la resolucion del escritorio de Kira.
///
/// No la pone al instante: el proximo [`reajustar`] (o el despertar de la
/// tarea del compositor) es cuando el modo cambia, para no pisar un video que
/// este siendo usado por otro subsistema.
pub fn fijar_resolucion(ancho: u32, alto: u32) {
    unsafe { *RESOLUCION.get() = Some((ancho, alto)) };
}

/// Resolucion pedida con [`fijar_resolucion`], si la hay.
pub fn resolucion() -> Option<(u32, u32)> {
    unsafe { *RESOLUCION.get() }
}

/// Cambia el modo de video de Kira y vuelve a tomar la pantalla (lo pide la
/// orden `gui`).
///
/// Aparta el escritorio (modo texto) y vuelve a poner el modo aqui mismo, sin
/// esperar a la tarea del compositor, para que la orden `gui` pueda decirle al
/// usuario que resolucion ha quedado puesta de verdad. Si el modo pedido no
/// existe, cae en la cascada de siempre (1024x768, 800x600, el primero).
pub fn reajustar() {
    tarea::critica(|| {
        let k = comp();
        ocultar(k);
        k.cedido = false;
    });
    // `reanudar` lee del disco al refrescar el escritorio: fuera de la seccion
    // critica para que el planificador pueda seguir cambiando de tarea.
    reanudar(comp());
}

/// `(ancho, alto)` del modo grafico activo del escritorio, si lo hay.
pub fn tamano() -> Option<(i32, i32)> {
    tarea::critica(|| comp().canvas.as_ref().map(|c| (c.width, c.height)))
}

/// Posición actual del puntero del escritorio.
pub fn posicion_puntero() -> Option<(i32, i32)> {
    tarea::critica(|| {
        let k = comp();
        k.activo.then_some((k.mx, k.my))
    })
}

/// Añade una superficie Toby sobre el escritorio sin cambiar el modo de vídeo.
pub fn abrir_ventana_app(
    app: App<demo::Estado>,
    ctx: demo::Estado,
    pixels: Vec<u32>,
    w: u32,
    h: u32,
    ruta: String,
    titulo: String,
    icono: String,
) -> Option<(i32, i32)> {
    abrir_ventana_app_interno(app, ctx, pixels, w, h, false, false, ruta, titulo, icono)
}

/// Abre Toby Finder sobre la misma superficie de ventana de Kira y conecta
/// su contenido al componente de archivos compartido con el escritorio.
pub fn abrir_ventana_finder(
    app: App<demo::Estado>,
    ctx: demo::Estado,
    pixels: Vec<u32>,
    w: u32,
    h: u32,
    ruta: String,
    titulo: String,
    icono: String,
) -> Option<(i32, i32)> {
    abrir_ventana_app_interno(app, ctx, pixels, w, h, true, false, ruta, titulo, icono)
}

/// Carga el directorio inicial después de que la ventana vacía ya se haya
/// mostrado. El escaneo se hace SIN `tarea::critica` para que el ratón,
/// el compositor y el resto del sistema sigan respondiendo mientras se lee
/// el directorio.
pub fn cargar_finder_inicial(ruta: &str, ancho: u32, alto: u32) {
    let mut archivos = crate::kira::desktop::Escritorio::navegador("/");
    // Sin critica: permite preempción durante el escaneo completo
    archivos.refrescar(ancho as i32, alto as i32);
    // El repintado final se hace junto al intercambio del estado de la ventana.
    // Adelanta aquí sus recursos para que no haya lecturas ni decodificación
    // PNG dentro de esa sección crítica.
    crate::toby_ui::kit::precalentar_finder();
    archivos.precalentar_iconos();

    tarea::critica(|| {
        let k = comp();
        let Some(indice) = k.ventanas_app.iter().rposition(|ventana| {
            ventana.ruta == ruta
                && ventana
                    .finder
                    .as_ref()
                    .is_some_and(|finder| finder.cargando)
        }) else {
            return;
        };
        if let Some(finder) = k.ventanas_app[indice].finder.as_mut() {
            finder.archivos = archivos;
            finder.cargando = false;
        }
        pintar_superficie_ventana(&mut k.ventanas_app[indice]);
        repintar_completo(k);
    });
}

/// Abre la Terminal como ventana gráfica con entrada y salida ASCII.
pub fn abrir_ventana_terminal(
    app: App<demo::Estado>,
    ctx: demo::Estado,
    pixels: Vec<u32>,
    w: u32,
    h: u32,
    ruta: String,
    titulo: String,
    icono: String,
) -> Option<(i32, i32)> {
    abrir_ventana_app_interno(app, ctx, pixels, w, h, false, true, ruta, titulo, icono)
}

/// Abre la ventana «Acerca de» usando el ciclo normal de ventanas de Kira.
/// Se crea y pinta fuera de la sección crítica del compositor.
fn abrir_ventana_acerca() {
    const ANCHO: u32 = 560;
    const ALTO: u32 = 320;
    let ya_abierta = tarea::critica(|| {
        let k = comp();
        let Some(indice) = k.ventanas_app.iter().position(|ventana| ventana.acerca) else {
            return false;
        };
        let restaurada = k.ventanas_app[indice].minimizada;
        k.ventanas_app[indice].minimizada = false;
        k.ventanas_app[indice].genie = None;
        let elevada = elevar_ventana(&mut k.ventanas_app, indice);
        if restaurada || elevada {
            repintar_completo(k);
        }
        true
    });
    if ya_abierta {
        return;
    }
    let Some(cantidad) = (ANCHO as usize).checked_mul(ALTO as usize) else {
        return;
    };
    let mut pixels = Vec::new();
    if pixels.try_reserve_exact(cantidad).is_err() {
        return;
    }
    pixels.resize(
        cantidad,
        crate::toby_ui::theme::TEMA_POR_DEFECTO.paleta.fondo,
    );
    let Ok(app) = App::nueva(ACERCA_DOC, ANCHO, ALTO, demo::registro()) else {
        return;
    };
    let _ = abrir_ventana_app_interno(
        app,
        demo::Estado::default(),
        pixels,
        ANCHO,
        ALTO,
        false,
        false,
        String::from(RUTA_ACERCA),
        String::from("Acerca de Toby & Rocky OS"),
        String::from(ICONO_TOBY),
    );
}

fn abrir_ventana_app_interno(
    app: App<demo::Estado>,
    ctx: demo::Estado,
    pixels: Vec<u32>,
    w: u32,
    h: u32,
    es_finder: bool,
    es_terminal: bool,
    ruta: String,
    titulo: String,
    icono: String,
) -> Option<(i32, i32)> {
    let len = (w as usize).checked_mul(h as usize)?;
    if w == 0 || h == 0 || pixels.len() < len {
        return None;
    }
    // Valida primero con una sección breve. El pintado de AppKit puede leer y
    // decodificar varios recursos la primera vez; no debe retener el estado
    // global del compositor ni impedir que el planificador mantenga viva Kira.
    let puede_abrir = tarea::critica(|| {
        let k = comp();
        k.canvas.as_ref().is_some_and(|c| {
            k.activo
                && w as i32 <= c.width
                && h as i32 <= c.height
                && k.ventanas_app.len() < MAX_VENTANAS_APP
        })
    });
    if !puede_abrir {
        return None;
    }

    let es_textedit = ruta == "textedit.app" || ruta.ends_with("/textedit.app");
    let es_acerca = ruta == RUTA_ACERCA;
    let es_system_preferences =
        ruta == "system_preferences.app" || ruta.ends_with("/system_preferences.app");
    let fuente_sistema = es_system_preferences;
    let permite_redimension = !es_system_preferences && !es_acerca;
    let mut ventana = VentanaApp {
        ruta,
        titulo,
        icono,
        fuente_sistema,
        permite_redimension,
        x: 0,
        y: 0,
        w,
        h,
        pixels,
        app,
        ctx,
        finder: if es_finder {
            Some(FinderWindow::nuevo())
        } else {
            None
        },
        terminal: es_terminal.then(TerminalWindow::nueva),
        textedit: es_textedit.then(TextEditWindow::nuevo),
        acerca: es_acerca,
        id: 0,
        arrastrando: false,
        captura_raton: false,
        ancla_arrastre: (0, 0),
        ancla_redimension: None,
        hover_control: false,
        hover_boton_cabecera: None,
        control_armado: None,
        boton_cabecera_armado: None,
        segmento_cabecera_armado: None,
        geometria_normal: None,
        minimizada: false,
        genie: None,
        fondo: Vec::new(),
        fondo_intento: false,
    };
    // Se pinta en la tarea de carga y sobre un buffer privado. El compositor
    // sigue presentando escritorio y cursor mientras se cargan/decomprimen los
    // PNG iniciales; la ventana se publica sólo cuando la superficie está lista.
    if es_finder {
        // Incluye los estados normal y hover de navegación antes de publicar
        // la ventana, para que el primer movimiento nunca sea una carga fría.
        crate::toby_ui::kit::precalentar_finder();
    }
    pintar_superficie_ventana(&mut ventana);

    tarea::critica(|| {
        let k = comp();
        let c = k.canvas.as_ref()?;
        if !k.activo
            || w as i32 > c.width
            || h as i32 > c.height
            || k.ventanas_app.len() >= MAX_VENTANAS_APP
        {
            return None;
        }

        let ancho_pantalla = c.width;
        let alto_pantalla = c.height;
        let ancho = w as i32;
        let alto = h as i32;
        // Abre las nuevas ventanas en cascada para que no oculten por completo
        // a las que ya estaban abiertas.
        let desplazamiento = (k.ventanas_app.len() as i32 * 32).min(160);
        let max_x = (ancho_pantalla - ancho).max(0);
        let alto_dock = crate::dock::alto_reservado(alto_pantalla);
        let max_y = (alto_pantalla - alto - alto_dock).max(ALTO_MENU);
        let x = ((ancho_pantalla - ancho) / 2 + desplazamiento).clamp(0, max_x);
        let y = ((alto_pantalla - alto) / 2 + desplazamiento).clamp(ALTO_MENU, max_y);
        ventana.id = k.siguiente_id_ventana.max(1);
        k.siguiente_id_ventana = ventana.id.wrapping_add(1).max(1);
        ventana.x = x;
        ventana.y = y;
        k.ventanas_app.push(ventana);
        if let Some(ventana) = k.ventanas_app.last_mut() {
            ventana.app.colocar_puntero(k.mx - x, k.my - y);
        }
        repintar_completo(k);
        Some((x, y))
    })
}

fn pintar_superficie_ventana(ventana: &mut VentanaApp) {
    let borde = ventana.app.tema().paleta.borde;
    let mut lienzo = Canvas::from_buffer(
        ventana.pixels.as_mut_ptr(),
        ventana.w as i32,
        ventana.h as i32,
        ventana.w as usize,
    );
    let fuente = bios::get_font();
    if ventana.fuente_sistema {
        if let Some(fuente_suave) = crate::fuentes::para_barra_menu() {
            ventana
                .app
                .pintar_con_fuente_suave(&mut lienzo, fuente.as_ref(), &fuente_suave);
        } else {
            ventana.app.pintar(&mut lienzo, fuente.as_ref());
        }
    } else {
        ventana.app.pintar(&mut lienzo, fuente.as_ref());
    }
    if let Some(terminal) = ventana.terminal.as_mut() {
        pintar_terminal(&mut lienzo, terminal);
    }
    if let Some(textedit) = ventana.textedit.as_mut() {
        pintar_textedit(&mut lienzo, textedit);
    }
    if ventana.acerca {
        pintar_acerca(&mut lienzo);
    }
    if let Some(finder) = ventana.finder.as_mut() {
        pintar_finder(&mut lienzo, finder);
    }
    if ventana.finder.is_some() {
        let fuente = bios::get_font();
        let mut r = CanvasRenderer::nuevo(&mut lienzo, fuente.as_ref());
        pintar_cabecera_finder(&mut r, ventana, None);
        let finder = ventana.finder.as_mut().expect("Finder comprobado");
        if let Some((ax, ay, bx, by)) = finder.rectangulo {
            let area = area_archivos_finder(ventana.w, ventana.h);
            let actual = rectangulo_marco_finder(area, ax, ay, bx, by);
            finder.marco_base.clear();
            if !pintar_marco_finder_incremental(&lienzo, finder, Rect::VACIO, actual) {
                // El marco sigue siendo visible aunque el heap no pueda
                // reservar el respaldo incremental de su perímetro.
                pintar_borde_rectangulo(&lienzo, actual);
            }
        }
    }
    let fuente = bios::get_font();
    let mut r = CanvasRenderer::nuevo(&mut lienzo, fuente.as_ref());
    if ventana.permite_redimension {
        renderer::pintar_pie_ventana(
            &mut r,
            Rect {
                x: 0,
                y: 0,
                w: ventana.w,
                h: ventana.h,
            },
            borde,
        );
    }
}

/// Pinta la cabecera y los controles propios del Finder. El clip opcional
/// permite actualizar un único botón sin redibujar la lista, miniaturas ni el
/// resto de la ventana al cambiar el hover.
fn pintar_cabecera_finder<R: Renderer>(r: &mut R, ventana: &VentanaApp, clip: Option<Rect>) {
    let Some(finder) = ventana.finder.as_ref() else {
        return;
    };
    if let Some(clip) = clip {
        r.recorte(clip);
    }
    let rect = Rect {
        x: 0,
        y: 0,
        w: ventana.w,
        h: ventana.h,
    };
    let titulo = titulo_finder(ruta_visible_finder(finder), ventana.w);
    let botones = [
        renderer::BotonCabecera {
            imagen: crate::toby_ui::kit::indicador_izquierda(),
            habilitado: !finder.historial.is_empty()
                || (finder.vista == 2 && !finder.columnas.is_empty()),
        },
        renderer::BotonCabecera {
            imagen: crate::toby_ui::kit::indicador_derecha(),
            habilitado: !finder.historial_siguiente.is_empty(),
        },
    ];
    renderer::pintar_cabecera_ventana_appkit(
        r,
        rect,
        0,
        ventana.app.tema().paleta.borde,
        &titulo,
        Some(ventana.app.puntero()),
        renderer::CabeceraVentana {
            alto: ALTO_CABECERA_FINDER,
            botones: &botones,
        },
    );
    let recursos_vista = [
        crate::toby_ui::kit::icono_vista_finder(0),
        crate::toby_ui::kit::icono_vista_finder(1),
        crate::toby_ui::kit::icono_vista_finder(2),
        crate::toby_ui::kit::icono_vista_finder(3),
    ];
    if recursos_vista.iter().all(Option::is_some) {
        let caja_segmentada = rect_segmentado_finder(rect);
        let control_vista = renderer::Control {
            etiqueta: "segmented",
            variante: "blue",
            texto: SEGMENTOS_VISTA_FINDER,
            valor: finder.vista as i64,
            marcado: false,
            placeholder: "",
            estado: crate::toby_ui::theme::Estado::Normal,
        };
        renderer::pintar_control(
            r,
            &crate::toby_ui::theme::TEMA_POR_DEFECTO,
            caja_segmentada,
            &control_vista,
        );
        for (indice, segmento) in
            renderer::rectos_segmentos(SEGMENTOS_VISTA_FINDER, caja_segmentada)
                .into_iter()
                .enumerate()
        {
            if let Some(Some(imagen)) = recursos_vista.get(indice).copied() {
                let lado = 16u32.min(segmento.w).min(segmento.h);
                let icono = Rect {
                    x: segmento.x + (segmento.w as i32 - lado as i32) / 2,
                    y: segmento.y + (segmento.h as i32 - lado as i32) / 2,
                    w: lado,
                    h: lado,
                };
                if indice == finder.vista {
                    r.imagen_tintada(icono, imagen, 0x00FF_FFFF);
                } else {
                    r.imagen(icono, imagen);
                }
            }
        }
    }
    if clip.is_some() {
        r.recorte_fin();
    }
}

/// Renderiza y presenta solo la región de uno de los botones de navegación.
/// El estado del Finder y el buffer de ventana siguen siendo propiedad de Kira;
/// no se copia ni se vuelve a recorrer el contenido completo.
fn repintar_boton_navegacion_finder(k: &mut Comp, indice: usize) -> bool {
    let Some(ventana) = k.ventanas_app.last_mut() else {
        return false;
    };
    let marco = Rect {
        x: 0,
        y: 0,
        w: ventana.w,
        h: ventana.h,
    };
    let Some(caja) = renderer::rect_boton_cabecera(marco, ALTO_CABECERA_FINDER, indice) else {
        return false;
    };
    let mut lienzo = Canvas::from_buffer(
        ventana.pixels.as_mut_ptr(),
        ventana.w as i32,
        ventana.h as i32,
        ventana.w as usize,
    );
    let fuente = bios::get_font();
    let mut r = CanvasRenderer::nuevo(&mut lienzo, fuente.as_ref());
    pintar_cabecera_finder(&mut r, ventana, Some(caja));

    if !k.activo {
        return true;
    }
    if let Some(c) = k.canvas.as_mut() {
        k.sup.ocultar(c);
        let Some(ventana) = k.ventanas_app.last() else {
            return false;
        };
        presentar_rect_ventana(c, ventana, caja);
        true
    } else {
        false
    }
}

fn titulo_finder(ruta: &str, ancho: u32) -> String {
    const PREFIJO: &str = "Toby Finder — ";
    let max_caracteres = (ancho as usize).saturating_sub(176) / gfx::FONT_W as usize;
    let max_ruta = max_caracteres
        .saturating_sub(PREFIJO.chars().count())
        .max(4);
    if ruta.chars().count() <= max_ruta {
        return alloc::format!("{}{}", PREFIJO, ruta);
    }
    let ultimos: String = ruta.chars().rev().take(max_ruta - 1).collect();
    let sufijo: String = ultimos.chars().rev().collect();
    alloc::format!("{}…{}", PREFIJO, sufijo)
}

/// El segmented usa el mismo renglón y margen inferior que las flechas.
/// Su borde izquierdo se ancla al último botón AppKit de navegación.
fn rect_segmentado_finder(ventana: Rect) -> Rect {
    let x = renderer::rect_boton_cabecera(ventana, ALTO_CABECERA_FINDER, 1)
        .map_or(80, |boton| boton.der() + 8);
    let derecha = (ventana.der() - 10).max(x);
    let alto = ALTO_CABECERA_FINDER
        .min(ventana.h)
        .saturating_sub(renderer::ALTO_BARRA_VENTANA)
        .saturating_sub(4)
        .min(24);
    Rect {
        x,
        y: renderer::ALTO_BARRA_VENTANA as i32,
        w: ANCHO_SEGMENTADO_FINDER.min((derecha - x).max(0) as u32),
        h: alto,
    }
}

fn segmento_finder_en(ventana: Rect, x: i32, y: i32) -> Option<usize> {
    let control = rect_segmentado_finder(ventana);
    if !control.contiene(x, y) {
        return None;
    }
    renderer::rectos_segmentos(SEGMENTOS_VISTA_FINDER, control)
        .iter()
        .position(|segmento| segmento.contiene(x, y))
}

fn pintar_terminal(lienzo: &mut Canvas, terminal: &mut TerminalWindow) {
    let ancho = lienzo.width;
    let alto = lienzo.height;
    let area = Rect {
        x: 1,
        y: ALTO_MENU,
        w: (ancho - 2).max(1) as u32,
        h: (alto - ALTO_MENU - 20).max(1) as u32,
    };
    lienzo.fill_rect(
        area.x,
        area.y,
        area.w as i32,
        area.h as i32,
        gfx::rgb(18, 20, 22),
    );

    let columnas = ((area.w.saturating_sub(24)) / gfx::FONT_W as u32).max(1) as usize;
    let filas = (area.h / 16).max(1) as usize;
    let mut lineas = Vec::new();
    let _ = lineas.try_reserve_exact(filas.min(64));
    lineas.push(String::new());
    for &byte in terminal.salida.iter().chain(terminal.entrada.iter()) {
        match byte {
            b'\r' => {}
            b'\n' => {
                lineas.push(String::new());
            }
            0x08 => {
                if let Some(actual) = lineas.last_mut() {
                    actual.pop();
                }
            }
            b if (0x20..0x7F).contains(&b) || b >= 0x80 => {
                if lineas
                    .last()
                    .is_some_and(|linea| linea.chars().count() >= columnas)
                {
                    lineas.push(String::new());
                }
                if let Some(actual) = lineas.last_mut() {
                    actual.push(crate::console::char_de_cp437(b));
                }
            }
            _ => {}
        }
        if lineas.len() > filas {
            lineas.remove(0);
        }
    }

    let fuente = fuentes::para_terminal();
    let fuente_suave = fuentes::para_terminal_suave();
    {
        let mut renderer = CanvasRenderer::nuevo(lienzo, fuente.as_ref());
        renderer.recorte(area);
        for (fila, linea) in lineas.iter().enumerate() {
            let x = area.x + 12;
            let y = area.y + 8 + fila as i32 * 16;
            let color = gfx::rgb(224, 232, 226);
            if let Some(ref suave) = fuente_suave {
                renderer.texto_suavizado(x, y, linea, color, suave);
            } else {
                renderer.texto(x, y, linea, color, 1);
            }
        }
        renderer.recorte_fin();
    }
    let ultima = lineas.last().map_or(0, |linea| linea.chars().count());
    lienzo.fill_rect(
        area.x + 12 + (ultima.min(columnas.saturating_sub(1)) as i32 * gfx::FONT_W),
        area.y + 8 + (lineas.len().saturating_sub(1) as i32 * 16),
        2,
        14,
        gfx::rgb(126, 226, 151),
    );
    terminal.sucia = false;
}

/// Dibuja el documento con Times antialias. Solo se conservan en memoria las
/// líneas que caben en el viewport visible.
fn pintar_textedit(lienzo: &mut Canvas, editor: &mut TextEditWindow) {
    let area = Rect {
        x: 1,
        y: ALTO_MENU,
        w: (lienzo.width - 2).max(1) as u32,
        h: (lienzo.height - ALTO_MENU - 1).max(1) as u32,
    };
    lienzo.fill_rect(
        area.x,
        area.y,
        area.w as i32,
        area.h as i32,
        gfx::rgb(255, 255, 255),
    );

    let margen_x = 14;
    let margen_y = 12;
    let fuente_suave = fuentes::para_textedit();
    let fuente = if fuente_suave.is_none() {
        fuentes::para_kira()
    } else {
        None
    };
    let alto_glifo = fuente_suave
        .as_ref()
        .map_or_else(
            || {
                fuente
                    .as_ref()
                    .map_or(16, |font| font.bytes_per_char as i32)
            },
            |font| font.alto as i32,
        )
        .max(1);
    let ancho_texto = (area.w as i32 - margen_x * 2).max(1);
    let filas = ((area.h as i32 - margen_y * 2) / alto_glifo).max(1) as usize;
    let mut lineas = VecDeque::new();
    if lineas.try_reserve_exact(filas.saturating_add(1)).is_err() {
        editor.sucia = false;
        return;
    }
    let mut inicio_linea = 0usize;
    let mut ancho_linea = 0i32;
    let mut glifo_anterior = None;
    for (indice, &byte) in editor.contenido.iter().enumerate() {
        if byte == b'\n' {
            lineas.push_back((inicio_linea, indice));
            inicio_linea = indice + 1;
            ancho_linea = 0;
            glifo_anterior = None;
        } else if byte >= 0x20 {
            let caracter = crate::console::char_de_cp437(byte);
            let glifo = crate::texto::glifo_de(caracter) as usize;
            let (kerning, avance) = if let Some(font) = fuente_suave.as_ref() {
                let kerning = glifo_anterior.map_or(0, |anterior| {
                    if font.kerning_addr == 0 {
                        return 0;
                    }
                    let par = anterior * 256 + glifo;
                    unsafe {
                        core::ptr::read_volatile((font.kerning_addr + par) as *const i8) as i32
                    }
                });
                let avance = unsafe {
                    core::ptr::read_volatile((font.avances_addr + glifo) as *const u8) as i32
                }
                .max(1);
                (kerning, avance)
            } else {
                (0, gfx::FONT_W)
            };
            if ancho_linea > 0 && ancho_linea + kerning + avance > ancho_texto {
                lineas.push_back((inicio_linea, indice));
                inicio_linea = indice;
                ancho_linea = 0;
                glifo_anterior = None;
            }
            let kerning =
                if let (Some(font), Some(anterior)) = (fuente_suave.as_ref(), glifo_anterior) {
                    if font.kerning_addr == 0 {
                        0
                    } else {
                        let par = anterior * 256 + glifo;
                        unsafe {
                            core::ptr::read_volatile((font.kerning_addr + par) as *const i8) as i32
                        }
                    }
                } else {
                    0
                };
            ancho_linea += kerning + avance;
            glifo_anterior = fuente_suave.as_ref().map(|_| glifo);
        }
        if lineas.len() > filas {
            lineas.pop_front();
        }
    }
    lineas.push_back((inicio_linea, editor.contenido.len()));
    if lineas.len() > filas {
        lineas.pop_front();
    }

    let color_texto = gfx::rgb(32, 32, 36);
    let mut renderer = CanvasRenderer::nuevo(lienzo, fuente.as_ref());
    renderer.recorte(area);
    for (fila, &(inicio, fin)) in lineas.iter().enumerate() {
        let mut linea = String::new();
        if linea
            .try_reserve_exact(fin.saturating_sub(inicio).saturating_mul(4))
            .is_err()
        {
            editor.sucia = false;
            return;
        }
        for &byte in &editor.contenido[inicio..fin] {
            linea.push(crate::console::char_de_cp437(byte));
        }
        let x = area.x + margen_x;
        let y = area.y + margen_y + fila as i32 * alto_glifo;
        if let Some(ref suave) = fuente_suave {
            renderer.texto_suavizado(x, y, &linea, color_texto, suave);
        } else {
            renderer.texto(x, y, &linea, color_texto, 1);
        }
    }
    let caret_y = area.y + margen_y + (lineas.len().saturating_sub(1) as i32 * alto_glifo);
    let ancho_caret = lineas
        .back()
        .map(|&(inicio, fin)| {
            let mut ancho = 0i32;
            let mut anterior = None;
            for &byte in &editor.contenido[inicio..fin] {
                if byte < 0x20 {
                    continue;
                }
                if let Some(font) = fuente_suave.as_ref() {
                    let glifo =
                        crate::texto::glifo_de(crate::console::char_de_cp437(byte)) as usize;
                    if let Some(previo) = anterior.filter(|_| font.kerning_addr != 0) {
                        let par = previo * 256 + glifo;
                        ancho += unsafe {
                            core::ptr::read_volatile((font.kerning_addr + par) as *const i8) as i32
                        };
                    }
                    ancho += unsafe {
                        core::ptr::read_volatile((font.avances_addr + glifo) as *const u8) as i32
                    }
                    .max(1);
                    anterior = Some(glifo);
                } else {
                    ancho += gfx::FONT_W;
                }
            }
            ancho
        })
        .unwrap_or(0);
    renderer.recorte_fin();
    lienzo.fill_rect(
        area.x + margen_x + ancho_caret,
        caret_y,
        2,
        alto_glifo,
        gfx::rgb(44, 115, 230),
    );
    editor.sucia = false;
}

/// Superficie blanca de «Acerca de»: encabezado Times como el hero del HTML y
/// debajo especificaciones obtenidas de los subsistemas que están activos.
fn pintar_acerca(lienzo: &mut Canvas) {
    let area = Rect {
        x: 1,
        y: ALTO_MENU,
        w: (lienzo.width - 2).max(1) as u32,
        h: (lienzo.height - ALTO_MENU - 1).max(1) as u32,
    };
    lienzo.fill_rect(
        area.x,
        area.y,
        area.w as i32,
        area.h as i32,
        gfx::rgb(255, 255, 255),
    );

    const TAMANO_TITULO: u32 = 60;
    const TAMANO_DATOS: u32 = 17;
    const ETIQUETAS: [&str; 4] = ["RAM", "Almacenamiento", "CPU", "Gráficos"];
    let valores = [
        informacion_ram(),
        informacion_almacenamiento(),
        informacion_cpu(),
        informacion_graficos(),
    ];

    if let Some(titulo) = fuentes::titulo_acerca() {
        let ancho_primera = titulo.rocky.ancho as i32 + titulo.ampersand.ancho as i32;
        let x_primera = area.x + ((area.w as i32 - ancho_primera) / 2).max(0);
        let x_ampersand = x_primera + titulo.rocky.ancho as i32;
        let x_segunda = area.x + ((area.w as i32 - titulo.toby_os.ancho as i32) / 2).max(0);
        let y_primera = area.y + 8;
        let mut renderer = CanvasRenderer::nuevo(lienzo, None);
        renderer.recorte(area);
        renderer.mascara_suave(x_primera, y_primera, &titulo.rocky, gfx::rgb(32, 42, 52));
        renderer.mascara_suave(
            x_ampersand,
            y_primera,
            &titulo.ampersand,
            gfx::rgb(54, 129, 185),
        );
        renderer.mascara_suave(
            x_segunda,
            y_primera + TAMANO_TITULO as i32,
            &titulo.toby_os,
            gfx::rgb(32, 42, 52),
        );
        renderer.recorte_fin();
    } else {
        // Compatibilidad con discos anteriores que aún no incluyen about-title.rka.
        let fuente_titulo = fuentes::para_textedit();
        let fuente_bitmap = if fuente_titulo.is_none() {
            fuentes::para_kira()
        } else {
            None
        };
        let mut renderer = if let Some(ref suave) = fuente_titulo {
            CanvasRenderer::nuevo_con_fuente_suave(lienzo, fuente_bitmap.as_ref(), suave)
        } else {
            CanvasRenderer::nuevo(lienzo, fuente_bitmap.as_ref())
        };
        renderer.recorte(area);
        let piezas = ["Rocky ", "&"];
        let anchos = piezas.map(|pieza| renderer.medir_texto_tamano(pieza, TAMANO_TITULO) as i32);
        let ancho_total = anchos.iter().copied().sum::<i32>();
        let mut x = area.x + ((area.w as i32 - ancho_total) / 2).max(0);
        let y = area.y + 8;
        for (indice, pieza) in piezas.iter().enumerate() {
            let color = if indice == 1 {
                gfx::rgb(54, 129, 185)
            } else {
                gfx::rgb(32, 42, 52)
            };
            renderer.texto_tamano(x, y, pieza, color, TAMANO_TITULO);
            x += anchos[indice];
        }
        let subtitulo = "Toby OS";
        let ancho_subtitulo = renderer.medir_texto_tamano(subtitulo, TAMANO_TITULO) as i32;
        let x_subtitulo = area.x + ((area.w as i32 - ancho_subtitulo) / 2).max(0);
        renderer.texto_tamano(
            x_subtitulo,
            y + TAMANO_TITULO as i32,
            subtitulo,
            gfx::rgb(32, 42, 52),
            TAMANO_TITULO,
        );
        renderer.recorte_fin();
    }

    let separador_y = area.y + 136;
    lienzo.fill_rect(
        area.x + 36,
        separador_y,
        (area.w as i32 - 72).max(1),
        1,
        gfx::rgb(220, 227, 233),
    );

    let fuente_datos = fuentes::para_acerca();
    let fuente_bitmap = if fuente_datos.is_none() {
        fuentes::para_kira()
    } else {
        None
    };
    let mut renderer = if let Some(ref suave) = fuente_datos {
        CanvasRenderer::nuevo_con_fuente_suave(lienzo, fuente_bitmap.as_ref(), suave)
    } else {
        CanvasRenderer::nuevo(lienzo, fuente_bitmap.as_ref())
    };
    renderer.recorte(area);
    let x_etiqueta = area.x + 40;
    let x_valor = area.x + 177;
    let y_inicio = separador_y + 19;
    for indice in 0..ETIQUETAS.len() {
        let y = y_inicio + indice as i32 * 29;
        renderer.texto_tamano(
            x_etiqueta,
            y,
            ETIQUETAS[indice],
            gfx::rgb(36, 77, 104),
            TAMANO_DATOS,
        );
        renderer.texto_tamano(
            x_valor,
            y,
            &valores[indice],
            gfx::rgb(69, 88, 101),
            TAMANO_DATOS,
        );
    }
    renderer.recorte_fin();
}

fn informacion_ram() -> String {
    alloc::format!(
        "{} gestionada, {} libre",
        formatear_capacidad(crate::pmm::total_bytes() as u64),
        formatear_capacidad(crate::pmm::free_bytes() as u64)
    )
}

fn informacion_almacenamiento() -> String {
    let (total, libres) = if crate::fs::montado() {
        (
            crate::fs::total_bloques().saturating_mul(crate::fs::BLOQUE as u64),
            crate::fs::bloques_libres().saturating_mul(crate::fs::BLOQUE as u64),
        )
    } else if crate::disk::presente() {
        (crate::disk::sectores().saturating_mul(512), 0)
    } else {
        (0, 0)
    };
    if total == 0 {
        String::from("No disponible")
    } else if libres > 0 {
        alloc::format!(
            "{} total, {} libres",
            formatear_capacidad(total),
            formatear_capacidad(libres)
        )
    } else {
        alloc::format!("{} total", formatear_capacidad(total))
    }
}

fn informacion_cpu() -> String {
    let marca = crate::cpu::marca();
    let fin = marca
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(marca.len());
    let nombre = core::str::from_utf8(&marca[..fin])
        .unwrap_or("CPU x86-64")
        .trim();
    if nombre.is_empty() {
        String::from("CPU x86-64")
    } else {
        alloc::format!("{nombre}")
    }
}

fn informacion_graficos() -> String {
    let adaptador = crate::pci::funciones()
        .iter()
        .find(|funcion| funcion.es_video());
    let nombre = adaptador.map_or_else(
        || String::from("Adaptador no identificado"),
        |funcion| match crate::pci::producto_nombre(funcion.vendor, funcion.device) {
            Some(nombre) => String::from(nombre),
            None => alloc::format!(
                "{} {:04X}:{:04X}",
                crate::pci::fabricante_nombre(funcion.vendor),
                funcion.vendor,
                funcion.device
            ),
        },
    );
    match tamano() {
        Some((ancho, alto)) => alloc::format!("{nombre} - {ancho} x {alto}"),
        None => nombre,
    }
}

fn formatear_capacidad(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        alloc::format!("{}.{:01} GiB", bytes / GIB, (bytes % GIB) * 10 / GIB)
    } else if bytes >= MIB {
        alloc::format!("{}.{:01} MiB", bytes / MIB, (bytes % MIB) * 10 / MIB)
    } else if bytes >= 1024 {
        alloc::format!("{} KiB", bytes / 1024)
    } else {
        alloc::format!("{bytes} B")
    }
}

fn area_archivos_finder(ancho: u32, alto: u32) -> Rect {
    Rect {
        x: 142,
        y: ALTO_CABECERA_FINDER as i32 + 16,
        w: ancho.saturating_sub(154).max(1),
        h: alto.saturating_sub(ALTO_CABECERA_FINDER + 50).max(1),
    }
}

fn pintar_finder(c: &mut Canvas, finder: &mut FinderWindow) {
    let ancho = c.width.max(0) as u32;
    let alto = c.height.max(0) as u32;
    if ancho < 180 || alto <= ALTO_CABECERA_FINDER {
        return;
    }

    let fondo = gfx::rgb(250, 250, 252);
    let fuente_bios = bios::get_font();
    c.fill_rect(
        0,
        ALTO_CABECERA_FINDER as i32,
        c.width,
        c.height - ALTO_CABECERA_FINDER as i32,
        fondo,
    );

    let color_texto = gfx::rgb(55, 58, 64);

    let panel_lateral = Rect {
        x: 0,
        y: ALTO_CABECERA_FINDER as i32,
        w: 132,
        h: (alto as i32 - ALTO_CABECERA_FINDER as i32).max(0) as u32,
    };
    c.fill_rect(
        panel_lateral.x,
        panel_lateral.y,
        panel_lateral.w as i32,
        panel_lateral.h as i32,
        gfx::rgb(244, 245, 248),
    );
    c.fill_rect(
        131,
        ALTO_CABECERA_FINDER as i32,
        1,
        c.height - ALTO_CABECERA_FINDER as i32,
        gfx::rgb(224, 226, 230),
    );
    let accesos = [
        ("Equipo", "/"),
        ("Inicio", "/home"),
        ("Escritorio", crate::espacio::ESCRITORIO),
        ("Sistema", "/system"),
    ];
    for (indice, (etiqueta, destino)) in accesos.iter().enumerate() {
        let boton = Rect {
            x: 7,
            y: ALTO_CABECERA_FINDER as i32 + 14 + indice as i32 * 34,
            w: 116,
            h: 27,
        };
        let activo = ruta_visible_finder(finder) == *destino;
        if activo {
            crate::kira::desktop::pintar_superficie_redondeada(
                c,
                boton,
                6,
                gfx::rgb(218, 231, 251),
            );
        }
        let mut r = CanvasRenderer::nuevo(c, fuente_bios.as_ref());
        if let Some(font) = crate::fuentes::para_barra_menu() {
            r.texto_suavizado(
                boton.x + 9,
                boton.y + 7,
                etiqueta,
                if activo {
                    gfx::rgb(30, 82, 153)
                } else {
                    color_texto
                },
                &font,
            );
        } else {
            r.texto(boton.x + 9, boton.y + 7, etiqueta, color_texto, 1);
        }
    }

    let area = area_archivos_finder(ancho, alto);
    if finder.cargando {
        let mut r = CanvasRenderer::nuevo(c, fuente_bios.as_ref());
        let texto = "Cargando...";
        let x = area.x + ((area.w as i32 - gfx::text_width(texto, 1)).max(0) / 2);
        let y = area.y + ((area.h as i32 - 16).max(0) / 2);
        if let Some(font) = crate::fuentes::para_barra_menu() {
            r.texto_suavizado(x, y, texto, gfx::rgb(116, 119, 124), &font);
        } else {
            r.texto(x, y, "Cargando...", gfx::rgb(116, 119, 124), 1);
        }
        return;
    }
    let capacidad = capacidad_pagina_finder(finder, area);
    let total = finder.archivos.elementos.len();
    if finder.vista == 0 {
        let paginas = total.saturating_add(capacidad - 1) / capacidad;
        finder.pagina = finder.pagina.min(paginas.saturating_sub(1));
    } else if finder.vista == 2 {
        let capacidad_columna = capacidad_columna_finder(area);
        finder.inicio_elementos = finder
            .inicio_elementos
            .min(total.saturating_sub(capacidad_columna));
        for indice in 0..finder.inicios_columnas.len().min(finder.columnas.len()) {
            finder.inicios_columnas[indice] = finder.inicios_columnas[indice].min(
                finder.columnas[indice]
                    .elementos
                    .len()
                    .saturating_sub(capacidad_columna),
            );
        }
    } else {
        finder.inicio_elementos = finder.inicio_elementos.min(total.saturating_sub(capacidad));
    }
    if finder.vista == 1 {
        finder.desplazamiento_x_lista = finder
            .desplazamiento_x_lista
            .min((ANCHO_VIRTUAL_LISTA_FINDER as usize).saturating_sub(area.w as usize));
    }
    if finder.vista == 2 {
        ajustar_inicio_columnas_finder(finder, area);
    }
    if finder.vista == 2 {
        pintar_columnas_finder(c, finder, area);
    } else {
        let inicio = if finder.vista == 0 {
            finder.pagina.saturating_mul(capacidad)
        } else {
            finder.inicio_elementos
        };
        match finder.vista {
            1 => pintar_lista_finder(
                c,
                &finder.archivos,
                area,
                inicio,
                finder.desplazamiento_x_lista as i32,
            ),
            3 => pintar_galeria_finder(c, &finder.archivos, area, inicio),
            _ => finder.archivos.pintar_en(c, area, inicio),
        }
        if let Some(geometria) = geometria_objetivo_scroll_finder(
            finder,
            ObjetivoScrollFinder::ListaVertical,
            ancho,
            alto,
        ) {
            pintar_scroll_finder(c, &geometria);
        }
        if let Some(geometria) = geometria_objetivo_scroll_finder(
            finder,
            ObjetivoScrollFinder::ListaHorizontal,
            ancho,
            alto,
        ) {
            pintar_scroll_finder(c, &geometria);
        }
    }
    if total == 0 {
        let mut r = CanvasRenderer::nuevo(c, fuente_bios.as_ref());
        if let Some(font) = crate::fuentes::para_barra_menu() {
            r.texto_suavizado(
                area.x + 24,
                area.y + 24,
                "Vacía",
                gfx::rgb(116, 119, 124),
                &font,
            );
        } else {
            r.texto(
                area.x + 24,
                area.y + 24,
                "Vacia",
                gfx::rgb(116, 119, 124),
                1,
            );
        }
    }
}

fn ruta_visible_finder(finder: &FinderWindow) -> &str {
    finder.columnas.last().map_or_else(
        || finder.archivos.ruta_actual(),
        |columna| columna.ruta_actual(),
    )
}

fn capacidad_pagina_finder(finder: &FinderWindow, area: Rect) -> usize {
    match finder.vista {
        1 => {
            ((area.h as i32 - ALTO_CABECERA_LISTA_FINDER) / ALTO_FILA_LISTA_FINDER).max(1) as usize
        }
        2 => capacidad_columna_finder(area),
        3 => ((area.h as i32 - 8) / ALTO_FILA_GALERIA_FINDER).max(1) as usize,
        _ => finder.archivos.capacidad_en(area),
    }
}

fn capacidad_columna_finder(area: Rect) -> usize {
    ((area.h as i32 - 24) / ALTO_FILA_COLUMNA_FINDER).max(1) as usize
}

fn rango_columnas_finder(finder: &FinderWindow, area: Rect) -> (usize, usize, u32) {
    let total = finder.columnas.len() + 1;
    let visibles = ((area.w as i32) / ANCHO_COLUMNA_FINDER).max(1).min(4) as usize;
    let primera = finder.columna_inicio.min(total.saturating_sub(visibles));
    let cantidad = total.saturating_sub(primera).min(visibles).max(1);
    let ancho_columna = (area.w as usize / cantidad) as u32;
    (primera, cantidad, ancho_columna)
}

fn ajustar_inicio_columnas_finder(finder: &mut FinderWindow, area: Rect) {
    let total = finder.columnas.len() + 1;
    let visibles = ((area.w as i32) / ANCHO_COLUMNA_FINDER).max(1).min(4) as usize;
    finder.columna_inicio = finder.columna_inicio.min(total.saturating_sub(visibles));
}

fn geometria_scroll_finder(
    pista: Rect,
    total: usize,
    visibles: usize,
    desplazamiento: usize,
    paso: usize,
    horizontal: bool,
) -> Option<GeometriaScrollFinder> {
    let visibles = visibles.max(1).min(total);
    let desplazamiento_maximo = if paso > 1 && total > 0 {
        (total - 1) / paso * paso
    } else {
        total.saturating_sub(visibles)
    };
    let largo = if horizontal {
        pista.w as i32
    } else {
        pista.h as i32
    };
    if desplazamiento_maximo == 0 || largo <= 0 {
        return None;
    }
    let minimo_pulgar = if horizontal { 18 } else { 28 };
    let largo_pulgar = ((largo as i64 * visibles as i64) / total as i64)
        .clamp(minimo_pulgar.min(largo) as i64, largo as i64) as i32;
    let recorrido = (largo - largo_pulgar).max(0);
    let desplazamiento = desplazamiento.min(desplazamiento_maximo);
    let posicion = if desplazamiento_maximo == 0 {
        0
    } else {
        (recorrido as i64 * desplazamiento as i64 / desplazamiento_maximo as i64) as i32
    };
    let pulgar = if horizontal {
        Rect {
            x: pista.x + posicion,
            y: pista.y,
            w: largo_pulgar as u32,
            h: pista.h,
        }
    } else {
        Rect {
            x: pista.x,
            y: pista.y + posicion,
            w: pista.w,
            h: largo_pulgar as u32,
        }
    };
    Some(GeometriaScrollFinder {
        pista,
        pulgar,
        desplazamiento_maximo,
        desplazamiento,
        paso: paso.max(1),
        horizontal,
    })
}

fn geometria_objetivo_scroll_finder(
    finder: &FinderWindow,
    objetivo: ObjetivoScrollFinder,
    ancho: u32,
    alto: u32,
) -> Option<GeometriaScrollFinder> {
    let area = area_archivos_finder(ancho, alto);
    match objetivo {
        ObjetivoScrollFinder::ListaVertical if finder.vista != 2 => {
            let capacidad = capacidad_pagina_finder(finder, area);
            let pista = Rect {
                x: area.der(),
                y: area.y + 4,
                w: GROSOR_SCROLL_FINDER,
                h: area.h.saturating_sub(8),
            };
            let (desplazamiento, paso) = if finder.vista == 0 {
                (finder.pagina.saturating_mul(capacidad), capacidad)
            } else {
                (finder.inicio_elementos, 1)
            };
            geometria_scroll_finder(
                pista,
                finder.archivos.elementos.len(),
                capacidad,
                desplazamiento,
                paso,
                false,
            )
        }
        ObjetivoScrollFinder::ListaHorizontal if finder.vista == 1 => {
            let ancho_virtual = (area.w as usize).max(ANCHO_VIRTUAL_LISTA_FINDER as usize);
            let pista = Rect {
                x: area.x + 4,
                y: area.abajo() + 7,
                w: area.w.saturating_sub(8),
                h: GROSOR_SCROLL_FINDER,
            };
            geometria_scroll_finder(
                pista,
                ancho_virtual,
                area.w as usize,
                finder.desplazamiento_x_lista,
                1,
                true,
            )
        }
        ObjetivoScrollFinder::ColumnasHorizontal if finder.vista == 2 => {
            let (primera, visibles, _) = rango_columnas_finder(finder, area);
            let pista = Rect {
                x: area.x + 4,
                y: area.abajo() + 7,
                w: area.w.saturating_sub(8),
                h: GROSOR_SCROLL_FINDER,
            };
            geometria_scroll_finder(pista, finder.columnas.len() + 1, visibles, primera, 1, true)
        }
        ObjetivoScrollFinder::ColumnaVertical(indice) if finder.vista == 2 => {
            let (primera, visibles, ancho_columna) = rango_columnas_finder(finder, area);
            if indice < primera || indice >= primera + visibles {
                return None;
            }
            let listado = if indice == 0 {
                &finder.archivos
            } else {
                finder.columnas.get(indice - 1)?
            };
            let capacidad = capacidad_columna_finder(area);
            let inicio = if indice == 0 {
                finder.inicio_elementos
            } else {
                finder
                    .inicios_columnas
                    .get(indice - 1)
                    .copied()
                    .unwrap_or(0)
            };
            let posicion = indice - primera;
            let x = area.x + posicion as i32 * ancho_columna as i32;
            let pista = Rect {
                x: x + ancho_columna as i32 - GROSOR_SCROLL_FINDER as i32 - 2,
                y: area.y + 4,
                w: GROSOR_SCROLL_FINDER,
                h: area.h.saturating_sub(32),
            };
            geometria_scroll_finder(pista, listado.elementos.len(), capacidad, inicio, 1, false)
        }
        _ => None,
    }
}

fn pintar_scroll_finder(c: &mut Canvas, geometria: &GeometriaScrollFinder) {
    crate::kira::desktop::pintar_superficie_redondeada(
        c,
        geometria.pista,
        (GROSOR_SCROLL_FINDER / 2) as i32,
        gfx::rgb(232, 235, 240),
    );
    let mut appkit = CanvasRenderer::nuevo(c, None);
    renderer::pintar_pulgar_scroll_appkit(&mut appkit, geometria.pulgar, !geometria.horizontal);
}

fn iniciar_scroll_finder(
    finder: &FinderWindow,
    x: i32,
    y: i32,
    ancho: u32,
    alto: u32,
) -> Option<ArrastreScrollFinder> {
    let area = area_archivos_finder(ancho, alto);
    let mut objetivos = [None; 5];
    let mut cantidad_objetivos = 0;
    if finder.vista == 2 {
        objetivos[cantidad_objetivos] = Some(ObjetivoScrollFinder::ColumnasHorizontal);
        cantidad_objetivos += 1;
        let (primera, cantidad, _) = rango_columnas_finder(finder, area);
        for indice in primera..primera + cantidad {
            objetivos[cantidad_objetivos] = Some(ObjetivoScrollFinder::ColumnaVertical(indice));
            cantidad_objetivos += 1;
        }
    } else {
        objetivos[cantidad_objetivos] = Some(ObjetivoScrollFinder::ListaVertical);
        cantidad_objetivos += 1;
        if finder.vista == 1 {
            objetivos[cantidad_objetivos] = Some(ObjetivoScrollFinder::ListaHorizontal);
            cantidad_objetivos += 1;
        }
    }
    for objetivo in objetivos.into_iter().take(cantidad_objetivos).flatten() {
        let Some(geometria) = geometria_objetivo_scroll_finder(finder, objetivo, ancho, alto)
        else {
            continue;
        };
        if !geometria.pista.contiene(x, y) {
            continue;
        }
        let coordenada = if geometria.horizontal { x } else { y };
        let inicio_pulgar = if geometria.horizontal {
            geometria.pulgar.x
        } else {
            geometria.pulgar.y
        };
        let largo_pulgar = if geometria.horizontal {
            geometria.pulgar.w as i32
        } else {
            geometria.pulgar.h as i32
        };
        let agarre = if geometria.pulgar.contiene(x, y) {
            coordenada - inicio_pulgar
        } else {
            largo_pulgar / 2
        };
        return Some(ArrastreScrollFinder { objetivo, agarre });
    }
    None
}

fn mover_scroll_finder(
    finder: &mut FinderWindow,
    arrastre: ArrastreScrollFinder,
    x: i32,
    y: i32,
    ancho: u32,
    alto: u32,
) -> bool {
    let Some(geometria) = geometria_objetivo_scroll_finder(finder, arrastre.objetivo, ancho, alto)
    else {
        return false;
    };
    let coordenada = if geometria.horizontal { x } else { y };
    let inicio_pista = if geometria.horizontal {
        geometria.pista.x
    } else {
        geometria.pista.y
    };
    let largo_pista = if geometria.horizontal {
        geometria.pista.w as i32
    } else {
        geometria.pista.h as i32
    };
    let largo_pulgar = if geometria.horizontal {
        geometria.pulgar.w as i32
    } else {
        geometria.pulgar.h as i32
    };
    let recorrido = (largo_pista - largo_pulgar).max(0);
    if recorrido == 0 {
        return false;
    }
    let posicion = (coordenada - inicio_pista - arrastre.agarre).clamp(0, recorrido);
    let desplazamiento =
        (posicion as i64 * geometria.desplazamiento_maximo as i64 / recorrido as i64) as usize;
    let desplazamiento =
        (desplazamiento / geometria.paso * geometria.paso).min(geometria.desplazamiento_maximo);
    if desplazamiento == geometria.desplazamiento {
        return false;
    }
    match arrastre.objetivo {
        ObjetivoScrollFinder::ListaVertical => {
            if finder.vista == 0 {
                finder.pagina = desplazamiento / geometria.paso;
            } else {
                finder.inicio_elementos = desplazamiento;
            }
            finder.archivos.limpiar_seleccion();
            finder.ultimo_click = None;
        }
        ObjetivoScrollFinder::ListaHorizontal => {
            finder.desplazamiento_x_lista = desplazamiento;
        }
        ObjetivoScrollFinder::ColumnaVertical(indice) => {
            if indice == 0 {
                finder.inicio_elementos = desplazamiento;
                finder.archivos.limpiar_seleccion();
            } else if let Some(actual) = finder.inicios_columnas.get_mut(indice - 1) {
                *actual = desplazamiento;
                if let Some(columna) = finder.columnas.get_mut(indice - 1) {
                    columna.limpiar_seleccion();
                }
            }
            finder.ultimo_click = None;
        }
        ObjetivoScrollFinder::ColumnasHorizontal => {
            finder.columna_inicio = desplazamiento;
        }
    }
    true
}

fn pintar_lista_finder(
    c: &mut Canvas,
    archivos: &crate::kira::desktop::Escritorio,
    area: Rect,
    inicio: usize,
    desplazamiento_x: i32,
) {
    const DIVISION: i32 = 49;
    let fuente_bios = bios::get_font();
    let alto_cabecera = ALTO_CABECERA_LISTA_FINDER;
    let origen_x = area.x - desplazamiento_x;
    let ancho_virtual = (area.w as i32).max(ANCHO_VIRTUAL_LISTA_FINDER as i32);
    let separador_x = origen_x + ancho_virtual * DIVISION / 100;

    // Cabecera de dos columnas: Name queda neutra y Date Modified aparece
    // activa en azul, como en la referencia del Finder clásico.
    c.fill_rect(
        area.x,
        area.y,
        area.w as i32,
        alto_cabecera,
        gfx::rgb(241, 242, 244),
    );
    let inicio_fecha = separador_x.clamp(area.x, area.der());
    if inicio_fecha < area.der() {
        c.fill_rect(
            inicio_fecha,
            area.y,
            area.der() - inicio_fecha,
            alto_cabecera,
            gfx::rgb(105, 166, 222),
        );
    }
    c.fill_rect(area.x, area.y, area.w as i32, 1, gfx::rgb(255, 255, 255));
    if separador_x >= area.x && separador_x < area.der() {
        c.fill_rect(
            separador_x,
            area.y,
            1,
            area.h as i32,
            gfx::rgb(196, 200, 206),
        );
    }
    {
        let mut r = CanvasRenderer::nuevo(c, fuente_bios.as_ref());
        r.recorte(area);
        pintar_texto_finder(
            &mut r,
            "Name",
            origen_x + 28,
            area.y + 4,
            gfx::rgb(37, 39, 43),
        );
        pintar_texto_finder(
            &mut r,
            "Date Modified",
            separador_x + 12,
            area.y + 4,
            gfx::rgb(31, 48, 66),
        );
        r.recorte_fin();
    }
    let capacidad = ((area.h as i32 - alto_cabecera) / ALTO_FILA_LISTA_FINDER).max(1) as usize;
    for fila_indice in 0..capacidad {
        let color = if (inicio + fila_indice) % 2 == 1 {
            gfx::rgb(231, 241, 252)
        } else {
            gfx::rgb(255, 255, 255)
        };
        let y = area.y + alto_cabecera + fila_indice as i32 * ALTO_FILA_LISTA_FINDER;
        c.fill_rect(area.x, y, area.w as i32, ALTO_FILA_LISTA_FINDER, color);
    }
    let orden = orden_lista_finder(archivos);
    for posicion in inicio..orden.len().min(inicio.saturating_add(capacidad)) {
        let indice = orden[posicion];
        let item = &archivos.elementos[indice];
        let fila = Rect {
            x: area.x,
            y: area.y + alto_cabecera + (posicion - inicio) as i32 * ALTO_FILA_LISTA_FINDER,
            w: area.w,
            h: ALTO_FILA_LISTA_FINDER as u32,
        };
        let fondo_fila = if item.seleccionado {
            COLOR_SELECCION_LISTA_FINDER
        } else if posicion % 2 == 1 {
            gfx::rgb(231, 241, 252)
        } else {
            gfx::rgb(255, 255, 255)
        };
        c.fill_rect(fila.x, fila.y, fila.w as i32, fila.h as i32, fondo_fila);
        let icono = Rect {
            x: fila.x + 5 - desplazamiento_x,
            y: fila.y + (ALTO_FILA_LISTA_FINDER - 16) / 2,
            w: 16,
            h: 16,
        };
        if icono.x >= area.x && icono.der() <= area.der() {
            crate::kira::desktop::Escritorio::pintar_icono_en(c, item, icono);
        }
        let color = if item.seleccionado {
            gfx::rgb(255, 255, 255)
        } else {
            gfx::rgb(45, 48, 54)
        };
        let fuente = bios::get_font();
        let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
        r.recorte(area);
        let nombre_x = fila.x + 28 - desplazamiento_x;
        let nombre_ancho = (separador_x - nombre_x - 12).max(0);
        r.recorte(Rect {
            x: nombre_x,
            y: fila.y,
            w: nombre_ancho as u32,
            h: fila.h,
        });
        let nombre = truncar_texto_finder(&item.nombre, nombre_ancho);
        pintar_texto_finder(
            &mut r,
            &nombre,
            nombre_x,
            fila.y + (ALTO_FILA_LISTA_FINDER - 16) / 2,
            color,
        );
        r.recorte_fin();
        let fecha_x = separador_x + 12;
        let fecha_ancho = (fila.der() - fecha_x - 10).max(0);
        r.recorte(Rect {
            x: fecha_x,
            y: fila.y,
            w: fecha_ancho as u32,
            h: fila.h,
        });
        let fecha = truncar_texto_finder(&fecha_modificada_finder(item.modificado), fecha_ancho);
        pintar_texto_finder(
            &mut r,
            &fecha,
            fecha_x,
            fila.y + (ALTO_FILA_LISTA_FINDER - 16) / 2,
            color,
        );
        r.recorte_fin();
        r.recorte_fin();
    }
}

fn orden_lista_finder(archivos: &crate::kira::desktop::Escritorio) -> Vec<usize> {
    let mut orden: Vec<usize> = (0..archivos.elementos.len()).collect();
    orden.sort_unstable_by(|&a, &b| {
        archivos.elementos[b]
            .modificado
            .cmp(&archivos.elementos[a].modificado)
            .then_with(|| {
                archivos.elementos[a]
                    .nombre
                    .cmp(&archivos.elementos[b].nombre)
            })
    });
    orden
}

fn truncar_texto_finder(texto: &str, ancho: i32) -> String {
    let font = crate::fuentes::para_barra_menu();
    let medir = |s: &str| crate::kira::desktop::medir_texto(s, font.as_ref());
    if medir(texto) <= ancho {
        return String::from(texto);
    }
    let mut salida = String::new();
    for c in texto.chars() {
        let mut prueba = salida.clone();
        prueba.push(c);
        prueba.push('…');
        if medir(&prueba) > ancho {
            break;
        }
        salida.push(c);
    }
    if !salida.is_empty() {
        salida.push('…');
    }
    salida
}

fn fecha_modificada_finder(timestamp: u32) -> String {
    if timestamp == 0 {
        return String::from("—");
    }
    let hoy = crate::cmos::unix_time().max(0) as u32 / 86_400;
    let dias = timestamp / 86_400;
    let segundos = timestamp % 86_400;
    let hora = segundos / 3600;
    let minuto = (segundos / 60) % 60;
    let (anio, mes, dia, semana) = fecha_desde_dias_finder(dias);
    if dias == hoy {
        return alloc::format!("Today, {:02}:{:02}", hora, minuto);
    }
    const DIAS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MESES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    alloc::format!(
        "{}, {} {} {} {:02}:{:02}",
        DIAS[semana],
        dia,
        MESES[mes as usize - 1],
        anio,
        hora,
        minuto
    )
}

fn fecha_desde_dias_finder(mut dias: u32) -> (u32, u32, u32, usize) {
    let semana = (dias as usize + 4) % 7;
    let mut anio = 1970u32;
    loop {
        let bisiesto = anio % 4 == 0 && (anio % 100 != 0 || anio % 400 == 0);
        let longitud = if bisiesto { 366 } else { 365 };
        if dias < longitud {
            break;
        }
        dias -= longitud;
        anio += 1;
    }
    let mes_largo = [
        31,
        if anio % 4 == 0 && (anio % 100 != 0 || anio % 400 == 0) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mes = 0usize;
    while mes + 1 < mes_largo.len() && dias >= mes_largo[mes] {
        dias -= mes_largo[mes];
        mes += 1;
    }
    (anio, mes as u32 + 1, dias + 1, semana)
}

fn pintar_galeria_finder(
    c: &mut Canvas,
    archivos: &crate::kira::desktop::Escritorio,
    area: Rect,
    inicio: usize,
) {
    if archivos.elementos.is_empty() {
        return;
    }
    let ancho_previo = (area.w as i32 * 2 / 5).max(120);
    let lista = Rect {
        x: area.x + ancho_previo + 8,
        y: area.y + 4,
        w: area.w.saturating_sub(ancho_previo as u32 + 12),
        h: area.h.saturating_sub(8),
    };
    let capacidad = ((lista.h as i32) / ALTO_FILA_GALERIA_FINDER).max(1) as usize;
    let fin = archivos
        .elementos
        .len()
        .min(inicio.saturating_add(capacidad));
    let seleccionado = archivos
        .elementos
        .iter()
        .position(|item| item.seleccionado)
        .filter(|indice| *indice >= inicio && *indice < fin)
        .unwrap_or(inicio.min(archivos.elementos.len() - 1));
    let panel = Rect {
        x: area.x + 4,
        y: area.y + 4,
        w: ancho_previo.saturating_sub(8) as u32,
        h: area.h.saturating_sub(8),
    };
    crate::kira::desktop::pintar_superficie_redondeada(c, panel, 8, gfx::rgb(244, 245, 248));
    let tamano = (panel.w.min(panel.h.saturating_sub(64))).min(144).max(32);
    let icono = Rect {
        x: panel.x + (panel.w as i32 - tamano as i32) / 2,
        y: panel.y + 18,
        w: tamano,
        h: tamano,
    };
    let item = &archivos.elementos[seleccionado];
    crate::kira::desktop::Escritorio::pintar_icono_en(c, item, icono);
    {
        let fuente = bios::get_font();
        let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
        r.recorte(panel);
        pintar_texto_finder(
            &mut r,
            &item.nombre,
            panel.x + 10,
            icono.y + tamano as i32 + 16,
            gfx::rgb(45, 48, 54),
        );
        pintar_texto_finder(
            &mut r,
            clase_elemento_finder(item),
            panel.x + 10,
            icono.y + tamano as i32 + 34,
            gfx::rgb(112, 115, 121),
        );
        r.recorte_fin();
    }
    for indice in inicio..fin {
        let item = &archivos.elementos[indice];
        let fila = Rect {
            x: lista.x,
            y: lista.y + (indice - inicio) as i32 * ALTO_FILA_GALERIA_FINDER,
            w: lista.w,
            h: (ALTO_FILA_GALERIA_FINDER - 2) as u32,
        };
        if item.seleccionado || indice == seleccionado {
            crate::kira::desktop::pintar_superficie_redondeada(c, fila, 6, gfx::rgb(55, 125, 232));
        }
        crate::kira::desktop::Escritorio::pintar_icono_en(
            c,
            item,
            Rect {
                x: fila.x + 5,
                y: fila.y + 8,
                w: 22,
                h: 22,
            },
        );
        let color = if item.seleccionado || indice == seleccionado {
            gfx::rgb(255, 255, 255)
        } else {
            gfx::rgb(45, 48, 54)
        };
        let fuente = bios::get_font();
        let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
        r.recorte(fila);
        pintar_texto_finder(&mut r, &item.nombre, fila.x + 34, fila.y + 14, color);
        r.recorte_fin();
    }
}

fn pintar_columnas_finder(c: &mut Canvas, finder: &FinderWindow, area: Rect) {
    let (primera, visibles, ancho_col) = rango_columnas_finder(finder, area);
    let alto_filas = capacidad_columna_finder(area);
    for visible in 0..visibles {
        let columna_indice = primera + visible;
        let columna = if columna_indice == 0 {
            &finder.archivos
        } else {
            &finder.columnas[columna_indice - 1]
        };
        let x = area.x + visible as i32 * ancho_col as i32;
        let caja = Rect {
            x,
            y: area.y,
            w: ancho_col,
            h: area.h,
        };
        c.fill_rect(
            caja.x,
            caja.y,
            caja.w as i32,
            caja.h as i32,
            gfx::rgb(252, 252, 253),
        );
        if visible > 0 {
            c.fill_rect(caja.x, caja.y, 1, caja.h as i32, gfx::rgb(224, 226, 230));
        }
        let inicio = if columna_indice == 0 {
            finder.inicio_elementos
        } else {
            finder
                .inicios_columnas
                .get(columna_indice - 1)
                .copied()
                .unwrap_or(0)
        };
        for indice in inicio
            ..columna
                .elementos
                .len()
                .min(inicio.saturating_add(alto_filas))
        {
            let item = &columna.elementos[indice];
            let fila = Rect {
                x: caja.x + 4,
                y: caja.y + 3 + (indice - inicio) as i32 * ALTO_FILA_COLUMNA_FINDER,
                w: caja.w.saturating_sub(8),
                h: (ALTO_FILA_COLUMNA_FINDER - 1) as u32,
            };
            if item.seleccionado {
                crate::kira::desktop::pintar_superficie_redondeada(
                    c,
                    fila,
                    5,
                    gfx::rgb(55, 125, 232),
                );
            }
            crate::kira::desktop::Escritorio::pintar_icono_en(
                c,
                item,
                Rect {
                    x: fila.x + 5,
                    y: fila.y + 4,
                    w: 19,
                    h: 19,
                },
            );
            let color = if item.seleccionado {
                gfx::rgb(255, 255, 255)
            } else {
                gfx::rgb(45, 48, 54)
            };
            let fuente = bios::get_font();
            let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
            r.recorte(fila);
            pintar_texto_finder(&mut r, &item.nombre, fila.x + 29, fila.y + 9, color);
            r.recorte_fin();
        }
        if let Some(geometria) = geometria_objetivo_scroll_finder(
            finder,
            ObjetivoScrollFinder::ColumnaVertical(columna_indice),
            c.width.max(0) as u32,
            c.height.max(0) as u32,
        ) {
            pintar_scroll_finder(c, &geometria);
        }
    }
    if let Some(geometria) = geometria_objetivo_scroll_finder(
        finder,
        ObjetivoScrollFinder::ColumnasHorizontal,
        c.width.max(0) as u32,
        c.height.max(0) as u32,
    ) {
        pintar_scroll_finder(c, &geometria);
    }
}

fn pintar_texto_finder(r: &mut CanvasRenderer<'_, '_>, texto: &str, x: i32, y: i32, color: u32) {
    if let Some(font) = crate::fuentes::para_barra_menu() {
        r.texto_suavizado(x, y, texto, color, &font);
    } else {
        r.texto(x, y, texto, color, 1);
    }
}

fn clase_elemento_finder(item: &crate::kira::desktop::Elemento) -> &'static str {
    if item.tipo == crate::fs::Tipo::Directorio {
        if item.nombre.ends_with(".app") {
            "Aplicación"
        } else {
            "Carpeta"
        }
    } else {
        "Archivo"
    }
}

fn actualizar_marco_finder(
    finder: &mut FinderWindow,
    x: i32,
    y: i32,
    ancho: u32,
    alto: u32,
) -> Option<(Rect, Rect)> {
    let Some((ax, ay, anterior_x, anterior_y)) = finder.rectangulo else {
        return None;
    };
    let area = area_archivos_finder(ancho, alto);
    let bx = x.clamp(area.x, area.der() - 1);
    let by = y.clamp(area.y, area.abajo() - 1);
    if (bx, by) == (anterior_x, anterior_y) {
        return None;
    }
    let anterior = if finder.marco_base.is_empty() {
        Rect::VACIO
    } else {
        rectangulo_marco_finder(area, ax, ay, anterior_x, anterior_y)
    };
    let actual = rectangulo_marco_finder(area, ax, ay, bx, by);
    finder.rectangulo = Some((ax, ay, bx, by));
    if actual.w == 0 || actual.h == 0 {
        finder.archivos.limpiar_seleccion();
        for columna in &mut finder.columnas {
            columna.limpiar_seleccion();
        }
    } else {
        seleccionar_rectangulo_vista_finder(finder, ax, ay, bx, by, area);
    }
    (anterior != actual).then_some((anterior, actual))
}

fn seleccionar_rectangulo_vista_finder(
    finder: &mut FinderWindow,
    ax: i32,
    ay: i32,
    bx: i32,
    by: i32,
    area: Rect,
) {
    if finder.vista == 0 {
        let inicio = finder
            .pagina
            .saturating_mul(finder.archivos.capacidad_en(area));
        finder
            .archivos
            .seleccionar_rectangulo_en(ax, ay, bx, by, area, inicio);
        return;
    }
    let (x0, x1) = (ax.min(bx), ax.max(bx));
    let (y0, y1) = (ay.min(by), ay.max(by));
    if finder.vista == 1 {
        let capacidad = capacidad_pagina_finder(finder, area);
        let orden = orden_lista_finder(&finder.archivos);
        seleccionar_filas_finder(
            &mut finder.archivos,
            &Rect {
                y: area.y + ALTO_CABECERA_LISTA_FINDER,
                h: area.h.saturating_sub(ALTO_CABECERA_LISTA_FINDER as u32),
                ..area
            },
            finder.inicio_elementos,
            capacidad,
            ALTO_FILA_LISTA_FINDER,
            Some(&orden),
            x0,
            y0,
            x1,
            y1,
        );
    } else if finder.vista == 3 {
        let ancho_previo = (area.w as i32 * 2 / 5).max(120);
        let filas = Rect {
            x: area.x + ancho_previo + 8,
            y: area.y + 4,
            w: area.w.saturating_sub(ancho_previo as u32 + 12),
            h: area.h.saturating_sub(8),
        };
        let capacidad = capacidad_pagina_finder(finder, area);
        seleccionar_filas_finder(
            &mut finder.archivos,
            &filas,
            finder.inicio_elementos,
            capacidad,
            ALTO_FILA_GALERIA_FINDER,
            None,
            x0,
            y0,
            x1,
            y1,
        );
    } else {
        let total = finder.columnas.len() + 1;
        let visibles = ((area.w as i32) / ANCHO_COLUMNA_FINDER).max(1).min(4) as usize;
        let primera = total.saturating_sub(visibles);
        let ancho_col = (area.w as usize / (total - primera).max(1)) as u32;
        let capacidad = capacidad_columna_finder(area);
        for visible in 0..(total - primera) {
            let columna_indice = primera + visible;
            let inicio = if columna_indice == 0 {
                finder.inicio_elementos
            } else {
                finder
                    .inicios_columnas
                    .get(columna_indice - 1)
                    .copied()
                    .unwrap_or(0)
            };
            let caja = Rect {
                x: area.x + visible as i32 * ancho_col as i32,
                y: area.y + 3,
                w: ancho_col,
                h: area.h.saturating_sub(26),
            };
            let listado = if columna_indice == 0 {
                &mut finder.archivos
            } else {
                &mut finder.columnas[columna_indice - 1]
            };
            seleccionar_filas_finder(
                listado,
                &caja,
                inicio,
                capacidad,
                ALTO_FILA_COLUMNA_FINDER,
                None,
                x0,
                y0,
                x1,
                y1,
            );
        }
    }
}

fn seleccionar_filas_finder(
    archivos: &mut crate::kira::desktop::Escritorio,
    area: &Rect,
    inicio: usize,
    capacidad: usize,
    alto_fila: i32,
    orden: Option<&[usize]>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) {
    archivos.limpiar_seleccion();
    for posicion in inicio..inicio.saturating_add(capacidad) {
        let indice = match orden {
            Some(orden) => match orden.get(posicion) {
                Some(&indice) => indice,
                None => break,
            },
            None if posicion < archivos.elementos.len() => posicion,
            None => break,
        };
        let y = area.y + (posicion - inicio) as i32 * alto_fila;
        let fila = Rect {
            x: area.x,
            y,
            w: area.w,
            h: alto_fila.max(1) as u32,
        };
        archivos.elementos[indice].seleccionado =
            x0 < fila.der() && x1 >= fila.x && y0 < fila.abajo() && y1 >= fila.y;
    }
}

fn rectangulo_marco_finder(area: Rect, ax: i32, ay: i32, bx: i32, by: i32) -> Rect {
    if (bx - ax).abs() < 3 && (by - ay).abs() < 3 {
        return Rect::VACIO;
    }
    let x = ax.min(bx).clamp(area.x, area.der() - 1);
    let y = ay.min(by).clamp(area.y, area.abajo() - 1);
    Rect {
        x,
        y,
        w: (ax.max(bx).clamp(area.x, area.der() - 1) - x + 1) as u32,
        h: (ay.max(by).clamp(area.y, area.abajo() - 1) - y + 1) as u32,
    }
}

/// Restaura el borde anterior desde su pequeña copia de fondo y dibuja el
/// nuevo; no vuelve a componer iconos, textos ni el resto de la ventana.
fn pintar_marco_finder_incremental(
    c: &Canvas,
    finder: &mut FinderWindow,
    anterior: Rect,
    actual: Rect,
) -> bool {
    if anterior.w > 0 && anterior.h > 0 {
        let mut i = 0;
        let mut poner = |x, y| {
            if let Some(&color) = finder.marco_base.get(i) {
                c.put(x, y, color);
            }
            i += 1;
        };
        for x in anterior.x..anterior.der() {
            poner(x, anterior.y);
            poner(x, anterior.abajo() - 1);
        }
        for y in anterior.y..anterior.abajo() {
            poner(anterior.x, y);
            poner(anterior.der() - 1, y);
        }
    }
    finder.marco_base.clear();
    if actual.w == 0 || actual.h == 0 {
        return true;
    }
    let perimetro = 2 * (actual.w as usize + actual.h as usize);
    if finder.marco_base.try_reserve_exact(perimetro).is_err() {
        return false;
    }
    for x in actual.x..actual.der() {
        finder.marco_base.push(c.get(x, actual.y));
        finder.marco_base.push(c.get(x, actual.abajo() - 1));
    }
    for y in actual.y..actual.abajo() {
        finder.marco_base.push(c.get(actual.x, y));
        finder.marco_base.push(c.get(actual.der() - 1, y));
    }
    pintar_borde_rectangulo(c, actual);
    true
}

/// Actualiza el búfer de Finder antes de presentar los bordes dañados en
/// pantalla. Sin este paso, el compositor copiaba píxeles antiguos y el marco
/// de selección sólo aparecía al repintar la ventana por otro motivo.
fn pintar_dano_marco_finder(ventana: &mut VentanaApp, anterior: Rect, actual: Rect) -> bool {
    let Some(finder) = ventana.finder.as_mut() else {
        return false;
    };
    let lienzo = Canvas::from_buffer(
        ventana.pixels.as_mut_ptr(),
        ventana.w as i32,
        ventana.h as i32,
        ventana.w as usize,
    );
    pintar_marco_finder_incremental(&lienzo, finder, anterior, actual)
}

fn atender_finder(
    finder: &mut FinderWindow,
    x: i32,
    y: i32,
    ancho: u32,
    alto: u32,
    ahora: u64,
) -> bool {
    let tamano = (ancho as i32, alto as i32);
    for (indice, destino) in ["/", "/home", crate::espacio::ESCRITORIO, "/system"]
        .iter()
        .enumerate()
    {
        let boton = Rect {
            x: 7,
            y: ALTO_CABECERA_FINDER as i32 + 14 + indice as i32 * 34,
            w: 116,
            h: 27,
        };
        if boton.contiene(x, y) {
            navegar_finder(finder, destino, tamano);
            return true;
        }
    }

    let area = area_archivos_finder(ancho, alto);
    let capacidad = capacidad_pagina_finder(finder, area);

    if area.contiene(x, y) {
        if finder.vista == 2 {
            return atender_columnas_finder(finder, x, y, area, tamano, ahora);
        }
        let inicio = if finder.vista == 0 {
            finder.pagina.saturating_mul(capacidad)
        } else {
            finder.inicio_elementos
        };
        let indice = indice_elemento_vista_finder(finder, x, y, area, inicio, capacidad);
        finder.archivos.limpiar_seleccion();
        if let Some(indice) = indice {
            finder.archivos.seleccionar_elemento(indice);
            let doble = finder.ultimo_click.is_some_and(|(anterior, tic)| {
                anterior == indice && ahora.saturating_sub(tic) <= 45
            });
            finder.ultimo_click = Some((indice, ahora));
            if doble {
                if let Some(destino) = finder.archivos.ruta_elemento(indice) {
                    if destino.ends_with(".app") {
                        finder.aplicacion_pendiente = Some(destino);
                    } else {
                        navegar_finder(finder, &destino, tamano);
                    }
                }
            }
        } else {
            finder.ultimo_click = None;
        }
        return true;
    }
    false
}

fn indice_elemento_vista_finder(
    finder: &FinderWindow,
    x: i32,
    y: i32,
    area: Rect,
    inicio: usize,
    capacidad: usize,
) -> Option<usize> {
    match finder.vista {
        1 => {
            let y_inicio = area.y + ALTO_CABECERA_LISTA_FINDER;
            if y < y_inicio {
                return None;
            }
            let fila = ((y - y_inicio) / ALTO_FILA_LISTA_FINDER) as usize;
            if fila >= capacidad {
                return None;
            }
            orden_lista_finder(&finder.archivos)
                .get(inicio + fila)
                .copied()
        }
        3 => {
            let x_lista = area.x + (area.w as i32 * 2 / 5).max(120) + 8;
            if x < x_lista {
                return finder
                    .archivos
                    .elementos
                    .iter()
                    .position(|item| item.seleccionado);
            }
            let fila = ((y - area.y - 4) / ALTO_FILA_GALERIA_FINDER) as usize;
            (y >= area.y + 4 && fila < capacidad)
                .then_some(inicio + fila)
                .filter(|&indice| indice < finder.archivos.elementos.len())
        }
        _ => finder.archivos.elemento_bajo_en(x, y, area, inicio),
    }
}

fn atender_columnas_finder(
    finder: &mut FinderWindow,
    x: i32,
    y: i32,
    area: Rect,
    tamano: (i32, i32),
    ahora: u64,
) -> bool {
    let (primera, visibles_ahora, ancho_col) = rango_columnas_finder(finder, area);
    let visible = ((x - area.x).max(0) as u32 / ancho_col.max(1)) as usize;
    if visible >= visibles_ahora {
        return true;
    }
    let indice_columna = primera + visible;
    let capacidad = capacidad_columna_finder(area);
    let inicio = if indice_columna == 0 {
        finder.inicio_elementos
    } else {
        finder
            .inicios_columnas
            .get(indice_columna - 1)
            .copied()
            .unwrap_or(0)
    };
    let fila = (y - area.y - 3) / ALTO_FILA_COLUMNA_FINDER;
    if fila < 0 || fila as usize >= capacidad {
        return true;
    }
    let indice_item = inicio.saturating_add(fila as usize);
    let (destino, es_app, existe) = {
        let listado = if indice_columna == 0 {
            &mut finder.archivos
        } else {
            &mut finder.columnas[indice_columna - 1]
        };
        if indice_item >= listado.elementos.len() {
            listado.limpiar_seleccion();
            (None, false, false)
        } else {
            listado.limpiar_seleccion();
            listado.seleccionar_elemento(indice_item);
            let item = &listado.elementos[indice_item];
            (
                listado.ruta_elemento(indice_item),
                item.tipo == crate::fs::Tipo::Directorio && item.nombre.ends_with(".app"),
                true,
            )
        }
    };
    if !existe {
        finder.ultimo_click = None;
        finder.columnas.truncate(indice_columna);
        finder.inicios_columnas.truncate(indice_columna);
        ajustar_inicio_columnas_finder(finder, area);
        return true;
    }
    let id = indice_columna
        .saturating_mul(100)
        .saturating_add(indice_item);
    let doble = finder
        .ultimo_click
        .is_some_and(|(anterior, tic)| anterior == id && ahora.saturating_sub(tic) <= 45);
    finder.ultimo_click = Some((id, ahora));
    finder.columnas.truncate(indice_columna);
    finder.inicios_columnas.truncate(indice_columna);
    ajustar_inicio_columnas_finder(finder, area);
    if let Some(destino) = destino {
        if es_app && doble {
            finder.aplicacion_pendiente = Some(destino);
        } else if doble {
            navegar_finder(finder, &destino, tamano);
        } else {
            let mut columna = crate::kira::desktop::Escritorio::navegador(&destino);
            columna.refrescar(area.w as i32, area.h as i32);
            finder.columnas.push(columna);
            finder.inicios_columnas.push(0);
            finder.columna_inicio =
                (finder.columnas.len() + 1).saturating_sub(rango_columnas_finder(finder, area).1);
        }
    }
    true
}

fn navegar_finder(finder: &mut FinderWindow, destino: &str, tamano: (i32, i32)) {
    let anterior = String::from(ruta_visible_finder(finder));
    if finder.archivos.navegar_a(destino, tamano.0, tamano.1) {
        if anterior != finder.archivos.ruta_actual() {
            finder.historial.push(anterior);
            finder.historial_siguiente.clear();
        }
        finder.pagina = 0;
        finder.inicio_elementos = 0;
        finder.desplazamiento_x_lista = 0;
        finder.columnas.clear();
        finder.inicios_columnas.clear();
        finder.columna_inicio = 0;
        finder.arrastre_scroll = None;
        finder.ultimo_click = None;
    }
}

fn navegar_historial_finder(finder: &mut FinderWindow, atras: bool, tamano: (i32, i32)) -> bool {
    let destino = if atras {
        finder.historial.last().cloned()
    } else {
        finder.historial_siguiente.last().cloned()
    };
    let Some(destino) = destino else {
        return false;
    };
    let actual = String::from(ruta_visible_finder(finder));
    if !finder.archivos.navegar_a(&destino, tamano.0, tamano.1) {
        return false;
    }
    if atras {
        finder.historial.pop();
        finder.historial_siguiente.push(actual);
    } else {
        finder.historial_siguiente.pop();
        finder.historial.push(actual);
    }
    finder.pagina = 0;
    finder.inicio_elementos = 0;
    finder.desplazamiento_x_lista = 0;
    finder.columnas.clear();
    finder.inicios_columnas.clear();
    finder.columna_inicio = 0;
    finder.arrastre_scroll = None;
    finder.ultimo_click = None;
    true
}

/// Presenta la superficie de la app; al moverla restaura y captura el fondo
/// guardado, sin reconstruir ni transferir toda la pantalla.
fn actualizar_ventana_app_en(k: &mut Comp, x: i32, y: i32) -> bool {
    if !k.activo || k.canvas.is_none() {
        return false;
    }
    let (anterior, nueva, necesita_fondo) = match k.ventanas_app.last() {
        Some(w) if !w.minimizada => (
            (w.x, w.y, w.w as i32, w.h as i32),
            (x, y, w.w as i32, w.h as i32),
            w.fondo.len() < w.pixels.len(),
        ),
        None | Some(_) => return false,
    };

    if anterior == nueva && necesita_fondo {
        // No hay una instantánea para una actualización parcial; reconstruir
        // la escena garantiza que las esquinas vuelvan a mezclarse con el
        // fondo, no con los píxeles de la ventana anterior.
        repintar_completo(k);
        return true;
    }

    if anterior != nueva && necesita_fondo {
        if let Some(ventana) = k.ventanas_app.last_mut() {
            if !ventana.fondo_intento {
                ventana.fondo_intento = true;
                let mut fondo = Vec::new();
                if fondo.try_reserve_exact(ventana.pixels.len()).is_ok() {
                    fondo.resize(ventana.pixels.len(), FONDO_LIMPIO);
                    ventana.fondo = fondo;
                }
            }
        }
        let tiene_fondo = k
            .ventanas_app
            .last()
            .is_some_and(|w| w.fondo.len() >= w.pixels.len());
        if tiene_fondo {
            // Captura el fondo limpio en la posición antigua antes de usar el
            // camino incremental.
            repintar_completo(k);
        } else {
            // Si no hay memoria para la instantánea, mueve igualmente la
            // ventana con un solo repintado completo y no vuelve a reservar
            // hasta que termine este arrastre.
            if let Some(ventana) = k.ventanas_app.last_mut() {
                ventana.x = x;
                ventana.y = y;
            }
            repintar_completo(k);
            return true;
        }
    }

    if let Some(c) = k.canvas.as_ref() {
        c.esperar_retrazo_vertical();
    }
    if let Some(c) = k.canvas.as_mut() {
        k.sup.ocultar(c);
        if anterior == nueva {
            if let Some(ventana) = k.ventanas_app.last() {
                blit_ventana_app(c, ventana);
            }
            c.present_rect(nueva.0, nueva.1, nueva.2, nueva.3);
        } else {
            let ventana = k.ventanas_app.last_mut().unwrap();
            c.blit(
                ventana.fondo.as_ptr(),
                ventana.w as usize,
                anterior.0,
                anterior.1,
                anterior.2,
                anterior.3,
            );
            if !c.copiar_rect_recortado_a(
                &mut ventana.fondo,
                ventana.w as usize,
                nueva.0,
                nueva.1,
                nueva.2,
                nueva.3,
            ) {
                return false;
            }
            ventana.x = nueva.0;
            ventana.y = nueva.1;
            blit_ventana_app(c, ventana);
            let dx = (nueva.0 - anterior.0).abs();
            let dy = (nueva.1 - anterior.1).abs();
            if dx < nueva.2 && dy < nueva.3 {
                c.present_rect(
                    anterior.0.min(nueva.0),
                    anterior.1.min(nueva.1),
                    nueva.2 + dx,
                    nueva.3 + dy,
                );
            } else {
                c.present_rect(anterior.0, anterior.1, anterior.2, anterior.3);
                c.present_rect(nueva.0, nueva.1, nueva.2, nueva.3);
            }
        }
    }
    true
}

/// Presenta solo los bordes de selección viejos y nuevos. El gesto ocurre
/// dentro del panel de archivos, lejos de las esquinas redondeadas.
fn actualizar_marco_ventana_app_en(k: &mut Comp, anterior: Rect, actual: Rect) -> bool {
    if !k.activo {
        return false;
    }
    let Some(c) = k.canvas.as_mut() else {
        return false;
    };
    k.sup.ocultar(c);
    let Some(ventana) = k.ventanas_app.last() else {
        return false;
    };
    presentar_borde_ventana(c, ventana, anterior);
    presentar_borde_ventana(c, ventana, actual);
    true
}

fn presentar_borde_ventana(c: &Canvas, ventana: &VentanaApp, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    presentar_rect_ventana(c, ventana, Rect { h: 1, ..rect });
    presentar_rect_ventana(
        c,
        ventana,
        Rect {
            y: rect.abajo() - 1,
            h: 1,
            ..rect
        },
    );
    presentar_rect_ventana(c, ventana, Rect { w: 1, ..rect });
    presentar_rect_ventana(
        c,
        ventana,
        Rect {
            x: rect.der() - 1,
            w: 1,
            ..rect
        },
    );
}

fn presentar_rect_ventana(c: &Canvas, ventana: &VentanaApp, rect: Rect) {
    let ancho = ventana.w as usize;
    let x = ventana.x + rect.x;
    let y = ventana.y + rect.y;
    let offset = rect.y as usize * ancho + rect.x as usize;
    let pixels = unsafe { ventana.pixels.as_ptr().add(offset) };
    c.blit(pixels, ancho, x, y, rect.w as i32, rect.h as i32);
    c.present_rect(x, y, rect.w as i32, rect.h as i32);
}

/// Dibuja la superficie de la app y compone las esquinas antialiasadas sobre el
/// contenido que ya está debajo en el backbuffer. Así no hace falta mantener
/// una copia del fondo del tamaño entero de cada ventana.
fn blit_ventana_app(c: &Canvas, ventana: &VentanaApp) {
    let ancho = ventana.w as usize;
    let radio = renderer::RADIO_ESQUINA_VENTANA
        .min(ventana.w / 2)
        .min(ventana.h / 2);
    if radio == 0 {
        c.blit(
            ventana.pixels.as_ptr(),
            ancho,
            ventana.x,
            ventana.y,
            ventana.w as i32,
            ventana.h as i32,
        );
        return;
    }

    // En la zona superior dibuja primero el centro, dejando intacto el fondo
    // bajo los píxeles transparentes de las dos esquinas.
    for fila in 0..radio {
        let inicio_fila = fila as usize * ancho;
        let x_centro = radio as usize;
        let ancho_centro = ancho.saturating_sub(x_centro * 2);
        if ancho_centro > 0 {
            let centro = unsafe { ventana.pixels.as_ptr().add(inicio_fila + x_centro) };
            c.blit(
                centro,
                ancho,
                ventana.x + radio as i32,
                ventana.y + fila as i32,
                ancho_centro as i32,
                1,
            );
        }
        for columna in 0..radio {
            let cobertura = renderer::cobertura_exterior_esquina(fila, columna, radio);
            let izquierda = columna as usize;
            let derecha = ancho - 1 - izquierda;
            let indice_izquierda = fila as usize * ancho + izquierda;
            let indice_derecha = fila as usize * ancho + derecha;
            if cobertura == 0 {
                // El píxel está completamente dentro de la silueta: copiarlo
                // también es necesario; sólo los píxeles totalmente fuera
                // deben dejar intacto el fondo.
                c.put(
                    ventana.x + izquierda as i32,
                    ventana.y + fila as i32,
                    ventana.pixels[indice_izquierda],
                );
                c.put(
                    ventana.x + derecha as i32,
                    ventana.y + fila as i32,
                    ventana.pixels[indice_derecha],
                );
                continue;
            }
            let y = ventana.y + fila as i32;
            let fondo_izquierda = if ventana.fondo.len() >= ventana.pixels.len() {
                ventana.fondo[indice_izquierda]
            } else {
                c.get(ventana.x + izquierda as i32, y)
            };
            let fondo_derecha = if ventana.fondo.len() >= ventana.pixels.len() {
                ventana.fondo[indice_derecha]
            } else {
                c.get(ventana.x + derecha as i32, y)
            };
            c.put(
                ventana.x + izquierda as i32,
                y,
                sumar_cobertura_exterior(
                    ventana.pixels[indice_izquierda],
                    fondo_izquierda,
                    cobertura,
                ),
            );
            c.put(
                ventana.x + derecha as i32,
                y,
                sumar_cobertura_exterior(ventana.pixels[indice_derecha], fondo_derecha, cobertura),
            );
        }
    }

    // Desde el radio hacia abajo la silueta es cuadrada y se copia de una vez.
    if radio < ventana.h {
        let inicio = radio as usize * ancho;
        let pixels = unsafe { ventana.pixels.as_ptr().add(inicio) };
        c.blit(
            pixels,
            ancho,
            ventana.x,
            ventana.y + radio as i32,
            ventana.w as i32,
            (ventana.h - radio) as i32,
        );
    }
}

/// Los píxeles de esquina ya traen la cobertura interior premultiplicada
/// contra negro; añade la fracción exterior del fondo del escritorio.
fn sumar_cobertura_exterior(ventana: u32, fondo: u32, cobertura: u32) -> u32 {
    let canal = |desplazamiento: u32| {
        let frente = (ventana >> desplazamiento) & 0xFF;
        let detras = (fondo >> desplazamiento) & 0xFF;
        (frente + (detras * cobertura + 32) / 64).min(0xFF)
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}

/// Cambia tamaño real de la app y de sus dos superficies, manteniendo layout,
/// controles y framebuffer sincronizados.
fn fijar_geometria_ventana_app(
    ventana: &mut VentanaApp,
    x: i32,
    y: i32,
    ancho: u32,
    alto: u32,
    max_pixeles: usize,
) -> bool {
    let Some(longitud) = (ancho as usize).checked_mul(alto as usize) else {
        return false;
    };
    if ancho == 0 || alto == 0 || longitud > max_pixeles {
        return false;
    }
    // Al redimensionar ya no hace falta conservar el fondo del tamaño previo;
    // soltarlo evita mantener una segunda superficie completa de la ventana.
    ventana.fondo = Vec::new();
    ventana.fondo_intento = false;
    if !reservar_superficie_redimension(&mut ventana.pixels, longitud, max_pixeles)
        || !ventana.app.cambiar_tamano_ventana(ancho, alto)
    {
        return false;
    }
    ventana.pixels.resize(longitud, FONDO_LIMPIO);
    ventana.x = x;
    ventana.y = y;
    ventana.w = ancho;
    ventana.h = alto;
    pintar_superficie_ventana(ventana);
    true
}

/// Reserva capacidad en incrementos amortizados y acotados por la pantalla.
/// `try_reserve_exact` con el tamaño de cada evento del ratón causaba una
/// realocación y copia de toda la superficie en cada pequeño crecimiento.
fn reservar_superficie_redimension(
    superficie: &mut Vec<u32>,
    longitud: usize,
    max_pixeles: usize,
) -> bool {
    if longitud > max_pixeles {
        return false;
    }
    if longitud <= superficie.capacity() {
        return true;
    }
    // En redimensionado rápido, evitar reservas enormes que provoquen
    // `memory allocation failed`. Si no hay margen suficiente, redimensionar
    // sin reserva amortizada (se expandirá según sea necesario).
    let margen = (superficie.capacity() / 8).max(4 * 1024);
    let objetivo = longitud.saturating_add(margen).min(max_pixeles);
    let extra = objetivo.saturating_sub(superficie.len());
    if extra > 1024 * 1024 {
        // Intentar reserva más pequeña primero
        if superficie.try_reserve_exact(margen.min(64 * 1024)).is_ok() {
            return superficie.capacity() >= longitud;
        }
        return superficie.capacity() >= longitud;
    }
    match superficie.try_reserve_exact(extra) {
        Ok(()) => true,
        Err(_) => {
            // Caer a reserva pequeña y dejar que crezca si es necesario
            let _ = superficie.try_reserve_exact(margen.min(32 * 1024));
            superficie.capacity() >= longitud
        }
    }
}

/// Alterna la ventana entre su geometría anterior y el área libre del escritorio.
fn alternar_maximizacion_ventana(k: &mut Comp) -> bool {
    let Some((ancho_pantalla, alto_pantalla)) = k
        .canvas
        .as_ref()
        .filter(|_| k.activo)
        .map(|c| (c.width, c.height))
    else {
        return false;
    };
    if ancho_pantalla <= 0 || alto_pantalla <= ALTO_MENU {
        return false;
    }
    let Some(max_pixeles) = (ancho_pantalla as usize).checked_mul(alto_pantalla as usize) else {
        return false;
    };
    let Some(ventana) = k.ventanas_app.last_mut() else {
        return false;
    };

    let (x, y, ancho, alto, geometria_normal) = if let Some(normal) = ventana.geometria_normal {
        (normal.0, normal.1, normal.2, normal.3, None)
    } else {
        let normal = (ventana.x, ventana.y, ventana.w, ventana.h);
        let alto_dock = crate::dock::alto_reservado(alto_pantalla);
        let alto_util = (alto_pantalla - ALTO_MENU - alto_dock).max(1) as u32;
        (
            0,
            ALTO_MENU,
            ancho_pantalla.max(1) as u32,
            alto_util,
            Some(normal),
        )
    };
    if !fijar_geometria_ventana_app(ventana, x, y, ancho, alto, max_pixeles) {
        return false;
    }
    ventana.geometria_normal = geometria_normal;
    true
}

/// Enfoca o restaura una app del Dock que ya tiene una ventana.
fn activar_ventana_por_ruta(ruta: &str) -> bool {
    tarea::critica(|| {
        let k = comp();
        let Some(indice) = k
            .ventanas_app
            .iter()
            .rposition(|ventana| ventana.ruta == ruta)
        else {
            return false;
        };
        activar_ventana_en(k, indice)
    })
}

fn activar_ventana_por_id(id: u32) -> bool {
    tarea::critica(|| {
        let k = comp();
        let Some(indice) = k.ventanas_app.iter().position(|ventana| ventana.id == id) else {
            return false;
        };
        activar_ventana_en(k, indice)
    })
}

fn activar_ventana_en(k: &mut Comp, indice: usize) -> bool {
    // No se pisan dos deformaciones simultáneas. El clic sigue consumiéndose
    // como activación de la app ya abierta.
    if k.ventanas_app.iter().any(|ventana| ventana.genie.is_some()) {
        return true;
    }
    let ventana_id = k.ventanas_app[indice].id;
    let restaurar = k.ventanas_app[indice].minimizada;
    let origen_dock = if restaurar {
        k.canvas.as_ref().and_then(|c| {
            let abiertas =
                aplicaciones_abiertas_dock_con_carga(&k.ventanas_app, k.rebote_dock.as_ref());
            crate::dock::rect_icono_ventana(ventana_id, c.width, c.height, &abiertas).or_else(
                || {
                    crate::dock::rect_icono_ruta(
                        &k.ventanas_app[indice].ruta,
                        c.width,
                        c.height,
                        &abiertas,
                    )
                },
            )
        })
    } else {
        None
    };
    let inicio = crate::pit::ticks();
    let mut ventana = k.ventanas_app.remove(indice);
    ventana.minimizada = false;
    ventana.control_armado = None;
    if let Some(origen) = origen_dock {
        ventana.genie = Some(AnimacionGenie {
            origen: toby_appkit::genie::Rect {
                x: ventana.x,
                y: ventana.y,
                w: ventana.w,
                h: ventana.h,
            },
            destino: toby_appkit::genie::Rect {
                x: origen.x,
                y: origen.y,
                w: origen.w,
                h: origen.h,
            },
            reloj: toby_appkit::animation::Animation::new(inicio, DURACION_GENIE_TICKS),
            ultimo_tick: inicio,
            accion: AccionGenie::Restaurar,
        });
    }
    k.ventanas_app.push(ventana);
    repintar_completo(k);
    true
}

/// ¿Hay ya una app alojada como ventana en Kira?
pub fn ventana_app_abierta() -> bool {
    tarea::critica(|| !comp().ventanas_app.is_empty())
}

/// Indica que la ventana abierta es la Terminal gráfica.
pub fn terminal_activa() -> bool {
    tarea::critica(|| {
        let k = comp();
        k.activo
            && k.ventanas_app
                .iter()
                .any(|ventana| ventana.terminal.is_some())
    })
}

/// Enfoca la Terminal existente en vez de crear una segunda instancia.
pub fn enfocar_terminal() -> bool {
    tarea::critica(|| {
        let k = comp();
        let Some(indice) = k
            .ventanas_app
            .iter()
            .rposition(|ventana| ventana.terminal.is_some())
        else {
            return false;
        };
        let mut ventana = k.ventanas_app.remove(indice);
        ventana.minimizada = false;
        ventana.control_armado = None;
        k.ventanas_app.push(ventana);
        repintar_completo(k);
        true
    })
}

/// Escribe un byte de la consola en la Terminal si está abierta.
pub fn terminal_write_byte(byte: u8) -> bool {
    tarea::critica(|| {
        let k = comp();
        if !k.activo {
            return false;
        }
        let Some(terminal) = k
            .ventanas_app
            .iter_mut()
            .find_map(|ventana| ventana.terminal.as_mut())
        else {
            return false;
        };
        terminal.escribir(byte);
        true
    })
}

/// Limpia el buffer visible de Terminal en lugar de borrar el VGA oculto.
pub fn terminal_clear() -> bool {
    tarea::critica(|| {
        let k = comp();
        if !k.activo {
            return false;
        }
        let Some(terminal) = k
            .ventanas_app
            .iter_mut()
            .find_map(|ventana| ventana.terminal.as_mut())
        else {
            return false;
        };
        terminal.salida.clear();
        terminal.sucia = true;
        true
    })
}

/// Extrae la siguiente orden introducida en la ventana de Terminal.
pub fn terminal_tomar_orden() -> Option<Vec<u8>> {
    tarea::critica(|| {
        let k = comp();
        if !k.activo {
            return None;
        }
        let ventana = k.ventanas_app.last_mut()?;
        if ventana.minimizada {
            return None;
        }
        ventana.terminal.as_mut()?.ordenes.pop_front()
    })
}

/// Entrega la entrada de Kira a la ventana alojada y conserva el resto del
/// escritorio en el mismo ciclo de compositor. Los gestos que empiezan en el
/// fondo no se transforman en clics de la app al cruzar sobre su superficie.
fn atender_ventana_app(botones_antes: u8, teclas: &[u8], cursor_pendiente: bool) -> bool {
    let resultado = tarea::critica(|| {
        let k = comp();
        if !k.activo || k.ventanas_app.is_empty() {
            return false;
        }
        if let Some(indice_animada) = k
            .ventanas_app
            .iter()
            .position(|ventana| ventana.genie.is_some())
        {
            if teclas.contains(&ESC) {
                k.ventanas_app.remove(indice_animada);
                k.ventana_captura = false;
                repintar_completo(k);
                return true;
            }
            if teclas.is_empty() {
                return false;
            }
            let (repintar, cerrar) = {
                let ventana = &mut k.ventanas_app[indice_animada];
                let mut repintar = false;
                if let Some(terminal) = ventana.terminal.as_mut() {
                    for &tecla in teclas {
                        repintar |= terminal.tecla(tecla);
                    }
                } else if let Some(textedit) = ventana.textedit.as_mut() {
                    for &tecla in teclas {
                        repintar |= textedit.tecla(tecla);
                    }
                } else if ventana.finder.is_none() {
                    let x = (k.mx - ventana.x).clamp(0, ventana.w as i32 - 1);
                    let y = (k.my - ventana.y).clamp(0, ventana.h as i32 - 1);
                    let eventos = ventana.app.eventos_desde_anfitrion(x, y, 0, teclas);
                    repintar |= ventana
                        .app
                        .atender_eventos(&eventos, &mut ventana.ctx)
                        .repintar();
                }
                let cerrar = ventana.app.cerrada() || ventana.ctx.cerrado;
                if repintar {
                    pintar_superficie_ventana(ventana);
                }
                (repintar, cerrar)
            };
            if cerrar {
                k.ventanas_app.remove(indice_animada);
                k.ventana_captura = false;
            }
            if repintar || cerrar {
                repintar_completo(k);
            }
            return repintar || cerrar;
        }

        if teclas.contains(&ESC) {
            k.ventanas_app.pop();
            k.ventana_captura = false;
            repintar_completo(k);
            return true;
        }

        let (mx, my, botones) = (k.mx, k.my, k.botones);
        let marco_escritorio_capturado = k.seleccion.is_some();
        if botones_antes == 0 && botones != 0 {
            if let Some(indice) = indice_ventana_bajo(&k.ventanas_app, mx, my) {
                if elevar_ventana(&mut k.ventanas_app, indice) {
                    repintar_completo(k);
                }
            }
        }
        if k.ventanas_app
            .last()
            .map_or(true, |ventana| ventana.minimizada)
        {
            return false;
        }
        let tamano_pantalla = k
            .canvas
            .as_ref()
            .map(|c| (c.width.max(1) as u32, c.height.max(1) as u32))
            .unwrap_or((u32::MAX, u32::MAX));
        let max_pixeles = (tamano_pantalla.0 as usize)
            .checked_mul(tamano_pantalla.1 as usize)
            .unwrap_or(0);
        let mut nuevo_xy = None;
        let mut nuevo_tamano = None;
        let mut repintar_app = false;
        let mut dano_marco = None;
        let mut botones_cabecera_danados = [None, None];
        let (cerrar, control_activado, boton_cabecera_activado, segmento_activado) = {
            let ventana = k.ventanas_app.last_mut().unwrap();
            let dentro = mx >= ventana.x
                && mx < ventana.x + ventana.w as i32
                && my >= ventana.y
                && my < ventana.y + ventana.h as i32;
            let local_x = mx - ventana.x;
            let local_y = my - ventana.y;
            let alto_cabecera = if ventana.finder.is_some() {
                ALTO_CABECERA_FINDER
            } else {
                renderer::ALTO_BARRA_VENTANA
            };
            let nueva_pulsacion = botones & !botones_antes;
            let suelto_izquierdo = botones & 1 == 0 && botones_antes & 1 != 0;
            let control_bajo_cursor = if dentro {
                renderer::control_ventana_en(ventana.app.disposicion().rect(1), local_x, local_y)
            } else {
                None
            };
            let mut control_activado = None;
            let mut boton_cabecera_activado = None;
            let mut segmento_activado = None;
            let mut scroll_finalizado = false;

            if botones_antes == 0 && nueva_pulsacion != 0 && dentro {
                if nueva_pulsacion & 1 != 0 && local_y < alto_cabecera as i32 {
                    if let Some(control) = control_bajo_cursor {
                        ventana.control_armado = Some(control);
                        ventana.boton_cabecera_armado = None;
                        ventana.segmento_cabecera_armado = None;
                        ventana.arrastrando = false;
                        ventana.captura_raton = false;
                    } else if let Some(indice) = ventana.finder.as_ref().and_then(|_| {
                        renderer::boton_cabecera_en(
                            ventana.app.disposicion().rect(1),
                            alto_cabecera,
                            local_x,
                            local_y,
                            2,
                        )
                    }) {
                        ventana.boton_cabecera_armado = Some(indice);
                        ventana.segmento_cabecera_armado = None;
                        ventana.control_armado = None;
                        ventana.arrastrando = false;
                        ventana.captura_raton = false;
                    } else if let Some(indice) = ventana.finder.as_ref().and_then(|_| {
                        segmento_finder_en(ventana.app.disposicion().rect(1), local_x, local_y)
                    }) {
                        ventana.segmento_cabecera_armado = Some(indice);
                        ventana.control_armado = None;
                        ventana.boton_cabecera_armado = None;
                        ventana.arrastrando = false;
                        ventana.captura_raton = false;
                    } else {
                        ventana.boton_cabecera_armado = None;
                        ventana.segmento_cabecera_armado = None;
                        ventana.arrastrando = true;
                        ventana.fondo_intento = false;
                        ventana.ancla_arrastre = (local_x, local_y);
                        ventana.captura_raton = false;
                    }
                } else {
                    ventana.boton_cabecera_armado = None;
                    ventana.segmento_cabecera_armado = None;
                    if ventana.permite_redimension
                        && nueva_pulsacion & 1 != 0
                        && sobre_tirador_redimension(ventana.w, ventana.h, local_x, local_y)
                    {
                        ventana.control_armado = None;
                        ventana.arrastrando = false;
                        ventana.captura_raton = false;
                        ventana.ancla_redimension = Some((mx, my, ventana.w, ventana.h));
                    } else {
                        ventana.captura_raton = true;
                        if nueva_pulsacion & 1 != 0 {
                            if let Some(finder) = ventana.finder.as_mut() {
                                if let Some(arrastre) = iniciar_scroll_finder(
                                    finder, local_x, local_y, ventana.w, ventana.h,
                                ) {
                                    finder.arrastre_scroll = Some(arrastre);
                                    mover_scroll_finder(
                                        finder, arrastre, local_x, local_y, ventana.w, ventana.h,
                                    );
                                    repintar_app = true;
                                } else {
                                    let area = area_archivos_finder(ventana.w, ventana.h);
                                    if area.contiene(local_x, local_y) {
                                        finder.rectangulo =
                                            Some((local_x, local_y, local_x, local_y));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if ventana.arrastrando && botones & 1 != 0 {
                nuevo_xy = Some((mx - ventana.ancla_arrastre.0, my - ventana.ancla_arrastre.1));
            }
            if let Some((ancla_x, ancla_y, ancho_inicial, alto_inicial)) = ventana.ancla_redimension
            {
                if botones & 1 != 0 || suelto_izquierdo {
                    let (ancho_minimo, alto_minimo) = if ventana.finder.is_some() {
                        TAMANO_MIN_FINDER
                    } else {
                        TAMANO_MIN_VENTANA
                    };
                    nuevo_tamano = Some((
                        dimension_redimensionada(
                            ancho_inicial,
                            mx - ancla_x,
                            ancho_minimo,
                            tamano_pantalla.0,
                        ),
                        dimension_redimensionada(
                            alto_inicial,
                            my - ancla_y,
                            alto_minimo,
                            tamano_pantalla.1,
                        ),
                    ));
                }
            }
            if botones & 1 != 0 && ventana.captura_raton {
                if let Some(finder) = ventana.finder.as_mut() {
                    if let Some(arrastre) = finder.arrastre_scroll {
                        repintar_app |= mover_scroll_finder(
                            finder, arrastre, local_x, local_y, ventana.w, ventana.h,
                        );
                    } else {
                        dano_marco =
                            actualizar_marco_finder(finder, local_x, local_y, ventana.w, ventana.h);
                    }
                }
                if let Some((anterior, actual)) = dano_marco {
                    if !pintar_dano_marco_finder(ventana, anterior, actual) {
                        // Si no se pudo reservar la pequeña copia del borde,
                        // recompone la ventana completa para que el marco no
                        // desaparezca ni deje restos durante el arrastre.
                        dano_marco = None;
                        repintar_app = true;
                    }
                }
            }
            if suelto_izquierdo {
                ventana.arrastrando = false;
                ventana.fondo_intento = false;
                ventana.ancla_redimension = None;
                if let Some(armado) = ventana.control_armado.take() {
                    if control_bajo_cursor == Some(armado) {
                        control_activado = Some(armado);
                    }
                }
                if let Some(armado) = ventana.boton_cabecera_armado.take() {
                    let sobre_boton = ventana.finder.as_ref().and_then(|_| {
                        renderer::boton_cabecera_en(
                            ventana.app.disposicion().rect(1),
                            alto_cabecera,
                            local_x,
                            local_y,
                            2,
                        )
                    });
                    if sobre_boton == Some(armado) {
                        boton_cabecera_activado = Some(armado);
                    }
                }
                if let Some(armado) = ventana.segmento_cabecera_armado.take() {
                    if ventana.finder.is_some()
                        && segmento_finder_en(ventana.app.disposicion().rect(1), local_x, local_y)
                            == Some(armado)
                    {
                        segmento_activado = Some(armado);
                    }
                }
                if let Some(finder) = ventana.finder.as_mut() {
                    if let Some(arrastre) = finder.arrastre_scroll.take() {
                        mover_scroll_finder(
                            finder, arrastre, local_x, local_y, ventana.w, ventana.h,
                        );
                        scroll_finalizado = true;
                        repintar_app = true;
                    }
                }
                if ventana.captura_raton && ventana.finder.is_some() {
                    let (ancho, alto) = (ventana.w, ventana.h);
                    if let Some(finder) = ventana.finder.as_mut() {
                        if !scroll_finalizado {
                            actualizar_marco_finder(finder, local_x, local_y, ancho, alto);
                        }
                        let arrastro = !scroll_finalizado
                            && finder.rectangulo.is_some_and(|(ax, ay, bx, by)| {
                                (bx - ax).abs() >= 3 || (by - ay).abs() >= 3
                            });
                        finder.rectangulo = None;
                        finder.marco_base.clear();
                        repintar_app = true;
                        if !scroll_finalizado && !arrastro && dentro {
                            repintar_app |= atender_finder(
                                finder,
                                local_x,
                                local_y,
                                ancho,
                                alto,
                                crate::pit::ticks(),
                            );
                        }
                    }
                }
            }

            let con_captura = ventana.captura_raton;
            let enviar = ventana.control_armado.is_none()
                && control_activado.is_none()
                && boton_cabecera_activado.is_none()
                && segmento_activado.is_none()
                && (con_captura
                    || (!ventana.arrastrando && dentro && botones == 0)
                    || !teclas.is_empty());
            let x = local_x.clamp(0, ventana.w as i32 - 1);
            let y = local_y.clamp(0, ventana.h as i32 - 1);
            ventana.app.colocar_puntero(x, y);
            let hover_control =
                renderer::control_ventana_en(ventana.app.disposicion().rect(1), x, y).is_some();
            if !marco_escritorio_capturado && hover_control != ventana.hover_control {
                repintar_app = true;
                ventana.hover_control = hover_control;
            }
            let hover_boton_cabecera = if dentro {
                ventana.finder.as_ref().and_then(|_| {
                    renderer::boton_cabecera_en(
                        ventana.app.disposicion().rect(1),
                        alto_cabecera,
                        x,
                        y,
                        2,
                    )
                })
            } else {
                None
            };
            if !marco_escritorio_capturado && hover_boton_cabecera != ventana.hover_boton_cabecera {
                if ventana.finder.is_some() {
                    let anterior = ventana.hover_boton_cabecera;
                    if let Some(indice) = anterior {
                        botones_cabecera_danados[0] = Some(indice);
                    }
                    if let Some(indice) = hover_boton_cabecera {
                        if Some(indice) != anterior {
                            let slot = usize::from(botones_cabecera_danados[0].is_some());
                            botones_cabecera_danados[slot] = Some(indice);
                        }
                    }
                } else {
                    repintar_app = true;
                }
                ventana.hover_boton_cabecera = hover_boton_cabecera;
            }
            if ventana
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.sucia)
                || ventana
                    .textedit
                    .as_ref()
                    .is_some_and(|textedit| textedit.sucia)
            {
                repintar_app = true;
            }
            if enviar && ventana.finder.is_none() {
                if let Some(terminal) = ventana.terminal.as_mut() {
                    for &tecla in teclas {
                        repintar_app |= terminal.tecla(tecla);
                    }
                } else if let Some(textedit) = ventana.textedit.as_mut() {
                    for &tecla in teclas {
                        repintar_app |= textedit.tecla(tecla);
                    }
                } else {
                    let botones_app = if con_captura { botones } else { 0 };
                    let eventos = ventana
                        .app
                        .eventos_desde_anfitrion(x, y, botones_app, teclas);
                    let paso = ventana.app.atender_eventos(&eventos, &mut ventana.ctx);
                    repintar_app |= paso.repintar();
                }
            }
            if botones == 0 {
                ventana.captura_raton = false;
            }
            (
                ventana.app.cerrada() || ventana.ctx.cerrado,
                control_activado,
                boton_cabecera_activado,
                segmento_activado,
            )
        };

        if cerrar {
            k.ventanas_app.pop();
            k.ventana_captura = false;
            repintar_completo(k);
            return true;
        }

        if repintar_app {
            if let Some(ventana) = k.ventanas_app.last_mut() {
                pintar_superficie_ventana(ventana);
            }
        } else if botones_cabecera_danados.iter().any(Option::is_some) {
            if let Some(c) = k.canvas.as_ref() {
                c.esperar_retrazo_vertical();
            }
            for indice in botones_cabecera_danados.into_iter().flatten() {
                repintar_boton_navegacion_finder(k, indice);
            }
        }

        let mut redimensionado = false;
        if let Some((ancho, alto)) = nuevo_tamano {
            let tamano_actual = k.ventanas_app.last().map(|ventana| (ventana.w, ventana.h));
            if tamano_actual.is_some_and(|actual| actual != (ancho, alto)) {
                let aplicado = if let Some(ventana) = k.ventanas_app.last_mut() {
                    let (x, y) = (ventana.x, ventana.y);
                    if fijar_geometria_ventana_app(ventana, x, y, ancho, alto, max_pixeles) {
                        ventana.geometria_normal = None;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if aplicado {
                    // La superficie, el fondo capturado y el layout ya tienen
                    // la nueva geometría; se recompone la escena en un solo
                    // fotograma para no dejar restos al cambiar su perímetro.
                    repintar_completo(k);
                    redimensionado = true;
                }
            }
        }

        if let Some(indice) = segmento_activado {
            let cambiado = if let Some(ventana) = k.ventanas_app.last_mut() {
                if let Some(finder) = ventana.finder.as_mut() {
                    if indice < 4 && finder.vista != indice {
                        let ruta = String::from(ruta_visible_finder(finder));
                        let tamano = (ventana.w as i32, ventana.h as i32);
                        if ruta != finder.archivos.ruta_actual() {
                            finder.archivos.navegar_a(&ruta, tamano.0, tamano.1);
                        }
                        finder.columnas.clear();
                        finder.inicios_columnas.clear();
                        finder.columna_inicio = 0;
                        finder.inicio_elementos = 0;
                        finder.desplazamiento_x_lista = 0;
                        finder.arrastre_scroll = None;
                        finder.archivos.limpiar_seleccion();
                        finder.vista = indice;
                        finder.pagina = 0;
                        finder.ultimo_click = None;
                        finder.rectangulo = None;
                        pintar_superficie_ventana(ventana);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            if cambiado {
                repintar_completo(k);
            }
            return true;
        }

        if let Some(indice) = boton_cabecera_activado {
            let cambiado = if let Some(ventana) = k.ventanas_app.last_mut() {
                let tamano = (ventana.w as i32, ventana.h as i32);
                if let Some(finder) = ventana.finder.as_mut() {
                    if indice == 0 && finder.vista == 2 && !finder.columnas.is_empty() {
                        finder.columnas.pop();
                        finder.inicios_columnas.pop();
                        ajustar_inicio_columnas_finder(
                            finder,
                            area_archivos_finder(tamano.0 as u32, tamano.1 as u32),
                        );
                        finder.ultimo_click = None;
                        pintar_superficie_ventana(ventana);
                        true
                    } else if navegar_historial_finder(finder, indice == 0, tamano) {
                        pintar_superficie_ventana(ventana);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            if cambiado {
                repintar_completo(k);
            }
            return true;
        }

        match control_activado {
            Some(renderer::ControlVentana::Cerrar) => {
                k.ventanas_app.pop();
                k.ventana_captura = false;
                repintar_completo(k);
                return true;
            }
            Some(renderer::ControlVentana::Maximizar) => {
                if alternar_maximizacion_ventana(k) {
                    repintar_completo(k);
                    return true;
                }
            }
            Some(renderer::ControlVentana::Minimizar) => {
                let datos_ventana = k.ventanas_app.last().map(|ventana| {
                    (
                        ventana.id,
                        toby_appkit::genie::Rect {
                            x: ventana.x,
                            y: ventana.y,
                            w: ventana.w,
                            h: ventana.h,
                        },
                    )
                });
                if let Some((id, origen)) = datos_ventana {
                    let inicio = crate::pit::ticks();
                    if let Some(ventana) = k.ventanas_app.last_mut() {
                        // Publica primero la miniatura usando la superficie
                        // estable de la ventana; así el Dock reserva su celda
                        // antes de calcular el destino de la malla Genie.
                        ventana.genie = Some(AnimacionGenie {
                            origen,
                            destino: origen,
                            reloj: toby_appkit::animation::Animation::new(
                                inicio,
                                DURACION_GENIE_TICKS,
                            ),
                            ultimo_tick: inicio,
                            accion: AccionGenie::Minimizar,
                        });
                        ventana.control_armado = None;
                    }
                    let destino = k.canvas.as_ref().and_then(|c| {
                        let abiertas = aplicaciones_abiertas_dock_con_carga(
                            &k.ventanas_app,
                            k.rebote_dock.as_ref(),
                        );
                        crate::dock::rect_icono_ventana(id, c.width, c.height, &abiertas)
                    });
                    if let Some(destino) = destino {
                        if let Some(animacion) = k
                            .ventanas_app
                            .last_mut()
                            .and_then(|ventana| ventana.genie.as_mut())
                        {
                            animacion.destino = toby_appkit::genie::Rect {
                                x: destino.x,
                                y: destino.y,
                                w: destino.w,
                                h: destino.h,
                            };
                        }
                        k.ventana_captura = false;
                        repintar_completo(k);
                        return true;
                    }
                    if let Some(ventana) = k.ventanas_app.last_mut() {
                        ventana.genie = None;
                    }
                }
                if let Some(ventana) = k.ventanas_app.last_mut() {
                    ventana.minimizada = true;
                    ventana.control_armado = None;
                }
                if !k.ventanas_app.is_empty() {
                    let ventana = k.ventanas_app.remove(k.ventanas_app.len() - 1);
                    k.ventanas_app.insert(0, ventana);
                }
                k.ventana_captura = false;
                repintar_completo(k);
                return true;
            }
            None => {}
        }

        let posicion_actual = k.ventanas_app.last().map(|ventana| (ventana.x, ventana.y));
        let posicion_nueva = nuevo_xy.or(posicion_actual);
        let cambia_posicion = posicion_nueva != posicion_actual;
        let cambia_ventana = repintar_app
            || botones_cabecera_danados.iter().any(Option::is_some)
            || cambia_posicion
            || dano_marco.is_some()
            || redimensionado;
        if cambia_ventana && !redimensionado {
            if let Some((x, y)) = posicion_nueva {
                if !repintar_app && !cambia_posicion {
                    if let Some((anterior, actual)) = dano_marco {
                        let _ = actualizar_marco_ventana_app_en(k, anterior, actual);
                    }
                } else {
                    let _ = actualizar_ventana_app_en(k, x, y);
                }
            }
        }

        // El compositor difiere el cursor hasta que fondo y ventana han
        // terminado de actualizarse; así no lo borra y vuelve a dibujar dos
        // veces durante el mismo paquete de movimiento.
        if (cambia_ventana || cursor_pendiente) && !redimensionado {
            if let Some(c) = k.canvas.as_ref() {
                let ticks = crate::pit::ticks();
                let escala = escala_actual();
                pintar_puntero(c, &mut k.sup, k.mx, k.my, escala, k.cursor_estado, ticks);
                k.cursor_frame = cursor::indice_fotograma(k.cursor_estado, ticks);
                k.cursor_escala = escala;
            }
        }
        cambia_ventana || cursor_pendiente
    });

    // El anfitrión carga/valida el bundle y abre su vista; se hace fuera de la
    // sección crítica para que no retenga el compositor durante el acceso a FS.
    let aplicacion = tarea::critica(|| {
        comp().ventanas_app.iter_mut().rev().find_map(|ventana| {
            ventana
                .finder
                .as_mut()
                .and_then(|finder| finder.aplicacion_pendiente.take())
        })
    });
    if let Some(ruta) = aplicacion {
        solicitar_apertura_app(ruta);
        true
    } else {
        resultado
    }
}

/// Retira la app y vuelve a mostrar el escritorio y las ventanas de Kira.
pub fn cerrar_ventana_app() {
    tarea::critica(|| {
        let k = comp();
        if k.ventanas_app.pop().is_some() {
            k.ventana_captura = false;
            repintar_completo(k);
        }
    });
}

/// Cierra la Terminal por identidad, aunque otra ventana haya tomado el foco.
pub fn cerrar_terminal() -> bool {
    tarea::critica(|| {
        let k = comp();
        let Some(indice) = k
            .ventanas_app
            .iter()
            .rposition(|ventana| ventana.terminal.is_some())
        else {
            return false;
        };
        k.ventanas_app.remove(indice);
        k.ventana_captura = false;
        repintar_completo(k);
        true
    })
}

fn repintar_completo(k: &mut Comp) {
    let ticks = crate::pit::ticks();
    let escala = escala_actual();
    if let Some(c) = k.canvas.as_mut() {
        k.sup.ocultar(c);
        pintar_escritorio(
            c,
            k.seleccion,
            k.mx,
            k.my,
            k.boton_menu_pulsado,
            k.menu_toby_abierto,
            k.menu_toby_acerca,
            &k.escritorio,
            &mut k.ventanas_app,
            &mut k.marco_base,
            k.rebote_dock.as_ref(),
        );
        pintar_puntero(c, &mut k.sup, k.mx, k.my, escala, k.cursor_estado, ticks);
        k.cursor_frame = cursor::indice_fotograma(k.cursor_estado, ticks);
        k.cursor_escala = escala;
    }
}

/// Avanza sólo las piezas animadas de una ventana Toby sin reconstruir el
/// árbol de la app, volver a pintar Finder ni crear buffers temporales.
fn pintar_progreso_ventana(ventana: &mut VentanaApp, ahora: u64) {
    let mut lienzo = Canvas::from_buffer(
        ventana.pixels.as_mut_ptr(),
        ventana.w as i32,
        ventana.h as i32,
        ventana.w as usize,
    );
    let fuente = bios::get_font();
    ventana
        .app
        .pintar_progreso_animado(&mut lienzo, fuente.as_ref(), ahora);
}

/// Presenta sólo los rectángulos de progreso de la ventana superior. La ruta
/// rápida evita recomponer el fondo, Finder, Dock y el framebuffer completo.
/// Devuelve `false` si el estado visual requiere la composición general.
fn presentar_progreso_ventana(k: &mut Comp, indice: usize, ahora: u64) -> bool {
    let (mx, my, escala, estado) = (k.mx, k.my, escala_actual(), k.cursor_estado);
    let (canvas, sup, ventanas) = (&k.canvas, &mut k.sup, &k.ventanas_app);
    let Some(c) = canvas.as_ref() else {
        return false;
    };
    if ventanas.iter().rposition(|ventana| !ventana.minimizada) != Some(indice) {
        return false;
    }
    let Some(ventana) = ventanas.get(indice) else {
        return false;
    };
    if ventana.genie.is_some() {
        return false;
    }
    let radio = renderer::RADIO_ESQUINA_VENTANA
        .min(ventana.w / 2)
        .min(ventana.h / 2) as i32;
    for rect in ventana.app.regiones_progreso() {
        let dentro_x = rect.x >= radio && rect.der() <= ventana.w as i32 - radio;
        let dentro_y = rect.y >= radio && rect.abajo() <= ventana.h as i32 - radio;
        if !dentro_x && !dentro_y {
            return false;
        }
    }

    let cursor_anterior = sup.ocultar_en_buffer(c);
    for rect in ventana.app.regiones_progreso() {
        let origen = unsafe {
            ventana
                .pixels
                .as_ptr()
                .add(rect.y as usize * ventana.w as usize + rect.x as usize)
        };
        c.blit(
            origen,
            ventana.w as usize,
            ventana.x + rect.x,
            ventana.y + rect.y,
            rect.w as i32,
            rect.h as i32,
        );
    }
    pintar_puntero(c, sup, mx, my, escala, estado, ahora);
    for rect in ventana.app.regiones_progreso() {
        c.present_rect(
            ventana.x + rect.x,
            ventana.y + rect.y,
            rect.w as i32,
            rect.h as i32,
        );
    }
    if let Some((x, y, w, h)) = cursor_anterior {
        c.present_rect(x, y, w, h);
    }
    k.cursor_frame = cursor::indice_fotograma(estado, ahora);
    k.cursor_escala = escala;
    true
}

/// Refresca superficies sucias y barras indeterminadas con daño parcial.
fn refrescar_ventanas_sucias(ahora: u64) -> bool {
    tarea::critica(|| {
        let k = comp();
        if !k.activo {
            return false;
        }
        let avanzar_progreso =
            ahora.wrapping_sub(k.ultimo_tick_progreso) >= INTERVALO_PROGRESS_TICKS;
        if avanzar_progreso {
            k.ultimo_tick_progreso = ahora;
        }
        // La ventana superior es la única que puede actualizarse y presentarse
        // en la ruta rápida; las ocultas recuperan su fase actual al mostrarse.
        let ventana_visible = k
            .ventanas_app
            .iter()
            .rposition(|ventana| !ventana.minimizada);
        let progreso_visible = avanzar_progreso
            .then_some(ventana_visible)
            .flatten()
            .filter(|&i| k.ventanas_app[i].app.tiene_progreso_animado());
        let mut cambio_completo = false;
        let mut progreso_actualizado = None;
        for (indice, ventana) in k.ventanas_app.iter_mut().enumerate() {
            let sucia = ventana
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.sucia)
                || ventana
                    .textedit
                    .as_ref()
                    .is_some_and(|textedit| textedit.sucia);
            if sucia {
                pintar_superficie_ventana(ventana);
                cambio_completo = true;
            } else if Some(indice) == progreso_visible {
                pintar_progreso_ventana(ventana, ahora);
                progreso_actualizado = Some(indice);
            }
        }
        if cambio_completo {
            repintar_completo(k);
            return true;
        }
        if let Some(indice) = progreso_actualizado {
            if !presentar_progreso_ventana(k, indice, ahora) {
                repintar_completo(k);
            }
            return true;
        }
        false
    })
}

/// Indica si el compositor de Kira tiene ahora mismo el video.
pub fn activo() -> bool {
    tarea::critica(|| comp().activo)
}

/// Toma el video para Kira si nadie más lo tiene (escritorio + cursor).
///
/// Lo llama el servicio `kira` al arrancar; es inocuo si otra cosa
/// esta usando la pantalla.
pub fn arrancar() {
    // Los iconos de la Papelera se decodifican antes de entrar en la sección
    // crítica que activa y pinta Kira por primera vez.
    crate::dock::precalentar_papelera();
    let _ = crate::dock::actualizar_estado_papelera();
    let activo = tarea::critica(|| {
        let k = comp();
        k.cedido = false;
        k.activo
    });
    if !activo {
        // Fuera de la seccion critica: `reanudar` hace E/S de disco.
        reanudar(comp());
    }
}

/// Cede el video a otro subsistema (una app `.toby`, la demo, el visor...).
///
/// Quien cede pone despues su propio modo de video y, al terminar (o si no
/// consigue modo), tiene que llamar a [`recuperar`] para devolver la pantalla
/// al compositor.
pub fn ceder() {
    tarea::critica(|| {
        ocultar(comp());
    });
}

/// Devuelve la pantalla al compositor tras una [`ceder`].
///
/// En cuanto la tarea de fondo vuelva a despertar, Kira recupera su modo de
/// video (escritorio y cursor) sin que nadie más haga nada.
pub fn recuperar() {
    tarea::critica(|| {
        comp().cedido = false;
    });
}

/// Para el compositor y devuelve el equipo al modo texto (servicio parado).
///
/// No borra la consola VGA: el texto que se imprimio mientras Kira tenia la
/// pantalla (el saludo de arranque, el interprete...) sigue ahi y vuelve a
/// verse en cuanto se recupera el modo texto.
pub fn parar() {
    tarea::critica(|| {
        ocultar(comp());
    });
}

/// Aparta el escritorio de Kira de la pantalla: devuelve el modo texto y deja
/// marcado que el compositor no vuelva a tomarla (hasta un [`recuperar`] o un
/// [`arrancar`]). Es lo que comparten [`ceder`] (lo pide otro subsistema) y
/// [`parar`] (lo pide el servicio), y tambien la salida con ESC. No borra la
/// consola VGA: el texto impreso mientras Kira tenia la pantalla (el saludo de
/// arranque, el interprete...) sigue ahi y vuelve a verse en cuanto se
/// recupera el modo texto.
fn ocultar(k: &mut Comp) {
    k.cedido = true;
    if !k.activo {
        return;
    }
    bios::set_text_mode();
    k.activo = false;
    k.canvas = None;
    k.sup = Superposicion::new();
}

/// Tarea de fondo del servicio `kira`: sigue al raton y atiende la salida con
/// ESC. Mientras esta pantalla esta puesta, el teclado es suyo (el interprete
/// no lo sondea), asi que el ESC no se pierde.
pub extern "C" fn tarea_kira(_arg: u64) -> ! {
    // La Papelera se precalienta una sola vez en `arrancar`, antes de que esta
    // tarea exista. Repetir aquí la consulta de disco retrasaba el primer ciclo
    // que atiende teclado y ratón.
    let mut proximo_chequeo_papelera = crate::pit::ticks().saturating_add(100);
    loop {
        // Mientras otro subsistema tenga el video, Kira no toca el
        // framebuffer (el cedido se levanta con `recuperar`).
        let cedido = tarea::critica(|| comp().cedido);
        if cedido {
            tarea::dormir(50);
            continue;
        }
        // Recupera el modo si hace falta (tras salir de TobyUI, de la demo o
        // del visor de imagenes). Si no hay modo VESA util, se reintenta tarde.
        // `reanudar` cambia el modo de video y refresca el escritorio, lo que
        // lee del disco; debe correr FUERA de la seccion critica. Si se hiciera
        // dentro, el planificador no podria cambiar de tarea mientras se espera
        // al disco y el sistema se quedaria congelado si otra tarea lo tiene.
        let activo = tarea::critica(|| comp().activo);
        let recuperado = if activo {
            true
        } else {
            reanudar(comp());
            tarea::critica(|| comp().activo)
        };
        if !recuperado {
            tarea::dormir(2_000);
            continue;
        }
        // ESC: se sale del escritorio de Kira y vuelve el modo texto. El
        // servicio sigue activo (el servidor de Kira no se para); para volver
        // a ver el escritorio esta `servicio restart kira`.
        // Kira entrega a la ventana todos los bytes de texto ya traducidos por
        // input (ASCII/CP437). Drenar una cola acotada evita perder caracteres
        // cuando el repintado de una ventana tarda más que el ritmo del teclado.
        let hay_ventana = ventana_app_abierta();
        let mut buffer_teclas = [0u8; 16];
        let mut control_de_tecla = [false; 16];
        let mut cantidad_teclas = 0usize;
        if hay_ventana {
            while cantidad_teclas < buffer_teclas.len() {
                let Some((byte, control)) = input::poll_event() else {
                    break;
                };
                buffer_teclas[cantidad_teclas] = byte;
                control_de_tecla[cantidad_teclas] = control;
                cantidad_teclas += 1;
                if byte == ESC {
                    break;
                }
            }
        } else if let Some((byte, control)) = input::poll_event() {
            buffer_teclas[0] = byte;
            control_de_tecla[0] = control;
            cantidad_teclas = 1;
        }
        let teclas_leidas = &buffer_teclas[..cantidad_teclas];
        let mut teclas_app = [0u8; 16];
        let mut cantidad_teclas_app = 0usize;
        let mut zoom_escritorio = [None; 16];
        let mut cantidad_zoom = 0usize;
        for indice in 0..cantidad_teclas {
            if let Some(agrandar) =
                atajo_zoom_escritorio(buffer_teclas[indice], control_de_tecla[indice])
            {
                zoom_escritorio[cantidad_zoom] = Some(agrandar);
                cantidad_zoom += 1;
            } else {
                teclas_app[cantidad_teclas_app] = buffer_teclas[indice];
                cantidad_teclas_app += 1;
            }
        }
        let teclas = &teclas_app[..cantidad_teclas_app];
        if teclas_leidas.contains(&ESC) && !hay_ventana {
            let oculto = tarea::critica(|| {
                let k = comp();
                if k.menu_toby_abierto {
                    k.menu_toby_abierto = false;
                    k.menu_toby_acerca = false;
                    k.boton_menu_pulsado = false;
                    k.menu_item_armado = None;
                    if let Some(c) = k.canvas.as_mut() {
                        k.sup.ocultar(c);
                        pintar_escritorio(
                            c,
                            k.seleccion,
                            k.mx,
                            k.my,
                            false,
                            false,
                            false,
                            &k.escritorio,
                            &mut k.ventanas_app,
                            &mut k.marco_base,
                            k.rebote_dock.as_ref(),
                        );
                    }
                    false
                } else {
                    ocultar(k);
                    true
                }
            });
            if oculto {
                tarea::dormir(120);
            }
            continue;
        }

        mouse::poll();
        let (dx, dy, botones) = mouse::tomar();
        let botones_antes = tarea::critica(|| comp().botones);

        // Kira procesa puntero, selección, barra, dock y Finder en el mismo
        // ciclo. La ventana no toma el bucle de entrada ni bloquea el fondo.
        let cambio = tarea::critica(|| {
            let k = comp();
            if !k.activo {
                return (false, false, None, None, false, false);
            }
            let antes = k.botones;
            let anterior = k.seleccion;
            let boton_menu_antes = k.boton_menu_pulsado;
            let menu_antes = k.menu_toby_abierto;
            let acerca_antes = k.menu_toby_acerca;
            let mx_anterior = k.mx;
            let my_anterior = k.my;
            let control_ventana_antes =
                anterior.is_none() && sobre_control_ventana(mx_anterior, my_anterior);
            let opcion_menu_antes = menu_antes
                .then(|| indice_opcion_menu_toby(mx_anterior, my_anterior, acerca_antes))
                .flatten();
            let mut accion_energia = None;
            let mut abrir_app_dock = None;
            let mut abrir_acerca = false;
            if let Some(c) = k.canvas.as_mut() {
                k.mx = (k.mx + dx).clamp(0, c.width - 1);
                k.my = (k.my + dy).clamp(0, c.height - 1);
                let izquierdo = botones & 1 != 0;
                let estaba_izquierdo = antes & 1 != 0;
                let derecho = botones & 2 != 0;
                let estaba_derecho = antes & 2 != 0;
                let sobre_boton = dentro_boton_menu(k.mx, k.my);
                let opcion_menu = if k.menu_toby_abierto {
                    indice_opcion_menu_toby(k.mx, k.my, k.menu_toby_acerca)
                } else {
                    None
                };
                let apps_abiertas =
                    aplicaciones_abiertas_dock_con_carga(&k.ventanas_app, k.rebote_dock.as_ref());
                let app_sobre =
                    crate::dock::indice_icono_dock(k.mx, k.my, c.width, c.height, &apps_abiertas);
                let sobre_dock =
                    crate::dock::dentro_dock(k.mx, k.my, c.width, c.height, &apps_abiertas);
                let objetivo_dock_sobre =
                    app_sobre.and_then(|indice| crate::dock::objetivo_dock(indice, &apps_abiertas));
                let control_ventana_ahora = anterior.is_none()
                    && k.seleccion.is_none()
                    && sobre_control_ventana(k.mx, k.my);
                let sobre_ventana = indice_ventana_bajo(&k.ventanas_app, k.mx, k.my).is_some();

                if antes & 1 == 0 && izquierdo {
                    k.ventana_captura = sobre_ventana;
                }
                let mut cambio_escritorio = false;
                for agrandar in zoom_escritorio
                    .iter()
                    .take(cantidad_zoom)
                    .flatten()
                    .copied()
                {
                    cambio_escritorio |= k
                        .escritorio
                        .cambiar_tamano_icono(agrandar, c.width, c.height);
                }
                if derecho && !estaba_derecho && !sobre_ventana && k.my >= ALTO_MENU && !sobre_dock
                {
                    k.escritorio.abrir_menu(k.mx, k.my, c.width, c.height);
                    k.menu_boton_armado = false;
                    k.menu_item_armado = None;
                    k.menu_toby_acerca = false;
                    k.dock_app_armada = None;
                    k.seleccion = None;
                    k.arrastrando_escritorio = false;
                    cambio_escritorio = true;
                }

                let menu_escritorio_en_gesto = k.escritorio.menu_abierto();
                if menu_escritorio_en_gesto && izquierdo && !estaba_izquierdo {
                    k.escritorio.armar_accion_menu(k.mx, k.my);
                }
                if menu_escritorio_en_gesto && !izquierdo && estaba_izquierdo {
                    cambio_escritorio |= k.escritorio.soltar_menu(k.mx, k.my, c.width, c.height);
                    k.seleccion = None;
                    k.arrastrando_escritorio = false;
                    k.arrastre_escritorio_modificado = false;
                } else if !menu_escritorio_en_gesto {
                    if izquierdo && !estaba_izquierdo {
                        k.menu_boton_armado = !k.ventana_captura && sobre_boton;
                        k.menu_item_armado = if k.ventana_captura { None } else { opcion_menu };
                        k.dock_app_armada = if k.ventana_captura {
                            None
                        } else {
                            objetivo_dock_sobre.map(|objetivo| DockAppArmada {
                                ruta: String::from(objetivo.ruta),
                                ventana_id: objetivo.ventana_id,
                            })
                        };
                        if !k.ventana_captura
                            && k.menu_toby_abierto
                            && !sobre_boton
                            && !dentro_menu_toby(k.mx, k.my, k.menu_toby_acerca)
                        {
                            k.menu_toby_abierto = false;
                            k.menu_toby_acerca = false;
                        }
                        if k.my >= ALTO_MENU
                            && !k.ventana_captura
                            && !dentro_menu_toby(k.mx, k.my, k.menu_toby_acerca)
                            && !sobre_dock
                        {
                            if let Some(indice) = k.escritorio.elemento_bajo(k.mx, k.my) {
                                k.escritorio.seleccionar_elemento(indice);
                                k.arrastrando_escritorio = true;
                                k.arrastre_escritorio_modificado = false;
                                k.seleccion = None;
                            } else {
                                k.escritorio.limpiar_seleccion();
                                k.arrastrando_escritorio = false;
                                k.seleccion = Some((k.mx, k.my));
                            }
                            cambio_escritorio = true;
                        }
                    } else if izquierdo
                        && estaba_izquierdo
                        && k.arrastrando_escritorio
                        && (dx != 0 || dy != 0)
                    {
                        k.escritorio.mover_seleccionados(dx, dy, c.width, c.height);
                        k.arrastre_escritorio_modificado = true;
                        cambio_escritorio = true;
                    } else if !izquierdo && estaba_izquierdo {
                        let captura_app = k.ventana_captura;
                        if !captura_app
                            && k.menu_item_armado.is_some()
                            && k.menu_item_armado == opcion_menu
                        {
                            k.menu_toby_abierto = false;
                            k.menu_toby_acerca = false;
                            match opcion_menu {
                                Some(FILA_ACERCA_MENU) => abrir_acerca = true,
                                Some(FILA_SUSPENDER_MENU) => {
                                    accion_energia = Some(AccionEnergia::Suspender)
                                }
                                Some(FILA_REINICIAR_MENU) => {
                                    accion_energia = Some(AccionEnergia::Reiniciar)
                                }
                                Some(FILA_APAGAR_MENU) => {
                                    accion_energia = Some(AccionEnergia::Apagar)
                                }
                                _ => {}
                            }
                        } else if !captura_app && k.menu_boton_armado && sobre_boton {
                            k.menu_toby_abierto = !k.menu_toby_abierto;
                            k.menu_toby_acerca = false;
                        } else if !captura_app
                            && k.dock_app_armada.as_ref().is_some_and(|armado| {
                                objetivo_dock_sobre.is_some_and(|objetivo| {
                                    armado.ruta == objetivo.ruta
                                        && armado.ventana_id == objetivo.ventana_id
                                })
                            })
                        {
                            abrir_app_dock = objetivo_dock_sobre.map(|objetivo| DockAppArmada {
                                ruta: String::from(objetivo.ruta),
                                ventana_id: objetivo.ventana_id,
                            });
                        }
                        if k.arrastrando_escritorio {
                            if k.arrastre_escritorio_modificado {
                                k.escritorio.soltar_arrastre(c.width, c.height);
                            }
                            k.arrastrando_escritorio = false;
                            k.arrastre_escritorio_modificado = false;
                            cambio_escritorio = true;
                        } else if let Some((ax, ay)) = k.seleccion.take() {
                            k.escritorio.seleccionar_rectangulo(ax, ay, k.mx, k.my);
                            cambio_escritorio = true;
                        }
                        k.menu_boton_armado = false;
                        k.menu_item_armado = None;
                        k.dock_app_armada = None;
                        k.seleccion = None;
                    }
                }
                if botones == 0 {
                    k.ventana_captura = false;
                }
                k.boton_menu_pulsado =
                    k.menu_toby_abierto || (izquierdo && k.menu_boton_armado && sobre_boton);
                let movimiento_marco = anterior.is_some()
                    && k.seleccion.is_some()
                    && izquierdo
                    && estaba_izquierdo
                    && (dx != 0 || dy != 0);
                k.botones = botones;
                let mut rehacer_escena = anterior != k.seleccion
                    || boton_menu_antes != k.boton_menu_pulsado
                    || menu_antes != k.menu_toby_abierto
                    || (anterior.is_none()
                        && k.seleccion.is_none()
                        && control_ventana_antes != control_ventana_ahora)
                    || (opcion_menu_antes
                        != if k.menu_toby_abierto {
                            indice_opcion_menu_toby(k.mx, k.my, k.menu_toby_acerca)
                        } else {
                            None
                        })
                    || acerca_antes != k.menu_toby_acerca
                    || cambio_escritorio
                    || (anterior.is_none()
                        && k.seleccion.is_none()
                        && (my_anterior < ALTO_MENU || k.my < ALTO_MENU)
                        && (dx != 0 || dy != 0))
                    || (k.escritorio.menu_abierto() && (dx != 0 || dy != 0));
                if movimiento_marco && !rehacer_escena {
                    let dano_cursor = k.sup.ocultar_en_buffer(c);
                    if let Some((ax, ay)) = k.seleccion {
                        if !actualizar_seleccion_incremental(
                            c,
                            &k.marco_base,
                            ax,
                            ay,
                            mx_anterior,
                            my_anterior,
                            k.mx,
                            k.my,
                            dano_cursor,
                        ) {
                            rehacer_escena = true;
                        }
                    }
                }
                if rehacer_escena {
                    k.sup.ocultar(c);
                    pintar_escritorio(
                        c,
                        k.seleccion,
                        k.mx,
                        k.my,
                        k.boton_menu_pulsado,
                        k.menu_toby_abierto,
                        k.menu_toby_acerca,
                        &k.escritorio,
                        &mut k.ventanas_app,
                        &mut k.marco_base,
                        k.rebote_dock.as_ref(),
                    );
                }
                let ticks = crate::pit::ticks();
                let escala = escala_actual();
                let frame = cursor::indice_fotograma(k.cursor_estado, ticks);
                let cursor_pendiente = mx_anterior != k.mx
                    || my_anterior != k.my
                    || rehacer_escena
                    || !k.sup.visible()
                    || frame != k.cursor_frame
                    || escala != k.cursor_escala;
                let animando_genie = k.ventanas_app.iter().any(|ventana| ventana.genie.is_some());
                let diferir_cursor =
                    !animando_genie && k.ventanas_app.iter().any(|ventana| !ventana.minimizada);
                if cursor_pendiente && !diferir_cursor {
                    pintar_puntero(c, &mut k.sup, k.mx, k.my, escala, k.cursor_estado, ticks);
                    k.cursor_frame = frame;
                    k.cursor_escala = escala;
                }
                (
                    dx != 0 || dy != 0 || antes != botones,
                    rehacer_escena,
                    accion_energia,
                    abrir_app_dock,
                    cursor_pendiente && diferir_cursor,
                    abrir_acerca,
                )
            } else {
                (false, false, None, None, false, false)
            }
        });

        let cambio_ventana = atender_ventana_app(botones_antes, teclas, cambio.4);
        iniciar_resolucion_iconos_finder();
        let genie_actualizada = actualizar_animacion_genie();
        let rebote_actualizado = actualizar_animacion_rebote_dock();

        // La entrada se recoge y se aplica antes de consultar el sistema de
        // archivos o repintar superficies de apps. Así una operación lenta de
        // disco/composición no deja el cursor ni el teclado esperando detrás.
        let ahora = crate::pit::ticks();
        if ahora >= proximo_chequeo_papelera {
            proximo_chequeo_papelera = ahora.saturating_add(100);
            if crate::dock::actualizar_estado_papelera() {
                repintar_ventanas();
            }
        }
        let ventana_actualizada = refrescar_ventanas_sucias(ahora);

        match cambio.2 {
            Some(AccionEnergia::Suspender) => {
                crate::cpu::suspender();
                tarea::critica(|| repintar_completo(comp()));
            }
            Some(AccionEnergia::Reiniciar) => crate::cpu::reiniciar(),
            Some(AccionEnergia::Apagar) if !crate::cpu::shutdown() => {
                // Si ACPI/APM no logra apagar el equipo, se deja el menú accesible
                // para que el usuario pueda volver a intentarlo.
                tarea::critica(|| {
                    let k = comp();
                    k.menu_toby_abierto = true;
                    k.menu_toby_acerca = false;
                    k.boton_menu_pulsado = true;
                    if let Some(c) = k.canvas.as_mut() {
                        k.sup.ocultar(c);
                        pintar_escritorio(
                            c,
                            k.seleccion,
                            k.mx,
                            k.my,
                            true,
                            true,
                            false,
                            &k.escritorio,
                            &mut k.ventanas_app,
                            &mut k.marco_base,
                            k.rebote_dock.as_ref(),
                        );
                        let ticks = crate::pit::ticks();
                        let escala = escala_actual();
                        pintar_puntero(c, &mut k.sup, k.mx, k.my, escala, k.cursor_estado, ticks);
                        k.cursor_frame = cursor::indice_fotograma(k.cursor_estado, ticks);
                        k.cursor_escala = escala;
                    }
                });
            }
            _ => {}
        }
        if let Some(objetivo) = cambio.3 {
            solicitar_apertura_dock(objetivo);
        }
        if cambio.5 {
            abrir_ventana_acerca();
        }

        // Con el raton quieto la vuelta duerme (no hay nada que repintar). En
        // cuanto se mueve, no duerme: sigue los paquetes del PS/2 a la
        // velocidad que llegan, que es lo que hace que el puntero vaya fluido
        // y no a tirones. Repintar la caja del puntero es poquitos KB, y si
        // llega un paquete nuevo en el siguiente ciclo se consume aqui mismo.
        if !cambio.0
            && !cambio_ventana
            && !ventana_actualizada
            && !genie_actualizada
            && !rebote_actualizado
        {
            tarea::dormir(10);
        }
    }
}

/// Avanza una deformación Genie como máximo una vez por tick del PIT.
/// Así el compositor duerme entre fotogramas y nunca entra en un bucle ocupado.
fn actualizar_animacion_genie() -> bool {
    tarea::critica(|| {
        let k = comp();
        let Some(indice) = k
            .ventanas_app
            .iter()
            .position(|ventana| ventana.genie.is_some())
        else {
            return false;
        };
        let ahora = crate::pit::ticks();
        let Some(animacion) = k.ventanas_app[indice].genie else {
            return false;
        };
        if ahora == animacion.ultimo_tick {
            return false;
        }

        let terminada = animacion.reloj.finished(ahora);
        if terminada {
            if let Some(ventana) = k.ventanas_app.get_mut(indice) {
                ventana.genie = None;
                if matches!(animacion.accion, AccionGenie::Minimizar) {
                    ventana.minimizada = true;
                    ventana.arrastrando = false;
                    ventana.captura_raton = false;
                    ventana.control_armado = None;
                }
            }
            if matches!(animacion.accion, AccionGenie::Minimizar) {
                let ventana = k.ventanas_app.remove(indice);
                k.ventanas_app.insert(0, ventana);
            }
        } else if let Some(animacion) = k.ventanas_app[indice].genie.as_mut() {
            animacion.ultimo_tick = ahora;
        }
        repintar_completo(k);
        true
    })
}

/// Avanza el rebote del icono de Dock una vez por tick PIT.
fn actualizar_animacion_rebote_dock() -> bool {
    tarea::critica(|| {
        let k = comp();
        let Some(animacion) = k.rebote_dock.as_ref() else {
            return false;
        };
        let ahora = crate::pit::ticks();
        if ahora == animacion.ultimo_tick {
            return false;
        }
        if let Some(animacion) = k.rebote_dock.as_mut() {
            animacion.ultimo_tick = ahora;
        }
        repintar_completo(k);
        true
    })
}

/// Inicia la carga del Dock en segundo plano. El compositor no queda atrapado
/// leyendo y validando el paquete, así puede avanzar los fotogramas del cursor.
fn iniciar_carga_app(ruta: String) {
    let inicio = crate::pit::ticks();
    let iniciar = tarea::critica(|| {
        let k = comp();
        if k.cargando_app {
            return false;
        }
        k.cargando_app = true;
        k.rebote_dock = Some(ReboteDock {
            ruta: ruta.clone(),
            icono: String::from(crate::dock::ICONO_APP_GENERICO),
            reloj: toby_appkit::animation::Bounce::new(
                inicio,
                DURACION_REBOTE_DOCK_TICKS,
                ALTURA_REBOTE_DOCK_PX,
            ),
            ultimo_tick: inicio,
        });
        k.ruta_carga_app = Some(ruta);
        k.cursor_estado_antes_carga = Some(k.cursor_estado);
        true
    });
    if !iniciar {
        return;
    }

    fijar_estado_cursor(cursor::Estado::Pensando);
    tarea::critica(|| repintar_completo(comp()));
    tarea::crear("carga de app", tarea_cargar_app, 0);
}

/// Unifica el lanzamiento desde el Finder y el Dock: las apps abiertas se
/// enfocan/restauran; las cerradas pasan por la tarea de carga y su rebote.
fn solicitar_apertura_app(ruta: String) {
    if !activar_ventana_por_ruta(&ruta) {
        iniciar_carga_app(ruta);
    }
}

fn solicitar_apertura_dock(objetivo: DockAppArmada) {
    if let Some(id) = objetivo.ventana_id {
        if !activar_ventana_por_id(id) {
            solicitar_apertura_app(objetivo.ruta);
        }
    } else {
        solicitar_apertura_app(objetivo.ruta);
    }
}

/// Carga y abre la app seleccionada mientras la tarea del compositor sigue
/// atendiendo la pantalla. Al terminar, restaura el estado previo del cursor.
extern "C" fn tarea_cargar_app(_arg: u64) -> ! {
    let ruta = tarea::critica(|| comp().ruta_carga_app.take());
    if let Some(ruta) = ruta {
        // Resolver y decodificar el icono antes de montar la app. Así el Dock
        // muestra su icono real durante el rebote y el pintado posterior sólo
        // consulta la caché de imágenes; el trabajo de disco nunca bloquea Kira.
        let icono = crate::toby_ui::paquete::cargar_ruta_icono(&ruta)
            .and_then(|ruta_icono| {
                crate::toby_ui::kit::imagen_recurso(&ruta_icono).map(|_| ruta_icono)
            })
            .unwrap_or_else(|| String::from(crate::dock::ICONO_APP_GENERICO));
        tarea::critica(|| {
            let k = comp();
            let Some(rebote) = k.rebote_dock.as_mut().filter(|rebote| rebote.ruta == ruta) else {
                return;
            };
            rebote.icono = icono;
            repintar_completo(k);
        });
        let _ = crate::toby_ui::host::ver(&ruta);
    }

    let (estado_anterior, borrar_rebote) = tarea::critica(|| {
        let k = comp();
        k.cargando_app = false;
        k.ruta_carga_app = None;
        let estado = k
            .cursor_estado_antes_carga
            .take()
            .unwrap_or(cursor::Estado::Flecha);
        (estado, k.rebote_dock.take().is_some())
    });
    fijar_estado_cursor(estado_anterior);
    if borrar_rebote {
        tarea::critica(|| repintar_completo(comp()));
    }
    tarea::terminar()
}

/// Programa la resolución de iconos si alguna vista Finder acaba de descubrir
/// bundles nuevos. La lectura del manifiesto y la decodificación PNG ocurren
/// en otra tarea, nunca durante el pintado ni dentro de la sección crítica.
fn iniciar_resolucion_iconos_finder() {
    let iniciar = tarea::critica(|| {
        let k = comp();
        if k.resolviendo_iconos_finder
            || !k.ventanas_app.iter().any(|ventana| {
                ventana.finder.as_ref().is_some_and(|finder| {
                    finder.archivos.tiene_iconos_app_pendientes()
                        || finder
                            .columnas
                            .iter()
                            .any(|columna| columna.tiene_iconos_app_pendientes())
                })
            })
        {
            return false;
        }
        k.resolviendo_iconos_finder = true;
        true
    });
    if iniciar {
        tarea::crear("iconos de apps Finder", tarea_cargar_iconos_finder, 0);
    }
}

/// Lee los manifiestos pendientes en segundo plano y precarga cada PNG antes
/// de publicar su ruta en las vistas Finder.
extern "C" fn tarea_cargar_iconos_finder(_arg: u64) -> ! {
    let mut cambio_total = false;
    loop {
        let bundle = tarea::critica(|| {
            let k = comp();
            for ventana in &mut k.ventanas_app {
                let Some(finder) = ventana.finder.as_mut() else {
                    continue;
                };
                if let Some(ruta) = finder.archivos.tomar_ruta_icono_app_pendiente() {
                    return Some(ruta);
                }
                for columna in &mut finder.columnas {
                    if let Some(ruta) = columna.tomar_ruta_icono_app_pendiente() {
                        return Some(ruta);
                    }
                }
            }
            None
        });
        let Some(bundle) = bundle else {
            break;
        };

        let icono = crate::toby_ui::paquete::cargar_ruta_icono(&bundle)
            .and_then(|ruta| crate::toby_ui::kit::imagen_recurso(&ruta).map(|_| ruta));
        let cambio = tarea::critica(|| {
            let k = comp();
            let mut cambio = false;
            for ventana in &mut k.ventanas_app {
                let Some(finder) = ventana.finder.as_mut() else {
                    continue;
                };
                cambio |= finder.archivos.asignar_icono_app(&bundle, icono.clone());
                for columna in &mut finder.columnas {
                    cambio |= columna.asignar_icono_app(&bundle, icono.clone());
                }
            }
            cambio
        });
        cambio_total |= cambio;
    }

    tarea::critica(|| {
        let k = comp();
        k.resolviendo_iconos_finder = false;
        if cambio_total {
            repintar_completo(k);
        }
    });
    tarea::terminar()
}

/// Pone el modo de video de Kira: escritorio sin wallpaper y cursor.
///
/// Ya no se ejecuta dentro de una sección crítica (hace E/S de disco al
/// refrescar el escritorio), así que dos tareas podrían coincidir. Este cerrojo
/// deja que solo una ponga el modo; la otra se retira y lo reintenta después.
static REANUDANDO: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn reanudar(k: &mut Comp) {
    use core::sync::atomic::Ordering;
    if REANUDANDO.swap(true, Ordering::Acquire) {
        return;
    }
    struct Suelta;
    impl Drop for Suelta {
        fn drop(&mut self) {
            REANUDANDO.store(false, Ordering::Release);
        }
    }
    let _suelta = Suelta;
    // Antes de dejar el modo texto, guardar texto.
    crate::console::guardar_texto();
    let mi = match resolucion() {
        Some((w, h)) => gfx::poner_modo(w, h).or_else(|| gfx::poner_modo(1024, 768)),
        None => gfx::poner_modo(1024, 768),
    }
    .or_else(|| gfx::poner_modo(800, 600))
    .or_else(|| gfx::poner_modo(640, 480));
    let mi = match mi {
        Some(v) => v,
        None => return,
    };
    // Algunos BIOS dejan el PIT en one-shot al cambiar a VBE. La escena puede
    // verse aunque el planificador haya dejado de recibir IRQ0; comprobarlo
    // aquí evita confundir un framebuffer pintado con Kira ya operativa.
    if !bios::asegurar_reloj_kernel() {
        crate::println!("kira: IRQ0 sigue sin latir tras rearmar PIC/PIT.");
    }
    // Algunos firmwares vuelven a configurar el 8042 al cambiar de modo VBE.
    // Reclamamos teclado y puntero después de la última llamada BIOS, mientras
    // el cambio de modo sigue protegido contra el cambio de tarea.
    let entrada = crate::ps2::init();
    if !entrada.teclado_ok || !entrada.raton_ack {
        entrada.imprimir();
    }
    let sw = mi.width as i32;
    let sh = mi.height as i32;
    // El servidor Kira refleja el modo realmente activo: el pedido puede no
    // existir y el BIOS acabar ofreciendo otro.
    crate::kira::ajustar_pantalla(sw, sh);
    // Kira antes dibujaba cada elemento directamente al LFB: el puntero o la
    // selección dejaban visibles los pasos intermedios del repintado. Mantener
    // un buffer trasero hace que la escena aparezca completa en cada refresco.
    let pixels = gfx::buffer_pixels(&mi);
    k.backbuffer.clear();
    let buffer = if k.backbuffer.try_reserve_exact(pixels).is_ok() {
        k.backbuffer.resize(pixels, FONDO_LIMPIO);
        k.backbuffer.as_mut_ptr()
    } else {
        core::ptr::null_mut()
    };
    let mut c = Canvas::new_buffered(&mi, buffer);
    k.boton_menu_pulsado = false;
    k.menu_toby_abierto = false;
    k.menu_toby_acerca = false;
    k.menu_boton_armado = false;
    k.menu_item_armado = None;
    k.dock_app_armada = None;
    k.rebote_dock = None;
    k.escritorio.refrescar(sw, sh);
    k.marco_base = Vec::new();
    pintar_escritorio(
        &mut c,
        None,
        0,
        0,
        false,
        false,
        false,
        &k.escritorio,
        &mut k.ventanas_app,
        &mut k.marco_base,
        k.rebote_dock.as_ref(),
    );
    // El puntero del raton, ya analizado (se reutiliza si TobyUI ya lo uso).
    cursor::preparar();
    // El puntero arranca arriba a la izquierda y separado del borde
    // (`cursor::MARGEN_INICIAL`), no en el centro: es la posicion que la gente
    // espera al encender y a la que tambien sale el cursor por hardware.
    k.mx = cursor::MARGEN_INICIAL.clamp(0, sw.saturating_sub(1));
    k.my = cursor::MARGEN_INICIAL.clamp(0, sh.saturating_sub(1));
    k.botones = 0;
    k.seleccion = None;
    k.canvas = Some(c);
    k.activo = true;
    // El primer puntero pertenece al mismo fotograma que el escritorio: no
    // depender de que la tarea de fondo reciba su primer turno para hacerlo
    // visible. Esto también da una señal visual inmediata si el planificador
    // necesita recuperarse después de una llamada al BIOS.
    if let Some(canvas) = k.canvas.as_ref() {
        let ticks = crate::pit::ticks();
        let escala = escala_actual();
        pintar_puntero(
            canvas,
            &mut k.sup,
            k.mx,
            k.my,
            escala,
            k.cursor_estado,
            ticks,
        );
        k.cursor_frame = cursor::indice_fotograma(k.cursor_estado, ticks);
        k.cursor_escala = escala;
    }
}

/// Vuelve a componer escritorio y superficies de ventanas después de un cambio.
pub fn repintar_ventanas() {
    tarea::critica(|| {
        let k = comp();
        if !k.activo {
            return;
        }
        repintar_completo(k);
    });
}

/// Repinta la superficie base, las ventanas y los adornos del escritorio.
fn pintar_escritorio(
    c: &mut Canvas,
    origen: Option<(i32, i32)>,
    mx: i32,
    my: i32,
    boton_menu_pulsado: bool,
    menu_toby_abierto: bool,
    menu_toby_acerca: bool,
    escritorio: &crate::kira::desktop::Escritorio,
    ventanas_app: &mut Vec<VentanaApp>,
    marco_base: &mut Vec<u32>,
    rebote_dock: Option<&ReboteDock>,
) {
    pintar_escena_base(
        c,
        mx,
        my,
        boton_menu_pulsado,
        menu_toby_abierto,
        menu_toby_acerca,
        escritorio,
        ventanas_app,
        rebote_dock,
    );
    let Some((x0, y0)) = origen else {
        if !marco_base.is_empty() {
            *marco_base = Vec::new();
        }
        return;
    };
    if guardar_base_marco(c, marco_base) {
        pintar_seleccion(c, marco_base, x0, y0, mx, my);
    } else {
        // Sin memoria para la copia incremental, conserva un marco útil sin
        // hacer un barrido alpha de cientos de miles de píxeles por paquete.
        pintar_borde_seleccion(c, x0, y0, mx, my);
    }
    presentar_rectangulo_seleccion(c, x0, y0, mx, my);
}

/// Pinta el escritorio estable, sin el cursor ni el marco temporal.
fn pintar_escena_base(
    c: &mut Canvas,
    mx: i32,
    my: i32,
    boton_menu_pulsado: bool,
    menu_toby_abierto: bool,
    menu_toby_acerca: bool,
    escritorio: &crate::kira::desktop::Escritorio,
    ventanas_app: &mut [VentanaApp],
    rebote_dock: Option<&ReboteDock>,
) {
    pintar_fondo_escritorio(c);
    escritorio.pintar(c);
    pintar_ventanas(c, mx, my);
    let titulo_app = ventanas_app
        .iter()
        .rev()
        .find(|ventana| !ventana.minimizada)
        .map(|ventana| ventana.titulo.as_str());
    pintar_barra_menu(c, boton_menu_pulsado, mx, my, titulo_app);
    if menu_toby_abierto {
        pintar_menu_toby(c, mx, my, menu_toby_acerca);
    }
    let abiertas = aplicaciones_abiertas_dock_con_carga(ventanas_app, rebote_dock);
    let rebote = rebote_dock.map(|rebote| {
        (
            rebote.ruta.as_str(),
            rebote.reloj.offset_y_repeating(crate::pit::ticks()),
        )
    });
    crate::dock::pintar(c, &abiertas, rebote);
    let ticks = crate::pit::ticks();
    for ventana in ventanas_app
        .iter_mut()
        .filter(|ventana| !ventana.minimizada)
    {
        if ventana.fondo.len() >= ventana.pixels.len() {
            let _ = c.copiar_rect_recortado_a(
                &mut ventana.fondo,
                ventana.w as usize,
                ventana.x,
                ventana.y,
                ventana.w as i32,
                ventana.h as i32,
            );
        }
        if let Some(animacion) = ventana.genie {
            // La restauración recorre exactamente la misma malla al revés;
            // así no cambia la silueta ni aparece un salto al salir del Dock.
            let progreso = match animacion.accion {
                AccionGenie::Minimizar => animacion.reloj.progress(ticks),
                AccionGenie::Restaurar => animacion.reloj.progress_reversed(ticks),
            };
            crate::kira::genie::pintar(
                c,
                &ventana.pixels,
                ventana.w,
                ventana.h,
                animacion.origen,
                animacion.destino,
                progreso,
            );
        } else {
            blit_ventana_app(c, ventana);
        }
    }
    escritorio.pintar_menu_superpuesto(c, mx, my);
    c.present();
}

/// Pinta el fondo precalculado. El PNG solo se decodifica y filtra al cambiar
/// la resolución, no en cada movimiento del ratón o del marco de selección.
fn pintar_fondo_escritorio(c: &mut Canvas) {
    if c.width <= 0 || c.height <= 0 {
        return;
    }
    let sw = c.width;
    let sh = c.height;
    let cache = unsafe { &mut *FONDO_CACHE.get() };
    if cache.ancho != sw || cache.alto != sh {
        // Marcar primero el intento evita volver a consultar un recurso
        // ausente en cada repintado. Si falla, el respaldo negro permanece.
        cache.ancho = sw;
        cache.alto = sh;
        cache.pixeles.clear();
        let Some(total) = (sw as usize).checked_mul(sh as usize) else {
            return;
        };
        if cache.pixeles.try_reserve_exact(total).is_err() {
            return;
        }
        cache.pixeles.resize(total, FONDO_LIMPIO);

        let Ok(img) = crate::toby_ui::resource::cargar_png(FONDO_ESCRITORIO) else {
            cache.pixeles.clear();
            return;
        };
        if img.ancho == 0 || img.alto == 0 {
            cache.pixeles.clear();
            return;
        }

        let (sw64, sh64) = (sw as i64, sh as i64);
        let (iw, ih) = (img.ancho as i64, img.alto as i64);
        // Ajuste "cover": toma el eje que necesita mayor aumento y centra el
        // sobrante fuera de pantalla, como un fondo de escritorio convencional.
        let (w, h) = if sw64 * ih >= sh64 * iw {
            (sw64, (sw64 * ih + iw - 1) / iw)
        } else {
            ((sh64 * iw + ih - 1) / ih, sh64)
        };
        let destino = Rect {
            x: ((sw64 - w) / 2) as i32,
            y: ((sh64 - h) / 2) as i32,
            w: w as u32,
            h: h as u32,
        };
        {
            let mut fondo = Canvas::from_buffer(cache.pixeles.as_mut_ptr(), sw, sh, sw as usize);
            let mut r = CanvasRenderer::nuevo(&mut fondo, None);
            r.recorte(Rect {
                x: 0,
                y: 0,
                w: sw as u32,
                h: sh as u32,
            });
            r.imagen(destino, &img);
            r.recorte_fin();
        }
    }
    if cache.pixeles.len() == (sw as usize).saturating_mul(sh as usize) {
        c.blit(cache.pixeles.as_ptr(), sw as usize, 0, 0, sw, sh);
    } else {
        c.fill_rect(0, 0, sw, sh, FONDO_LIMPIO);
    }
}

/// Barra superior del escritorio; no depende de cargar una imagen de fondo.
fn pintar_barra_menu(
    c: &mut Canvas,
    boton_menu_pulsado: bool,
    mx: i32,
    my: i32,
    titulo_app: Option<&str>,
) {
    let alto = ALTO_MENU.min(c.height).max(0);
    let ancho = c.width.max(0);
    let fuente = fuentes::para_kira();
    let fuente_suave = fuentes::para_barra_menu();
    let tinta = gfx::rgb(48, 50, 54);
    let icono = crate::toby_ui::kit::imagen_recurso(if boton_menu_pulsado {
        ICONO_TOBY_PULSADO
    } else {
        ICONO_TOBY
    });
    let inicio_menu = if icono.is_some() { MENU_TEXTO_X } else { 76 };
    {
        let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
        let barra = Rect {
            x: 0,
            y: 0,
            w: ancho as u32,
            h: alto as u32,
        };
        let borde = crate::toby_ui::theme::TEMA_POR_DEFECTO.paleta.borde;
        renderer::pintar_textura_cabecera(&mut r, barra, borde);
        // Redondea solo las esquinas superiores; la silueta deja la base recta.
        renderer::pintar_silueta_ventana(&mut r, barra, FONDO_LIMPIO, borde);
        if let Some(icono) = icono {
            r.imagen(
                Rect {
                    x: BOTON_MENU_X,
                    y: BOTON_MENU_Y,
                    w: BOTON_MENU_ANCHO as u32,
                    h: BOTON_MENU_ALTO as u32,
                },
                icono,
            );
        } else {
            // Respaldo si aún no se ha instalado el bundle o falta el recurso.
            r.texto(16, MENU_TEXTO_Y, "Rocky", tinta, 1);
        }
    }

    let etiquetas = ["Archivo", "Editar", "Ventana", "Ayuda"];
    let titulo_app = titulo_app.filter(|nombre| !nombre.trim().is_empty());
    let espacio = crate::kira::desktop::medir_texto("  ", fuente_suave.as_ref());
    let mut siguiente_x = inicio_menu;
    if let Some(nombre) = titulo_app {
        // Reserva el píxel extra del refuerzo de peso aplicado solo al nombre.
        siguiente_x +=
            crate::kira::desktop::medir_texto(nombre, fuente_suave.as_ref()) + 1 + espacio;
    }
    let mut posiciones = [0; 4];
    for (i, etiqueta) in etiquetas.iter().enumerate() {
        posiciones[i] = siguiente_x;
        siguiente_x += crate::kira::desktop::medir_texto(etiqueta, fuente_suave.as_ref()) + espacio;
    }
    let azul = gfx::rgb(49, 119, 231);
    let mut activos = [false; 4];
    for (i, etiqueta) in etiquetas.iter().enumerate() {
        let x = posiciones[i];
        let ancho_texto = crate::kira::desktop::medir_texto(etiqueta, fuente_suave.as_ref());
        let hit = Rect {
            x: x - 6,
            y: 3,
            w: (ancho_texto + 12).max(12) as u32,
            h: 22,
        };
        if hit.contiene(mx, my) {
            crate::kira::desktop::pintar_superficie_redondeada(c, hit, 7, azul);
            activos[i] = true;
        }
    }

    let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
    if let Some(nombre) = titulo_app {
        if let Some(fuente) = fuente_suave.as_ref() {
            r.texto_suavizado(inicio_menu, MENU_TEXTO_Y, nombre, tinta, fuente);
            // Simula un peso semibold con un trazo contiguo, reutilizando la
            // caché SF ya cargada y sin reservar otra fuente completa.
            r.texto_suavizado(inicio_menu + 1, MENU_TEXTO_Y, nombre, tinta, fuente);
        } else {
            r.texto(inicio_menu, MENU_TEXTO_Y, nombre, tinta, 1);
            r.texto(inicio_menu + 1, MENU_TEXTO_Y, nombre, tinta, 1);
        }
    }
    for (i, etiqueta) in etiquetas.iter().enumerate() {
        let x = posiciones[i];
        let color = if activos[i] {
            gfx::rgb(255, 255, 255)
        } else {
            tinta
        };
        if let Some(fuente) = fuente_suave.as_ref() {
            r.texto_suavizado(x, MENU_TEXTO_Y, etiqueta, color, fuente);
        } else {
            r.texto(x, MENU_TEXTO_Y, etiqueta, color, 1);
        }
    }
}

/// Comprueba si el ratón está sobre el botón de Toby de la barra.
fn dentro_boton_menu(x: i32, y: i32) -> bool {
    x >= BOTON_MENU_X
        && x < BOTON_MENU_X + BOTON_MENU_ANCHO
        && y >= BOTON_MENU_Y
        && y < BOTON_MENU_Y + BOTON_MENU_ALTO
}

fn dentro_menu_toby(x: i32, y: i32, _acerca: bool) -> bool {
    Rect {
        x: MENU_TOBY_X,
        y: MENU_TOBY_Y,
        w: MENU_TOBY_ANCHO,
        h: MENU_TOBY_ALTO,
    }
    .contiene(x, y)
}

fn fila_menu_toby(indice: usize) -> Option<Rect> {
    let mut y = MENU_TOBY_Y + MENU_ITEM_MARGEN;
    for (i, fila) in FILAS_MENU_TOBY.iter().enumerate() {
        match fila {
            FilaMenuToby::Opcion(_) => {
                if i == indice {
                    return Some(Rect {
                        x: MENU_TOBY_X + MENU_ITEM_MARGEN,
                        y,
                        w: MENU_TOBY_ANCHO - (MENU_ITEM_MARGEN * 2) as u32,
                        h: MENU_ITEM_ALTO as u32,
                    });
                }
                y += MENU_ITEM_ALTO + 1;
            }
            FilaMenuToby::Separador => y += MENU_SEPARADOR_ALTO,
        }
    }
    None
}

fn indice_opcion_menu_toby(x: i32, y: i32, _acerca: bool) -> Option<u8> {
    FILAS_MENU_TOBY
        .iter()
        .enumerate()
        .find_map(|(indice, fila)| match fila {
            FilaMenuToby::Opcion(_) => fila_menu_toby(indice)
                .filter(|rect| rect.contiene(x, y))
                .map(|_| indice as u8),
            FilaMenuToby::Separador => None,
        })
}

/// Menú principal de Toby Desktop, con acciones de energía en sus últimas filas.
fn pintar_menu_toby(c: &mut Canvas, mx: i32, my: i32, _acerca: bool) {
    let fuente = fuentes::para_kira();
    let fuente_suave = fuentes::para_barra_menu();
    let panel = Rect {
        x: MENU_TOBY_X,
        y: MENU_TOBY_Y,
        w: MENU_TOBY_ANCHO,
        h: MENU_TOBY_ALTO,
    };
    crate::kira::desktop::pintar_panel_redondeado(
        c,
        panel,
        9,
        gfx::rgb(250, 250, 252),
        gfx::rgb(145, 148, 154),
    );

    let tinta = gfx::rgb(35, 37, 40);
    let gris_separador = gfx::rgb(207, 209, 213);
    let mut y = MENU_TOBY_Y + MENU_ITEM_MARGEN;
    for (indice, entrada) in FILAS_MENU_TOBY.iter().enumerate() {
        match entrada {
            FilaMenuToby::Opcion(etiqueta) => {
                let fila = fila_menu_toby(indice).expect("fila de menú válida");
                let sobre_fila = fila.contiene(mx, my);
                if sobre_fila {
                    crate::kira::desktop::pintar_superficie_redondeada(
                        c,
                        fila,
                        5,
                        gfx::rgb(48, 115, 230),
                    );
                }
                let color = if sobre_fila {
                    gfx::rgb(255, 255, 255)
                } else {
                    tinta
                };
                let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
                if let Some(fuente) = fuente_suave.as_ref() {
                    r.texto_suavizado(fila.x + 8, fila.y + 2, etiqueta, color, fuente);
                } else {
                    r.texto(fila.x + 8, fila.y + 2, etiqueta, color, 1);
                }
                y += MENU_ITEM_ALTO + 1;
            }
            FilaMenuToby::Separador => {
                c.fill_rect(
                    panel.x + 8,
                    y + MENU_SEPARADOR_ALTO / 2,
                    panel.w as i32 - 16,
                    1,
                    gris_separador,
                );
                y += MENU_SEPARADOR_ALTO;
            }
        }
    }
}

fn rectangulo_seleccion(c: &Canvas, ax: i32, ay: i32, bx: i32, by: i32) -> Rect {
    if c.width <= 0 || c.height <= ALTO_MENU {
        return Rect::VACIO;
    }
    let x0 = ax.min(bx).clamp(0, c.width - 1);
    let x1 = ax.max(bx).clamp(0, c.width - 1);
    let y0 = ay.min(by).max(ALTO_MENU).clamp(0, c.height - 1);
    let y1 = ay.max(by).clamp(0, c.height - 1);
    if x0 > x1 || y0 > y1 {
        return Rect::VACIO;
    }
    Rect {
        x: x0,
        y: y0,
        w: (x1 - x0 + 1) as u32,
        h: (y1 - y0 + 1) as u32,
    }
}

/// Marco de selección al arrastrar con el botón izquierdo, estilo Finder.
fn pintar_seleccion(c: &Canvas, base: &[u32], ax: i32, ay: i32, bx: i32, by: i32) {
    let rect = rectangulo_seleccion(c, ax, ay, bx, by);
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let interior = Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        w: rect.w.saturating_sub(2),
        h: rect.h.saturating_sub(2),
    };
    if interior.w > 0 && interior.h > 0 {
        mezclar_region_seleccion(c, base, c.width.max(0) as usize, interior);
    }
    pintar_borde_rectangulo(c, rect);
}

fn pintar_borde_seleccion(c: &Canvas, ax: i32, ay: i32, bx: i32, by: i32) {
    pintar_borde_rectangulo(c, rectangulo_seleccion(c, ax, ay, bx, by));
}

fn pintar_borde_rectangulo(c: &Canvas, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let x0 = rect.x;
    let x1 = rect.der() - 1;
    let y0 = rect.y;
    let y1 = rect.abajo() - 1;
    let claro = gfx::rgb(210, 228, 255);
    let oscuro = gfx::rgb(48, 132, 255);
    for x in x0..=x1 {
        let color = if ((x - x0) / 5) % 2 == 0 {
            claro
        } else {
            oscuro
        };
        c.put(x, y0, color);
        c.put(x, y1, color);
    }
    for y in y0..=y1 {
        let color = if ((y - y0) / 5) % 2 == 0 {
            claro
        } else {
            oscuro
        };
        c.put(x0, y, color);
        c.put(x1, y, color);
    }
}

fn presentar_rectangulo_seleccion(c: &Canvas, ax: i32, ay: i32, bx: i32, by: i32) {
    let rect = rectangulo_seleccion(c, ax, ay, bx, by);
    if rect.w > 0 && rect.h > 0 {
        c.present_rect(rect.x, rect.y, rect.w as i32, rect.h as i32);
    }
}

fn guardar_base_marco(c: &Canvas, base: &mut Vec<u32>) -> bool {
    let (ancho, alto) = (c.width.max(0) as usize, c.height.max(0) as usize);
    let Some(total) = ancho.checked_mul(alto) else {
        base.clear();
        return false;
    };
    base.clear();
    if base.try_reserve_exact(total).is_err() {
        return false;
    }
    base.resize(total, FONDO_LIMPIO);
    if !c.copiar_rect_a(base, ancho, 0, 0, ancho as i32, alto as i32) {
        base.clear();
        return false;
    }
    true
}

fn color_relleno_seleccion(fondo: u32) -> u32 {
    const ALFA: u32 = 42;
    let inversa = 256 - ALFA;
    let rojo = (fondo >> 16) & 0xFF;
    let verde = (fondo >> 8) & 0xFF;
    let azul = fondo & 0xFF;
    let nr = (48 * ALFA + rojo * inversa) >> 8;
    let ng = (132 * ALFA + verde * inversa) >> 8;
    let nb = (255 * ALFA + azul * inversa) >> 8;
    (nr << 16) | (ng << 8) | nb
}

/// Compone el relleno desde el fotograma limpio, nunca desde el azul ya pintado.
fn mezclar_region_seleccion(c: &Canvas, base: &[u32], stride: usize, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let ancho = c.width.max(0) as usize;
    let alto = c.height.max(0) as usize;
    if stride < ancho || base.len() < stride.saturating_mul(alto) {
        return;
    }
    for y in rect.y..rect.abajo() {
        for x in rect.x..rect.der() {
            if x < 0 || y < 0 || x as usize >= ancho || y as usize >= alto {
                continue;
            }
            let fondo = base[y as usize * stride + x as usize];
            c.put(x, y, color_relleno_seleccion(fondo));
        }
    }
}

/// Particiona `a \\ b` en hasta cuatro rectángulos disjuntos.
fn diferencia_rectangulos(a: Rect, b: Rect) -> ([Rect; 4], usize) {
    let mut partes = [Rect::VACIO; 4];
    if a.w == 0 || a.h == 0 {
        return (partes, 0);
    }
    let inter = a.recortar(b);
    if inter.w == 0 || inter.h == 0 {
        partes[0] = a;
        return (partes, 1);
    }
    let candidatas = [
        Rect {
            x: a.x,
            y: a.y,
            w: a.w,
            h: (inter.y - a.y).max(0) as u32,
        },
        Rect {
            x: a.x,
            y: inter.abajo(),
            w: a.w,
            h: (a.abajo() - inter.abajo()).max(0) as u32,
        },
        Rect {
            x: a.x,
            y: inter.y,
            w: (inter.x - a.x).max(0) as u32,
            h: inter.h,
        },
        Rect {
            x: inter.der(),
            y: inter.y,
            w: (a.der() - inter.der()).max(0) as u32,
            h: inter.h,
        },
    ];
    let mut n = 0;
    for rect in candidatas {
        if rect.w > 0 && rect.h > 0 {
            partes[n] = rect;
            n += 1;
        }
    }
    (partes, n)
}

fn bordes_rectangulo(rect: Rect) -> [Rect; 4] {
    if rect.w == 0 || rect.h == 0 {
        return [Rect::VACIO; 4];
    }
    let x1 = rect.der() - 1;
    let y1 = rect.abajo() - 1;
    [
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: 1,
        },
        if rect.h > 1 {
            Rect {
                x: rect.x,
                y: y1,
                w: rect.w,
                h: 1,
            }
        } else {
            Rect::VACIO
        },
        Rect {
            x: rect.x,
            y: rect.y + 1,
            w: 1,
            h: rect.h.saturating_sub(2),
        },
        if rect.w > 1 {
            Rect {
                x: x1,
                y: rect.y + 1,
                w: 1,
                h: rect.h.saturating_sub(2),
            }
        } else {
            Rect::VACIO
        },
    ]
}

fn restaurar_region(c: &Canvas, base: &[u32], stride: usize, rect: Rect) {
    if rect.w > 0 && rect.h > 0 && rect.x >= 0 && rect.y >= 0 {
        // `Canvas::blit` interpreta `src` como el primer pixel del rectangulo
        // origen, no como una superficie completa. Desplazarlo a (x, y)
        // evita copiar por error la esquina superior izquierda del escritorio
        // al limpiar cada posicion anterior del marco.
        let inicio = rect.y as usize * stride + rect.x as usize;
        let origen = unsafe { base.as_ptr().add(inicio) };
        c.blit(origen, stride, rect.x, rect.y, rect.w as i32, rect.h as i32);
    }
}

/// Actualiza el marco cambiando solo las regiones afectadas entre fotogramas.
fn actualizar_seleccion_incremental(
    c: &Canvas,
    base: &[u32],
    ax: i32,
    ay: i32,
    anterior_x: i32,
    anterior_y: i32,
    mx: i32,
    my: i32,
    dano_cursor: Option<(i32, i32, i32, i32)>,
) -> bool {
    let stride = c.width.max(0) as usize;
    let Some(total) = stride.checked_mul(c.height.max(0) as usize) else {
        return false;
    };
    if base.len() != total {
        return false;
    }
    let anterior = rectangulo_seleccion(c, ax, ay, anterior_x, anterior_y);
    let actual = rectangulo_seleccion(c, ax, ay, mx, my);
    if anterior == actual {
        if let Some((x, y, w, h)) = dano_cursor {
            c.present_rect(x, y, w, h);
        }
        return true;
    }
    let (restaurar, n_restaurar) = diferencia_rectangulos(anterior, actual);
    let (rellenar, n_rellenar) = diferencia_rectangulos(actual, anterior);
    let bordes_anteriores = bordes_rectangulo(anterior);

    for rect in &restaurar[..n_restaurar] {
        restaurar_region(c, base, stride, *rect);
    }
    for borde in bordes_anteriores {
        if borde.w > 0 && borde.h > 0 {
            restaurar_region(c, base, stride, borde);
        }
    }
    for rect in &rellenar[..n_rellenar] {
        mezclar_region_seleccion(c, base, stride, *rect);
    }
    for borde in bordes_anteriores {
        mezclar_region_seleccion(c, base, stride, borde.recortar(actual));
    }
    pintar_borde_rectangulo(c, actual);

    // Volcar solo las partes que cambiaron. Presentar la caja que engloba el
    // marco anterior y el nuevo copiaba tambien todo el interior intacto; y
    // esperar aqui al retrazo vertical limitaba cada movimiento a ~60 Hz.
    for rect in &restaurar[..n_restaurar] {
        presentar_rect(c, *rect);
    }
    for borde in bordes_anteriores {
        presentar_rect(c, borde);
        presentar_rect(c, borde.recortar(actual));
    }
    for rect in &rellenar[..n_rellenar] {
        presentar_rect(c, *rect);
    }
    for borde in bordes_rectangulo(actual) {
        presentar_rect(c, borde);
    }
    if let Some((x, y, w, h)) = dano_cursor {
        c.present_rect(x, y, w, h);
    }
    true
}

#[inline]
fn presentar_rect(c: &Canvas, rect: Rect) {
    if rect.w > 0 && rect.h > 0 {
        c.present_rect(rect.x, rect.y, rect.w as i32, rect.h as i32);
    }
}

fn sobre_control_ventana(mx: i32, my: i32) -> bool {
    let estado = crate::kira::opcion();
    let st = match estado.as_ref() {
        Some(st) => st,
        None => return false,
    };
    let root = st.root_wid;
    st.conns.iter().any(|conn| {
        !conn.closed
            && conn.windows.iter().any(|w| {
                if !w.mapped || w.id == root || w.parent != root {
                    return false;
                }
                let rect = Rect {
                    x: w.x as i32,
                    y: w.y as i32,
                    w: w.w as u32,
                    h: (w.h as u32).saturating_add(renderer::ALTO_BARRA_VENTANA * 2),
                };
                renderer::control_ventana_en(rect, mx, my).is_some()
            })
    })
}

fn pintar_ventanas(c: &mut Canvas, mx: i32, my: i32) {
    let estado = crate::kira::opcion();
    let st = match estado.as_ref() {
        Some(st) => st,
        None => return,
    };
    let root = st.root_wid;
    let total = st
        .conns
        .iter()
        .flat_map(|conn| conn.windows.iter())
        .filter(|w| w.mapped && w.id != root && w.parent == root)
        .count();
    let mut ventanas = Vec::new();
    if ventanas.try_reserve_exact(total).is_err() {
        for conn in &st.conns {
            for w in &conn.windows {
                if w.mapped && w.id != root && w.parent == root {
                    pintar_ventana(c, conn, w, mx, my);
                }
            }
        }
        return;
    }
    for conn in &st.conns {
        if conn.closed {
            continue;
        }
        for w in &conn.windows {
            if !w.mapped || w.id == root || w.parent != root {
                continue;
            }
            ventanas.push((conn, w));
        }
    }
    ventanas.sort_unstable_by_key(|(_, w)| w.z_order);
    for (conn, ventana) in ventanas {
        pintar_ventana(c, conn, ventana, mx, my);
    }
}

fn pintar_ventana(c: &mut Canvas, conn: &crate::kira::conn::Conn, w: &Window, mx: i32, my: i32) {
    let alto_barra = renderer::ALTO_BARRA_VENTANA;
    let rect = Rect {
        x: w.x as i32,
        y: w.y as i32,
        w: w.w as u32,
        h: (w.h as u32).saturating_add(alto_barra.saturating_mul(2)),
    };
    let cuerpo = Rect {
        x: rect.x,
        y: rect.y + alto_barra as i32,
        w: rect.w,
        h: w.h as u32,
    };
    let tema = &crate::toby_ui::theme::TEMA_POR_DEFECTO;
    c.fill_rect(
        cuerpo.x,
        cuerpo.y,
        cuerpo.w as i32,
        cuerpo.h as i32,
        tema.paleta.fondo,
    );
    c.blit(
        w.pixels.as_ptr(),
        w.w as usize,
        rect.x,
        cuerpo.y,
        w.w as i32,
        w.h as i32,
    );
    let recorte = (cuerpo.x, cuerpo.y, cuerpo.w as i32, cuerpo.h as i32);
    pintar_hijas(c, conn, w.id, cuerpo.x, cuerpo.y, recorte);
    let fuente = fuentes::para_kira();
    let mut r = CanvasRenderer::nuevo(c, fuente.as_ref());
    renderer::pintar_cabecera_ventana_con_cursor(
        &mut r,
        rect,
        AZUL_CABECERA,
        tema.paleta.borde,
        "Kira",
        Some((mx, my)),
    );
    renderer::pintar_pie_ventana(&mut r, rect, tema.paleta.borde);
}

fn pintar_hijas(
    c: &mut Canvas,
    conn: &crate::kira::conn::Conn,
    padre: u32,
    origen_x: i32,
    origen_y: i32,
    clip: (i32, i32, i32, i32),
) {
    let mut hijas: Vec<&Window> = Vec::new();
    let cantidad = conn
        .windows
        .iter()
        .filter(|w| w.mapped && w.parent == padre && !w.pixels.is_empty())
        .count();
    if hijas.try_reserve_exact(cantidad).is_ok() {
        hijas.extend(
            conn.windows
                .iter()
                .filter(|w| w.mapped && w.parent == padre && !w.pixels.is_empty()),
        );
        hijas.sort_unstable_by_key(|w| w.z_order);
    } else {
        for hija in conn
            .windows
            .iter()
            .filter(|w| w.mapped && w.parent == padre && !w.pixels.is_empty())
        {
            pintar_hija(c, conn, hija, origen_x, origen_y, clip);
        }
        return;
    }
    for hija in hijas {
        pintar_hija(c, conn, hija, origen_x, origen_y, clip);
    }
}

fn pintar_hija(
    c: &mut Canvas,
    conn: &crate::kira::conn::Conn,
    hija: &Window,
    origen_x: i32,
    origen_y: i32,
    clip: (i32, i32, i32, i32),
) {
    let x = origen_x + hija.x as i32;
    let y = origen_y + hija.y as i32;
    c.blit_clipped(
        hija.pixels.as_ptr(),
        hija.w as usize,
        x,
        y,
        hija.w as i32,
        hija.h as i32,
        clip.0,
        clip.1,
        clip.2,
        clip.3,
    );
    let x0 = x.max(clip.0);
    let y0 = y.max(clip.1);
    let x1 = (x + hija.w as i32).min(clip.0 + clip.2);
    let y1 = (y + hija.h as i32).min(clip.1 + clip.3);
    if x0 < x1 && y0 < y1 {
        pintar_hijas(c, conn, hija.id, x, y, (x0, y0, x1 - x0, y1 - y0));
    }
}

/// Pinta el puntero en su posicion actual, eligiendo el camino segun la escala
/// pedida con [`fijar_escala_puntero`].
///
/// Escala 1 (la normal) usa el pintado pixel a pixel de siempre, que es el mas
/// rapido y el que no toca nada. Escala mayor reescala el dibujo con el mismo
/// filtro del resto del sistema ([`Superposicion::pintar_escalado`]), que es lo
/// que quiera quien necesite un puntero mas grande (pantallas densas o
/// accesibilidad). La llama el compositor dentro de su seccion critica.
fn pintar_puntero(
    c: &Canvas,
    sup: &mut Superposicion,
    mx: i32,
    my: i32,
    escala: i32,
    estado: cursor::Estado,
    ticks: u64,
) {
    match (estado, escala <= 1) {
        (cursor::Estado::Flecha, true) => sup.pintar(c, mx, my),
        (cursor::Estado::Flecha, false) => sup.pintar_escalado(c, mx, my, escala),
        _ => sup.pintar_estado(c, mx, my, escala, estado, ticks),
    }
}

/// Fija la escala del puntero del raton (1 = tamano natural, 2 = doble...).
///
/// No repinta nada aqui: el compositor usara el nuevo tamano en su proxima
/// vuelta. Devuelve la escala realmente quedada (se limita a un tope razonable
/// para que un puntero gigante no tape el escritorio entero).
pub fn fijar_escala_puntero(escala: i32) -> i32 {
    let e = escala.clamp(1, 8);
    unsafe { *ESCALA.get() = e };
    e
}

/// Escala actual del puntero del raton.
pub fn escala_puntero() -> i32 {
    unsafe { *ESCALA.get() }
}

/// Vuelve a pintar el puntero donde esta, con la escala pedida.
///
/// Lo llama el interprete tras cambiar la escala (`gui puntero`), para que el
/// cambio se vea al instante en vez de esperar a que se mueva el raton. Si no
/// hay escritorio activo no hace nada.
pub fn repintar_puntero() {
    tarea::critica(|| {
        let k = comp();
        if !k.activo {
            return;
        }
        if let Some(c) = k.canvas.as_ref() {
            let ticks = crate::pit::ticks();
            let escala = escala_actual();
            pintar_puntero(c, &mut k.sup, k.mx, k.my, escala, k.cursor_estado, ticks);
            k.cursor_frame = cursor::indice_fotograma(k.cursor_estado, ticks);
            k.cursor_escala = escala;
        }
    });
}

/// Cambia el cursor del escritorio y repinta inmediatamente si Kira está visible.
/// Los clientes pueden elegir un estado contextual; `Pensando` rota su sprite
/// con los ticks del PIT aunque el ratón permanezca quieto.
pub fn fijar_estado_cursor(estado: cursor::Estado) {
    tarea::critica(|| {
        let k = comp();
        k.cursor_estado = estado;
        if let Some(c) = k.canvas.as_ref() {
            if k.activo {
                let ticks = crate::pit::ticks();
                let escala = escala_actual();
                pintar_puntero(c, &mut k.sup, k.mx, k.my, escala, estado, ticks);
                k.cursor_frame = cursor::indice_fotograma(estado, ticks);
                k.cursor_escala = escala;
            }
        }
    });
}

pub fn estado_cursor() -> cursor::Estado {
    tarea::critica(|| comp().cursor_estado)
}

/// Pinta una imagen escalada sobre el escritorio de Kira, por encima del fondo
/// azul y por debajo del puntero.
///
/// Es el mismo muestreo [`crate::toby_ui::renderer::muestra`] que usan las
/// pieles de TobyUI y el visor de imagenes: al reducir promedia el area que le
/// toca a cada pixel, al ampliar interpola bilinealmente y a tamano 1:1 copia
/// el pixel exacto, con el alfa premultiplicado para no ensuciar los bordes.
/// Solo enteros y como mucho 4 muestras por eje, asi que no se nota al repintar.
///
/// La imagen va en formato AARRGGBB (el byte alto es el alfa). Devuelve `false`
/// si ahora mismo no hay escritorio (Kira no tiene el video): no pinta nada.
/// Es el camino para las imagenes del escritorio: el puntero se repinta solo,
/// en su vuelta, por encima.
pub fn pintar_imagen_escalada(
    pixeles: &[u32],
    sw: i32,
    sh: i32,
    x: i32,
    y: i32,
    dw: i32,
    dh: i32,
) -> bool {
    if sw <= 0 || sh <= 0 || dw <= 0 || dh <= 0 {
        return false;
    }
    tarea::critica(|| {
        let k = comp();
        if !k.activo {
            return false;
        }
        let c = match k.canvas.as_ref() {
            Some(c) => c,
            None => return false,
        };
        // La imagen se compone sobre el fondo y por debajo del puntero: se
        // aparta este y se vuelve a pintar encima al terminar.
        k.sup.ocultar(c);
        for dy in 0..dh {
            let yy = y + dy;
            if yy < 0 || yy >= c.height {
                continue;
            }
            for dx in 0..dw {
                let xx = x + dx;
                if xx < 0 || xx >= c.width {
                    continue;
                }
                let (color, a) = crate::toby_ui::renderer::muestra(pixeles, sw, sh, dw, dh, dx, dy);
                if a == 0 {
                    continue;
                }
                if a >= 255 {
                    c.put(xx, yy, color);
                } else {
                    c.blend(xx, yy, color, a);
                }
            }
        }
        c.present_rect(x, y, dw, dh);
        let ticks = crate::pit::ticks();
        let escala = escala_actual();
        pintar_puntero(c, &mut k.sup, k.mx, k.my, escala, k.cursor_estado, ticks);
        k.cursor_frame = cursor::indice_fotograma(k.cursor_estado, ticks);
        k.cursor_escala = escala;
        true
    })
}
