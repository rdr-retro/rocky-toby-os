//! GPU AMD/ATI: ventana de registros, volcado y cursor por hardware (fase 14).
//!
//! La fase 13 dejo identificada la GPU y sus BAR. Este es el controlador de
//! AMD: localiza la **ventana de registros** (en las Radeon desde Bonaire, la
//! familia de Polaris, es el BAR 5, de 256 KiB), la marca como no cacheable y
//! ofrece lo que el sistema sabe hacer con la tarjeta:
//!
//! * la orden `gpu`: identifica el modelo, mide los BAR y ensena unos cuantos
//!   registros del bloque de pantalla (fase 14, primer paso, solo lectura);
//! * la orden `cursor`: pone un cursor por hardware de 64x64 (fase 14b). El
//!   bitmap se escribe en la VRAM a traves del BAR0 y se programa el bloque
//!   `CUR_*` del DCE, tal y como lo hace `dce_v11_0.c`.
//!
//! Los nombres de registro y sus campos estan copiados de las cabeceras de
//! `amdgpu` (`dce_11_0_d.h` y `dce_11_0_sh_mask.h`), no inventados. Dos
//! detalles valen para todo el bloque:
//!
//! * los numeros de registro de esas cabeceras van en **palabras de 32 bits**,
//!   no en bytes: el desplazamiento real es `reg * 4` (es lo que hace
//!   `amdgpu_device_rreg`, que termina en `readl(rmmio + reg * 4)`);
//! * el bootloader mapea los primeros 4 GiB por identidad, asi que tanto el BAR
//!   de registros como la abertura de VRAM (mientras esten por debajo de 4 GiB)
//!   se alcanzan escribiendo en su direccion fisica.
//!
//! Codigo propio de rocky.

use crate::pci::{self, Bar, Funcion};
use crate::sync::SyncCell;

use super::atom;

/// Identificador de fabricante de AMD/ATI.
pub const FABRICANTE: u16 = 0x1002;

// --- Bloque de pantalla (DCE 11.2, la familia de Polaris) ---
// Numeros de registro en palabras de 32 bits (`mm...` de `dce_11_0_d.h`).
// (registros reusados abajo con definiciones ampliadas)
const CUR_CONTROL: u32 = 0x1a66;
const CUR_SURFACE_ADDRESS: u32 = 0x1a67;
const CUR_SIZE: u32 = 0x1a68;
const CUR_SURFACE_ADDRESS_HIGH: u32 = 0x1a69;
const CUR_POSITION: u32 = 0x1a6a;
const CUR_HOT_SPOT: u32 = 0x1a6b;
const CUR_UPDATE: u32 = 0x1a6e;
const CRTC_H_TOTAL: u32 = 0x1b80;
const CRTC_H_BLANK_START_END: u32 = 0x1b81;
const CRTC_H_SYNC_A: u32 = 0x1b82;
const CRTC_V_TOTAL: u32 = 0x1b87;
const CRTC_V_TOTAL_MIN: u32 = 0x1b88;
const CRTC_V_TOTAL_MAX: u32 = 0x1b89;
const CRTC_V_BLANK_START_END: u32 = 0x1b8d;
const CRTC_V_SYNC_A: u32 = 0x1b8e;
const CRTC_CONTROL: u32 = 0x1b9c;
const CRTC_STATUS_FRAME_COUNT: u32 = 0x1ba6;
/// `DCCG`: de donde saca el CRTC su reloj de pixeles (`CRTC0_PIXEL_RATE_CNTL`).
const CRTC0_PIXEL_RATE_CNTL: u32 = 0x140;
/// `CRTC0_PIXEL_RATE_CNTL.DP_DTO0_ENABLE`: el reloj lo pone el DTO de DP.
const DP_DTO_ENABLE: u32 = 1 << 4;
/// Primer registro de front-end digital (`DIG0_DIG_FE_CNTL`); los demas van de
/// `0x100` en `0x100`.
const DIG_FE_CNTL: u32 = 0x4a00;
/// `DIG_FE_CNTL.DIG_START` y `.DIG_SYMCLK_FE_ON`: el front-end esta sacando
/// imagen. Los bits 0-2 dicen de que CRTC la toma (0 = CRTC 1).
const DIG_START: u32 = 1 << 10;
const DIG_SYMCLK_FE_ON: u32 = 1 << 24;

/// Paso, en palabras, entre bloques de registro de CRTC contiguos (`0x100`
/// bytes en la DCE). El que se usa aqui es el CRTC1 (indice 0), asi que la
/// constante solo hace falta para dejarlo dicho.
const CRTC_OFFSET: u32 = 0x40;
/// `CRTC_CONTROL.CRTC_MASTER_EN` (bit 0): el CRTC esta sacando imagen.
const CRTC_MASTER_EN: u32 = 1 << 0;
/// `GRPH_CONTROL`: `GRPH_DEPTH` = 2 (32 bpp) y `GRPH_FORMAT` = 0 (ARGB8888).
const GRPH_DEPTH_32BPP: u32 = 2;
const GRPH_ENABLE: u32 = 0x1a00;
const GRPH_CONTROL: u32 = 0x1a01;
const GRPH_PITCH: u32 = 0x1a06;
const GRPH_PRIMARY_SURFACE_ADDRESS: u32 = 0x1a04;
const GRPH_PRIMARY_SURFACE_ADDRESS_HIGH: u32 = 0x1a07;
const GRPH_X_START: u32 = 0x1a0b;
const GRPH_X_END: u32 = 0x1a0d;
const GRPH_Y_START: u32 = 0x1a0c;
const GRPH_Y_END: u32 = 0x1a0e;
const GRPH_SURFACE_OFFSET_X: u32 = 0x1a10;
const GRPH_SURFACE_OFFSET_Y: u32 = 0x1a11;
const LB_DESKTOP_HEIGHT: u32 = 0x1ac3;
const VIEWPORT_START: u32 = 0x1b5c;
const VIEWPORT_SIZE: u32 = 0x1b5d;

/// Si este registro no cabe en la ventana, no se interpreta el volcado: los
/// ceros que devuelve `leer` fuera de la ventana no significan nada.
const REG_MAS_ALTO: u32 = CRTC_V_BLANK_START_END;

// Campos de `CUR_CONTROL` y `CUR_UPDATE` (los `...__SHIFT` de la cabecera).
/// `CUR_CONTROL.CURSOR_EN` (bit 0).
const CUR_EN: u32 = 1 << 0;
/// `CUR_CONTROL.CURSOR_MODE` (bits 8-9); 2 es el modo de 32 bpp que usa
/// `dce_v11_0_show_cursor`.
const CUR_MODO32: u32 = 2 << 8;
/// `CUR_UPDATE.CURSOR_UPDATE_LOCK` (bit 16).
const CUR_BLOQUEO: u32 = 1 << 16;

/// Bloque del cursor: el registro mas alto que toca la fase 14b.
const CUR_MAS_ALTO: u32 = CUR_UPDATE;

