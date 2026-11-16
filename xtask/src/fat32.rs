//! Formateador **FAT32** pequeno, para la particion de logs del sistema.
//!
//! rocky reserva una particion **propia** (tipo `0x0C`, FAT32 LBA) donde va
//! dejando un registro de lo que se hace en la consola. Que sea FAT32 y no el
//! sistema propio (Estrato) tiene una razon practica: **Windows la monta tal
//! cual**, asi que los logs se pueden leer desde el PC sin mirar la pantalla.
//!
//! Aqui solo se *formatea*: se escribe el sector de arranque (BPB), el FSInfo,
//! las dos copias de la FAT y el directorio raiz, y se deja creado el fichero
//! [`FICHERO_LOG`] con una cadena de [`CLUSTERS_LOG`] clusters reservados (de
//! tamano 0). Reservar los clusters por adelantado hace que el kernel, para
//! anotar una linea, no tenga que asignar clusters ni tocar la FAT: solo
//! escribe los datos en el hueco que ya es suyo y sube el tamano del fichero.
//!
//! Es deliberadamente minimo: un solo sector por cluster, sin subdirectorios,
//! sin nombres largos y con la etiqueta del volumen en el directorio raiz. Es
//! lo justo para que Windows lo lea.

/// Tamano de un sector de la particion (la unica geometria que usamos).
pub const SECTOR: usize = 512;
/// Nombre 8.3 del fichero de log que se crea vacio al formatear.
pub const FICHERO_LOG: &str = "COMANDOS.LOG";
/// Clusters reservados al fichero de log (256 KiB con un sector por cluster).
pub const CLUSTERS_LOG: u32 = 512;
/// Sectores por cluster del formateo (uno).
const SECTOR_POR_CLUSTER: u32 = 1;
/// Clusters reservados antes de la FAT (sector de arranque, FSInfo y copias).
const RESERVADOS: u32 = 32;
/// Copias de la FAT (Windows espera dos).
const NFATS: u32 = 2;
/// Sector de la copia de seguridad del sector de arranque.
const COPIA_BOOT: u32 = 6;
/// Sector (relativo a la particion) donde vive el FSInfo.
const FSINFO: u32 = 1;
/// Sector de la copia del FSInfo.
const FSINFO_COPIA: u32 = 7;
/// Primer cluster de datos (el 0 y el 1 estan reservados).
const PRIMER_CLUSTER: u32 = 2;
/// Primer cluster de la cadena del fichero de log.
pub const PRIMER_CLUSTER_LOG: u32 = 3;

// Marcas que el estandar pone en el sector FSInfo.
const FSINFO_LEAD: u32 = 0x4161_5252;
const FSINFO_STRUC: u32 = 0x6141_7272;
const FSINFO_TRAIL: u32 = 0xAA55_0000;
// Banderas de fin de cadena de la FAT.
const EOC: u32 = 0x0FFF_FFFF;
const EOC_MEDIA: u32 = 0x0FFF_FFF8; // entrada 0 (medios + los tres bits altos)

/// Geometria de la particion recien formateada.
///
/// El kernel **no** recibe estos datos por un canal aparte: vuelve a leer el
/// BPB de la particion. Se devuelven aqui solo para informar por consola.
pub struct Formato {
    /// Sectores que ocupa la particion.
    pub sectores: u32,
    /// Clusters de que consta (uno por sector en esta geometria).
    pub clusters: u32,
    /// Sectores de cada copia de la FAT.
    pub fat_sectores: u32,
    /// Offset (en bytes) de los datos dentro de la particion.
    pub offset_datos: u64,
}

