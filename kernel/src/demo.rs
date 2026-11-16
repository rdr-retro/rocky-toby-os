//! Demo grafica 2D de rocky.
//!
//! Activa un modo VESA VBE con framebuffer lineal y dibuja una pequena
//! animacion por software: fondo con degradado, campo de estrellas, un planeta
//! con su luna, un poligono girando, varias esferas que rebotan y un espectro
//! animado, con un rotulo de texto tomado de la fuente ROM de la VGA. Al salir
//! vuelve al modo texto.
//!
//! Todo el dibujo se hace pixel a pixel sobre la memoria del framebuffer; no
//! hay aceleracion ni GPU, es la misma idea que usaba el software de la epoca.

use crate::bios::{self, Font};
use crate::console;
use crate::gfx::{self, Canvas, ModeInfo};
use crate::input;
use crate::pit;
use crate::println;
use crate::sound;

const PI: f32 = 3.14159265;
const NUM_STARS: usize = 140;
const NUM_BALLS: usize = 5;

/// Seno aproximado por serie de Taylor (sin depender de `libm`).
fn sinf(x: f32) -> f32 {
    let two_pi = 2.0 * PI;
    let mut x = x % two_pi;
    if x > PI {
        x -= two_pi;
    } else if x < -PI {
        x += two_pi;
    }
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0)))
}

fn cosf(x: f32) -> f32 {
    sinf(x + PI / 2.0)
}

#[derive(Clone, Copy)]
struct Star {
    x: i32,
    y: i32,
    speed: i32,
    bright: u32,
}

#[derive(Clone, Copy)]
struct Ball {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    r: i32,
    color: u32,
}
/// Color del fondo en la fila `y` (degradado azul profundo -> negro).
fn bg_color(y: i32, h: i32, frame: u32) -> u32 {
    let f = (y * 255) / h.max(1);
    let pulse = ((frame / 5) % 64) as i32;
    let r = (16 - f / 20).clamp(0, 255) as u8;
    let g = (18 - f / 24).clamp(0, 255) as u8;
    let b = (78 + pulse / 2 - f / 3).clamp(0, 255) as u8;
    gfx::rgb(r, g, b)
}

fn init_stars(w: i32, h: i32) -> [Star; NUM_STARS] {
    let mut stars = [Star {
        x: 0,
        y: 0,
        speed: 0,
        bright: 0,
    }; NUM_STARS];
    let mut i = 0;
    // Reparto pseudoaleatorio determinista (sin generador aleatorio).
    let mut seed: u32 = 0x1234_5678;
    let mut next = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as i32
    };
    while i < NUM_STARS {
        let speed = 1 + (next() % 4);
        let shade = 120 + (speed * 40).min(135);
        stars[i] = Star {
            x: next() % w.max(1),
            y: next() % h.max(1),
            speed,
            bright: gfx::rgb(shade as u8, shade as u8, 255),
        };
        i += 1;
    }
    stars
}

fn init_balls(w: i32, h: i32) -> [Ball; NUM_BALLS] {
    let colors = [
        gfx::rgb(230, 70, 70),
        gfx::rgb(70, 200, 90),
        gfx::rgb(80, 140, 255),
        gfx::rgb(240, 200, 60),
        gfx::rgb(220, 90, 220),
    ];
    let mut balls = [Ball {
        x: 0.0,
        y: 0.0,
        vx: 0.0,
        vy: 0.0,
        r: 0,
        color: 0,
    }; NUM_BALLS];
    let base = (w.min(h) / 12).max(6);
    let mut i = 0;
    while i < NUM_BALLS {
        let r = base + (i as i32 * 2);
        balls[i] = Ball {
            x: (w as f32) * (0.15 + 0.18 * i as f32),
            y: (h as f32) * 0.5,
            vx: 1.6 + 0.5 * i as f32,
            vy: 1.2 + 0.4 * i as f32,
            r,
            color: colors[i],
        };
        i += 1;
    }
    balls
}

