//! ext4: el sistema de ficheros propio de rocky.
//!
//! rocky guarda sus datos en un volumen **ext4 de verdad**: el mismo formato que
//! produce `mke2fs` de Linux y que valida `e2fsck`. No es una imitacion: los
//! superbloques, los descriptores de grupo, los mapas de bits, las tablas de
//! inodos, los arboles de extents y las entradas de directorio se escriben tal y
//! como los espera el resto del mundo.
//!
//! El formato que usa rocky es deliberadamente minimo (el que `mke2fs` genera
//! para el conjunto de caracteristicas mas pequeno): bloques de 1024 B, inodos de
//! 128 B, sin diario, sin `flex_bg` ni sumas de comprobacion. Aun asi es ext4
//! autentico y queda limpio al pasarle el `e2fsck` real.
//!
//! La API publica de este modulo no ha cambiado respecto al sistema anterior:
//! el resto del nucleo (la shell y el escritorio) siguen llamando a las mismas
//! funciones (`raiz`, `crear`, `leer`, `escribir`, `listar`, ...).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::disk;
use crate::sync::SyncCell;

/// Tamano de bloque con el que rocky formatea (bytes).
pub const BLOQUE: usize = 1024;
/// Longitud maxima de un nombre (la de ext4).
pub const NOMBRE_MAX: usize = 255;

// ---------------------------------------------------------------------------
// Constantes del formato ext4.
// ---------------------------------------------------------------------------

/// El superbloque vive siempre en el desplazamiento 1024 del volumen.
/// (Se usa implícitamente: el superbloque ocupa los sectores 2 y 3.)
const _OFFSET_SB: usize = 1024;
/// Numero magico del superbloque ext2/3/4.
const MAGIA: u16 = 0xEF53;
/// Numero magico de la cabecera de un extent.
const MAGIA_EXTENT: u16 = 0xF30A;
/// Bandera del inodo que indica que `i_block` es un arbol de extents.
const FLAG_EXTENTS: u32 = 0x0008_0000;

/// El directorio guarda el tipo de fichero en cada entrada.
const FEAT_INCOMPAT_FILETYPE: u32 = 0x0000_0002;
/// El inodo usa extents.
const FEAT_INCOMPAT_EXTENTS: u32 = 0x0000_0040;
/// Numeros de bloque de 64 bits en los descriptores.
const FEAT_INCOMPAT_64BIT: u32 = 0x0000_0080;
/// Las copias de respaldo del superbloque solo estan en algunos grupos.
const FEAT_RO_SPARSE_SUPER: u32 = 0x0000_0001;
/// Ficheros de mas de 2 GiB.
const FEAT_RO_LARGE_FILE: u32 = 0x0000_0002;

/// Geometria con la que formatea rocky.
const IPG_FMT: u32 = 2048;
const BPG_FMT: u32 = 8192;
const PRIMER_BLOQUE_FMT: u32 = 1;
/// Tamano de inodo por defecto.
const ITAM_FMT: usize = 128;

/// Inodo de la raiz.
const RAIZ_INODO: u32 = 2;
/// Primer inodo utilizable (los 1..10 estan reservados).
const PRIMER_INODO: u32 = 11;
/// Inodo de `lost+found`.
const LOST_FOUND: u32 = 11;

/// Extents que caben en el propio inodo (4 x 12 + cabecera de 12 = 60).
const MAX_EXT_INODO: usize = 4;
/// Extents que caben en un bloque hoja (1024 - 12) / 12.
const MAX_EXT_HOJA: usize = 84;

// Campos del inodo (desplazamientos en bytes).
const I_MODE: usize = 0x00;
const I_SIZE: usize = 0x04;
const I_MTIME: usize = 0x10;
const I_DTIME: usize = 0x14;
const I_LINKS: usize = 0x1A;
const I_BLOCKS: usize = 0x1C;
const I_FLAGS: usize = 0x20;
const I_BLOCK: usize = 0x28; // 60 bytes: cabecera de extents + 4 extents

// Bits de modo.
const S_IFMT: u16 = 0o170000;
const S_IFDIR: u16 = 0o040000;
const MODO_DIR: u16 = 0o040755;
const MODO_FIC: u16 = 0o100644;

// Tipos de fichero en las entradas de directorio.
const FT_REG: u8 = 1;
const FT_DIR: u8 = 2;

// ---------------------------------------------------------------------------
// Lectura y escritura de enteros pequenos en un trozo de bytes.
// ---------------------------------------------------------------------------

#[inline]
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

#[inline]
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[inline]
fn poner_u16(b: &mut [u8], o: usize, x: u16) {
    b[o..o + 2].copy_from_slice(&x.to_le_bytes());
}

#[inline]
fn poner_u32(b: &mut [u8], o: usize, x: u32) {
    b[o..o + 4].copy_from_slice(&x.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Tipos publicos.
// ---------------------------------------------------------------------------

/// Tipo de una entrada.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Fichero,
    Directorio,
}

impl Tipo {
    /// Bits de modo (tipo + permisos) del inodo.
    fn modo(self) -> u16 {
        match self {
            Tipo::Fichero => MODO_FIC,
            Tipo::Directorio => MODO_DIR,
        }
    }

    /// Tipo de fichero tal y como se guarda en una entrada de directorio.
    fn ftype(self) -> u8 {
        match self {
            Tipo::Fichero => FT_REG,
            Tipo::Directorio => FT_DIR,
        }
    }

    /// Traduce el tipo de una entrada de directorio.
    fn desde_ftype(f: u8) -> Tipo {
        if f == FT_DIR {
            Tipo::Directorio
        } else {
            Tipo::Fichero
        }
    }
}

/// Errores del sistema de ficheros.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    NoFormateado,
    NoExiste,
    YaExiste,
    SinEspacio,
    NoEsDirectorio,
    EsDirectorio,
    NoVacio,
    NombreInvalido,
    DemasiadoLargo,
    Disco,
}

impl Error {
    /// Mensaje en castellano para mostrar al usuario.
    pub fn mensaje(self) -> &'static str {
        match self {
            Error::NoFormateado => "el volumen no esta formateado",
            Error::NoExiste => "no existe esa ruta",
            Error::YaExiste => "ya existe una entrada con ese nombre",
            Error::SinEspacio => "no queda espacio en el disco",
            Error::NoEsDirectorio => "esa ruta no es un directorio",
            Error::EsDirectorio => "esa ruta es un directorio",
            Error::NoVacio => "el directorio no esta vacio",
            Error::NombreInvalido => "nombre invalido",
            Error::DemasiadoLargo => "el nombre es demasiado largo",
            Error::Disco => "error de disco",
        }
    }
}

/// Una entrada lista para mostrar (resultado de listar un directorio).
pub struct Entrada {
    pub registro: u32,
    pub tipo: Tipo,
    pub nombre: String,
    pub tam: u32,
    /// Segundos Unix del `i_mtime` ext4.
    pub modificado: u32,
}

/// Datos de un grupo de bloques que necesitamos tener a mano.
#[derive(Clone, Copy)]
struct GrupoInfo {
    /// Bloque donde empieza el mapa de bloques del grupo.
    bm_bloques: u64,
    /// Bloque donde empieza el mapa de inodos del grupo.
    bm_inodos: u64,
    /// Bloque donde empieza la tabla de inodos del grupo.
    tabla_inodos: u64,
    /// Cuantos bloques tiene este grupo (el ultimo puede tener menos).
    bloques: u32,
}

/// Estado de un volumen ext4 montado.
struct Volumen {
    /// Tamano de bloque en bytes.
    bs: usize,
    /// Sectores de 512 B por bloque.
    spb: u32,
    /// Bloques totales del volumen.
    bloques: u64,
    /// Inodos totales.
    inodos: u32,
    /// Bloques por grupo (el ultimo puede tener menos).
    bpg: u32,
    /// Inodos por grupo.
    ipg: u32,
    /// Tamano de un inodo en bytes.
    itam: usize,
    /// Tamano de un descriptor de grupo en bytes.
    dtam: usize,
    /// Numero de grupos.
    grupos: u32,
    /// Primer bloque con datos (1 si el bloque mide 1024, 0 si no).
    primer_bloque: u32,
    /// Caracteristicas incompatibles del superbloque.
    feat_incompat: u32,
    /// Caracteristicas de solo lectura del superbloque.
    feat_ro: u32,
    /// Etiqueta del volumen.
    etiqueta: [u8; 16],
    /// Tabla de descriptores de grupo completa.
    desc: Vec<u8>,
    /// Datos derivados de cada grupo.
    info: Vec<GrupoInfo>,
    /// Mapas de bloques de todos los grupos, uno tras otro.
    bm_bloques: Vec<u8>,
    /// Mapas de inodos de todos los grupos, uno tras otro.
    bm_inodos: Vec<u8>,
    /// Bloques libres (se recalcula al volcar).
    libres_bloques: u64,
    /// Inodos libres (se recalcula al volcar).
    libres_inodos: u32,
}

static VOL: SyncCell<Option<Volumen>> = SyncCell::new(None);

fn vol() -> &'static mut Volumen {
    unsafe { (*VOL.get()).as_mut().expect("ext4: volumen no montado") }
}

fn hay_volumen() -> bool {
    unsafe { (*VOL.get()).is_some() }
}

// ---------------------------------------------------------------------------
// Acceso de bajo nivel al disco (siempre por sectores de 512 bytes).
// ---------------------------------------------------------------------------

/// Lee `datos.len()` bytes (multiplo de 512) a partir del sector `lba`.
fn leer_bytes(lba: u32, datos: &mut [u8]) {
    let mut i = 0;
    while i + 512 <= datos.len() {
        let mut s = [0u8; 512];
        if !crate::disk_srv::pedir(
            crate::disk_srv::Tipo::Leer,
            lba + (i / 512) as u32,
            &mut s,
            512,
        ) {
            return;
        }
        datos[i..i + 512].copy_from_slice(&s);
        i += 512;
    }
}

