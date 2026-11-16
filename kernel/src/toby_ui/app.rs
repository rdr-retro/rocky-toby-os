//! Aplicación Toby UI (Fase 8): une todas las piezas del lenguaje.
//!
//! Cargar → validar → disponer → pintar → interactuar → ejecutar acciones.
//! Este módulo no añade reglas nuevas al lenguaje: sólo encadena parser,
//! validation, layout, renderer, event y action, y guarda el estado que debe
//! sobrevivir entre fotogramas ([`Sesion`], el puntero y los botones).
//!
//! El anfitrión decide de dónde sale el lienzo: cualquier `gfx::Canvas` sirve,
//! tanto el framebuffer completo como el búfer de una ventana del escritorio.
//!
//! Flujo de un fotograma:
//!
//! ```text
//! App::fotograma(lienzo, fuente, ctx)
//!   ├─ event::sondear          (hardware: ratón y teclado)
//!   ├─ App::procesar           (puro: sesión + clic + acción)
//!   └─ App::pintar             (renderer sobre el lienzo)
//! ```
//!
//! Para pruebas o para un anfitrión que ya tiene sus propios eventos, `pintar`,
//! `procesar` y `sondear` se pueden llamar por separado: todo salvo `sondear`
//! es puro.
//!
//! Cómo lo engancha el anfitrión del kernel (ver `host.rs`):
//!
//! ```text
//! // 1. Un lienzo sobre el framebuffer del modo VESA activo.
//! let mut lienzo = Canvas::new(&mi);                          // gfx.rs
//! // 2. La aplicación, con su registro de acciones.
//! let mut app = App::desde_fs(ruta, mi.width, mi.height, demo::registro())?;
//! // 3. Bucle: entrada, acciones, pintura (hasta que se pulse ESC).
//! loop {
//!     let eventos = app.sondear();
//!     let paso = app.atender_eventos(&eventos, &mut ctx);     // hardware
//!     if paso.repintar() || primera_vez {
//!         app.pintar(&mut lienzo, bios::get_font().as_ref()); // bios.rs
//!     }
//!     if app.cerrada() { break; }
//! }
//! ```
//!
//! La aplicación ocupa la pantalla entera: las coordenadas del ratón coinciden
//! con las del lienzo sin más traducción.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::bios::Font;
use crate::gfx::Canvas;
use toby_appkit::animation::{Effect, TransitionConfig, BOUNCE_DURATION_MS, GENIE_DURATION_MS};
use toby_appkit::genie::Rect as GenieRect;

use super::action::{ErrorAccion, Registro, Resultado};
use super::document::ValidatedDocument;
use super::event::{self, Boton, Cambios, Entrada, Evento, Interaccion, Sesion, Tecla};
use super::layout::{self, Disposicion};
use super::parser::{self, ParserError};
use super::renderer::{self, CanvasRenderer, FuenteSuave, Renderer};
use super::resource::{self, ErrorResource};
use super::theme::{Estado, Tema, TEMA_POR_DEFECTO};
use super::validation::{self, ValidationError};
use super::valores::{self, Valores};

/// Longitud máxima del texto de un `<field>`, en caracteres.
///
/// El lenguaje no tiene límites de memoria porque no tiene memoria: esto es
/// sólo para que un campo no crezca sin control mientras se teclea.
const MAX_TEXTO_CAMPO: usize = 64;

/// Geometría precalculada de un control animado. El repintado periódico usa
/// estas entradas directamente, sin recorrer el árbol ni reservar memoria.
#[derive(Clone, Copy, Debug)]
struct ProgresoAnimado {
    rect: layout::Rect,
    clip: layout::Rect,
    indice: usize,
    determinado: bool,
}

fn recopilar_progresos(
    doc: &ValidatedDocument,
    disp: &Disposicion,
    ancho: u32,
    alto: u32,
) -> Vec<ProgresoAnimado> {
    let pre = doc.preorden();
    let capacidad = disp
        .nodos
        .iter()
        .filter(|n| n.def.etiqueta == "progress")
        .count();
    let mut progresos = Vec::new();
    if progresos.try_reserve_exact(capacidad).is_err() {
        return progresos;
    }
    let lienzo = layout::Rect {
        x: 0,
        y: 0,
        w: ancho,
        h: alto,
    };
    for (indice, colocado) in disp.nodos.iter().enumerate() {
        if colocado.def.etiqueta != "progress" {
            continue;
        }
        let Some(nodo) = pre.get(indice).copied() else {
            continue;
        };
        let determinado = nodo.texto("mode", "indeterminate") == "determinate";
        let rect = colocado.rect;
        if rect.w <= 2 || rect.h <= 2 {
            continue;
        }
        let mut visible = true;
        let mut clip = lienzo;
        let mut ancestro = Some(indice);
        while let Some(i) = ancestro {
            let Some(nodo) = pre.get(i).copied() else {
                visible = false;
                break;
            };
            if !nodo.booleano("visible", true) {
                visible = false;
                break;
            }
            if i != indice
                && (nodo.def.etiqueta == "window"
                    || nodo.def.etiqueta == "box"
                    || nodo.prop("width").is_some()
                    || nodo.prop("height").is_some())
            {
                clip = clip.recortar(disp.rect(i));
            }
            ancestro = disp.nodos.get(i).and_then(|n| n.indice_padre);
        }
        let clip = rect.recortar(clip);
        if visible && clip.w != 0 && clip.h != 0 {
            progresos.push(ProgresoAnimado {
                rect,
                clip,
                indice,
                determinado,
            });
        }
    }
    progresos
}

/// Qué ha impedido montar la aplicación.
#[derive(Clone, Debug)]
pub enum ErrorApp {
    /// El texto no es un documento Toby UI bien formado.
    Sintaxis(ParserError),
    /// Es sintácticamente correcto pero incumple las reglas del lenguaje.
    Validacion(Vec<ValidationError>),
    /// No se pudo leer el fichero del documento.
    Recurso(ErrorResource),
}

impl ErrorApp {
    /// Resumen corto, en español, para enseñar al usuario.
    pub fn resumen(&self) -> String {
        match self {
            ErrorApp::Sintaxis(e) => e.to_string(),
            ErrorApp::Validacion(es) => {
                let mut s = String::from("el documento no es válido");
                for e in es.iter().take(3) {
                    s.push_str("; ");
                    s.push_str(&e.to_string());
                }
                if es.len() > 3 {
                    s.push_str("; ...");
                }
                s
            }
            ErrorApp::Recurso(e) => String::from(e.descripcion()),
        }
    }
}

impl core::fmt::Display for ErrorApp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.resumen())
    }
}