/// Formatea `part` (los bytes de la particion entera) como FAT32.
///
/// `lba_oculto` es el LBA donde empieza la particion en el disco (va al BPB,
/// como hace cualquier formateador) y `etiqueta` es la etiqueta del volumen
/// (se recorta o se rellena con espacios hasta 11 caracteres).
pub fn formatear(part: &mut [u8], lba_oculto: u32, etiqueta: &str) -> Formato {
    let total = (part.len() / SECTOR) as u32;
    let fat = tamano_fat(total);
    let primer_datos = RESERVADOS + NFATS * fat;
    let clusters = (total - primer_datos) / SECTOR_POR_CLUSTER;

    // Windows solo reconoce como FAT32 un volumen con mas de 65525 clusters.
    assert!(
        clusters > 65_525,
        "la particion de FAT32 tiene {clusters} clusters (hacen falta mas de 65525)"
    );
    assert!(
        PRIMER_CLUSTER_LOG + CLUSTERS_LOG <= clusters + PRIMER_CLUSTER,
        "no hay sitio en la particion para reservar el fichero de log"
    );

    part.fill(0);

    // --- Sector de arranque (BPB) y su copia ---
    let bpb = bpb_bytes(total, fat, lba_oculto, etiqueta);
    part[..SECTOR].copy_from_slice(&bpb);
    let cb = COPIA_BOOT as usize * SECTOR;
    part[cb..cb + SECTOR].copy_from_slice(&bpb);

    // --- FSInfo y su copia ---
    let fs = fsinfo_bytes(clusters);
    let fo = FSINFO as usize * SECTOR;
    part[fo..fo + SECTOR].copy_from_slice(&fs);
    let fc = FSINFO_COPIA as usize * SECTOR;
    part[fc..fc + SECTOR].copy_from_slice(&fs);

    // --- Las dos copias de la FAT ---
    let fat_bytes = fat as usize * SECTOR;
    for copia in 0..NFATS {
        let o = (RESERVADOS + copia * fat) as usize * SECTOR;
        escribir_fat(&mut part[o..o + fat_bytes]);
    }

    // --- Directorio raiz (cluster 2) ---
    let rd = primer_datos as usize * SECTOR;
    let dir = &mut part[rd..rd + SECTOR];
    // Entrada 0: la etiqueta del volumen (atributo 0x08).
    dir[0..11].copy_from_slice(&etiqueta_11(etiqueta));
    dir[11] = 0x08;
    // Entrada 1: el fichero de log, creado vacio con su cadena ya reservada.
    let (nombre, ext) = nombre_83(FICHERO_LOG);
    dir[32..40].copy_from_slice(&nombre);
    dir[40..43].copy_from_slice(&ext);
    dir[43] = 0x20; // atributo: fichero
    dir[52..54].copy_from_slice(&0u16.to_le_bytes()); // cluster alto
    dir[58..60].copy_from_slice(&(PRIMER_CLUSTER_LOG as u16).to_le_bytes());
    dir[60..64].copy_from_slice(&0u32.to_le_bytes()); // tamano: 0

    Formato {
        sectores: total,
        clusters,
        fat_sectores: fat,
        offset_datos: primer_datos as u64 * SECTOR as u64,
    }
}

/// Tamano (en sectores) de una FAT para `total` sectores de particion.
///
/// El numero de clusters depende del tamano de la FAT, que a su vez depende
/// del numero de clusters: se empieza de poco y se crece **si** todavia no
/// entra. Atencion a no exigir la igualdad exacta: dos valores vecinos pueden
/// pedirse el uno al otro (que la FAT pida 1008 y con 1008 se necesiten 1009)
/// y se quedarian girando sin salir. Dejar una FAT de mas es lo que hace
/// cualquier formateador y no cuesta nada.
fn tamano_fat(total: u32) -> u32 {
    let mut fat = 1u32;
    loop {
        let clusters = (total - RESERVADOS - NFATS * fat) / SECTOR_POR_CLUSTER;
        let pedida = ((clusters as u64 + 2) * 4).div_ceil(SECTOR as u64) as u32;
        if pedida <= fat {
            return fat;
        }
        fat = pedida;
    }
}

/// Etiqueta del volumen, recortada y rellena con espacios hasta 11 bytes.
fn etiqueta_11(etiqueta: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    for (i, b) in etiqueta.bytes().take(11).enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    out
}

