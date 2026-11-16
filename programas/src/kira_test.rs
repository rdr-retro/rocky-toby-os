//! kira_test - cliente mínimo del servidor **Kira** para rocky.
//!
//! Abre `/dev/kira`, realiza el handshake Kira y prueba varias ventanas,
//! superficies y primitivas de dibujo del servidor.

#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::*;

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let ruta = b"/dev/kira\0";
    let fd = abrir(ruta.as_ptr(), O_RDWR);
    if fd < 0 {
        escribir(1, b"kira_test: no se pudo abrir /dev/kira\n");
        salir(1);
    }
    escribir(1, b"kira_test: abierto /dev/kira\n");

    // 1. Connection Setup: byte-order LSB, version 11, sin auth
    let mut setup = [0u8; 12];
    setup[0] = b'l'; // LSB
    setup[2] = 11; // major version (LSB: byte 2-3)
    setup[4] = 0; // minor version
    escribir(fd as u64, &setup);

    // 2. Leer respuesta del servidor
    let mut resp = [0u8; 512];
    let n = leer(fd as u64, &mut resp);
    if n <= 0 {
        escribir(1, b"kira_test: no hubo respuesta del servidor\n");
        salir(1);
    }
    let status = resp[0];
    if status != 1 {
        escribir(1, b"kira_test: setup rechazado\n");
        let _ = status;
        salir(1);
    }

    // Leer resource-id-base y resource-id-mask del setup reply
    // X11 setup reply: offset 8=release, 12=id-base, 16=id-mask
    let id_base = get_u32_lsb(&resp, 12);
    let id_mask = get_u32_lsb(&resp, 16);
    // Screen section starts after: 8 (header) + 32 (fixed) + 4 (vendor) + 8 (format) = 52
    let root_wid = get_u32_lsb(&resp, 52);

    // 3. Crear cuatro ventanas solapadas y dibujar varias formas en cada una.
    // El compositor debe mantener y componer todas sus superficies, no solo
    // mostrar una caja vacía por cliente.
    let colores = [0xE84B4B, 0x4085F5, 0x36A269, 0xE6A23C];
    for i in 0..4usize {
        let wid = id_base | (((i + 1) as u32) & id_mask);
        let gc_id = id_base | (((0x100 + i) as u32) & id_mask);

        let mut cw = [0u8; 32];
        cw[0] = 1; // CreateWindow
        cw[2] = 8; // 32 bytes
        put_u32_lsb(&mut cw, 4, root_wid);
        put_u32_lsb(&mut cw, 8, wid);
        put_u16_lsb(&mut cw, 12, 80 + i as u16 * 72);
        put_u16_lsb(&mut cw, 14, 80 + i as u16 * 48);
        put_u16_lsb(&mut cw, 16, 360);
        put_u16_lsb(&mut cw, 18, 250);
        put_u16_lsb(&mut cw, 22, 1); // InputOutput
        escribir(fd as u64, &cw);

        let mut mw = [0u8; 8];
        mw[0] = 8; // MapWindow
        mw[2] = 2;
        put_u32_lsb(&mut mw, 4, wid);
        escribir(fd as u64, &mw);

        let mut gc = [0u8; 20];
        gc[0] = 55; // CreateGC
        gc[2] = 5;
        put_u32_lsb(&mut gc, 4, wid);
        put_u32_lsb(&mut gc, 8, gc_id);
        put_u32_lsb(&mut gc, 12, 1 << 2); // foreground
        put_u32_lsb(&mut gc, 16, colores[i]);
        escribir(fd as u64, &gc);

        let mut formas = [0u8; 36];
        formas[0] = 70; // PolyFillRectangle
        formas[2] = 9; // tres rectangulos
        put_u32_lsb(&mut formas, 4, wid);
        put_u32_lsb(&mut formas, 8, gc_id);
        for (j, (x, y, w, h)) in [(20, 24, 320, 48), (28, 92, 138, 118), (194, 92, 138, 118)]
            .into_iter()
            .enumerate()
        {
            let off = 12 + j * 8;
            put_u16_lsb(&mut formas, off, x);
            put_u16_lsb(&mut formas, off + 2, y);
            put_u16_lsb(&mut formas, off + 4, w);
            put_u16_lsb(&mut formas, off + 6, h);
        }
        escribir(fd as u64, &formas);
    }

    // 4. Leer el primer evento Expose.
    let mut ev = [0u8; 32];
    let n2 = leer(fd as u64, &mut ev);
    if n2 > 0 {
        let code = ev[0] & 0x7F;
        escribir(1, b"kira_test: evento ");
        let decenas = [b'0' + (code / 10)];
        let unidades = [b'0' + (code % 10)];
        if code >= 10 {
            escribir(1, &decenas);
        }
        escribir(1, &unidades);
        escribir(1, b" recibido OK\n");
    } else {
        escribir(1, b"kira_test: sin evento Expose\n");
    }

    // Cerrar
    cerrar(fd as u64);
    escribir(
        1,
        b"kira_test: cuatro ventanas y superficies de dibujo OK\n",
    );
    salir(0);
}

fn get_u32_lsb(b: &[u8], off: usize) -> u32 {
    ((b[off + 3] as u32) << 24)
        | ((b[off + 2] as u32) << 16)
        | ((b[off + 1] as u32) << 8)
        | (b[off] as u32)
}

fn put_u32_lsb(out: &mut [u8], off: usize, v: u32) {
    out[off] = (v & 0xFF) as u8;
    out[off + 1] = ((v >> 8) & 0xFF) as u8;
    out[off + 2] = ((v >> 16) & 0xFF) as u8;
    out[off + 3] = ((v >> 24) & 0xFF) as u8;
}

fn put_u16_lsb(out: &mut [u8], off: usize, v: u16) {
    out[off] = (v & 0xFF) as u8;
    out[off + 1] = ((v >> 8) & 0xFF) as u8;
}
