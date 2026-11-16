//! Elementos del escritorio Toby: lectura de `/home/escritorio`, iconos,
//! selección y posiciones manuales/alineadas persistentes.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::espacio;
use crate::fs::{self, Tipo};
use crate::gfx::{self, Canvas};
use crate::toby_ui::layout::Rect;
use crate::toby_ui::renderer::{self, CanvasRenderer, FuenteSuave, Renderer};

const MAX_ELEMENTOS: usize = 96;
const MAX_CONFIG: usize = 64 * 1024;
const ARCHIVO_CONFIG: &str = ".toby-desktop-layout";
const TAMANO_ICONO_MIN: u32 = 16;
const TAMANO_ICONO_INICIAL: u32 = 48;
const TAMANO_ICONO_MAX: u32 = 96;
const PASO_TAMANO_ICONO: u32 = 8;
const ESPACIO_CELDA_X: i32 = 32;
const ESPACIO_CELDA_Y: i32 = 44;
const MARGEN: i32 = 10;
const ALTO_ETIQUETA: i32 = 34;
const MENU_ANCHO: i32 = 224;
const MENU_ALTO: i32 = 92;
const MENU_FILA: i32 = 26;
const MENU_RADIO_ESQUINA: i32 = 10;

const ICONO_CARPETA: &str = "/system/apps/toby_desktop.app/img/folder.png";
const ICONO_DOCUMENTO: &str = "/system/apps/toby_desktop.app/img/document.png";
const ICONO_IMAGEN: &str = "/system/apps/toby_desktop.app/img/image.png";
const ICONO_APP: &str = "/system/apps/toby_desktop.app/img/app.png";
const ICONO_ACCESO_DIRECTO: &str = "/system/apps/toby_desktop.app/img/direct.png";
const SUFIJO_ACCESO_DIRECTO: &str = ".tobylink";
const MAX_ACCESO_DIRECTO: usize = 512;

pub struct Elemento {
    pub nombre: String,
    pub tipo: Tipo,
    pub acceso_directo: bool,
    pub modificado: u32,
    ruta_destino: Option<String>,
    ruta_app: Option<String>,
    icono_app: Option<String>,
    icono_app_resuelto: bool,
    pub x: i32,
    pub y: i32,
    pub seleccionado: bool,
}

struct PosicionGuardada {
    nombre: String,
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccionMenu {
    AlternarAlineacion,
    OrdenarNombre,
    Actualizar,
}

/// Estado del escritorio independiente del compositor gráfico.
pub struct Escritorio {
    pub elementos: Vec<Elemento>,
    /// `None` identifica el escritorio persistente; `Some` es una vista de
    /// archivos reutilizable (por ejemplo, Toby Finder) sobre otra carpeta.
    ruta: Option<String>,
    tamano_icono: u32,
    alineado: bool,
    config_leida: bool,
    guardadas: Vec<PosicionGuardada>,
    menu: Option<(i32, i32)>,
    menu_armado: Option<AccionMenu>,
}

impl Escritorio {
    pub const fn nuevo() -> Self {
        Self {
            elementos: Vec::new(),
            ruta: None,
            tamano_icono: TAMANO_ICONO_INICIAL,
            alineado: true,
            config_leida: false,
            guardadas: Vec::new(),
            menu: None,
            menu_armado: None,
        }
    }

    /// Crea la misma vista de archivos del escritorio, asociada a una ruta
    /// navegable y sin cargar ni guardar el layout del escritorio.
    pub fn navegador(ruta: &str) -> Self {
        Self {
            elementos: Vec::new(),
            ruta: Some(String::from(ruta)),
            tamano_icono: TAMANO_ICONO_INICIAL,
            alineado: true,
            config_leida: true,
            guardadas: Vec::new(),
            menu: None,
            menu_armado: None,
        }
    }

    pub fn ruta_actual(&self) -> &str {
        self.ruta.as_deref().unwrap_or(espacio::ESCRITORIO)
    }

    /// Cambia de carpeta si existe y es un directorio. La selección y el
    /// paginado pertenecen a la ventana, no se guardan en el layout del dock.
    pub fn navegar_a(&mut self, ruta: &str, ancho: i32, alto: i32) -> bool {
        let Ok(registro) = fs::resolver(fs::raiz(), ruta) else {
            return false;
        };
        if fs::tipo(registro) != Tipo::Directorio {
            return false;
        }
        self.ruta = Some(fs::ruta_de(registro));
        self.elementos.clear();
        self.refrescar(ancho, alto);
        true
    }

    pub fn ruta_elemento(&self, indice: usize) -> Option<String> {
        let item = self.elementos.get(indice)?;
        if item.tipo != Tipo::Directorio {
            return None;
        }
        if let Some(ruta) = &item.ruta_destino {
            return Some(ruta.clone());
        }
        let actual = self.ruta_actual();
        Some(if actual == "/" {
            alloc::format!("/{}", item.nombre)
        } else {
            alloc::format!("{}/{}", actual.trim_end_matches('/'), item.nombre)
        })
    }

