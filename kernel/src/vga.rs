//! Consola de texto VGA en modo 3 (80x25, memoria en 0xB8000).

const VGA_BUFFER: *mut u8 = 0xB8000 as *mut u8;
const WIDTH: usize = 80;
const HEIGHT: usize = 25;
const ATTR: u8 = 0x07; // gris claro sobre negro

pub struct Vga {
    col: usize,
    row: usize,
}

impl Vga {
    pub const fn new() -> Self {
        Vga { col: 0, row: 0 }
    }

    fn cell_offset(row: usize, col: usize) -> usize {
        (row * WIDTH + col) * 2
    }

    pub fn clear(&mut self) {
        unsafe {
            for i in 0..(WIDTH * HEIGHT) {
                *VGA_BUFFER.add(i * 2) = b' ';
                *VGA_BUFFER.add(i * 2 + 1) = ATTR;
            }
        }
        self.col = 0;
        self.row = 0;
    }

    fn newline(&mut self) {
        self.col = 0;
        self.row += 1;
        if self.row >= HEIGHT {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        unsafe {
            for row in 1..HEIGHT {
                for col in 0..WIDTH {
                    let src = Self::cell_offset(row, col);
                    let dst = Self::cell_offset(row - 1, col);
                    *VGA_BUFFER.add(dst) = *VGA_BUFFER.add(src);
                    *VGA_BUFFER.add(dst + 1) = *VGA_BUFFER.add(src + 1);
                }
            }
            for col in 0..WIDTH {
                let off = Self::cell_offset(HEIGHT - 1, col);
                *VGA_BUFFER.add(off) = b' ';
                *VGA_BUFFER.add(off + 1) = ATTR;
            }
        }
        self.row = HEIGHT - 1;
    }

    pub fn put_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            0x08 => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = WIDTH - 1;
                }
                let off = Self::cell_offset(self.row, self.col);
                unsafe {
                    *VGA_BUFFER.add(off) = b' ';
                    *VGA_BUFFER.add(off + 1) = ATTR;
                }
            }
            b'\t' => {
                for _ in 0..4 {
                    self.put_byte(b' ');
                }
            }
            b => {
                if self.col >= WIDTH {
                    self.newline();
                }
                // El generador de caracteres de la VGA usa su propio juego
                // CP437, que no trae los acentos en mayuscula: los sustituye
                // por la letra base (la GUI, con la fuente copiada, si los
                // dibuja).
                let b = crate::bios::glifo_texto(b);
                let off = Self::cell_offset(self.row, self.col);
                unsafe {
                    *VGA_BUFFER.add(off) = b;
                    *VGA_BUFFER.add(off + 1) = ATTR;
                }
                self.col += 1;
            }
        }
    }

    /// Posicion del cursor de texto como `(fila, columna)`.
    pub fn posicion(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// Recoloca el cursor de texto en `(fila, columna)`.
    pub fn situar(&mut self, fila: usize, columna: usize) {
        self.row = fila.min(HEIGHT - 1);
        self.col = columna.min(WIDTH - 1);
    }

    /// Copia toda la memoria de texto (caracteres y atributos) a `dst`.
    pub fn volcar(&self, dst: &mut [u8; TAM_TEXTO]) {
        unsafe {
            for (i, b) in dst.iter_mut().enumerate() {
                *b = *VGA_BUFFER.add(i);
            }
        }
    }

    /// Restaura la memoria de texto desde `src`. No toca el cursor.
    pub fn cargar(&mut self, src: &[u8; TAM_TEXTO]) {
        unsafe {
            for (i, b) in src.iter().enumerate() {
                *VGA_BUFFER.add(i) = *b;
            }
        }
    }
}

/// Tamano en bytes de la memoria de texto (80x25 celdas de 2 bytes: caracter y
/// atributo).
pub const TAM_TEXTO: usize = WIDTH * HEIGHT * 2;
