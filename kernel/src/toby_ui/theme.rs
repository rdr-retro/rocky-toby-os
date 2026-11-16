//! Tema visual de Toby UI (Fase 4).
//!
//! Separa la APARIENCIA de la estructura: el documento sólo dice `variant`,
//! el tema dice qué color/pieza le corresponde y en qué estado.
//!
//! La paleta reproduce el kit AQUA (azul de acento con variantes verde, rojo,
//! naranja, amarillo y magenta; blanco reservado a las variantes "white").
//! Hoy los componentes se dibujan con métricas y colores sólidos; las piezas
//! PNG del kit AQUA se cargan con `resource.rs` y se pintan con
//! `nine_slice()` cuando el tema las declara.

use super::layout::Rect;
use super::renderer::Renderer;

/// Estado visual de un componente interactivo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Estado {
    /// Reposo.
    Normal,
    /// El puntero está encima.
    Hover,
    /// Botón mantenido pulsado.
    Pressed,
    /// Enfocado por teclado.
    Focused,
    /// Marcado (checkbox, radio, segmento, ítem de lista).
    Selected,
    /// No interactivo.
    Disabled,
}

/// Métricas del tema (píxeles enteros).
#[derive(Clone, Copy, Debug)]
pub struct Metricas {
    /// Alto de un botón.
    pub alto_boton: u32,
    /// Ancho mínimo de un botón.
    pub ancho_boton_min: u32,
    /// Reserva horizontal de texto dentro del botón.
    pub margen_boton: u32,
    /// Alto de un campo de texto.
    pub alto_campo: u32,
    /// Lado de una casilla de verificación.
    pub lado_casilla: u32,
    /// Radio de la esfera de radio/slider circular.
    pub radio_esfera: u32,
    /// Ancho de un carácter de la fuente bitmap.
    pub ancho_caracter: u32,
    /// Alto de una línea de texto.
    pub alto_linea: u32,
    /// Separación entre el borde y el texto.
    pub padding_texto: u32,
}

/// Colores fijos del tema.
#[derive(Clone, Copy, Debug)]
pub struct Paleta {
    /// Texto principal.
    pub texto: u32,
    /// Texto sobre un acento saturado.
    pub texto_sobre_acento: u32,
    /// Fondo de la aplicación.
    pub fondo: u32,
    /// Fondo de un panel.
    pub panel: u32,
    /// Color de borde.
    pub borde: u32,
}

/// Variantes de color del kit AQUA.
#[derive(Clone, Copy, Debug)]
pub struct Acentos {
    /// Azul del kit AQUA (variante por defecto).
    pub azul: u32,
    pub verde: u32,
    pub rojo: u32,
    pub naranja: u32,
    pub amarillo: u32,
    pub magenta: u32,
    /// Blanco saturado ×0.06 del kit AQUA (botón blanco).
    pub blanco: u32,
    /// Gris de controles inactivos.
    pub gris: u32,
}

/// Un tema completo.
#[derive(Clone, Copy, Debug)]
pub struct Tema {
    /// Nombre del tema ("aqua").
    pub nombre: &'static str,
    pub metricas: Metricas,
    pub paleta: Paleta,
    pub acentos: Acentos,
    /// Sombra sólida (RGB) desplazada bajo los controles.
    pub sombra: (u8, u8, u8),
}

/// Tema por defecto de Toby UI, con la paleta del kit AQUA.
pub static TEMA_POR_DEFECTO: Tema = Tema {
    nombre: "aqua",
    metricas: Metricas {
        alto_boton: 24,
        ancho_boton_min: 72,
        margen_boton: 24,
        alto_campo: 24,
        lado_casilla: 18,
        radio_esfera: 9,
        ancho_caracter: 8,
        alto_linea: 8,
        padding_texto: 12,
    },
    paleta: Paleta {
        texto: 0x1F2328,
        texto_sobre_acento: 0xFFFFFF,
        fondo: 0xFFFFFF,
        panel: 0xF2F4F6,
        borde: 0xC9CED4,
    },
    acentos: Acentos {
        azul: 0x0E5CB8,
        verde: 0x2E9E4F,
        rojo: 0xD64545,
        naranja: 0xE8792B,
        amarillo: 0xE8B923,
        magenta: 0xC3489C,
        // Blanco saturado ×0.06 del kit AQUA (botón blanco).
        blanco: 0xD0D3D7,
        gris: 0x8E969E,
    },
    sombra: (48, 53, 60),
};

/// Color de acento de una variante (`variant` del documento).
///
/// Variante desconocida o vacía = azul del kit AQUA.
pub fn acento_de(t: &Tema, variante: &str) -> u32 {
    match variante {
        "green" => t.acentos.verde,
        "red" => t.acentos.rojo,
        "orange" => t.acentos.naranja,
        "yellow" => t.acentos.amarillo,
        "magenta" => t.acentos.magenta,
        "white" => t.acentos.blanco,
        "gray" | "grey" => t.acentos.gris,
        _ => t.acentos.azul,
    }
}

