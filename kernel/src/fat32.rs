//! Registro del sistema en una particion **FAT32**.
//!
//! rocky anota en un fichero (`COMANDOS.LOG`) lo que se ejecuta en la consola.
//! El fichero vive en una particion **FAT32 propia** (tipo `0x0C`), no en el
//! volumen Estrato: asi **Windows la monta tal cual** y los logs se pueden leer
//! desde el PC sin mirar la pantalla. La particion la formatea `xtask` al
//! construir la imagen (ver `xtask/src/fat32.rs`), que ademas deja creado el
//! fichero con una cadena de clusters ya reservada.
//!
//! Por eso este modulo es pequeno a proposito: no formatea, no crea ficheros y
//! no asigna clusters. Solo **lee la geometria** del sector de arranque,
//! **encuentra** la entrada del fichero en el directorio raiz, sigue su cadena
//! en la FAT y **escribe** al final. Si la particion no existe (un disco sin
//! MBR), todo esto queda apagado y no pasa nada.
//!
//! Codigo propio de rocky.

use crate::disk;
use crate::sync::SyncCell;

/// Tamano de sector (el unico que produce el formateador).
const SECTOR: usize = disk::SECTOR;
/// Clusters que se siguen, como mucho, de la cadena del fichero.
const CADENA_MAX: usize = 512;
/// Tamano maximo del fichero de log (los clusters que reserva el formateador).
const BYTES_MAX: u32 = (CADENA_MAX * SECTOR) as u32;
/// Nombre 8.3 del fichero de log, tal cual aparece en la entrada de directorio.
const NOMBRE_LOG: [u8; 11] = *b"COMANDOSLOG";

/// Estado del fichero de log dentro de la particion FAT32.
struct Log {
    /// LBA fisico del primer sector de la particion.
    base: u32,
    /// Sector (relativo a la particion) donde empiezan los datos.
    datos: u32,
    /// Sectores por cluster.
    spc: u32,
    /// Clusters del fichero, en orden.
    cadena: [u32; CADENA_MAX],
    /// Cuantos clusters validos hay en `cadena`.
    n: usize,
    /// Sector (relativo) donde esta la entrada de directorio del fichero.
    entrada_sector: u32,
    /// Offset de esa entrada dentro del sector.
    entrada_offset: usize,
    /// Tamano actual del fichero, en bytes.
    tamano: u32,
}

static LOG: SyncCell<Option<Log>> = SyncCell::new(None);

/// Lee la geometria de la particion de logs y localiza el fichero.
///
/// Devuelve `true` si la particion existe y el fichero esta donde se espera.
pub fn init() -> bool {
    let Some((base, _)) = disk::particion_log() else {
        return false;
    };
    let mut b = [0u8; SECTOR];
    if disk::leer_crudo(base, &mut b).is_err() || b[510..512] != [0x55, 0xAA] {
        return false;
    }

    // --- Geometria del BPB de FAT32 ---
    let bytes_sector = u16::from_le_bytes([b[11], b[12]]) as u32;
    let spc = b[13] as u32;
    let reservados = u16::from_le_bytes([b[14], b[15]]) as u32;
    let nfats = b[16] as u32;
    let fat_sectores = u32::from_le_bytes([b[36], b[37], b[38], b[39]]);
    let raiz = u32::from_le_bytes([b[44], b[45], b[46], b[47]]);
    if bytes_sector != SECTOR as u32
        || spc == 0
        || !(1..=4).contains(&nfats)
        || fat_sectores == 0
        || raiz < 2
    {
        return false;
    }
    let fat = reservados;
    let datos = reservados + nfats * fat_sectores;

    // --- Busca el fichero en el directorio raiz ---
    let mut hallado = false;
    let mut primer_cluster = 0u32;
    let mut tamano = 0u32;
    let mut entrada_sector = 0u32;
    let mut entrada_offset = 0usize;
    for k in 0..spc {
        let sec = datos + (raiz - 2) * spc + k;
        let mut s = [0u8; SECTOR];
        if disk::leer_crudo(base + sec, &mut s).is_err() {
            return false;
        }
        for i in 0..SECTOR / 32 {
            let e = &s[i * 32..i * 32 + 32];
            if e[0] == 0x00 {
                break;
            }
            // Se saltan las entradas borradas, la etiqueta y los nombres largos.
            if e[0] == 0xE5 || e[11] & 0x08 != 0 || e[11] & 0x0F == 0x0F {
                continue;
            }
            if e[0..11] == NOMBRE_LOG {
                let alto = u16::from_le_bytes([e[20], e[21]]) as u32;
                let bajo = u16::from_le_bytes([e[26], e[27]]) as u32;
                primer_cluster = (alto << 16) | bajo;
                tamano = u32::from_le_bytes([e[28], e[29], e[30], e[31]]);
                entrada_sector = sec;
                entrada_offset = i * 32;
                hallado = true;
                break;
            }
        }
        if hallado {
            break;
        }
    }
    if !hallado || primer_cluster < 2 || tamano > BYTES_MAX {
        return false;
    }

    // --- Sigue la cadena del fichero en la FAT ---
    let mut cadena = [0u32; CADENA_MAX];
    let mut n = 0usize;
    let mut c = primer_cluster;
    while c >= 2 && c < 0x0FFF_FFF8 && n < CADENA_MAX {
        cadena[n] = c;
        n += 1;
        let mut s = [0u8; SECTOR];
        let sec = fat + (c * 4) / SECTOR as u32;
        let off = ((c * 4) % SECTOR as u32) as usize;
        if disk::leer_crudo(base + sec, &mut s).is_err() {
            break;
        }
        c = u32::from_le_bytes([s[off], s[off + 1], s[off + 2], s[off + 3]]) & 0x0FFF_FFFF;
    }
    if n == 0 {
        return false;
    }

    unsafe {
        LOG.get().write(Some(Log {
            base,
            datos,
            spc,
            cadena,
            n,
            entrada_sector,
            entrada_offset,
            tamano,
        }));
    }
    true
}