/// Escribe `datos.len()` bytes (multiplo de 512) a partir del sector `lba`.
fn escribir_bytes(lba: u32, datos: &[u8]) {
    let mut i = 0;
    while i + 512 <= datos.len() {
        let mut s = [0u8; 512];
        s.copy_from_slice(&datos[i..i + 512]);
        let _ = crate::disk_srv::pedir(
            crate::disk_srv::Tipo::Escribir,
            lba + (i / 512) as u32,
            &mut s,
            512,
        );
        i += 512;
    }
}

/// Lee un bloque de disco con la geometria dada (sin volumen montado).
fn leer_bloque_raw(bs: usize, spb: u32, b: u32) -> Vec<u8> {
    let mut out = vec![0u8; bs];
    leer_bloque_en(bs, spb, b, &mut out);
    out
}

/// Lee un bloque en un búfer reutilizable, evitando crear un `Vec` por lectura.
fn leer_bloque_en(bs: usize, spb: u32, b: u32, out: &mut [u8]) {
    let longitud = bs.min(out.len());
    out[..longitud].fill(0);
    let base = b as u64 * spb as u64;
    for i in 0..spb as usize {
        let inicio = i * 512;
        if inicio + 512 > bs || inicio + 512 > out.len() {
            break;
        }
        let mut s = [0u8; 512];
        if !crate::disk_srv::pedir(
            crate::disk_srv::Tipo::Leer,
            (base + i as u64) as u32,
            &mut s,
            512,
        ) {
            break;
        }
        out[inicio..inicio + 512].copy_from_slice(&s);
    }
}

/// Escribe un bloque con la geometria dada (sin volumen montado).
fn escribir_bloque_raw(_bs: usize, spb: u32, b: u32, datos: &[u8]) {
    let base = b as u64 * spb as u64;
    for i in 0..spb as usize {
        let mut s = [0u8; 512];
        s.copy_from_slice(&datos[i * 512..i * 512 + 512]);
        let _ = crate::disk_srv::pedir(
            crate::disk_srv::Tipo::Escribir,
            (base + i as u64) as u32,
            &mut s,
            512,
        );
    }
}

/// Lee el bloque `b` del volumen montado.
fn leer_bloque(v: &Volumen, b: u32) -> Vec<u8> {
    leer_bloque_raw(v.bs, v.spb, b)
}

/// Escribe el bloque `b` del volumen montado.
fn escribir_bloque(v: &Volumen, b: u32, datos: &[u8]) {
    escribir_bloque_raw(v.bs, v.spb, b, datos);
}

// ---------------------------------------------------------------------------
// Descriptores de grupo.
// ---------------------------------------------------------------------------

#[inline]
fn desc_off(v: &Volumen, g: u32) -> usize {
    g as usize * v.dtam
}

/// Lee un puntero (32 o 64 bits) del descriptor `g` en el campo `o` (0, 4 u 8).
fn desc_u64(desc: &[u8], o: usize, dtam: usize, campo: usize) -> u64 {
    let lo = u32le(desc, o + campo) as u64;
    if dtam >= 64 {
        lo | ((u32le(desc, o + campo + 0x20) as u64) << 32)
    } else {
        lo
    }
}

/// Bloques libres del grupo `g` segun el descriptor.
#[allow(dead_code)]
fn desc_libre_bloques(v: &Volumen, g: u32) -> u32 {
    let o = desc_off(v, g);
    let lo = u16le(&v.desc, o + 0x0C) as u32;
    if v.dtam >= 64 {
        lo | ((u16le(&v.desc, o + 0x2C) as u32) << 16)
    } else {
        lo
    }
}

/// Inodos libres del grupo `g` segun el descriptor.
#[allow(dead_code)]
fn desc_libre_inodos(v: &Volumen, g: u32) -> u32 {
    let o = desc_off(v, g);
    let lo = u16le(&v.desc, o + 0x0E) as u32;
    if v.dtam >= 64 {
        lo | ((u16le(&v.desc, o + 0x2E) as u32) << 16)
    } else {
        lo
    }
}

/// Directorios del grupo `g` segun el descriptor.
fn desc_dirs(v: &Volumen, g: u32) -> u32 {
    let o = desc_off(v, g);
    let lo = u16le(&v.desc, o + 0x10) as u32;
    if v.dtam >= 64 {
        lo | ((u16le(&v.desc, o + 0x30) as u32) << 16)
    } else {
        lo
    }
}

/// Ajusta el contador de directorios del grupo al que pertenece `ino`.
fn ajustar_dirs(v: &mut Volumen, ino: u32, delta: i32) {
    let g = (ino - 1) / v.ipg;
    let o = desc_off(v, g);
    let actual = desc_dirs(v, g) as i32;
    let nuevo = (actual + delta).max(0) as u32;
    poner_u16(&mut v.desc, o + 0x10, nuevo as u16);
    if v.dtam >= 64 {
        poner_u16(&mut v.desc, o + 0x30, (nuevo >> 16) as u16);
    }
}

// ---------------------------------------------------------------------------
// Inodos.
// ---------------------------------------------------------------------------

/// Desplazamiento en bytes del inodo `ino` dentro del volumen.
fn inodo_off(v: &Volumen, ino: u32) -> u64 {
    let g = (ino - 1) / v.ipg;
    let i = (ino - 1) % v.ipg;
    v.info[g as usize].tabla_inodos * v.bs as u64 + i as u64 * v.itam as u64
}

/// Lee los bytes de un inodo.
fn leer_inodo(v: &Volumen, ino: u32) -> Vec<u8> {
    let off = inodo_off(v, ino);
    let blk = (off / v.bs as u64) as u32;
    let dentro = (off % v.bs as u64) as usize;
    let b = leer_bloque(v, blk);
    b[dentro..dentro + v.itam].to_vec()
}

/// Escribe los bytes de un inodo (lee-modifica-escribe su bloque).
fn escribir_inodo(v: &Volumen, ino: u32, ib: &[u8]) {
    let off = inodo_off(v, ino);
    let blk = (off / v.bs as u64) as u32;
    let dentro = (off % v.bs as u64) as usize;
    let mut b = leer_bloque(v, blk);
    b[dentro..dentro + v.itam].copy_from_slice(&ib[..v.itam]);
    poner_u32(
        &mut b,
        dentro + I_MTIME,
        crate::cmos::unix_time().clamp(0, u32::MAX as i64) as u32,
    );
    escribir_bloque(v, blk, &b);
}

/// Comprueba si un inodo corresponde a un directorio.
fn es_dir(ib: &[u8]) -> bool {
    (u16le(ib, I_MODE) & S_IFMT) == S_IFDIR
}

// ---------------------------------------------------------------------------
// Mapas de bits (se tienen copiados en memoria).
// ---------------------------------------------------------------------------

#[inline]
fn bit_bloque(v: &Volumen, g: u32, i: u32) -> bool {
    let o = g as usize * v.bs + (i / 8) as usize;
    v.bm_bloques[o] & (1u8 << (i % 8)) != 0
}

#[inline]
fn fijar_bit_bloque(v: &mut Volumen, g: u32, i: u32, usado: bool) {
    let o = g as usize * v.bs + (i / 8) as usize;
    let m = 1u8 << (i % 8);
    if usado {
        v.bm_bloques[o] |= m;
    } else {
        v.bm_bloques[o] &= !m;
    }
}

#[inline]
fn bit_inodo(v: &Volumen, g: u32, i: u32) -> bool {
    let o = g as usize * v.bs + (i / 8) as usize;
    v.bm_inodos[o] & (1u8 << (i % 8)) != 0
}

#[inline]
fn fijar_bit_inodo(v: &mut Volumen, g: u32, i: u32, usado: bool) {
    let o = g as usize * v.bs + (i / 8) as usize;
    let m = 1u8 << (i % 8);
    if usado {
        v.bm_inodos[o] |= m;
    } else {
        v.bm_inodos[o] &= !m;
    }
}

/// Marca/desmarca un bloque por su numero absoluto.
fn liberar_bloque(v: &mut Volumen, b: u32) {
    if b < v.primer_bloque {
        return;
    }
    let d = b - v.primer_bloque;
    let g = d / v.bpg;
    let i = d % v.bpg;
    if g < v.grupos && i < v.info[g as usize].bloques {
        fijar_bit_bloque(v, g, i, false);
    }
}

/// Reserva el primer bloque libre que encuentra.
fn asignar_bloque(v: &mut Volumen) -> Option<u32> {
    for g in 0..v.grupos {
        let nb = v.info[g as usize].bloques;
        for i in 0..nb {
            if !bit_bloque(v, g, i) {
                fijar_bit_bloque(v, g, i, true);
                return Some(v.primer_bloque + g * v.bpg + i);
            }
        }
    }
    None
}

/// Reserva el primer inodo libre que encuentra.
fn asignar_inodo(v: &mut Volumen) -> Option<u32> {
    for g in 0..v.grupos {
        for i in 0..v.ipg {
            let n = g * v.ipg + i + 1;
            if n > v.inodos {
                break;
            }
            if !bit_inodo(v, g, i) {
                fijar_bit_inodo(v, g, i, true);
                return Some(n);
            }
        }
    }
    None
}

/// Libera un inodo.
fn liberar_inodo(v: &mut Volumen, ino: u32) {
    let g = (ino - 1) / v.ipg;
    let i = (ino - 1) % v.ipg;
    fijar_bit_inodo(v, g, i, false);
}

// ---------------------------------------------------------------------------
// Extents.
//
// Un inodo con la bandera EXTENTS guarda en `i_block` una cabecera de 12 bytes
// seguida de hasta 4 extents de 12 bytes. Cada extent es un tramo contiguo:
// bloque logico inicial, longitud y bloque fisico inicial. Si hacen falta mas de
// 4 tramos, la cabecera pasa a profundidad 1 y apunta a bloques hoja con 84
// extents cada uno.
// ---------------------------------------------------------------------------

fn es_extents(ib: &[u8]) -> bool {
    u16le(ib, I_BLOCK) == MAGIA_EXTENT
}

/// Cuantos bloques logicos cubre un inodo (maximo logico + longitud).
fn bloques_asignados(v: &Volumen, ib: &[u8]) -> u32 {
    let mut max = 0u32;
    con_extents(v, ib, |lg, _fi, ln| {
        let fin = lg.saturating_add(ln);
        if fin > max {
            max = fin;
        }
    });
    max
}

