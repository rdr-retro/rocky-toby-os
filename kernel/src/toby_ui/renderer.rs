//! Renderer de Toby UI (Fase 5).
//!
//! El trait `Renderer` es el contrato: el layout decide DÓNDE va cada cosa y
//! el renderer decide CÓMO se pinta. La implementación inicial,
//! `CanvasRenderer`, pinta sobre el `gfx::Canvas` de VESA que ya existe.
//!
//! El trait no conoce `gfx`: permite sustituir la salida (Kira, consola,
//! un renderer de pruebas) sin tocar el resto del lenguaje.

use alloc::vec::Vec;

use super::component;
use super::document::{Nodo, PropValue, ValidatedDocument};
use super::kit;
use super::layout::{self, Disposicion, Rect, ALTO_ITEM, ALTO_LINEA};
use super::resource::Imagen;
use super::theme::{self, Estado, Tema};
use crate::bios;
use crate::gfx::{self, Canvas};

/// Radio compartido por las ventanas de Kira y la vista previa de AppKit.
pub const RADIO_ESQUINA_VENTANA: u32 = 12;
/// Alto de la barra superior común a Kira y al lienzo de AppKit.
pub const ALTO_BARRA_VENTANA: u32 = 28;

/// Acción representada por cada zona de la píldora verde al pasar el cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlVentana {
    Cerrar,
    Maximizar,
    Minimizar,
}

/// Control opcional que una app añade a su propia barra de título AppKit.
#[derive(Clone, Copy)]
pub struct BotonCabecera<'a> {
    /// Icono del tema o del bundle de la app.
    pub imagen: Option<&'a Imagen>,
    /// Si está inactivo, AppKit atenúa tanto la esfera como el icono.
    pub habilitado: bool,
}

/// Configuración por ventana de la cabecera AppKit. Los botones se ordenan
/// en una fila adicional debajo de la píldora y pueden ampliarse por app.
#[derive(Clone, Copy)]
pub struct CabeceraVentana<'a> {
    pub alto: u32,
    pub botones: &'a [BotonCabecera<'a>],
}

/// Fuente antialias empaquetada, con métricas proporcionales y kerning.
#[derive(Clone, Copy)]
pub struct FuenteSuave {
    pub addr: usize,
    pub avances_addr: usize,
    pub kerning_addr: usize,
    pub ancho: usize,
    pub alto: usize,
    /// Altura a la que se presenta sin escalar (16 px UI, 24 px TextEdit).
    pub alto_base: usize,
    pub bytes_por_glifo: usize,
}

/// Máscara de cobertura precalculada al tamaño final de presentación.
#[derive(Clone, Copy)]
pub struct MascaraSuave {
    pub addr: usize,
    pub ancho: u32,
    pub alto: u32,
}

/// Coordenadas de muestra bilineal para una máscara de glifo.
fn coordenada_mascara(indice: u32, destino: u32, origen: usize) -> (usize, usize, u32) {
    let ultimo = origen.saturating_sub(1) as u64;
    let centro = (((indice as u64 * 2 + 1) * origen as u64 * 32_768) / destino as u64)
        .saturating_sub(32_768)
        .min(ultimo << 16);
    let primero = (centro >> 16) as usize;
    let segundo = (primero + 1).min(origen - 1);
    (primero, segundo, (centro & 0xFFFF) as u32)
}

/// Interpola cuatro coberturas en punto fijo, sin introducir píxeles duros.
fn interpolar_cobertura(a: u8, b: u8, c: u8, d: u8, fx: u32, fy: u32) -> u8 {
    let fx = fx as u64;
    let fy = fy as u64;
    let ix = 65_536 - fx;
    let iy = 65_536 - fy;
    let superior = (a as u64 * ix + b as u64 * fx + 32_768) >> 16;
    let inferior = (c as u64 * ix + d as u64 * fx + 32_768) >> 16;
    ((superior * iy + inferior * fy + 32_768) >> 16) as u8
}

/// Salida gráfica abstracta de Toby UI.
pub trait Renderer {
    /// Borra toda la superficie.
    fn limpiar(&mut self, color: u32);
    /// Rellena un rectángulo.
    fn relleno(&mut self, rect: Rect, color: u32);
    /// Dibuja el contorno de 1 px de un rectángulo.
    fn borde(&mut self, rect: Rect, color: u32);
    /// Dibuja texto con la fuente bitmap del tema.
    fn texto(&mut self, x: i32, y: i32, s: &str, color: u32, escala: i32);
    /// Dibuja texto usando un tamaño CSS en píxeles.
    fn texto_tamano(&mut self, x: i32, y: i32, s: &str, color: u32, alto: u32) {
        self.texto(
            x,
            y,
            s,
            color,
            ((alto + ALTO_LINEA / 2) / ALTO_LINEA).max(1) as i32,
        );
    }
    /// Mide con la métrica disponible para el mismo tamaño que pintará el host.
    fn medir_texto_tamano(&self, s: &str, alto: u32) -> u32 {
        layout::ancho_texto_css(s, alto)
    }
    /// Refuerzo tipográfico para encabezados; no añade piel de control.
    fn texto_tamano_negrita(&mut self, x: i32, y: i32, s: &str, color: u32, alto: u32) {
        self.texto_tamano(x, y, s, color, alto);
        self.texto_tamano(x + 1, y, s, color, alto);
    }
    /// Apila un área de recorte (intersección con la anterior).
    fn recorte(&mut self, rect: Rect);
    /// Desapila el último área de recorte.
    fn recorte_fin(&mut self);
    /// Dibuja una imagen escalada a `rect`, respetando el alfa y el recorte.
    ///
    /// El escalado **se filtra**: no puede dejar dientes de sierra (ver
    /// [`muestra_escalada`]). Por defecto no pinta nada: las salidas que no
    /// conocen imágenes (la consola, un renderer de pruebas) siguen valiendo sin
    /// tocar nada.
    fn imagen(&mut self, _rect: Rect, _img: &Imagen) {}
    /// Dibuja una sección de una imagen en `rect`, escalando sólo esa sección.
    /// `fuente` usa coordenadas en píxeles dentro del PNG; se emplea para
    /// montar piezas 3-slice sin estirar sus extremos.
    fn imagen_fragmento(&mut self, rect: Rect, img: &Imagen, fuente: Rect) {
        let _ = fuente;
        self.imagen(rect, img);
    }
    /// Dibuja la máscara alfa de una imagen con una tinta plana.
    ///
    /// El renderer de framebuffer lo usa para mantener legibles en azul los
    /// iconos negros del segmented seleccionado.
    fn imagen_tintada(&mut self, rect: Rect, img: &Imagen, color: u32) {
        let _ = color;
        self.imagen(rect, img);
    }
    /// Igual que [`Renderer::imagen`], pero la pieza se dibuja en **espejo**
    /// (volteada en horizontal, el `scaleX(-1)` del CSS del kit AQUA).
    ///
    /// El montaje AQUA pone la MISMA pieza en los dos extremos de un
    /// segmentado, de un campo o de un disparador, girada en el derecho; sin
    /// esta primitiva no se puede reproducir el ensamblaje del HTML. Por
    /// defecto la pinta sin girar (como `imagen`, las salidas que ignoran
    /// imágenes no se enteran).
    fn imagen_espejo(&mut self, rect: Rect, img: &Imagen) {
        self.imagen(rect, img)
    }
    /// Dibuja una imagen aplicando los filtros CSS de AQUA (brillo,
    /// saturación y opacidad, expresados como porcentajes).
    fn imagen_filtrada(
        &mut self,
        rect: Rect,
        img: &Imagen,
        _brillo: u16,
        _saturacion: u16,
        _opacidad: u8,
    ) {
        self.imagen(rect, img)
    }
    /// Igual que [`Renderer::imagen_filtrada`], volteando la imagen.
    fn imagen_espejo_filtrada(
        &mut self,
        rect: Rect,
        img: &Imagen,
        brillo: u16,
        saturacion: u16,
        opacidad: u8,
    ) {
        let _ = (brillo, saturacion, opacidad);
        self.imagen_espejo(rect, img)
    }
    /// Rellena un círculo de `radio` px centrado en (`cx`, `cy`).
    ///
    /// El kit AQUA no trae piezas circulares para todo (el botón redondo, el
    /// pulgar redondo del slider), así que el círculo es primitiva del trait.
    /// Por defecto no pinta nada, como `imagen`.
    fn circulo(&mut self, _cx: i32, _cy: i32, _radio: i32, _color: u32) {}
    /// Presenta lo dibujado (por defecto, nada que hacer).
    fn presentar(&mut self) {}
}

/// Renderer sobre el framebuffer VESA de Rocky.
pub struct CanvasRenderer<'a, 'f> {
    lienzo: &'a mut Canvas,
    /// Fuente bitmap del BIOS; si no hay, no se dibuja texto.
    fuente: Option<&'f bios::Font>,
    /// Fuente SF Pro antialias opcional para las etiquetas del sistema.
    fuente_suave: Option<&'f FuenteSuave>,
    /// Pila inline: repintar controles animados no debe asignar memoria por
    /// fotograma. El parser limita la profundidad del árbol a 32 niveles.
    recortes: [Rect; super::parser::MAX_PROFUNDIDAD + 2],
    profundidad_recortes: usize,
    recortes_excedentes: usize,
}

impl<'a, 'f> CanvasRenderer<'a, 'f> {
    /// Nuevo renderer sobre un lienzo, con fuente opcional.
    pub fn nuevo(lienzo: &'a mut Canvas, fuente: Option<&'f bios::Font>) -> Self {
        CanvasRenderer {
            lienzo,
            fuente,
            fuente_suave: None,
            recortes: [Rect::VACIO; super::parser::MAX_PROFUNDIDAD + 2],
            profundidad_recortes: 0,
            recortes_excedentes: 0,
        }
    }

    /// Renderer con la misma caché tipográfica SF Pro que usa Kira.
    pub fn nuevo_con_fuente_suave(
        lienzo: &'a mut Canvas,
        fuente: Option<&'f bios::Font>,
        fuente_suave: &'f FuenteSuave,
    ) -> Self {
        let mut renderer = Self::nuevo(lienzo, fuente);
        renderer.fuente_suave = Some(fuente_suave);
        renderer
    }

    fn medir_suave(&self, s: &str, alto: u32, fuente: &FuenteSuave) -> u32 {
        let mut ancho = 0i32;
        let mut anterior = None;
        for caracter in s.chars() {
            let indice = crate::texto::glifo_de(caracter) as usize;
            if let Some(previo) = anterior.filter(|_| fuente.kerning_addr != 0) {
                let par = previo * 256 + indice;
                ancho += unsafe {
                    core::ptr::read_volatile((fuente.kerning_addr + par) as *const i8) as i32
                };
            }
            ancho += unsafe {
                core::ptr::read_volatile((fuente.avances_addr + indice) as *const u8) as i32
            }
            .max(1);
            anterior = Some(indice);
        }
        ((ancho.max(0) as u64 * alto as u64 + fuente.alto as u64 / 2) / fuente.alto.max(1) as u64)
            as u32
    }

    fn texto_suave_tamano(
        &mut self,
        x: i32,
        y: i32,
        s: &str,
        color: u32,
        alto: u32,
        negrita: bool,
        fuente: &FuenteSuave,
    ) {
        if alto == 0 || fuente.ancho == 0 || fuente.alto == 0 || fuente.bytes_por_glifo == 0 {
            return;
        }
        let mut cx = x;
        let escala = alto as u64;
        let mut anterior = None;
        for caracter in s.chars() {
            let indice = crate::texto::glifo_de(caracter) as usize;
            if let Some(previo) = anterior.filter(|_| fuente.kerning_addr != 0) {
                let par = previo * 256 + indice;
                let ajuste = unsafe {
                    core::ptr::read_volatile((fuente.kerning_addr + par) as *const i8) as i32
                };
                cx += (ajuste as i64 * escala as i64 / fuente.alto as i64) as i32;
            }

            let glifo = fuente.addr + indice * fuente.bytes_por_glifo;
            let ancho_glifo = ((fuente.ancho as u64 * escala + fuente.alto as u64 / 2)
                / fuente.alto as u64)
                .max(1) as u32;
            for dy in 0..alto {
                let (sy0, sy1, fy) = coordenada_mascara(dy, alto, fuente.alto);
                for dx in 0..ancho_glifo {
                    let (sx0, sx1, fx) = coordenada_mascara(dx, ancho_glifo, fuente.ancho);
                    let cobertura = unsafe {
                        interpolar_cobertura(
                            core::ptr::read_volatile(
                                (glifo + sy0 * fuente.ancho + sx0) as *const u8,
                            ),
                            core::ptr::read_volatile(
                                (glifo + sy0 * fuente.ancho + sx1) as *const u8,
                            ),
                            core::ptr::read_volatile(
                                (glifo + sy1 * fuente.ancho + sx0) as *const u8,
                            ),
                            core::ptr::read_volatile(
                                (glifo + sy1 * fuente.ancho + sx1) as *const u8,
                            ),
                            fx,
                            fy,
                        )
                    };
                    if cobertura == 0 {
                        continue;
                    }
                    let px = cx + dx as i32;
                    let py = y + dy as i32;
                    if !self.punto_valido(px, py) {
                        continue;
                    }
                    if cobertura == u8::MAX {
                        self.lienzo.put(px, py, color);
                    } else {
                        self.lienzo.blend(px, py, color, cobertura as u32);
                    }
                    if negrita && self.punto_valido(px + 1, py) {
                        if cobertura == u8::MAX {
                            self.lienzo.put(px + 1, py, color);
                        } else {
                            self.lienzo.blend(px + 1, py, color, cobertura as u32);
                        }
                    }
                }
            }
            let avance = unsafe {
                core::ptr::read_volatile((fuente.avances_addr + indice) as *const u8) as u64
            };
            cx += ((avance * escala + fuente.alto as u64 / 2) / fuente.alto as u64) as i32;
            anterior = Some(indice);
        }
    }

    /// Dibuja glifos antialias y aplica el kerning OpenType del par anterior.
    pub fn texto_suavizado(&mut self, x: i32, y: i32, s: &str, color: u32, fuente: &FuenteSuave) {
        // Las máscaras de alta resolución se filtran al tamaño tipográfico
        // nominal de cada fuente; TextEdit conserva sus 24 px de altura.
        self.texto_suave_tamano(x, y, s, color, fuente.alto_base as u32, false, fuente);
    }

    /// Dibuja una máscara ya rasterizada a su tamaño de destino, sin ampliarla.
    pub fn mascara_suave(&mut self, x: i32, y: i32, mascara: &MascaraSuave, color: u32) {
        for dy in 0..mascara.alto {
            for dx in 0..mascara.ancho {
                let cobertura = unsafe {
                    core::ptr::read_volatile(
                        (mascara.addr + (dy * mascara.ancho + dx) as usize) as *const u8,
                    )
                };
                if cobertura == 0 {
                    continue;
                }
                let px = x + dx as i32;
                let py = y + dy as i32;
                if !self.punto_valido(px, py) {
                    continue;
                }
                if cobertura == u8::MAX {
                    self.lienzo.put(px, py, color);
                } else {
                    self.lienzo.blend(px, py, color, cobertura as u32);
                }
            }
        }
    }

    /// Área de recorte activa (ninguna = todo el lienzo).
    fn clip(&self) -> Option<Rect> {
        self.profundidad_recortes
            .checked_sub(1)
            .and_then(|i| self.recortes.get(i).copied())
    }

    /// Intersección de un rectángulo con el recorte activo.
    fn aplicar_clip(&self, rect: Rect) -> Rect {
        match self.clip() {
            Some(c) => rect.recortar(c),
            None => rect,
        }
    }

    /// ¿El punto está dentro del recorte activo?
    fn punto_valido(&self, x: i32, y: i32) -> bool {
        match self.clip() {
            Some(c) => c.contiene(x, y),
            None => true,
        }
    }

    /// Dibuja una imagen escalada a `rect`, respetando alfa y recorte.
    ///
    /// `volteada` la dibuja en espejo horizontal: el píxel de destino `dx`
    /// toma el de la fuente `dw - 1 - dx`, que es exactamente lo que hace
    /// `scaleX(-1)` del CSS con la misma imagen.
    fn pintar_imagen(&mut self, rect: Rect, img: &Imagen, volteada: bool) {
        self.pintar_imagen_filtrada(rect, img, volteada, 100, 100, 100);
    }