/// Lo ocurrido al atender uno o varios eventos.
#[derive(Clone, Debug, Default)]
pub struct Paso {
    /// Qué ha cambiado (para decidir si hay que repintar).
    pub cambios: Cambios,
    /// Resultado de la última acción ejecutada en este paso.
    pub accion: Option<Resultado>,
    /// Error controlado de la última acción que no se pudo ejecutar.
    pub error: Option<ErrorAccion>,
}

impl Paso {
    /// ¿Merece la pena repintar?
    pub fn repintar(&self) -> bool {
        self.cambios.algo()
    }
}

/// Una aplicación Toby UI en marcha.
///
/// `C` es el estado del anfitrión: lo único que ven los manejadores de
/// acciones. La aplicación no interpreta nada del documento por su cuenta.
pub struct App<C> {
    doc: ValidatedDocument,
    disp: Disposicion,
    sesion: Sesion,
    entrada: Entrada,
    /// Valores actuales de los componentes de estado (marcado, texto, opción).
    valores: Valores,
    /// Arrastre de un `slider` en curso: (nodo, valor al empezar).
    slider: Option<(usize, i64)>,
    registro: Registro<C>,
    tema: &'static Tema,
    ancho: u32,
    alto: u32,
    cerrada: bool,
    progresos: Vec<ProgresoAnimado>,
}

impl<C> App<C> {
    /// Monta la aplicación a partir del texto de un documento.
    pub fn nueva(
        src: &str,
        ancho: u32,
        alto: u32,
        registro: Registro<C>,
    ) -> Result<App<C>, ErrorApp> {
        App::de_bytes(src.as_bytes(), ancho, alto, registro)
    }

    /// Igual que [`App::nueva`], desde bytes (un fichero tal cual).
    pub fn de_bytes(
        bytes: &[u8],
        ancho: u32,
        alto: u32,
        registro: Registro<C>,
    ) -> Result<App<C>, ErrorApp> {
        let doc = parser::analizar_bytes(bytes).map_err(ErrorApp::Sintaxis)?;
        let doc = validation::validar(&doc).map_err(ErrorApp::Validacion)?;
        Ok(App::montada(doc, ancho, alto, registro))
    }

    /// Monta la aplicación leyendo recursos del sistema o de un bundle `.app`.
    ///
    /// Sólo `/usr/share/tobyui/**.toby`; cualquier otra ruta se rechaza.
    pub fn desde_fs(
        ruta: &str,
        ancho: u32,
        alto: u32,
        registro: Registro<C>,
    ) -> Result<App<C>, ErrorApp> {
        let bytes = resource::leer_doc(ruta).map_err(ErrorApp::Recurso)?;
        App::de_bytes(&bytes, ancho, alto, registro)
    }

    fn montada(doc: ValidatedDocument, ancho: u32, alto: u32, registro: Registro<C>) -> App<C> {
        let disp = layout::calcular_en(&doc, ancho, alto);
        let valores = Valores::del_documento(&doc);
        let progresos = recopilar_progresos(&doc, &disp, ancho, alto);
        App {
            doc,
            disp,
            sesion: Sesion::default(),
            entrada: Entrada::nueva((ancho / 2) as i32, (alto / 2) as i32),
            valores,
            slider: None,
            registro,
            tema: &TEMA_POR_DEFECTO,
            ancho,
            alto,
            cerrada: false,
            progresos,
        }
    }

    /// Cambia el tema con el que se pinta.
    pub fn con_tema(mut self, tema: &'static Tema) -> App<C> {
        self.tema = tema;
        self
    }

    // --- Consulta -------------------------------------------------------

    /// Documento validado que está en pantalla.
    pub fn documento(&self) -> &ValidatedDocument {
        &self.doc
    }

    /// Disposición calculada (rectángulos por índice de preorden).
    pub fn disposicion(&self) -> &Disposicion {
        &self.disp
    }

    /// Acciones registradas.
    pub fn registro(&self) -> &Registro<C> {
        &self.registro
    }

    /// Registro mutable (para añadir acciones con la aplicación en marcha).
    pub fn registro_mut(&mut self) -> &mut Registro<C> {
        &mut self.registro
    }

    /// Tema activo.
    pub fn tema(&self) -> &Tema {
        self.tema
    }

    /// Nombre declarado en `<toby-app name="...">`.
    pub fn nombre(&self) -> &str {
        &self.doc.nombre
    }

    /// Tamaño del área de la aplicación, en píxeles.
    pub fn tamano(&self) -> (u32, u32) {
        (self.ancho, self.alto)
    }

    /// Transición declarada en el `<window>` raíz del documento, si la hay.
    /// La geometría origen/destino y el reloj los aporta el anfitrión.
    pub fn transition_config(&self) -> Option<TransitionConfig> {
        let ventana = self.doc.raiz.hijos.first()?;
        if ventana.def.etiqueta != "window" {
            return None;
        }
        let effect = match ventana.texto("transition", "none") {
            "genie" => Effect::Genie,
            "bounce" => Effect::Bounce,
            _ => return None,
        };
        Some(TransitionConfig {
            effect,
            duration_ms: ventana
                .entero(
                    "transition-duration",
                    match effect {
                        Effect::Genie => GENIE_DURATION_MS,
                        Effect::Bounce => BOUNCE_DURATION_MS,
                        Effect::None => GENIE_DURATION_MS,
                    } as i64,
                )
                .clamp(1, 10_000) as u32,
        })
    }

    /// Tamaño solicitado por una ventana con `mode="window"`, si la raíz de
    /// la app declara ese modo. El layout ya ha limitado el tamaño al host.
    pub fn tamano_ventana(&self) -> Option<(u32, u32)> {
        let ventana = self.doc.raiz.hijos.first()?;
        if ventana.def.etiqueta != "window" || ventana.texto("mode", "fullscreen") != "window" {
            return None;
        }
        let rect = self.disp.rect(1);
        (rect.w > 0 && rect.h > 0).then_some((rect.w, rect.h))
    }

    /// Estado de interacción que se conservará al siguiente fotograma.
    pub fn sesion(&self) -> Sesion {
        self.sesion
    }

    /// Nodo con el foco de teclado.
    pub fn foco(&self) -> Option<usize> {
        self.sesion.foco
    }

    /// Nodo bajo el puntero.
    pub fn hover(&self) -> Option<usize> {
        self.sesion.hover
    }

    /// Nodo pulsado con el ratón.
    pub fn pulsado(&self) -> Option<usize> {
        self.sesion.pulsado
    }

    /// Valores actuales de los componentes de estado.
    pub fn valores(&self) -> &Valores {
        &self.valores
    }

    /// Valores, para escribir en ellos (un editor puede sembrarlos).
    pub fn valores_mut(&mut self) -> &mut Valores {
        &mut self.valores
    }

