//! ATOMBIOS: el interprete de las tablas de la VBIOS de una GPU AMD.
//!
//! La VBIOS de una tarjeta AMD no es codigo x86 nada mas: lleva ademas un
//! "firmware" propio, el ATOMBIOS, hecho de tablas de comandos con bytecode. Es
//! lo que usa el driver de verdad (amdgpu) para el *modesetting*: no se tocan
//! los registros del bloque de pantalla a mano, se llama a la tabla que la
//! propia ATI/AMD grabo en la tarjeta y es ella la que calcula los divisores del
//! PLL, ajusta los tiempos y enciende el CRTC segun el **modelo exacto** de
//! tarjeta. Es, por tanto, el camino "real" frente a recetas fijas.
//!
//! Este fichero implementa:
//!
//! 1. La carga de la imagen de la VBIOS (por el BAR de ROM del PCI, con el
//!    `0xC0000` de reserva) y el recorrido de sus cabeceras maestro hasta las
//!    tablas de comandos y de datos.
//! 2. El interprete del bytecode ATOM (los 127 opcodes, con los mismos indices
//!    de operando que usa `drivers/gpu/drm/amd/amdgpu/atom.c`).
//! 3. Los envoltorios de las tablas que hacen falta para poner un modo:
//!    `SetPixelClock`, `SetCRTC_UsingDTDTiming`, `BlankCRTC`, `EnableCRTC`,
//!    `EnableDispPowerGating`, `UpdateCRTC_DoubleBufferRegisters` y
//!    `EnableScaler`.
//! 4. La generacion de tiempos (CVT reducido de margen, "CVT-RB") para una
//!    resolucion cualquiera, ya que sin EDID no hay modelo que copiar.
//!
//! Los comentarios van sin tildes (ASCII) porque este es un fichero de driver.

use alloc::vec::Vec;

use crate::pci::{self, Funcion};
use crate::sync::SyncCell;

use super::amd::{self, Gpu};

// ---------------------------------------------------------------------------
// Formato de la VBIOS
// ---------------------------------------------------------------------------

/// Firma que empieza toda ROM de expansion en el offset 0.
const BIOS_MAGIC: u16 = 0xAA55;
/// En la VBIOS de ATI hay una cadena fija en este offset.
const ATI_MAGIC_PTR: u32 = 0x30;
const ATI_MAGIC: &[u8] = b" 761295520";
/// Puntero (u16) a la cabecera `ATOM_ROM_HEADER` dentro de la imagen.
const ROM_TABLE_PTR: u32 = 0x48;
/// La cabecera ATOM empieza por estas cuatro letras.
const ROM_MAGIC: &[u8] = b"ATOM";
const ROM_MAGIC_PTR: u32 = 4;
/// Dentro de `ATOM_ROM_HEADER`: punteros a las tablas maestras.
const ROM_CMD_PTR: u32 = 0x1E;
const ROM_DATA_PTR: u32 = 0x20;
/// Dentro de `ATOM_MASTER_DATA_TABLE`: puntero a la tabla de metodos IIO.
const DATA_IIO_PTR: u32 = 0x32;

// Cabecera de una tabla de comandos.
const CT_WS_PTR: u32 = 4;
const CT_PS_PTR: u32 = 5;
const CT_PS_MASK: u32 = 0x7F;

// El bytecode: numero de opcodes y el de fin de tabla.
const OP_CNT: u8 = 127;
const OP_EOT: u8 = 91;
/// Marcas del opcode `switch`.
const CASE_MAGIC: u8 = 0x63;
const CASE_END: u32 = 0x5A5A;

/// Profundidad maxima de `calltable` (igual que `ATOM_EXECUTE_MAX_DEPTH`).
const MAX_DEPTH: u32 = 32;
/// Palabras de trabajo (WS) de una tabla; las tablas reales usan unas pocas.
const WS_SIZE: usize = 64;
/// Tamano del "espacio de scratch" del interprete, en palabras de 32 bits.
const SCRATCH: usize = 256;

/// Longitud maxima de la imagen de VBIOS que se copia.
const ROM_MAX: usize = 0x1_0000;

// ---------------------------------------------------------------------------
// Indices de tabla (posicion en ATOM_MASTER_LIST_OF_COMMAND_TABLES)
// ---------------------------------------------------------------------------

const CMD_SETPIXELCLOCK: usize = 12;
const CMD_ENABLE_DISP_POWER_GATING: usize = 13;
const CMD_ENABLE_SCALER: usize = 33;
const CMD_BLANK_CRTC: usize = 34;
const CMD_ENABLE_CRTC: usize = 35;
const CMD_UPDATE_CRTC_DOUBLE_BUFFER: usize = 44;
const CMD_SETCRTC_DTD_TIMING: usize = 49;

// ---------------------------------------------------------------------------
// Constantes del bytecode
// ---------------------------------------------------------------------------

const ARG_REG: u32 = 0;
const ARG_PS: u32 = 1;
const ARG_WS: u32 = 2;
const ARG_FB: u32 = 3;
const ARG_ID: u32 = 4;
const ARG_IMM: u32 = 5;
const ARG_PLL: u32 = 6;
const ARG_MC: u32 = 7;

const SRC_DWORD: u32 = 0;
const SRC_BYTE0: u32 = 4;

const WS_QUOTIENT: u32 = 0x40;
const WS_REMAINDER: u32 = 0x41;
const WS_DATAPTR: u32 = 0x42;
const WS_SHIFT: u32 = 0x43;
const WS_OR_MASK: u32 = 0x44;
const WS_AND_MASK: u32 = 0x45;
const WS_FB_WINDOW: u32 = 0x46;
const WS_ATTRIBUTES: u32 = 0x47;
const WS_REGPTR: u32 = 0x48;

const IO_MM: u32 = 0;
const IO_PCI: u32 = 1;
const IO_SYSIO: u32 = 2;
const IO_IIO: u32 = 0x80;

const PORT_ATI: u32 = 0;
const PORT_PCI: u32 = 1;
const PORT_SYSIO: u32 = 2;

const COND_ABOVE: u32 = 0;
const COND_ABOVEOREQUAL: u32 = 1;
const COND_ALWAYS: u32 = 2;
const COND_BELOW: u32 = 3;
const COND_BELOWOREQUAL: u32 = 4;
const COND_EQUAL: u32 = 5;
const COND_NOTEQUAL: u32 = 6;

const IIO_NOP: u32 = 0;
const IIO_READ: u32 = 2;
const IIO_WRITE: u32 = 3;
const IIO_CLEAR: u32 = 4;
const IIO_SET: u32 = 5;
const IIO_MOVE_INDEX: u32 = 6;
const IIO_MOVE_ATTR: u32 = 7;
const IIO_MOVE_DATA: u32 = 8;
const IIO_END: u32 = 9;