    fn pintar_imagen_fragmento(&mut self, rect: Rect, img: &Imagen, fuente: Rect) {
        if rect.w == 0 || rect.h == 0 || img.ancho == 0 || img.alto == 0 {
            return;
        }
        let limites = Rect {
            x: 0,
            y: 0,
            w: img.ancho,
            h: img.alto,
        };
        let fuente = fuente.recortar(limites);
        if fuente.w == 0 || fuente.h == 0 {
            return;
        }
        let recorte = self.aplicar_clip(rect);
        if recorte.w == 0 || recorte.h == 0 {
            return;
        }
        let (dw, dh) = (rect.w as i32, rect.h as i32);
        for dy in 0..recorte.h as i32 {
            let y = recorte.y + dy;
            for dx in 0..recorte.w as i32 {
                let x = recorte.x + dx;
                let (color, alpha) =
                    muestra_fragmento_escalada(img, fuente, dw, dh, x - rect.x, y - rect.y);
                if alpha == 0 {
                    continue;
                }
                if alpha >= 255 {
                    self.lienzo.put(x, y, color);
                } else {
                    self.lienzo.blend(x, y, color, alpha);
                }
            }
        }
    }

    fn pintar_imagen_filtrada(
        &mut self,
        rect: Rect,
        img: &Imagen,
        volteada: bool,
        brillo: u16,
        saturacion: u16,
        opacidad: u8,
    ) {
        if rect.w == 0 || rect.h == 0 || img.ancho == 0 || img.alto == 0 {
            return;
        }
        let r = self.aplicar_clip(rect);
        if r.w == 0 || r.h == 0 {
            return;
        }
        // El escalado lo manda el rectángulo entero, no el recortado: cada
        // píxel se muestrea por su sitio, se pinte o no.
        let (dw, dh) = (rect.w as i32, rect.h as i32);
        for dy in 0..r.h as i32 {
            let y = r.y + dy;
            for dx in 0..r.w as i32 {
                let x = r.x + dx;
                // El clip puede empezar dentro de la imagen (p. ej. cover con
                // destino parcialmente fuera de pantalla): muestrear desde el
                // origen del rectángulo conserva el encuadre original.
                let (dx_img, dy_img) = (x - rect.x, y - rect.y);
                let sx = if volteada { dw - 1 - dx_img } else { dx_img };
                let (color, a) = muestra_escalada(img, dw, dh, sx, dy_img);
                let color = filtrar_color(color, brillo, saturacion);
                let a = (a * opacidad as u32 + 50) / 100;
                if a == 0 {
                    continue;
                }
                if a >= 255 {
                    self.lienzo.put(x, y, color);
                } else {
                    self.lienzo.blend(x, y, color, a);
                }
            }
        }
    }

    fn pintar_imagen_tintada(&mut self, rect: Rect, img: &Imagen, color: u32) {
        if rect.w == 0 || rect.h == 0 || img.ancho == 0 || img.alto == 0 {
            return;
        }
        let r = self.aplicar_clip(rect);
        if r.w == 0 || r.h == 0 {
            return;
        }
        let (dw, dh) = (rect.w as i32, rect.h as i32);
        let color = color & 0x00FF_FFFF;
        for dy in 0..r.h as i32 {
            let y = r.y + dy;
            for dx in 0..r.w as i32 {
                let x = r.x + dx;
                let (_, alpha) = muestra_escalada(img, dw, dh, x - rect.x, y - rect.y);
                if alpha == 0 {
                    continue;
                }
                if alpha >= 255 {
                    self.lienzo.put(x, y, color);
                } else {
                    self.lienzo.blend(x, y, color, alpha);
                }
            }
        }
    }
}

impl<'a, 'f> Renderer for CanvasRenderer<'a, 'f> {
    fn limpiar(&mut self, color: u32) {
        self.lienzo
            .fill_rect(0, 0, self.lienzo.width, self.lienzo.height, color);
    }

    fn relleno(&mut self, rect: Rect, color: u32) {
        let r = self.aplicar_clip(rect);
        if r.w == 0 || r.h == 0 {
            return;
        }
        self.lienzo
            .fill_rect(r.x, r.y, r.w as i32, r.h as i32, color);
    }

    fn borde(&mut self, rect: Rect, color: u32) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        let d = rect.der() - 1;
        let b = rect.abajo() - 1;
        // Los cuatro lados, recortados al área activa: el anillo de foco de un
        // control se pinta FUERA de su rectángulo y no puede salirse del
        // recorte del contenedor (antes el borde ignoraba el clip).
        let (x1, x2, y1, y2) = match self.clip() {
            Some(c) => (
                rect.x.max(c.x),
                d.min(c.der() - 1),
                rect.y.max(c.y),
                b.min(c.abajo() - 1),
            ),
            None => (rect.x, d, rect.y, b),
        };
        if x1 <= x2 {
            if rect.y >= y1 && rect.y <= y2 {
                self.lienzo.hline(x1, x2, rect.y, color);
            }
            if b >= y1 && b <= y2 {
                self.lienzo.hline(x1, x2, b, color);
            }
        }
        if y1 <= y2 {
            if rect.x >= x1 && rect.x <= x2 {
                self.lienzo.vline(rect.x, y1, y2, color);
            }
            if d >= x1 && d <= x2 {
                self.lienzo.vline(d, y1, y2, color);
            }
        }
    }

    fn texto(&mut self, x: i32, y: i32, s: &str, color: u32, escala: i32) {
        let font = match self.fuente {
            Some(f) => f,
            None => return,
        };
        let esc = escala.max(1);
        let alto = font.bytes_per_char as i32;
        let mut cx = x;
        for c in s.chars() {
            let ch = crate::texto::glifo_de(c) as usize;
            let glifo = font.addr + ch * font.bytes_per_char;
            for fila in 0..alto {
                let bits =
                    unsafe { core::ptr::read_volatile((glifo + fila as usize) as *const u8) };
                if bits == 0 {
                    continue;
                }
                for col in 0..gfx::FONT_W {
                    if bits & (0x80 >> col) == 0 {
                        continue;
                    }
                    let px = cx + col * esc;
                    let py = y + fila * esc;
                    for dy in 0..esc {
                        for dx in 0..esc {
                            let sx = px + dx;
                            let sy = py + dy;
                            if self.punto_valido(sx, sy) {
                                self.lienzo.put(sx, sy, color);
                            }
                        }
                    }
                }
            }
            cx += gfx::FONT_W * esc;
        }
    }

    fn texto_tamano(&mut self, x: i32, y: i32, s: &str, color: u32, alto_css: u32) {
        if let Some(fuente) = self.fuente_suave {
            self.texto_suave_tamano(x, y, s, color, alto_css, false, &fuente);
            return;
        }
        let font = match self.fuente {
            Some(f) => f,
            None => return,
        };
        let alto = alto_css.max(1) as i32;
        let ancho = ((alto_css.saturating_mul(52) + 50) / 100).max(1) as i32;
        let origen_alto = font.bytes_per_char as i32;
        let mut cx = x;
        for c in s.chars() {
            let ch = crate::texto::glifo_de(c) as usize;
            let glifo = font.addr + ch * font.bytes_per_char;
            for dy in 0..alto {
                let fila = (dy * origen_alto / alto).min(origen_alto - 1);
                let bits =
                    unsafe { core::ptr::read_volatile((glifo + fila as usize) as *const u8) };
                if bits == 0 {
                    continue;
                }
                for dx in 0..ancho {
                    let col = (dx * gfx::FONT_W / ancho).min(gfx::FONT_W - 1);
                    if bits & (0x80 >> col) != 0 {
                        let px = cx + dx;
                        let py = y + dy;
                        if self.punto_valido(px, py) {
                            self.lienzo.put(px, py, color);
                        }
                    }
                }
            }
            cx += ancho;
        }
    }

    fn medir_texto_tamano(&self, s: &str, alto: u32) -> u32 {
        self.fuente_suave
            .map(|fuente| self.medir_suave(s, alto, fuente))
            .unwrap_or_else(|| layout::ancho_texto_css(s, alto))
    }

    fn texto_tamano_negrita(&mut self, x: i32, y: i32, s: &str, color: u32, alto: u32) {
        if let Some(fuente) = self.fuente_suave {
            self.texto_suave_tamano(x, y, s, color, alto, true, &fuente);
        } else {
            self.texto_tamano(x, y, s, color, alto);
            self.texto_tamano(x + 1, y, s, color, alto);
        }
    }

    fn recorte(&mut self, rect: Rect) {
        let nuevo = match self.clip() {
            Some(c) => rect.recortar(c),
            None => rect,
        };
        if self.profundidad_recortes < self.recortes.len() {
            self.recortes[self.profundidad_recortes] = nuevo;
            self.profundidad_recortes += 1;
        } else {
            self.recortes_excedentes = self.recortes_excedentes.saturating_add(1);
        }
    }

    fn recorte_fin(&mut self) {
        if self.recortes_excedentes != 0 {
            self.recortes_excedentes -= 1;
        } else {
            self.profundidad_recortes = self.profundidad_recortes.saturating_sub(1);
        }
    }

    fn imagen(&mut self, rect: Rect, img: &Imagen) {
        self.pintar_imagen(rect, img, false);
    }

    fn imagen_fragmento(&mut self, rect: Rect, img: &Imagen, fuente: Rect) {
        self.pintar_imagen_fragmento(rect, img, fuente);
    }

    fn imagen_tintada(&mut self, rect: Rect, img: &Imagen, color: u32) {
        self.pintar_imagen_tintada(rect, img, color);
    }

    fn imagen_espejo(&mut self, rect: Rect, img: &Imagen) {
        self.pintar_imagen(rect, img, true);
    }

    fn imagen_filtrada(
        &mut self,
        rect: Rect,
        img: &Imagen,
        brillo: u16,
        saturacion: u16,
        opacidad: u8,
    ) {
        self.pintar_imagen_filtrada(rect, img, false, brillo, saturacion, opacidad);
    }

    fn imagen_espejo_filtrada(
        &mut self,
        rect: Rect,
        img: &Imagen,
        brillo: u16,
        saturacion: u16,
        opacidad: u8,
    ) {
        self.pintar_imagen_filtrada(rect, img, true, brillo, saturacion, opacidad);
    }

    fn circulo(&mut self, cx: i32, cy: i32, radio: i32, color: u32) {
        if radio <= 0 {
            return;
        }
        // Disco relleno tramo a tramo: cada fila es un hline de media anchura
        // sqrt(r² - dy²). Pasa por `relleno`, así que respeta el recorte.
        let r2 = (radio as i64) * (radio as i64);
        for dy in -radio..=radio {
            let dentro = r2 - (dy as i64) * (dy as i64);
            let half = if dentro <= 0 {
                0
            } else {
                raiz_entera(dentro as u64) as i32
            };
            let y = cy + dy;
            if half == 0 {
                self.relleno(
                    Rect {
                        x: cx,
                        y,
                        w: 1,
                        h: 1,
                    },
                    color,
                );
            } else {
                self.relleno(
                    Rect {
                        x: cx - half,
                        y,
                        w: (half * 2 + 1) as u32,
                        h: 1,
                    },
                    color,
                );
            }
        }
    }

    fn presentar(&mut self) {
        self.lienzo.present();
    }
}