/// Construye los 512 bytes del sector de arranque (BPB de FAT32).
fn bpb_bytes(total: u32, fat: u32, lba_oculto: u32, etiqueta: &str) -> [u8; SECTOR] {
    let mut s = [0u8; SECTOR];
    s[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]); // salto + nop
    s[3..11].copy_from_slice(b"MSWIN4.1"); // firma del formateador
    s[11..13].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    s[13] = SECTOR_POR_CLUSTER as u8;
    s[14..16].copy_from_slice(&(RESERVADOS as u16).to_le_bytes());
    s[16] = NFATS as u8;
    s[17..19].copy_from_slice(&0u16.to_le_bytes()); // sin entradas fijas en la raiz
    s[19..21].copy_from_slice(&0u16.to_le_bytes()); // TotSec16 = 0 (se usa la de 32)
    s[21] = 0xF8; // medio fijo (disco duro)
    s[22..24].copy_from_slice(&0u16.to_le_bytes()); // FATSz16 = 0 (se usa la de 32)
    s[24..26].copy_from_slice(&63u16.to_le_bytes()); // sectores por pista
    s[26..28].copy_from_slice(&255u16.to_le_bytes()); // cabezales
    s[28..32].copy_from_slice(&lba_oculto.to_le_bytes());
    s[32..36].copy_from_slice(&total.to_le_bytes());
    s[36..40].copy_from_slice(&fat.to_le_bytes());
    s[40..42].copy_from_slice(&0u16.to_le_bytes()); // banderas extendidas
    s[42..44].copy_from_slice(&0u16.to_le_bytes()); // version del FS
    s[44..48].copy_from_slice(&PRIMER_CLUSTER.to_le_bytes()); // cluster raiz
    s[48..50].copy_from_slice(&(FSINFO as u16).to_le_bytes());
    s[50..52].copy_from_slice(&(COPIA_BOOT as u16).to_le_bytes());
    s[64] = 0x80; // unidad
    s[66] = 0x29; // firma extendida
                  // Numero de serie: una marca fija de rocky ("ROCK").
    s[67..71].copy_from_slice(&0x524F_434B_u32.to_le_bytes());
    s[71..82].copy_from_slice(&etiqueta_11(etiqueta));
    s[82..90].copy_from_slice(b"FAT32   ");
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

/// Construye los 512 bytes del sector FSInfo.
fn fsinfo_bytes(clusters: u32) -> [u8; SECTOR] {
    let mut s = [0xFFu8; SECTOR];
    s[0..4].copy_from_slice(&FSINFO_LEAD.to_le_bytes());
    s[484..488].copy_from_slice(&FSINFO_STRUC.to_le_bytes());
    // Libres: todos menos los reservados (0 y 1), el raiz (2) y la cadena del log.
    let libres = clusters + PRIMER_CLUSTER - PRIMER_CLUSTER_LOG - CLUSTERS_LOG;
    s[488..492].copy_from_slice(&libres.to_le_bytes());
    s[492..496].copy_from_slice(&(PRIMER_CLUSTER_LOG + CLUSTERS_LOG).to_le_bytes());
    s[508..512].copy_from_slice(&FSINFO_TRAIL.to_le_bytes());
    s
}

/// Rellena una copia de la FAT: entradas reservadas y la cadena del log.
///
/// Los clusters 0 y 1 no existen (estan reservados) y el 2 es el directorio
/// raiz, que aqui cabe de sobra en un solo cluster, asi que su entrada marca
/// fin de cadena. Sin esa entrada Windows cree que el directorio raiz esta
/// vacio y no ve COMANDOS.LOG.
fn escribir_fat(fat: &mut [u8]) {
    fat.fill(0); // 0 = cluster libre
    let mut pon = |i: u32, v: u32| {
        let o = i as usize * 4;
        fat[o..o + 4].copy_from_slice(&(v & 0x0FFF_FFFF).to_le_bytes());
    };
    pon(0, EOC_MEDIA);
    pon(1, EOC); // marca de reservado, por si algun dia se lee
    pon(2, EOC); // directorio raiz: un solo cluster
                 // La cadena del log empieza en PRIMER_CLUSTER_LOG, no en PRIMER_CLUSTER:
                 // empezar en 2 enlazaria el directorio raiz con los clusters del log, y
                 // Windows leeria los datos del log como si fueran entradas de directorio.
    let fin_log = PRIMER_CLUSTER_LOG + CLUSTERS_LOG;
    for c in PRIMER_CLUSTER_LOG..fin_log {
        let siguiente = if c + 1 == fin_log { EOC } else { c + 1 };
        pon(c, siguiente);
    }
}

/// Separa un nombre 8.3 en nombre y extension, ambos rellenos con espacios.
fn nombre_83(nombre: &str) -> ([u8; 8], [u8; 3]) {
    let mut n = [b' '; 8];
    let mut e = [b' '; 3];
    let (base, ext) = nombre.split_once('.').unwrap_or((nombre, ""));
    for (i, b) in base.bytes().take(8).enumerate() {
        n[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().take(3).enumerate() {
        e[i] = b.to_ascii_uppercase();
    }
    (n, e)
}