/// Recorre los extents de un inodo en el orden en que estan guardados.
fn con_extents<F: FnMut(u32, u32, u32)>(v: &Volumen, ib: &[u8], mut f: F) {
    let h = &ib[I_BLOCK..I_BLOCK + 60];
    if u16le(h, 0) != MAGIA_EXTENT {
        return;
    }
    let depth = u16le(h, 6);
    let ent = (u16le(h, 2) as usize).min(MAX_EXT_INODO);
    if depth == 0 {
        for k in 0..ent {
            let o = 12 + k * 12;
            f(u32le(h, o), u32le(h, o + 8), u16le(h, o + 4) as u32);
        }
    } else if depth == 1 {
        for k in 0..ent {
            let o = 12 + k * 12;
            let hoja = u32le(h, o + 4);
            let b = leer_bloque(v, hoja);
            if u16le(&b, 0) != MAGIA_EXTENT {
                continue;
            }
            let he = (u16le(&b, 2) as usize).min(MAX_EXT_HOJA);
            for j in 0..he {
                let p = 12 + j * 12;
                f(u32le(&b, p), u32le(&b, p + 8), u16le(&b, p + 4) as u32);
            }
        }
    }
}

/// Busca en una cabecera de extents el bloque fisico del bloque logico `k`.
fn buscar_ext(h: &[u8], k: u32) -> Option<u32> {
    let maxe = (h.len().saturating_sub(12)) / 12;
    let ent = (u16le(h, 2) as usize).min(maxe);
    for j in 0..ent {
        let o = 12 + j * 12;
        let eb = u32le(h, o);
        let el = u16le(h, o + 4) as u32;
        if k >= eb && k < eb + el {
            return Some(u32le(h, o + 8) + (k - eb));
        }
    }
    None
}

/// Mapea un bloque logico a fisico usando extents (profundidad 0 o 1).
fn ext_fisico(v: &Volumen, ib: &[u8], k: u32) -> Option<u32> {
    let h = &ib[I_BLOCK..I_BLOCK + 60];
    if u16le(h, 0) != MAGIA_EXTENT {
        return None;
    }
    let depth = u16le(h, 6);
    if depth == 0 {
        return buscar_ext(h, k);
    }
    if depth == 1 {
        let ent = (u16le(h, 2) as usize).min(MAX_EXT_INODO);
        let mut hoja = 0u32;
        let mut mejor = 0u32;
        let mut hay = false;
        for j in 0..ent {
            let o = 12 + j * 12;
            let eb = u32le(h, o);
            if eb <= k && (!hay || eb >= mejor) {
                hoja = u32le(h, o + 4);
                mejor = eb;
                hay = true;
            }
        }
        if !hay {
            return None;
        }
        let b = leer_bloque(v, hoja);
        if u16le(&b, 0) != MAGIA_EXTENT {
            return None;
        }
        return buscar_ext(&b, k);
    }
    None
}

/// Mapea un bloque logico a fisico usando el esquema clasico (sin extents).
fn clasico_fisico(v: &Volumen, ib: &[u8], k: u32) -> Option<u32> {
    let p = &ib[I_BLOCK..I_BLOCK + 60];
    let por_bloque = (v.bs / 4) as u32;
    if k < 12 {
        let x = u32le(p, k as usize * 4);
        return if x != 0 { Some(x) } else { None };
    }
    let mut k = k - 12;
    if k < por_bloque {
        let ind = u32le(p, 12 * 4);
        if ind == 0 {
            return None;
        }
        let b = leer_bloque(v, ind);
        let x = u32le(&b, k as usize * 4);
        return if x != 0 { Some(x) } else { None };
    }
    k -= por_bloque;
    if k < por_bloque * por_bloque {
        let dind = u32le(p, 13 * 4);
        if dind == 0 {
            return None;
        }
        let b1 = leer_bloque(v, dind);
        let i1 = (k / por_bloque) as usize;
        let i2 = (k % por_bloque) as usize;
        let x = u32le(&b1, i1 * 4);
        if x == 0 {
            return None;
        }
        let b2 = leer_bloque(v, x);
        let y = u32le(&b2, i2 * 4);
        return if y != 0 { Some(y) } else { None };
    }
    None
}

/// Mapea un bloque logico a fisico, sea con extents o con el esquema clasico.
fn fisico_de(v: &Volumen, ib: &[u8], k: u32) -> Option<u32> {
    if es_extents(ib) {
        ext_fisico(v, ib, k)
    } else {
        clasico_fisico(v, ib, k)
    }
}

/// Anexa un bloque a un bloque hoja. Devuelve `true` si cabia.
fn anexar_en_hoja(v: &mut Volumen, hoja: u32, logico: u32, fisico: u32) -> Result<bool, Error> {
    let mut b = leer_bloque(v, hoja);
    if u16le(&b, 0) != MAGIA_EXTENT {
        return Ok(false);
    }
    let ent = u16le(&b, 2) as usize;
    if ent > 0 {
        let o = 12 + (ent - 1) * 12;
        let lb = u32le(&b, o);
        let ln = u16le(&b, o + 4) as u32;
        let fb = u32le(&b, o + 8);
        if lb + ln == logico && fb + ln == fisico && ln < 32768 {
            poner_u16(&mut b, o + 4, (ln + 1) as u16);
            escribir_bloque(v, hoja, &b);
            return Ok(true);
        }
    }
    if ent < MAX_EXT_HOJA {
        let o = 12 + ent * 12;
        poner_u32(&mut b, o, logico);
        poner_u16(&mut b, o + 4, 1);
        poner_u16(&mut b, o + 6, 0);
        poner_u32(&mut b, o + 8, fisico);
        poner_u16(&mut b, 2, (ent + 1) as u16);
        escribir_bloque(v, hoja, &b);
        return Ok(true);
    }
    Ok(false)
}

/// Anexa un bloque fisico al final del arbol de extents del inodo `ib`.
fn anexar_extent(v: &mut Volumen, ib: &mut [u8], logico: u32, fisico: u32) -> Result<(), Error> {
    let depth = u16le(&ib[I_BLOCK..], 6);
    if depth == 0 {
        let ent = (u16le(&ib[I_BLOCK..], 2) as usize).min(MAX_EXT_INODO);
        // Intenta extender el ultimo extent si viene a continuacion.
        if ent > 0 {
            let o = I_BLOCK + 12 + (ent - 1) * 12;
            let lb = u32le(ib, o);
            let ln = u16le(ib, o + 4) as u32;
            let fb = u32le(ib, o + 8);
            if lb + ln == logico && fb + ln == fisico && ln < 32768 {
                poner_u16(ib, o + 4, (ln + 1) as u16);
                return Ok(());
            }
        }
        if ent < MAX_EXT_INODO {
            let o = I_BLOCK + 12 + ent * 12;
            poner_u32(ib, o, logico);
            poner_u16(ib, o + 4, 1);
            poner_u16(ib, o + 6, 0);
            poner_u32(ib, o + 8, fisico);
            poner_u16(ib, I_BLOCK + 2, (ent + 1) as u16);
            return Ok(());
        }
        // Promociona a profundidad 1: los 4 extents pasan a un bloque hoja.
        let hoja = asignar_bloque(v).ok_or(Error::SinEspacio)?;
        let mut hb = vec![0u8; v.bs];
        poner_u16(&mut hb, 0, MAGIA_EXTENT);
        poner_u16(&mut hb, 2, MAX_EXT_INODO as u16);
        poner_u16(&mut hb, 4, MAX_EXT_HOJA as u16);
        poner_u16(&mut hb, 6, 0);
        hb[12..12 + 48].copy_from_slice(&ib[I_BLOCK + 12..I_BLOCK + 60]);
        escribir_bloque(v, hoja, &hb);
        let primero = u32le(&hb, 12);
        poner_u16(ib, I_BLOCK + 2, 1);
        poner_u16(ib, I_BLOCK + 6, 1);
        poner_u32(ib, I_BLOCK + 12, primero);
        poner_u32(ib, I_BLOCK + 16, hoja);
        poner_u16(ib, I_BLOCK + 20, 0);
        for k in 24..60 {
            ib[I_BLOCK + k] = 0;
        }
        anexar_en_hoja(v, hoja, logico, fisico)?;
        return Ok(());
    }
    if depth == 1 {
        let ent = (u16le(&ib[I_BLOCK..], 2) as usize).min(MAX_EXT_INODO);
        let o = I_BLOCK + 12 + (ent - 1) * 12;
        let hoja = u32le(ib, o + 4);
        if anexar_en_hoja(v, hoja, logico, fisico)? {
            return Ok(());
        }
        if ent >= MAX_EXT_INODO {
            return Err(Error::SinEspacio);
        }
        let nh = asignar_bloque(v).ok_or(Error::SinEspacio)?;
        let mut nb = vec![0u8; v.bs];
        poner_u16(&mut nb, 0, MAGIA_EXTENT);
        poner_u16(&mut nb, 2, 0);
        poner_u16(&mut nb, 4, MAX_EXT_HOJA as u16);
        poner_u16(&mut nb, 6, 0);
        escribir_bloque(v, nh, &nb);
        let o2 = I_BLOCK + 12 + ent * 12;
        poner_u32(ib, o2, logico);
        poner_u32(ib, o2 + 4, nh);
        poner_u16(ib, o2 + 8, 0);
        poner_u16(ib, I_BLOCK + 2, (ent + 1) as u16);
        anexar_en_hoja(v, nh, logico, fisico)?;
        return Ok(());
    }
    Err(Error::SinEspacio)
}

/// Devuelve el bloque fisico del bloque logico `k`, reservando lo que falte
/// (incluso los bloques de un posible hueco, que se rellenan con ceros).
fn asegurar_bloque(v: &mut Volumen, ib: &mut [u8], k: u32) -> Result<u32, Error> {
    if let Some(f) = fisico_de(v, ib, k) {
        return Ok(f);
    }
    let mut siguiente = bloques_asignados(v, ib);
    while siguiente <= k {
        let f = asignar_bloque(v).ok_or(Error::SinEspacio)?;
        escribir_bloque(v, f, &vec![0u8; v.bs]);
        anexar_extent(v, ib, siguiente, f)?;
        siguiente += 1;
    }
    fisico_de(v, ib, k).ok_or(Error::SinEspacio)
}