/// Muestrea una imagen escalada a `dw` x `dh` en el píxel de destino `(dx, dy)`.
///
/// Devuelve `(color RGB, alfa)`. Pintar con vecino más próximo deja **dientes de
/// sierra**: al reducir —las pieles AQUA pasan de 258 × 409 a 15 × 24— se salta
/// 16 de cada 17 píxeles de la fuente y el borde redondeado sale a escalones de
/// 255 a 0. Aquí se hace lo que toca en cada caso:
///
///   * si el píxel de destino cubre **varios** de la fuente (se reduce), se
///     promedia el área que le toca (filtro de caja);
///   * si cubre **uno o menos** (se amplía, o tamaño 1:1), se interpola
///     bilinealmente entre sus cuatro vecinos.
///
/// En los dos casos el alfa pesa **premultiplicado**, que es lo único que no
/// ensucia el borde: un píxel transparente (RGB 0,0,0) al lado de uno blanco
/// opaco da blanco al 50 %, no gris. Sólo enteros, y como mucho 4 × 4 lecturas
/// por píxel de destino, para que el kernel no lo note al repintar.
///
/// Trabaja sobre píxeles crudos en vez de sobre una [`Imagen`] para que la
/// reutilice el visor del escritorio (`crate::visor`): el muestreo está
/// implementado una sola vez y con sus pruebas.
pub fn muestra(
    pixeles: &[u32],
    sw: i32,
    sh: i32,
    dw: i32,
    dh: i32,
    dx: i32,
    dy: i32,
) -> (u32, u32) {
    if sw <= 0 || sh <= 0 {
        return (0, 0);
    }
    let (dw, dh) = (dw.max(1), dh.max(1));
    // Franja de la fuente que cubre este píxel de destino: [xa, xb) x [ya, yb).
    let franja = |d: i32, dst: i32, src: i32| -> (i32, i32) {
        let a = ((d as i64 * src as i64) / dst as i64) as i32;
        let b = (((d as i64 + 1) * src as i64 + dst as i64 - 1) / dst as i64) as i32;
        (a.clamp(0, src - 1), b.clamp(a + 1, src))
    };
    let (xa, xb) = franja(dx, dw, sw);
    let (ya, yb) = franja(dy, dh, sh);
    if xb - xa == 1 && yb - ya == 1 {
        return interpolada(pixeles, sw, sh, dw, dh, dx, dy);
    }
    // Filtro de caja, con como mucho 4 muestras por eje: una franja de 4 o
    // menos se recorre entera, y una de 17 (el caso de las pieles) se recorre
    // con 4.
    let (px, py) = ((xb - xa + 3) / 4, (yb - ya + 3) / 4);
    let (mut sa, mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut y = ya;
    while y < yb {
        let mut x = xa;
        while x < xb {
            let p = pixeles[y as usize * sw as usize + x as usize] as u64;
            let a = p >> 24;
            sa += a;
            sr += ((p >> 16) & 0xFF) * a;
            sg += ((p >> 8) & 0xFF) * a;
            sb += (p & 0xFF) * a;
            n += 1;
            x += px;
        }
        y += py;
    }
    mezclada(sr, sg, sb, sa, n)
}

/// Igual que [`muestra`], pero para una [`Imagen`] del motor.
pub fn muestra_escalada(img: &Imagen, dw: i32, dh: i32, dx: i32, dy: i32) -> (u32, u32) {
    muestra(
        &img.pixeles,
        img.ancho as i32,
        img.alto as i32,
        dw,
        dh,
        dx,
        dy,
    )
}

/// Muestrea un fragmento de un PNG, manteniendo el filtrado bilineal dentro
/// de sus propios bordes para que un slice no mezcle colores del vecino.
fn muestra_fragmento_escalada(
    img: &Imagen,
    fuente: Rect,
    dw: i32,
    dh: i32,
    dx: i32,
    dy: i32,
) -> (u32, u32) {
    let fuente = fuente.recortar(Rect {
        x: 0,
        y: 0,
        w: img.ancho,
        h: img.alto,
    });
    if fuente.w == 0 || fuente.h == 0 {
        return (0, 0);
    }
    interpolada_region(
        &img.pixeles,
        img.ancho as i32,
        dw.max(1),
        dh.max(1),
        dx,
        dy,
        fuente,
    )
}

/// Interpolación bilineal del píxel `(dx, dy)` de una imagen ampliada (o igual).
///
/// La posición de la fuente va en punto fijo 8.8 y centrada en el píxel, de
/// forma que a tamaño 1:1 sale exactamente el mismo píxel, sin reblandecer.
fn interpolada(
    pixeles: &[u32],
    sw: i32,
    sh: i32,
    dw: i32,
    dh: i32,
    dx: i32,
    dy: i32,
) -> (u32, u32) {
    interpolada_region(
        pixeles,
        sw,
        dw,
        dh,
        dx,
        dy,
        Rect {
            x: 0,
            y: 0,
            w: sw as u32,
            h: sh as u32,
        },
    )
}

fn interpolada_region(
    pixeles: &[u32],
    sw: i32,
    dw: i32,
    dh: i32,
    dx: i32,
    dy: i32,
    fuente: Rect,
) -> (u32, u32) {
    let centro = |d: i32, dst: i32, src: i32| -> i32 {
        (((2 * d as i64 + 1) * src as i64 * 128) / dst as i64 - 128).max(0) as i32
    };
    let (sx, sy) = (
        centro(dx, dw, fuente.w as i32),
        centro(dy, dh, fuente.h as i32),
    );
    let (x_inicio, y_inicio) = (fuente.x, fuente.y);
    let x0 = x_inicio + (sx >> 8).min(fuente.w as i32 - 1);
    let y0 = y_inicio + (sy >> 8).min(fuente.h as i32 - 1);
    let x1 = (x0 + 1).min(x_inicio + fuente.w as i32 - 1);
    let y1 = (y0 + 1).min(y_inicio + fuente.h as i32 - 1);
    let fx = (sx & 0xFF) as u64;
    let fy = (sy & 0xFF) as u64;
    let esquinas = [
        (x0, y0, (256 - fx) * (256 - fy)),
        (x1, y0, fx * (256 - fy)),
        (x0, y1, (256 - fx) * fy),
        (x1, y1, fx * fy),
    ];
    let (mut sa, mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64, 0u64);
    for (x, y, w) in esquinas {
        let p = pixeles[y as usize * sw as usize + x as usize] as u64;
        let a = p >> 24;
        sa += a * w;
        sr += ((p >> 16) & 0xFF) * a * w;
        sg += ((p >> 8) & 0xFF) * a * w;
        sb += (p & 0xFF) * a * w;
    }
    mezclada(sr, sg, sb, sa, 256 * 256)
}

/// Convierte promedios con alfa premultiplicado en `(color, alfa)`.
///
/// `sr`/`sg`/`sb` son las sumas de canal por alfa, `sa` la suma de alfas y `n`
/// el número de muestras (las pesadas cuentan como una sola: la media de alfas
/// es `sa / n`).
fn mezclada(sr: u64, sg: u64, sb: u64, sa: u64, n: u64) -> (u32, u32) {
    if sa == 0 || n == 0 {
        return (0, 0);
    }
    let color = (((sr / sa) as u32) << 16) | (((sg / sa) as u32) << 8) | ((sb / sa) as u32);
    (color, (sa / n) as u32)
}

/// Raíz cuadrada entera, por sucesión de Newton (sin libm: el kernel no tiene
/// libm y `f64::sqrt` no es const en no_std).
fn raiz_entera(v: u64) -> u64 {
    if v == 0 {
        return 0;
    }
    let mut x = v;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// Dibuja un documento validado ya dispuesto (render estático).
///
/// Recorre los nodos en orden de dibujo (preorden) y pinta según el
/// componente y el tema. Los componentes que todavía no tienen pintura
/// propia simplemente no dibujan nada.
///
/// Los hijos de un contenedor de tamaño FIJO (una `window`, o cualquier nodo
/// con `width`/`height` explícitos) se recortan a su rectángulo. Los
/// contenedores que se ajustan a su contenido no recortan, para no cortar la
/// sombra de sus hijos.
pub fn pintar<R: Renderer>(r: &mut R, doc: &ValidatedDocument, disp: &Disposicion, tema: &Tema) {
    let valores = super::valores::Valores::del_documento(doc);
    pintar_con_estado(r, doc, disp, tema, &|_| Estado::Normal, &valores);
}

/// Igual que [`pintar`], pero cada nodo puede estar en un [`Estado`] distinto
/// y con los valores actuales de los componentes de estado.
///
/// El renderer no conoce el sistema de eventos: quien llama pasa una función
/// que, dado un índice de nodo, devuelve su estado (`event::Interaccion` ya
/// ofrece `estado_de(i)` para eso). `valores` es lo que el usuario ha
/// cambiado desde que el documento se montó: el marcado de un `check`, la
/// opción de un `segmented`, el texto de un `field`...
pub fn pintar_con_estado<R: Renderer, F: Fn(usize) -> Estado>(
    r: &mut R,
    doc: &ValidatedDocument,
    disp: &Disposicion,
    tema: &Tema,
    estado: &F,
    valores: &super::valores::Valores,
) {
    pintar_con_estado_y_cursor(r, doc, disp, tema, estado, valores, None);
}

/// Igual que [`pintar_con_estado`], mostrando los controles de la píldora
/// verde cuando el cursor cae sobre ella. `cursor` usa coordenadas del lienzo.
pub fn pintar_con_estado_y_cursor<R: Renderer, F: Fn(usize) -> Estado>(
    r: &mut R,
    doc: &ValidatedDocument,
    disp: &Disposicion,
    tema: &Tema,
    estado: &F,
    valores: &super::valores::Valores,
    cursor: Option<(i32, i32)>,
) {
    pintar_con_estado_y_cursor_en(
        r,
        doc,
        disp,
        tema,
        estado,
        valores,
        cursor,
        crate::pit::ticks(),
    );
}

/// Igual que [`pintar_con_estado_y_cursor`], con el instante del anfitrión
/// expresado en ticks PIT (100 Hz). Separar el reloj permite que AppKit reuse
/// la misma animación sin que el renderer mantenga estado ni consulte hardware.
pub fn pintar_con_estado_y_cursor_en<R: Renderer, F: Fn(usize) -> Estado>(
    r: &mut R,
    doc: &ValidatedDocument,
    disp: &Disposicion,
    tema: &Tema,
    estado: &F,
    valores: &super::valores::Valores,
    cursor: Option<(i32, i32)>,
    ahora_ticks: u64,
) {
    let pre = doc.preorden();
    let ventana_app = pre
        .get(1)
        .copied()
        .filter(|n| n.def.etiqueta == "window" && n.texto("mode", "fullscreen") == "window");
    let exterior_ventana = 0x000000;
    r.limpiar(if ventana_app.is_some() {
        exterior_ventana
    } else {
        tema.paleta.fondo
    });
    if !pre.is_empty() {
        pintar_nodo(r, &pre, disp, tema, 0, estado, valores, 0, ahora_ticks);
        if let Some(ventana) = ventana_app {
            pintar_cabecera_ventana_con_cursor(
                r,
                disp.rect(1),
                exterior_ventana,
                tema.paleta.borde,
                ventana.texto("title", ""),
                cursor,
            );
        }
        // El menú desplegado de un `popup` pinta ENCIMA de todo lo demás:
        // su sitio en el orden es el último plano.
        pintar_menus(r, &pre, disp, tema, valores);
    }
}

fn pintar_nodo<R: Renderer, F: Fn(usize) -> Estado>(
    r: &mut R,
    pre: &[&Nodo],
    disp: &Disposicion,
    tema: &Tema,
    i: usize,
    estado: &F,
    valores: &super::valores::Valores,
    nivel_box: u8,
    ahora_ticks: u64,
) {
    let n = pre[i];
    let rect = disp.rect(i);
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    match n.def.etiqueta {
        "window" => {
            r.relleno(rect, tema.paleta.fondo);
        }
        "box" => pintar_box(r, n, rect, tema, nivel_box),
        "button" => pintar_boton(r, n, rect, tema, estado(i)),
        "label" => pintar_etiqueta(r, n, rect, tema),
        "progress" => {
            if n.texto("mode", "indeterminate") == "determinate" {
                let value = valores.entero(i).clamp(0, 100) as u8;
                pintar_progreso_determinado(r, tema, rect, value, ahora_ticks);
            } else {
                pintar_progreso_indeterminado(r, tema, rect, ahora_ticks);
            }
        }
        "image" => pintar_imagen(r, n, rect),
        "check" | "radio" | "circular" | "segmented" | "popup" | "field" | "slider"
        | "slider-gota" => {
            let mut c = control_de(n, i, valores, estado(i));
            c.texto = match n.def.etiqueta {
                "check" | "radio" => n.texto("label", ""),
                "circular" => n.texto("title", ""),
                "segmented" | "popup" => n.texto("items", ""),
                _ => n.texto("value", ""),
            };
            pintar_control(r, tema, rect, &c);
        }
        _ => {}
    }

    let hijos = match disp.nodos.get(i) {
        Some(c) => c.hijos.clone(),
        None => return,
    };
    if hijos.is_empty() {
        return;
    }
    let recorta = n.def.etiqueta == "window"
        || n.def.etiqueta == "box"
        || n.prop("width").is_some()
        || n.prop("height").is_some();
    if recorta {
        r.recorte(rect);
    }
    let nivel_hijos = if n.def.etiqueta == "box" {
        nivel_box.saturating_add(1)
    } else {
        nivel_box
    };
    for h in hijos {
        pintar_nodo(
            r,
            pre,
            disp,
            tema,
            h,
            estado,
            valores,
            nivel_hijos,
            ahora_ticks,
        );
    }
    if recorta {
        r.recorte_fin();
    }
}

/// Pinta el patrón AQUA en mosaico y lo desplaza en un ciclo lineal. El fondo
/// y el clip se conservan incluso si falta el PNG, pero el recurso obligatorio
/// del sistema hace que tanto el HTML como AppKit utilicen la misma pieza.
pub(super) fn pintar_progreso_indeterminado<R: Renderer>(
    r: &mut R,
    tema: &Tema,
    rect: Rect,
    ahora_ticks: u64,
) {
    r.relleno(rect, 0x00E6_E8EB);
    r.borde(rect, tema.paleta.borde);
    if rect.w <= 2 || rect.h <= 2 {
        return;
    }
    let interior = Rect {
        x: rect.x.saturating_add(1),
        y: rect.y.saturating_add(1),
        w: rect.w - 2,
        h: rect.h - 2,
    };
    if let Some(img) = kit::pieza("indeterminate.png") {
        let fase = toby_appkit::animation::IndeterminateProgress::from_millis(0, 100)
            .offset_al_mismo_ritmo(ahora_ticks, img.ancho, img.ancho);
        pintar_textura_progreso(
            r,
            interior,
            img,
            Rect {
                x: 0,
                y: 0,
                w: img.ancho,
                h: img.alto,
            },
            fase,
        );
    }
}

/// Pinta la pista, revela el relleno al porcentaje indicado y desplaza sobre
/// éste el fragmento central de `loader.png` a la misma velocidad lineal.
pub(super) fn pintar_progreso_determinado<R: Renderer>(
    r: &mut R,
    tema: &Tema,
    rect: Rect,
    value: u8,
    ahora_ticks: u64,
) {
    let progreso = toby_appkit::animation::DeterminateProgress::new(value);
    if let Some(back) = kit::pieza("loader_back.png") {
        pintar_progreso_fragmentado(r, rect, back);
    } else {
        r.relleno(rect, tema.paleta.panel);
        r.borde(rect, tema.paleta.borde);
    }

    let filled = progreso.filled_width(rect.w);
    if filled > 0 {
        if let Some(loader) = kit::pieza("loader.png") {
            r.recorte(Rect { w: filled, ..rect });
            pintar_progreso_fragmentado(r, rect, loader);
            r.recorte_fin();
        }

        // La textura conserva la velocidad lineal de la variante
        // indeterminada y anima todo el tramo completado, desde x=0.
        if rect.w > 2 && rect.h > 2 {
            if let Some(loader) = kit::pieza("loader.png") {
                let ancho_extremo = 4u32.min(loader.ancho / 2);
                let ancho_patron = filled.min(rect.w);
                if ancho_patron != 0 {
                    let ancho_centro = loader.ancho.saturating_sub(ancho_extremo * 2);
                    pintar_textura_progreso(
                        r,
                        Rect {
                            x: rect.x,
                            y: rect.y,
                            w: ancho_patron,
                            h: rect.h,
                        },
                        loader,
                        Rect {
                            x: ancho_extremo as i32,
                            y: 0,
                            w: ancho_centro,
                            h: loader.alto,
                        },
                        progreso.texture_offset_at(
                            ahora_ticks,
                            100,
                            toby_appkit::animation::INDETERMINATE_PROGRESS_TILE_WIDTH_PX,
                            ancho_centro,
                        ),
                    );
                }
            }
        }
    }

    // Formato porcentual en pila, sin String/Vec ni reserva durante el paint.
    let mut texto = [0u8; 4];
    texto[3] = b'%';
    let mut numero = value.min(100) as u32;
    let mut inicio = 3;
    loop {
        inicio -= 1;
        texto[inicio] = b'0' + (numero % 10) as u8;
        numero /= 10;
        if numero == 0 {
            break;
        }
    }
    let etiqueta = core::str::from_utf8(&texto[inicio..]).unwrap_or("0%");
    let alto_fuente = rect.h.min(10).max(1);
    let ancho_glifo = (alto_fuente.saturating_mul(52).saturating_add(50) / 100).max(1);
    let ancho_texto = ancho_glifo.saturating_mul((4 - inicio) as u32);
    let x = rect
        .x
        .saturating_add(rect.w.saturating_sub(ancho_texto) as i32 / 2);
    let y = rect
        .y
        .saturating_add(rect.h.saturating_sub(alto_fuente) as i32 / 2);
    r.texto_tamano(x, y, etiqueta, 0x001D_2A39, alto_fuente);
}

/// Repite un fragmento nativo de una textura dentro del clip indicado. Ambos
/// modos usan este camino y el mismo reloj; cada uno aporta su propia imagen.
fn pintar_textura_progreso<R: Renderer>(
    r: &mut R,
    interior: Rect,
    img: &Imagen,
    fuente: Rect,
    fase: u32,
) {
    if fuente.w == 0 || fuente.h == 0 || interior.h == 0 || interior.w == 0 {
        return;
    }
    let tile_w = fuente.w;
    let fase = fase % tile_w;
    let derecha = interior
        .x
        .saturating_add(interior.w.min(i32::MAX as u32) as i32);
    let mut x = interior.x.saturating_sub(fase.min(i32::MAX as u32) as i32);
    r.recorte(interior);
    while x < derecha {
        r.imagen_fragmento(
            Rect {
                x,
                y: interior.y,
                w: tile_w,
                h: fuente.h,
            },
            img,
            fuente,
        );
        let siguiente = x.saturating_add(tile_w as i32);
        if siguiente <= x {
            break;
        }
        x = siguiente;
    }
    r.recorte_fin();
}

/// Monta un PNG 3-slice: los 4 px de cada extremo permanecen intactos y sólo
/// se escala el centro. La misma división se usa en el CSS de AQUA.
fn pintar_progreso_fragmentado<R: Renderer>(r: &mut R, rect: Rect, img: &Imagen) {
    if rect.w == 0 || rect.h == 0 || img.ancho < 9 || img.alto == 0 {
        return;
    }
    let ancho_extremo = 4u32.min(img.ancho / 2);
    let ancho_centro = img.ancho - ancho_extremo * 2;
    let extremo_destino = ancho_extremo.min(rect.w / 2);
    if extremo_destino == 0 {
        r.imagen(rect, img);
        return;
    }

    let fuente_y = Rect {
        x: 0,
        y: 0,
        w: ancho_extremo,
        h: img.alto,
    };
    r.imagen_fragmento(
        Rect {
            w: extremo_destino,
            ..rect
        },
        img,
        fuente_y,
    );

    let ancho_centro_destino = rect.w.saturating_sub(extremo_destino * 2);
    if ancho_centro_destino != 0 {
        r.imagen_fragmento(
            Rect {
                x: rect.x.saturating_add(extremo_destino as i32),
                w: ancho_centro_destino,
                ..rect
            },
            img,
            Rect {
                x: ancho_extremo as i32,
                y: 0,
                w: ancho_centro,
                h: img.alto,
            },
        );
    }

    r.imagen_fragmento(
        Rect {
            x: rect.der().saturating_sub(extremo_destino as i32),
            w: extremo_destino,
            ..rect
        },
        img,
        Rect {
            x: (img.ancho - ancho_extremo) as i32,
            ..fuente_y
        },
    );
}

/// Caja contenedora anidable; el título ocupa una banda exterior sobre el marco.
fn pintar_box<R: Renderer>(r: &mut R, n: &Nodo, rect: Rect, tema: &Tema, nivel: u8) {
    let titulo = n.texto("title", "");
    let fondo = match n.prop("background") {
        Some(PropValue::Color(color)) => *color,
        _ => match nivel {
            0 => tema.paleta.fondo,
            1 => tema.paleta.panel,
            _ => mezclar(tema.paleta.panel, 0xB8C1CA, 25),
        },
    };
    r.recorte(rect);
    r.relleno(rect, fondo);

    let marco = if titulo.is_empty() {
        rect
    } else {
        Rect {
            x: rect.x,
            y: rect.y.saturating_add(layout::ALTO_BANDA_TITULO_BOX as i32),
            w: rect.w,
            h: rect.h.saturating_sub(layout::ALTO_BANDA_TITULO_BOX),
        }
    };
    if n.booleano("bordered", true) {
        pintar_borde_box_relieve(r, marco, tema, fondo);
    }

    if !titulo.is_empty() {
        r.texto_tamano(
            rect.x.saturating_add(14),
            rect.y.saturating_add(2),
            titulo,
            tema.paleta.texto,
            10,
        );
    }
    r.recorte_fin();
}

/// Bisel Aqua suave: sombra azul-gris arriba/izquierda y luz abajo/derecha.
fn pintar_borde_box_relieve<R: Renderer>(r: &mut R, rect: Rect, tema: &Tema, fondo: u32) {
    r.borde(rect, tema.paleta.borde);
    if rect.w < 3 || rect.h < 3 {
        return;
    }

    let sombra = mezclar(fondo, 0x566676, 18);
    let luz = mezclar(fondo, 0xFFFFFF, 80);
    let interior = Rect {
        x: rect.x.saturating_add(1),
        y: rect.y.saturating_add(1),
        w: rect.w.saturating_sub(2),
        h: rect.h.saturating_sub(2),
    };

    r.relleno(
        Rect {
            x: interior.x,
            y: interior.y,
            w: interior.w,
            h: 1,
        },
        sombra,
    );
    r.relleno(
        Rect {
            x: interior.x,
            y: interior.y,
            w: 1,
            h: interior.h,
        },
        sombra,
    );
    r.relleno(
        Rect {
            x: interior.x,
            y: rect.abajo().saturating_sub(2),
            w: interior.w,
            h: 1,
        },
        luz,
    );
    r.relleno(
        Rect {
            x: rect.der().saturating_sub(2),
            y: interior.y,
            w: 1,
            h: interior.h,
        },
        luz,
    );
}

/// Perfila la ventana con cobertura subpíxel en las esquinas superiores y
/// laterales/base rectos. El contenido y el exterior los decide quien llama.
pub fn pintar_silueta_ventana<R: Renderer>(r: &mut R, rect: Rect, exterior: u32, borde: u32) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let radio = RADIO_ESQUINA_VENTANA.min(rect.w / 2).min(rect.h / 2);
    let alto_cabecera = ALTO_BARRA_VENTANA.min(rect.h);

    // 8×8 muestras por píxel: mezcla escritorio, borde y barra según la
    // cobertura real del círculo, eliminando el efecto de escalera del arco.
    for fila in 0..radio {
        let superficie = color_cabecera(fila, alto_cabecera);
        for columna in 0..radio {
            let (dentro, interior) = cobertura_esquina(fila, columna, radio);
            let pixel = color_cobertura(exterior, borde, superficie, dentro, interior);
            let y = rect.y + fila as i32;
            r.relleno(
                Rect {
                    x: rect.x + columna as i32,
                    y,
                    w: 1,
                    h: 1,
                },
                pixel,
            );
            let x_derecha = rect.der() - 1 - columna as i32;
            r.relleno(
                Rect {
                    x: x_derecha,
                    y,
                    w: 1,
                    h: 1,
                },
                pixel,
            );
        }
    }
    let ancho_arriba = rect.w.saturating_sub(radio.saturating_mul(2));
    if ancho_arriba > 0 {
        r.relleno(
            Rect {
                x: rect.x + radio as i32,
                y: rect.y,
                w: ancho_arriba,
                h: 1,
            },
            borde,
        );
    }

    // Laterales rectos y base sin radio.
    for fila in radio..rect.h {
        let y = rect.y + fila as i32;
        r.relleno(
            Rect {
                x: rect.x,
                y,
                w: 1,
                h: 1,
            },
            borde,
        );
        if rect.w > 1 {
            r.relleno(
                Rect {
                    x: rect.der() - 1,
                    y,
                    w: 1,
                    h: 1,
                },
                borde,
            );
        }
    }
    r.relleno(
        Rect {
            x: rect.x,
            y: rect.abajo() - 1,
            w: rect.w,
            h: 1,
        },
        borde,
    );
}

/// Dibuja la barra superior AQUA compartida por Kira y el lienzo del
/// AppKit. El llamador pinta primero el contenido; esta función superpone
/// la barra, sus controles y el contorno sin añadir sombra.
///
/// También se comparte el degradado de superficie con la barra de menú del
/// escritorio de Kira, para que ambas piezas tengan exactamente la misma
/// textura aunque la barra de menú no lleve botones de ventana.
pub fn pintar_textura_cabecera<R: Renderer>(r: &mut R, rect: Rect, borde: u32) {
    pintar_textura_cabecera_alto(r, rect, borde, ALTO_BARRA_VENTANA);
}

/// Dibuja la misma textura AQUA con la altura solicitada por cada ventana.
pub fn pintar_textura_cabecera_alto<R: Renderer>(
    r: &mut R,
    rect: Rect,
    borde: u32,
    alto_solicitado: u32,
) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let alto = alto_solicitado.min(rect.h);
    if alto == 0 {
        return;
    }
    r.recorte(rect);
    for fila in 0..alto.saturating_sub(1) {
        r.relleno(
            Rect {
                x: rect.x,
                y: rect.y + fila as i32,
                w: rect.w,
                h: 1,
            },
            color_cabecera(fila, alto),
        );
    }
    r.relleno(
        Rect {
            x: rect.x,
            y: rect.y + alto as i32 - 1,
            w: rect.w,
            h: 1,
        },
        borde,
    );
    r.recorte_fin();
}