/// Mascaras de los operandos; el indice es el campo `align` del byte de atributo.
const ARG_MASK: [u32; 8] = [
    0xFFFF_FFFF,
    0x0000_FFFF,
    0x00FF_FF00,
    0xFFFF_0000,
    0x0000_00FF,
    0x0000_FF00,
    0x00FF_0000,
    0xFF00_0000,
];
const ARG_SHIFT: [u32; 8] = [0, 0, 8, 16, 0, 8, 16, 24];
/// De alineamiento de destino (bits 3-5 + 6-7) a alineamiento de origen.
const DST_TO_SRC: [[u32; 4]; 8] = [
    [0, 0, 0, 0],
    [1, 2, 3, 0],
    [1, 2, 3, 0],
    [1, 2, 3, 0],
    [4, 5, 6, 7],
    [4, 5, 6, 7],
    [4, 5, 6, 7],
    [4, 5, 6, 7],
];
const DEF_DST: [u32; 8] = [0, 0, 1, 2, 0, 1, 2, 3];

/// Argumentos de los grupos de opcodes (en el orden de la tabla del original).
const GRP: [u32; 6] = [ARG_REG, ARG_PS, ARG_WS, ARG_FB, ARG_PLL, ARG_MC];
/// Condiciones de los siete saltos.
const JMP: [u32; 7] = [
    COND_ALWAYS,
    COND_EQUAL,
    COND_BELOW,
    COND_ABOVE,
    COND_BELOWOREQUAL,
    COND_ABOVEOREQUAL,
    COND_NOTEQUAL,
];

const OP_LIMIT_OPS: u32 = 500_000;

// ---------------------------------------------------------------------------
// Estado del interprete
// ---------------------------------------------------------------------------

struct Ctx {
    /// La imagen de la VBIOS copiada a RAM.
    bios: Vec<u8>,
    /// Desplazamiento de la tabla maestra de comandos.
    cmd_table: u32,
    /// Desplazamiento de la tabla maestra de datos.
    data_table: u32,
    /// Metodos IIO indexados por su numero.
    iio: [u16; 256],

    // Estado compartido entre tablas, igual que en `struct atom_context`.
    data_block: u32,
    reg_block: u32,
    fb_base: u32,
    io_attr: u32,
    divmul: [u32; 2],
    shift: u32,
    cs_equal: bool,
    cs_above: bool,
    io_mode: u32,
    /// Memoria de trabajo "FB" del propio bytecode.
    scratch: [u32; SCRATCH],
}

static CTX: SyncCell<Option<Ctx>> = SyncCell::new(None);

fn ctx() -> &'static mut Ctx {
    // Seguro: solo lo toca la tarea del nucleo, de una en una.
    let o: &'static mut Option<Ctx> = unsafe { &mut *CTX.get() };
    o.as_mut().expect("atom: contexto no inicializado")
}

/// Esta cargada la VBIOS y lista para ejecutar tablas?
pub fn disponible() -> bool {
    unsafe { (*CTX.get()).is_some() }
}

// ---------------------------------------------------------------------------
// Lectura de la imagen (con comprobacion de limites)
// ---------------------------------------------------------------------------

fn b8(off: u32) -> u32 {
    match ctx().bios.get(off as usize) {
        Some(v) => *v as u32,
        None => 0,
    }
}

fn b16(off: u32) -> u32 {
    b8(off) | (b8(off.wrapping_add(1)) << 8)
}

fn b32(off: u32) -> u32 {
    b16(off) | (b16(off.wrapping_add(2)) << 16)
}

// ---------------------------------------------------------------------------
// Carga de la VBIOS
// ---------------------------------------------------------------------------

/// Direccion base del BAR de ROM y su tamano, tal como los declara el PCI.
fn bar_rom(f: Funcion) -> Option<(u64, u64)> {
    let orig = pci::leer32(f.bus, f.dev, f.func, 0x30);
    if orig == 0 || orig == 0xFFFF_FFFF {
        return None;
    }
    // Se apaga el bit de habilitacion para poder medir: los bits de direccion
    // que la tarjeta no usa se leen como cero.
    let tamano = {
        pci::escribir32(f.bus, f.dev, f.func, 0x30, 0xFFFF_FFFE);
        let mascara = pci::leer32(f.bus, f.dev, f.func, 0x30);
        pci::escribir32(f.bus, f.dev, f.func, 0x30, orig);
        let libre = !(mascara & 0xFFFF_F800);
        (libre as u64 + 1) & 0xFFFF_F800
    };
    let base = (orig as u64) & !0x7FF;
    if base == 0 {
        return None;
    }
    Some((base, tamano.max(0x800)))
}

/// Copia la imagen de la ROM de la tarjeta.
///
/// Se prueba primero por el BAR de ROM (poniendolo en marcha y volviendolo a
/// dejar como estaba) y, si no, por la ventana clasica `0xC0000`.
fn cargar_imagen(f: Funcion) -> Option<Vec<u8>> {
    let mut imagen: Option<Vec<u8>> = None;

    if let Some((base, tamano)) = bar_rom(f) {
        if base + 4 <= 0x1_0000_0000 {
            let orig = pci::leer32(f.bus, f.dev, f.func, 0x30);
            pci::escribir32(f.bus, f.dev, f.func, 0x30, (orig & !0x7FF) | 1);
            crate::paging::sin_cache(base as usize);
            let firma = unsafe { core::ptr::read_volatile(base as *const u16) };
            if firma == BIOS_MAGIC {
                let n = tamano.min(ROM_MAX as u64) as usize;
                let mut v = Vec::with_capacity(n);
                for i in 0..n {
                    let b = unsafe { core::ptr::read_volatile((base as usize + i) as *const u8) };
                    v.push(b);
                }
                imagen = Some(v);
            }
            pci::escribir32(f.bus, f.dev, f.func, 0x30, orig);
        }
    }

    if imagen.is_none() {
        let base = 0xC_0000usize;
        let firma = unsafe { core::ptr::read_volatile(base as *const u16) };
        if firma == BIOS_MAGIC {
            let mut v = Vec::with_capacity(ROM_MAX);
            for i in 0..ROM_MAX {
                let b = unsafe { core::ptr::read_volatile((base + i) as *const u8) };
                v.push(b);
            }
            imagen = Some(v);
        }
    }

    imagen
}

/// Lee un u16 de un trozo de la imagen ya copiado a RAM.
fn b16_at(v: &[u8], off: u32) -> u32 {
    let a = v.get(off as usize).copied().unwrap_or(0) as u32;
    let b = v.get(off as usize + 1).copied().unwrap_or(0) as u32;
    a | (b << 8)
}

