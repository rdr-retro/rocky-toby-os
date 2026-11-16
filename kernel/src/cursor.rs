//! Cursores de Kira desde un paquete binario compacto.
//!
//! `xtask cursor-assets` convierte los originales BMP/PNG de `arrow/` al
//! paquete KCUR v2 antes de compilar el kernel. En ejecución solo se decodifican
//! índices de paleta; no hay lector BMP/PNG en este módulo.

use alloc::vec::Vec;

use crate::gfx::Canvas;
use crate::sync::SyncCell;

/// Paquete de cursores indexados generado fuera del kernel e incrustado al compilar.
static PAQUETE: &[u8] = include_bytes!("../assets/cursor_theme.bin");
const NUM_ESTADOS: usize = 8;

/// Separación respecto al borde al aparecer el cursor por primera vez.
pub const MARGEN_INICIAL: i32 = 24;

/// Estados del paquete KCUR. Los IDs se mantienen sincronizados con xtask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Estado {
    #[default]
    Flecha = 0,
    Mano = 1,
    Texto = 2,
    Mover = 3,
    RedimensionarHorizontal = 4,
    RedimensionarVertical = 5,
    RedimensionarDiagonal = 6,
    Pensando = 7,
}

impl Estado {
    pub fn desde_nombre(nombre: &str) -> Option<Self> {
        Some(match nombre {
            "arrow" | "flecha" => Self::Flecha,
            "hand" | "mano" => Self::Mano,
            "ibeam" | "texto" => Self::Texto,
            "move" | "mover" => Self::Mover,
            "resize-ew" | "horizontal" | "ew" => Self::RedimensionarHorizontal,
            "resize-ns" | "vertical" | "ns" => Self::RedimensionarVertical,
            "resize-nwse" | "diagonal" | "nwse" => Self::RedimensionarDiagonal,
            "spinning" | "busy" | "pensando" => Self::Pensando,
            _ => return None,
        })
    }

    fn indice(self) -> usize {
        self as usize
    }
}

/// Sprite precargado. Los píxeles son AARRGGBB y llevan su alfa real.
pub struct Sprite {
    pub ancho: i32,
    pub alto: i32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    ticks_por_fotograma: u8,
    cuadros: Vec<Vec<u32>>,
}

static VACIO: Sprite = Sprite {
    ancho: 0,
    alto: 0,
    hotspot_x: 0,
    hotspot_y: 0,
    ticks_por_fotograma: 0,
    cuadros: Vec::new(),
};

static TEMA: SyncCell<Option<Vec<Sprite>>> = SyncCell::new(None);

/// Lee KCUR v2: índices de 2/8 bpp y paleta ARGB para cada estado.
fn leer_paquete() -> Option<Vec<Sprite>> {
    if PAQUETE.get(..6)? != b"KCUR\x02\x08" {
        return None;
    }
    let mut sprites: [Option<Sprite>; NUM_ESTADOS] =
        [None, None, None, None, None, None, None, None];
    let mut pos = 6usize;
    for _ in 0..NUM_ESTADOS {
        let h = PAQUETE.get(pos..pos.checked_add(8)?)?;
        let (id, w, alto, hx, hy, n, periodo, bits) = (
            h[0] as usize,
            h[1] as usize,
            h[2] as usize,
            h[3] as i32,
            h[4] as i32,
            h[5] as usize,
            h[6],
            h[7],
        );
        pos += 8;
        if id >= NUM_ESTADOS
            || w == 0
            || alto == 0
            || w > 64
            || alto > 64
            || hx >= w as i32
            || hy >= alto as i32
            || n == 0
            || n > 32
            || (bits != 2 && bits != 8)
        {
            return None;
        }
        let cabecera_paleta = PAQUETE.get(pos..pos.checked_add(2)?)?;
        let cantidad_colores =
            u16::from_le_bytes([cabecera_paleta[0], cabecera_paleta[1]]) as usize;
        pos += 2;
        if cantidad_colores == 0 || cantidad_colores > 256 || (bits == 2 && cantidad_colores > 4) {
            return None;
        }
        let bytes_paleta = cantidad_colores.checked_mul(4)?;
        let fin_paleta = pos.checked_add(bytes_paleta)?;
        let datos_paleta = PAQUETE.get(pos..fin_paleta)?;
        let mut paleta = Vec::with_capacity(cantidad_colores);
        for color in datos_paleta.chunks_exact(4) {
            paleta.push(u32::from_le_bytes([color[0], color[1], color[2], color[3]]));
        }
        pos = fin_paleta;

        let pixeles_por_fotograma = w.checked_mul(alto)?;
        let pixeles_totales = pixeles_por_fotograma.checked_mul(n)?;
        let bytes_datos = if bits == 2 {
            pixeles_totales.checked_add(3)? / 4
        } else {
            pixeles_totales
        };
        let fin_datos = pos.checked_add(bytes_datos)?;
        let datos = PAQUETE.get(pos..fin_datos)?;
        let mut cuadros = Vec::with_capacity(n);
        for frame in 0..n {
            let mut pixeles = Vec::with_capacity(pixeles_por_fotograma);
            for i in 0..pixeles_por_fotograma {
                let indice_global = frame * pixeles_por_fotograma + i;
                let indice = if bits == 2 {
                    let byte = *datos.get(indice_global / 4)?;
                    ((byte >> ((indice_global % 4) * 2)) & 0x03) as usize
                } else {
                    *datos.get(indice_global)? as usize
                };
                pixeles.push(*paleta.get(indice)?);
            }
            cuadros.push(pixeles);
        }
        pos = fin_datos;
        sprites[id] = Some(Sprite {
            ancho: w as i32,
            alto: alto as i32,
            hotspot_x: hx,
            hotspot_y: hy,
            ticks_por_fotograma: periodo,
            cuadros,
        });
    }
    if pos != PAQUETE.len() || sprites.iter().any(Option::is_none) {
        return None;
    }
    Some(sprites.into_iter().map(Option::unwrap).collect())
}

