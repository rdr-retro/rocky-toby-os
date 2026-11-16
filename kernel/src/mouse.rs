//! Raton PS/2 colgado del puerto auxiliar del controlador 8042.
//!
//! rocky acumula paquetes recibidos por IRQ12; [`poll`] queda como respaldo si
//! el firmware no enruta la interrupción. El movimiento se acumula en `dx`/`dy` (en el sistema de
//! pantalla, con el eje Y creciendo hacia abajo) y el estado de los botones en
//! `botones` (bit 0 = izquierdo, bit 1 = derecho, bit 2 = central).
//!
//! Aqui solo vive el montaje de los paquetes de 3 bytes; el controlador en si
//! (habilitar el puerto auxiliar, activar el envio de datos...) se prepara en
//! [`crate::ps2`], junto con el teclado.

use crate::cpu::inb;
use crate::sync::SyncCell;

/// Estado interno del raton.
struct Estado {
    dx: i32,
    dy: i32,
    /// Botones pulsados en conjunto: la union de las dos fuentes.
    botones: u8,
    /// Lo que tiene pulsado el PS/2.
    botones_ps2: u8,
    /// Lo que tiene pulsado el USB (el ultimo informe boot).
    botones_usb: u8,
    paquete: [u8; 3],
    indice: usize,
}

impl Estado {
    const fn new() -> Self {
        Estado {
            dx: 0,
            dy: 0,
            botones: 0,
            botones_ps2: 0,
            botones_usb: 0,
            paquete: [0; 3],
            indice: 0,
        }
    }

    /// Recompone los botones: los de un dispositivo no pisan a los del otro.
    fn unir_botones(&mut self) {
        self.botones = (self.botones_ps2 | self.botones_usb) & 0x07;
    }
}

static ESTADO: SyncCell<Estado> = SyncCell::new(Estado::new());

/// Presupuesto por llamada al sondeo. Una controladora que mantenga OBF activo
/// no puede monopolizar para siempre la tarea de Kira: el resto de la vuelta
/// debe poder actualizar el cursor y ceder el procesador.
const MAX_BYTES_POR_SONDEO: usize = 64;

fn estado() -> &'static mut Estado {
    // Seguro: rocky es mononucleo y monotarea; no hay dos referencias vivas a
    // la vez porque cada funcion es de corta duracion.
    unsafe { &mut *ESTADO.get() }
}

/// Introduce un byte del raton. Se protege frente a IRQ12 al ser compartido.
pub fn alimentar(byte: u8) {
    unsafe { crate::cpu::sin_interrupciones(|| alimentar_interno(byte)) }
}

/// Introduce un informe de un raton USB boot.
///
/// `dx` y `dy` ya vienen en el sistema de pantalla: el protocolo boot cuenta
/// el eje Y **hacia abajo**, asi que se suma tal cual (el PS/2 lo resta). Los
/// botones se recortan a los tres que entiende el cursor. La suma satura para
/// que un dispositivo que mande desplazamientos absurdos no desborde el
/// acumulador entre dos fotogramas.
pub fn alimentar_usb(dx: i32, dy: i32, botones: u8) {
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            let st = estado();
            st.dx = st.dx.saturating_add(dx);
            st.dy = st.dy.saturating_add(dy);
            st.botones_usb = botones & 0x07;
            st.unir_botones();
        });
    }
}

/// Suelta los botones que tuviera pulsados el raton USB.
///
/// Se llama al desconectarse o al pararse su endpoint: si se va con un boton
/// abajo, el cursor se queda arrastrando para siempre. Los del PS/2 no se
/// tocan.
pub fn soltar_usb() {
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            let st = estado();
            st.botones_usb = 0;
            st.unir_botones();
        });
    }
}

fn alimentar_interno(byte: u8) {
    let st = estado();
    match st.indice {
        0 => {
            // El primer byte del paquete siempre lleva el bit 3 a 1.
            if byte & 0x08 != 0 {
                st.paquete[0] = byte;
                st.indice = 1;
            }
        }
        1 => {
            st.paquete[1] = byte;
            st.indice = 2;
        }
        _ => {
            st.paquete[2] = byte;
            st.indice = 0;
            aplicar(st);
        }
    }
}

fn aplicar(st: &mut Estado) {
    let b0 = st.paquete[0];
    if b0 & 0xC0 != 0 {
        return; // desbordamiento de movimiento: se descarta el paquete
    }
    let mut dx = st.paquete[1] as u16;
    if b0 & 0x10 != 0 {
        dx |= 0xFF00; // extension de signo
    }
    let mut dy = st.paquete[2] as u16;
    if b0 & 0x20 != 0 {
        dy |= 0xFF00;
    }
    st.dx += dx as i16 as i32;
    st.dy -= dy as i16 as i32; // el eje Y del raton crece hacia arriba
    st.botones_ps2 = b0 & 0x07;
    st.unir_botones();
}

/// Vacia los bytes del raton pendientes en el controlador (sin tocar el teclado).
pub fn poll() {
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            for _ in 0..MAX_BYTES_POR_SONDEO {
                let st = inb(0x64);
                if st & 0x01 == 0 {
                    break;
                }
                if st & 0x20 == 0 {
                    break; // es del teclado: lo deja para `input`
                }
                alimentar_interno(inb(0x60));
            }
        });
    }
}

/// Devuelve el movimiento acumulado desde la ultima llamada y los botones.
pub fn tomar() -> (i32, i32, u8) {
    unsafe {
        crate::cpu::sin_interrupciones(|| {
            let st = estado();
            let r = (st.dx, st.dy, st.botones);
            st.dx = 0;
            st.dy = 0;
            r
        })
    }
}