/// Carga y valida la VBIOS de la tarjeta. Devuelve `false` si no hay ATOMBIOS.
///
/// Solo se llama una vez; llamadas posteriores devuelven si ya estaba cargada.
pub fn init(f: Funcion) -> bool {
    if disponible() {
        return true;
    }

    let Some(bios) = cargar_imagen(f) else {
        return false;
    };

    if bios.len() < 2 || b16_at(&bios, 0) != BIOS_MAGIC as u32 {
        return false;
    }
    if bios.len() < ROM_TABLE_PTR as usize + 2 {
        return false;
    }
    if bios.get(ATI_MAGIC_PTR as usize..ATI_MAGIC_PTR as usize + ATI_MAGIC.len()) != Some(ATI_MAGIC)
    {
        return false;
    }

    let rom_table = b16_at(&bios, ROM_TABLE_PTR);
    if rom_table == 0 {
        return false;
    }
    if bios.get((rom_table + ROM_MAGIC_PTR) as usize..(rom_table + ROM_MAGIC_PTR) as usize + 4)
        != Some(ROM_MAGIC)
    {
        return false;
    }

    let cmd_table = b16_at(&bios, rom_table + ROM_CMD_PTR);
    let data_table = b16_at(&bios, rom_table + ROM_DATA_PTR);
    if cmd_table == 0 || data_table == 0 {
        return false;
    }
    // El indice 0 (ASIC_Init) tiene que existir: es la prueba de que la tabla
    // maestra de comandos es de verdad la que creemos.
    if b16_at(&bios, cmd_table + 4) == 0 {
        return false;
    }

    let mut c = Ctx {
        bios,
        cmd_table,
        data_table,
        iio: [0; 256],
        data_block: 0,
        reg_block: 0,
        fb_base: 0,
        io_attr: 0,
        divmul: [0; 2],
        shift: 0,
        cs_equal: false,
        cs_above: false,
        io_mode: IO_MM,
        scratch: [0; SCRATCH],
    };

    // La tabla de metodos IIO se indexa a partir de la maestra de datos.
    let iio_ptr = b16_at(&c.bios, data_table + DATA_IIO_PTR);
    if iio_ptr != 0 {
        let base = iio_ptr + 4;
        let n = b16_at(&c.bios, base);
        for i in 0..n.min(256) {
            let off = b16_at(&c.bios, base + 2 + i * 2);
            if off == 0 || off as usize >= c.bios.len() {
                continue;
            }
            c.iio[i as usize] = (off + 4) as u16;
        }
    }

    unsafe { *CTX.get() = Some(c) };
    true
}

/// Revisiones (`frev`, `crev`) de una tabla de comandos.
pub fn cabecera(index: usize) -> Option<(u8, u8)> {
    if !disponible() {
        return None;
    }
    let cmd_table = ctx().cmd_table;
    let base = b16(cmd_table + 4 + (index as u32) * 2);
    if base == 0 {
        return None;
    }
    Some((b8(base + 2) as u8, b8(base + 3) as u8))
}

// ---------------------------------------------------------------------------
// Lectura/escritura de un operando en PS
// ---------------------------------------------------------------------------

/// Lee el dword en el que cae el operando `idx` de la memoria de parametros.
///
/// El bytecode direcciona el espacio de parametros en **palabras de 32 bits**
/// (igual que `ctx->ps[idx]` en la referencia, con `ps` de tipo `uint32_t *`),
/// no en bytes. Guardamos PS como bytes para que los campos se puedan escribir
/// por desplazamiento, como en las cabeceras de C.
fn ps_leer(ps: &[u8], idx: u32) -> u32 {
    let off = (idx as usize) * 4;
    if off + 4 > ps.len() {
        return 0;
    }
    u32::from_le_bytes([ps[off], ps[off + 1], ps[off + 2], ps[off + 3]])
}