pub fn pintar_cabecera_ventana<R: Renderer>(
    r: &mut R,
    rect: Rect,
    exterior: u32,
    borde: u32,
    titulo: &str,
) {
    pintar_cabecera_ventana_con_cursor(r, rect, exterior, borde, titulo, None);
}

/// Variante interactiva: revela X, maximizar y minimizar al posar el cursor
/// sobre la píldora verde.
pub fn pintar_cabecera_ventana_con_cursor<R: Renderer>(
    r: &mut R,
    rect: Rect,
    exterior: u32,
    borde: u32,
    titulo: &str,
    cursor: Option<(i32, i32)>,
) {
    pintar_cabecera_ventana_con_botones(r, rect, exterior, borde, titulo, cursor, &[]);
}

/// Pinta la cabecera AppKit con controles específicos de la app.
///
/// Las apps que no pasan controles conservan la cabecera estándar.
pub fn pintar_cabecera_ventana_con_botones<R: Renderer>(
    r: &mut R,
    rect: Rect,
    exterior: u32,
    borde: u32,
    titulo: &str,
    cursor: Option<(i32, i32)>,
    botones: &[BotonCabecera<'_>],
) {
    pintar_cabecera_ventana_appkit(
        r,
        rect,
        exterior,
        borde,
        titulo,
        cursor,
        CabeceraVentana {
            alto: ALTO_BARRA_VENTANA,
            botones,
        },
    );
}

/// Cabecera AppKit configurable por ventana: la altura y la colección de
/// controles pertenecen a la app que la declara, no a un estilo global.
pub fn pintar_cabecera_ventana_appkit<R: Renderer>(
    r: &mut R,
    rect: Rect,
    exterior: u32,
    borde: u32,
    titulo: &str,
    cursor: Option<(i32, i32)>,
    cabecera: CabeceraVentana<'_>,
) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let alto = cabecera.alto.min(rect.h);
    if alto == 0 {
        return;
    }
    pintar_textura_cabecera_alto(r, rect, borde, alto);
    r.recorte(rect);

    // Píldora verde larga a la izquierda y píldora blanca más corta a la
    // derecha; ambas reutilizan las pieles PNG AQUA de los botones normales.
    let alto_superior = ALTO_BARRA_VENTANA.min(alto);
    let alto_boton = alto_superior.min(16);
    let alto_blanco = alto_superior.min(10);
    let ancho_blanco = rect.w.saturating_sub(20).min(20);
    if alto_boton > 0 {
        if let Some(boton_verde) = rect_pildora_verde(rect) {
            if let Some(piel) = kit::piel("green") {
                pintar_piel_filtrada(r, piel, boton_verde, 100, 100, 100);
            } else {
                pintar_pildora_verde_respaldo(r, boton_verde);
            }
            if cursor.is_some_and(|(x, y)| control_ventana_en(rect, x, y).is_some()) {
                pintar_simbolos_controles(r, boton_verde);
            }
        }
        if ancho_blanco > 0 && alto_blanco > 0 {
            let boton_blanco = Rect {
                x: rect.der() - 10 - ancho_blanco as i32,
                y: rect.y + (alto_superior as i32 - alto_blanco as i32) / 2,
                w: ancho_blanco,
                h: alto_blanco,
            };
            if let Some(piel) = kit::piel("white") {
                pintar_piel_filtrada(r, piel, boton_blanco, 100, 100, 100);
            } else {
                pintar_pildora_blanca_respaldo(r, boton_blanco);
            }
        }
    }

    for (indice, boton) in cabecera.botones.iter().enumerate() {
        let Some(caja) = rect_boton_cabecera(rect, alto, indice) else {
            break;
        };
        let sobre = cursor.is_some_and(|(x, y)| caja.contiene(x, y));
        let variante = if sobre { "blue" } else { "white" };
        // No inventar una esfera si falta el PNG AppKit: en ese caso se omite
        // el botón completo (también su icono), en vez de dibujar un sustituto.
        if kit::esfera(variante).is_none() {
            continue;
        }
        let control = Control {
            etiqueta: "circular",
            variante,
            texto: "",
            valor: 0,
            marcado: false,
            placeholder: "",
            estado: if sobre {
                Estado::Hover
            } else if boton.habilitado {
                Estado::Normal
            } else {
                Estado::Disabled
            },
        };
        // Usa el componente circular de AppKit y su PNG de variante
        // (white_radio o blue_radio); encima va el icono de la ventana.
        pintar_control(r, &super::theme::TEMA_POR_DEFECTO, caja, &control);
        if let Some(imagen) = boton.imagen {
            let icono = Rect {
                // Los indicadores del kit son PNG 431×224 (~1,92:1);
                // crecen con la esfera para conservar proporción y legibilidad.
                x: caja.x + (caja.w as i32 - 20) / 2,
                y: caja.y + (caja.h as i32 - 10) / 2,
                w: 20,
                h: 10,
            };
            if boton.habilitado {
                r.imagen(icono, imagen);
            } else {
                // Mantener el PNG legible cuando no hay historial; el estado
                // inactivo ya se distingue por la cara más gris del botón.
                r.imagen_filtrada(icono, imagen, 100, 100, 80);
            }
        }
    }

    let alto_titulo = 16;
    let ancho_titulo = r.medir_texto_tamano(titulo, alto_titulo) as i32;
    let inicio_titulo = rect.x;
    let fin_titulo = rect.der();
    let ancho_disponible = (fin_titulo - inicio_titulo).max(0);
    if !titulo.is_empty() && ancho_titulo <= ancho_disponible {
        let x = inicio_titulo + (ancho_disponible - ancho_titulo) / 2;
        let y = rect.y + (alto_superior as i32 - alto_titulo as i32).max(0) / 2;
        r.texto_tamano(x, y, titulo, 0x303236, alto_titulo);
    }
    r.recorte_fin();
    pintar_silueta_ventana(r, rect, exterior, borde);
}

/// Posición AppKit del botón adicional `indice` en una cabecera.
/// La misma geometría sirve para dibujarlo y para comprobar clics.
pub fn rect_boton_cabecera(rect: Rect, alto_cabecera: u32, indice: usize) -> Option<Rect> {
    let alto = alto_cabecera.min(rect.h);
    let alto_fila = alto.saturating_sub(ALTO_BARRA_VENTANA);
    let lado = alto_fila.saturating_sub(4).min(24);
    if lado == 0 {
        return None;
    }
    let pill = rect_pildora_verde(rect)?;
    // Respeta la proporción de white_radio.png (682×653), igual que
    // circular.css: con 24 px de alto la cara mide 25 px de ancho.
    let ancho = layout::ancho_esfera(lado);
    let paso = ancho.saturating_add(4) as i32;
    // Los dos controles de navegación quedan centrados bajo la píldora; los
    // siguientes se agregan a su derecha, dejando su centro como ancla.
    let ancho_grupo = (ancho * 2).saturating_add(4) as i32;
    let inicio = pill.x + (pill.w as i32 - ancho_grupo) / 2;
    let x = inicio + (indice as i32).checked_mul(paso)?;
    let limite_derecho = rect.der() - 10;
    if x + ancho as i32 > limite_derecho {
        return None;
    }
    Some(Rect {
        x,
        y: rect.y + alto as i32 - 4 - lado as i32,
        w: ancho,
        h: lado,
    })
}

/// Botón adicional bajo el puntero, si la app declaró ese índice.
pub fn boton_cabecera_en(
    rect: Rect,
    alto_cabecera: u32,
    x: i32,
    y: i32,
    cantidad: usize,
) -> Option<usize> {
    (0..cantidad).find(|&indice| {
        rect_boton_cabecera(rect, alto_cabecera, indice).is_some_and(|boton| boton.contiene(x, y))
    })
}

/// Devuelve el control de cabecera situado bajo el cursor, si está en la
/// píldora verde. El orden visual es cerrar, maximizar y minimizar.
pub fn control_ventana_en(rect: Rect, x: i32, y: i32) -> Option<ControlVentana> {
    let boton = rect_pildora_verde(rect)?;
    if !boton.contiene(x, y) || boton.w < 3 {
        return None;
    }
    let segmento = ((x - boton.x) * 3 / boton.w as i32).clamp(0, 2);
    Some(match segmento {
        0 => ControlVentana::Cerrar,
        1 => ControlVentana::Maximizar,
        _ => ControlVentana::Minimizar,
    })
}

fn rect_pildora_verde(rect: Rect) -> Option<Rect> {
    let alto = ALTO_BARRA_VENTANA.min(rect.h);
    let alto_boton = alto.min(16);
    let ancho_blanco = rect.w.saturating_sub(20).min(20);
    let ancho_verde = rect
        .w
        .saturating_sub(ancho_blanco.saturating_add(30))
        .min(72);
    if alto_boton == 0 || ancho_verde == 0 {
        return None;
    }
    Some(Rect {
        x: rect.x + 10,
        y: rect.y + (alto as i32 - alto_boton as i32) / 2,
        w: ancho_verde,
        h: alto_boton,
    })
}

fn pintar_simbolos_controles<R: Renderer>(r: &mut R, boton: Rect) {
    if boton.w < 18 || boton.h < 7 {
        return;
    }
    let tinta = 0x185D2B;
    let ancho = boton.w as i32;
    let centro_y = boton.y + boton.h as i32 / 2;
    let centros_x = [
        boton.x + ancho / 6,
        boton.x + ancho / 2,
        boton.x + ancho * 5 / 6,
    ];
    let radio = 3;

    // X de cierre.
    pintar_trazo(
        r,
        centros_x[0] - radio,
        centro_y - radio,
        centros_x[0] + radio,
        centro_y + radio,
        tinta,
    );
    pintar_trazo(
        r,
        centros_x[0] - radio,
        centro_y + radio,
        centros_x[0] + radio,
        centro_y - radio,
        tinta,
    );

    // Cuadrado de maximizar.
    let cuadro = Rect {
        x: centros_x[1] - radio,
        y: centro_y - radio,
        w: (radio * 2 + 1) as u32,
        h: (radio * 2 + 1) as u32,
    };
    r.relleno(Rect { h: 1, ..cuadro }, tinta);
    r.relleno(
        Rect {
            y: cuadro.abajo() - 1,
            h: 1,
            ..cuadro
        },
        tinta,
    );
    r.relleno(Rect { w: 1, ..cuadro }, tinta);
    r.relleno(
        Rect {
            x: cuadro.der() - 1,
            w: 1,
            ..cuadro
        },
        tinta,
    );

    // Raya de minimizar.
    r.relleno(
        Rect {
            x: centros_x[2] - radio,
            y: centro_y,
            w: (radio * 2 + 1) as u32,
            h: 1,
        },
        tinta,
    );
}

fn pintar_trazo<R: Renderer>(r: &mut R, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
    let (mut x, mut y) = (x0, y0);
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        r.relleno(Rect { x, y, w: 1, h: 1 }, color);
        if x == x1 && y == y1 {
            break;
        }
        let doble = 2 * error;
        if doble >= dy {
            error += dy;
            x += sx;
        }
        if doble <= dx {
            error += dx;
            y += sy;
        }
    }
}

/// Pinta en la base de la ventana una franja con el mismo degradado que la
/// cabecera. Se deja el último píxel al contorno compartido, de modo que la
/// base conserva sus esquinas cuadradas y el contenido queda intacto encima.
pub fn pintar_pie_ventana<R: Renderer>(r: &mut R, rect: Rect, borde: u32) {
    if rect.w < 3 || rect.h == 0 {
        return;
    }
    let alto = ALTO_BARRA_VENTANA.min(rect.h);
    if alto < 2 {
        return;
    }
    let y = rect.abajo() - alto as i32;
    r.recorte(rect);
    r.relleno(
        Rect {
            x: rect.x + 1,
            y,
            w: rect.w - 2,
            h: 1,
        },
        borde,
    );
    for fila in 1..alto.saturating_sub(1) {
        r.relleno(
            Rect {
                x: rect.x + 1,
                y: y + fila as i32,
                w: rect.w - 2,
                h: 1,
            },
            color_cabecera(fila, alto),
        );
    }
    if let Some(shooter) = kit::shooter() {
        const LADO_MAXIMO: u32 = 20;
        let lado_fuente = shooter.ancho.max(shooter.alto);
        if lado_fuente > 0 {
            let w =
                ((shooter.ancho as u64 * LADO_MAXIMO as u64) / lado_fuente as u64).max(1) as u32;
            let h = ((shooter.alto as u64 * LADO_MAXIMO as u64) / lado_fuente as u64).max(1) as u32;
            // Conserva la proporción y el margen alfa del PNG, reducido y
            // anclado en la esquina inferior derecha del pie.
            if rect.w > w + 1 && rect.h > h + 1 {
                r.imagen(
                    Rect {
                        x: rect.der() - w as i32 - 1,
                        y: rect.abajo() - h as i32 - 1,
                        w,
                        h,
                    },
                    shooter,
                );
            }
        }
    }
    r.recorte_fin();
}