/// Libera todos los bloques de datos (y los bloques hoja del arbol de extents).
fn liberar_datos(v: &mut Volumen, ib: &[u8]) {
    if es_extents(ib) {
        let depth = u16le(ib, I_BLOCK + 6);
        if depth >= 1 {
            let ent = (u16le(ib, I_BLOCK + 2) as usize).min(MAX_EXT_INODO);
            for k in 0..ent {
                let o = I_BLOCK + 12 + k * 12;
                let hoja = u32le(ib, o + 4);
                let hb = leer_bloque(v, hoja);
                let he = (u16le(&hb, 2) as usize).min(MAX_EXT_HOJA);
                for j in 0..he {
                    let p = 12 + j * 12;
                    let fb = u32le(&hb, p + 8);
                    let ln = u16le(&hb, p + 4) as u32;
                    for t in 0..ln {
                        liberar_bloque(v, fb + t);
                    }
                }
                liberar_bloque(v, hoja);
            }
        } else {
            let ent = (u16le(ib, I_BLOCK + 2) as usize).min(MAX_EXT_INODO);
            for k in 0..ent {
                let o = I_BLOCK + 12 + k * 12;
                let fb = u32le(ib, o + 8);
                let ln = u16le(ib, o + 4) as u32;
                for t in 0..ln {
                    liberar_bloque(v, fb + t);
                }
            }
        }
    } else {
        for k in 0..12 {
            let x = u32le(ib, I_BLOCK + k * 4);
            if x != 0 {
                liberar_bloque(v, x);
            }
        }
        let ind = u32le(ib, I_BLOCK + 48);
        if ind != 0 {
            let b = leer_bloque(v, ind);
            for k in 0..(v.bs / 4) {
                let x = u32le(&b, k * 4);
                if x != 0 {
                    liberar_bloque(v, x);
                }
            }
            liberar_bloque(v, ind);
        }
    }
}

/// Restablece `i_block` a un arbol de extents vacio.
fn extents_vacios(ib: &mut [u8]) {
    for k in I_BLOCK..I_BLOCK + 60 {
        ib[k] = 0;
    }
    poner_u16(ib, I_BLOCK, MAGIA_EXTENT);
    poner_u16(ib, I_BLOCK + 2, 0);
    poner_u16(ib, I_BLOCK + 4, MAX_EXT_INODO as u16);
    poner_u16(ib, I_BLOCK + 6, 0);
}

// ---------------------------------------------------------------------------
// Directorios.
//
// Cada entrada es: inodo (u32), longitud (u16), longitud del nombre (u8), tipo
// (u8) y el nombre. La longitud de la entrada es multiplo de 4 y la ultima
// entrada de un bloque ocupa lo que sobra hasta el final del bloque.
// ---------------------------------------------------------------------------

/// Una entrada de directorio ya decodificada.
struct EntradaDir {
    ino: u32,
    rec_len: u16,
    name_len: usize,
    ftype: u8,
}

/// Devuelve los bloques fisicos de un inodo en orden logico (para directorios).
fn bloques_datos(v: &Volumen, ib: &[u8]) -> Vec<u32> {
    if es_extents(ib) {
        let mut pares: Vec<(u32, u32)> = Vec::new();
        con_extents(v, ib, |lg, fi, ln| {
            for j in 0..ln {
                pares.push((lg + j, fi + j));
            }
        });
        pares.sort_by_key(|p| p.0);
        pares.into_iter().map(|p| p.1).collect()
    } else {
        let tam = u32le(ib, I_SIZE);
        let n = (tam + v.bs as u32 - 1) / v.bs as u32;
        let mut out = Vec::new();
        for k in 0..n {
            if let Some(f) = clasico_fisico(v, ib, k) {
                out.push(f);
            }
        }
        out
    }
}

/// Decodifica la entrada de directorio que empieza en `off`.
fn leer_entrada(v: &Volumen, b: &[u8], off: usize) -> Option<EntradaDir> {
    if off + 8 > v.bs {
        return None;
    }
    let ino = u32le(b, off);
    let rec_len = u16le(b, off + 4);
    if rec_len < 8 || off + rec_len as usize > v.bs {
        return None;
    }
    let (name_len, ftype) = if v.feat_incompat & FEAT_INCOMPAT_FILETYPE != 0 {
        (b[off + 6] as usize, b[off + 7])
    } else {
        (u16le(b, off + 6) as usize, FT_REG)
    };
    if off + 8 + name_len > v.bs {
        return None;
    }
    Some(EntradaDir {
        ino,
        rec_len,
        name_len,
        ftype,
    })
}

/// Escribe una entrada de directorio en `blk[off..off+rec_len]`.
fn escribir_entrada(
    blk: &mut [u8],
    off: usize,
    ino: u32,
    rec_len: u16,
    nombre: &str,
    t: Tipo,
    incompat: u32,
) {
    let fin = off + rec_len as usize;
    for x in blk[off..fin].iter_mut() {
        *x = 0;
    }
    poner_u32(blk, off, ino);
    poner_u16(blk, off + 4, rec_len);
    if incompat & FEAT_INCOMPAT_FILETYPE != 0 {
        blk[off + 6] = nombre.len() as u8;
        blk[off + 7] = t.ftype();
    } else {
        poner_u16(blk, off + 6, nombre.len() as u16);
    }
    blk[off + 8..off + 8 + nombre.len()].copy_from_slice(nombre.as_bytes());
}

/// Cuantos bytes necesita una entrada para un nombre dado.
#[inline]
fn rec_len_de(nombre: &str) -> usize {
    (8 + nombre.len() + 3) & !3
}

/// Busca un nombre en un directorio. Devuelve (inodo, tipo).
fn buscar_en(v: &Volumen, dir: u32, nombre: &str) -> Option<(u32, Tipo)> {
    let ib = leer_inodo(v, dir);
    if !es_dir(&ib) {
        return None;
    }
    for b in bloques_datos(v, &ib) {
        let blk = leer_bloque(v, b);
        let mut off = 0usize;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino != 0 && e.name_len == nombre.len() {
                let n = &blk[off + 8..off + 8 + e.name_len];
                if n == nombre.as_bytes() {
                    return Some((e.ino, Tipo::desde_ftype(e.ftype)));
                }
            }
            off += e.rec_len as usize;
        }
    }
    None
}

/// Anade una entrada a un directorio, dividiendo hueco o creando un bloque.
fn anadir_entrada(v: &mut Volumen, dir: u32, nombre: &str, ino: u32, t: Tipo) -> Result<(), Error> {
    let neces = rec_len_de(nombre);
    let mut ib = leer_inodo(v, dir);
    if !es_dir(&ib) {
        return Err(Error::NoEsDirectorio);
    }
    let bloques = bloques_datos(v, &ib);
    if !bloques.is_empty() {
        for b in bloques.iter().copied() {
            let mut blk = leer_bloque(v, b);
            let mut off = 0usize;
            while off + 8 <= v.bs {
                let e = match leer_entrada(v, &blk, off) {
                    Some(e) => e,
                    None => break,
                };
                let usado = 8 + e.name_len;
                if e.ino == 0 && e.rec_len as usize >= neces {
                    let rl = e.rec_len;
                    escribir_entrada(&mut blk, off, ino, rl, nombre, t, v.feat_incompat);
                    escribir_bloque(v, b, &blk);
                    return Ok(());
                }
                if e.rec_len as usize >= usado + neces {
                    let nuevo = ((usado + 3) & !3) as u16;
                    poner_u16(&mut blk, off + 4, nuevo);
                    let off2 = off + nuevo as usize;
                    let resto = e.rec_len - nuevo;
                    escribir_entrada(&mut blk, off2, ino, resto, nombre, t, v.feat_incompat);
                    escribir_bloque(v, b, &blk);
                    return Ok(());
                }
                off += e.rec_len as usize;
            }
        }
    }
    // No hay hueco: se anade un bloque nuevo al directorio.
    let k = bloques_asignados(v, &ib);
    let f = asegurar_bloque(v, &mut ib, k)?;
    let mut blk = vec![0u8; v.bs];
    escribir_entrada(&mut blk, 0, ino, v.bs as u16, nombre, t, v.feat_incompat);
    escribir_bloque(v, f, &blk);
    poner_u32(&mut ib, I_SIZE, (k + 1) * v.bs as u32);
    let nb = bloques_asignados(v, &ib);
    poner_u32(&mut ib, I_BLOCKS, nb * (v.bs as u32 / 512));
    escribir_inodo(v, dir, &ib);
    Ok(())
}

/// Quita una entrada de un directorio (fusionandola con la anterior).
fn quitar_entrada(v: &mut Volumen, dir: u32, nombre: &str) -> bool {
    let ib = leer_inodo(v, dir);
    for b in bloques_datos(v, &ib) {
        let mut blk = leer_bloque(v, b);
        let mut off = 0usize;
        let mut prev = 0usize;
        let mut hay_prev = false;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino != 0
                && e.name_len == nombre.len()
                && &blk[off + 8..off + 8 + e.name_len] == nombre.as_bytes()
            {
                if hay_prev {
                    let prec = u16le(&blk, prev + 4);
                    poner_u16(&mut blk, prev + 4, prec + e.rec_len);
                } else {
                    poner_u32(&mut blk, off, 0);
                    blk[off + 6] = 0;
                    blk[off + 7] = 0;
                }
                escribir_bloque(v, b, &blk);
                return true;
            }
            prev = off;
            hay_prev = true;
            off += e.rec_len as usize;
        }
    }
    false
}

/// Tamano de un inodo.
fn tamano_de(v: &Volumen, ino: u32) -> u32 {
    u32le(&leer_inodo(v, ino), I_SIZE)
}