    /// Lee la carpeta Desktop canónica del usuario y conserva sus posiciones.
    pub fn refrescar(&mut self, ancho: i32, alto: i32) {
        if !fs::montado() {
            return;
        }
        if self.ruta.is_none() && !self.config_leida {
            self.leer_configuracion();
        }
        let Ok(dir) = fs::resolver(fs::raiz(), self.ruta_actual()) else {
            return;
        };

        let mut entradas = Vec::new();
        fs::listar_hasta(dir, |entrada| {
            if entrada.nombre.starts_with('.') {
                return true;
            }
            if entradas.len() >= MAX_ELEMENTOS {
                return false;
            }
            entradas.push((
                entrada.nombre.clone(),
                entrada.tipo,
                entrada.registro,
                entrada.modificado,
            ));
            true
        });
        entradas.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        // Estrato no implementa symlinks: los accesos directos del escritorio
        // son descriptores pequenos y se resuelven despues de cerrar listar().
        let mut visibles = Vec::with_capacity(entradas.len());
        for (nombre, tipo, registro, modificado) in entradas {
            let destino = if tipo == Tipo::Fichero && nombre.ends_with(SUFIJO_ACCESO_DIRECTO) {
                leer_destino_acceso_directo(registro).and_then(|ruta| {
                    fs::resolver(fs::raiz(), &ruta)
                        .ok()
                        .filter(|&objetivo| fs::tipo(objetivo) == Tipo::Directorio)
                        .map(|_| ruta)
                })
            } else {
                None
            };
            if destino.is_some() {
                let nombre_visible = nombre
                    .strip_suffix(SUFIJO_ACCESO_DIRECTO)
                    .unwrap_or(&nombre);
                visibles.push((
                    String::from(nombre_visible),
                    Tipo::Directorio,
                    true,
                    destino,
                    None,
                    modificado,
                ));
            } else {
                let ruta_app = (tipo == Tipo::Directorio && nombre.ends_with(".app")).then(|| {
                    alloc::format!("{}/{}", self.ruta_actual().trim_end_matches('/'), nombre)
                });
                visibles.push((nombre, tipo, false, None, ruta_app, modificado));
            }
        }

        let anteriores = core::mem::take(&mut self.elementos);
        self.elementos.reserve(visibles.len());
        for (indice, (nombre, tipo, acceso_directo, ruta_destino, ruta_app, modificado)) in
            visibles.into_iter().enumerate()
        {
            let anterior = anteriores.iter().find(|item| item.nombre == nombre);
            let (icono_app, icono_app_resuelto) = anterior
                .filter(|item| item.ruta_app.as_deref() == ruta_app.as_deref())
                .map(|item| (item.icono_app.clone(), item.icono_app_resuelto))
                .unwrap_or((None, false));
            let guardada = self.guardadas.iter().find(|pos| pos.nombre == nombre);
            let (x, y, seleccionado) = if let Some(item) = anterior {
                (item.x, item.y, item.seleccionado)
            } else if let Some(pos) = guardada {
                (pos.x, pos.y, false)
            } else {
                self.posicion_libre(indice, ancho, alto)
            };
            let (x, y) = limitar_posicion(x, y, ancho, alto, self.tamano_icono);
            self.elementos.push(Elemento {
                nombre,
                tipo,
                acceso_directo,
                modificado,
                ruta_destino,
                ruta_app,
                icono_app,
                icono_app_resuelto,
                x,
                y,
                seleccionado,
            });
        }
        // Mantiene posiciones guardadas; cualquier entrada nueva entra en un
        // hueco libre incluso si su nombre la intercala antes que otra entrada.
        self.ajustar_nuevos_a_cuadricula(ancho, alto);
    }

    fn leer_configuracion(&mut self) {
        self.config_leida = true;
        let ruta = alloc::format!("{}/{}", espacio::ESCRITORIO, ARCHIVO_CONFIG);
        let Ok(reg) = fs::resolver(fs::raiz(), &ruta) else {
            return;
        };
        let tamano = fs::tamano(reg) as usize;
        if tamano == 0 || tamano > MAX_CONFIG {
            return;
        }
        let mut bytes = vec![0; tamano];
        if fs::leer(reg, 0, &mut bytes) != tamano {
            return;
        }
        let texto = String::from_utf8_lossy(&bytes);
        let mut lineas = texto.lines();
        if lineas.next() != Some("TBD1") {
            return;
        }
        self.alineado = lineas.next() != Some("grid=0");
        for linea in lineas {
            if let Some(valor) = linea.strip_prefix("icon=") {
                if let Ok(valor) = valor.parse::<u32>() {
                    self.tamano_icono = normalizar_tamano_icono(valor);
                }
                continue;
            }
            let mut campos = linea.split_whitespace();
            let (Some(x), Some(y), Some(nombre_hex)) =
                (campos.next(), campos.next(), campos.next())
            else {
                continue;
            };
            let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>()) else {
                continue;
            };
            if let Some(nombre) = decodificar_hex(nombre_hex) {
                self.guardadas.push(PosicionGuardada { nombre, x, y });
            }
        }
    }

    /// Persiste el modo y las posiciones una vez al terminar cada operación,
    /// nunca durante el movimiento continuo del ratón.
    pub fn guardar(&mut self) {
        if self.ruta.is_some() || !fs::montado() {
            return;
        }
        let mut texto = String::from("TBD1\n");
        texto.push_str(if self.alineado {
            "grid=1\n"
        } else {
            "grid=0\n"
        });
        texto.push_str(&alloc::format!("icon={}\n", self.tamano_icono));
        for item in &self.elementos {
            texto.push_str(&alloc::format!("{} {} ", item.x, item.y));
            for byte in item.nombre.as_bytes() {
                texto.push(HEX[(byte >> 4) as usize] as char);
                texto.push(HEX[(byte & 0x0F) as usize] as char);
            }
            texto.push('\n');
        }

        let Ok(dir) = fs::resolver(fs::raiz(), espacio::ESCRITORIO) else {
            return;
        };
        let reg = match fs::resolver(dir, ARCHIVO_CONFIG) {
            Ok(reg) => reg,
            Err(_) => match fs::crear(dir, ARCHIVO_CONFIG, Tipo::Fichero) {
                Ok(reg) => reg,
                Err(_) => return,
            },
        };
        fs::truncar(reg);
        if fs::escribir(reg, 0, texto.as_bytes()).is_ok() {
            self.guardadas.clear();
            self.guardadas.reserve(self.elementos.len());
            for item in &self.elementos {
                self.guardadas.push(PosicionGuardada {
                    nombre: item.nombre.clone(),
                    x: item.x,
                    y: item.y,
                });
            }
        }
    }

    pub fn limpiar_seleccion(&mut self) {
        for item in &mut self.elementos {
            item.seleccionado = false;
        }
    }

    pub fn elemento_bajo(&self, x: i32, y: i32) -> Option<usize> {
        let ancho = ancho_celda(self.tamano_icono);
        let alto = alto_celda(self.tamano_icono);
        self.elementos.iter().rposition(|item| {
            Rect {
                x: item.x + 2,
                y: item.y,
                w: (ancho - 4) as u32,
                h: (alto - 3) as u32,
            }
            .contiene(x, y)
        })
    }

    /// Número de elementos que caben en una página de la vista compartida.
    pub fn capacidad_en(&self, area: Rect) -> usize {
        let ancho = ancho_celda(self.tamano_icono).max(1);
        let alto = alto_celda(self.tamano_icono).max(1);
        let columnas = ((area.w as i32) / ancho).max(1) as usize;
        let filas = ((area.h as i32) / alto).max(1) as usize;
        columnas.saturating_mul(filas).max(1)
    }