fn pintar_pildora_verde_respaldo<R: Renderer>(r: &mut R, rect: Rect) {
    let radio = (rect.h / 2).min(rect.w / 2) as i32;
    let centro_y = rect.y + rect.h as i32 / 2;
    let verde = 0x2E9E4F;
    if rect.w > rect.h {
        r.relleno(
            Rect {
                x: rect.x + radio,
                y: rect.y,
                w: rect.w - radio as u32 * 2,
                h: rect.h,
            },
            verde,
        );
    }
    r.circulo(rect.x + radio, centro_y, radio, verde);
    r.circulo(rect.der() - 1 - radio, centro_y, radio, verde);
}

fn pintar_pildora_blanca_respaldo<R: Renderer>(r: &mut R, rect: Rect) {
    let radio = (rect.h / 2).min(rect.w / 2) as i32;
    let centro_y = rect.y + rect.h as i32 / 2;
    let blanco = 0xF4F5F7;
    if rect.w > rect.h {
        r.relleno(
            Rect {
                x: rect.x + radio,
                y: rect.y,
                w: rect.w - radio as u32 * 2,
                h: rect.h,
            },
            blanco,
        );
    }
    r.circulo(rect.x + radio, centro_y, radio, blanco);
    r.circulo(rect.der() - 1 - radio, centro_y, radio, blanco);
}

fn mezclar_color(a: u32, b: u32, porcentaje: u32) -> u32 {
    let p = porcentaje.min(100);
    let canal = |desplazamiento: u32| {
        let inicio = (a >> desplazamiento) & 0xFF;
        let fin = (b >> desplazamiento) & 0xFF;
        (inicio * (100 - p) + fin * p + 50) / 100
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}

fn color_cabecera(fila: u32, alto: u32) -> u32 {
    let porcentaje = if alto > 1 { fila * 100 / (alto - 1) } else { 0 };
    mezclar_color(0xF7F7F7, 0xD9DADD, porcentaje)
}

fn cobertura_esquina(fila: u32, columna: u32, radio: u32) -> (u32, u32) {
    const MUESTRAS: i64 = 8;
    const ESCALA: i64 = MUESTRAS * 2;
    let centro = radio as i64 * ESCALA;
    let radio_exterior2 = centro * centro;
    let radio_interior = (radio as i64 - 1).max(0) * ESCALA;
    let radio_interior2 = radio_interior * radio_interior;
    let mut dentro = 0;
    let mut interior = 0;
    for sy in 0..MUESTRAS {
        let y = fila as i64 * ESCALA + sy * 2 + 1;
        let dy = centro - y;
        for sx in 0..MUESTRAS {
            let x = columna as i64 * ESCALA + sx * 2 + 1;
            let dx = centro - x;
            let distancia2 = dx * dx + dy * dy;
            if distancia2 <= radio_exterior2 {
                dentro += 1;
                if radio > 1 && x >= ESCALA && y >= ESCALA && distancia2 <= radio_interior2 {
                    interior += 1;
                }
            }
        }
    }
    (dentro, interior)
}

/// Cobertura exterior (transparente) de un píxel de las esquinas superiores.
/// Kira usa el valor para componer el antialias contra el escritorio real.
pub(crate) fn cobertura_exterior_esquina(fila: u32, columna: u32, radio: u32) -> u32 {
    64 - cobertura_esquina(fila, columna, radio).0
}

fn color_cobertura(exterior: u32, borde: u32, superficie: u32, dentro: u32, interior: u32) -> u32 {
    const TOTAL: u32 = 64;
    let fuera = TOTAL - dentro;
    let ribete = dentro - interior;
    let canal = |desplazamiento: u32| {
        let suma = ((exterior >> desplazamiento) & 0xFF) * fuera
            + ((borde >> desplazamiento) & 0xFF) * ribete
            + ((superficie >> desplazamiento) & 0xFF) * interior;
        (suma + TOTAL / 2) / TOTAL
    };
    (canal(16) << 16) | (canal(8) << 8) | canal(0)
}

/// Pinta texto plano con la tipografía del host, sin piel ni comportamiento de botón.
fn pintar_etiqueta<R: Renderer>(r: &mut R, n: &Nodo, rect: Rect, tema: &Tema) {
    let texto = n.texto("text", "");
    if texto.is_empty() {
        return;
    }
    let alto = n.entero("size", 14).clamp(8, 32) as u32;
    let salto = alto.saturating_add(3);
    let cuenta = texto.split('\n').count() as u32;
    let alto_total = salto.saturating_mul(cuenta);
    let mut y = rect.y + (rect.h.saturating_sub(alto_total) / 2) as i32;
    let color = tema.paleta.texto;
    let negrita = n.booleano("bold", false);
    r.recorte(rect);
    for linea in texto.split('\n') {
        let ancho = r.medir_texto_tamano(linea, alto).min(rect.w);
        let x = match n.texto("align", "start") {
            "center" => rect.x + rect.w.saturating_sub(ancho) as i32 / 2,
            "end" => rect.x + rect.w.saturating_sub(ancho) as i32,
            _ => rect.x,
        };
        if negrita {
            r.texto_tamano_negrita(x, y, linea, color, alto);
        } else {
            r.texto_tamano(x, y, linea, color, alto);
        }
        y = y.saturating_add(salto as i32);
    }
    r.recorte_fin();
}

/// Pinta un botón con el color y la sombra que le toca por su estado.
///
/// Con el kit, la piel ya trae silueta, brillo y sombra, y la etiqueta va en
/// la tinta azul marino del CSS (`--aqua-label: #0a2e52`) sobre el cuerpo
/// claro del PNG. El foco es el `outline: 3px solid #8fd0ff` del kit, pintado
/// FUERA del rectángulo. Sin kit (o apagado), cae al rectángulo de acento de
/// siempre.
fn pintar_boton<R: Renderer>(r: &mut R, n: &Nodo, rect: Rect, tema: &Tema, estado: Estado) {
    let variante = n.texto("variant", "blue");
    let (brillo, saturacion, opacidad) = filtro_boton(estado);
    let desplazamiento = match estado {
        Estado::Hover => -1,
        Estado::Pressed => 1,
        _ => 0,
    };
    let piel_rect = Rect {
        y: rect.y + desplazamiento,
        ..rect
    };
    if let Some(piel) = super::kit::piel(variante) {
        pintar_piel_filtrada(r, piel, piel_rect, brillo, saturacion, opacidad);
    } else {
        let base = theme::acento_de(tema, variante);
        let mut color = filtrar_color(base, brillo, saturacion);
        if opacidad < 100 {
            color = mezclar(color, 0xFFFFFF, 100 - opacidad as u32);
        }
        r.relleno(piel_rect, color);
    }
    let titulo = n.texto("title", "");
    let alto_fuente = (rect.h.saturating_mul(28) + 50) / 100;
    let ancho = layout::ancho_texto_css(titulo, alto_fuente) as i32;
    let alto = alto_fuente as i32;
    let x = rect.x + (rect.w as i32 - ancho) / 2;
    let y = rect.y + (rect.h as i32 - alto) / 2;
    let texto = tinta_css(0x0A2E52, opacidad, saturacion);
    r.texto_tamano(x, y, titulo, texto, alto_fuente);
    if estado == Estado::Focused {
        anillo_foco(r, rect, 3, 3);
    }
}

// --- Controles (Fase 10) --------------------------------------------------

/// Un control del kit ya con todos sus datos a mano.
///
/// Lo usan el motor (`pintar_nodo`, que los saca del nodo y de los valores)
/// y los editores visuales, que pueden pintar un control de muestra sin
/// montar un documento entero. Es UNA sola implementación del pintado: lo que
/// se ve en AppKit es lo que se ve en el intérprete.
#[derive(Clone, Copy)]
pub struct Control<'a> {
    /// Etiqueta del componente: `check`, `radio`, `circular`, `segmented`,
    /// `popup`, `field`, `slider` o `slider-gota`.
    pub etiqueta: &'a str,
    /// Variante del kit (`variant`).
    pub variante: &'a str,
    /// Texto: `title`, `label` o la lista `items`, según el componente.
    pub texto: &'a str,
    /// Opción elegida (`segmented`/`popup`) o porcentaje 0..100 (`slider`).
    pub valor: i64,
    /// ¿Marcado? (`check`/`radio`).
    pub marcado: bool,
    /// Texto tenue mientras un `field` está vacío.
    pub placeholder: &'a str,
    /// Estado de interacción (normal, hover, pulsado, apagado, con foco).
    pub estado: Estado,
}

/// Control de un nodo, según sus valores actuales y su estado de interacción.
fn control_de<'a>(
    n: &'a Nodo,
    i: usize,
    valores: &super::valores::Valores,
    estado: Estado,
) -> Control<'a> {
    let valor = match n.def.etiqueta {
        "segmented" | "popup" => valores.entero(i),
        "slider" | "slider-gota" => {
            let (min, max) = super::valores::rango_slider(n);
            let v = valores.entero(i);
            if max > min {
                ((v - min) * 100 / (max - min)).clamp(0, 100)
            } else {
                0
            }
        }
        _ => 0,
    };
    // AQUA deja el botón circular gris si no se especifica una clase de color.
    let variante = if n.def.etiqueta == "circular" && n.prop("variant").is_none() {
        ""
    } else {
        n.texto("variant", "blue")
    };
    Control {
        etiqueta: n.def.etiqueta,
        variante,
        texto: "",
        valor,
        marcado: valores.marcado(i),
        placeholder: n.texto("placeholder", ""),
        estado,
    }
}

/// Pinta un control del kit en `rect`, según su etiqueta.
pub fn pintar_control<R: Renderer>(r: &mut R, tema: &Tema, rect: Rect, c: &Control<'_>) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    match c.etiqueta {
        "check" => pintar_marca(r, tema, rect, c, false),
        "radio" => pintar_marca(r, tema, rect, c, true),
        "circular" => pintar_circular(r, tema, rect, c),
        "segmented" => pintar_segmentado(r, tema, rect, c),
        "popup" => pintar_disparador(r, tema, rect, c),
        "field" => pintar_campo(r, tema, rect, c),
        "slider" | "slider-gota" => pintar_slider(r, tema, rect, c),
        _ => {}
    }
}

// --- Montaje AQUA ----------------------------------------------------------
//
// Todo lo de aquí para abajo reproduce el ensamblaje de los CSS del kit
// (`AQUA/*.css`): mismas piezas, mismos volteos y mismas proporciones, sólo
// que redondeadas a píxeles enteros y con la fuente bitmap del tema en vez
// de Segoe UI. Las medidas salen todas de `extremo`, que es el `calc()` del
// CSS escrito con enteros.

/// Tintas fijas del kit (el HTML no las toma del tema).
mod tinta {
    /// Etiqueta de un botón (`--aqua-label: #0a2e52`).
    pub const BOTON: u32 = 0x0A2E52;
    /// Texto sobre piezas claras (`--aqua-seg-ink`/`--aqua-pop-ink`/…: #3c3c3e).
    pub const CLARA: u32 = 0x3C3C3E;
    /// Texto tenue de un `field` sin escribir (`::placeholder`: #9a9a9e).
    pub const TENUE: u32 = 0x9A9A9E;
    /// Texto de la opción activa (`--aqua-seg-on: #ffffff`: el CSS lo deja
    /// blanco en TODAS las variantes, también en amarillo).
    pub const ACTIVA: u32 = 0xFFFFFF;
    /// Anillo de foco (`outline: 3px solid #8fd0ff`).
    pub const FOCO: u32 = 0x8FD0FF;
}

/// Proporción de una pieza del CSS, con enteros: `alto × num ÷ den`.
fn extremo(alto: u32, num: u32, den: u32) -> u32 {
    (alto.saturating_mul(num) + den / 2) / den
}

/// Filtros de `.aqua-btn`: hover 1,06/1,06 y activo 0,94.
fn filtro_boton(estado: Estado) -> (u16, u16, u8) {
    match estado {
        Estado::Hover => (106, 106, 100),
        Estado::Pressed => (94, 100, 100),
        Estado::Disabled => (100, 30, 55),
        _ => (100, 100, 100),
    }
}

/// Filtros de check/radio/circular/slider: el estado activo sólo escala la
/// pieza; no la oscurece. El hover sí usa brillo 1,08 y saturación 1,05.
fn filtro_pieza(estado: Estado) -> (u16, u16, u8) {
    match estado {
        Estado::Hover => (108, 105, 100),
        Estado::Disabled => (100, 30, 55),
        _ => (100, 100, 100),
    }
}

fn filtrar_color(color: u32, brillo: u16, saturacion: u16) -> u32 {
    let (r, g, b) = ((color >> 16) & 0xFF, (color >> 8) & 0xFF, color & 0xFF);
    let gris = (r * 2126 + g * 7152 + b * 722 + 5000) / 10000;
    let canal = |c: u32| -> u32 {
        let sat =
            (gris as i32 + (c as i32 - gris as i32) * saturacion as i32 / 100).clamp(0, 255) as u32;
        (sat * brillo as u32 / 100).min(255)
    };
    (canal(r) << 16) | (canal(g) << 8) | canal(b)
}

fn mezclar(color: u32, fondo: u32, porcentaje_fondo: u32) -> u32 {
    let p = porcentaje_fondo.min(100);
    let canal = |des: u32, src: u32| (des * (100 - p) + src * p + 50) / 100;
    (canal((color >> 16) & 0xFF, (fondo >> 16) & 0xFF) << 16)
        | (canal((color >> 8) & 0xFF, (fondo >> 8) & 0xFF) << 8)
        | canal(color & 0xFF, fondo & 0xFF)
}

fn tinta_css(color: u32, opacidad: u8, saturacion: u16) -> u32 {
    let gris = filtrar_color(color, 100, saturacion);
    mezclar(gris, 0xFFFFFF, 100 - opacidad.min(100) as u32)
}

fn pintar_piel_filtrada<R: Renderer>(
    r: &mut R,
    piel: &kit::Piel,
    rect: Rect,
    brillo: u16,
    saturacion: u16,
    opacidad: u8,
) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let cap = (rect.h.saturating_mul(258) / 409).max(1);
    if rect.w <= cap.saturating_mul(2) {
        r.imagen_filtrada(rect, &piel.izq, brillo, saturacion, opacidad);
        return;
    }
    r.imagen_filtrada(
        Rect {
            x: rect.x,
            y: rect.y,
            w: cap,
            h: rect.h,
        },
        &piel.izq,
        brillo,
        saturacion,
        opacidad,
    );
    r.imagen_filtrada(
        Rect {
            x: rect.x + cap as i32,
            y: rect.y,
            w: rect.w - cap * 2,
            h: rect.h,
        },
        &piel.cuerpo,
        brillo,
        saturacion,
        opacidad,
    );
    r.imagen_filtrada(
        Rect {
            x: rect.der() - cap as i32,
            y: rect.y,
            w: cap,
            h: rect.h,
        },
        &piel.der,
        brillo,
        saturacion,
        opacidad,
    );
}

/// Pinta el pulgar de una barra de desplazamiento usando la piel azul real de
/// botón AQUA. Si faltan recursos no sustituye la píldora por un rectángulo.
pub fn pintar_pulgar_scroll_appkit<R: Renderer>(r: &mut R, rect: Rect, vertical: bool) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    if vertical {
        let Some(piel) = kit::piel_vertical("blue") else {
            return;
        };
        pintar_piel_vertical(r, piel, rect);
    } else if let Some(piel) = kit::piel("blue") {
        pintar_piel_filtrada(r, piel, rect, 100, 100, 100);
    }
}