/// Dibuja un fotograma completo.
fn draw_frame(
    c: &Canvas,
    mi: &ModeInfo,
    font: Option<&Font>,
    frame: u32,
    stars: &mut [Star; NUM_STARS],
    balls: &mut [Ball; NUM_BALLS],
) {
    let w = c.width;
    let h = c.height;

    // 1) Fondo con degradado vertical.
    for y in 0..h {
        c.hline(0, w - 1, y, bg_color(y, h, frame));
    }

    // 2) Campo de estrellas en movimiento.
    for s in stars.iter_mut() {
        s.x += s.speed;
        s.y += (s.speed * 2) / 3 + 1;
        if s.x >= w {
            s.x -= w;
        }
        if s.y >= h {
            s.y -= h;
        }
        let size = if s.speed >= 3 { 2 } else { 1 };
        c.fill_rect(s.x, s.y, size, size, s.bright);
    }

    // 3) Planeta con anillo (esfera sombreada + circulo).
    let pr = (h / 9).max(9);
    let px = w - pr * 3;
    let py = pr * 2 + 12;
    c.sphere(px, py, pr, gfx::rgb(120, 150, 205));
    c.circle(px, py, pr + 5, gfx::rgb(95, 115, 160));

    // Luna orbitando el planeta (disco solido).
    let moon_ang = frame as f32 * 0.05;
    let mr = (pr / 3).max(3);
    let mx = px + ((pr * 2) as f32 * cosf(moon_ang)) as i32;
    let my = py + ((pr as f32) * 0.55 * sinf(moon_ang)) as i32;
    c.disc(mx, my, mr, gfx::rgb(215, 212, 195));

    // 4) Poligono girando en el centro.
    let cx = w / 2;
    let cy = h / 2;
    let radius = (w.min(h) / 3).max(24);
    let sides = 6;
    let ang = frame as f32 * 0.02;
    let color = gfx::rgb(0, 220, 220);
    let mut prev: Option<(i32, i32)> = None;
    let mut first: Option<(i32, i32)> = None;
    for i in 0..sides {
        let a = ang + (i as f32) * (2.0 * PI / sides as f32);
        let px = cx + (radius as f32 * cosf(a)) as i32;
        let py = cy + (radius as f32 * sinf(a)) as i32;
        if let Some((qx, qy)) = prev {
            c.line(qx, qy, px, py, color);
        } else {
            first = Some((px, py));
        }
        prev = Some((px, py));
    }
    if let (Some((qx, qy)), Some((fx, fy))) = (prev, first) {
        c.line(qx, qy, fx, fy, color);
    }

    // 5) Esferas rebotando.
    for b in balls.iter_mut() {
        b.x += b.vx;
        b.y += b.vy;
        let r = b.r as f32;
        if b.x - r < 0.0 {
            b.x = r;
            b.vx = -b.vx;
        } else if b.x + r > w as f32 {
            b.x = w as f32 - r;
            b.vx = -b.vx;
        }
        if b.y - r < 0.0 {
            b.y = r;
            b.vy = -b.vy;
        } else if b.y + r > h as f32 {
            b.y = h as f32 - r;
            b.vy = -b.vy;
        }
        c.sphere(b.x as i32, b.y as i32, b.r, b.color);
    }

    // 6) Espectro animado en la franja inferior (usa `vline`).
    let bars = (w / 24).clamp(8, 40) as i32;
    let bw = (w / bars).max(3);
    let eq_max = (h / 6).max(8);
    let eq_top = h - eq_max - 4;
    for i in 0..bars {
        let phase = frame as f32 * 0.11 + i as f32 * 0.7;
        let amp = ((sinf(phase) * 0.5 + 0.5) * eq_max as f32) as i32 + 3;
        let bx = i * bw + 1;
        // Color tipo arcoiris segun la posicion de la barra.
        let hue = (i * 255 / bars.max(1)) as i32;
        let col = gfx::rgb(
            (60 + (hue - 128).abs()).clamp(0, 255) as u8,
            (140 + (hue / 2 - 64).abs()).clamp(0, 255) as u8,
            (200 - (hue / 3)).clamp(0, 255) as u8,
        );
        for x in bx..(bx + bw - 2).min(w) {
            c.vline(x, h - 2 - amp, h - 2, col);
        }
    }

    // 7) Rotulos (con panel translucido detras del titulo).
    if let Some(f) = font {
        let scale = ((w / 320).clamp(1, 3)) as i32;
        let title = "rocky";
        let tpad = 10 * scale;
        let mut buf = [0u8; 64];
        let line: &str = fmt_res(&mut buf, mi);

        let panel_w =
            (gfx::text_width(title, scale * 2).max(gfx::text_width(line, scale)) + tpad * 2).min(w);
        let panel_x = (w - panel_w) / 2;
        let panel_h = f.rows as i32 * scale * 2 + 6 + f.rows as i32 * scale + tpad;
        for yy in 6..(6 + panel_h).min(h) {
            for xx in panel_x..(panel_x + panel_w).min(w) {
                c.blend(xx, yy, 0x00_00_20, 110);
            }
        }

        let tx = (w - gfx::text_width(title, scale * 2)) / 2;
        gfx::draw_text_shadow(c, f, tx, 12, title, gfx::rgb(255, 235, 120), scale * 2);
        let lx = (w - gfx::text_width(line, scale)) / 2;
        gfx::draw_text_shadow(
            c,
            f,
            lx,
            12 + f.rows as i32 * scale * 2 + 6,
            line,
            0x00FF_FFFF,
            scale,
        );

        let hint = "Pulsa una tecla para volver al prompt";
        let hx = (w - gfx::text_width(hint, 1)) / 2;
        let hy = (eq_top - f.rows as i32 - 6).max(0);
        gfx::draw_text_shadow(c, f, hx, hy, hint, gfx::rgb(200, 200, 200), 1);
    }
}