    /// Hit test para la rejilla de Finder; usa la misma celda y el mismo
    /// elemento que pinta `pintar_en`.
    pub fn elemento_bajo_en(&self, x: i32, y: i32, area: Rect, inicio: usize) -> Option<usize> {
        for indice in inicio
            ..self
                .elementos
                .len()
                .min(inicio.saturating_add(self.capacidad_en(area)))
        {
            let celda = self.celda_elemento_en(indice, area, inicio);
            if (Rect {
                x: celda.x + 2,
                y: celda.y,
                w: celda.w.saturating_sub(4),
                h: celda.h.saturating_sub(3),
            })
            .contiene(x, y)
            {
                return Some(indice);
            }
        }
        None
    }

    /// Selecciona los iconos que intersecta el marco en una página Finder.
    pub fn seleccionar_rectangulo_en(
        &mut self,
        ax: i32,
        ay: i32,
        bx: i32,
        by: i32,
        area: Rect,
        inicio: usize,
    ) {
        self.limpiar_seleccion();
        let (x0, x1) = (ax.min(bx), ax.max(bx));
        let (y0, y1) = (ay.min(by), ay.max(by));
        let fin = self
            .elementos
            .len()
            .min(inicio.saturating_add(self.capacidad_en(area)));
        for indice in inicio..fin {
            let celda = self.celda_elemento_en(indice, area, inicio);
            let item = &mut self.elementos[indice];
            let ix1 = celda.der() - 4;
            let iy1 = celda.abajo() - 3;
            item.seleccionado = x0 <= ix1 && x1 >= celda.x + 2 && y0 <= iy1 && y1 >= celda.y;
        }
    }

    /// Selecciona un elemento, conservando el grupo si ya estaba seleccionado.
    pub fn seleccionar_elemento(&mut self, indice: usize) {
        if !self
            .elementos
            .get(indice)
            .is_some_and(|item| item.seleccionado)
        {
            self.limpiar_seleccion();
            if let Some(item) = self.elementos.get_mut(indice) {
                item.seleccionado = true;
            }
        }
    }

    pub fn seleccionar_rectangulo(&mut self, ax: i32, ay: i32, bx: i32, by: i32) {
        let (x0, x1) = (ax.min(bx), ax.max(bx));
        let (y0, y1) = (ay.min(by), ay.max(by));
        let ancho = ancho_celda(self.tamano_icono);
        let alto = alto_celda(self.tamano_icono);
        for item in &mut self.elementos {
            let ix1 = item.x + ancho - 4;
            let iy1 = item.y + alto - 3;
            item.seleccionado = x0 <= ix1 && x1 >= item.x + 2 && y0 <= iy1 && y1 >= item.y;
        }
    }

    pub fn mover_seleccionados(&mut self, dx: i32, dy: i32, ancho: i32, alto: i32) {
        for item in &mut self.elementos {
            if item.seleccionado {
                (item.x, item.y) = limitar_posicion(
                    item.x.saturating_add(dx),
                    item.y.saturating_add(dy),
                    ancho,
                    alto,
                    self.tamano_icono,
                );
            }
        }
    }

    pub fn soltar_arrastre(&mut self, ancho: i32, alto: i32) {
        if self.alineado {
            self.encajar_seleccionados(ancho, alto);
        }
        self.guardar();
    }

    /// Cambia el tamaño de los iconos y mantiene sus celdas libres y visibles.
    pub fn cambiar_tamano_icono(&mut self, agrandar: bool, ancho: i32, alto: i32) -> bool {
        let siguiente = if agrandar {
            self.tamano_icono.saturating_add(PASO_TAMANO_ICONO)
        } else {
            self.tamano_icono.saturating_sub(PASO_TAMANO_ICONO)
        };
        let siguiente = normalizar_tamano_icono(siguiente);
        if siguiente == self.tamano_icono {
            return false;
        }
        self.tamano_icono = siguiente;
        if self.alineado {
            self.recolocar_en_cuadricula(ancho, alto);
        } else {
            for item in &mut self.elementos {
                (item.x, item.y) = limitar_posicion(item.x, item.y, ancho, alto, self.tamano_icono);
            }
        }
        self.guardar();
        true
    }

    pub fn menu_abierto(&self) -> bool {
        self.menu.is_some()
    }

    pub fn abrir_menu(&mut self, x: i32, y: i32, ancho: i32, alto: i32) {
        let x = x.clamp(4, (ancho - MENU_ANCHO - 4).max(4));
        let y_min = renderer::ALTO_BARRA_VENTANA as i32 + 4;
        let y_max = (alto - crate::dock::alto_reservado(alto) - MENU_ALTO - 4).max(y_min);
        let y = y.clamp(y_min, y_max);
        self.menu = Some((x, y));
        self.menu_armado = None;
    }

    pub fn armar_accion_menu(&mut self, x: i32, y: i32) {
        self.menu_armado = self.accion_menu_en(x, y);
    }

    /// Cierra el menú y ejecuta una acción sólo si se soltó sobre la fila
    /// inicialmente pulsada.
    pub fn soltar_menu(&mut self, x: i32, y: i32, ancho: i32, alto: i32) -> bool {
        let accion = self.accion_menu_en(x, y);
        let ejecutar = accion.filter(|accion| Some(*accion) == self.menu_armado);
        let estaba_abierto = self.menu.take().is_some();
        self.menu_armado = None;
        if let Some(accion) = ejecutar {
            match accion {
                AccionMenu::AlternarAlineacion => {
                    if self.alineado {
                        self.alineado = false;
                    } else {
                        self.alinear(ancho, alto);
                    }
                    self.guardar();
                }
                AccionMenu::OrdenarNombre => {
                    self.elementos
                        .sort_unstable_by(|a, b| a.nombre.cmp(&b.nombre));
                    self.alineado = true;
                    self.asignar_cuadricula(ancho, alto);
                    self.guardar();
                }
                AccionMenu::Actualizar => self.refrescar(ancho, alto),
            }
        }
        estaba_abierto
    }