/// Color de texto del tema para una variante.
///
/// Sobre acentos oscuros el texto es blanco; sobre amarillo/naranja claro,
/// negro — igual que en el kit AQUA.
pub fn texto_de(t: &Tema, variante: &str) -> u32 {
    match variante {
        // Superficies claras: texto oscuro (igual que el kit AQUA).
        "yellow" | "white" => 0x000000,
        _ => t.paleta.texto_sobre_acento,
    }
}

/// Sombra sólida en formato 0x00RRGGBB.
pub fn color_sombra(t: &Tema) -> u32 {
    let (r, g, b) = t.sombra;
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// Mezcla un color hacia el blanco (`pct` de 0 a 100).
///
/// Sirve para el estado hover sin necesitar una imagen por estado.
pub fn aclarar(color: u32, pct: u32) -> u32 {
    let p = pct.min(100);
    componer(
        (color >> 16) & 0xFF,
        (color >> 8) & 0xFF,
        color & 0xFF,
        |c| c + ((255 - c) * p) / 100,
    )
}

/// Oscurece un color (`pct` de 0 a 100).
///
/// Sirve para el estado pulsado.
pub fn oscurecer(color: u32, pct: u32) -> u32 {
    let factor = 100 - pct.min(100);
    componer(
        (color >> 16) & 0xFF,
        (color >> 8) & 0xFF,
        color & 0xFF,
        |c| (c * factor) / 100,
    )
}

fn componer(r: u32, g: u32, b: u32, f: impl Fn(u32) -> u32) -> u32 {
    ((f(r) & 0xFF) << 16) | ((f(g) & 0xFF) << 8) | (f(b) & 0xFF)
}

/// Tres piezas horizontales de un nine-slice (cabecera, cuerpo, pie).
///
/// Las dos exteriores conservan su ancho y el cuerpo se estira. Es el
/// esquema 3-slice con el que funciona el kit AQUA (`left`, `body`, `right`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NineSlice {
    /// Ancho fijo de la pieza izquierda.
    pub izquierda: u32,
    /// Ancho fijo de la pieza derecha.
    pub derecha: u32,
    /// Color a aplicar al cuerpo (mismo tono que las piezas).
    pub cuerpo: u32,
    /// Color de las piezas laterales.
    pub laterales: u32,
}

impl NineSlice {
    /// Nine-slice plano de un solo color (sin piezas externas).
    pub const PLANO: NineSlice = NineSlice {
        izquierda: 0,
        derecha: 0,
        cuerpo: 0,
        laterales: 0,
    };
}

/// Dibuja un nine-slice dentro de `rect`, estirando sólo el cuerpo.
///
/// Si `rect` es más estrecho que las dos piezas juntas, se rellena entero con
/// el color del cuerpo (recorte sin distorsión).
pub fn nine_slice<R: Renderer>(r: &mut R, ns: &NineSlice, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let fijo = ns.izquierda.saturating_add(ns.derecha);
    if fijo == 0 || rect.w <= fijo {
        r.relleno(rect, ns.cuerpo);
        return;
    }
    let x_i = rect.x;
    let x_d = rect.der() - ns.derecha as i32;
    let x_c = rect.x + ns.izquierda as i32;
    let ancho_cuerpo = rect.w - fijo;
    // Laterales.
    r.relleno(
        Rect {
            x: x_i,
            y: rect.y,
            w: ns.izquierda,
            h: rect.h,
        },
        ns.laterales,
    );
    r.relleno(
        Rect {
            x: x_d,
            y: rect.y,
            w: ns.derecha,
            h: rect.h,
        },
        ns.laterales,
    );
    // Cuerpo estirado.
    r.relleno(
        Rect {
            x: x_c,
            y: rect.y,
            w: ancho_cuerpo,
            h: rect.h,
        },
        ns.cuerpo,
    );
}