/// Lado del cursor, en pixeles (DCE admite hasta 128).
const CURSOR_LADO: u32 = 64;
/// Bytes del bitmap: 64x64 a 32 bpp = 16 KiB. El cursor va empaquetado (no hay
/// registro de paso: el paso es `ancho * 4`).
const CURSOR_BYTES: u64 = (CURSOR_LADO * CURSOR_LADO * 4) as u64;

/// La GPU AMD localizada: la ventana de registros y, si la hay, la de VRAM.
#[derive(Clone, Copy)]
pub(crate) struct Gpu {
    /// El BAR con los registros (el 5 en las Radeon desde Bonaire).
    pub(crate) registros: Bar,
    /// La abertura por la que se ve la VRAM (el BAR 0 grande), si la hay.
    pub(crate) vram: Option<Bar>,
}

/// Cursor por hardware encendido.
#[derive(Clone, Copy)]
struct Cursor {
    /// Desplazamiento del bitmap dentro de la abertura de VRAM.
    off: u64,
    /// Posicion en pantalla.
    x: u32,
    y: u32,
}

static GPU: SyncCell<Option<Gpu>> = SyncCell::new(None);
static CURSOR: SyncCell<Option<Cursor>> = SyncCell::new(None);

fn slot() -> &'static mut Option<Gpu> {
    // Seguro: solo lo toca la tarea del nucleo, de una en una.
    unsafe { &mut *GPU.get() }
}

fn cursor_slot() -> &'static mut Option<Cursor> {
    // Seguro: igual que `slot`, un solo hilo de ejecucion.
    unsafe { &mut *CURSOR.get() }
}

/// La primera GPU de AMD/ATI del bus (una funcion de clase video).
pub fn buscar() -> Option<Funcion> {
    pci::funciones()
        .iter()
        .copied()
        .find(|f| f.vendor == FABRICANTE && f.es_video())
}

/// Localiza la GPU y deja lista su ventana de registros. Se llama al arrancar.
///
/// Se comprueba que la ventana existe y que esta por debajo de los 4 GiB (donde
/// llega el mapa de identidad del bootloader) y se marca como no cacheable.
pub fn init() {
    let Some(f) = buscar() else { return };

    // La identificacion se ensena siempre, aunque luego no se pueda abrir la
    // ventana: es lo primero que se quiere saber en un PC real.
    crate::println!(
        "rocky: GPU AMD {:04X}:{:04X} en {:02X}:{:02X}.{} ({}).",
        f.vendor,
        f.device,
        f.bus,
        f.dev,
        f.func,
        pci::producto_nombre(f.vendor, f.device).unwrap_or("GPU AMD")
    );

    let bares = pci::medir_bares(f);

    // Desde Bonaire (la familia de Polaris) los registros viven en el BAR 5; en
    // las Radeon anteriores, en el BAR 2. Se prefiere el 5 y, si no lo hay, el 2.
    let dentro_del_mapa = |b: &Bar| {
        b.base != 0
            && b.base < 0x1_0000_0000
            && b.base
                .checked_add(b.size)
                .is_some_and(|fin| fin <= 0x1_0000_0000)
    };
    let registros = bares[5]
        .filter(|b| !b.es_io && b.base != 0)
        .or_else(|| bares[2].filter(|b| !b.es_io && b.base != 0));

    let registros = match registros {
        Some(b) if dentro_del_mapa(&b) => b,
        Some(b) => {
            crate::println!(
                "rocky: GPU: los registros caen en 0x{:X}, por encima de 4 GiB; aun sin mapear.",
                b.base
            );
            return;
        }
        None => return,
    };

    // La memoria de un dispositivo no se cachea.
    crate::paging::sin_cache(registros.base as usize);

    let vram = bares[0].filter(|b| !b.es_io && dentro_del_mapa(b));

    // El kernel aun no envia comandos DMA. Dejamos activado el permiso PCI
    // exclusivamente en Polaris con ambos recursos MMIO dentro del mapa que
    // prepara el bootloader; asi el siguiente paso SDMA no depende de que la
    // BIOS haya dejado Bus Mastering encendido. No se toca la ati-vga de QEMU.
    if es_polaris(f.device) && vram.is_some() {
        if pci::habilitar_memoria_y_bus_master(f) {
            crate::println!("rocky: AMD Polaris: PCI Memory Space + Bus Master habilitados (SDMA aun no iniciado).");
        } else {
            crate::println!(
                "rocky: AMD Polaris: no se pudieron habilitar PCI Memory Space/Bus Master."
            );
        }
    }

    *slot() = Some(Gpu { registros, vram });

    let (n, u) = pci::tam_txt(registros.size);
    crate::println!(
        "rocky: GPU: registros en 0x{:08X} ({} {}).",
        registros.base,
        n,
        u
    );
}

/// Orden `gpu`: identifica la tarjeta y ensena sus BAR y el bloque de pantalla.
///
/// Devuelve `false` si no hay ninguna GPU AMD, para que el despacho de la clase
/// deje probar al siguiente fabricante.
pub fn mostrar() -> bool {
    let Some(g) = buscar() else {
        return false;
    };

    let sub = pci::leer32(g.bus, g.dev, g.func, 0x2C);
    crate::println!(
        "GPU AMD {:04X}:{:04X} - {} - en {:02X}:{:02X}.{}",
        g.vendor,
        g.device,
        pci::producto_nombre(g.vendor, g.device).unwrap_or("GPU AMD"),
        g.bus,
        g.dev,
        g.func
    );
    crate::println!(
        "  revision 0x{:02X}, clase {:02X}{:02X}{:02X}, subsistema {:04X}:{:04X}",
        g.revision,
        g.clase,
        g.subclase,
        g.prog_if,
        (sub >> 16) as u16,
        sub as u16
    );

    // Estado de la funcion: interesa saber si tiene la decodificacion de memoria
    // activa, que es como el firmware la deja para que se vea la pantalla.
    let comando = pci::leer32(g.bus, g.dev, g.func, 0x04);
    crate::println!(
        "  comando 0x{:04X} (memoria {}, E/S {}, bus maestro {})",
        comando & 0xFFFF,
        bit(comando, 1),
        bit(comando, 0),
        bit(comando, 2)
    );

    // Para medir el tamano de un BAR hay que escribirle todo a unos y leer la
    // mascara; `medir_bares` apaga la decodificacion mientras dura y la restaura.
    let bares = pci::medir_bares(g);
    let mut i = 0u8;
    while i < 6 {
        match bares[i as usize] {
            Some(b) => {
                let (n, u) = pci::tam_txt(b.size);
                crate::println!(
                    "  BAR{}  {:<4}{}  base 0x{:012X}  tamano {} {}",
                    i,
                    if b.es_io { "E/S" } else { "MMIO" },
                    if b.es_64 { " 64" } else { "" },
                    b.base,
                    n,
                    u
                );
                if b.base >= 0x1_0000_0000 {
                    crate::println!("        ^ por encima de 4 GiB: habra que mapearlo");
                }
                i += if b.es_64 { 2 } else { 1 };
            }
            None => {
                crate::println!("  BAR{}  (sin asignar)", i);
                i += 1;
            }
        }
    }

    // Resumen en una sola linea (base/tamano en hexadecimal), comodo de copiar.
    crate::print!("  resumen:");
    for (i, b) in bares.iter().enumerate() {
        match b {
            Some(b) => crate::print!(" BAR{}=0x{:X}/0x{:X}", i, b.base, b.size),
            None => crate::print!(" BAR{}=0", i),
        }
    }
    crate::println!();
    crate::println!(
        "  (en las Radeon: el BAR grande es la VRAM; los registros, el BAR5 de 256 KiB)"
    );

    // Y, ya que la ventana esta localizada, el estado del bloque de pantalla.
    volcado();
    true
}