    fn accion_menu_en(&self, x: i32, y: i32) -> Option<AccionMenu> {
        let (mx, my) = self.menu?;
        if x < mx + 8 || x >= mx + MENU_ANCHO - 8 {
            return None;
        }
        let fila = (y - (my + 6)) / MENU_FILA;
        if y < my + 6 || fila < 0 {
            return None;
        }
        match fila {
            0 => Some(AccionMenu::AlternarAlineacion),
            1 => Some(AccionMenu::OrdenarNombre),
            2 => Some(AccionMenu::Actualizar),
            _ => None,
        }
    }

    fn posicion_libre(&self, desde: usize, ancho: i32, alto: i32) -> (i32, i32, bool) {
        let (x, y) = buscar_hueco(desde, &self.elementos, ancho, alto, self.tamano_icono);
        (x, y, false)
    }

    fn ajustar_nuevos_a_cuadricula(&mut self, ancho: i32, alto: i32) {
        // Los elementos preexistentes ya guardados conservan su sitio. Este
        // pase sólo evita que una nueva entrada tape una celda ocupada.
        for i in 0..self.elementos.len() {
            let duplicado = self.elementos[..i]
                .iter()
                .any(|prev| prev.x == self.elementos[i].x && prev.y == self.elementos[i].y);
            if duplicado {
                let (x, y) = buscar_hueco(i, &self.elementos[..i], ancho, alto, self.tamano_icono);
                self.elementos[i].x = x;
                self.elementos[i].y = y;
            }
        }
    }

    fn alinear(&mut self, ancho: i32, alto: i32) {
        // Conserva el orden espacial de lectura del escritorio (columnas de
        // derecha a izquierda) y ajusta cada elemento a una celda única.
        self.elementos
            .sort_unstable_by(|a, b| b.x.cmp(&a.x).then(a.y.cmp(&b.y)));
        self.alineado = true;
        self.asignar_cuadricula(ancho, alto);
    }

    fn asignar_cuadricula(&mut self, ancho: i32, alto: i32) {
        for (indice, item) in self.elementos.iter_mut().enumerate() {
            let (x, y) = posicion_celda(indice, ancho, alto, self.tamano_icono);
            item.x = x;
            item.y = y;
        }
    }

    fn encajar_seleccionados(&mut self, ancho: i32, alto: i32) {
        let total = total_celdas(ancho, alto, self.tamano_icono);
        let mut ocupadas: Vec<(i32, i32)> = Vec::new();
        for item in self.elementos.iter().filter(|item| !item.seleccionado) {
            if let Some((x, y)) = celda_mas_cercana(
                item.x,
                item.y,
                ancho,
                alto,
                self.tamano_icono,
                &ocupadas,
                total,
            ) {
                ocupadas.push((x, y));
            }
        }
        for item in self.elementos.iter_mut().filter(|item| item.seleccionado) {
            let Some((x, y)) = celda_mas_cercana(
                item.x,
                item.y,
                ancho,
                alto,
                self.tamano_icono,
                &ocupadas,
                total,
            ) else {
                continue;
            };
            item.x = x;
            item.y = y;
            ocupadas.push((x, y));
        }
    }

    fn recolocar_en_cuadricula(&mut self, ancho: i32, alto: i32) {
        let total = total_celdas(ancho, alto, self.tamano_icono);
        let mut ocupadas: Vec<(i32, i32)> = Vec::new();
        for item in &mut self.elementos {
            if let Some((x, y)) = celda_mas_cercana(
                item.x,
                item.y,
                ancho,
                alto,
                self.tamano_icono,
                &ocupadas,
                total,
            ) {
                item.x = x;
                item.y = y;
                ocupadas.push((x, y));
            } else {
                (item.x, item.y) = limitar_posicion(item.x, item.y, ancho, alto, self.tamano_icono);
            }
        }
    }

    pub fn pintar(&self, c: &mut Canvas) {
        let font = crate::fuentes::para_barra_menu();
        let fuente_bios = crate::fuentes::para_kira();
        for item in &self.elementos {
            self.pintar_elemento(c, item, item.x, item.y, font.as_ref(), fuente_bios.as_ref());
        }
    }

    /// Dibuja una página del mismo componente de iconos dentro de un panel,
    /// sin posiciones persistentes ni estilos paralelos al escritorio.
    pub fn pintar_en(&self, c: &mut Canvas, area: Rect, inicio: usize) {
        let capacidad = self.capacidad_en(area);
        let font = crate::fuentes::para_barra_menu();
        let fuente_bios = crate::fuentes::para_kira();
        for indice in inicio..self.elementos.len().min(inicio.saturating_add(capacidad)) {
            let celda = self.celda_elemento_en(indice, area, inicio);
            self.pintar_elemento(
                c,
                &self.elementos[indice],
                celda.x,
                celda.y,
                font.as_ref(),
                fuente_bios.as_ref(),
            );
        }
    }

    /// Carga fuera de la sección crítica los PNG que pueden aparecer al
    /// pintar estos elementos. Finder lo usa antes de publicar la lista, para
    /// que el primer repintado no haga E/S ni decodifique imágenes mientras
    /// Kira tiene detenido el planificador.
    pub fn precalentar_iconos(&self) {
        for ruta in [ICONO_CARPETA, ICONO_DOCUMENTO, ICONO_IMAGEN, ICONO_APP] {
            let _ = crate::toby_ui::kit::imagen_recurso(ruta);
        }
        if self.elementos.iter().any(|item| item.acceso_directo) {
            let _ = crate::toby_ui::kit::imagen_recurso(ICONO_ACCESO_DIRECTO);
        }
    }

    /// Devuelve si esta vista tiene al menos un paquete cuya metadata de icono
    /// todavía debe consultarse fuera de la sección crítica del compositor.
    pub fn tiene_iconos_app_pendientes(&self) -> bool {
        self.elementos
            .iter()
            .any(|item| item.ruta_app.is_some() && !item.icono_app_resuelto)
    }

    /// Toma una ruta de bundle pendiente y marca su consulta como iniciada.
    pub fn tomar_ruta_icono_app_pendiente(&mut self) -> Option<String> {
        let item = self
            .elementos
            .iter_mut()
            .find(|item| item.ruta_app.is_some() && !item.icono_app_resuelto)?;
        item.icono_app_resuelto = true;
        item.ruta_app.clone()
    }