/// Dibuja un botón AQUA ensamblando sus **tres piezas PNG** (3-slice).
///
/// Los dos extremos conservan su proporción nativa (258 × 409) y el cuerpo
/// (30 × 409) se estira para llenar lo que sobra, así el botón crece con el
/// texto sin deformarse. Si el botón es más estrecho que los dos extremos, se
/// dibuja el extremo izquierdo estirado a todo el ancho.
pub fn pintar_piel<R: Renderer>(r: &mut R, piel: &super::kit::Piel, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let cap = ((rect.h as u64 * 258 / 409) as u32).max(1);
    if rect.w <= cap.saturating_mul(2) {
        r.imagen(rect, &piel.izq);
        return;
    }
    let izq = Rect {
        x: rect.x,
        y: rect.y,
        w: cap,
        h: rect.h,
    };
    let cuerpo = Rect {
        x: rect.x + cap as i32,
        y: rect.y,
        w: rect.w - cap * 2,
        h: rect.h,
    };
    let der = Rect {
        x: rect.x + (rect.w - cap) as i32,
        y: rect.y,
        w: cap,
        h: rect.h,
    };
    r.imagen(izq, &piel.izq);
    r.imagen(cuerpo, &piel.cuerpo);
    r.imagen(der, &piel.der);
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn aclarar_y_oscurecer_por_canal() {
        // Blanco y negro son los extremos estables.
        assert_eq!(aclarar(0x000000, 100), 0xFFFFFF);
        assert_eq!(oscurecer(0xFFFFFF, 100), 0x000000);
        // Aclarar poco y oscurecer poco respetan el orden por canal.
        let azul = TEMA_POR_DEFECTO.acentos.azul;
        let claro = aclarar(azul, 50);
        let oscuro = oscurecer(azul, 50);
        assert!(claro > azul && azul > oscuro);
        // El canal rojo del azul AQUA (0x0E) sube al aclarar y baja al oscurecer.
        assert!(((claro >> 16) & 0xFF) > 0x0E);
        assert!(((oscuro >> 16) & 0xFF) < 0x0E);
    }

    #[test]
    fn variantes_de_acento() {
        let t = &TEMA_POR_DEFECTO;
        assert_eq!(acento_de(t, ""), t.acentos.azul);
        assert_eq!(acento_de(t, "blue"), t.acentos.azul);
        assert_eq!(acento_de(t, "green"), t.acentos.verde);
        assert_eq!(acento_de(t, "desconocida"), t.acentos.azul);
        // El amarillo usa texto negro, el resto blanco.
        assert_eq!(texto_de(t, "yellow"), 0x000000);
        assert_eq!(texto_de(t, "blue"), 0xFFFFFF);
    }

    #[test]
    fn nine_slice_estira_solo_el_cuerpo() {
        // Un renderer de prueba que sólo registra los rellenos.
        struct Falso {
            rellenos: alloc::vec::Vec<(Rect, u32)>,
        }
        impl Renderer for Falso {
            fn limpiar(&mut self, _color: u32) {}
            fn relleno(&mut self, rect: Rect, color: u32) {
                self.rellenos.push((rect, color));
            }
            fn borde(&mut self, _rect: Rect, _color: u32) {}
            fn texto(&mut self, _x: i32, _y: i32, _s: &str, _c: u32, _e: i32) {}
            fn recorte(&mut self, _rect: Rect) {}
            fn recorte_fin(&mut self) {}
        }
        let mut f = Falso {
            rellenos: alloc::vec::Vec::new(),
        };
        let ns = NineSlice {
            izquierda: 4,
            derecha: 4,
            cuerpo: 0x111111,
            laterales: 0x222222,
        };
        nine_slice(
            &mut f,
            &ns,
            Rect {
                x: 10,
                y: 20,
                w: 40,
                h: 10,
            },
        );
        assert_eq!(f.rellenos.len(), 3);
        assert_eq!(
            f.rellenos[0],
            (
                Rect {
                    x: 10,
                    y: 20,
                    w: 4,
                    h: 10
                },
                0x222222
            )
        );
        assert_eq!(
            f.rellenos[1],
            (
                Rect {
                    x: 46,
                    y: 20,
                    w: 4,
                    h: 10
                },
                0x222222
            )
        );
        assert_eq!(
            f.rellenos[2],
            (
                Rect {
                    x: 14,
                    y: 20,
                    w: 32,
                    h: 10
                },
                0x111111
            )
        );
    }

    #[test]
    fn nine_slice_estrecho_no_se_deforma() {
        struct Falso {
            rellenos: alloc::vec::Vec<u32>,
        }
        impl Renderer for Falso {
            fn limpiar(&mut self, _color: u32) {}
            fn relleno(&mut self, _rect: Rect, color: u32) {
                self.rellenos.push(color);
            }
            fn borde(&mut self, _rect: Rect, _color: u32) {}
            fn texto(&mut self, _x: i32, _y: i32, _s: &str, _c: u32, _e: i32) {}
            fn recorte(&mut self, _rect: Rect) {}
            fn recorte_fin(&mut self) {}
        }
        let mut f = Falso {
            rellenos: alloc::vec::Vec::new(),
        };
        let ns = NineSlice {
            izquierda: 4,
            derecha: 4,
            cuerpo: 0x111111,
            laterales: 0x222222,
        };
        // Más estrecho que 4+4: se rellena entero con el cuerpo.
        nine_slice(
            &mut f,
            &ns,
            Rect {
                x: 0,
                y: 0,
                w: 6,
                h: 8,
            },
        );
        assert_eq!(f.rellenos, alloc::vec![0x111111]);
    }
}
