//! `fs3`: el sistema de ficheros visto desde el anillo 3 (Fase 11).
//!
//! El shell de rocky (anillo 0) ya creaba, movia y borraba entradas llamando
//! directamente a [`crate::fs`]; un proceso de usuario no podia hacer **nada**
//! de eso. Linux 0.01 si lo permitia, con `sys_mkdir`, `sys_rmdir`,
//! `sys_unlink`, `sys_link`, `sys_rename`, `sys_stat` y `sys_readdir`. Este
//! programa ejerce esas llamadas desde un proceso de verdad y comprueba que el
//! volumen queda como estaba al final.
#![no_std]
#![no_main]

#[path = "comun.rs"]
mod comun;

use comun::{
    abrir, access, cerrar, contiene, entrada64, escribir, fstat, getdents64, leer, link, mkdir,
    rename, rmdir, salir, stat, sync, unlink, Stat, DT_DIR, DT_REG, O_CREAT, O_RDONLY, O_WRONLY,
    S_IFDIR, S_IFMT, S_IFREG,
};

const MSG_MKDIR: &[u8] = b"fs3: mkdir crea el directorio y rechaza repetirlo\n";
const MSG_ESCRIBIR: &[u8] = b"fs3: open, write y read devuelven el contenido\n";
const MSG_STAT: &[u8] = b"fs3: stat da size, mode, nlink e ino correctos\n";
const MSG_LINK: &[u8] = b"fs3: link anade un segundo nombre al mismo inodo\n";
const MSG_RENAME: &[u8] = b"fs3: rename mueve el fichero entre directorios\n";
const MSG_DIR: &[u8] = b"fs3: getdents64 lista el directorio y respeta el buffer\n";
const MSG_BORRAR: &[u8] = b"fs3: unlink y rmdir distinguen fichero de directorio\n";
const MSG_FSTAT: &[u8] = b"fs3: access y fstat responden lo esperado\n";
const OK: &[u8] = b"rocky: el filesystem de anillo 3 funciona\n";
const FALLO: &[u8] = b"rocky: la prueba de filesystem fallo\n";

const RAIZ: &[u8] = b"/\0";
const DIR: &[u8] = b"/fase11\0";
const FICHERO: &[u8] = b"/fase11/nota.txt\0";
const COPIA: &[u8] = b"/fase11/copia.txt\0";
const SUBDIR: &[u8] = b"/fase11/sub\0";
const MOVIDO: &[u8] = b"/fase11/sub/nota.txt\0";
const NADA: &[u8] = b"/fase11/no-existe\0";
const CONTENIDO: &[u8] = b"hola desde el anillo 3";

fn fallo() -> ! {
    escribir(1, FALLO);
    salir(1)
}

/// `d_type` de la entrada con ese nombre dentro del volcado de `getdents64`.
fn tipo_de(buf: &[u8], nombre: &[u8]) -> Option<u8> {
    let mut off = 0usize;
    while let Some((_, t, n)) = entrada64(buf, &mut off) {
        if n == nombre {
            return Some(t);
        }
    }
    None
}