fn pintar_piel_vertical<R: Renderer>(r: &mut R, piel: &kit::Piel, rect: Rect) {
    // Tras girar las piezas, el borde de 409 px es el ancho y el de 258 px
    // es el alto longitudinal de cada tapa. Mantener esta proporción evita
    // que la píldora se aplaste al estrechar el thumb.
    let cap = (rect.w.saturating_mul(258) / 409).max(1);
    if rect.h <= cap.saturating_mul(2) {
        // Si no cabe el cuerpo, reducir ambas tapas conservando su aspecto;
        // nunca estirar una tapa para rellenar todo el thumb.
        if rect.h <= 1 {
            r.imagen(rect, &piel.izq);
            return;
        }
        let alto_tapa = (rect.h / 2).max(1);
        let ancho_tapa = (alto_tapa.saturating_mul(409) / 258).max(1).min(rect.w);
        let x_tapa = rect.x + ((rect.w - ancho_tapa) / 2) as i32;
        let alto_cuerpo = rect.h.saturating_sub(alto_tapa * 2);
        if alto_cuerpo > 0 {
            r.imagen(
                Rect {
                    x: rect.x,
                    y: rect.y + alto_tapa as i32,
                    w: rect.w,
                    h: alto_cuerpo,
                },
                &piel.cuerpo,
            );
        }
        r.imagen(
            Rect {
                x: x_tapa,
                y: rect.y,
                w: ancho_tapa,
                h: alto_tapa,
            },
            &piel.izq,
        );
        r.imagen(
            Rect {
                x: x_tapa,
                y: rect.abajo() - alto_tapa as i32,
                w: ancho_tapa,
                h: alto_tapa,
            },
            &piel.der,
        );
        return;
    }
    r.imagen(
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: cap,
        },
        &piel.izq,
    );
    r.imagen(
        Rect {
            x: rect.x,
            y: rect.y + cap as i32,
            w: rect.w,
            h: rect.h - cap * 2,
        },
        &piel.cuerpo,
    );
    r.imagen(
        Rect {
            x: rect.x,
            y: rect.abajo() - cap as i32,
            w: rect.w,
            h: cap,
        },
        &piel.der,
    );
}

/// Pinta una pieza del kit en `rect`, dada la vuelta si `volteada`
/// (el `scaleX(-1)` con el que el CSS reutiliza las piezas).
fn pieza_en<R: Renderer>(r: &mut R, rect: Rect, img: &Imagen, volteada: bool) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    if volteada {
        r.imagen_espejo(rect, img);
    } else {
        r.imagen(rect, img);
    }
}

fn pieza_en_filtrada<R: Renderer>(
    r: &mut R,
    rect: Rect,
    img: &Imagen,
    volteada: bool,
    brillo: u16,
    saturacion: u16,
    opacidad: u8,
) {
    if volteada {
        r.imagen_espejo_filtrada(rect, img, brillo, saturacion, opacidad);
    } else {
        r.imagen_filtrada(rect, img, brillo, saturacion, opacidad);
    }
}

/// Monta un skin de tres piezas al modo CSS: cuerpo estirado en medio y los
/// extremos `pizq`/`pder` (con su giro) pegados a los bordes.
///
/// Si no cabe cuerpo (`w ≤ 2 × cap`) no se pinta —el CSS pone
/// `max(0px, …)`— y los dos extremos quedan solapados, igual que en el
/// navegador. Orden: primero el cuerpo, encima los extremos.
fn tres_piezas<R: Renderer>(
    r: &mut R,
    cuerpo: &Imagen,
    rect: Rect,
    cap: u32,
    pizq: &Imagen,
    voltea_izq: bool,
    pder: &Imagen,
    voltea_der: bool,
) {
    if rect.w > cap.saturating_mul(2) {
        r.imagen(
            Rect {
                x: rect.x + cap as i32,
                y: rect.y,
                w: rect.w - cap * 2,
                h: rect.h,
            },
            cuerpo,
        );
    }
    if cap > 0 {
        pieza_en(
            r,
            Rect {
                x: rect.x,
                y: rect.y,
                w: cap.min(rect.w),
                h: rect.h,
            },
            pizq,
            voltea_izq,
        );
        if rect.w > cap {
            pieza_en(
                r,
                Rect {
                    x: rect.der() - cap as i32,
                    y: rect.y,
                    w: cap,
                    h: rect.h,
                },
                pder,
                voltea_der,
            );
        }
    }
}

fn tres_piezas_filtradas<R: Renderer>(
    r: &mut R,
    cuerpo: &Imagen,
    rect: Rect,
    cap: u32,
    pizq: &Imagen,
    voltea_izq: bool,
    pder: &Imagen,
    voltea_der: bool,
    brillo: u16,
    saturacion: u16,
    opacidad: u8,
) {
    if rect.w > cap.saturating_mul(2) {
        r.imagen_filtrada(
            Rect {
                x: rect.x + cap as i32,
                y: rect.y,
                w: rect.w - cap * 2,
                h: rect.h,
            },
            cuerpo,
            brillo,
            saturacion,
            opacidad,
        );
    }
    if cap > 0 {
        pieza_en_filtrada(
            r,
            Rect {
                x: rect.x,
                y: rect.y,
                w: cap.min(rect.w),
                h: rect.h,
            },
            pizq,
            voltea_izq,
            brillo,
            saturacion,
            opacidad,
        );
        if rect.w > cap {
            pieza_en_filtrada(
                r,
                Rect {
                    x: rect.der() - cap as i32,
                    y: rect.y,
                    w: cap,
                    h: rect.h,
                },
                pder,
                voltea_der,
                brillo,
                saturacion,
                opacidad,
            );
        }
    }
}

/// Anillo de foco como el `outline: 3px solid #8fd0ff` del kit: una vuelta
/// clara 3 px por fuera del control (respeta el recorte activo).
fn anillo_foco<R: Renderer>(r: &mut R, rect: Rect, offset: i32, grosor: u32) {
    let grosor = grosor.max(1);
    let exterior = Rect {
        x: rect.x - offset - grosor as i32,
        y: rect.y - offset - grosor as i32,
        w: rect.w + ((offset + grosor as i32) * 2).max(0) as u32,
        h: rect.h + ((offset + grosor as i32) * 2).max(0) as u32,
    };
    for i in 0..grosor {
        let marco = Rect {
            x: exterior.x + i as i32,
            y: exterior.y + i as i32,
            w: exterior.w.saturating_sub(i * 2),
            h: exterior.h.saturating_sub(i * 2),
        };
        r.borde(marco, tinta::FOCO);
    }
}

fn rect_escalado(rect: Rect, porcentaje: u32) -> Rect {
    let w = (rect.w.saturating_mul(porcentaje) / 100).max(1);
    let h = (rect.h.saturating_mul(porcentaje) / 100).max(1);
    Rect {
        x: rect.x + ((rect.w - w) / 2) as i32,
        y: rect.y + ((rect.h - h) / 2) as i32,
        w,
        h,
    }
}

/// `<check>` y `<radio>`: la casilla o la esfera del kit, con su marca.
///
/// El montaje es el de `check.css` y `radio.css`:
///
///   * **sin marcar** → la pieza GRISEA (`gray_check.png`/`gray_radio.png`);
///   * **marcado** → la pieza de color (`<color>_…png`) y encima la marca:
///     el tick `check_v.png` (que SOBRESALIÉN ~15 % del alto, como en el kit)
///     o el punto `dot.png` (34 % del alto);
///   * **apagado** → gris, que es lo más parecido al `grayscale` del CSS.
///
/// `esfera` distingue el radio (redondo, con punto) del check (cuadrado, con
/// tick); las dos llevan su proporción nativa (635 × 649 y 682 × 653).
fn pintar_marca<R: Renderer>(r: &mut R, tema: &Tema, rect: Rect, c: &Control<'_>, esfera: bool) {
    let alto = rect.h.max(1);
    let ancho = if esfera {
        super::layout::ancho_esfera(alto)
    } else {
        super::layout::ancho_check(alto)
    };
    let mut caja = Rect {
        x: rect.x,
        y: rect.y,
        w: ancho,
        h: alto,
    };
    let (brillo, saturacion, opacidad) = filtro_pieza(c.estado);
    if c.estado == Estado::Pressed {
        caja = rect_escalado(caja, 93);
    }
    let pieza = if !c.marcado {
        kit::gris(esfera)
    } else if esfera {
        kit::esfera_activa(c.variante)
    } else {
        kit::casilla(c.variante)
    };
    let cx = caja.x + (caja.w as i32) / 2;
    let cy = caja.y + (caja.h as i32) / 2;
    match pieza {
        Some(img) => r.imagen_filtrada(caja, img, brillo, saturacion, opacidad),
        // Sin kit: la forma a mano, para que el control nunca desaparezca.
        None => {
            if esfera {
                r.circulo(cx, cy, (ancho / 2).max(2) as i32, tema.paleta.borde);
                r.circulo(
                    cx,
                    cy,
                    (ancho / 2).saturating_sub(1).max(1) as i32,
                    tema.paleta.panel,
                );
            } else {
                r.relleno(caja, tema.paleta.panel);
                r.borde(caja, tema.paleta.borde);
            }
        }
    }
    if c.marcado {
        if esfera {
            // `dot.png`: un punto de 232 px dentro de un lienzo de 1000,
            // equivalente al 34 % del alto de la esfera.
            let d = extremo(alto, 34, 100).max(2);
            let s = (d as u64 * 1000 + 116) / 232;
            let ix = cx - ((501 * s + 500) / 1000) as i32;
            let iy = cy - ((496 * s + 500) / 1000) as i32;
            match kit::punto() {
                Some(img) => r.imagen_filtrada(
                    Rect {
                        x: ix,
                        y: iy,
                        w: s as u32,
                        h: s as u32,
                    },
                    img,
                    brillo,
                    saturacion,
                    opacidad,
                ),
                None => r.circulo(
                    cx,
                    cy,
                    (d / 2).max(1) as i32,
                    tinta_css(0, opacidad, saturacion),
                ),
            }
        } else {
            // `check_v.png`: el dibujo ocupa 611 px del lienzo de 1000 y el
            // CSS lo pinta a 1,15 × el alto de la casilla, CENTRADO en ella,
            // así que las puntas sobresalen (como en el Aqua clásico).
            let tick = extremo(alto, 115, 100).max(4);
            let s = (tick as u64 * 1000 + 305) / 611;
            let ix = cx - ((627 * s + 500) / 1000) as i32;
            let iy = cy - ((372 * s + 500) / 1000) as i32;
            match kit::marca() {
                Some(img) => r.imagen_filtrada(
                    Rect {
                        x: ix,
                        y: iy,
                        w: s as u32,
                        h: s as u32,
                    },
                    img,
                    brillo,
                    saturacion,
                    opacidad,
                ),
                None => {
                    // Respaldo: un tick grueso a mano con dos trazos.
                    let w = (alto / 3).max(3);
                    r.relleno(
                        Rect {
                            x: cx - w as i32,
                            y: cy,
                            w,
                            h: (w / 2 + 1).max(2),
                        },
                        tinta_css(0, opacidad, saturacion),
                    );
                    r.relleno(
                        Rect {
                            x: cx,
                            y: cy - (w / 2) as i32,
                            w: w + 2,
                            h: (w * 2 / 3 + 1).max(3),
                        },
                        tinta_css(0, opacidad, saturacion),
                    );
                }
            }
        }
    }
    if !c.texto.is_empty() {
        // `--aqua-check-text: #1d1d1f` sobre la pieza clara.
        let alto_fuente = (alto.saturating_mul(62) + 50) / 100;
        r.texto_tamano(
            rect.x + ancho as i32 + layout::hueco_marca(alto) as i32,
            rect.y + (rect.h as i32 - alto_fuente as i32) / 2,
            c.texto,
            tinta_css(0x1D1D1F, opacidad, saturacion),
            alto_fuente,
        );
    }
    if c.estado == Estado::Focused {
        anillo_foco(r, rect, 3, 3);
    }
}

/// `<circular>`: botón redondo.
///
/// `circular.css`: la cara es la MISMA esfera que usa el radio —`gray_radio.png`
/// o `<color>_radio.png`— pero SIN punto, y el texto (si tiene) va centrado
/// con la tinta azul marino del kit. Apagado → esfera gris.
fn pintar_circular<R: Renderer>(r: &mut R, _tema: &Tema, rect: Rect, c: &Control<'_>) {
    let alto = rect.h;
    let ancho = super::layout::ancho_esfera(alto).min(rect.w).max(1);
    let caja = Rect {
        x: rect.x + ((rect.w - ancho) / 2) as i32,
        y: rect.y,
        w: ancho,
        h: alto,
    };
    let gris = c.estado == Estado::Disabled;
    let (brillo, saturacion, opacidad) = filtro_pieza(c.estado);
    let caja_imagen = if c.estado == Estado::Pressed {
        rect_escalado(caja, 93)
    } else {
        caja
    };
    let pieza = if c.variante.is_empty() && !gris {
        kit::gris(true)
    } else {
        kit::esfera(c.variante)
    };
    let Some(img) = pieza else {
        // Los circulares siempre usan artwork del kit AppKit. Si no está
        // instalado, fallan cerrados en lugar de dibujar una esfera distinta.
        return;
    };
    r.imagen_filtrada(caja_imagen, img, brillo, saturacion, opacidad);
    if !c.texto.is_empty() {
        let alto_fuente = (rect.h.saturating_mul(42) + 50) / 100;
        let ancho_t = layout::ancho_texto_css(c.texto, alto_fuente) as i32;
        r.texto_tamano(
            rect.x + (rect.w as i32 - ancho_t) / 2,
            rect.y + (rect.h as i32 - alto_fuente as i32) / 2,
            c.texto,
            tinta_css(0x0A2E52, opacidad, saturacion),
            alto_fuente,
        );
    }
    if c.estado == Estado::Focused {
        anillo_foco(r, rect, 3, 3);
    }
}

/// `<segmented>`: control segmentado estilo macOS, montado segmento a
/// segmento con las piezas del kit.
///
/// `segmented.css`: cada opción es [izquierda] + [cuerpo estirado] +
/// [derecha]. Sin activar lleva las piezas BLANCAS (`segmented_withe_*`) y la
/// activa las de color (`segmented_<color>_*`); los extremos redondeados del
/// control (`withe_right` y `<color>_left`) se usan VOLTEADOS en los bordes
/// que no les corresponden. El contenedor no pinta nada.
fn pintar_segmentado<R: Renderer>(r: &mut R, _tema: &Tema, rect: Rect, c: &Control<'_>) {
    let gris = c.estado == Estado::Disabled;
    let opacidad = if gris { 50 } else { 100 };
    // AppKit es la única fuente visual del control: si falta cualquier piel,
    // se omite completo en lugar de dibujar segmentos aproximados.
    let (Some(activo), Some(pasivo)) = (
        kit::segmentado(kit::color_activo(c.variante)),
        kit::segmentado("white"),
    ) else {
        return;
    };
    let items = c.texto;
    let cuenta = component::cuenta_items(items);
    if cuenta == 0 {
        return;
    }
    let sel = (c.valor.max(0) as usize).min(cuenta - 1);
    let cap = extremo(rect.h, 170, 418);
    for (i, fila) in rectos_segmentos(items, rect).into_iter().enumerate() {
        let encendido = i == sel;
        let p = if encendido { activo } else { pasivo };
        let primero = i == 0;
        let ultimo = i + 1 == cuenta;
        // Qué pieza va a cada extremo y si hay que girarla: las
        // reglas de `:first-child`/`:last-child`/`:checked` del CSS.
        let (pizq, gira_izq) = if encendido {
            if primero {
                (&p.izq, false)
            } else {
                (&p.der, true)
            }
        } else if primero {
            (&p.der, true)
        } else {
            (&p.izq, false)
        };
        let (pder, gira_der) = if encendido {
            if ultimo {
                (&p.izq, true)
            } else {
                (&p.der, false)
            }
        } else if ultimo {
            (&p.der, false)
        } else {
            (&p.izq, true)
        };
        tres_piezas_filtradas(
            r, &p.cuerpo, fila, cap, pizq, gira_izq, pder, gira_der, 100, 100, opacidad,
        );
        let opcion = component::item_en(items, i);
        let color = if encendido {
            tinta::ACTIVA
        } else if c.estado == Estado::Hover && !gris {
            0x1C1C1E
        } else {
            tinta::CLARA
        };
        let alto_fuente = (rect.h.saturating_mul(42) + 50) / 100;
        let tw = layout::ancho_texto_css(opcion, alto_fuente) as i32;
        r.texto_tamano(
            fila.x + (fila.w as i32 - tw) / 2,
            fila.y + (fila.h as i32 - alto_fuente as i32) / 2,
            opcion,
            tinta_css(color, opacidad, 100),
            alto_fuente,
        );
    }
    if c.estado == Estado::Focused {
        anillo_foco(r, rect, -3, 3);
    }
}

