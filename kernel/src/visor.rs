//! Visor de imagenes a pantalla completa (la orden `imagen`).
//!
//! Flujo: lee el PNG del disco, lo decodifica con `png.rs`, activa un modo VESA
//! con framebuffer lineal y lo pinta escalado para que quepa en pantalla
//! (conservando la proporcion). ESC vuelve al modo texto de 80x25.
//!
//! El escalado se **filtra** con el mismo muestreo del motor de Toby UI
//! (`toby_ui::renderer::muestra`): al reducir promedia el area que le toca a
//! cada pixel de pantalla, asi una foto no sale dentada. El alfa del PNG se
//! mezcla sobre el fondo negro, que ya esta pintado.
//!
//! Se apoya en el mismo camino que ya usa `demo.rs` para cambiar de modo de
//! video (`gfx::set_mode` + `bios::set_text_mode`), asi que solo funciona desde
//! el interprete en modo texto, no dentro del escritorio grafico.
//!
//! Codigo propio de rocky.

use alloc::string::{String, ToString};
use alloc::vec;

use crate::bios;
use crate::console;
use crate::fs::{self, Tipo};
use crate::gfx::{self, Canvas};
use crate::input;
use crate::println;

/// Decodifica y muestra el PNG indicado. Devuelve un mensaje de error legible
/// si no se puede (para imprimirlo en la consola).
pub fn ver(ruta: &str) -> Result<(), String> {
    if !fs::montado() {
        return Err("no hay ningun disco montado".to_string());
    }
    let reg = fs::resolver(fs::raiz(), ruta).map_err(|e| e.mensaje().to_string())?;
    if fs::tipo(reg) != Tipo::Fichero {
        return Err("esa ruta no es un fichero".to_string());
    }
    let tam = fs::tamano(reg) as usize;
    if tam == 0 {
        return Err("el fichero esta vacio".to_string());
    }

    let mut datos = vec![0u8; tam];
    let n = fs::leer(reg, 0, &mut datos);
    datos.truncate(n);

    let img = png::decodificar(&datos)
        .map_err(|_| "no es un PNG valido (o usa una variante no soportada)".to_string())?;

    mostrar(&img);
    Ok(())
}

/// Pinta la imagen a pantalla completa y espera a que se pulse ESC.
fn mostrar(img: &png::Imagen) {
    // Si Kira tenía el vídeo (escritorio), lo cede antes de poner el modo VESA.
    crate::kira::comp::ceder();

    if img.ancho == 0 || img.alto == 0 {
        println!("imagen: la imagen no tiene tamano.");
        crate::kira::comp::recuperar();
        return;
    }
    let (modo, mi) = match gfx::find_first_demo_mode() {
        Some(v) => v,
        None => {
            println!("imagen: este equipo no ofrece ningun modo VESA de 32 bpp.");
            crate::kira::comp::recuperar();
            return;
        }
    };
    if gfx::set_mode(modo).is_none() {
        println!("imagen: no se pudo activar el modo grafico.");
        crate::kira::comp::recuperar();
        return;
    }

    let pant_w = mi.width as i64;
    let pant_h = mi.height as i64;
    let img_w = img.ancho as i64;
    let img_h = img.alto as i64;

    let c = Canvas::new(&mi);
    c.fill_rect(0, 0, mi.width as i32, mi.height as i32, 0);

    // Escala para que quepa entera, conservando la proporcion; se centra.
    let mut dest_w = img_w * pant_h / img_h;
    if dest_w > pant_w {
        dest_w = pant_w;
    }
    let dest_h = img_h * dest_w / img_w;
    let fondo_x = (pant_w - dest_w) / 2;
    let fondo_y = (pant_h - dest_h) / 2;

    // Escalado filtrado: el mismo muestreo que usa el motor de Toby UI, que
    // promedia el area que le toca a cada pixel de pantalla en vez de coger el
    // mas cercano (con vecino mas proximo una foto sale dentada).
    for y in 0..dest_h {
        for x in 0..dest_w {
            let (color, a) = crate::toby_ui::renderer::muestra(
                &img.pixeles,
                img.ancho as i32,
                img.alto as i32,
                dest_w as i32,
                dest_h as i32,
                x as i32,
                y as i32,
            );
            let (px, py) = ((fondo_x + x) as i32, (fondo_y + y) as i32);
            if a >= 255 {
                c.put(px, py, color);
            } else if a > 0 {
                c.blend(px, py, color, a);
            }
        }
    }

    // Espera a ESC para volver.
    loop {
        if let Some(k) = input::poll() {
            if k == 0x1B {
                break;
            }
        }
        core::hint::spin_loop();
    }

    bios::set_text_mode();
    console::clear();
    // Kira recupera la pantalla (escritorio y cursor) si su servicio sigue
    // activo.
    crate::kira::comp::recuperar();
}