/// Recorre el arbol de directorios buscando quien contiene a `reg`.
fn buscar_padre(v: &Volumen, dir: u32, reg: u32, prof: u32) -> Option<u32> {
    if prof > 64 {
        return None;
    }
    let ib = leer_inodo(v, dir);
    if !es_dir(&ib) {
        return None;
    }
    let mut subdirs: Vec<u32> = Vec::new();
    for b in bloques_datos(v, &ib) {
        let blk = leer_bloque(v, b);
        let mut off = 0usize;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino != 0 && e.name_len > 0 {
                let nombre = &blk[off + 8..off + 8 + e.name_len];
                if nombre != b"." && nombre != b".." {
                    if e.ino == reg {
                        return Some(dir);
                    }
                    if e.ftype == FT_DIR {
                        subdirs.push(e.ino);
                    }
                }
            }
            off += e.rec_len as usize;
        }
    }
    for s in subdirs {
        if let Some(p) = buscar_padre(v, s, reg, prof + 1) {
            return Some(p);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Volcado del estado a disco, montaje y formateo.
// ---------------------------------------------------------------------------

/// Indica si el grupo `g` lleva copia del superbloque y de los descriptores.
fn es_backup(v: &Volumen, g: u32) -> bool {
    if v.feat_ro & FEAT_RO_SPARSE_SUPER == 0 {
        return true;
    }
    if g == 0 || g == 1 {
        return true;
    }
    fn potencia(mut n: u32, base: u32) -> bool {
        if n < base {
            return false;
        }
        while n % base == 0 {
            n /= base;
        }
        n == 1
    }
    potencia(g, 3) || potencia(g, 5) || potencia(g, 7)
}

/// Recalcula los contadores libres y escribe mapas, descriptores y superbloque.
fn volcar(v: &mut Volumen) {
    // 1. Recalcula los contadores por grupo y los totales.
    let mut libre_b: u64 = 0;
    let mut libre_i: u32 = 0;
    for g in 0..v.grupos {
        let nb = v.info[g as usize].bloques;
        let mut fb = 0u32;
        for i in 0..nb {
            if !bit_bloque(v, g, i) {
                fb += 1;
            }
        }
        let mut fi = 0u32;
        for i in 0..v.ipg {
            if !bit_inodo(v, g, i) {
                fi += 1;
            }
        }
        libre_b += fb as u64;
        libre_i += fi;
        let o = desc_off(v, g);
        poner_u16(&mut v.desc, o + 0x0C, fb as u16);
        poner_u16(&mut v.desc, o + 0x0E, fi as u16);
        if v.dtam >= 64 {
            poner_u16(&mut v.desc, o + 0x2C, (fb >> 16) as u16);
            poner_u16(&mut v.desc, o + 0x2E, (fi >> 16) as u16);
        }
    }
    v.libres_bloques = libre_b;
    v.libres_inodos = libre_i;

    // 2. Mapas de bits.
    for g in 0..v.grupos {
        let s = g as usize * v.bs;
        let datos_b = v.bm_bloques[s..s + v.bs].to_vec();
        escribir_bloque(v, v.info[g as usize].bm_bloques as u32, &datos_b);
        let datos_i = v.bm_inodos[s..s + v.bs].to_vec();
        escribir_bloque(v, v.info[g as usize].bm_inodos as u32, &datos_i);
    }

    // 3. Tabla de descriptores (bloque completa, con relleno).
    let nbloques_gdt = (v.desc.len() + v.bs - 1) / v.bs;
    let mut gdt = vec![0u8; nbloques_gdt * v.bs];
    gdt[..v.desc.len()].copy_from_slice(&v.desc);
    escribir_bloque(v, v.primer_bloque + 1, &gdt);
    for g in 1..v.grupos {
        if es_backup(v, g) {
            let base = v.primer_bloque + g * v.bpg;
            escribir_bloque(v, base + 1, &gdt);
        }
    }

    // 4. Superbloque con los contadores al dia.
    let mut sb = vec![0u8; 1024];
    leer_bytes(2, &mut sb);
    poner_u32(&mut sb, 0x0C, v.libres_bloques as u32);
    poner_u32(&mut sb, 0x10, v.libres_inodos);
    escribir_bytes(2, &sb);
    for g in 1..v.grupos {
        if es_backup(v, g) {
            let base = v.primer_bloque + g * v.bpg;
            escribir_bytes(base * v.spb, &sb);
        }
    }
}

/// Rellena un superbloque virgen con la geometria indicada.
#[allow(clippy::too_many_arguments)]
fn escribir_superbloque(
    sb: &mut [u8],
    etiqueta: &str,
    bloques: u64,
    inodos: u32,
    bs: usize,
    bpg: u32,
    ipg: u32,
    itam: usize,
    feat_incompat: u32,
    feat_ro: u32,
    grupos: u32,
) {
    for b in sb.iter_mut() {
        *b = 0;
    }
    poner_u32(sb, 0x00, inodos);
    poner_u32(sb, 0x04, bloques as u32);
    poner_u32(sb, 0x0C, 0);
    poner_u32(sb, 0x10, inodos - LOST_FOUND);
    poner_u32(sb, 0x14, if bs == 1024 { 1 } else { 0 });
    poner_u32(sb, 0x18, (bs / 1024).trailing_zeros());
    poner_u32(sb, 0x1C, (bs / 1024).trailing_zeros());
    poner_u32(sb, 0x20, bpg);
    poner_u32(sb, 0x24, bpg);
    poner_u32(sb, 0x28, ipg);
    poner_u16(sb, 0x34, 0);
    poner_u16(sb, 0x36, 0xFFFF);
    poner_u16(sb, 0x38, MAGIA);
    poner_u16(sb, 0x3A, 1); // limpio
    poner_u16(sb, 0x3C, 1); // continuar
    poner_u32(sb, 0x48, 0); // creado por Linux
    poner_u32(sb, 0x4C, 1); // revision dinamica
    poner_u32(sb, 0x54, PRIMER_INODO);
    poner_u16(sb, 0x58, itam as u16);
    poner_u16(sb, 0x5A, 0);
    poner_u32(sb, 0x5C, 0); // compat
    poner_u32(sb, 0x60, feat_incompat);
    poner_u32(sb, 0x64, feat_ro);
    // UUID estable (no aleatoria: rocky no tiene entropia).
    sb[0x68..0x78].copy_from_slice(&[
        0x52, 0x4f, 0x43, 0x4b, 0x59, 0x2d, 0x00, 0x01, 0x91, 0x8b, 0x77, 0x1a, 0x0c, 0xe4, 0x5a,
        0x39,
    ]);
    let e = etiqueta.as_bytes();
    let n = e.len().min(16);
    sb[0x78..0x78 + n].copy_from_slice(&e[..n]);
    sb[0xFC] = 1; // def_hash_version: half_md4
    poner_u16(sb, 0xFE, 0); // sin descriptores de 64 bits
    let _ = grupos;
}

/// Monta el volumen ext4 que hay en el disco.
pub fn montar() -> Result<(), Error> {
    if !disk::presente() {
        return Err(Error::Disco);
    }
    let sectores = disk::sectores();
    if sectores < 256 {
        return Err(Error::Disco);
    }
    let mut sb = vec![0u8; 1024];
    leer_bytes(2, &mut sb);
    if u16le(&sb, 0x38) != MAGIA {
        return Err(Error::NoFormateado);
    }
    let log_bs = u32le(&sb, 0x18) as usize;
    if log_bs > 2 {
        return Err(Error::Disco);
    }
    let bs = 1024usize << log_bs;
    let mut bloques = u32le(&sb, 0x04) as u64;
    let inodos = u32le(&sb, 0x00);
    let bpg = u32le(&sb, 0x20);
    let ipg = u32le(&sb, 0x28);
    let mut itam = u16le(&sb, 0x58) as usize;
    if itam < 128 {
        itam = 128;
    }
    let feat_incompat = u32le(&sb, 0x60);
    let feat_ro = u32le(&sb, 0x64);
    if bpg == 0 || ipg == 0 || inodos == 0 || bloques == 0 {
        return Err(Error::Disco);
    }
    if feat_incompat & FEAT_INCOMPAT_64BIT != 0 {
        bloques |= (u32le(&sb, 0x150) as u64) << 32;
    }
    let primer_bloque = if bs == 1024 { 1 } else { 0 };
    let mut dtam = 32usize;
    if feat_incompat & FEAT_INCOMPAT_64BIT != 0 {
        let d = u16le(&sb, 0xFE) as usize;
        if d >= 32 {
            dtam = d;
        }
    }
    let mut etiqueta = [0u8; 16];
    etiqueta.copy_from_slice(&sb[0x78..0x88]);
    let grupos = ((bloques + bpg as u64 - 1) / bpg as u64) as u32;
    if grupos == 0 || grupos > 1_000_000 {
        return Err(Error::Disco);
    }
    let spb = (bs / 512) as u32;

    // Tabla de descriptores de grupo.
    let gdt_bytes = ((grupos as usize * dtam + 511) / 512) * 512;
    let mut desc = vec![0u8; gdt_bytes];
    leer_bytes((primer_bloque + 1) * spb, &mut desc);

    // Datos derivados de cada grupo.
    let mut info = Vec::with_capacity(grupos as usize);
    for g in 0..grupos {
        let o = g as usize * dtam;
        let bm = desc_u64(&desc, o, dtam, 0);
        let im = desc_u64(&desc, o, dtam, 4);
        let tb = desc_u64(&desc, o, dtam, 8);
        let inicio = primer_bloque as u64 + g as u64 * bpg as u64;
        let nb = (bloques.saturating_sub(inicio)).min(bpg as u64) as u32;
        info.push(GrupoInfo {
            bm_bloques: bm,
            bm_inodos: im,
            tabla_inodos: tb,
            bloques: nb,
        });
    }

    // Copia de los mapas de bits a memoria.
    let mut bm_bloques = vec![0u8; grupos as usize * bs];
    let mut bm_inodos = vec![0u8; grupos as usize * bs];
    for g in 0..grupos {
        let s = g as usize * bs;
        let b = leer_bloque_raw(bs, spb, info[g as usize].bm_bloques as u32);
        bm_bloques[s..s + bs].copy_from_slice(&b);
        let i = leer_bloque_raw(bs, spb, info[g as usize].bm_inodos as u32);
        bm_inodos[s..s + bs].copy_from_slice(&i);
    }

    let libres_bloques = u32le(&sb, 0x0C) as u64;
    let libres_inodos = u32le(&sb, 0x10);

    unsafe {
        *VOL.get() = Some(Volumen {
            bs,
            spb,
            bloques,
            inodos,
            bpg,
            ipg,
            itam,
            dtam,
            grupos,
            primer_bloque,
            feat_incompat,
            feat_ro,
            etiqueta,
            desc,
            info,
            bm_bloques,
            bm_inodos,
            libres_bloques,
            libres_inodos,
        });
    }
    Ok(())
}

/// Formatea el disco con un volumen ext4 nuevo.
pub fn formatear(etiqueta: &str) -> Result<(), Error> {
    if !disk::presente() {
        return Err(Error::Disco);
    }
    let total = disk::sectores();
    if total < 256 {
        return Err(Error::Disco);
    }
    let bs = 1024usize;
    let spb = 2u32;
    let bloques = total / 2;
    let bpg = BPG_FMT;
    let ipg = IPG_FMT;
    let grupos = ((bloques + bpg as u64 - 1) / bpg as u64) as u32;
    if grupos == 0 {
        return Err(Error::Disco);
    }
    let inodos = grupos * ipg;
    let itam = ITAM_FMT;
    let itable_bloques = (ipg as usize * itam / bs) as u32;
    let primer_bloque = PRIMER_BLOQUE_FMT;
    let dtam = 32usize;
    let feat_incompat = FEAT_INCOMPAT_FILETYPE | FEAT_INCOMPAT_EXTENTS;
    let feat_ro = FEAT_RO_LARGE_FILE;

    // Descriptores.
    let mut desc = vec![0u8; grupos as usize * dtam];
    for g in 0..grupos {
        let base = primer_bloque + g * bpg;
        let o = g as usize * dtam;
        poner_u32(&mut desc, o, base + 2);
        poner_u32(&mut desc, o + 4, base + 3);
        poner_u32(&mut desc, o + 8, base + 4);
    }

    // Mapas de bits: metadatos de cada grupo reservados, relleno a 1.
    let mut bm_b = vec![0u8; grupos as usize * bs];
    let mut bm_i = vec![0u8; grupos as usize * bs];
    for g in 0..grupos {
        let s = g as usize * bs;
        let nb =
            ((bloques - (primer_bloque as u64 + g as u64 * bpg as u64)).min(bpg as u64)) as u32;
        for k in 0..(4 + itable_bloques) {
            bm_b[s + (k / 8) as usize] |= 1 << (k % 8);
        }
        for k in nb..bpg {
            bm_b[s + (k / 8) as usize] |= 1 << (k % 8);
        }
        for n in 0..11u32 {
            bm_i[s + (n / 8) as usize] |= 1 << (n % 8);
        }
        for k in ipg..(bs as u32 * 8) {
            bm_i[s + (k / 8) as usize] |= 1 << (k % 8);
        }
    }

    // Vuelca mapas y limpia las tablas de inodos.
    let ceros = vec![0u8; bs];
    for g in 0..grupos {
        let base = primer_bloque + g * bpg;
        let s = g as usize * bs;
        escribir_bloque_raw(bs, spb, base + 2, &bm_b[s..s + bs]);
        escribir_bloque_raw(bs, spb, base + 3, &bm_i[s..s + bs]);
        for k in 0..itable_bloques {
            escribir_bloque_raw(bs, spb, base + 4 + k, &ceros);
        }
    }

    // Superbloque y tabla de descriptores (original y copias de respaldo).
    let mut sb = vec![0u8; 1024];
    escribir_superbloque(
        &mut sb,
        etiqueta,
        bloques,
        inodos,
        bs,
        bpg,
        ipg,
        itam,
        feat_incompat,
        feat_ro,
        grupos,
    );
    let nbloques_gdt = (desc.len() + bs - 1) / bs;
    let mut gdt = vec![0u8; nbloques_gdt * bs];
    gdt[..desc.len()].copy_from_slice(&desc);
    escribir_bytes(2, &sb);
    for i in 0..nbloques_gdt {
        escribir_bloque_raw(
            bs,
            spb,
            primer_bloque + 1 + i as u32,
            &gdt[i * bs..i * bs + bs],
        );
    }
    for g in 1..grupos {
        let base = primer_bloque + g * bpg;
        escribir_bytes(base * spb, &sb);
        for i in 0..nbloques_gdt {
            escribir_bloque_raw(bs, spb, base + 1 + i as u32, &gdt[i * bs..i * bs + bs]);
        }
    }
    disk::sincronizar();

    // Ya se puede montar; crea la raiz y lost+found como cualquier otro dir.
    montar()?;
    let v = vol();
    crear_especial(v, RAIZ_INODO, RAIZ_INODO, 3)?;
    crear_especial(v, LOST_FOUND, RAIZ_INODO, 2)?;
    anadir_entrada(v, RAIZ_INODO, "lost+found", LOST_FOUND, Tipo::Directorio)?;
    ajustar_dirs(v, RAIZ_INODO, 1);
    ajustar_dirs(v, LOST_FOUND, 1);
    volcar(v);
    disk::sincronizar();
    Ok(())
}

/// Crea un inodo de directorio con un bloque que solo tiene "." y "..".
fn crear_especial(v: &mut Volumen, ino: u32, padre: u32, enlaces: u16) -> Result<(), Error> {
    let mut ib = vec![0u8; v.itam];
    poner_u16(&mut ib, I_MODE, MODO_DIR);
    poner_u16(&mut ib, I_LINKS, enlaces);
    poner_u32(&mut ib, I_FLAGS, FLAG_EXTENTS);
    extents_vacios(&mut ib);
    let f = asignar_bloque(v).ok_or(Error::SinEspacio)?;
    escribir_bloque(v, f, &vec![0u8; v.bs]);
    let mut d = vec![0u8; v.bs];
    escribir_entrada(&mut d, 0, ino, 12, ".", Tipo::Directorio, v.feat_incompat);
    escribir_entrada(
        &mut d,
        12,
        padre,
        (v.bs - 12) as u16,
        "..",
        Tipo::Directorio,
        v.feat_incompat,
    );
    escribir_bloque(v, f, &d);
    anexar_extent(v, &mut ib, 0, f)?;
    poner_u32(&mut ib, I_SIZE, v.bs as u32);
    poner_u32(&mut ib, I_BLOCKS, (v.bs / 512) as u32);
    escribir_inodo(v, ino, &ib);
    Ok(())
}

// ---------------------------------------------------------------------------
// API publica (la misma que usaba el sistema de ficheros anterior).
// ---------------------------------------------------------------------------

/// Indica si hay un volumen montado.
pub fn montado() -> bool {
    hay_volumen()
}

/// Inodo de la raiz.
pub fn raiz() -> u32 {
    RAIZ_INODO
}

/// Etiqueta del volumen.
pub fn etiqueta() -> String {
    let e = vol().etiqueta;
    let fin = e.iter().position(|&b| b == 0).unwrap_or(e.len());
    String::from_utf8_lossy(&e[..fin]).into_owned()
}

/// Total de bloques del volumen (en unidades de `BLOQUE`).
pub fn total_bloques() -> u64 {
    let v = vol();
    v.bloques * v.bs as u64 / BLOQUE as u64
}

/// Bloques libres (en unidades de `BLOQUE`).
pub fn bloques_libres() -> u64 {
    let v = vol();
    v.libres_bloques * v.bs as u64 / BLOQUE as u64
}

/// Registros (inodos) libres.
pub fn registros_libres() -> u32 {
    vol().libres_inodos
}

/// Tipo de un registro.
pub fn tipo(reg: u32) -> Tipo {
    let v = vol();
    let ib = leer_inodo(v, reg);
    if es_dir(&ib) {
        Tipo::Directorio
    } else {
        Tipo::Fichero
    }
}

/// Tamano en bytes de un registro.
pub fn tamano(reg: u32) -> u32 {
    tamano_de(vol(), reg)
}

/// Registro del directorio padre.
pub fn padre(reg: u32) -> u32 {
    let v = vol();
    let ib = leer_inodo(v, reg);
    if es_dir(&ib) {
        for b in bloques_datos(v, &ib) {
            let blk = leer_bloque(v, b);
            let mut off = 0usize;
            while off + 8 <= v.bs {
                let e = match leer_entrada(v, &blk, off) {
                    Some(e) => e,
                    None => break,
                };
                if e.ino != 0 && e.name_len == 2 && &blk[off + 8..off + 10] == b".." {
                    return e.ino;
                }
                off += e.rec_len as usize;
            }
            break;
        }
        RAIZ_INODO
    } else {
        buscar_padre(v, RAIZ_INODO, reg, 0).unwrap_or(RAIZ_INODO)
    }
}

/// Separa una ruta en (inodo del padre, nombre).
fn dividir(cwd: u32, ruta: &str) -> Result<(u32, String), Error> {
    let ruta = ruta.trim();
    if ruta.is_empty() {
        return Err(Error::NoExiste);
    }
    let (padre_ruta, nombre) = match ruta.rfind('/') {
        Some(i) => (&ruta[..i], &ruta[i + 1..]),
        None => ("", ruta),
    };
    if nombre.is_empty() || nombre == "." || nombre == ".." {
        return Err(Error::NombreInvalido);
    }
    if nombre.len() > NOMBRE_MAX {
        return Err(Error::DemasiadoLargo);
    }
    if nombre.contains(|c: char| c == '\\' || c == ':') {
        return Err(Error::NombreInvalido);
    }
    let padre = if padre_ruta.is_empty() {
        cwd
    } else {
        resolver(cwd, padre_ruta)?
    };
    Ok((padre, String::from(nombre)))
}

/// Resuelve una ruta (absoluta si empieza por `/`, relativa si no).
pub fn resolver(cwd: u32, ruta: &str) -> Result<u32, Error> {
    let ruta = ruta.trim();
    let mut actual = if ruta.starts_with('/') {
        RAIZ_INODO
    } else {
        cwd
    };
    if ruta == "/" || ruta.is_empty() {
        return Ok(actual);
    }
    for parte in ruta.split('/') {
        if parte.is_empty() || parte == "." {
            continue;
        }
        if parte == ".." {
            actual = padre(actual);
            continue;
        }
        let v = vol();
        let ib = leer_inodo(v, actual);
        if !es_dir(&ib) {
            return Err(Error::NoEsDirectorio);
        }
        match buscar_en(v, actual, parte) {
            Some((ino, _)) => actual = ino,
            None => return Err(Error::NoExiste),
        }
    }
    Ok(actual)
}

/// Ruta completa de un registro.
pub fn ruta_de(reg: u32) -> String {
    if reg == RAIZ_INODO {
        return String::from("/");
    }
    let mut partes: Vec<String> = Vec::new();
    let mut actual = reg;
    let mut guarda = 0;
    while actual != RAIZ_INODO && guarda < 256 {
        let p = padre(actual);
        match nombre_de(p, actual) {
            Some(n) => partes.push(n),
            None => break,
        }
        if p == actual {
            break;
        }
        actual = p;
        guarda += 1;
    }
    let mut s = String::new();
    for p in partes.iter().rev() {
        s.push('/');
        s.push_str(p);
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

/// Nombre de un registro dentro de su directorio.
pub fn nombre_de(padre: u32, reg: u32) -> Option<String> {
    let v = vol();
    let ib = leer_inodo(v, padre);
    if !es_dir(&ib) {
        return None;
    }
    for b in bloques_datos(v, &ib) {
        let blk = leer_bloque(v, b);
        let mut off = 0usize;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino == reg && e.name_len > 0 {
                let nombre = &blk[off + 8..off + 8 + e.name_len];
                if nombre != b"." && nombre != b".." {
                    return Some(String::from_utf8_lossy(nombre).into_owned());
                }
            }
            off += e.rec_len as usize;
        }
    }
    None
}

/// Crea un fichero o directorio en la ruta indicada.
pub fn crear(cwd: u32, ruta: &str, t: Tipo) -> Result<u32, Error> {
    let (padre, nombre) = dividir(cwd, ruta)?;
    {
        let v = vol();
        if buscar_en(v, padre, &nombre).is_some() {
            return Err(Error::YaExiste);
        }
    }
    let v = vol();
    let ino = asignar_inodo(v).ok_or(Error::SinEspacio)?;
    let mut ib = vec![0u8; v.itam];
    poner_u16(&mut ib, I_MODE, t.modo());
    poner_u16(&mut ib, I_LINKS, if t == Tipo::Directorio { 2 } else { 1 });
    poner_u32(&mut ib, I_FLAGS, FLAG_EXTENTS);
    extents_vacios(&mut ib);
    if t == Tipo::Directorio {
        let f = asignar_bloque(v).ok_or(Error::SinEspacio)?;
        escribir_bloque(v, f, &vec![0u8; v.bs]);
        let mut d = vec![0u8; v.bs];
        escribir_entrada(&mut d, 0, ino, 12, ".", Tipo::Directorio, v.feat_incompat);
        escribir_entrada(
            &mut d,
            12,
            padre,
            (v.bs - 12) as u16,
            "..",
            Tipo::Directorio,
            v.feat_incompat,
        );
        escribir_bloque(v, f, &d);
        anexar_extent(v, &mut ib, 0, f)?;
        poner_u32(&mut ib, I_SIZE, v.bs as u32);
        poner_u32(&mut ib, I_BLOCKS, (v.bs / 512) as u32);
    }
    escribir_inodo(v, ino, &ib);
    anadir_entrada(v, padre, &nombre, ino, t)?;
    if t == Tipo::Directorio {
        let mut pb = leer_inodo(v, padre);
        let l = u16le(&pb, I_LINKS);
        poner_u16(&mut pb, I_LINKS, l + 1);
        escribir_inodo(v, padre, &pb);
        ajustar_dirs(v, ino, 1);
    }
    volcar(v);
    disk::sincronizar();
    Ok(ino)
}

/// Lee hasta `buf.len()` bytes del registro a partir de `offset`.
pub fn leer(reg: u32, offset: usize, buf: &mut [u8]) -> usize {
    let v = vol();
    let ib = leer_inodo(v, reg);
    let tam = u32le(&ib, I_SIZE) as usize;
    if offset >= tam || buf.is_empty() {
        return 0;
    }
    let fin = tam.min(offset + buf.len());
    let mut hecho = 0usize;
    if es_extents(&ib) {
        let mut pares: Vec<(u32, u32, u32)> = Vec::new();
        con_extents(v, &ib, |lg, fi, ln| pares.push((lg, fi, ln)));
        pares.sort_by_key(|p| p.0);
        for (lg, fi, ln) in pares {
            let ini = lg as usize * v.bs;
            let fin_ext = ini + ln as usize * v.bs;
            if fin_ext <= offset {
                continue;
            }
            if ini >= fin {
                break;
            }
            let desde = ini.max(offset);
            let hasta = fin_ext.min(fin);
            let mut pos = desde;
            while pos < hasta {
                let kb = (pos / v.bs) as u32;
                let dentro = pos % v.bs;
                let phys = fi + (kb - lg);
                let blk = leer_bloque(v, phys);
                let n = (v.bs - dentro).min(hasta - pos);
                buf[pos - offset..pos - offset + n].copy_from_slice(&blk[dentro..dentro + n]);
                pos += n;
            }
            hecho = hasta - offset;
        }
    } else {
        let mut abs = offset;
        while abs < fin {
            let kb = (abs / v.bs) as u32;
            let dentro = abs % v.bs;
            let phys = match clasico_fisico(v, &ib, kb) {
                Some(p) => p,
                None => break,
            };
            let blk = leer_bloque(v, phys);
            let n = (v.bs - dentro).min(fin - abs);
            buf[abs - offset..abs - offset + n].copy_from_slice(&blk[dentro..dentro + n]);
            abs += n;
        }
        hecho = abs - offset;
    }
    hecho
}

/// Escribe `datos` en el registro a partir de `offset`, ampliando si hace falta.
pub fn escribir(reg: u32, offset: usize, datos: &[u8]) -> Result<usize, Error> {
    let v = vol();
    let mut ib = leer_inodo(v, reg);
    let mut abs = offset;
    let fin = offset + datos.len();
    while abs < fin {
        let kb = (abs / v.bs) as u32;
        let dentro = abs % v.bs;
        let phys = asegurar_bloque(v, &mut ib, kb)?;
        let mut blk = leer_bloque(v, phys);
        let n = (v.bs - dentro).min(fin - abs);
        blk[dentro..dentro + n].copy_from_slice(&datos[abs - offset..abs - offset + n]);
        escribir_bloque(v, phys, &blk);
        abs += n;
    }
    let mut tam = u32le(&ib, I_SIZE) as usize;
    if fin > tam {
        tam = fin;
    }
    poner_u32(&mut ib, I_SIZE, tam as u32);
    let nb = bloques_asignados(v, &ib);
    poner_u32(&mut ib, I_BLOCKS, nb * (v.bs as u32 / 512));
    escribir_inodo(v, reg, &ib);
    volcar(v);
    disk::sincronizar();
    Ok(datos.len())
}

/// Vacia un registro (libera sus bloques) dejandolo con tamano cero.
pub fn truncar(reg: u32) {
    let v = vol();
    let mut ib = leer_inodo(v, reg);
    liberar_datos(v, &ib);
    extents_vacios(&mut ib);
    poner_u32(&mut ib, I_SIZE, 0);
    poner_u32(&mut ib, I_BLOCKS, 0);
    escribir_inodo(v, reg, &ib);
    volcar(v);
    disk::sincronizar();
}

/// Comprueba si un directorio esta vacio (solo tiene "." y "..").
pub fn es_vacio(reg: u32) -> bool {
    let v = vol();
    let ib = leer_inodo(v, reg);
    for b in bloques_datos(v, &ib) {
        let blk = leer_bloque(v, b);
        let mut off = 0usize;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino != 0 && e.name_len > 0 {
                let nombre = &blk[off + 8..off + 8 + e.name_len];
                if nombre != b"." && nombre != b".." {
                    return false;
                }
            }
            off += e.rec_len as usize;
        }
    }
    true
}

/// Borra un fichero o un directorio (que debe estar vacio).
pub fn borrar(cwd: u32, ruta: &str) -> Result<(), Error> {
    let (padre, nombre) = dividir(cwd, ruta)?;
    let v = vol();
    let (reg, t) = buscar_en(v, padre, &nombre).ok_or(Error::NoExiste)?;
    if reg == RAIZ_INODO {
        return Err(Error::NoVacio);
    }
    if t == Tipo::Directorio && !es_vacio(reg) {
        return Err(Error::NoVacio);
    }
    let ib = leer_inodo(v, reg);
    liberar_datos(v, &ib);
    let mut cero = vec![0u8; v.itam];
    poner_u32(&mut cero, I_DTIME, 1);
    escribir_inodo(v, reg, &cero);
    liberar_inodo(v, reg);
    quitar_entrada(v, padre, &nombre);
    if t == Tipo::Directorio {
        let mut pb = leer_inodo(v, padre);
        let l = u16le(&pb, I_LINKS);
        if l > 1 {
            poner_u16(&mut pb, I_LINKS, l - 1);
        }
        escribir_inodo(v, padre, &pb);
        ajustar_dirs(v, reg, -1);
    }
    volcar(v);
    disk::sincronizar();
    Ok(())
}

/// Recorre las entradas de un directorio (sin "." ni "..").
pub fn listar<F: FnMut(&Entrada)>(dir: u32, mut f: F) {
    let _ = listar_hasta(dir, |entrada| {
        f(entrada);
        true
    });
}

/// Recorre entradas hasta que el callback devuelva `false`.
///
/// Los metadatos de inodos se leen por bloques y se reutiliza un solo búfer:
/// los inodos vecinos suelen compartir bloque, por lo que una lista de carpeta
/// evita lecturas de disco y asignaciones repetidas. El límite permite a los
/// consumidores paginados dejar de recorrer una carpeta cuando ya tienen
/// suficientes elementos.
///
/// En modo incremental (Fase 5), tras cada lote se puede ceder CPU fuera
/// del recorrido para mantener el sistema responsivo.
pub fn listar_hasta<F: FnMut(&Entrada) -> bool>(dir: u32, mut f: F) -> usize {
    let v = vol();
    let ib = leer_inodo(v, dir);
    if !es_dir(&ib) {
        return 0;
    }
    let bloques = bloques_datos(v, &ib);
    let mut bloque_inodo = vec![0u8; v.bs];
    let mut bloque_inodo_cargado = None;
    let mut cantidad = 0;
    let mut _entradas_bloque = 0;
    'directorios: for b in bloques {
        let blk = leer_bloque(v, b);
        let mut off = 0usize;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino != 0 && e.name_len > 0 {
                let nombre = &blk[off + 8..off + 8 + e.name_len];
                if nombre != b"." && nombre != b".." {
                    let off_inodo = inodo_off(v, e.ino);
                    let bloque = (off_inodo / v.bs as u64) as u32;
                    if bloque_inodo_cargado != Some(bloque) {
                        leer_bloque_en(v.bs, v.spb, bloque, &mut bloque_inodo);
                        bloque_inodo_cargado = Some(bloque);
                    }
                    let dentro = (off_inodo % v.bs as u64) as usize;
                    let datos_inodo = &bloque_inodo[dentro..dentro + v.itam];
                    let entrada = Entrada {
                        registro: e.ino,
                        tipo: Tipo::desde_ftype(e.ftype),
                        nombre: String::from_utf8_lossy(nombre).into_owned(),
                        tam: u32le(datos_inodo, I_SIZE),
                        modificado: u32le(datos_inodo, I_MTIME),
                    };
                    cantidad += 1;
                    // Fase 5: ceder cada cierto número de entradas para mantener
                    // el sistema responsivo durante la enumeración de carpetas
                    if cantidad % 32 == 0 {
                        unsafe {
                            crate::tarea::ceder();
                        }
                    }
                    if !f(&entrada) {
                        break 'directorios;
                    }
                }
            }
            off += e.rec_len as usize;
        }
    }
    cantidad
}