    /// Publica el recurso ya validado y precargado para todos los accesos al
    /// mismo bundle presentes en esta vista.
    pub fn asignar_icono_app(&mut self, bundle: &str, icono: Option<String>) -> bool {
        let mut cambiado = false;
        for item in &mut self.elementos {
            if item.ruta_app.as_deref() == Some(bundle) {
                item.icono_app_resuelto = true;
                item.icono_app = icono.clone();
                cambiado |= item.icono_app.is_some();
            }
        }
        cambiado
    }

    /// Pinta el icono compartido de un elemento en un rectangulo arbitrario.
    /// Finder reutiliza esta primitiva en lista, columnas y galeria para que
    /// sus vistas no sustituyan los recursos reales por iconos dibujados aparte.
    pub fn pintar_icono_en(c: &mut Canvas, item: &Elemento, rect: Rect) {
        let fuente_bios = crate::fuentes::para_kira();
        let mut r = CanvasRenderer::nuevo(c, fuente_bios.as_ref());
        if let Some(imagen) = crate::toby_ui::kit::imagen_recurso(icono_recurso_de(item)) {
            r.imagen(rect, imagen);
        } else {
            pintar_icono_respaldo(&mut r, rect, item.tipo);
        }
        if item.acceso_directo {
            if let Some(emblema) = crate::toby_ui::kit::imagen_recurso(ICONO_ACCESO_DIRECTO) {
                let lado = (rect.w.min(rect.h) / 3).max(8);
                r.imagen(
                    Rect {
                        x: rect.x + 1,
                        y: rect.y + rect.h as i32 - lado as i32 - 1,
                        w: lado,
                        h: lado,
                    },
                    emblema,
                );
            }
        }
    }

    fn celda_elemento_en(&self, indice: usize, area: Rect, inicio: usize) -> Rect {
        let ancho = ancho_celda(self.tamano_icono);
        let alto = alto_celda(self.tamano_icono);
        let columnas = ((area.w as i32) / ancho).max(1) as usize;
        let celda = indice - inicio;
        Rect {
            x: area.x + (celda % columnas) as i32 * ancho,
            y: area.y + (celda / columnas) as i32 * alto,
            w: ancho as u32,
            h: alto as u32,
        }
    }

    fn pintar_elemento(
        &self,
        c: &mut Canvas,
        item: &Elemento,
        x: i32,
        y: i32,
        font: Option<&FuenteSuave>,
        fuente_bios: Option<&crate::bios::Font>,
    ) {
        let ancho_celda = ancho_celda(self.tamano_icono);
        let tamano_icono = self.tamano_icono;
        let caja_icono = Rect {
            x: x + (ancho_celda - tamano_icono as i32) / 2,
            y,
            w: tamano_icono,
            h: tamano_icono,
        };
        {
            let mut r = CanvasRenderer::nuevo(c, fuente_bios);
            if let Some(imagen) = crate::toby_ui::kit::imagen_recurso(icono_recurso_de(item)) {
                r.imagen(caja_icono, imagen);
            } else {
                pintar_icono_respaldo(&mut r, caja_icono, item.tipo);
            }
            if item.acceso_directo {
                if let Some(emblema) = crate::toby_ui::kit::imagen_recurso(ICONO_ACCESO_DIRECTO) {
                    let lado_emblema = (tamano_icono / 4).max(8);
                    r.imagen(
                        Rect {
                            x: caja_icono.x + 2,
                            y: caja_icono.y + caja_icono.h as i32 - lado_emblema as i32 - 2,
                            w: lado_emblema,
                            h: lado_emblema,
                        },
                        emblema,
                    );
                }
            }
        }

        let (linea1, linea2, label) = geometria_nombre_en(item, font, tamano_icono, x, y);
        if item.seleccionado {
            pintar_marco_redondeado(
                c,
                Rect {
                    x: caja_icono.x - 3,
                    y: caja_icono.y - 2,
                    w: caja_icono.w + 6,
                    h: caja_icono.h + 4,
                },
                (tamano_icono as i32 / 7).clamp(6, 12),
                gfx::rgb(115, 177, 255),
            );
            let ancho_texto = [&linea1, &linea2]
                .iter()
                .filter(|texto| !texto.is_empty())
                .map(|texto| medir_texto(texto, font))
                .max()
                .unwrap_or(0);
            let ancho_seleccion = (ancho_texto + 12).min(label.w as i32);
            pintar_superficie_redondeada(
                c,
                Rect {
                    x: label.x + (label.w as i32 - ancho_seleccion) / 2,
                    y: label.y,
                    w: ancho_seleccion as u32,
                    h: label.h,
                },
                5,
                gfx::rgb(42, 112, 230),
            );
        }

        let mut r = CanvasRenderer::nuevo(c, fuente_bios);
        self.pintar_nombre(&mut r, font, &linea1, &linea2, label);
    }

    /// Pinta el menú contextual por encima de ventanas y aplicaciones.
    pub fn pintar_menu_superpuesto(&self, c: &mut Canvas, mx: i32, my: i32) {
        let Some((x, y)) = self.menu else {
            return;
        };
        let panel = Rect {
            x,
            y,
            w: MENU_ANCHO as u32,
            h: MENU_ALTO as u32,
        };
        pintar_panel_redondeado(
            c,
            panel,
            MENU_RADIO_ESQUINA,
            gfx::rgb(246, 247, 249),
            gfx::rgb(133, 137, 144),
        );
        if self.accion_menu_en(mx, my).is_some() {
            let fila = ((my - (y + 6)) / MENU_FILA).clamp(0, 2);
            pintar_superficie_redondeada(
                c,
                Rect {
                    x: x + MENU_RADIO_ESQUINA,
                    y: y + 6 + fila * MENU_FILA,
                    w: (MENU_ANCHO - MENU_RADIO_ESQUINA * 2) as u32,
                    h: MENU_FILA as u32,
                },
                6,
                gfx::rgb(49, 119, 231),
            );
        }
        let font = crate::fuentes::para_barra_menu();
        let fuente_bios = crate::fuentes::para_kira();
        let mut r = CanvasRenderer::nuevo(c, fuente_bios.as_ref());
        self.pintar_menu(&mut r, font.as_ref(), x, y, mx, my);
    }