fn tema() -> &'static [Sprite] {
    unsafe {
        let slot = &mut *TEMA.get();
        if slot.is_none() {
            *slot = leer_paquete();
        }
        slot.as_deref().unwrap_or(&[])
    }
}

pub fn sprite() -> &'static Sprite {
    sprite_estado(Estado::Flecha)
}

pub fn sprite_estado(estado: Estado) -> &'static Sprite {
    tema().get(estado.indice()).unwrap_or(&VACIO)
}

pub fn indice_fotograma(estado: Estado, ticks: u64) -> usize {
    let s = sprite_estado(estado);
    if s.cuadros.len() <= 1 || s.ticks_por_fotograma == 0 {
        0
    } else {
        ((ticks / s.ticks_por_fotograma as u64) as usize) % s.cuadros.len()
    }
}

/// Decodifica el paquete binario una vez, antes de entrar al escritorio.
pub fn preparar() {
    let _ = tema();
}

/// Puntero dibujado en software sobre el lienzo activo, guardando el fondo que
/// queda debajo para restaurarlo sin recomponer toda la escena. Si Kira usa
/// doble buffer, el puntero vive en ese mismo buffer y no se desincroniza del
/// framebuffer al presentar una escena nueva.
pub struct Superposicion {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    fondo: Vec<u32>,
    visible: bool,
}

impl Superposicion {
    pub const fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            fondo: Vec::new(),
            visible: false,
        }
    }

    #[allow(dead_code)]
    pub fn pisa(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.visible
            && self.x < x + w
            && x < self.x + self.w
            && self.y < y + h
            && y < self.y + self.h
    }

    pub fn ocultar(&mut self, c: &Canvas) {
        if let Some((x, y, w, h)) = self.ocultar_en_buffer(c) {
            c.present_rect(x, y, w, h);
        }
    }

    /// Restaura el fondo del puntero en el lienzo sin presentarlo todavía.
    /// Devuelve el área dañada para que el compositor pueda agruparla con el
    /// resto de cambios del mismo fotograma.
    pub fn ocultar_en_buffer(&mut self, c: &Canvas) -> Option<(i32, i32, i32, i32)> {
        if !self.visible {
            return None;
        }
        for j in 0..self.h {
            for i in 0..self.w {
                c.put(
                    self.x + i,
                    self.y + j,
                    self.fondo[(j * self.w + i) as usize],
                );
            }
        }
        self.visible = false;
        Some((self.x, self.y, self.w, self.h))
    }

    #[inline]
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Cursor arrow 1:1, conservado para llamadas antiguas.
    pub fn pintar(&mut self, c: &Canvas, mx: i32, my: i32) {
        self.pintar_estado(c, mx, my, 1, Estado::Flecha, 0);
    }

    /// Arrow escalada, conservada para la orden de accesibilidad existente.
    pub fn pintar_escalado(&mut self, c: &Canvas, mx: i32, my: i32, escala: i32) {
        self.pintar_estado(c, mx, my, escala, Estado::Flecha, 0);
    }

    /// Dibuja cualquier estado del tema; `mx,my` indican el hotspot del cursor.
    pub fn pintar_estado(
        &mut self,
        c: &Canvas,
        mx: i32,
        my: i32,
        escala: i32,
        estado: Estado,
        ticks: u64,
    ) {
        self.ocultar(c);
        let s = sprite_estado(estado);
        if s.ancho <= 0 || s.alto <= 0 {
            return;
        }
        let escala = escala.clamp(1, 8);
        let (dw, dh) = (s.ancho * escala, s.alto * escala);
        let (left, top) = (mx - s.hotspot_x * escala, my - s.hotspot_y * escala);
        let (x0, y0) = (left.max(0), top.max(0));
        let (x1, y1) = ((left + dw).min(c.width), (top + dh).min(c.height));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let wi = x1 - x0;
        let he = y1 - y0;
        self.fondo.clear();
        self.fondo.resize((wi * he) as usize, 0);
        let frame = &s.cuadros[indice_fotograma(estado, ticks)];
        for y in y0..y1 {
            for x in x0..x1 {
                let bajo = c.get(x, y);
                self.fondo[((y - y0) * wi + x - x0) as usize] = bajo;
                let (color, alfa) = if escala == 1 {
                    let p = frame[(y - top) as usize * s.ancho as usize + (x - left) as usize];
                    (p & 0x00FF_FFFF, p >> 24)
                } else {
                    crate::toby_ui::renderer::muestra(
                        frame,
                        s.ancho,
                        s.alto,
                        dw,
                        dh,
                        x - left,
                        y - top,
                    )
                };
                if alfa >= 255 {
                    c.put(x, y, color);
                } else if alfa > 0 {
                    c.blend(x, y, color, alfa);
                }
            }
        }
        self.x = x0;
        self.y = y0;
        self.w = wi;
        self.h = he;
        self.visible = true;
        c.present_rect(x0, y0, wi, he);
    }
}