/// Ensena algunos registros del bloque de pantalla.
///
/// El bloque es el que el firmware BIOS/VBE dejo en marcha antes de ceder el
/// control al kernel: sirve para ver como esta configurada hoy la pantalla.
pub fn volcado() {
    let Some(g) = *slot() else { return };
    let base = g.registros.base;
    let size = g.registros.size;

    let (n, u) = pci::tam_txt(size);
    crate::println!(
        "  Registros del bloque de pantalla (ventana 0x{:08X}, {} {}):",
        base,
        n,
        u
    );
    match g.vram {
        Some(v) => {
            let (vn, vu) = pci::tam_txt(v.size);
            crate::println!(
                "  (la abertura de VRAM esta en 0x{:08X}, {} {})",
                v.base,
                vn,
                vu
            );
        }
        None => crate::println!("  (no hay abertura de VRAM declarada)"),
    }

    macro_rules! muestra {
        ($nombre:expr, $reg:expr) => {
            match leer(base, size, $reg) {
                Some(v) => crate::println!("    {:<28} 0x{:08X}", $nombre, v),
                None => crate::println!("    {:<28} (fuera de la ventana)", $nombre),
            }
        };
    }

    muestra!("GRPH_ENABLE", GRPH_ENABLE);
    muestra!("GRPH_CONTROL", GRPH_CONTROL);
    muestra!("GRPH_PITCH", GRPH_PITCH);
    muestra!("GRPH_PRIMARY_SURFACE_ADDRESS", GRPH_PRIMARY_SURFACE_ADDRESS);
    muestra!(
        "GRPH_PRIMARY_SURFACE_ADDRESS_HIGH",
        GRPH_PRIMARY_SURFACE_ADDRESS_HIGH
    );
    muestra!("CRTC_H_TOTAL", CRTC_H_TOTAL);
    muestra!("CRTC_H_BLANK_START_END", CRTC_H_BLANK_START_END);
    muestra!("CRTC_V_TOTAL", CRTC_V_TOTAL);
    muestra!("CRTC_V_BLANK_START_END", CRTC_V_BLANK_START_END);
    muestra!("CRTC_CONTROL", CRTC_CONTROL);
    muestra!("CUR_CONTROL", CUR_CONTROL);
    muestra!("CUR_SURFACE_ADDRESS", CUR_SURFACE_ADDRESS);
    muestra!("CUR_SURFACE_ADDRESS_HIGH", CUR_SURFACE_ADDRESS_HIGH);
    muestra!("CUR_SIZE", CUR_SIZE);
    muestra!("CUR_POSITION", CUR_POSITION);
    muestra!("CUR_HOT_SPOT", CUR_HOT_SPOT);
    muestra!("CUR_UPDATE", CUR_UPDATE);

    // Si el bloque de pantalla no cabe entero, no se interpreta nada: los ceros
    // que devuelve `leer` fuera de la ventana no significan nada.
    if (REG_MAS_ALTO as u64 + 1) * 4 > size {
        crate::println!("    (el bloque DCE no cabe en esta ventana: parece una GPU anterior)");
        return;
    }

    let valor = |reg: u32| leer(base, size, reg).unwrap_or(0);
    let ancho = (valor(CRTC_H_TOTAL) & 0x3fff) + 1;
    let alto = (valor(CRTC_V_TOTAL) & 0x3fff) + 1;
    let pitch = valor(GRPH_PITCH) & 0x7fff;
    let plano = (valor(GRPH_PRIMARY_SURFACE_ADDRESS) as u64)
        | ((valor(GRPH_PRIMARY_SURFACE_ADDRESS_HIGH) as u64) << 32);
    let cursor = valor(CUR_CONTROL);
    crate::println!(
        "    -> modo {}x{} px; plano en 0x{:X} ({} px/linea); cursor {}",
        ancho,
        alto,
        plano,
        pitch,
        if cursor & CUR_EN != 0 {
            "activo"
        } else {
            "apagado"
        }
    );
}

// --------------------------------------------------------------------------
// Cursor por hardware (fase 14b)
// --------------------------------------------------------------------------

/// Orden `cursor`: cursor por hardware (fase 14b).
///
/// ```text
///   cursor            deja el bitmap en la VRAM y lo enciende
///   cursor X Y        lo mueve a (X, Y)
///   cursor off        lo apaga (`CUR_CONTROL.CURSOR_EN = 0`)
///   cursor info       solo ensena el estado
/// ```
///
/// Devuelve `false` si no hay ninguna GPU AMD con ventana de registros, para
/// que el despacho de la clase pruebe con el siguiente fabricante.
pub fn cursor(arg: &str) -> bool {
    let Some(g) = *slot() else { return false };

    let mut it = arg.split_whitespace();
    match it.next() {
        Some("off") | Some("apaga") => apagar(&g),
        Some("info") => informar(&g),
        Some(n) => {
            let x = n.parse::<u32>().ok();
            let y = it.next().and_then(|s| s.parse::<u32>().ok());
            match (x, y) {
                (Some(x), Some(y)) => mover(&g, x, y),
                _ => crate::println!("uso: cursor [X Y | off | info]"),
            }
        }
        None => encender(&g),
    }
    true
}