fn ps_escribir(ps: &mut [u8], idx: u32, val: u32) {
    let off = (idx * 4) as usize;
    if off + 4 > ps.len() {
        return;
    }
    ps[off..off + 4].copy_from_slice(&val.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Acceso indirecto (IIO) y registros
// ---------------------------------------------------------------------------

fn reg_leer_io(g: &Gpu, idx: u32) -> u32 {
    match ctx().io_mode {
        IO_MM => amd::reg_leer(g, idx),
        IO_PCI | IO_SYSIO => 0,
        m if m & 0x80 != 0 => {
            let met = (m & 0x7F) as usize;
            let base = ctx().iio[met] as u32;
            if base == 0 {
                0
            } else {
                iio_ejecutar(g, base, idx, 0)
            }
        }
        _ => 0,
    }
}

fn reg_escribir_io(g: &Gpu, idx: u32, val: u32) {
    match ctx().io_mode {
        IO_MM => {
            let _ = amd::reg_escribir(g, idx, val);
        }
        IO_PCI | IO_SYSIO => {}
        m if m & 0x80 != 0 => {
            let met = (m & 0xFF) as usize;
            let base = ctx().iio[met] as u32;
            if base != 0 {
                let _ = iio_ejecutar(g, base, idx, val);
            }
        }
        _ => {}
    }
}

/// Mascara de `n` bits bajos a uno (`n == 0` cuenta como 32, como el `>> 32` del
/// x86 que usa la referencia; Rust no admite ese desplazamiento).
fn mascara_bits(n: u32) -> u32 {
    if n >= 32 {
        0xFFFF_FFFF
    } else {
        0xFFFF_FFFFu32 >> (32 - n)
    }
}

/// Ejecuta un metodo IIO (una receta de accesos a registros).
fn iio_ejecutar(g: &Gpu, base: u32, index: u32, data: u32) -> u32 {
    let mut temp: u32 = 0xCDCD_CDCD;
    let mut b = base;
    let limite = ctx().bios.len() as u32;
    while b + 3 < limite {
        match b8(b) {
            IIO_NOP => b += 1,
            IIO_READ => {
                temp = amd::reg_leer(g, b16(b + 1));
                b += 3;
            }
            IIO_WRITE => {
                let _ = amd::reg_escribir(g, b16(b + 1), temp);
                b += 3;
            }
            IIO_CLEAR => {
                let ancho = mascara_bits(b8(b + 1)) << b8(b + 2);
                temp &= !ancho;
                b += 3;
            }
            IIO_SET => {
                let ancho = mascara_bits(b8(b + 1)) << b8(b + 2);
                temp |= ancho;
                b += 3;
            }
            IIO_MOVE_INDEX => {
                let ancho = mascara_bits(b8(b + 1)) << b8(b + 3);
                let valor = ((index >> b8(b + 2)) & mascara_bits(b8(b + 1))) << b8(b + 3);
                temp = (temp & !ancho) | valor;
                b += 4;
            }
            IIO_MOVE_ATTR => {
                let ancho = mascara_bits(b8(b + 1)) << b8(b + 3);
                let valor = ((ctx().io_attr >> b8(b + 2)) & mascara_bits(b8(b + 1))) << b8(b + 3);
                temp = (temp & !ancho) | valor;
                b += 4;
            }
            IIO_MOVE_DATA => {
                let ancho = mascara_bits(b8(b + 1)) << b8(b + 3);
                let valor = ((data >> b8(b + 2)) & mascara_bits(b8(b + 1))) << b8(b + 3);
                temp = (temp & !ancho) | valor;
                b += 4;
            }
            IIO_END => return temp,
            _ => return 0,
        }
        if b8(b) == IIO_END {
            return temp;
        }
    }
    temp
}

// ---------------------------------------------------------------------------
// Interprete
// ---------------------------------------------------------------------------

/// Valor de un operando de origen: el valor recortado (`val`) y el valor crudo
/// de 32 bits (`raw`), que es el que hace falta para volver a escribir el resto
/// de los bits sin tocarlos.
#[derive(Clone, Copy)]
struct Src {
    val: u32,
    raw: u32,
}

struct Frame<'a> {
    ps: &'a mut [u8],
    /// Desplazamiento de PS en palabras, para `calltable`.
    ps_shift: u32,
    ws: [u32; WS_SIZE],
    ws_size: u32,
    start: u32,
    abort: bool,
    salto_igual: u32,
    repeticiones: u32,
    ops: u32,
    depth: u32,
}

fn leer8(ptr: &mut u32) -> u32 {
    let v = b8(*ptr);
    *ptr = ptr.wrapping_add(1);
    v
}

fn leer16(ptr: &mut u32) -> u32 {
    let v = b16(*ptr);
    *ptr = ptr.wrapping_add(2);
    v
}

fn leer_directo(align: u32, ptr: &mut u32) -> u32 {
    match align {
        SRC_DWORD => {
            let v = b32(*ptr);
            *ptr = ptr.wrapping_add(4);
            v
        }
        1 | 2 | 3 => {
            let v = b16(*ptr);
            *ptr = ptr.wrapping_add(2);
            v
        }
        _ => {
            let v = b8(*ptr);
            *ptr = ptr.wrapping_add(1);
            v
        }
    }
}

fn saltar_directo(align: u32, ptr: &mut u32) {
    match align {
        SRC_DWORD => *ptr = ptr.wrapping_add(4),
        1 | 2 | 3 => *ptr = ptr.wrapping_add(2),
        _ => *ptr = ptr.wrapping_add(1),
    }
}

fn get_src(g: &Gpu, f: &mut Frame, attr: u32, ptr: &mut u32) -> Src {
    let arg = attr & 7;
    let align = (attr >> 3) & 7;
    if arg == ARG_IMM {
        let val = leer_directo(align, ptr);
        return Src { val, raw: val };
    }

    let mut val: u32 = 0xCDCD_CDCD;
    match arg {
        ARG_REG => {
            let idx = leer16(ptr) + ctx().reg_block;
            val = reg_leer_io(g, idx);
        }
        ARG_PS => {
            let idx = leer8(ptr);
            val = ps_leer(f.ps, idx);
        }
        ARG_WS => {
            let idx = leer8(ptr);
            val = match idx {
                WS_QUOTIENT => ctx().divmul[0],
                WS_REMAINDER => ctx().divmul[1],
                WS_DATAPTR => ctx().data_block,
                WS_SHIFT => ctx().shift,
                WS_OR_MASK => 1u32 << (ctx().shift & 31),
                WS_AND_MASK => !(1u32 << (ctx().shift & 31)),
                WS_FB_WINDOW => ctx().fb_base,
                WS_ATTRIBUTES => ctx().io_attr,
                WS_REGPTR => ctx().reg_block,
                _ => {
                    if idx < f.ws_size {
                        f.ws[idx as usize]
                    } else {
                        0
                    }
                }
            };
        }
        ARG_ID => {
            let idx = leer16(ptr);
            val = b32(idx + ctx().data_block);
        }
        ARG_FB => {
            let idx = leer8(ptr);
            let base = (ctx().fb_base / 4) as usize;
            if base + (idx as usize) < SCRATCH {
                val = ctx().scratch[base + idx as usize];
            } else {
                val = 0;
            }
        }
        ARG_PLL => {
            let _ = leer8(ptr);
        }
        ARG_MC => {
            let _ = leer8(ptr);
        }
        _ => {}
    }

    let raw = val;
    val = (val & ARG_MASK[align as usize]) >> ARG_SHIFT[align as usize];
    Src { val, raw }
}

fn saltar_src(attr: u32, ptr: &mut u32) {
    match attr & 7 {
        ARG_REG | ARG_ID => *ptr = ptr.wrapping_add(2),
        ARG_PS | ARG_WS | ARG_FB | ARG_PLL | ARG_MC => *ptr = ptr.wrapping_add(1),
        _ => saltar_directo((attr >> 3) & 7, ptr),
    }
}

fn get_dst(g: &Gpu, f: &mut Frame, arg: u32, attr: u32, ptr: &mut u32) -> Src {
    let align = DST_TO_SRC[((attr >> 3) & 7) as usize][((attr >> 6) & 3) as usize];
    get_src(g, f, arg | (align << 3), ptr)
}

fn saltar_dst(arg: u32, attr: u32, ptr: &mut u32) {
    let align = DST_TO_SRC[((attr >> 3) & 7) as usize][((attr >> 6) & 3) as usize];
    saltar_src(arg | (align << 3), ptr);
}

fn put_dst(g: &Gpu, f: &mut Frame, arg: u32, attr: u32, ptr: &mut u32, val: u32, saved: u32) {
    let align = DST_TO_SRC[((attr >> 3) & 7) as usize][((attr >> 6) & 3) as usize];
    let mascara = ARG_MASK[align as usize];
    let mut v = val << ARG_SHIFT[align as usize];
    v &= mascara;
    v |= saved & !mascara;

    match arg {
        ARG_REG => {
            let idx = leer16(ptr) + ctx().reg_block;
            reg_escribir_io(g, idx, v);
        }
        ARG_PS => {
            let idx = leer8(ptr);
            ps_escribir(f.ps, idx, v);
        }
        ARG_WS => {
            let idx = leer8(ptr);
            match idx {
                WS_QUOTIENT => ctx().divmul[0] = v,
                WS_REMAINDER => ctx().divmul[1] = v,
                WS_DATAPTR => ctx().data_block = v,
                WS_SHIFT => ctx().shift = v,
                WS_OR_MASK | WS_AND_MASK => {}
                WS_FB_WINDOW => ctx().fb_base = v,
                WS_ATTRIBUTES => ctx().io_attr = v,
                WS_REGPTR => ctx().reg_block = v,
                _ => {
                    if idx < f.ws_size {
                        f.ws[idx as usize] = v;
                    }
                }
            }
        }
        ARG_FB => {
            let idx = leer8(ptr);
            let base = (ctx().fb_base / 4) as usize;
            if base + (idx as usize) < SCRATCH {
                ctx().scratch[base + idx as usize] = v;
            }
        }
        ARG_PLL | ARG_MC => {
            let _ = leer8(ptr);
        }
        _ => {
            let _ = leer8(ptr);
        }
    }
}

// --- operaciones ---

fn op_move(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32) {
    let attr = leer8(ptr);
    let mut dptr = *ptr;
    let saved;
    if ((attr >> 3) & 7) != SRC_DWORD {
        saved = get_dst(g, f, arg, attr, ptr).raw;
    } else {
        saltar_dst(arg, attr, ptr);
        saved = 0xCDCD_CDCD;
    }
    let src = get_src(g, f, attr, ptr);
    put_dst(g, f, arg, attr, &mut dptr, src.val, saved);
}

#[derive(Clone, Copy)]
enum Bin {
    And,
    Or,
    Add,
    Sub,
    Xor,
}

fn op_bin(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32, op: Bin) {
    let attr = leer8(ptr);
    let mut dptr = *ptr;
    let d = get_dst(g, f, arg, attr, ptr);
    let s = get_src(g, f, attr, ptr);
    let dst = match op {
        Bin::And => d.val & s.val,
        Bin::Or => d.val | s.val,
        Bin::Add => d.val.wrapping_add(s.val),
        Bin::Sub => d.val.wrapping_sub(s.val),
        Bin::Xor => d.val ^ s.val,
    };
    put_dst(g, f, arg, attr, &mut dptr, dst, d.raw);
}

fn op_desplaza(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32, izquierda: bool) {
    let mut attr = leer8(ptr);
    attr &= 0x38;
    attr |= DEF_DST[(attr >> 3) as usize] << 6;
    let mut dptr = *ptr;
    let d = get_dst(g, f, arg, attr, ptr);
    // La referencia usa `atom_get_src_direct(ATOM_SRC_BYTE0)` para el
    // desplazamiento: consume **un** byte, no un dword. Leer cuatro bytes
    // desincronizaba el puntero de la tabla y solo se programaba la mitad
    // horizontal del modo (fallo visto en la RX 550 real).
    let shift = leer_directo(SRC_BYTE0, ptr);
    let dst = if izquierda {
        d.val.wrapping_shl(shift)
    } else {
        d.val.wrapping_shr(shift)
    };
    put_dst(g, f, arg, attr, &mut dptr, dst, d.raw);
}

fn op_shl_shr(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32, izquierda: bool) {
    let attr = leer8(ptr);
    let mut dptr = *ptr;
    let align = DST_TO_SRC[((attr >> 3) & 7) as usize][((attr >> 6) & 3) as usize];
    let d = get_dst(g, f, arg, attr, ptr);
    let mut dst = d.raw;
    let shift = get_src(g, f, attr, ptr).val & 31;
    dst = if izquierda {
        dst.wrapping_shl(shift)
    } else {
        dst.wrapping_shr(shift)
    };
    dst &= ARG_MASK[align as usize];
    dst >>= ARG_SHIFT[align as usize];
    put_dst(g, f, arg, attr, &mut dptr, dst, d.raw);
}

fn op_mul_div(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32, multiplica: bool) {
    let attr = leer8(ptr);
    let d = get_dst(g, f, arg, attr, ptr);
    let s = get_src(g, f, attr, ptr);
    if multiplica {
        ctx().divmul[0] = d.val.wrapping_mul(s.val);
    } else if s.val != 0 {
        ctx().divmul[0] = d.val / s.val;
        ctx().divmul[1] = d.val % s.val;
    } else {
        ctx().divmul[0] = 0;
        ctx().divmul[1] = 0;
    }
}

fn op_mul32(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32) {
    let attr = leer8(ptr);
    let d = get_dst(g, f, arg, attr, ptr);
    let s = get_src(g, f, attr, ptr);
    let prod = (d.val as u64).wrapping_mul(s.val as u64);
    ctx().divmul[0] = (prod & 0xFFFF_FFFF) as u32;
    ctx().divmul[1] = (prod >> 32) as u32;
}

fn op_div32(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32) {
    let attr = leer8(ptr);
    let d = get_dst(g, f, arg, attr, ptr);
    let s = get_src(g, f, attr, ptr);
    if s.val != 0 {
        let val64 = (d.val as u64) | ((ctx().divmul[1] as u64) << 32);
        ctx().divmul[0] = (val64 / s.val as u64) as u32;
        ctx().divmul[1] = (val64 % s.val as u64) as u32;
    } else {
        ctx().divmul[0] = 0;
        ctx().divmul[1] = 0;
    }
}

fn op_compare(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32) {
    let attr = leer8(ptr);
    let d = get_dst(g, f, arg, attr, ptr);
    let s = get_src(g, f, attr, ptr);
    ctx().cs_equal = d.val == s.val;
    ctx().cs_above = d.val > s.val;
}

fn op_test(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32) {
    let attr = leer8(ptr);
    let d = get_dst(g, f, arg, attr, ptr);
    let s = get_src(g, f, attr, ptr);
    ctx().cs_equal = (d.val & s.val) == 0;
}

fn op_clear(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32) {
    let mut attr = leer8(ptr);
    attr &= 0x38;
    attr |= DEF_DST[(attr >> 3) as usize] << 6;
    let mut dptr = *ptr;
    let d = get_dst(g, f, arg, attr, ptr);
    put_dst(g, f, arg, attr, &mut dptr, 0, d.raw);
}

fn op_mask(g: &Gpu, f: &mut Frame, ptr: &mut u32, arg: u32) {
    let attr = leer8(ptr);
    let mut dptr = *ptr;
    let d = get_dst(g, f, arg, attr, ptr);
    let mascara = leer_directo((attr >> 3) & 7, ptr);
    let s = get_src(g, f, attr, ptr);
    let dst = (d.val & mascara) | s.val;
    put_dst(g, f, arg, attr, &mut dptr, dst, d.raw);
}

fn op_set_port(f: &mut Frame, ptr: &mut u32, puerto: u32) {
    let _ = f;
    match puerto {
        PORT_ATI => {
            let p = leer16(ptr);
            ctx().io_mode = if p == 0 { IO_MM } else { IO_IIO | p };
        }
        PORT_PCI => {
            let _ = leer8(ptr);
            ctx().io_mode = IO_PCI;
        }
        PORT_SYSIO => {
            let _ = leer8(ptr);
            ctx().io_mode = IO_SYSIO;
        }
        _ => {}
    }
}

fn op_switch(g: &Gpu, f: &mut Frame, ptr: &mut u32) {
    let attr = leer8(ptr);
    let src = get_src(g, f, attr, ptr).val;
    while b16(*ptr) != CASE_END {
        if b8(*ptr) as u8 == CASE_MAGIC {
            *ptr = ptr.wrapping_add(1);
            let val = get_src(g, f, (attr & 0x38) | ARG_IMM, ptr).val;
            let target = b16(*ptr);
            if val == src {
                let destino = f.start.wrapping_add(target);
                *ptr = destino;
                return;
            }
            *ptr = ptr.wrapping_add(2);
        } else {
            return;
        }
    }
    *ptr = ptr.wrapping_add(2);
}

fn op_jump(f: &mut Frame, ptr: &mut u32, cond: u32) {
    let salta = match cond {
        COND_ABOVE => ctx().cs_above,
        COND_ABOVEOREQUAL => ctx().cs_above || ctx().cs_equal,
        COND_ALWAYS => true,
        COND_BELOW => !ctx().cs_above && !ctx().cs_equal,
        COND_BELOWOREQUAL => !ctx().cs_above,
        COND_EQUAL => ctx().cs_equal,
        COND_NOTEQUAL => !ctx().cs_equal,
        _ => false,
    };
    let target = b16(*ptr);
    *ptr = ptr.wrapping_add(2);
    if salta {
        let destino = f.start.wrapping_add(target);
        if f.salto_igual == destino {
            f.repeticiones += 1;
            if f.repeticiones > 1_000_000 {
                f.abort = true;
            }
        } else {
            f.salto_igual = destino;
            f.repeticiones = 0;
        }
        *ptr = destino;
    }
}

fn op_calltable(g: &Gpu, f: &mut Frame, ptr: &mut u32) {
    let idx = leer8(ptr) as usize;
    let cmd_table = ctx().cmd_table;
    if b16(cmd_table + 4 + 2 * idx as u32) == 0 {
        return;
    }
    let shift = (f.ps_shift * 4) as usize;
    let depth = f.depth + 1;
    let sub = if shift < f.ps.len() {
        &mut f.ps[shift..]
    } else {
        &mut f.ps[0..0]
    };
    let ok = ejecutar(g, idx, sub, depth);
    if !ok {
        f.abort = true;
    }
}

fn op_set_data_block(f: &mut Frame, ptr: &mut u32) {
    let idx = leer8(ptr);
    let data_table = ctx().data_table;
    ctx().data_block = if idx == 0 {
        0
    } else if idx == 255 {
        f.start
    } else {
        b16(data_table + 4 + 2 * idx)
    };
}

// --- despacho ---

fn ejecutar_op(g: &Gpu, f: &mut Frame, op: u8, ptr: &mut u32) {
    let o = op as u32;
    match o {
        1..=6 => op_move(g, f, ptr, GRP[(o - 1) as usize]),
        7..=12 => op_bin(g, f, ptr, GRP[(o - 7) as usize], Bin::And),
        13..=18 => op_bin(g, f, ptr, GRP[(o - 13) as usize], Bin::Or),
        19..=24 => op_desplaza(g, f, ptr, GRP[(o - 19) as usize], true),
        25..=30 => op_desplaza(g, f, ptr, GRP[(o - 25) as usize], false),
        31..=36 => op_mul_div(g, f, ptr, GRP[(o - 31) as usize], true),
        37..=42 => op_mul_div(g, f, ptr, GRP[(o - 37) as usize], false),
        43..=48 => op_bin(g, f, ptr, GRP[(o - 43) as usize], Bin::Add),
        49..=54 => op_bin(g, f, ptr, GRP[(o - 49) as usize], Bin::Sub),
        55 => op_set_port(f, ptr, PORT_ATI),
        56 => op_set_port(f, ptr, PORT_PCI),
        57 => op_set_port(f, ptr, PORT_SYSIO),
        58 => {
            let v = leer16(ptr);
            ctx().reg_block = v;
        }
        59 => {
            let attr = leer8(ptr);
            let v = get_src(g, f, attr, ptr).val;
            ctx().fb_base = v;
        }
        60..=65 => op_compare(g, f, ptr, GRP[(o - 60) as usize]),
        66 => op_switch(g, f, ptr),
        67..=73 => op_jump(f, ptr, JMP[(o - 67) as usize]),
        74..=79 => op_test(g, f, ptr, GRP[(o - 74) as usize]),
        80 => {
            let n = leer8(ptr);
            esperar_ms(n);
        }
        81 => {
            let n = leer8(ptr);
            esperar_us(n);
        }
        82 => op_calltable(g, f, ptr),
        83 => {} // repeat: no lo usan las VBIOS normales
        84..=89 => op_clear(g, f, ptr, GRP[(o - 84) as usize]),
        90 => {} // nop
        91 => {} // eot: el bucle de ejecucion lo detecta
        92..=97 => op_mask(g, f, ptr, GRP[(o - 92) as usize]),
        98 => {
            let _ = leer8(ptr);
        }
        99 => {}        // beep
        100 | 101 => {} // savereg / restorereg
        102 => op_set_data_block(f, ptr),
        103..=108 => op_bin(g, f, ptr, GRP[(o - 103) as usize], Bin::Xor),
        109..=114 => op_shl_shr(g, f, ptr, GRP[(o - 109) as usize], true),
        115..=120 => op_shl_shr(g, f, ptr, GRP[(o - 115) as usize], false),
        121 => {
            let _ = leer8(ptr);
        }
        122 => {
            let val = leer16(ptr);
            *ptr = ptr.wrapping_add(val + 2);
        }
        123 => op_mul32(g, f, ptr, ARG_PS),
        124 => op_mul32(g, f, ptr, ARG_WS),
        125 => op_div32(g, f, ptr, ARG_PS),
        126 => op_div32(g, f, ptr, ARG_WS),
        _ => {}
    }
}

fn ejecutar(g: &Gpu, index: usize, ps: &mut [u8], depth: u32) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let cmd_table = ctx().cmd_table;
    let base = b16(cmd_table + 4 + 2 * index as u32);
    if base == 0 {
        return false;
    }
    let ws = b8(base + CT_WS_PTR);
    let ps_header = (b8(base + CT_PS_PTR) & CT_PS_MASK) as usize;

    let mut f = Frame {
        ps,
        ps_shift: (ps_header / 4) as u32,
        ws: [0; WS_SIZE],
        ws_size: ws.min(WS_SIZE as u32),
        start: (base as u16) as u32,
        abort: false,
        salto_igual: 0,
        repeticiones: 0,
        ops: 0,
        depth,
    };

    let mut ptr = base + 6;
    while f.ops < OP_LIMIT_OPS {
        let op = b8(ptr) as u8;
        ptr = ptr.wrapping_add(1);
        if op == 0 || op >= OP_CNT {
            break;
        }
        ejecutar_op(g, &mut f, op, &mut ptr);
        if f.abort {
            break;
        }
        if op == OP_EOT {
            break;
        }
        f.ops += 1;
    }
    !f.abort
}