/// Anade una linea al fichero de log.
///
/// Si la particion no existe, o el fichero ya esta lleno, no hace nada: el log
/// es lo ultimo que debe dar problemas.
pub fn registrar(texto: &str) {
    let Some(l) = (unsafe { (*LOG.get()).as_mut() }) else {
        return;
    };
    let nuevo = l.tamano as usize + texto.len() + 2;
    if nuevo as u32 > BYTES_MAX {
        return;
    }
    if escribir(l, l.tamano, texto.as_bytes()).is_err() {
        return;
    }
    if escribir(l, l.tamano + texto.len() as u32, b"\r\n").is_err() {
        return;
    }
    if !actualizar_tamano(l, nuevo as u32) {
        return;
    }
    l.tamano = nuevo as u32;
}

/// Escribe `datos` en el fichero a partir del desplazamiento `off`.
fn escribir(l: &Log, mut off: u32, mut datos: &[u8]) -> Result<(), ()> {
    let bytes_cluster = l.spc * SECTOR as u32;
    while !datos.is_empty() {
        let idx = (off / bytes_cluster) as usize;
        if idx >= l.n {
            return Err(());
        }
        let cl = l.cadena[idx];
        let dentro = off % bytes_cluster;
        let sector = l.datos + (cl - 2) * l.spc + dentro / SECTOR as u32;
        let en = (dentro % SECTOR as u32) as usize;
        let n = (SECTOR - en).min(datos.len());
        let mut s = [0u8; SECTOR];
        if disk::leer_crudo(l.base + sector, &mut s).is_err() {
            return Err(());
        }
        s[en..en + n].copy_from_slice(&datos[..n]);
        if disk::escribir_crudo(l.base + sector, &s).is_err() {
            return Err(());
        }
        off += n as u32;
        datos = &datos[n..];
    }
    Ok(())
}

/// Relee la entrada de directorio, le pone el tamano nuevo y la reescribe.
fn actualizar_tamano(l: &Log, tamano: u32) -> bool {
    let mut s = [0u8; SECTOR];
    if disk::leer_crudo(l.base + l.entrada_sector, &mut s).is_err() {
        return false;
    }
    let o = l.entrada_offset + 28;
    s[o..o + 4].copy_from_slice(&tamano.to_le_bytes());
    disk::escribir_crudo(l.base + l.entrada_sector, &s).is_ok()
}

/// Tamano actual del log y su tope, para que la consola lo pueda ensenar.
pub fn estado() -> Option<(u32, u32)> {
    unsafe { (*LOG.get()).as_ref().map(|l| (l.tamano, BYTES_MAX)) }
}