/// Deja el bitmap del cursor en la VRAM y enciende el cursor por hardware.
fn encender(g: &Gpu) {
    let Some(vram) = g.vram else {
        crate::println!("cursor: la GPU no declara abertura de VRAM (un BAR0 grande).");
        return;
    };
    if vram.size < CURSOR_BYTES + 0x20_0000 {
        crate::println!(
            "cursor: la abertura de VRAM ({} bytes) es demasiado pequena para el bitmap.",
            vram.size
        );
        return;
    }

    // El bitmap va al final de la abertura, lejos del framebuffer que deja VBE.
    // Pero la abertura puede ser mas grande que la VRAM de verdad: en QEMU la
    // ati-vga declara 64 MiB de abertura y solo tiene 16 MiB de memoria, y lo
    // que se escribe por encima se pierde. Asi que se busca el ultimo tramo que
    // responda, en vez de dar por hecho que la abertura entera es memoria.
    let Some(off) = buscar_hueco(g, vram) else {
        crate::println!(
            "cursor: no encuentro un tramo de VRAM que responda al final de la abertura."
        );
        return;
    };

    let (vn, vu) = pci::tam_txt(vram.size);
    crate::println!(
        "cursor: bitmap de {}x{} px (32 bpp, {} KiB) en la VRAM, desplazamiento 0x{:X} (abertura 0x{:08X}, {} {}).",
        CURSOR_LADO,
        CURSOR_LADO,
        CURSOR_BYTES / 1024,
        off,
        vram.base,
        vn,
        vu
    );
    if off + CURSOR_BYTES < vram.size {
        crate::println!(
            "cursor: la abertura es mas grande que la VRAM de verdad: el bitmap va en el ultimo tramo que responde."
        );
    }

    match escribir_bitmap(g, off) {
        Err(()) => {
            crate::println!("cursor: el bitmap se sale de la abertura de VRAM; no se toca nada.");
            return;
        }
        Ok(0) => crate::println!(
            "cursor: VRAM escribe/lee OK ({} px comprobados).",
            CURSOR_LADO * CURSOR_LADO
        ),
        Ok(malos) => {
            crate::println!(
                "cursor: la VRAM no devuelve lo escrito ({} de {} px distintos).",
                malos,
                CURSOR_LADO * CURSOR_LADO
            );
            return;
        }
    }

    // El cursor por hardware es del bloque DCE; si no cabe entero en esta
    // ventana, la tarjeta es anterior y no hay nada que programar.
    let size = g.registros.size;
    if (CUR_MAS_ALTO as u64 + 1) * 4 > size {
        crate::println!(
            "cursor: el bloque del cursor no cabe en esta ventana: parece una GPU anterior."
        );
        return;
    }

    let addr = direccion_gpu(g, off);
    let (x, y) = posicion_inicial();
    programar(g, off, addr, x, y);
}

/// Busca un tramo al final de la abertura de VRAM donde la memoria responda.
///
/// La abertura del BAR0 puede ser mas grande que la memoria de verdad (en QEMU,
/// la ati-vga declara 64 MiB de abertura y tiene 16 MiB): lo que se escribe por
/// encima se pierde. Se prueba desde el final hacia abajo, de 2 MiB en 2 MiB
/// (lo que mide una pagina del mapa de identidad), y se para en el primer tramo
/// que devuelve lo escrito y releido.
///
/// No se baja de [`VRAM_MINIMA`] para no pisar el framebuffer que dejo VBE, que
/// vive al principio de la abertura.
fn buscar_hueco(g: &Gpu, vram: Bar) -> Option<u64> {
    let mut off = (vram.size - CURSOR_BYTES) & !0x1F_FFFF;
    while off >= VRAM_MINIMA {
        // La pagina se marca como no cacheable *antes* de sondearla: si no, en
        // un PC real la relectura podria salir de la cache y la sonda mentiria.
        crate::paging::sin_cache((vram.base + off) as usize);
        if prueba_escritura(g, off) {
            return Some(off);
        }
        off -= 0x20_0000;
    }
    None
}

/// Tamano por debajo del cual no se busca el bitmap del cursor (8 MiB).
///
/// Debajo puede estar el framebuffer de VBE, y no se pisa: es preferible no
/// poner el cursor a borrar lo que se ve.
const VRAM_MINIMA: u64 = 0x80_0000;

/// Escribe dos palabras al principio y al final del tramo y las vuelve a leer.
///
/// Las dos esquinas se comprueban porque el tramo entero tiene que caer en la
/// misma pagina de 2 MiB (y en el mismo trozo de memoria que responde).
fn prueba_escritura(g: &Gpu, off: u64) -> bool {
    const PRUEBA: [u32; 2] = [0xA5A5_A5A5, 0x5A5A_5A5A];
    let fin = off + CURSOR_BYTES - 4 * PRUEBA.len() as u64;

    for (i, valor) in PRUEBA.iter().enumerate() {
        let d = i as u64 * 4;
        if !vram_escribir(g, off + d, *valor) || !vram_escribir(g, fin + d, *valor) {
            return false;
        }
    }
    (0..PRUEBA.len()).all(|i| {
        let d = i as u64 * 4;
        vram_leer(g, off + d) == Some(PRUEBA[i]) && vram_leer(g, fin + d) == Some(PRUEBA[i])
    })
}

/// Apaga el cursor por hardware.
fn apagar(g: &Gpu) {
    let base = g.registros.base;
    let size = g.registros.size;
    if (CUR_MAS_ALTO as u64 + 1) * 4 > size {
        crate::println!("cursor: esta GPU no tiene bloque de cursor que apagar.");
        return;
    }
    let control = leer(base, size, CUR_CONTROL).unwrap_or(0) & !CUR_EN;
    escribir(base, size, CUR_CONTROL, control);
    *cursor_slot() = None;
    crate::println!("cursor: apagado (CUR_CONTROL.CURSOR_EN = 0).");
}

/// Mueve el cursor por hardware a (`x`, `y`).
fn mover(g: &Gpu, x: u32, y: u32) {
    let base = g.registros.base;
    let size = g.registros.size;
    if (CUR_MAS_ALTO as u64 + 1) * 4 > size {
        crate::println!("cursor: esta GPU no tiene bloque de cursor.");
        return;
    }
    if cursor_slot().is_none() {
        crate::println!("cursor: todavia no esta encendido; prueba 'cursor' a secas.");
        return;
    }

    // Las actualizaciones se bloquean mientras se cambia la posicion (igual que
    // `dce_v11_0_lock_cursor`), para que el bloque no la lea a medias.
    let mut update = leer(base, size, CUR_UPDATE).unwrap_or(0);
    escribir(base, size, CUR_UPDATE, update | CUR_BLOQUEO);
    escribir(base, size, CUR_POSITION, (x << 16) | y);
    update &= !CUR_BLOQUEO;
    escribir(base, size, CUR_UPDATE, update);

    if let Some(c) = cursor_slot() {
        c.x = x;
        c.y = y;
    }
    crate::println!("cursor: movido a ({}, {}).", x, y);
}

/// Ensena el estado del cursor (el nuestro y el que dice el hardware).
fn informar(g: &Gpu) {
    match *cursor_slot() {
        Some(c) => crate::println!(
            "cursor: encendido en ({}, {}), bitmap en la VRAM (desplazamiento 0x{:X}).",
            c.x,
            c.y,
            c.off
        ),
        None => crate::println!("cursor: apagado."),
    }
    let control = leer(g.registros.base, g.registros.size, CUR_CONTROL).unwrap_or(0);
    crate::println!(
        "  CUR_CONTROL = 0x{:08X} (EN={}, modo={})",
        control,
        control & CUR_EN,
        (control >> 8) & 3
    );
}