    /// Actualiza el porcentaje de un `<progress mode="determinate">` por id.
    /// El valor se limita a 0–100. Devuelve `true` si cambió; el host puede
    /// entonces solicitar un repintado normal. El método no crea timers; el
    /// host anima las barras visibles mediante el planificador compartido.
    pub fn actualizar_progreso(&mut self, id: &str, value: i64) -> bool {
        let Some(i) = self.doc.indice_de(id) else {
            return false;
        };
        let Some(nodo) = self.doc.nodo_en(i) else {
            return false;
        };
        if nodo.def.etiqueta != "progress" || nodo.texto("mode", "indeterminate") != "determinate" {
            return false;
        }
        let value = value.clamp(0, 100);
        let celda = self.valores.celda_mut(i);
        if celda.entero == value {
            return false;
        }
        celda.entero = value;
        true
    }

    /// Posición del puntero acumulada por la aplicación.
    pub fn puntero(&self) -> (i32, i32) {
        (self.entrada.x, self.entrada.y)
    }

    /// Máscara de botones del ratón.
    pub fn botones(&self) -> u8 {
        self.entrada.botones()
    }

    /// Estado visual de un nodo (lo que usa el renderer).
    pub fn estado_de(&self, i: usize) -> Estado {
        Interaccion::retomar(&self.doc, &self.disp, self.sesion).estado_de(i)
    }

    /// ¿Alguna acción ha pedido cerrar?
    pub fn cerrada(&self) -> bool {
        self.cerrada
    }

    // --- Cambios --------------------------------------------------------

    /// Nuevo tamaño del área (por ejemplo, si cambia el modo de vídeo).
    pub fn cambiar_tamano(&mut self, ancho: u32, alto: u32) {
        self.ancho = ancho;
        self.alto = alto;
        self.disp = layout::calcular_en(&self.doc, ancho, alto);
        // El puntero puede haber quedado fuera del área nueva.
        self.entrada.mover(0, 0, ancho as i32, alto as i32);
        self.sesion = Sesion::default();
        self.slider = None;
    }

    /// Cambia el tamaño de una ventana `mode="window"` y recalcula su layout.
    /// Devuelve `false` si el documento no contiene una ventana flotante o no
    /// se pudo ampliar la lista de propiedades del documento.
    pub fn cambiar_tamano_ventana(&mut self, ancho: u32, alto: u32) -> bool {
        let Some(ventana) =
            self.doc.raiz.hijos.first_mut().filter(|n| {
                n.def.etiqueta == "window" && n.texto("mode", "fullscreen") == "window"
            })
        else {
            return false;
        };

        for (nombre, valor) in [("width", ancho), ("height", alto)] {
            if let Some((_, existente)) = ventana.props.iter_mut().find(|(k, _)| k == nombre) {
                *existente = super::document::PropValue::Int(valor as i64);
            } else {
                if ventana.props.try_reserve(1).is_err() {
                    return false;
                }
                ventana.props.push((
                    String::from(nombre),
                    super::document::PropValue::Int(valor as i64),
                ));
            }
        }
        self.cambiar_tamano(ancho, alto);
        true
    }

    /// Coloca el puntero en una posición absoluta.
    pub fn colocar_puntero(&mut self, x: i32, y: i32) {
        self.entrada.x = x.clamp(0, (self.ancho as i32 - 1).max(0));
        self.entrada.y = y.clamp(0, (self.alto as i32 - 1).max(0));
    }

    /// Marca la aplicación como cerrada (lo mismo que haría `window.close`).
    pub fn cerrar(&mut self) {
        self.cerrada = true;
    }

    // --- Ciclo ----------------------------------------------------------

    /// Lee el ratón y el teclado del kernel.
    ///
    /// Es el único punto de este módulo que toca hardware.
    pub fn sondear(&mut self) -> Vec<Evento> {
        event::sondear(&mut self.entrada, self.ancho as i32, self.alto as i32)
    }

    /// Construye eventos desde coordenadas y botones aportados por un
    /// compositor anfitrión (por ejemplo, una ventana dentro del escritorio).
    pub fn eventos_desde_anfitrion(
        &mut self,
        x: i32,
        y: i32,
        botones: u8,
        teclas: &[u8],
    ) -> Vec<Evento> {
        let mut eventos = Vec::new();
        let dx = x - self.entrada.x;
        let dy = y - self.entrada.y;
        if dx != 0 || dy != 0 {
            eventos.push(
                self.entrada
                    .mover(dx, dy, self.ancho as i32, self.alto as i32),
            );
        }
        eventos.extend(self.entrada.botones_nuevos(botones));
        // El anfitrion entrega caracteres ya traducidos (los que reparte Kira
        // a la ventana), no pulsaciones neutrales: se envuelven tal cual.
        eventos.extend(
            teclas
                .iter()
                .copied()
                .map(|b| Evento::Tecla(Tecla::del_byte(b))),
        );
        eventos
    }

    /// Aplica un evento ya normalizado y ejecuta la acción si corresponde.
    ///
    /// El evento pasa por tres etapas:
    ///   1. el menú desplegado de un `popup` (si lo hay), que pinta encima de
    ///      todo y por eso se atiende primero;
    ///   2. el sistema de eventos genérico (foco, hover, pulsado, clic);
    ///   3. el VALOR del componente: marcar un `check`, mover un `slider`,
    ///      elegir una opción, escribir en un `field`.
    ///
    /// El valor cambia ANTES de despachar la acción, así que el manejador
    /// siempre ve el estado ya actualizado.
    pub fn procesar(&mut self, ev: &Evento, ctx: &mut C) -> Paso {
        let menu_antes = self.menu_abierto();
        if let Some(paso) = self.procesar_menu(ev, ctx) {
            return paso;
        }
        // Escape cierra el menú desplegado; el evento sigue su camino (también
        // quita el foco, como siempre).
        if matches!(ev, Evento::Tecla(Tecla::Escape)) {
            self.cerrar_menus();
        }

        // Primero se resuelve el sistema de eventos (foco/hover/clic) en un
        // `Interaccion` de paso corto: sólo vive hasta aquí, porque aplicar
        // después el valor necesita `&mut self`.
        let cambios = {
            let mut it = Interaccion::retomar(&self.doc, &self.disp, self.sesion);
            let cambios = it.aplicar(ev);
            self.sesion = it.suspender();
            cambios
        };

        let mut paso = Paso {
            cambios,
            accion: None,
            error: None,
        };
        // Un clic (o un Escape) que cierra el menú hay que repintarlo.
        if menu_antes.is_some() && self.menu_abierto().is_none() {
            paso.cambios.valor = true;
        }

        self.aplicar_valor(ev, ctx, &mut paso);

        if let Some(i) = paso.cambios.click {
            if self.etiqueta_de(i) == "popup" {
                // Pulsar el disparador abre o cierra su menú: la acción la
                // dispara la OPCIÓN elegida, no el propio disparador.
                let abierto = self.valores.abierto(i);
                self.cerrar_menus();
                self.valores.celda_mut(i).abierto = !abierto;
                paso.cambios.valor = true;
            } else {
                let it = Interaccion::retomar(&self.doc, &self.disp, self.sesion);
                match self.registro.despachar_click(&it, &cambios, ctx) {
                    Ok(Some(r)) => {
                        if r == Resultado::Cerrar {
                            self.cerrada = true;
                        }
                        paso.accion = Some(r);
                    }
                    Ok(None) => {}
                    Err(e) => paso.error = Some(e),
                }
            }
        }
        paso
    }