/// `<popup>` cerrado: [opción elegida en blanco] + [botón de flechas].
///
/// `popup.css`: la parte blanca se monta con `segmented_withe_{right,body,left}`
/// (los DOS extremos volteados: el popup no redondea) y la de la derecha con
/// las piezas de color del mismo modo; en el botón van las DOS flechas del
/// kit, juntas y con una raya entre ellas. El texto va centrado en la parte
/// blanca con la tinta gris del kit.
///
/// El menú desplegado lo pinta [`pintar_menus`] después del árbol, para que
/// quede ENCIMA de los demás componentes.
fn pintar_disparador<R: Renderer>(r: &mut R, tema: &Tema, rect: Rect, c: &Control<'_>) {
    let gris = c.estado == Estado::Disabled;
    let opacidad = if gris { 50 } else { 100 };
    let paso_w = super::layout::ancho_paso(rect.h).min(rect.w);
    let blanco = Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w - paso_w,
        h: rect.h,
    };
    let paso = Rect {
        x: rect.der() - paso_w as i32,
        y: rect.y,
        w: paso_w,
        h: rect.h,
    };
    let cap = extremo(rect.h, 170, 418);
    match kit::segmentado("white") {
        Some(w) => tres_piezas_filtradas(
            r, &w.cuerpo, blanco, cap, &w.der, true, &w.izq, true, 100, 100, opacidad,
        ),
        None => r.relleno(
            blanco,
            mezclar(tema.paleta.panel, 0xFFFFFF, 100 - opacidad as u32),
        ),
    }
    let activo = kit::segmentado(kit::color_activo(c.variante));
    match activo {
        Some(a) => tres_piezas_filtradas(
            r, &a.cuerpo, paso, cap, &a.der, true, &a.izq, true, 100, 100, opacidad,
        ),
        None => r.relleno(
            paso,
            mezclar(
                theme::acento_de(tema, c.variante),
                0xFFFFFF,
                100 - opacidad as u32,
            ),
        ),
    }
    // Opción elegida, centrada en la parte blanca.
    let elegida = if c.valor > 0 { c.valor as usize } else { 0 };
    let texto = component::item_en(c.texto, elegida);
    let alto_fuente = (rect.h.saturating_mul(42) + 50) / 100;
    let tw = layout::ancho_texto_css(texto, alto_fuente) as i32;
    let color = if c.estado == Estado::Hover && !gris {
        0x1C1C1E
    } else {
        tinta::CLARA
    };
    r.texto_tamano(
        blanco.x + (blanco.w as i32 - tw) / 2,
        rect.y + (rect.h as i32 - alto_fuente as i32) / 2,
        texto,
        tinta_css(color, opacidad, 100),
        alto_fuente,
    );
    // La raya del medio, `rgba(8, 28, 58, .38)` sobre la pieza de color.
    if let Some(a) = activo {
        let x1 = paso.x + ((paso.w as i32) * 12 / 100);
        let x2 = paso.x + ((paso.w as i32) * 88 / 100);
        let y = rect.y + (rect.h as i32) / 2;
        if x2 > x1 && y >= rect.y && y < rect.abajo() {
            r.relleno(
                Rect {
                    x: x1,
                    y,
                    w: (x2 - x1) as u32,
                    h: 1,
                },
                mezclar(raya_sobre(&a.cuerpo), 0xFFFFFF, 100 - opacidad as u32),
            );
        }
    }
    // Las dos flechas, apretadas y centradas en el botón.
    let fh = extremo(rect.h, 22, 100).max(3);
    let fw = extremo(fh, 631, 431).max(2);
    let hueco = extremo(fh, 3, 10).max(1);
    let py = rect.y + ((rect.h.saturating_sub(fh * 2 + hueco)) / 2) as i32;
    let px = paso.x + ((paso.w.saturating_sub(fw)) / 2) as i32;
    if let Some(arriba) = kit::flecha_arriba() {
        r.imagen_filtrada(
            Rect {
                x: px,
                y: py,
                w: fw,
                h: fh,
            },
            arriba,
            100,
            100,
            opacidad,
        );
    }
    let y_abajo = py + (fh + hueco) as i32;
    if let Some(abajo) = kit::flecha() {
        r.imagen_filtrada(
            Rect {
                x: px,
                y: y_abajo,
                w: fw,
                h: fh,
            },
            abajo,
            100,
            100,
            opacidad,
        );
    }
    if c.estado == Estado::Focused {
        anillo_foco(r, rect, -3, 3);
    }
}

/// La raya entre las flechas de un `popup`: `rgba(8, 28, 58, .38)` mezclado
/// con el píxel CENTRAL de la pieza de color (asi sale el tono exacto que
/// pinta el CSS sobre esa piel, sin adivinar colores).
fn raya_sobre(img: &Imagen) -> u32 {
    let i = (img.alto / 2) as usize * img.ancho as usize + (img.ancho / 2) as usize;
    let p = img.pixeles.get(i).copied().unwrap_or(0);
    let mez = |dest: u32, fuente: u32| (fuente * 38 + dest * 62) / 100;
    (mez((p >> 16) & 0xFF, 8) << 16) | (mez((p >> 8) & 0xFF, 28) << 8) | mez(p & 0xFF, 58)
}

/// `<field>`: entrada de texto.
///
/// `field.css`: piel con las piezas BLANCAS del segmented pero SIN redondear:
/// [withe_left] + [withe_body estirable] + [withe_left VOLTEADA]. El texto
/// arranca pasado el extremo, con la tinta gris del kit; el placeholder es
/// más tenue; con foco aparece el cursor (y el anillo azul claro).
fn pintar_campo<R: Renderer>(r: &mut R, tema: &Tema, rect: Rect, c: &Control<'_>) {
    let cap = extremo(rect.h, 170, 418);
    let gris = c.estado == Estado::Disabled;
    let opacidad = if gris { 50 } else { 100 };
    match kit::segmentado("white") {
        Some(w) => tres_piezas_filtradas(
            r, &w.cuerpo, rect, cap, &w.izq, false, &w.izq, true, 100, 100, opacidad,
        ),
        None => {
            r.relleno(rect, tema.paleta.panel);
            r.borde(rect, tema.paleta.borde);
        }
    }
    let alto_fuente = (rect.h.saturating_mul(42) + 50) / 100;
    let color = tinta_css(0x3C3C3E, opacidad, 100);
    let ty = rect.y + (rect.h as i32 - alto_fuente as i32) / 2;
    let tx = rect.x + cap as i32;
    // El texto se recorta al campo: no puede desbordarlo.
    r.recorte(rect);
    if c.texto.is_empty() {
        if !c.placeholder.is_empty() {
            r.texto_tamano(
                tx,
                ty,
                c.placeholder,
                tinta_css(0x9A9A9E, opacidad, 100),
                alto_fuente,
            );
        }
    } else {
        r.texto_tamano(tx, ty, c.texto, color, alto_fuente);
    }
    if c.estado == Estado::Focused {
        let cx = tx + layout::ancho_texto_css(c.texto, alto_fuente) as i32;
        let h = alto_fuente.max(1);
        r.relleno(
            Rect {
                x: cx + 1,
                y: rect.y + ((rect.h - h) / 2) as i32,
                w: 1,
                h,
            },
            tinta_css(0x3C3C3E, opacidad, 100),
        );
    }
    r.recorte_fin();
    if c.estado == Estado::Focused {
        anillo_foco(r, rect, 2, 3);
    }
}

/// `<slider>` y `<slider-gota>`: la esfera/gota queda centrada dentro de la pista.
///
/// La pista son las tres piezas `graybar_*` y tiene la misma altura que el
/// pulgar (55 % del control), de modo que la esfera o la gota quedan enteras
/// dentro de la barra. No hay tramo recorrido pintado: en el kit es sólo
/// pista gris y pulgar.
fn pintar_slider<R: Renderer>(r: &mut R, tema: &Tema, rect: Rect, c: &Control<'_>) {
    let gris = c.estado == Estado::Disabled;
    let opacidad = if gris { 55 } else { 100 };
    let pista_alto = alto_pista(rect.h);
    let cap = extremo(pista_alto, 168, 298);
    let y_pista = y_pista_slider(rect, pista_alto);
    let pista = Rect {
        x: rect.x,
        y: y_pista,
        w: rect.w,
        h: pista_alto,
    };
    if let Some(carril) = kit::carril() {
        tres_piezas_filtradas(
            r,
            &carril.cuerpo,
            pista,
            cap,
            &carril.izq,
            false,
            &carril.der,
            false,
            100,
            100,
            opacidad,
        );
    } else {
        r.relleno(pista, tema.paleta.panel);
        r.borde(pista, tema.paleta.borde);
    }
    let gota = c.etiqueta == "slider-gota";
    let alto_p = alto_pulgar(rect.h).min(rect.h).max(1);
    let ancho_p = ancho_pulgar(alto_p, gota).min(rect.w).max(1);
    let pieza = if gota {
        kit::gota(c.variante)
    } else {
        kit::esfera_activa(c.variante)
    };
    let pct = c.valor.clamp(0, 100) as u32;
    let tx = rect.x + (((rect.w - ancho_p) * pct) / 100) as i32;
    let ty = y_pulgar_slider(rect, alto_p);
    let (brillo, saturacion, _) = filtro_pieza(c.estado);
    let pulgar = rect_escalado(
        Rect {
            x: tx,
            y: ty,
            w: ancho_p,
            h: alto_p,
        },
        if c.estado == Estado::Pressed { 94 } else { 100 },
    );
    match pieza {
        Some(img) => r.imagen_filtrada(
            pulgar,
            img,
            if gris { 100 } else { brillo },
            if gris { 30 } else { saturacion },
            opacidad,
        ),
        // Sin kit: esfera dibujada a mano.
        None => {
            let cx = tx + (ancho_p / 2) as i32;
            let cy = ty + (alto_p / 2) as i32;
            let radio = (alto_p / 2).max(2) as i32;
            let base = if gris {
                tema.acentos.gris
            } else {
                theme::acento_de(tema, c.variante)
            };
            r.circulo(cx + 1, cy + 2, radio, theme::color_sombra(tema));
            r.circulo(cx, cy, radio, theme::oscurecer(base, 25));
            r.circulo(cx, cy, (radio - 1).max(1), base);
            r.circulo(cx, cy - 1, (radio - 2).max(1), theme::aclarar(base, 30));
        }
    }
    if c.estado == Estado::Focused {
        anillo_foco(r, rect, 3, 3);
    }
}

/// Alto de la pista de un `<slider>`: igual al pulgar, 55 % del control.
pub fn alto_pista(alto: u32) -> u32 {
    alto_pulgar(alto)
}

/// Alto del pulgar de un `<slider>`: el 55 % del alto del control.
pub fn alto_pulgar(alto: u32) -> u32 {
    extremo(alto, 55, 100)
}

/// Posición vertical centrada de la pista dentro del control.
pub fn y_pista_slider(rect: Rect, alto_pista: u32) -> i32 {
    rect.y + ((rect.h.saturating_sub(alto_pista)) / 2) as i32
}

/// Posición vertical centrada del pulgar dentro del control.
pub fn y_pulgar_slider(rect: Rect, alto_pulgar: u32) -> i32 {
    rect.y + ((rect.h.saturating_sub(alto_pulgar)) / 2) as i32
}

/// Ancho del pulgar para un alto dado: la esfera (682 : 653) o, en el
/// `slider-gota`, la gota (404 : 371).
pub fn ancho_pulgar(alto: u32, gota: bool) -> u32 {
    if gota {
        extremo(alto, 404, 371)
    } else {
        super::layout::ancho_esfera(alto)
    }
}

/// Altura del menú desplegado de un `popup` con `cuenta` opciones.
pub fn alto_menu(cuenta: usize) -> u32 {
    cuenta as u32 * ALTO_ITEM + 4
}

/// Rectángulo de cada opción de un `segmented` dentro de su control.
///
/// Las opciones TAPAN el control entero, una junto a la otra y a toda la
/// altura, igual que los `<label>` del `segmented.css` (el marco no pinta
/// nada, así que no hay hueco que dejarle). El reparto es proporcional al
/// ancho natural de cada opción (texto y padding CSS). Sigue `flex: 1 1 auto`:
/// el espacio libre positivo se reparte por igual; si falta espacio, el
/// encogimiento es proporcional a la base de cada etiqueta.
/// Lo usan EL MISMO pintado y la detección de clics de `app::App`: lo que se
/// pinta es lo que se puede pulsar.
pub fn rectos_segmentos(items: &str, rect: Rect) -> Vec<Rect> {
    let cuenta = component::cuenta_items(items);
    let bases: Vec<u32> = (0..cuenta)
        .map(|i| layout::ancho_item(items, i, rect.h))
        .collect();
    let base_total: u64 = bases.iter().map(|w| *w as u64).sum();
    let anchos: Vec<u32> = if rect.w as u64 >= base_total {
        let extra = rect.w as u64 - base_total;
        let por_item = extra / cuenta as u64;
        let resto = extra % cuenta as u64;
        bases
            .iter()
            .enumerate()
            .map(|(i, w)| (*w as u64 + por_item + u64::from((i as u64) < resto)) as u32)
            .collect()
    } else {
        let total = base_total.max(1);
        let mut repartidos = Vec::with_capacity(cuenta);
        let mut usado = 0u32;
        for (i, base) in bases.iter().enumerate() {
            let w = if i + 1 == cuenta {
                rect.w.saturating_sub(usado)
            } else {
                (rect.w as u64 * *base as u64 / total) as u32
            };
            repartidos.push(w);
            usado = usado.saturating_add(w);
        }
        repartidos
    };
    let mut fuera = Vec::with_capacity(cuenta);
    let mut acc = 0u32;
    for sw in anchos {
        fuera.push(Rect {
            x: rect.x + acc as i32,
            y: rect.y,
            w: sw,
            h: rect.h,
        });
        acc = acc.saturating_add(sw);
    }
    fuera
}

/// Rectángulo del menú desplegado de un `popup`, debajo de su disparador.
///
/// Esta geometría la comparten el pintado y los eventos (`app::App`): lo que
/// se pinta es exactamente lo que se puede pulsar.
pub fn rect_menu(items: &str, disparador: Rect) -> Rect {
    Rect {
        x: disparador.x,
        y: disparador.abajo() + 2,
        w: disparador.w.max(96),
        h: alto_menu(component::cuenta_items(items)),
    }
}

/// Opción del menú que cae en una coordenada Y local del menú, si cae en ella.
pub fn item_menu(items: &str, ly: i32) -> Option<usize> {
    if ly < 2 {
        return None;
    }
    let i = ((ly - 2) / ALTO_ITEM as i32) as usize;
    if i < component::cuenta_items(items) {
        Some(i)
    } else {
        None
    }
}

/// Pinta por encima de todo el árbol los menús de los `popup` desplegados.
fn pintar_menus<R: Renderer>(
    r: &mut R,
    pre: &[&Nodo],
    disp: &Disposicion,
    tema: &Tema,
    valores: &super::valores::Valores,
) {
    for i in 0..pre.len() {
        let n = pre[i];
        if n.def.etiqueta != "popup" || !valores.abierto(i) {
            continue;
        }
        let rect = disp.rect(i);
        if rect.w == 0 || rect.h == 0 {
            continue;
        }
        let items = n.texto("items", "");
        let variante = n.texto("variant", "blue");
        let seleccion = valores.entero(i);
        let menu = rect_menu(items, rect);
        r.relleno(menu, tema.paleta.panel);
        r.borde(menu, tema.paleta.borde);
        for k in 0..component::cuenta_items(items) {
            let fila = Rect {
                x: menu.x + 2,
                y: menu.y + 2 + (k as i32) * (ALTO_ITEM as i32),
                w: menu.w.saturating_sub(4),
                h: ALTO_ITEM,
            };
            let elegida = k as i64 == seleccion;
            if elegida {
                r.relleno(fila, theme::acento_de(tema, variante));
            }
            r.texto(
                fila.x + 6,
                fila.y + (ALTO_ITEM as i32 - ALTO_LINEA as i32) / 2,
                component::item_en(items, k),
                if elegida {
                    theme::texto_de(tema, variante)
                } else {
                    tema.paleta.texto
                },
                1,
            );
        }
    }
}