/// Programa el bloque `CUR_*` para que el bloque de pantalla saque el cursor
/// desde `off` (VRAM) en la direccion de GPU `addr`, en (`x`, `y`).
fn programar(g: &Gpu, off: u64, addr: u64, x: u32, y: u32) {
    let base = g.registros.base;
    let size = g.registros.size;

    // 1) Se bloquean las actualizaciones mientras se cambian los registros.
    let mut update = leer(base, size, CUR_UPDATE).unwrap_or(0);
    escribir(base, size, CUR_UPDATE, update | CUR_BLOQUEO);

    // 2) Superficie, tamano, posicion y punto activo. En este bloque la parte
    //    "horizontal" (X, ancho) va en los bits altos y la vertical en los bajos.
    escribir(base, size, CUR_SURFACE_ADDRESS_HIGH, (addr >> 32) as u32);
    escribir(base, size, CUR_SURFACE_ADDRESS, addr as u32);
    escribir(
        base,
        size,
        CUR_SIZE,
        ((CURSOR_LADO - 1) << 16) | (CURSOR_LADO - 1),
    );
    escribir(base, size, CUR_POSITION, (x << 16) | y);
    escribir(base, size, CUR_HOT_SPOT, 0);

    // 3) Se enciende con el modo de 32 bpp.
    escribir(base, size, CUR_CONTROL, CUR_EN | CUR_MODO32);

    // 4) Se desbloquea, conservando los demas bits de estado.
    update &= !CUR_BLOQUEO;
    escribir(base, size, CUR_UPDATE, update);

    *cursor_slot() = Some(Cursor { off, x, y });
    crate::println!(
        "cursor: encendido en ({}, {}) con el bitmap en 0x{:X} (VRAM 0x{:X}), tamano {}x{}.",
        x,
        y,
        addr,
        off,
        CURSOR_LADO,
        CURSOR_LADO
    );
}

/// Direccion de GPU del bitmap (que es lo que espera `CUR_SURFACE_ADDRESS`).
///
/// El registro quiere una direccion **de la GPU** (de su espacio de memoria),
/// no la fisica del CPU. La BIOS deja la del plano de scanout en
/// `GRPH_PRIMARY_SURFACE_ADDRESS`; comparandola con la direccion fisica del
/// framebuffer que conocemos por VBE sale la diferencia entre los dos espacios,
/// que es la misma para toda la abertura.
fn direccion_gpu(g: &Gpu, off: u64) -> u64 {
    let base = g.registros.base;
    let size = g.registros.size;
    let plano = match (
        leer(base, size, GRPH_PRIMARY_SURFACE_ADDRESS),
        leer(base, size, GRPH_PRIMARY_SURFACE_ADDRESS_HIGH),
    ) {
        (Some(lo), Some(hi)) => (lo as u64) | ((hi as u64) << 32),
        _ => 0,
    };

    match crate::pantalla::modo() {
        Some(m) if m.lfb != 0 && plano != 0 => {
            let vram = g.vram.map(|v| v.base).unwrap_or(0);
            let dir = plano.wrapping_add((vram + off).wrapping_sub(m.lfb as u64));
            crate::println!(
                "cursor: scanout de la BIOS en 0x{:X}, framebuffer VBE en 0x{:X} -> cursor en 0x{:X}",
                plano,
                m.lfb,
                dir
            );
            dir
        }
        _ => {
            crate::println!(
                "cursor: sin modo apuntado (ni VBE ni driver); supongo que el desplazamiento en la abertura (0x{:X}) ya es la direccion de GPU.",
                off
            );
            off
        }
    }
}

/// Posicion de partida del cursor: arriba a la izquierda y separada del borde
/// (`cursor::MARGEN_INICIAL`), igual que el puntero por software de Kira. No
/// hace falta condicion por modo: cualquier modo util es bastante mas ancho y
/// alto que ese margen, y aun asi se recorta al borde por si acaso.
fn posicion_inicial() -> (u32, u32) {
    let m = crate::cursor::MARGEN_INICIAL;
    match crate::pantalla::modo() {
        Some(md) => (
            m.clamp(0, md.ancho.saturating_sub(1) as i32) as u32,
            m.clamp(0, md.alto.saturating_sub(1) as i32) as u32,
        ),
        None => (m as u32, m as u32),
    }
}

/// Escribe el bitmap completo en la VRAM y lo relee.
///
/// Devuelve `Err` si el bitmap no cabe en la abertura y, si no, cuantos pixeles
/// no coinciden al releerlos (0 = todo bien).
fn escribir_bitmap(g: &Gpu, off: u64) -> Result<u32, ()> {
    for y in 0..CURSOR_LADO {
        for x in 0..CURSOR_LADO {
            let dir = off + (y * CURSOR_LADO + x) as u64 * 4;
            if !vram_escribir(g, dir, pixel(x, y)) {
                return Err(());
            }
        }
    }

    let mut malos = 0;
    for y in 0..CURSOR_LADO {
        for x in 0..CURSOR_LADO {
            let dir = off + (y * CURSOR_LADO + x) as u64 * 4;
            if vram_leer(g, dir) != Some(pixel(x, y)) {
                malos += 1;
            }
        }
    }
    Ok(malos)
}

/// Flecha del cursor: una rejilla de 16x16 que se agranda x4 (64x64).
///
/// `#` es el borde (negro), `.` el relleno (blanco) y el espacio, transparente.
/// Es provisional: para la fase basta con que sea inconfundible y tenga las tres
/// clases de pixel (opaco negro, opaco blanco y transparente).
const FLECHA: [&str; 16] = [
    "#               ",
    "##              ",
    "#.#             ",
    "#..#            ",
    "#...#           ",
    "#....#          ",
    "#.....#         ",
    "#......#        ",
    "#.......#       ",
    "#........#      ",
    "#.....#####     ",
    "#..#....#       ",
    "#.#.#...#       ",
    "##..#...#       ",
    "    #...#       ",
    "    #...#       ",
];

/// Color ARGB8888 (el empaquetado que espera el cursor del DCE) del pixel
/// (`x`, `y`) del bitmap. Cada pixel de la rejilla ocupa 4x4.
fn pixel(x: u32, y: u32) -> u32 {
    let glifo = FLECHA
        .get((y / 4) as usize)
        .and_then(|fila| fila.as_bytes().get((x / 4) as usize))
        .copied()
        .unwrap_or(b' ');
    match glifo {
        b'#' => 0xFF00_0000, // negro opaco
        b'.' => 0xFFFF_FFFF, // blanco opaco
        _ => 0x0000_0000,    // transparente
    }
}

// --------------------------------------------------------------------------
// Acceso a la ventana de registros y a la abertura de VRAM
// --------------------------------------------------------------------------

/// Lee un registro de la ventana (numero en palabras de 32 bits).
///
/// Devuelve `None` si el registro no cabe en la ventana: asi no se toca memoria
/// de fuera del BAR, que en el peor caso haria abortar el acceso al bus.
fn leer(base: u64, size: u64, reg: u32) -> Option<u32> {
    if (reg as u64 + 1) * 4 > size {
        return None;
    }
    let direccion = base + (reg as u64) * 4;
    Some(unsafe { core::ptr::read_volatile(direccion as *const u32) })
}

pub(crate) fn reg_leer(g: &Gpu, reg: u32) -> u32 {
    leer(g.registros.base, g.registros.size, reg).unwrap_or(0)
}

pub(crate) fn reg_escribir(g: &Gpu, reg: u32, valor: u32) -> Option<()> {
    if escribir(g.registros.base, g.registros.size, reg, valor) {
        Some(())
    } else {
        None
    }
}