/// Formatea "1024x768 32bpp  modo VESA VBE" en un buffer fijo.
fn fmt_res<'a>(buf: &'a mut [u8; 64], mi: &ModeInfo) -> &'a str {
    let mut n = 0usize;
    let push = |buf: &mut [u8; 64], n: &mut usize, v: u32| {
        let mut tmp = [0u8; 10];
        let mut len = 0;
        let mut v = v;
        if v == 0 {
            tmp[0] = b'0';
            len = 1;
        }
        while v > 0 && len < 10 {
            tmp[len] = b'0' + (v % 10) as u8;
            v /= 10;
            len += 1;
        }
        while len > 0 && *n < buf.len() {
            len -= 1;
            buf[*n] = tmp[len];
            *n += 1;
        }
    };
    let push_str = |buf: &mut [u8; 64], n: &mut usize, s: &[u8]| {
        for &ch in s {
            if *n < buf.len() {
                buf[*n] = ch;
                *n += 1;
            }
        }
    };
    push(buf, &mut n, mi.width);
    push_str(buf, &mut n, b"x");
    push(buf, &mut n, mi.height);
    push_str(buf, &mut n, b"x");
    push(buf, &mut n, mi.bpp);
    push_str(buf, &mut n, b"bpp  modo VESA VBE");
    core::str::from_utf8(&buf[..n]).unwrap_or("modo VESA")
}

/// Ejecuta la demo. `request` pide una resolucion concreta (`None` = automatica).
pub fn run(request: Option<(u32, u32)>) {
    // Si Kira tenía el vídeo (escritorio), lo cede antes de poner el modo VESA.
    crate::kira::comp::ceder();

    let chosen = match request {
        Some((w, h)) => gfx::find_mode(w, h).or_else(gfx::find_first_demo_mode),
        None => gfx::find_first_demo_mode(),
    };

    let (mode_num, mi) = match chosen {
        Some(v) => v,
        None => {
            println!(
                "DEMO: este equipo no ofrece ningun modo VESA de 32 bpp con framebuffer lineal."
            );
            crate::kira::comp::recuperar();
            return;
        }
    };

    match gfx::set_mode(mode_num) {
        Some(_) => {}
        None => {
            println!("DEMO: no se pudo activar el modo VESA (error del BIOS).");
            crate::kira::comp::recuperar();
            return;
        }
    }

    // El buffer trasero sale del heap del kernel; debe vivir todo el `run`.
    let pixels = gfx::buffer_pixels(&mi);
    let mut back: alloc::vec::Vec<u32> = alloc::vec![0u32; pixels];
    let canvas = Canvas::new_buffered(&mi, back.as_mut_ptr());
    let font = bios::get_font();

    println!(
        "DEMO: modo 0x{:04X} {}x{}x{} bpp, LFB 0x{:08X} (fuente {}, doble buffer {})",
        mode_num,
        mi.width,
        mi.height,
        mi.bpp,
        mi.lfb,
        if font.is_some() { "BIOS" } else { "no" },
        if canvas.is_buffered() { "si" } else { "no" }
    );

    let mut stars = init_stars(mi.width as i32, mi.height as i32);
    let mut balls = init_balls(mi.width as i32, mi.height as i32);

    // Pequena melodia de bienvenida (mientras se pinta el primer fotograma).
    sound::play(&[(523, 90), (659, 90), (784, 90), (1047, 140)]);

    let mut frame = 0u32;
    let mut exit_key = None;
    let start = pit::ticks();
    loop {
        draw_frame(&canvas, &mi, font.as_ref(), frame, &mut stars, &mut balls);
        canvas.present();
        frame += 1;

        // Salir tras ~8 segundos, o antes si se pulsa una tecla.
        if pit::ticks().saturating_sub(start) >= 800 {
            break;
        }
        if frame > 25 {
            if let Some(key) = input::poll() {
                exit_key = Some(key);
                break;
            }
        }
    }

    // De vuelta al modo texto clasico de 80x25.
    bios::set_text_mode();
    console::clear();
    // Kira recupera la pantalla (escritorio y cursor) si su servicio sigue
    // activo; el mensaje de abajo ya se dibuja sobre su consola grafica.
    crate::kira::comp::recuperar();
    println!();
    match exit_key {
        Some(k) => println!(
            "DEMO terminada: {} fotogramas en modo {}x{} (tecla 0x{:02X}).",
            frame, mi.width, mi.height, k
        ),
        None => println!(
            "DEMO terminada: {} fotogramas en modo {}x{}.",
            frame, mi.width, mi.height
        ),
    }
    println!();
}