/// Pinta un nodo `<image>`: carga el PNG autorizado y lo ajusta al rect.
///
/// `fit` decide el ajuste: `stretch` (por defecto) deforma la imagen al rect;
/// `contain` la encaja entera sin deformar (con bandas); `cover` la agranda
/// hasta cubrir el rect y recorta el sobrante. Sin recurso se pinta una caja
/// gris: el fallo queda visible, nunca tumba nada.
fn pintar_imagen<R: Renderer>(r: &mut R, n: &Nodo, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let ruta = n.texto("src", "");
    let img = match super::kit::imagen_recurso(ruta) {
        Some(i) => i,
        None => {
            r.relleno(rect, 0xC9CED4);
            return;
        }
    };
    let iw = img.ancho.max(1) as i64;
    let ih = img.alto.max(1) as i64;
    let rw = rect.w as i64;
    let rh = rect.h as i64;
    // Rectángulo que ocupa la imagen dentro del rect, según `fit`.
    let (w, h) = match n.texto("fit", "stretch") {
        "contain" => {
            let (mut w, mut h) = (rw, rw * ih / iw);
            if h > rh {
                h = rh;
                w = rh * iw / ih;
            }
            (w, h)
        }
        "cover" => {
            let (mut w, mut h) = (rw, rw * ih / iw);
            if h < rh {
                h = rh;
                w = rh * iw / ih;
            }
            (w, h)
        }
        _ => (rw, rh),
    };
    let w = w.max(1) as u32;
    let h = h.max(1) as u32;
    let x = rect.x + ((rw - w as i64) / 2) as i32;
    let y = rect.y + ((rh - h as i64) / 2) as i32;
    let destino = Rect { x, y, w, h };
    if n.texto("fit", "stretch") == "cover" {
        r.recorte(rect);
        r.imagen(destino, img);
        r.recorte_fin();
    } else {
        r.imagen(destino, img);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn escala_mascaras_tipograficas_con_cobertura_bilineal() {
        assert_eq!(coordenada_mascara(0, 2, 4), (0, 1, 32_768));
        assert_eq!(interpolar_cobertura(0, 255, 0, 255, 32_768, 0), 128);
        assert_eq!(interpolar_cobertura(96, 96, 96, 96, 17_000, 51_000), 96);
    }

    /// Imagen de prueba de `w` x `h` con los píxeles dados.
    fn imagen(w: u32, h: u32, pixeles: &[u32]) -> Imagen {
        Imagen {
            ancho: w,
            alto: h,
            pixeles: alloc::vec::Vec::from(pixeles),
        }
    }

    /// Alfa y canal rojo de una muestra (para los mensajes de error).
    fn rojo(color: u32) -> u32 {
        (color >> 16) & 0xFF
    }

    #[test]
    fn al_reducir_promedia_el_area() {
        // 4x1 con dos negros y dos blancos reducida a 1x1: sale un gris medio.
        // Con vecino más próximo saldría negro puro (el escalón de siempre).
        let img = imagen(4, 1, &[0xFF00_0000, 0xFF00_0000, 0xFFFF_FFFF, 0xFFFF_FFFF]);
        let (color, a) = muestra_escalada(&img, 1, 1, 0, 0);
        assert_eq!(a, 255, "todo opaco, todo el área cubierta");
        assert!(
            rojo(color) > 100 && rojo(color) < 155,
            "esperaba gris medio, salió {:#08X}",
            color
        );
    }

    #[test]
    fn un_borde_duro_reducido_no_salta_de_0_a_255() {
        // Media izquierda transparente y derecha opaca, reducida a un ancho que
        // no divide exacto (16 -> 7): la transición pasa por algún píxel
        // semitransparente en vez de dar un escalón de 255 a 0.
        let mut src = alloc::vec![0u32; 16];
        for (i, p) in src.iter_mut().enumerate() {
            *p = if i < 8 { 0 } else { 0xFF00_0000 };
        }
        let img = imagen(16, 1, &src);
        let mut anterior = 0;
        let mut saltos = 0;
        let mut parciales = 0;
        for dx in 0..7 {
            let (_, a) = muestra_escalada(&img, 7, 1, dx, 0);
            if a > 0 && a < 255 {
                parciales += 1;
            }
            if anterior > 0 && a > anterior && a - anterior > 200 {
                saltos += 1;
            }
            anterior = a;
        }
        assert!(parciales > 0, "el borde necesita un píxel intermedio");
        assert_eq!(saltos, 0, "no puede haber un salto mayor de 200");
    }

    #[test]
    fn la_silueta_de_ventana_suaviza_el_arco_con_muestras_subpixel() {
        assert_eq!(RADIO_ESQUINA_VENTANA, 12);
        let (dentro, interior) = cobertura_esquina(7, 0, RADIO_ESQUINA_VENTANA);
        assert!(
            dentro > 0 && dentro < 64,
            "el arco debe tener cobertura parcial"
        );
        assert_eq!(interior, 0, "este píxel sólo mezcla borde y exterior");
        let pixel = color_cobertura(0xDDE1E5, 0xAAB0B6, 0xF7F7F7, dentro, interior);
        assert_ne!(
            pixel, 0xDDE1E5,
            "el borde suavizado no debe ser un píxel duro"
        );
        assert_ne!(
            pixel, 0xAAB0B6,
            "la cobertura debe mezclar el color del borde"
        );
    }

    #[test]
    fn al_ampliar_interpola_sin_escalones() {
        // Negro duro junto a blanco duro, ampliado a 8x1: la rampa sube sin
        // saltos grandes, y los extremos siguen siendo negro y blanco.
        let img = imagen(2, 1, &[0xFF00_0000, 0xFFFF_FFFF]);
        let mut anterior = 0;
        for dx in 0..8 {
            let (color, a) = muestra_escalada(&img, 8, 1, dx, 0);
            assert_eq!(a, 255);
            let r = rojo(color) as i32;
            assert!(
                r >= anterior,
                "la rampa no puede bajar ({} -> {})",
                anterior,
                r
            );
            assert!(
                r - anterior <= 100,
                "salto demasiado grande en la rampa ({} -> {})",
                anterior,
                r
            );
            anterior = r;
        }
        assert_eq!(rojo(muestra_escalada(&img, 8, 1, 0, 0).0), 0);
        assert_eq!(rojo(muestra_escalada(&img, 8, 1, 7, 0).0), 255);
    }

    #[test]
    fn a_tamano_uno_a_uno_copia_el_pixel_exacto() {
        // Sin escalar no se toca nada, ni el color ni el alfa.
        let src = [0xFF12_3456, 0x80AB_CDEF, 0x0000_0000, 0xFFED_CBA9];
        let img = imagen(2, 2, &src);
        for i in 0..4 {
            let (dx, dy) = ((i % 2) as i32, (i / 2) as i32);
            let (color, a) = muestra_escalada(&img, 2, 2, dx, dy);
            assert_eq!(color, src[i] & 0x00FF_FFFF, "color en ({}, {})", dx, dy);
            assert_eq!(a, src[i] >> 24, "alfa en ({}, {})", dx, dy);
        }
    }

    #[test]
    fn imagen_recortada_conserva_el_encuadre_del_rectangulo_original() {
        let img = imagen(4, 1, &[0xFF11_0000, 0xFF22_0000, 0xFF33_0000, 0xFF44_0000]);
        let mut buffer = [0u32; 2];
        #[allow(unused_unsafe)]
        let mut lienzo = unsafe { Canvas::from_buffer(buffer.as_mut_ptr(), 2, 1, 2) };
        {
            let mut r = CanvasRenderer::nuevo(&mut lienzo, None);
            r.recorte(Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 1,
            });
            r.imagen(
                Rect {
                    x: -1,
                    y: 0,
                    w: 4,
                    h: 1,
                },
                &img,
            );
            r.recorte_fin();
        }
        assert_eq!(buffer, [0x0022_0000, 0x0033_0000]);
    }

    #[test]
    fn fragmento_escalado_no_mezcla_pixeles_de_los_cortes_vecinos() {
        let img = imagen(3, 1, &[0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF]);
        for dx in 0..5 {
            let (color, alpha) = muestra_fragmento_escalada(
                &img,
                Rect {
                    x: 1,
                    y: 0,
                    w: 1,
                    h: 1,
                },
                5,
                1,
                dx,
                0,
            );
            assert_eq!(color, 0x0000_FF00);
            assert_eq!(alpha, 255);
        }
    }

    #[test]
    fn el_alfa_va_premultiplicado() {
        // Un píxel transparente (RGB 0,0,0) al lado de uno blanco opaco,
        // promediados: blanco al 50 %, no gris al 50 %.
        let img = imagen(2, 1, &[0xFFFF_FFFF, 0x0000_0000]);
        let (color, a) = muestra_escalada(&img, 1, 1, 0, 0);
        assert_eq!(color & 0x00FF_FFFF, 0x00FF_FFFF);
        assert!(a > 120 && a < 135, "media de alfas, salió {}", a);
    }

    #[test]
    fn la_pildora_verde_reparte_cerrar_maximizar_y_minimizar() {
        let rect = Rect {
            x: 0,
            y: 0,
            w: 640,
            h: 448,
        };
        assert_eq!(
            control_ventana_en(rect, 22, 14),
            Some(ControlVentana::Cerrar)
        );
        assert_eq!(
            control_ventana_en(rect, 46, 14),
            Some(ControlVentana::Maximizar)
        );
        assert_eq!(
            control_ventana_en(rect, 70, 14),
            Some(ControlVentana::Minimizar)
        );
        assert_eq!(control_ventana_en(rect, 90, 14), None);
    }

    #[test]
    fn los_controles_appkit_ocupan_la_cabecera_y_comparten_su_hitbox() {
        const ALTO_CABECERA: u32 = 56;
        let rect = Rect {
            x: 20,
            y: 12,
            w: 640,
            h: 448,
        };
        let izquierda = rect_boton_cabecera(rect, ALTO_CABECERA, 0).expect("primer botón visible");
        let derecha = rect_boton_cabecera(rect, ALTO_CABECERA, 1).expect("segundo botón visible");

        assert_eq!(izquierda.y, rect.y + 28);
        assert_eq!(izquierda.h, 24);
        assert_eq!(izquierda.w, 25, "proporción nativa de white_radio.png");
        assert_eq!(izquierda.x, rect.x + 19);
        assert!(izquierda.der() < derecha.x, "los botones no se solapan");
        let centro_grupo = (izquierda.x + derecha.der()) / 2;
        let centro_pildora =
            rect_pildora_verde(rect).unwrap().x + rect_pildora_verde(rect).unwrap().w as i32 / 2;
        assert_eq!(centro_grupo, centro_pildora);
        assert!(izquierda.y >= rect.y + ALTO_BARRA_VENTANA as i32);
        assert_eq!(derecha.abajo(), rect.y + ALTO_CABECERA as i32 - 4);
        assert_eq!(
            boton_cabecera_en(
                rect,
                ALTO_CABECERA,
                derecha.x + derecha.w as i32 / 2,
                derecha.y + derecha.h as i32 / 2,
                2,
            ),
            Some(1)
        );
        assert_eq!(boton_cabecera_en(rect, ALTO_CABECERA, 20, 16, 2), None);
    }

    // --- F10: geometría de los controles nuevos (CSS del kit AQUA) ---

    #[test]
    fn extremo_redondea_al_entero_mas_cercano() {
        // 18 x 1,15 = 20,7 (el tick) y 24 x 1,3 = 31,2 (el paso del popup).
        assert_eq!(extremo(18, 115, 100), 21);
        assert_eq!(extremo(24, 13, 10), 31);
        // Fracciones pequeñas y casos extremos no pueden dar 0 ni reventar.
        assert_eq!(extremo(5, 22, 100), 1);
        assert_eq!(extremo(24, 0, 100), 0);
        assert_eq!(extremo(0, 115, 100), 0);
    }

    #[test]
    fn pista_y_pulgar_siguen_las_proporciones_del_kit() {
        // La pista y el pulgar ocupan ambos el 55 % del alto del control.
        assert_eq!(alto_pista(24), 13);
        assert_eq!(alto_pulgar(24), 13);
        // el pulgar ES la esfera del radio (682:653) o la gota (404:371).
        assert_eq!(
            ancho_pulgar(13, false),
            crate::toby_ui::layout::ancho_esfera(13)
        );
        assert_eq!(ancho_pulgar(13, true), extremo(13, 404, 371));
        // Un control diminuto no puede dar un pulgar de 0.
        assert!(ancho_pulgar(1, false) >= 1);
    }

    #[test]
    fn slider_centra_el_pulgar_en_la_pista_y_lo_mantiene_en_el_control() {
        for alto in [24, 26, 32, 48] {
            let rect = Rect {
                x: 12,
                y: 30,
                w: 240,
                h: alto,
            };
            let hp = alto_pista(rect.h);
            let ht = alto_pulgar(rect.h);
            let yp = y_pista_slider(rect, hp);
            let yt = y_pulgar_slider(rect, ht);

            // Puede haber medio píxel de diferencia por redondeo entero.
            assert_eq!(hp, ht, "la pista debe contener el pulgar completo");
            assert_eq!(
                yp, yt,
                "pista y pulgar deben compartir el mismo rectángulo vertical"
            );
            assert!((yp * 2 + hp as i32 - yt * 2 - ht as i32).abs() <= 1);
            assert!(yp >= rect.y && yp + hp as i32 <= rect.abajo());
            assert!(yt >= rect.y && yt + ht as i32 <= rect.abajo());
        }
    }

    #[test]
    fn los_segmentos_tapan_el_control_sin_huecos() {
        let rect = Rect {
            x: 10,
            y: 20,
            w: 100,
            h: 24,
        };
        let segs = rectos_segmentos("a|bb|ccc", rect);
        assert_eq!(segs.len(), 3);
        // Uno pegado al otro, todos a la altura entera y sin solaparse.
        let mut x = rect.x;
        for s in &segs {
            assert_eq!(s.x, x, "cada segmento empieza donde acaba el anterior");
            assert_eq!(s.y, rect.y);
            assert_eq!(s.h, rect.h);
            assert!(s.w > 0, "ningún segmento puede quedar vacío");
            x += s.w as i32;
        }
        // Y entre todos cubren el ancho exacto del control.
        assert_eq!(x, rect.der(), "los segmentos cubren el control entero");
        // Al encogerse, flex-shrink:1 reparte la falta según el ancho base.
        let anchos: alloc::vec::Vec<u32> = segs.iter().map(|s| s.w).collect();
        assert_eq!(anchos, alloc::vec![28, 33, 39]);
        // Si sobra espacio, flex-grow:1 lo añade por igual a cada opción.
        let anchos: alloc::vec::Vec<u32> = rectos_segmentos("a|bb|ccc", Rect { w: 135, ..rect })
            .iter()
            .map(|s| s.w)
            .collect();
        assert_eq!(anchos, alloc::vec![40, 45, 50]);
    }

    #[test]
    fn el_menu_del_popup_reparte_sus_opciones() {
        // 3 opciones: 3 x 20 + 2 + 2 = 64 de alto.
        assert_eq!(alto_menu(3), 64);
        let disparador = Rect {
            x: 30,
            y: 40,
            w: 120,
            h: 24,
        };
        let menu = rect_menu("Rojo|Verde|Azul", disparador);
        assert_eq!(menu.x, 30, "cuelga de su disparador");
        assert_eq!(menu.y, disparador.abajo() + 2);
        assert_eq!(menu.w, 120);
        assert_eq!(menu.h, 64);
        // Un disparador estrecho no estrecha el menú por debajo del mínimo.
        assert_eq!(
            rect_menu(
                "a|b",
                Rect {
                    x: 0,
                    y: 0,
                    w: 95,
                    h: 24
                }
            )
            .w,
            96
        );
        // Cada opción cae en su fila; el margen de 2 px no es de nadie.
        let items = "Rojo|Verde|Azul";
        assert_eq!(item_menu(items, 0), None);
        assert_eq!(item_menu(items, 3), Some(0));
        assert_eq!(item_menu(items, 23), Some(1));
        assert_eq!(item_menu(items, 61), Some(2));
        assert_eq!(item_menu(items, 63), None, "el pie de 2 px tampoco");
    }

    #[test]
    fn el_tick_del_check_sobresale_de_su_casilla() {
        // El CSS pinta el tick a 1,15 del alto CENTRADO en la casilla: las
        // puntas tienen que quedar fuera de ella (Aqua clásico).
        let alto = 18;
        let tick = extremo(alto, 115, 100);
        assert_eq!(tick, 21);
        assert!(
            tick > alto,
            "si el tick no es mayor que la casilla, no sobresale"
        );
        // La tinta ocupa 611 px del lienzo de 1000: el lienzo completo hay
        // que dibujarlo mayor todavía para que la tinta mida `tick`.
        let lienzo = (tick as u64 * 1000 + 305) / 611;
        assert!(lienzo as u32 > tick);
        // Y al centrar la tinta (627, 372) en el centro de la casilla, el
        // lienzo arranca POR ENCIMA de la casilla (las puntas sobresalen).
        let centro = (alto / 2) as i64;
        let tinta_arriba = ((372 * lienzo + 500) / 1000) as i64;
        assert!(
            tinta_arriba > centro,
            "la tinta ocupa más de medio alto por arriba"
        );
        assert!(
            centro - tinta_arriba < 0,
            "el lienzo empieza fuera (encima) de la casilla"
        );
    }
}