/// Escribe un registro de la ventana (numero en palabras de 32 bits).
///
/// Devuelve `false` si el registro no cabe en esta ventana (no se toca nada).
fn escribir(base: u64, size: u64, reg: u32, valor: u32) -> bool {
    if (reg as u64 + 1) * 4 > size {
        return false;
    }
    let direccion = base + (reg as u64) * 4;
    unsafe { core::ptr::write_volatile(direccion as *mut u32, valor) };
    true
}

/// Espera activa de `ms` milisegundos, medida con el reloj del sistema (100 Hz).
fn esperar_ms(ms: u32) {
    let t0 = crate::pit::ticks();
    let objetivo = ((ms as u64) / 10).max(1);
    while crate::pit::ticks().wrapping_sub(t0) < objetivo {
        core::hint::spin_loop();
    }
}

/// Cuenta los cuadros que saca el CRTC 1 durante `ms` milisegundos.
///
/// Es la prueba de que el CRTC late de verdad (que el modo esta activo), aunque
/// no haya ningun monitor conectado: la logica de tiempos corre igual.
fn contar_cuadros(g: &Gpu, ms: u32) -> u32 {
    let f1 = reg_leer(g, CRTC_STATUS_FRAME_COUNT);
    esperar_ms(ms);
    let f2 = reg_leer(g, CRTC_STATUS_FRAME_COUNT);
    f2.wrapping_sub(f1)
}

/// Identificadores de tarjeta de la familia Polaris (la que usa DCE 11).
///
/// Solo con estas se sabe que los numeros de registro del bloque de pantalla de
/// este fichero son los correctos: son los de `dce_11_0_d.h`. En cualquier otra
/// tarjeta se deja el camino VBE.
fn es_polaris(device: u16) -> bool {
    matches!(device, 0x67C0..=0x67FF | 0x6980..=0x699F)
}

/// Enciende o apaga un CRTC con la secuencia de `dce_v11_0_crtc_dpms`.
///
/// Al encender: `EnableCRTC` si aun no lo estaba, y luego quitar el blanco. Al
/// apagar: poner el blanco si estaba encendido, y luego `EnableCRTC` a apagado.
fn dpms(g: &Gpu, crtc: u8, encender: bool) {
    let encendido = reg_leer(g, CRTC_CONTROL + (crtc as u32) * CRTC_OFFSET) & CRTC_MASTER_EN != 0;
    if encender {
        if !encendido {
            let _ = atom::enable_crtc(g, crtc, true);
        }
        let _ = atom::blank_crtc(g, crtc, false);
    } else {
        if encendido {
            let _ = atom::blank_crtc(g, crtc, true);
        }
        let _ = atom::enable_crtc(g, crtc, false);
    }
}

/// Programa el plano de scanout (`GRPH_*`) para un lienzo lineal de 32 bpp que
/// empieza en la direccion de GPU `gpu_addr`.
fn programar_plano(g: &Gpu, gpu_addr: u64, ancho: u32, alto: u32, pitch: u32) {
    // `GRPH_DEPTH` = 2 (32 bpp) y `GRPH_FORMAT` = 0 (ARGB8888 lineal), igual que
    // hace `dce_v11_0_crtc_do_set_base` para `DRM_FORMAT_ARGB8888`.
    let _ = reg_escribir(g, GRPH_CONTROL, GRPH_DEPTH_32BPP);
    let _ = reg_escribir(
        g,
        GRPH_PRIMARY_SURFACE_ADDRESS_HIGH,
        (gpu_addr >> 32) as u32,
    );
    let _ = reg_escribir(
        g,
        GRPH_PRIMARY_SURFACE_ADDRESS,
        (gpu_addr as u32) & 0xFFFF_FF00,
    );
    let _ = reg_escribir(g, GRPH_SURFACE_OFFSET_X, 0);
    let _ = reg_escribir(g, GRPH_SURFACE_OFFSET_Y, 0);
    let _ = reg_escribir(g, GRPH_X_START, 0);
    let _ = reg_escribir(g, GRPH_Y_START, 0);
    let _ = reg_escribir(g, GRPH_X_END, ancho);
    let _ = reg_escribir(g, GRPH_Y_END, alto);
    // Ojo: en este bloque el paso va en **pixeles**, no en bytes.
    let _ = reg_escribir(g, GRPH_PITCH, pitch / 4);
    let _ = reg_escribir(g, GRPH_ENABLE, 1);
    let _ = reg_escribir(g, LB_DESKTOP_HEIGHT, alto);
    let _ = reg_escribir(g, VIEWPORT_START, 0);
    let _ = reg_escribir(g, VIEWPORT_SIZE, (ancho << 16) | ((alto + 1) & !1));
}

/// Escribe a mano los registros de tiempos del CRTC.
///
/// Son las mismas formulas que aplica `dce110_timing_generator_program_blanking`
/// (el camino "sin VBIOS" del driver de referencia) para `CRTC_H_TOTAL`,
/// `CRTC_V_TOTAL`, `CRTC_V_TOTAL_MIN`/`MAX`, `CRTC_H_BLANK_START_END` y
/// `CRTC_V_BLANK_START_END`. No toca los registros de sincronia (`CRTC_H_SYNC_A`
/// y `CRTC_V_SYNC_A`): en la DCE 11 esos solo los programa la VBIOS, y aqui se
/// sigue confiando en la tabla ATOM para dejarlos.
///
/// Sirve de red de seguridad si la tabla ATOM no llega a completar los tiempos
/// (por ejemplo, si la ejecucion de la tabla se desvia a mitad). El sentido de
/// `CRTC_H_BLANK_END`/`CRTC_V_BLANK_END` es el "raro" de la referencia: cuenta
/// desde el arranque de la sincronia hasta el final de la linea (porche
/// posterior mas ancho de sincronia).
fn escribir_tiempos(g: &Gpu, t: &atom::Timing, crtc: u8) {
    let base = (crtc as u32) * CRTC_OFFSET;
    let h_total = t.h_total();
    let v_total = t.v_total();
    let h_sync_start = t.h_display + t.h_sync_offset;
    let v_sync_start = t.v_display + t.v_sync_offset;

    let h_blank_end = h_total.saturating_sub(h_sync_start);
    let h_blank_start = h_blank_end + t.h_display;
    let v_blank_end = v_total.saturating_sub(v_sync_start);
    let v_blank_start = v_blank_end + t.v_display;

    let _ = reg_escribir(g, CRTC_H_TOTAL + base, (h_total - 1) & 0x3FFF);
    let _ = reg_escribir(
        g,
        CRTC_H_BLANK_START_END + base,
        ((h_blank_start & 0x3FFF) << 16) | (h_blank_end & 0x3FFF),
    );
    let _ = reg_escribir(g, CRTC_V_TOTAL + base, (v_total - 1) & 0x3FFF);
    let _ = reg_escribir(g, CRTC_V_TOTAL_MIN + base, (v_total - 1) & 0x3FFF);
    let _ = reg_escribir(g, CRTC_V_TOTAL_MAX + base, (v_total - 1) & 0x3FFF);
    let _ = reg_escribir(
        g,
        CRTC_V_BLANK_START_END + base,
        ((v_blank_start & 0x3FFF) << 16) | (v_blank_end & 0x3FFF),
    );
}