    fn pintar_nombre(
        &self,
        r: &mut CanvasRenderer<'_, '_>,
        font: Option<&FuenteSuave>,
        linea1: &str,
        linea2: &str,
        label: Rect,
    ) {
        let lineas = [linea1, linea2];
        let color = gfx::rgb(255, 255, 255);
        for (i, texto) in lineas.iter().enumerate() {
            if texto.is_empty() {
                continue;
            }
            let texto_ancho = medir_texto(texto, font);
            let x = label.x + (label.w as i32 - texto_ancho).max(0) / 2;
            let ty = label.y + i as i32 * 15 + 1;
            if let Some(font) = font {
                r.texto_suavizado(x + 1, ty + 1, texto, 0x00101010, font);
                r.texto_suavizado(x, ty, texto, color, font);
            } else {
                r.texto(x + 1, ty + 1, texto, 0x00101010, 1);
                r.texto(x, ty, texto, color, 1);
            }
        }
    }

    fn pintar_menu(
        &self,
        r: &mut CanvasRenderer<'_, '_>,
        font: Option<&FuenteSuave>,
        x: i32,
        y: i32,
        mx: i32,
        my: i32,
    ) {
        let etiqueta_alinear = if self.alineado {
            "Desalinear iconos"
        } else {
            "Alinear a la cuadrícula"
        };
        let etiquetas = [
            etiqueta_alinear,
            "Ordenar por nombre",
            "Actualizar escritorio",
        ];
        for (i, etiqueta) in etiquetas.iter().enumerate() {
            let fila = Rect {
                x: x + MENU_RADIO_ESQUINA,
                y: y + 6 + i as i32 * MENU_FILA,
                w: (MENU_ANCHO - MENU_RADIO_ESQUINA * 2) as u32,
                h: MENU_FILA as u32,
            };
            let activo = fila.contiene(mx, my);
            let color = if activo {
                gfx::rgb(255, 255, 255)
            } else {
                gfx::rgb(38, 40, 44)
            };
            if let Some(font) = font {
                r.texto_suavizado(fila.x + 8, fila.y + 5, etiqueta, color, font);
            } else {
                r.texto(fila.x + 8, fila.y + 5, etiqueta, color, 1);
            }
        }
    }
}

fn geometria_nombre_en(
    item: &Elemento,
    font: Option<&FuenteSuave>,
    tamano_icono: u32,
    x: i32,
    y: i32,
) -> (String, String, Rect) {
    let ancho = ancho_celda(tamano_icono) - 10;
    let (linea1, linea2) = dividir_nombre(&item.nombre, font, ancho);
    let label = Rect {
        x: x + 5,
        y: y + tamano_icono as i32 + 3,
        w: ancho as u32,
        h: if linea2.is_empty() {
            18
        } else {
            ALTO_ETIQUETA as u32
        },
    };
    (linea1, linea2, label)
}

/// Dibuja el contorno seleccionado sin tapar el icono, con esquinas suaves.
fn pintar_marco_redondeado(c: &Canvas, rect: Rect, radio: i32, color: u32) {
    let (x, y, w, h) = (rect.x, rect.y, rect.w as i32, rect.h as i32);
    if w <= 0 || h <= 0 {
        return;
    }
    let radio = radio.max(1).min((w.min(h) / 2) as i32);
    c.fill_rect(x + radio, y, w - 2 * radio, 1, color);
    c.fill_rect(x + radio, y + h - 1, w - 2 * radio, 1, color);
    c.fill_rect(x, y + radio, 1, h - 2 * radio, color);
    c.fill_rect(x + w - 1, y + radio, 1, h - 2 * radio, color);
    for fila in 0..radio {
        for columna in 0..radio {
            let (exterior, interior) = cobertura_esquina_panel(fila, columna, radio);
            let cobertura = exterior - interior;
            if cobertura == 0 {
                continue;
            }
            let esquinas = [
                (x + columna, y + fila),
                (x + w - 1 - columna, y + fila),
                (x + columna, y + h - 1 - fila),
                (x + w - 1 - columna, y + h - 1 - fila),
            ];
            for (px, py) in esquinas {
                c.put(
                    px,
                    py,
                    componer_cobertura(c.get(px, py), 0, color, 0, cobertura),
                );
            }
        }
    }
}

/// Fondo azul compacto para el nombre seleccionado, con cuatro esquinas
/// redondeadas y cobertura antialiasada contra la escena que ya está pintada.
pub(crate) fn pintar_superficie_redondeada(c: &Canvas, rect: Rect, radio: i32, color: u32) {
    let (x, y, w, h) = (rect.x, rect.y, rect.w as i32, rect.h as i32);
    if w <= 0 || h <= 0 {
        return;
    }
    let radio = radio.max(1).min((w.min(h) / 2) as i32);
    c.fill_rect(x + radio, y, w - 2 * radio, h, color);
    c.fill_rect(x, y + radio, radio, h - 2 * radio, color);
    c.fill_rect(x + w - radio, y + radio, radio, h - 2 * radio, color);
    for fila in 0..radio {
        for columna in 0..radio {
            let (cobertura, _) = cobertura_esquina_panel(fila, columna, radio);
            if cobertura == 0 {
                continue;
            }
            let esquinas = [
                (x + columna, y + fila),
                (x + w - 1 - columna, y + fila),
                (x + columna, y + h - 1 - fila),
                (x + w - 1 - columna, y + h - 1 - fila),
            ];
            for (px, py) in esquinas {
                c.put(
                    px,
                    py,
                    componer_cobertura(c.get(px, py), color, color, cobertura, cobertura),
                );
            }
        }
    }
}

