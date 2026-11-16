//! Shim de `gfx` para el anfitrion de Windows.
//!
//! Toby UI en rocky pinta sobre el `gfx::Canvas` del framebuffer VESA. Aqui no
//! hay VESA: el lienzo es un bufer en RAM del proceso, que luego se vuelca a la
//! ventana con `StretchDIBits`.
//!
//! Se reproduce UNICAMENTE la parte del `gfx::Canvas` del kernel que usa el
//! motor del lenguaje (`put`, `get`, `blend`, `hline`, `vline`, `fill_rect`,
//! `present`, `width`, `height`) y `FONT_W`/`text_width`. Ni el motor ni el
//! lenguaje se tocan: los ficheros de `toby_ui` se incluyen tal cual.

use alloc::vec;
use alloc::vec::Vec;

/// Ancho de un caracter de la fuente bitmap (mismo valor que en el kernel).
pub const FONT_W: i32 = 8;

/// Lienzo 2D sobre un bufer de RAM en formato `0x00RRGGBB` por pixel.
///
/// En memoria (little-endian) cada pixel queda como `[B, G, R, 0]`, que es
/// justo lo que espera un DIB de 32 bits `BI_RGB`: se puede volcar a la ventana
/// sin convertir nada.
pub struct Canvas {
    /// Ancho en pixeles.
    pub width: i32,
    /// Alto en pixeles.
    pub height: i32,
    /// Paso de linea en pixeles.
    pub stride: usize,
    pixeles: Vec<u32>,
    // Se conserva como entero para que Canvas siga siendo Send dentro del
    // estado Win32 sincronizado; sólo el constructor unsafe lo acepta.
    prestado: Option<usize>,
}

impl Canvas {
    /// Nuevo lienzo de `width` x `height` pixeles, en negro transparente.
    pub fn nuevo(width: i32, height: i32) -> Canvas {
        let w = width.max(0) as usize;
        let h = height.max(0) as usize;
        Canvas {
            width,
            height,
            stride: w,
            pixeles: vec![0u32; w * h],
            prestado: None,
        }
    }

    /// Lienzo sobre un buffer del llamador. Su vida debe cubrir la del Canvas.
    ///
    /// # Safety
    /// `ptr` debe apuntar a `stride * height` píxeles mutables y seguir válido
    /// mientras exista el lienzo.
    pub unsafe fn from_buffer(ptr: *mut u32, width: i32, height: i32, stride: usize) -> Canvas {
        Canvas {
            width,
            height,
            stride,
            pixeles: Vec::new(),
            prestado: Some(ptr as usize),
        }
    }

    /// Los pixeles en crudo, para volcarlos a la ventana.
    pub fn pixeles(&self) -> &[u32] {
        if let Some(ptr) = self.prestado {
            let longitud = self.stride.saturating_mul(self.height.max(0) as usize);
            if ptr == 0 || longitud == 0 {
                return &[];
            }
            unsafe { core::slice::from_raw_parts(ptr as *const u32, longitud) }
        } else {
            &self.pixeles
        }
    }

    /// Direccion base del bufer (el `Vec` no se mueve de sitio en el heap).
    #[inline]
    fn base(&self) -> *mut u32 {
        self.prestado
            .map(|ptr| ptr as *mut u32)
            .unwrap_or_else(|| self.pixeles.as_ptr() as *mut u32)
    }

    /// Escribe un pixel (lo ignora si cae fuera del lienzo).
    #[inline]
    pub fn put(&self, x: i32, y: i32, color: u32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        unsafe {
            let p = self.base().add(y as usize * self.stride + x as usize);
            core::ptr::write_volatile(p, color);
        }
    }

    /// Lee un pixel (`0` si cae fuera).
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return 0;
        }
        unsafe { core::ptr::read_volatile(self.base().add(y as usize * self.stride + x as usize)) }
    }

    /// Mezcla un color con el pixel existente (`alpha` de 0 a 255).
    pub fn blend(&self, x: i32, y: i32, color: u32, alpha: u32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        unsafe {
            let p = self.base().add(y as usize * self.stride + x as usize);
            let dst = core::ptr::read_volatile(p);
            let r = (color >> 16) & 0xFF;
            let g = (color >> 8) & 0xFF;
            let b = color & 0xFF;
            let dr = (dst >> 16) & 0xFF;
            let dg = (dst >> 8) & 0xFF;
            let db = dst & 0xFF;
            let ia = 256 - alpha;
            let nr = (r * alpha + dr * ia) >> 8;
            let ng = (g * alpha + dg * ia) >> 8;
            let nb = (b * alpha + db * ia) >> 8;
            core::ptr::write_volatile(p, (nr << 16) | (ng << 8) | nb);
        }
    }

    /// Linea horizontal de `x0` a `x1` (incluidos) en la fila `y`.
    pub fn hline(&self, x0: i32, x1: i32, y: i32, color: u32) {
        if y < 0 || y >= self.height {
            return;
        }
        let a = x0.min(x1).max(0);
        let b = x0.max(x1).min(self.width - 1);
        let mut x = a;
        while x <= b {
            self.put(x, y, color);
            x += 1;
        }
    }

    /// Linea vertical de `y0` a `y1` (incluidos) en la columna `x`.
    pub fn vline(&self, x: i32, y0: i32, y1: i32, color: u32) {
        if x < 0 || x >= self.width {
            return;
        }
        let a = y0.min(y1).max(0);
        let b = y0.max(y1).min(self.height - 1);
        let mut y = a;
        while y <= b {
            self.put(x, y, color);
            y += 1;
        }
    }

    /// Relleno solido de un rectangulo.
    pub fn fill_rect(&self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        for yy in y..y + h {
            self.hline(x, x + w - 1, yy, color);
        }
    }

    /// En la ventana no hay framebuffer aparte: el lienzo ya es el destino.
    pub fn present(&self) {}
}

/// Ancho en pixeles de un texto con la fuente del tema.
/// Longitud en pixels que ocupara `s` al dibujarse con `scale`.
pub fn text_width(s: &str, scale: i32) -> i32 {
    s.chars().count() as i32 * FONT_W * scale.max(1)
}