    /// Etiqueta del componente de un nodo, según su disposición.
    fn etiqueta_de(&self, i: usize) -> &'static str {
        match self.disp.nodos.get(i) {
            Some(c) => c.def.etiqueta,
            None => "",
        }
    }

    /// El `popup` cuyo menú está desplegado, si hay alguno.
    ///
    /// Sólo puede haber uno abierto: abrir uno cierra el anterior.
    fn menu_abierto(&self) -> Option<usize> {
        (0..self.valores.len()).find(|&i| self.valores.abierto(i) && self.etiqueta_de(i) == "popup")
    }

    /// Cierra todos los menús desplegados.
    fn cerrar_menus(&mut self) {
        for i in 0..self.valores.len() {
            if self.etiqueta_de(i) == "popup" && self.valores.abierto(i) {
                self.valores.celda_mut(i).abierto = false;
            }
        }
    }

    /// Atiende el menú desplegado de un `popup`, si el evento va para él.
    ///
    /// Devuelve `Some(paso)` cuando el evento ha sido CONSUMIDO por el menú;
    /// `None` si el evento sigue su camino normal.
    fn procesar_menu(&mut self, ev: &Evento, ctx: &mut C) -> Option<Paso> {
        let abierto = self.menu_abierto()?;
        let (x, y) = match *ev {
            Evento::RatonAbajo { x, y, boton } if boton == Boton::Izquierdo => (x, y),
            _ => return None,
        };
        let rect = self.disp.rect(abierto);
        let items = match self.doc.nodo_en(abierto) {
            Some(n) => n.texto("items", "").to_string(),
            None => return None,
        };
        let menu = renderer::rect_menu(&items, rect);
        let mut paso = Paso::default();
        if !menu.contiene(x, y) {
            // Clic fuera: se cierra y el evento sigue su camino (como en
            // cualquier menú, el mismo clic puede pulsar lo que haya debajo).
            self.valores.celda_mut(abierto).abierto = false;
            return None;
        }
        paso.cambios.valor = true;
        match renderer::item_menu(&items, y - menu.y) {
            Some(k) => {
                let cambio = self.valores.entero(abierto) != k as i64;
                self.valores.celda_mut(abierto).entero = k as i64;
                self.valores.celda_mut(abierto).abierto = false;
                paso.cambios.click = Some(abierto);
                if cambio {
                    self.despachar_de(abierto, ctx, &mut paso);
                }
            }
            // En el margen del menú, sin opción: se cierra sin elegir.
            None => self.valores.celda_mut(abierto).abierto = false,
        }
        Some(paso)
    }

    /// Ejecuta la acción declarada por el nodo `i`, si la declara.
    fn despachar_de(&mut self, i: usize, ctx: &mut C, paso: &mut Paso) {
        let it = Interaccion::retomar(&self.doc, &self.disp, self.sesion);
        match self.registro.despachar_nodo(&it, i, ctx) {
            Ok(Some(r)) => {
                if r == Resultado::Cerrar {
                    self.cerrada = true;
                }
                paso.accion = Some(r);
            }
            Ok(None) => {}
            Err(e) => paso.error = Some(e),
        }
    }

    /// Aplica el evento al VALOR del componente que lo recibe.
    ///
    /// Aquí viven los gestos propios de cada control: conmutar un `check`,
    /// elegir un radio (excluyente dentro de su `group`), arrastrar un
    /// `slider` o escribir en un `field`.
    fn aplicar_valor(&mut self, ev: &Evento, ctx: &mut C, paso: &mut Paso) {
        match *ev {
            Evento::RatonAbajo { x, boton, .. } if boton == Boton::Izquierdo => {
                let i = match self.sesion.pulsado {
                    Some(i) => i,
                    None => return,
                };
                match self.etiqueta_de(i) {
                    "check" => {
                        let marcado = &mut self.valores.celda_mut(i).marcado;
                        *marcado = !*marcado;
                        paso.cambios.valor = true;
                    }
                    "radio" => self.marcar_radio(i, paso),
                    "segmented" => {
                        if let Some(k) = self.segmento_en(i, x) {
                            if self.valores.entero(i) != k as i64 {
                                self.valores.celda_mut(i).entero = k as i64;
                                paso.cambios.valor = true;
                            }
                        }
                    }
                    "slider" | "slider-gota" => {
                        let antes = self.valores.entero(i);
                        let v = self.valor_slider_en(i, x);
                        self.slider = Some((i, antes));
                        let c = self.valores.celda_mut(i);
                        c.trasiego = Some(antes);
                        if v != antes {
                            c.entero = v;
                            paso.cambios.valor = true;
                        }
                    }
                    _ => {}
                }
            }
            Evento::RatonMueve { x, .. } => {
                // Arrastre: el pulgar sigue al puntero mientras el botón sigue
                // pulsado (el sistema de eventos ya se encarga del resto).
                let i = match self.slider {
                    Some((i, _)) => i,
                    None => return,
                };
                if !self.entrada.esta_pulsado(Boton::Izquierdo) {
                    return;
                }
                if !matches!(self.etiqueta_de(i), "slider" | "slider-gota") {
                    return;
                }
                let v = self.valor_slider_en(i, x);
                if self.valores.entero(i) != v {
                    self.valores.celda_mut(i).entero = v;
                    paso.cambios.valor = true;
                }
            }
            Evento::RatonArriba { .. } => {
                // Fin del arrastre: si el valor cambió, se avisa con la acción.
                if let Some((i, antes)) = self.slider.take() {
                    if self.valores.entero(i) != antes {
                        self.despachar_de(i, ctx, paso);
                    }
                }
            }
            Evento::Tecla(t) => self.aplicar_tecla(t, ctx, paso),
            _ => {}
        }
    }

    /// Teclas con foco: escribir en un `field` o activar con Espacio/Intro.
    fn aplicar_tecla(&mut self, t: Tecla, ctx: &mut C, paso: &mut Paso) {
        let i = match self.sesion.foco {
            Some(i) => i,
            None => return,
        };
        match (self.etiqueta_de(i), t) {
            ("field", Tecla::Retroceso) => {
                if self.valores.celda_mut(i).texto.pop().is_some() {
                    paso.cambios.valor = true;
                }
            }
            ("field", Tecla::Intro) => self.despachar_de(i, ctx, paso),
            ("field", Tecla::Caracter(c)) => {
                let c = crate::texto::char_de_glifo(c);
                self.escribir_campo(i, c, paso);
            }
            ("field", Tecla::Espacio) => self.escribir_campo(i, ' ', paso),
            ("button" | "circular" | "check" | "radio", Tecla::Espacio | Tecla::Intro) => {
                match self.etiqueta_de(i) {
                    "check" => {
                        let marcado = &mut self.valores.celda_mut(i).marcado;
                        *marcado = !*marcado;
                        paso.cambios.valor = true;
                    }
                    "radio" => self.marcar_radio(i, paso),
                    _ => {}
                }
                self.despachar_de(i, ctx, paso);
            }
            _ => {}
        }
    }

    /// Añade un carácter al texto de un `field`, sin pasarse de largo.
    fn escribir_campo(&mut self, i: usize, c: char, paso: &mut Paso) {
        let texto = &mut self.valores.celda_mut(i).texto;
        if texto.chars().count() < MAX_TEXTO_CAMPO {
            texto.push(c);
            paso.cambios.valor = true;
        }
    }

    /// Marca un `radio` y desmarca los demás de su mismo `group`.
    fn marcar_radio(&mut self, i: usize, paso: &mut Paso) {
        let grupo = match self.doc.nodo_en(i) {
            Some(n) => n.texto("group", "").to_string(),
            None => return,
        };
        for k in 0..self.valores.len() {
            let mismo_grupo = match self.doc.nodo_en(k) {
                Some(n) => n.def.etiqueta == "radio" && n.texto("group", "") == grupo,
                None => false,
            };
            if mismo_grupo {
                self.valores.celda_mut(k).marcado = false;
            }
        }
        self.valores.celda_mut(i).marcado = true;
        paso.cambios.valor = true;
    }

    /// Opción de un `segmented` que cae en una coordenada X, si cae en alguna.
    ///
    /// Reparte las opciones EXACTAMENTE como el renderer (`rectos_segmentos`),
    /// de modo que lo que se pinta es lo que se puede pulsar.
    fn segmento_en(&self, i: usize, x: i32) -> Option<usize> {
        let rect = self.disp.rect(i);
        let items = match self.doc.nodo_en(i) {
            Some(n) => n.texto("items", ""),
            None => return None,
        };
        renderer::rectos_segmentos(items, rect)
            .iter()
            .position(|s| x >= s.x && x < s.x + s.w as i32)
    }

    /// Valor que toca un `slider` al pulsar (o arrastrar) en una coordenada X.
    ///
    /// El pulgar se pinta con el CENTRO en el valor y viaja entre los bordes
    /// del control menos su propio ancho; la cuenta de aquí es la MISMA que
    /// usa `renderer::pintar_slider`, así que lo que se ve es lo que se pulsa.
    fn valor_slider_en(&self, i: usize, x: i32) -> i64 {
        let rect = self.disp.rect(i);
        let n = match self.doc.nodo_en(i) {
            Some(n) => n,
            None => return 0,
        };
        let (min, max) = valores::rango_slider(n);
        let ancho = renderer::ancho_pulgar(
            renderer::alto_pulgar(rect.h),
            self.etiqueta_de(i) == "slider-gota",
        )
        .min(rect.w)
        .max(1);
        let via = rect.w.saturating_sub(ancho).max(1) as i64;
        let rel = (x - rect.x - (ancho / 2) as i32).clamp(0, via as i32) as i64;
        min + rel * (max - min) / via
    }

    /// Atiende toda la entrada pendiente y resume el resultado.
    pub fn atender(&mut self, ctx: &mut C) -> Paso {
        let eventos = self.sondear();
        self.atender_eventos(&eventos, ctx)
    }

    /// Atiende una lista de eventos concreta (útil para pruebas y para
    /// anfitriones que ya tienen su propia cola de entrada).
    pub fn atender_eventos(&mut self, eventos: &[Evento], ctx: &mut C) -> Paso {
        let mut paso = Paso::default();
        for ev in eventos {
            let p = self.procesar(ev, ctx);
            paso.cambios.unir(p.cambios);
            if p.accion.is_some() {
                paso.accion = p.accion;
            }
            if p.error.is_some() {
                paso.error = p.error;
            }
        }
        paso
    }

    /// Pinta la aplicación entera en un lienzo.
    ///
    /// Limpia con el fondo del tema, así que el lienzo pasa a ser de la
    /// aplicación: el anfitrión decide sobre qué área se la da.
    pub fn pintar(&self, lienzo: &mut Canvas, fuente: Option<&Font>) {
        self.pintar_en(lienzo, fuente, crate::pit::ticks());
    }

    /// Pinta usando el reloj explícito del anfitrión (ticks PIT a 100 Hz en
    /// Rocky). Kira y otros hosts pueden actualizar una animación a demanda,
    /// sin añadir un bucle interno ni reservar estado por fotograma.
    pub fn pintar_en(&self, lienzo: &mut Canvas, fuente: Option<&Font>, ahora_ticks: u64) {
        self.pintar_impl(lienzo, fuente, None, true, ahora_ticks);
    }

    /// Pinta usando la caché antialias SF Pro del sistema, sin cambiar la
    /// fuente de las apps que siguen usando el bitmap del anfitrión.
    pub fn pintar_con_fuente_suave(
        &self,
        lienzo: &mut Canvas,
        fuente: Option<&Font>,
        fuente_suave: &FuenteSuave,
    ) {
        self.pintar_impl(
            lienzo,
            fuente,
            Some(fuente_suave),
            true,
            crate::pit::ticks(),
        );
    }

    /// Indica si hay una barra de progreso visible que necesita fotogramas.
    pub fn tiene_progreso_animado(&self) -> bool {
        !self.progresos.is_empty()
    }

    /// Compatibilidad: ahora ambos modos de progreso tienen un patrón móvil.
    pub fn tiene_progreso_indeterminado(&self) -> bool {
        self.tiene_progreso_animado()
    }

    /// Áreas visibles locales de las piezas animadas, para que el compositor
    /// pueda presentar sólo las regiones dañadas y no el escritorio entero.
    pub fn regiones_progreso(&self) -> impl Iterator<Item = layout::Rect> + '_ {
        self.progresos.iter().map(|progreso| progreso.clip)
    }

    /// Repinta exclusivamente las barras animadas. Su geometría se precalcula
    /// al montar la app, por lo que este camino no recorre el árbol ni asigna
    /// memoria en cada fotograma.
    pub fn pintar_progreso_animado(
        &self,
        lienzo: &mut Canvas,
        fuente: Option<&Font>,
        ahora_ticks: u64,
    ) {
        if !self.tiene_progreso_animado() {
            return;
        }
        let mut r = CanvasRenderer::nuevo(lienzo, fuente);
        for progreso in &self.progresos {
            r.recorte(progreso.clip);
            if progreso.determinado {
                let value = self.valores.entero(progreso.indice).clamp(0, 100) as u8;
                renderer::pintar_progreso_determinado(
                    &mut r,
                    self.tema,
                    progreso.rect,
                    value,
                    ahora_ticks,
                );
            } else {
                renderer::pintar_progreso_indeterminado(
                    &mut r,
                    self.tema,
                    progreso.rect,
                    ahora_ticks,
                );
            }
            r.recorte_fin();
        }
        r.presentar();
    }

    /// Captura una vez la superficie de la app para la transición declarada.
    ///
    /// `captura` es un lienzo RAM reutilizable, de tamaño exacto a la app y con
    /// stride igual a su ancho. El cursor no se captura para que lo pinte el
    /// anfitrión por encima del efecto. Devuelve `false` si no se declaró
    /// Genie o la captura no coincide.
    pub fn capturar_transicion_genie(&self, captura: &mut Canvas, fuente: Option<&Font>) -> bool {
        if !matches!(
            self.transition_config().map(|config| config.effect),
            Some(Effect::Genie)
        ) || !self.captura_valida(captura)
        {
            return false;
        }
        self.pintar_impl(captura, fuente, None, false, crate::pit::ticks());
        true
    }

    /// Dibuja la captura estática con la malla compartida de AppKit.
    ///
    /// El anfitrión elige origen/destino y avanza `progreso` mediante el
    /// `Animation` creado desde el `TransitionConfig` de la app. El destino debe
    /// ser distinto de la captura. Si no se declaró Genie, o la captura no
    /// coincide con el tamaño de la app, devuelve `false` y se puede usar
    /// [`App::pintar`] para el pintado normal. La misma captura se reutiliza
    /// durante todos los frames: no recorre de nuevo el árbol ni reserva RAM.
    #[allow(clippy::too_many_arguments)]
    pub fn pintar_transicion_genie(
        &self,
        lienzo: &mut Canvas,
        captura: &Canvas,
        origen: GenieRect,
        destino: GenieRect,
        progreso: u16,
    ) -> bool {
        if !matches!(
            self.transition_config().map(|config| config.effect),
            Some(Effect::Genie)
        ) || !self.captura_valida(captura)
        {
            return false;
        }

        toby_appkit::genie::rasterizar(
            captura.pixeles(),
            self.ancho,
            self.alto,
            lienzo.width,
            lienzo.height,
            origen,
            destino,
            progreso,
            |x, y, _sx, _sy, color| lienzo.put(x, y, color),
        );
        true
    }

    fn captura_valida(&self, captura: &Canvas) -> bool {
        self.ancho > 0
            && self.alto > 0
            && captura.width == self.ancho as i32
            && captura.height == self.alto as i32
            && captura.stride == self.ancho as usize
    }

    fn pintar_impl(
        &self,
        lienzo: &mut Canvas,
        fuente: Option<&Font>,
        fuente_suave: Option<&FuenteSuave>,
        cursor: bool,
        ahora_ticks: u64,
    ) {
        let it = Interaccion::retomar(&self.doc, &self.disp, self.sesion);
        let mut r = match fuente_suave {
            Some(fuente_suave) => {
                CanvasRenderer::nuevo_con_fuente_suave(lienzo, fuente, fuente_suave)
            }
            None => CanvasRenderer::nuevo(lienzo, fuente),
        };
        renderer::pintar_con_estado_y_cursor_en(
            &mut r,
            &self.doc,
            &self.disp,
            self.tema,
            &|i| it.estado_de(i),
            &self.valores,
            cursor.then_some((self.entrada.x, self.entrada.y)),
            ahora_ticks,
        );
        r.presentar();
    }

    /// Un fotograma completo: atender la entrada, ejecutar acciones y pintar.
    pub fn fotograma(&mut self, lienzo: &mut Canvas, fuente: Option<&Font>, ctx: &mut C) -> Paso {
        let paso = self.atender(ctx);
        self.pintar(lienzo, fuente);
        paso
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::toby_ui::action::Peticion;

    #[derive(Default)]
    struct Cuenta {
        veces: usize,
    }

    fn sumar(c: &mut Cuenta, _p: &Peticion<'_>) -> Resultado {
        c.veces += 1;
        Resultado::Hecho
    }

    fn registro() -> Registro<Cuenta> {
        let mut r = Registro::nuevo();
        r.registrar("demo.sumar", sumar).expect("clave válida");
        r
    }

    const DOC: &str = concat!(
        "<toby-app name=\"sumadora\">",
        "<window width=\"320\" height=\"240\">",
        "<column padding=\"16\">",
        "<button id=\"mas\" title=\"Sumar\" action=\"demo.sumar\"/>",
        "</column>",
        "</window>",
        "</toby-app>"
    );

    const DOC_VENTANA: &str = concat!(
        "<toby-app name=\"ventana\">",
        "<window width=\"120\" height=\"80\" mode=\"window\">",
        "<column width=\"100\" height=\"60\"/>",
        "</window>",
        "</toby-app>"
    );

    fn app() -> App<Cuenta> {
        App::nueva(DOC, 320, 240, registro()).expect("documento válido")
    }

    #[test]
    fn progreso_porcentual_se_actualiza_y_anima_sin_salirse_del_rango() {
        let src = concat!(
            "<toby-app name=\"progreso\"><window width=\"220\" height=\"40\">",
            "<progress id=\"instalacion\" mode=\"determinate\" value=\"20\"/>",
            "</window></toby-app>"
        );
        let mut app = App::nueva(src, 220, 40, registro()).expect("documento válido");
        let indice = app.documento().indice_de("instalacion").expect("id");
        assert!(app.tiene_progreso_animado());
        assert_eq!(app.regiones_progreso().count(), 1);
        assert_eq!(app.valores().entero(indice), 20);
        assert!(app.actualizar_progreso("instalacion", 140));
        assert_eq!(app.valores().entero(indice), 100);
        assert!(!app.actualizar_progreso("instalacion", 100));
        assert!(!app.actualizar_progreso("no-existe", 50));
    }

    #[test]
    fn la_ventana_flotante_cambia_de_tamano_y_puede_restaurarse() {
        let mut app = App::nueva(DOC_VENTANA, 640, 420, registro()).expect("documento válido");
        assert_eq!(app.tamano_ventana(), Some((120, 80)));
        assert!(app.cambiar_tamano_ventana(640, 420));
        assert_eq!(app.tamano_ventana(), Some((640, 420)));
        assert!(app.cambiar_tamano_ventana(120, 80));
        assert_eq!(app.tamano_ventana(), Some((120, 80)));
    }

    #[test]
    fn genie_declarado_en_toby_usa_el_timeline_y_rasterizador_de_appkit() {
        let src = concat!(
            "<toby-app name=\"genie\">",
            "<window width=\"4\" height=\"4\" transition=\"genie\" transition-duration=\"700\">",
            "<column/>",
            "</window></toby-app>"
        );
        let app = App::nueva(src, 4, 4, registro()).expect("documento valido");
        let config = app.transition_config().expect("transicion declarada");
        assert_eq!(config.effect, Effect::Genie);
        assert_eq!(config.duration_ms, 700);
        let reloj = config.animation(20, 100);
        assert_eq!(reloj.duration(), 70);
        assert_eq!(reloj.progress(55), 32_767);
    }

    #[test]
    fn genie_usa_500_ms_por_defecto_y_el_host_lo_convierte_a_50_ticks() {
        let src = concat!(
            "<toby-app name=\"genie-default\">",
            "<window width=\"4\" height=\"4\" transition=\"genie\">",
            "<column/>",
            "</window></toby-app>"
        );
        let app = App::nueva(src, 4, 4, registro()).expect("documento valido");
        let config = app.transition_config().expect("transicion declarada");
        assert_eq!(config.duration_ms, 500);
        assert_eq!(config.animation(0, 100).duration(), 50);
    }

    #[test]
    fn bounce_de_toby_ui_se_expone_como_efecto_reutilizable_de_appkit() {
        let src = concat!(
            "<toby-app name=\"bounce\">",
            "<window width=\"4\" height=\"4\" transition=\"bounce\">",
            "<column/>",
            "</window></toby-app>"
        );
        let app = App::nueva(src, 4, 4, registro()).expect("documento valido");
        let config = app.transition_config().expect("transicion declarada");
        assert_eq!(config.effect, Effect::Bounce);
        assert_eq!(config.duration_ms, BOUNCE_DURATION_MS);
        let rebote = config.bounce(0, 1000, 18).expect("rebote reutilizable");
        assert_eq!(rebote.offset_y(150), -18);
        assert_eq!(rebote.offset_y(900), 0);
    }

    fn clic(app: &mut App<Cuenta>, x: i32, y: i32, c: &mut Cuenta) -> Paso {
        let mut paso = app.procesar(
            &Evento::RatonAbajo {
                x,
                y,
                boton: crate::toby_ui::event::Boton::Izquierdo,
            },
            c,
        );
        let p2 = app.procesar(
            &Evento::RatonArriba {
                x,
                y,
                boton: crate::toby_ui::event::Boton::Izquierdo,
            },
            c,
        );
        paso.cambios.unir(p2.cambios);
        paso.accion = p2.accion;
        paso.error = p2.error;
        paso
    }

    #[test]
    fn se_monta_y_se_consulta() {
        let a = app();
        assert_eq!(a.nombre(), "sumadora");
        assert_eq!(a.tamano(), (320, 240));
        assert_eq!(a.documento().num_nodos(), 4);
        assert_eq!(a.disposicion().num_nodos(), 4);
        assert!(!a.cerrada());
        assert_eq!(a.puntero(), (160, 120));
        assert_eq!(a.registro().len(), 1);
    }

    #[test]
    fn detecta_ventana_solicitada_por_la_app() {
        let src = concat!(
            "<toby-app name=\"finder\">",
            "<window width=\"640\" height=\"420\" mode=\"window\"/>",
            "</toby-app>"
        );
        let a = App::nueva(src, 1024, 768, registro()).expect("documento válido");
        assert_eq!(a.tamano_ventana(), Some((640, 420)));
        assert_eq!(a.disposicion().rect(1).x, 192);
        assert_eq!(a.disposicion().rect(1).y, 174);
    }

    #[test]
    fn se_puede_montar_desde_bytes_y_guardar_con_el_tema_por_defecto() {
        let a = App::de_bytes(DOC.as_bytes(), 320, 240, registro()).expect("válido");
        assert_eq!(a.tema().nombre, TEMA_POR_DEFECTO.nombre);
    }

    #[test]
    fn los_documentos_malos_se_rechazan_con_su_motivo() {
        let e = App::nueva(
            "<toby-app name=\"x\"><window width=\"1\" height=\"1\"></toby-app>",
            100,
            100,
            registro(),
        )
        .err()
        .expect("sintaxis");
        assert!(matches!(e, ErrorApp::Sintaxis(_)));
        assert!(!e.resumen().is_empty());

        let e = App::nueva(
            "<toby-app name=\"x\"><column/></toby-app>",
            100,
            100,
            registro(),
        )
        .err()
        .expect("validación");
        assert!(matches!(e, ErrorApp::Validacion(_)));
        assert!(e.resumen().contains("no es válido"));
        assert!(e.to_string().contains("window"));

        // Y una ruta de fichero no autorizada ni se intenta leer.
        let e = App::desde_fs("/etc/passwd", 100, 100, registro())
            .err()
            .expect("ruta");
        assert!(matches!(
            e,
            ErrorApp::Recurso(ErrorResource::RutaNoAutorizada)
        ));
        assert_eq!(e.resumen(), "ruta de recurso no autorizada");
    }

    #[test]
    fn el_clic_ejecuta_la_accion_y_no_cierra() {
        let mut a = app();
        let mut c = Cuenta::default();
        // El botón está en (16,16,80,24).
        let paso = clic(&mut a, 20, 20, &mut c);
        assert_eq!(paso.accion, Some(Resultado::Hecho));
        assert!(paso.error.is_none());
        assert!(paso.cambios.pulsado);
        assert!(!a.cerrada());
        assert_eq!(c.veces, 1);
        assert_eq!(a.foco(), Some(3));
    }

    #[test]
    fn la_accion_inexistente_es_un_error_controlado() {
        let src = concat!(
            "<toby-app name=\"x\">",
            "<window width=\"320\" height=\"240\">",
            "<column padding=\"16\">",
            "<button id=\"a\" title=\"A\" action=\"demo.desconocida\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        );
        let mut a =
            App::nueva(src, 320, 240, registro()).expect("válido: la clave está bien formada");
        let mut c = Cuenta::default();
        let paso = clic(&mut a, 20, 20, &mut c);
        assert!(paso.accion.is_none());
        assert_eq!(
            paso.error.as_ref().map(|e| e.clave()),
            Some("demo.desconocida")
        );
        assert_eq!(c.veces, 0);
        // La aplicación sigue viva y funcionando.
        assert!(!a.cerrada());
    }

    #[test]
    fn un_boton_sin_accion_no_despacha_nada() {
        let src = concat!(
            "<toby-app name=\"x\">",
            "<window width=\"320\" height=\"240\">",
            "<column padding=\"16\"><button id=\"a\" title=\"A\"/></column>",
            "</window>",
            "</toby-app>"
        );
        let mut a = App::nueva(src, 320, 240, registro()).expect("válido");
        let mut c = Cuenta::default();
        let paso = clic(&mut a, 20, 20, &mut c);
        assert!(paso.accion.is_none() && paso.error.is_none());
        assert_eq!(c.veces, 0);
    }

    #[test]
    fn tabulador_da_el_foco_y_escape_lo_suelta() {
        let mut a = app();
        let mut c = Cuenta::default();
        let p = a.procesar(
            &Evento::Tecla(crate::toby_ui::event::Tecla::Tabulador),
            &mut c,
        );
        assert!(p.cambios.foco);
        assert_eq!(a.foco(), Some(3));
        assert_eq!(a.estado_de(3), Estado::Focused);
        let p = a.procesar(&Evento::Tecla(crate::toby_ui::event::Tecla::Escape), &mut c);
        assert!(p.cambios.foco);
        assert_eq!(a.foco(), None);
    }

    #[test]
    fn la_sesion_sobrevive_entre_pasos() {
        let mut a = app();
        let mut c = Cuenta::default();
        a.procesar(
            &Evento::RatonMueve {
                x: 20,
                y: 20,
                dx: 0,
                dy: 0,
            },
            &mut c,
        );
        assert_eq!(a.hover(), Some(3));
        // Otro evento distinto no borra el hover.
        a.procesar(
            &Evento::Tecla(crate::toby_ui::event::Tecla::Tabulador),
            &mut c,
        );
        assert_eq!(a.hover(), Some(3));
        assert_eq!(a.foco(), Some(3));
        // Pero salir del botón sí.
        a.procesar(
            &Evento::RatonMueve {
                x: 300,
                y: 200,
                dx: 0,
                dy: 0,
            },
            &mut c,
        );
        assert_eq!(a.hover(), None);
        assert_eq!(a.sesion().foco, Some(3));
    }

    #[test]
    fn atender_eventos_resume_varios() {
        let mut a = app();
        let mut c = Cuenta::default();
        let evs = alloc::vec![
            Evento::RatonMueve {
                x: 20,
                y: 20,
                dx: 0,
                dy: 0
            },
            Evento::RatonAbajo {
                x: 20,
                y: 20,
                boton: crate::toby_ui::event::Boton::Izquierdo
            },
            Evento::RatonArriba {
                x: 20,
                y: 20,
                boton: crate::toby_ui::event::Boton::Izquierdo
            },
        ];
        let paso = a.atender_eventos(&evs, &mut c);
        assert!(paso.repintar());
        assert_eq!(paso.cambios.click, Some(3));
        assert_eq!(paso.accion, Some(Resultado::Hecho));
        assert_eq!(c.veces, 1);
    }

    #[test]
    fn cerrar_y_cambiar_de_tamano() {
        let mut a = app();
        a.cerrar();
        assert!(a.cerrada());

        let mut a = app();
        a.colocar_puntero(9_000, 9_000);
        assert_eq!(a.puntero(), (319, 239));
        a.colocar_puntero(-5, -5);
        assert_eq!(a.puntero(), (0, 0));

        // Al encoger el área, el puntero se acota y la ventana se recorta.
        a.colocar_puntero(9_000, 9_000);
        a.cambiar_tamano(200, 120);
        assert_eq!(a.tamano(), (200, 120));
        assert_eq!(a.disposicion().rect(1).w, 200);
        assert_eq!(a.disposicion().rect(1).h, 120);
        // Y todo lo de interacción se reinicia.
        assert_eq!(a.foco(), None);
        assert_eq!(a.hover(), None);
        assert_eq!(a.puntero(), (199, 119));
    }

    #[test]
    fn una_accion_puede_pedir_cerrar() {
        let src = concat!(
            "<toby-app name=\"x\">",
            "<window width=\"320\" height=\"240\">",
            "<column padding=\"16\">",
            "<button id=\"a\" title=\"A\" action=\"demo.cerrar\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        );
        fn cerrar(_c: &mut Cuenta, _p: &Peticion<'_>) -> Resultado {
            Resultado::Cerrar
        }
        let mut reg = registro();
        reg.registrar("demo.cerrar", cerrar).expect("clave válida");
        let mut a = App::nueva(src, 320, 240, reg).expect("válido");
        let mut c = Cuenta::default();
        let paso = clic(&mut a, 20, 20, &mut c);
        assert_eq!(paso.accion, Some(Resultado::Cerrar));
        assert!(a.cerrada());
        // El registro se puede ampliar con la aplicación en marcha.
        assert_eq!(a.registro().len(), 2);
        assert!(a.registro_mut().contiene("demo.sumar"));
    }

    #[test]
    fn el_campo_escribe_con_los_acentos_del_teclado() {
        let src = concat!(
            "<toby-app name=\"x\">",
            "<window width=\"320\" height=\"240\">",
            "<column padding=\"16\">",
            "<field id=\"f\" width=\"160\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        );
        let mut a = App::nueva(src, 320, 240, registro()).expect("válido");
        let mut c = Cuenta::default();
        // El clic pone el foco en el campo (nodo 3).
        a.procesar(
            &Evento::RatonAbajo {
                x: 20,
                y: 20,
                boton: crate::toby_ui::event::Boton::Izquierdo,
            },
            &mut c,
        );
        assert_eq!(a.foco(), Some(3));
        // El teclado entrega bytes CP437: 0xA4 es ñ y 0xC1 el Á compuesto.
        for b in [b'P', b'i', 0xA4, 0xC1] {
            let paso = a.procesar(&Evento::Tecla(Tecla::del_byte(b)), &mut c);
            assert!(paso.cambios.valor, "cada tecla cambia el valor");
        }
        assert_eq!(a.valores().texto(3), "PiñÁ");
        // Y la borradora se lleva el último caracter, no el último byte.
        let paso = a.procesar(&Evento::Tecla(Tecla::Retroceso), &mut c);
        assert!(paso.cambios.valor);
        assert_eq!(a.valores().texto(3), "Piñ");
    }

    #[test]
    fn el_campo_acepta_todo_ascii_imprimible() {
        let src = concat!(
            "<toby-app name=\"x\">",
            "<window width=\"320\" height=\"240\">",
            "<column padding=\"16\">",
            "<field id=\"f\" width=\"280\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        );
        let mut app = App::nueva(src, 320, 240, registro()).expect("válido");
        let mut ctx = Cuenta::default();
        app.procesar(
            &Evento::RatonAbajo {
                x: 20,
                y: 20,
                boton: crate::toby_ui::event::Boton::Izquierdo,
            },
            &mut ctx,
        );
        for byte in 0x20..=0x7E {
            app.valores.celda_mut(3).texto.clear();
            app.procesar(&Evento::Tecla(Tecla::del_byte(byte)), &mut ctx);
            assert_eq!(
                app.valores().texto(3).chars().next(),
                Some(char::from(byte))
            );
        }
    }
}