/// Panel opaco con borde de un píxel y cuatro esquinas antialiasadas. Las
/// esquinas se componen contra lo que ya hay en el lienzo, sin sombra exterior.
pub(crate) fn pintar_panel_redondeado(c: &Canvas, rect: Rect, radio: i32, fondo: u32, borde: u32) {
    let radio = radio.max(1).min((rect.w.min(rect.h) / 2) as i32);
    let (x, y, w, h) = (rect.x, rect.y, rect.w as i32, rect.h as i32);
    if w <= 0 || h <= 0 {
        return;
    }

    // Relleno central y laterales rectos. Las cuatro esquinas se calculan
    // aparte para dejar el píxel exterior realmente transparente.
    c.fill_rect(x + radio, y + 1, w - radio * 2, h - 2, fondo);
    c.fill_rect(x + 1, y + radio, radio - 1, h - radio * 2, fondo);
    c.fill_rect(x + w - radio, y + radio, radio - 1, h - radio * 2, fondo);
    c.fill_rect(x + radio, y, w - radio * 2, 1, borde);
    c.fill_rect(x + radio, y + h - 1, w - radio * 2, 1, borde);
    c.fill_rect(x, y + radio, 1, h - radio * 2, borde);
    c.fill_rect(x + w - 1, y + radio, 1, h - radio * 2, borde);

    for fila in 0..radio {
        for columna in 0..radio {
            let (exterior, interior) = cobertura_esquina_panel(fila, columna, radio);
            if exterior == 0 {
                continue;
            }
            let esquinas = [
                (x + columna, y + fila),
                (x + w - 1 - columna, y + fila),
                (x + columna, y + h - 1 - fila),
                (x + w - 1 - columna, y + h - 1 - fila),
            ];
            for (px, py) in esquinas {
                c.put(
                    px,
                    py,
                    componer_cobertura(c.get(px, py), fondo, borde, interior, exterior),
                );
            }
        }
    }
}

fn cobertura_esquina_panel(fila: i32, columna: i32, radio: i32) -> (u32, u32) {
    const MUESTRAS: i64 = 8;
    const ESCALA: i64 = MUESTRAS * 2;
    let centro = radio as i64 * ESCALA;
    let radio_exterior2 = centro * centro;
    let radio_interior = (radio as i64 - 1).max(0) * ESCALA;
    let radio_interior2 = radio_interior * radio_interior;
    let mut exterior = 0;
    let mut interior = 0;
    for sy in 0..MUESTRAS {
        let py = fila as i64 * ESCALA + sy * 2 + 1;
        let dy = centro - py;
        for sx in 0..MUESTRAS {
            let px = columna as i64 * ESCALA + sx * 2 + 1;
            let dx = centro - px;
            let distancia2 = dx * dx + dy * dy;
            if distancia2 <= radio_exterior2 {
                exterior += 1;
                if radio > 1 && distancia2 <= radio_interior2 {
                    interior += 1;
                }
            }
        }
    }
    (exterior, interior)
}

fn componer_cobertura(
    fondo_previo: u32,
    fondo: u32,
    borde: u32,
    interior: u32,
    exterior: u32,
) -> u32 {
    const TOTAL: u32 = 64;
    let fuera = TOTAL - exterior;
    let ribete = exterior - interior;
    let canal = |desplazamiento: u32| {
        let suma = ((fondo_previo >> desplazamiento) & 0xFF) * fuera
            + ((borde >> desplazamiento) & 0xFF) * ribete
            + ((fondo >> desplazamiento) & 0xFF) * interior;
        (suma + TOTAL / 2) / TOTAL
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}

pub fn icono_de(item: &Elemento) -> &'static str {
    if item.acceso_directo {
        return ICONO_CARPETA;
    }
    if item.tipo == Tipo::Directorio {
        return if item.nombre.ends_with(".app") {
            ICONO_APP
        } else {
            ICONO_CARPETA
        };
    }
    let extension = item.nombre.rsplit('.').next().unwrap_or("");
    if ["png", "jpg", "jpeg", "bmp", "ico"]
        .iter()
        .any(|e| extension.eq_ignore_ascii_case(e))
    {
        ICONO_IMAGEN
    } else {
        ICONO_DOCUMENTO
    }
}

fn icono_recurso_de(item: &Elemento) -> &str {
    item.icono_app.as_deref().unwrap_or_else(|| icono_de(item))
}

fn leer_destino_acceso_directo(registro: u32) -> Option<String> {
    let tamano = fs::tamano(registro) as usize;
    if tamano == 0 || tamano > MAX_ACCESO_DIRECTO {
        return None;
    }
    let mut bytes = vec![0; tamano];
    if fs::leer(registro, 0, &mut bytes) != tamano {
        return None;
    }
    let texto = core::str::from_utf8(&bytes).ok()?;
    let mut lineas = texto.lines();
    if lineas.next()? != "TOBYLINK1" {
        return None;
    }
    let ruta = lineas.next()?.trim();
    if !ruta.starts_with('/') || ruta.split('/').any(|parte| parte == "..") {
        return None;
    }
    Some(String::from(ruta))
}

fn pintar_icono_respaldo(r: &mut CanvasRenderer<'_, '_>, rect: Rect, tipo: Tipo) {
    if tipo == Tipo::Directorio {
        let azul = gfx::rgb(66, 139, 211);
        r.relleno(
            Rect {
                x: rect.x + 2,
                y: rect.y + 10,
                w: rect.w - 4,
                h: rect.h - 14,
            },
            azul,
        );
        r.relleno(
            Rect {
                x: rect.x + 5,
                y: rect.y + 5,
                w: rect.w / 2,
                h: 12,
            },
            azul,
        );
        r.relleno(
            Rect {
                x: rect.x + 3,
                y: rect.y + 11,
                w: rect.w - 6,
                h: 1,
            },
            gfx::rgb(146, 202, 239),
        );
    } else {
        r.relleno(
            Rect {
                x: rect.x + 12,
                y: rect.y + 3,
                w: rect.w - 24,
                h: rect.h - 6,
            },
            gfx::rgb(244, 244, 244),
        );
        for y in [rect.y + 20, rect.y + 28] {
            r.relleno(
                Rect {
                    x: rect.x + 20,
                    y,
                    w: rect.w - 40,
                    h: 2,
                },
                gfx::rgb(108, 133, 160),
            );
        }
    }
}

fn dividir_nombre(nombre: &str, font: Option<&FuenteSuave>, ancho: i32) -> (String, String) {
    let mut lineas = [String::new(), String::new()];
    let mut indice = 0usize;
    let mut truncado = false;
    for caracter in nombre.chars() {
        let candidato = alloc::format!("{}{}", lineas[indice], caracter);
        if medir_texto(&candidato, font) <= ancho {
            lineas[indice].push(caracter);
        } else if indice == 0 {
            indice = 1;
            if medir_texto(&alloc::format!("{}", caracter), font) <= ancho {
                lineas[indice].push(caracter);
            } else {
                truncado = true;
                break;
            }
        } else {
            truncado = true;
            break;
        }
    }
    if truncado {
        while !lineas[1].is_empty() && medir_texto(&alloc::format!("{}…", lineas[1]), font) > ancho
        {
            lineas[1].pop();
        }
        lineas[1].push('…');
    }
    (
        core::mem::take(&mut lineas[0]),
        core::mem::take(&mut lineas[1]),
    )
}