/// Renombra un registro dentro de su directorio.
pub fn renombrar(cwd: u32, ruta: &str, nuevo: &str) -> Result<(), Error> {
    let (padre, nombre) = dividir(cwd, ruta)?;
    if nuevo.is_empty() || nuevo.len() > NOMBRE_MAX || nuevo.contains('/') {
        return Err(Error::NombreInvalido);
    }
    let v = vol();
    if buscar_en(v, padre, nuevo).is_some() {
        return Err(Error::YaExiste);
    }
    let (reg, _t) = buscar_en(v, padre, &nombre).ok_or(Error::NoExiste)?;
    let neces = rec_len_de(nuevo);
    let ib = leer_inodo(v, padre);
    for b in bloques_datos(v, &ib) {
        let mut blk = leer_bloque(v, b);
        let mut off = 0usize;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino == reg
                && e.name_len == nombre.len()
                && &blk[off + 8..off + 8 + e.name_len] == nombre.as_bytes()
            {
                if neces <= e.rec_len as usize {
                    for x in blk[off + 8..off + e.rec_len as usize].iter_mut() {
                        *x = 0;
                    }
                    if v.feat_incompat & FEAT_INCOMPAT_FILETYPE != 0 {
                        blk[off + 6] = nuevo.len() as u8;
                    } else {
                        poner_u16(&mut blk, off + 6, nuevo.len() as u16);
                    }
                    blk[off + 8..off + 8 + nuevo.len()].copy_from_slice(nuevo.as_bytes());
                    escribir_bloque(v, b, &blk);
                    disk::sincronizar();
                    return Ok(());
                }
                break;
            }
            off += e.rec_len as usize;
        }
    }
    // Si no cabe en el sitio, se quita y se vuelve a anadir.
    quitar_entrada(v, padre, &nombre);
    let t = tipo(reg);
    anadir_entrada(v, padre, nuevo, reg, t)?;
    volcar(v);
    disk::sincronizar();
    Ok(())
}