#[no_mangle]
pub extern "C" fn rocky_main(_argc: u64) -> ! {
    let mut texto = [0u8; 64];
    let mut dirbuf = [0u8; 512];
    let mut st = Stat::vacio();
    let mut otra = Stat::vacio();

    // 0. Limpieza previa: el disco es el mismo en cada arranque, y una ejecucion
    //    interrumpida dejaria un `/fase11` que haria fallar todas las
    //    comprobaciones siguientes.
    unlink(MOVIDO.as_ptr());
    unlink(COPIA.as_ptr());
    unlink(FICHERO.as_ptr());
    rmdir(SUBDIR.as_ptr());
    rmdir(DIR.as_ptr());

    // 1. `mkdir` crea el directorio y devuelve un error si ya existe.
    if mkdir(DIR.as_ptr(), 0o755) != 0 {
        fallo();
    }
    if mkdir(DIR.as_ptr(), 0o755) != -1 {
        fallo();
    }
    escribir(1, MSG_MKDIR);

    // 2. `open` con `O_CREAT`, `write` y `read`: el fichero nace y guarda el
    //    texto. Los descriptores 0, 1 y 2 son la consola, asi que el primero
    //    libre es el 3.
    let fd = abrir(FICHERO.as_ptr(), O_CREAT | O_WRONLY);
    if fd < 3 {
        fallo();
    }
    if escribir(fd as u64, CONTENIDO) != CONTENIDO.len() as i64 {
        fallo();
    }
    if cerrar(fd as u64) != 0 {
        fallo();
    }
    let fd = abrir(FICHERO.as_ptr(), O_RDONLY);
    if fd < 3 {
        fallo();
    }
    let n = leer(fd as u64, &mut texto[..CONTENIDO.len()]);
    cerrar(fd as u64);
    if n != CONTENIDO.len() as i64 || &texto[..CONTENIDO.len()] != CONTENIDO {
        fallo();
    }
    escribir(1, MSG_ESCRIBIR);

    // 3. `stat`: el tamano, el modo, el inodo y los enlaces del fichero.
    if stat(FICHERO.as_ptr(), &mut st) != 0 {
        fallo();
    }
    if st.st_ino == 0 || st.st_size != CONTENIDO.len() as i64 {
        fallo();
    }
    if st.st_mode & S_IFMT != S_IFREG || st.st_mode & 0o7777 != 0o644 {
        fallo();
    }
    if st.st_nlink != 1 || st.st_blksize != 512 {
        fallo();
    }
    escribir(1, MSG_STAT);

    // 4. `link`: un segundo nombre para el **mismo** inodo. Borrar uno deja el
    //    otro con su contenido.
    if link(FICHERO.as_ptr(), COPIA.as_ptr()) != 0 {
        fallo();
    }
    if stat(COPIA.as_ptr(), &mut otra) != 0 {
        fallo();
    }
    if otra.st_ino != st.st_ino || otra.st_nlink != 2 || otra.st_size != st.st_size {
        fallo();
    }
    escribir(1, MSG_LINK);

    // 5. `rename` cruzando directorios: el registro se mueve con sus datos y
    //    el nombre viejo deja de existir.
    if mkdir(SUBDIR.as_ptr(), 0o755) != 0 {
        fallo();
    }
    if rename(FICHERO.as_ptr(), MOVIDO.as_ptr()) != 0 {
        fallo();
    }
    if stat(FICHERO.as_ptr(), &mut st) != -1 {
        fallo();
    }
    if stat(MOVIDO.as_ptr(), &mut st) != 0 {
        fallo();
    }
    // Sigue siendo el mismo inodo, con los dos nombres que tiene ahora.
    if st.st_ino != otra.st_ino || st.st_nlink != 2 || st.st_size != CONTENIDO.len() as i64 {
        fallo();
    }
    escribir(1, MSG_RENAME);

    // 6. `getdents64`: recorrer un directorio abierto, respetando el cookie de
    //    la posicion y sin tragarse un buffer pequeno.
    let fd = abrir(DIR.as_ptr(), O_RDONLY);
    if fd < 3 {
        fallo();
    }
    let mut pequeno = [0u8; 8];
    if getdents64(fd as u64, &mut pequeno) != -1 {
        fallo(); // ni una entrada cabe: eso es error, no "fin de directorio"
    }
    let total = getdents64(fd as u64, &mut dirbuf);
    if total <= 0 {
        fallo();
    }
    let visto = &dirbuf[..total as usize];
    if !contiene(visto, b"copia.txt") || !contiene(visto, b"sub") {
        fallo();
    }
    if contiene(visto, b"nota.txt") {
        fallo(); // se acaba de mover dentro de `sub`
    }
    if tipo_de(visto, b"sub") != Some(DT_DIR) || tipo_de(visto, b"copia.txt") != Some(DT_REG) {
        fallo();
    }
    // El descriptor no se ha movido: la siguiente llamada esta al final.
    if getdents64(fd as u64, &mut dirbuf) != 0 {
        fallo();
    }
    cerrar(fd as u64);
    // Y el subdirectorio tambien se puede recorrer por su descriptor.
    let fd = abrir(SUBDIR.as_ptr(), O_RDONLY);
    if fd < 3 {
        fallo();
    }
    let total = getdents64(fd as u64, &mut dirbuf);
    cerrar(fd as u64);
    if total <= 0 || !contiene(&dirbuf[..total as usize], b"nota.txt") {
        fallo();
    }
    escribir(1, MSG_DIR);

    // 7. `unlink` sobre un directorio es un error (`EISDIR`), `rmdir` sobre uno
    //    no vacio tambien, y sobre un fichero tambien (`ENOTDIR`).
    if unlink(SUBDIR.as_ptr()) != -1 {
        fallo();
    }
    if rmdir(SUBDIR.as_ptr()) != -1 {
        fallo();
    }
    if unlink(MOVIDO.as_ptr()) != 0 {
        fallo();
    }
    if unlink(MOVIDO.as_ptr()) != -1 {
        fallo();
    }
    if unlink(COPIA.as_ptr()) != 0 {
        fallo();
    }
    if rmdir(SUBDIR.as_ptr()) != 0 {
        fallo();
    }
    if rmdir(DIR.as_ptr()) != 0 {
        fallo();
    }
    if stat(DIR.as_ptr(), &mut st) != -1 {
        fallo();
    }
    escribir(1, MSG_BORRAR);

    // 8. `access` responde si la ruta existe; `fstat` llega al mismo sitio que
    //    `stat` pero sin volver a recorrer la ruta.
    if access(RAIZ.as_ptr(), 0) != 0 {
        fallo();
    }
    if access(NADA.as_ptr(), 0) != -1 {
        fallo();
    }
    let fd = abrir(RAIZ.as_ptr(), O_RDONLY);
    if fd < 3 {
        fallo();
    }
    let r = fstat(fd as u64, &mut otra);
    cerrar(fd as u64);
    if r != 0 {
        fallo();
    }
    if otra.st_mode & S_IFMT != S_IFDIR || otra.st_ino == 0 {
        fallo();
    }
    escribir(1, MSG_FSTAT);

    // 9. `sync`: lo escrito en esta sesion queda limpito en el disco.
    if sync() != 0 {
        fallo();
    }

    escribir(1, OK);
    salir(0)
}