/// Vuelca los registros de tiempos del CRTC para diagnosticar un modo que no
/// cuadra. Los cuatro campos (`START`/`END`) van en dos mitades de 16 bits.
fn volcar_tiempos(g: &Gpu, crtc: u8) {
    let base = (crtc as u32) * CRTC_OFFSET;
    let hb = reg_leer(g, CRTC_H_BLANK_START_END + base);
    let hs = reg_leer(g, CRTC_H_SYNC_A + base);
    let vb = reg_leer(g, CRTC_V_BLANK_START_END + base);
    let vs = reg_leer(g, CRTC_V_SYNC_A + base);
    crate::println!(
        "amd: H_BLANK {}..{} H_SYNC {}..{} V_BLANK {}..{} V_SYNC {}..{}",
        hb & 0x3FFF,
        hb >> 16,
        hs & 0x3FFF,
        hs >> 16,
        vb & 0x3FFF,
        vb >> 16,
        vs & 0x3FFF,
        vs >> 16
    );
}

/// Aplica un modo con la secuencia de DCE 11 (la de `dce_v11_0_crtc_prepare`,
/// `dce_v11_0_crtc_mode_set` y `dce_v11_0_crtc_commit`) y comprueba que los
/// registros de tiempos del CRTC quedan como se pidieron.
///
/// `crtc` es el indice de ATOM (`0` = CRTC1), `encoder_id`/`ppll`/`encoder_mode`
/// son los del candidato que se esta probando.
fn aplicar_modo(
    g: &Gpu,
    t: &atom::Timing,
    crtc: u8,
    encoder_id: u8,
    ppll: u8,
    encoder_mode: u8,
    gpu_addr: u64,
    ancho: u32,
    alto: u32,
    pitch: u32,
) -> bool {
    // --- prepare: powergate, bloqueo y apagado ---
    atom::disp_power_gating(g, crtc, atom::ATOM_DISABLE);
    atom::update_crtc_double_buffer(g, crtc, true);
    dpms(g, crtc, false);

    // --- mode_set: PLL, tiempos, plano y escalador ---
    if !atom::set_pixel_clock(g, t.clock_khz, crtc, ppll, encoder_id, encoder_mode) {
        return false;
    }
    if !atom::set_crtc_dtd(g, t, crtc, t.misc) {
        return false;
    }
    programar_plano(g, gpu_addr, ancho, alto, pitch);
    let _ = atom::enable_scaler(g, crtc, 0);
    // `amdgpu` llama aqui a `dce_v11_0_cursor_reset` para volver a poner su
    // cursor. Aqui el cursor lo pinta el compositor por software
    // (`kira::comp`), asi que lo que toca es lo contrario: apagar el cursor por
    // hardware si la BIOS lo dejo encendido, porque si no al cambiar de modo
    // quedaria un puntero fantasma en la posicion vieja. Es lo mismo que hace
    // la orden `cursor off`.
    let control = reg_leer(g, CUR_CONTROL) & !CUR_EN;
    let _ = reg_escribir(g, CUR_CONTROL, control);

    // --- commit: encendido y desbloqueo ---
    dpms(g, crtc, true);
    atom::update_crtc_double_buffer(g, crtc, false);

    // --- comprobacion: los tiempos del CRTC tienen que decir lo pedido ---
    esperar_ms(30);
    let mut h = reg_leer(g, CRTC_H_TOTAL) & 0x3FFF;
    let mut v = reg_leer(g, CRTC_V_TOTAL) & 0x3FFF;
    let h_esperado = (t.h_total() - 1) & 0x3FFF;
    let v_esperado = (t.v_total() - 1) & 0x3FFF;

    // Red de seguridad: si la tabla ATOM no dejo los tiempos pedidos (por
    // ejemplo, porque el interprete se desvio a mitad de la tabla), se
    // escriben los registros a mano. No se hace por sistema para no pisar lo
    // que la VBIOS dejo bien.
    if h.abs_diff(h_esperado) > 2 || v.abs_diff(v_esperado) > 2 {
        crate::println!("amd: la tabla ATOM no dejo los tiempos; se escriben a mano");
        volcar_tiempos(g, crtc);
        escribir_tiempos(g, t, crtc);
        esperar_ms(30);
        h = reg_leer(g, CRTC_H_TOTAL) & 0x3FFF;
        v = reg_leer(g, CRTC_V_TOTAL) & 0x3FFF;
    }

    let cuadros = contar_cuadros(g, 100);
    // El CRTC saca `clock / (h_total * v_total)` cuadros por segundo; como la
    // cuenta se mide en 100 ms, se esperan `clock_khz * 100 / (h*v)` cuadros.
    // Esto comprueba de verdad que el PLL (o el DTO de DisplayPort) programado
    // es el que relojea este CRTC: si el candidato no era el del conector, el
    // CRTC se queda con el reloj del modo anterior y la cuenta no sale.
    let esperados = (t.clock_khz as u64 * 100) / (t.h_total() as u64 * t.v_total() as u64);
    let cuadros_ok = (cuadros as u64) * 5 >= esperados * 3 && (cuadros as u64) * 5 <= esperados * 8;
    let bien =
        h.abs_diff(h_esperado) <= 2 && v.abs_diff(v_esperado) <= 2 && cuadros_ok && cuadros > 0;
    crate::println!(
        "amd: CRTC {} H_TOTAL {} (pedido {}), V_TOTAL {} (pedido {}), {} cuadros en 100 ms (esperados ~{}): {}",
        (crtc as u32) + 1,
        h,
        h_esperado,
        v,
        v_esperado,
        cuadros,
        esperados,
        if bien { "modo aplicado" } else { "no cuadra" }
    );
    if !bien {
        volcar_tiempos(g, crtc);
    }
    bien
}

fn caber_en_vram(g: &Gpu, pitch: u32, h: u32) -> bool {
    let v = match g.vram {
        Some(v) => v,
        None => return false,
    };
    let bytes = (pitch as u64) * (h as u64) + 0x200000;
    bytes <= v.size
}