// ---------------------------------------------------------------------------
// Fase 11: el filesystem visto desde el anillo 3.
//
// El shell de rocky (anillo 0) ya usaba `crear`/`borrar`/`renombrar`/`listar`
// directamente. Linux 0.01, en cambio, expone todo eso al programa de usuario
// con `sys_link`, `sys_rename`, `sys_mkdir`, `sys_rmdir`, `sys_unlink` y
// `sys_stat` (los inodos guardan ademas los metadatos que `stat` devuelve).
// Este bloque anade la parte que faltaba: enlaces duros, movimiento entre
// directorios y lectura de metadatos.
// ---------------------------------------------------------------------------

/// Numero de enlaces duros de un registro (`i_links_count` del inodo). Para un
/// directorio vale `2 + subdirectorios`, como manda Unix.
pub fn enlaces(reg: u32) -> u32 {
    let v = vol();
    u16le(&leer_inodo(v, reg), I_LINKS) as u32
}

/// Modo POSIX de un registro: tipo (`S_IFMT`) mas permisos, tal y como lo
/// guarda Estrato en `i_mode` (`0o040755` para directorios, `0o100644` para
/// ficheros). rocky es monousuario, de modo que estos bits se conservan pero no
/// se auditan todavia.
pub fn modo(reg: u32) -> u32 {
    let v = vol();
    u16le(&leer_inodo(v, reg), I_MODE) as u32
}