/// Ejecuta una tabla de comandos con sus parametros (`PS`), en bytes.
pub fn ejecutar_tabla(g: &Gpu, index: usize, ps: &mut [u8]) -> bool {
    if !disponible() {
        return false;
    }
    {
        let c = ctx();
        c.data_block = 0;
        c.reg_block = 0;
        c.fb_base = 0;
        c.io_attr = 0;
        c.io_mode = IO_MM;
        c.divmul = [0; 2];
        c.cs_equal = false;
        c.cs_above = false;
    }
    ejecutar(g, index, ps, 0)
}

// ---------------------------------------------------------------------------
// Esperas
// ---------------------------------------------------------------------------

fn esperar_ms(ms: u32) {
    if ms == 0 {
        return;
    }
    let inicio = crate::pit::ticks();
    let objetivo = (ms / 10).max(1) as u64;
    while crate::pit::ticks().wrapping_sub(inicio) < objetivo {
        core::hint::spin_loop();
    }
}

fn esperar_us(us: u32) {
    // Sin un microsegundo fiable, se cuenta un lazo corto: en el peor caso es
    // algo menos de lo pedido, y al BIOS le basta.
    let vueltas = (us as u64 * 20).min(2_000_000);
    for _ in 0..vueltas {
        core::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------
// Escritura de campos de los parametros (en bytes, como el struct de C)
// ---------------------------------------------------------------------------

fn poner_u8(ps: &mut [u8], off: usize, v: u8) {
    if off < ps.len() {
        ps[off] = v;
    }
}

fn poner_u16(ps: &mut [u8], off: usize, v: u16) {
    if off + 2 <= ps.len() {
        ps[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
}

fn poner_u32(ps: &mut [u8], off: usize, v: u32) {
    if off + 4 <= ps.len() {
        ps[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Envoltorios de tablas
// ---------------------------------------------------------------------------

/// PLLs del bloque de la DCE (combiophy) de Polaris. El CRTC 1 va por UNIPHY
/// (indice 0) y su PLL es el 0; UNIPHY1 usa el 2 y UNIPHY2 el 4
/// (`dce_v11_0_pick_pll`).
pub const COMBOPHY_PLL0: u8 = 20;
pub const COMBOPHY_PLL2: u8 = 22;
pub const COMBOPHY_PLL4: u8 = 24;
/// Para DisplayPort no se usa un combiophy: el reloj lo lleva el DTO del enlace.
pub const DP_DTO: u8 = 11;
/// Identificadores de encoder internos (`ENCODER_OBJECT_ID_*` de `ObjectID.h`).
pub const UNIPHY: u8 = 0x1E;
pub const UNIPHY1: u8 = 0x20;
pub const UNIPHY2: u8 = 0x21;
/// Modos de encoder (los `ATOM_ENCODER_MODE_*`).
pub const ENCODER_MODE_DP: u8 = 0;
pub const ENCODER_MODE_HDMI: u8 = 3;
/// Valor generico `ucEnable` de las tablas que no sea encender.
pub const ATOM_DISABLE: u8 = 0;

/// Programa el PLL de pixeles para `clock_khz` con el CRTC y el encoder dados.
///
/// En Polaris (revision 6 y 7 de la tabla) se pasan los divisores a cero: es la
/// propia BIOS la que los calcula ("SetPixelClock calculates the plls and ss
/// values now", en `dce_v11_0_crtc_program_pll`).
pub fn set_pixel_clock(
    g: &Gpu,
    clock_khz: u32,
    crtc: u8,
    ppll: u8,
    encoder_id: u8,
    encoder_mode: u8,
) -> bool {
    let Some((frev, crev)) = cabecera(CMD_SETPIXELCLOCK) else {
        return false;
    };
    let mut ps = [0u8; 24];

    if frev == 1 {
        match crev {
            5 => {
                poner_u8(&mut ps, 0, crtc);
                poner_u16(&mut ps, 2, (clock_khz / 10) as u16);
                poner_u8(&mut ps, 8, ppll);
                poner_u8(&mut ps, 9, encoder_id);
                poner_u8(&mut ps, 10, encoder_mode);
            }
            6 => {
                let disp = ((crtc as u32) << 24) | (clock_khz / 10);
                poner_u32(&mut ps, 0, disp);
                poner_u8(&mut ps, 8, ppll);
                poner_u8(&mut ps, 9, encoder_id);
                poner_u8(&mut ps, 10, encoder_mode);
            }
            7 => {
                poner_u32(&mut ps, 0, clock_khz * 10); // en unidades de 100 Hz
                poner_u8(&mut ps, 4, ppll);
                poner_u8(&mut ps, 5, encoder_id);
                poner_u8(&mut ps, 6, encoder_mode);
                poner_u8(&mut ps, 8, crtc);
            }
            _ => return false,
        }
    } else {
        return false;
    }

    ejecutar_tabla(g, CMD_SETPIXELCLOCK, &mut ps)
}

/// Fija los tiempos del CRTC a partir de un DTD.
pub fn set_crtc_dtd(g: &Gpu, t: &Timing, crtc: u8, misc: u16) -> bool {
    let mut ps = [0u8; 24];
    poner_u16(&mut ps, 0, t.h_display as u16);
    poner_u16(&mut ps, 2, t.h_blank as u16);
    poner_u16(&mut ps, 4, t.v_display as u16);
    poner_u16(&mut ps, 6, t.v_blank as u16);
    poner_u16(&mut ps, 8, t.h_sync_offset as u16);
    poner_u16(&mut ps, 10, t.h_sync_width as u16);
    poner_u16(&mut ps, 12, t.v_sync_offset as u16);
    poner_u16(&mut ps, 14, t.v_sync_width as u16);
    poner_u16(&mut ps, 16, misc);
    poner_u8(&mut ps, 18, 0); // h_border
    poner_u8(&mut ps, 19, 0); // v_border
    poner_u8(&mut ps, 20, crtc);
    ejecutar_tabla(g, CMD_SETCRTC_DTD_TIMING, &mut ps)
}

/// Apaga los divisores del CRTC (`UpdateCRTC_DoubleBufferRegisters`).
pub fn update_crtc_double_buffer(g: &Gpu, crtc: u8, bloquea: bool) -> bool {
    let mut ps = [0u8; 4];
    poner_u8(&mut ps, 0, crtc);
    poner_u8(&mut ps, 1, if bloquea { 1 } else { 0 });
    ejecutar_tabla(g, CMD_UPDATE_CRTC_DOUBLE_BUFFER, &mut ps)
}

/// Enciende o apaga el CRTC.
pub fn enable_crtc(g: &Gpu, crtc: u8, enable: bool) -> bool {
    let mut ps = [0u8; 4];
    poner_u8(&mut ps, 0, crtc);
    poner_u8(&mut ps, 1, if enable { 1 } else { 0 });
    ejecutar_tabla(g, CMD_ENABLE_CRTC, &mut ps)
}

/// Deja el CRTC en blanco (o lo saca del blanco).
pub fn blank_crtc(g: &Gpu, crtc: u8, blank: bool) -> bool {
    let mut ps = [0u8; 8];
    poner_u8(&mut ps, 0, crtc);
    poner_u8(&mut ps, 1, if blank { 1 } else { 0 });
    ejecutar_tabla(g, CMD_BLANK_CRTC, &mut ps)
}

/// Enciende o apaga el suministro del bloque de pantalla (`DispPowerGating`).
pub fn disp_power_gating(g: &Gpu, crtc: u8, enable: u8) -> bool {
    let mut ps = [0u8; 24];
    poner_u8(&mut ps, 0, crtc);
    poner_u8(&mut ps, 1, enable);
    ejecutar_tabla(g, CMD_ENABLE_DISP_POWER_GATING, &mut ps)
}

/// Ajusta el escalador del CRTC (0 = sin escalado).
pub fn enable_scaler(g: &Gpu, crtc: u8, modo: u8) -> bool {
    let mut ps = [0u8; 4];
    poner_u8(&mut ps, 0, crtc);
    poner_u8(&mut ps, 1, modo);
    ejecutar_tabla(g, CMD_ENABLE_SCALER, &mut ps)
}

// ---------------------------------------------------------------------------
// Tiempos (CVT reducido de margen)
// ---------------------------------------------------------------------------

/// Frecuencia de linea (horizontal) minima que se le pide al CRTC, en Hz.
///
/// Es la base de la especificacion VGA (31,5 kHz). Los monitores analogicos, y
/// en particular los CRT, no sincronizan por debajo.
const HZ_MINIMA: u32 = 31_500;

/// Reloj de pixeles minimo, en kHz.
///
/// Es el de la VGA clasica (25,175 MHz). Por debajo de el la ruta de salida
/// (el transmisor DisplayPort y su puente DP->VGA, en las placas sin RAMDAC)
/// no tiene margen util.
const RELOJ_MINIMO: u32 = 25_175;

/// Un modelo de tiempos listo para el CRTC.
#[derive(Clone, Copy)]
pub struct Timing {
    pub h_display: u32,
    /// Hueco horizontal total (front + sincronia + back).
    pub h_blank: u32,
    /// Porche frontal de sincronia horizontal.
    pub h_sync_offset: u32,
    pub h_sync_width: u32,
    pub v_display: u32,
    pub v_blank: u32,
    pub v_sync_offset: u32,
    pub v_sync_width: u32,
    /// Reloj de pixeles en kHz.
    pub clock_khz: u32,
    pub misc: u16,
}

impl Timing {
    /// Tiempos con el modelo CVT reducido de margen ("CVT-RB").
    ///
    /// En vez de repetir la tabla entera de VESA (que no acaba nunca), se usa la
    /// misma receta que aplican los monitores modernos: 160 px de hueco en
    /// horizontal y 31 lineas de hueco en vertical, que es lo que sale de la
    /// especificacion para el 1080p a 60 Hz y lo que aceptan los paneles
    /// digitales en cualquier resolucion. El reloj se elige para que el
    /// refresco pedido salga exacto.
    ///
    /// En los modos pequenos esos huecos dejan el reloj y la frecuencia de linea
    /// por debajo de la base de la VGA (320x240 a 60 Hz serian 7,8 MHz y 16 kHz
    /// de linea), y hay monitores analogicos que ahi no sincronizan; en ese caso
    /// se alargan los huecos hasta llegar a [`HZ_MINIMA`] y [`RELOJ_MINIMO`], sin
    /// tocar el refresco.
    pub fn cvt_rb(w: u32, h: u32, refresco: u32) -> Timing {
        let hz = refresco.max(1) as u64;
        // El CVT reducido fija los huecos (160 px y 31 lineas) y deja que el
        // reloj salga de ellos. Para resoluciones pequenas eso da un reloj muy
        // bajo: 320x240 a 60 Hz saldria a 7,8 MHz y 16 kHz de linea, y un
        // monitor analogico (un CRT, o la entrada VGA de un plano) no
        // sincroniza por debajo de la base VGA. Los dos minimos que se imponen
        // aqui son justo esos: la frecuencia de linea de 31,5 kHz (`hz *
        // v_total`) y el reloj clasico de 25,175 MHz. Cuando el modo se queda
        // corto se alargan los huecos, no se cambia el refresco: asi el monitor
        // recibe los 60 Hz que espera.
        let v_total = ((HZ_MINIMA as u64 + hz - 1) / hz).max(h as u64 + 31);
        let h_total = ((RELOJ_MINIMO as u64 * 1000 + v_total * hz - 1) / (v_total * hz))
            .max(w as u64 + 160) as u32;
        let h_blank = h_total - w;
        let clock_khz = (h_total as u64 * v_total as u64 * hz) / 1000;
        Timing {
            h_display: w,
            h_blank,
            h_sync_offset: 48,
            h_sync_width: 32,
            v_display: h,
            v_blank: (v_total - h as u64) as u32,
            v_sync_offset: 3,
            v_sync_width: 5,
            clock_khz: clock_khz as u32,
            // Sincronias activas por nivel alto: lo mas tolerante.
            misc: 0,
        }
    }

    pub fn h_total(&self) -> u32 {
        self.h_display + self.h_blank
    }

    pub fn v_total(&self) -> u32 {
        self.v_display + self.v_blank
    }
}