pub fn set_mode(width: u32, height: u32) -> Option<crate::gfx::ModeInfo> {
    let f = buscar()?;

    // Solo la familia Polaris (DCE 11) y solo si la VBIOS trae ATOMBIOS. En
    // cualquier otro caso se devuelve `None` y `gfx` sigue por VBE.
    if !es_polaris(f.device) {
        return None;
    }
    if !super::atom::init(f) {
        crate::println!("amd: la VBIOS no trae ATOMBIOS; sin modesetting por el driver.");
        return None;
    }
    if width < 320 || height < 200 || width > 4096 || height > 2160 {
        return None;
    }

    let bares = crate::pci::medir_bares(f);
    let dentro_del_mapa = |b: &Bar| {
        b.base != 0
            && b.base < 0x1_0000_0000
            && b.base
                .checked_add(b.size)
                .is_some_and(|fin| fin <= 0x1_0000_0000)
    };
    let registros = bares[5]
        .or(bares[2])
        .filter(|b| !b.es_io && dentro_del_mapa(b))?;
    let vram = bares[0].filter(|b| !b.es_io && dentro_del_mapa(b))?;
    let g = Gpu {
        registros,
        vram: Some(vram),
    };

    let pitch = width * 4;
    if !caber_en_vram(&g, pitch, height) {
        crate::println!("amd: el modo {}x{} no cabe en la VRAM.", width, height);
        return None;
    }

    // Tiempos CVT reducido de margen y refresco nominal de 60 Hz. El modelo
    // alarga los huecos en los modos pequenos para no bajar de la frecuencia
    // de linea de la VGA (ver `Timing::cvt_rb`).
    let t = super::atom::Timing::cvt_rb(width, height, 60);
    crate::println!(
        "amd: tiempos {}x{}: h_total {}, v_total {}, reloj {} kHz, linea {} Hz.",
        width,
        height,
        t.h_total(),
        t.v_total(),
        t.clock_khz,
        (t.clock_khz as u64 * 1000) / (t.h_total() as u64)
    );

    // El lienzo va al principio de la VRAM. `direccion_gpu` traduce ese
    // desplazamiento a una direccion de la GPU (que es lo que quieren los
    // registros) tomando como referencia el plano que dejo la BIOS; la CPU lo
    // pinta por `vram.base + off`.
    let off = 0u64;
    let gpu_addr = direccion_gpu(&g, off);
    let lfb = vram.base + off;

    // El BIOS deja configurado el camino que ahora mismo saca imagen. Se mira
    // de donde toma el CRTC 1 su reloj de pixeles: si es el DTO de
    // DisplayPort, el conector (o el puente DP->VGA de la placa) va por ahi.
    // Tambien se vuelca que front-end digital esta activo, que es lo que dice
    // por que UNIPHY sale la imagen.
    let rate = reg_leer(&g, CRTC0_PIXEL_RATE_CNTL);
    let por_dp = rate & DP_DTO_ENABLE != 0;
    crate::println!(
        "amd: el CRTC 1 toma el reloj de la fuente {} (DP_DTO {}), PIXEL_RATE_CNTL=0x{:X}",
        rate & 3,
        if por_dp { "si" } else { "no" },
        rate
    );
    for i in 0..6u32 {
        let fe = reg_leer(&g, DIG_FE_CNTL + i * 0x100);
        if fe & (DIG_START | DIG_SYMCLK_FE_ON) != 0 {
            crate::println!(
                "amd: DIG{} activo: sale del CRTC {}, FE_CNTL=0x{:X}",
                i,
                (fe & 7) + 1,
                fe
            );
        }
    }

    // Candidatos (encoder, PLL, modo de encoder). El CRTC 1 puede salir por
    // cualquiera de los bloques UNIPHY; como no se detecta el conector, se
    // prueban por orden, primero la familia que parece la del BIOS, y se acepta
    // el primero cuyo reloj llegue de verdad al CRTC y cuyos tiempos cuadren.
    let dp = [
        (
            super::atom::UNIPHY,
            super::atom::DP_DTO,
            super::atom::ENCODER_MODE_DP,
        ),
        (
            super::atom::UNIPHY1,
            super::atom::DP_DTO,
            super::atom::ENCODER_MODE_DP,
        ),
        (
            super::atom::UNIPHY2,
            super::atom::DP_DTO,
            super::atom::ENCODER_MODE_DP,
        ),
    ];
    let combophy = [
        (
            super::atom::UNIPHY,
            super::atom::COMBOPHY_PLL0,
            super::atom::ENCODER_MODE_HDMI,
        ),
        (
            super::atom::UNIPHY1,
            super::atom::COMBOPHY_PLL2,
            super::atom::ENCODER_MODE_HDMI,
        ),
        (
            super::atom::UNIPHY2,
            super::atom::COMBOPHY_PLL4,
            super::atom::ENCODER_MODE_HDMI,
        ),
    ];
    let candidatos = if por_dp {
        [dp[0], dp[1], dp[2], combophy[0], combophy[1], combophy[2]]
    } else {
        [combophy[0], combophy[1], combophy[2], dp[0], dp[1], dp[2]]
    };

    let crtc: u8 = 0; // ATOM_CRTC1
    let mut aplicado = false;
    for (encoder_id, ppll, encoder_mode) in candidatos {
        crate::println!(
            "amd: modeset por ATOMBIOS en CRTC 1 (encoder 0x{:02X}, PLL {}, modo {}).",
            encoder_id,
            ppll,
            encoder_mode
        );
        if aplicar_modo(
            &g,
            &t,
            crtc,
            encoder_id,
            ppll,
            encoder_mode,
            gpu_addr,
            width,
            height,
            pitch,
        ) {
            aplicado = true;
            break;
        }
    }
    if !aplicado {
        return None;
    }

    *slot() = Some(Gpu {
        registros: g.registros,
        vram: g.vram,
    });
    // Deja constancia del modo activo, igual que hace el camino VBE: la usa la
    // pantalla azul (si el kernel falla con este modo puesto) y
    // `direccion_gpu`, que traduce direcciones comparando el plano de scanout
    // de la GPU con el framebuffer que tiene apuntado. Si no se actualizara,
    // una segunda llamada a `gui` compararia el plano nuevo con el
    // framebuffer viejo y la direccion saldria corrida.
    crate::pantalla::fijar_modo(
        lfb as usize,
        width as usize,
        height as usize,
        pitch as usize,
    );
    Some(crate::gfx::ModeInfo {
        width,
        height,
        bpp: 32,
        pitch,
        lfb: lfb as u32,
        write_combining: false,
    })
}

/// Direccion fisica (mapeada por identidad) de `off` dentro de la VRAM.
///
/// `None` si la GPU no tiene abertura de VRAM, si `off` se sale de ella o si la
/// direccion se iria por encima de los 4 GiB del mapa de identidad.
fn vram_dir(g: &Gpu, off: u64) -> Option<usize> {
    let v = g.vram?;
    if off + 4 > v.size {
        return None;
    }
    let dir = v.base + off;
    if dir + 4 > 0x1_0000_0000 {
        return None;
    }
    Some(dir as usize)
}

/// Lee una palabra de la VRAM por la abertura del BAR0.
fn vram_leer(g: &Gpu, off: u64) -> Option<u32> {
    let dir = vram_dir(g, off)?;
    Some(unsafe { core::ptr::read_volatile(dir as *const u32) })
}

/// Escribe una palabra en la VRAM por la abertura del BAR0.
fn vram_escribir(g: &Gpu, off: u64, valor: u32) -> bool {
    match vram_dir(g, off) {
        Some(dir) => {
            unsafe { core::ptr::write_volatile(dir as *mut u32, valor) };
            true
        }
        None => false,
    }
}

/// Devuelve `1`/`0` para imprimir un bit del registro de comando.
fn bit(v: u32, n: u32) -> u8 {
    ((v >> n) & 1) as u8
}