/// `link`: anade una segunda entrada, `ruta_nueva`, que apunta al **mismo**
/// inodo que `ruta` (el `sys_link` de Linux 0.01). El fichero solo desaparece
/// cuando se borran los dos nombres, y `i_links_count` cuenta los que quedan.
///
/// Solo con ficheros: enlazar un directorio romperia el arbol de rutas (dos
/// caminos distintos llegarian al mismo sitio, y `..` dejaria de ser un arbol).
pub fn enlazar(cwd: u32, ruta: &str, ruta_nueva: &str) -> Result<(), Error> {
    let reg = resolver(cwd, ruta)?;
    let t = tipo(reg);
    if t == Tipo::Directorio {
        return Err(Error::EsDirectorio);
    }
    let (padre, nombre) = dividir(cwd, ruta_nueva)?;
    let v = vol();
    if buscar_en(v, padre, &nombre).is_some() {
        return Err(Error::YaExiste);
    }
    anadir_entrada(v, padre, &nombre, reg, t)?;
    let mut ib = leer_inodo(v, reg);
    let l = u16le(&ib, I_LINKS);
    poner_u16(&mut ib, I_LINKS, l.saturating_add(1));
    escribir_inodo(v, reg, &ib);
    volcar(v);
    disk::sincronizar();
    Ok(())
}

/// `rename`: mueve `ruta` a `ruta_nueva`.
///
/// Si `ruta_nueva` no lleva `/` es un simple renombrado dentro del mismo
/// directorio (el camino que ya recorria [`renombrar`]). Si lleva ruta, el
/// registro cambia de directorio: se quita la entrada vieja, se anade la nueva
/// y, si se mueve un directorio, se reapunta su `..` y se corrigen los
/// enlaces de los dos padres.
pub fn mover(cwd: u32, ruta: &str, ruta_nueva: &str) -> Result<(), Error> {
    if !ruta_nueva.contains('/') {
        return renombrar(cwd, ruta, ruta_nueva);
    }

    let reg = resolver(cwd, ruta)?;
    if reg == RAIZ_INODO {
        return Err(Error::NombreInvalido); // la raiz no se mueve
    }
    let t = tipo(reg);
    let (npadre, nnombre) = dividir(cwd, ruta_nueva)?;
    if tipo(npadre) != Tipo::Directorio {
        return Err(Error::NoEsDirectorio);
    }
    if t == Tipo::Directorio && es_o_contiene(reg, npadre) {
        // Meter un directorio dentro de si mismo cerraria el arbol.
        return Err(Error::NombreInvalido);
    }
    let (opadre, onombre) = dividir(cwd, ruta)?;
    if opadre == npadre && onombre == nnombre {
        return Ok(());
    }

    let v = vol();
    if buscar_en(v, npadre, &nnombre).is_some() {
        return Err(Error::YaExiste);
    }
    if buscar_en(v, opadre, &onombre).is_none() {
        return Err(Error::NoExiste);
    }
    quitar_entrada(v, opadre, &onombre);
    anadir_entrada(v, npadre, &nnombre, reg, t)?;
    if t == Tipo::Directorio && opadre != npadre {
        fijar_padre(v, reg, npadre);
        ajustar_enlaces(v, opadre, -1);
        ajustar_enlaces(v, npadre, 1);
    }
    volcar(v);
    disk::sincronizar();
    Ok(())
}

/// ¿Es `posible` el propio `ino` o uno de sus descendientes?
fn es_o_contiene(ino: u32, posible: u32) -> bool {
    let mut a = posible;
    for _ in 0..128 {
        if a == ino {
            return true;
        }
        if a == RAIZ_INODO {
            return false;
        }
        let p = padre(a);
        if p == a {
            return false;
        }
        a = p;
    }
    false
}

/// Cambia el `..` de un directorio para que apunte a `nuevo_padre`.
fn fijar_padre(v: &mut Volumen, dir: u32, nuevo_padre: u32) {
    let ib = leer_inodo(v, dir);
    if !es_dir(&ib) {
        return;
    }
    for b in bloques_datos(v, &ib) {
        let mut blk = leer_bloque(v, b);
        let mut off = 0usize;
        while off + 8 <= v.bs {
            let e = match leer_entrada(v, &blk, off) {
                Some(e) => e,
                None => break,
            };
            if e.ino != 0 && e.name_len == 2 && &blk[off + 8..off + 10] == b".." {
                poner_u32(&mut blk, off, nuevo_padre);
                escribir_bloque(v, b, &blk);
                return;
            }
            off += e.rec_len as usize;
        }
    }
}

/// Suma (o resta) uno al contador de enlaces de un directorio, que es
/// `2 + subdirectorios`: al ganar un subdirectorio se paga el enlace que le
/// corresponde y al perderlo se devuelve.
fn ajustar_enlaces(v: &mut Volumen, dir: u32, delta: i32) {
    let mut ib = leer_inodo(v, dir);
    let l = u16le(&ib, I_LINKS) as i32;
    poner_u16(&mut ib, I_LINKS, (l + delta).max(1) as u16);
    escribir_inodo(v, dir, &ib);
}