pub(crate) fn medir_texto(texto: &str, font: Option<&FuenteSuave>) -> i32 {
    let Some(font) = font else {
        return gfx::text_width(texto, 1);
    };
    // Debe medir con la misma escala y el mismo redondeo que
    // CanvasRenderer::texto_suave_tamano. La tabla de avances está guardada a
    // resolución nativa de la máscara; sumarla sin escalar descentraba las
    // etiquetas y exageraba los espacios de la barra de menús.
    let escala = font.alto_base as u64;
    let alto_fuente = font.alto.max(1) as u64;
    let mut ancho = 0i32;
    let mut anterior = None;
    for caracter in texto.chars() {
        let indice = crate::texto::glifo_de(caracter) as usize;
        if let Some(previo) = anterior.filter(|_| font.kerning_addr != 0) {
            let par = previo * 256 + indice;
            let ajuste =
                unsafe { core::ptr::read_volatile((font.kerning_addr + par) as *const i8) as i64 };
            ancho += (ajuste * escala as i64 / alto_fuente as i64) as i32;
        }
        let avance =
            unsafe { core::ptr::read_volatile((font.avances_addr + indice) as *const u8) as u64 }
                .max(1);
        ancho += ((avance * escala + alto_fuente / 2) / alto_fuente) as i32;
        anterior = Some(indice);
    }
    ancho.max(0)
}

fn normalizar_tamano_icono(tamano: u32) -> u32 {
    let limitado = tamano.clamp(TAMANO_ICONO_MIN, TAMANO_ICONO_MAX);
    let pasos = (limitado - TAMANO_ICONO_MIN + PASO_TAMANO_ICONO / 2) / PASO_TAMANO_ICONO;
    (TAMANO_ICONO_MIN + pasos * PASO_TAMANO_ICONO).min(TAMANO_ICONO_MAX)
}

fn ancho_celda(tamano_icono: u32) -> i32 {
    tamano_icono as i32 + ESPACIO_CELDA_X
}

fn alto_celda(tamano_icono: u32) -> i32 {
    tamano_icono as i32 + ESPACIO_CELDA_Y
}

fn limitar_posicion(x: i32, y: i32, ancho: i32, alto: i32, tamano_icono: u32) -> (i32, i32) {
    let ancho_celda = ancho_celda(tamano_icono);
    let alto_celda = alto_celda(tamano_icono);
    let max_x = (ancho - ancho_celda - MARGEN).max(0);
    let min_y = renderer::ALTO_BARRA_VENTANA as i32 + 8;
    let max_y = (alto - crate::dock::alto_reservado(alto) - alto_celda - MARGEN).max(min_y);
    (x.clamp(0, max_x), y.clamp(min_y, max_y))
}

fn area_cuadricula(ancho: i32, alto: i32, tamano_icono: u32) -> (i32, i32, usize, usize) {
    let arriba = renderer::ALTO_BARRA_VENTANA as i32 + 8;
    let celda_x = ancho_celda(tamano_icono);
    let celda_y = alto_celda(tamano_icono);
    let abajo = (alto - crate::dock::alto_reservado(alto) - MARGEN).max(arriba + celda_y);
    let filas = (((abajo - arriba) / celda_y).max(1)) as usize;
    let columnas = (((ancho - 2 * MARGEN) / celda_x).max(1)) as usize;
    (arriba, abajo, filas, columnas)
}

fn total_celdas(ancho: i32, alto: i32, tamano_icono: u32) -> usize {
    let (_, _, filas, columnas) = area_cuadricula(ancho, alto, tamano_icono);
    filas.saturating_mul(columnas).max(1)
}

fn posicion_celda(indice: usize, ancho: i32, alto: i32, tamano_icono: u32) -> (i32, i32) {
    let (arriba, _, filas, columnas) = area_cuadricula(ancho, alto, tamano_icono);
    let indice = indice.min(filas.saturating_mul(columnas).saturating_sub(1));
    let columna = indice / filas;
    let fila = indice % filas;
    let celda_x = ancho_celda(tamano_icono);
    let celda_y = alto_celda(tamano_icono);
    let x = (ancho - MARGEN - celda_x - columna as i32 * celda_x).max(MARGEN);
    (x, arriba + fila as i32 * celda_y)
}

fn buscar_hueco(
    desde: usize,
    elementos: &[Elemento],
    ancho: i32,
    alto: i32,
    tamano_icono: u32,
) -> (i32, i32) {
    let total = total_celdas(ancho, alto, tamano_icono);
    for paso in 0..total {
        let pos = posicion_celda(desde.saturating_add(paso), ancho, alto, tamano_icono);
        if !elementos.iter().any(|item| (item.x, item.y) == pos) {
            return pos;
        }
    }
    posicion_celda(desde, ancho, alto, tamano_icono)
}

fn celda_mas_cercana(
    x: i32,
    y: i32,
    ancho: i32,
    alto: i32,
    tamano_icono: u32,
    ocupadas: &[(i32, i32)],
    total: usize,
) -> Option<(i32, i32)> {
    let mut mejor = None;
    let mut distancia = i64::MAX;
    for indice in 0..total {
        let pos = posicion_celda(indice, ancho, alto, tamano_icono);
        if ocupadas.contains(&pos) {
            continue;
        }
        let dx = (pos.0 - x) as i64;
        let dy = (pos.1 - y) as i64;
        let d = dx * dx + dy * dy;
        if d < distancia {
            mejor = Some(pos);
            distancia = d;
        }
    }
    mejor
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn decodificar_hex(texto: &str) -> Option<String> {
    if texto.len() % 2 != 0 || texto.len() / 2 > fs::NOMBRE_MAX {
        return None;
    }
    let bytes = texto.as_bytes();
    let mut nombre = Vec::with_capacity(bytes.len() / 2);
    for pareja in bytes.chunks_exact(2) {
        let digito = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        };
        nombre.push((digito(pareja[0])? << 4) | digito(pareja[1])?);
    }
    String::from_utf8(nombre).ok()
}
