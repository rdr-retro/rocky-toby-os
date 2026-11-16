//! `xtask` de rocky: compila el bootloader y el kernel, los convierte en
//! binarios planos, monta la imagen de disquete y (opcionalmente) lanza QEMU.
//!
//! Uso:
//!   cargo run --manifest-path xtask/Cargo.toml -- build
//!   cargo run --manifest-path xtask/Cargo.toml -- run
//!   cargo run --manifest-path xtask/Cargo.toml -- run-serial
//!   cargo run --manifest-path xtask/Cargo.toml -- run-whpx
//!   cargo run --manifest-path xtask/Cargo.toml -- run-usb
//!   cargo run --manifest-path xtask/Cargo.toml -- run-usb-serial
//!   cargo run --manifest-path xtask/Cargo.toml -- clean

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod cursor_assets;
mod ext4;
mod fat32;

const TARGET: &str = "x86_64-unknown-none";
const FLOPPY_SIZE: usize = 1474560; // 1.44 MB
const BOOT_SECTOR_SIZE: usize = 512;
/// Tamano de un sector de disco.
const SECTOR: usize = 512;
/// Sectores maximos que puede ocupar el kernel.
///
/// `carga_kernel` (en `bootusb2.s`) ya no reparte la imagen por ventanas de
/// RAM baja: lee cada lote en un buffer de rebote y lo copia a 1 MiB, asi que
/// no hay limite por ventana. El limite real es el hueco entre el kernel y la
/// particion ESP del pendrive: LBA 5..2047 (la ESP empieza en 2048).
/// En el disquete sobran sectores (2880 en total) y el mismo limite vale.
///
/// El fallback CHS parte el LBA en 16 bits con dos divisiones: con este tope
/// (LBA <= 2047) el cilindro nunca pasa de 255 y no hay desbordes.
const MAX_KERNEL_SECTORES: u64 = USB_ESP_LBA as u64 - USB_KERNEL_LBA as u64;
/// Tamano maximo del kernel (~1021 KiB).
const MAX_KERNEL_SIZE: usize = MAX_KERNEL_SECTORES as usize * SECTOR;
/// Tamano del disco duro virtual que rocky usa para los ficheros (32 MiB).
const DISK_SIZE: u64 = 32 * 1024 * 1024;
/// Sectores de una particion de logs FAT32 (64 MiB: hacen falta mas de 65525
/// clusters para que Windows la reconozca como FAT32).
const LOG_SECTORES: u32 = 131_072;
/// Etiqueta del volumen de logs.
const ETIQUETA_LOG: &str = "ROCKYLOG";

// --- Disco de trabajo `rockydisk.img` ---
/// LBA de la particion 1 (Estrato) del disco de trabajo.
const DISCO_VOLUMEN_LBA: u32 = 2048;
/// Sectores de la particion Estrato del disco de trabajo (32 MiB).
const DISCO_VOLUMEN_SECTORES: u32 = (DISK_SIZE / SECTOR as u64) as u32; // 65536
/// LBA de la particion 2 (logs FAT32) del disco de trabajo.
const DISCO_LOG_LBA: u32 = DISCO_VOLUMEN_LBA + DISCO_VOLUMEN_SECTORES; // 67584

// --- Pendrive arrancable `rocky-usb.img` ---
/// LBA de inicio de la particion 1 (EFI System Partition) del pendrive USB.
/// Debe coincidir con la entrada de particion del MBR de `bootusb.s`.
const USB_ESP_LBA: u32 = 2048;
/// Tamano de la ESP en sectores (64 MiB).
const USB_ESP_SECTORES: u32 = 131_072;
/// LBA del volumen Estrato en el pendrive (justo detras de la ESP).
const USB_VOLUMEN_LBA: u32 = USB_ESP_LBA + USB_ESP_SECTORES; // 133120
/// Tamano del volumen Estrato en el pendrive (32 MiB).
const USB_VOLUMEN_SECTORES: u32 = (DISK_SIZE / SECTOR as u64) as u32; // 65536
/// LBA de la particion de logs del pendrive (justo detras de Estrato).
const USB_LOG_LBA: u32 = USB_VOLUMEN_LBA + USB_VOLUMEN_SECTORES; // 198656
/// LBA del segundo tramo del arranque USB (`bootusb2.bin`).
const USB_TRAMO2_LBA: u32 = 1;
/// Sectores que ocupa el segundo tramo: el MBR los carga de golpe. Dejamos
/// margen para la ruta CHS de reserva de BIOS sin INT 13h Extensions.
const USB_TRAMO2_SECTORES: u32 = 4;
/// LBA donde empieza el kernel en el pendrive (justo detras del segundo
/// tramo). Debe coincidir con la DAP de `bootusb2.s`.
const USB_KERNEL_LBA: u32 = USB_TRAMO2_LBA + USB_TRAMO2_SECTORES; // 5
/// Marcador que `bootusb2.s` deja para parchear la configuracion de arranque.
const MARCA_CFG: &[u8; 6] = b"RKYCFG";
/// Nombres de los programas ELF64 que xtask instala en `/bin` del disco de
/// ficheros de rocky. Esta lista crece a medida que se va migrando cada
/// programa de los builders embebidos al disco.
const PROGRAMAS: &[&str] = &[
    "usuario",
    "hola",
    "fork",
    "exec",
    "fd",
    "excepcion",
    "memoria",
    "pipe",
    "senal",
    "tty",
    "cwd",
    "fs3",
    "kira_test",
];

/// Artefactos AppKit que el sistema necesita para no sustituir controles por
/// dibujos aproximados cuando cambia su estado o variante.
const RECURSOS_APPKIT_OBLIGATORIOS: &[&str] = &[
    "left_button_blue.png",
    "body_button_blue.png",
    "right_button_blue.png",
    "gray_radio.png",
    "blue_radio.png",
    "green_radio.png",
    "red_radio.png",
    "orange_radio.png",
    "yellow_radio.png",
    "magenta_radio.png",
    "white_radio.png",
    "indicator_left.png",
    "indicator_right.png",
    "segmented_withe_left.png",
    "segmented_withe_body.png",
    "segmented_withe_right.png",
    "segmented_blue_left.png",
    "segmented_blue_body.png",
    "segmented_blue_right.png",
    "icon_view.png",
    "list_view.png",
    "column_view.png",
    "gallery_view.png",
    "shooter.png",
    "indeterminate.png",
    "loader.png",
    "loader_center.png",
    "loader_back.png",
];

fn main() {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("build");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("sin directorio raiz")
        .to_path_buf();

    match command {
        "build" => {
            build(&root);
        }
        "run" => {
            build(&root);
            run_qemu(&root, true, false, false);
        }
        "run-serial" => {
            build(&root);
            run_qemu(&root, false, false, false);
        }
        "run-whpx" => {
            if !cfg!(windows) {
                eprintln!("run-whpx solo esta disponible en Windows (WHPX).");
                std::process::exit(1);
            }
            build(&root);
            run_qemu(&root, true, true, false);
        }
        // Configuracion USB del plan (fase 8): un controlador xHCI con un
        // teclado y un raton USB. No sustituye a las anteriores: son
        // comandos aparte, para no tocar la configuracion de arranque que ya
        // se usa.
        "run-usb" => {
            build(&root);
            run_qemu(&root, true, false, true);
        }
        "run-usb-serial" => {
            build(&root);
            run_qemu(&root, false, false, true);
        }
        "clean" => clean(&root),
        "cursor-assets" => match cursor_assets::generar(&root) {
            Ok(path) => println!("cursores binarios: {}", path.display()),
            Err(error) => {
                eprintln!("no se pudieron generar los cursores: {error}");
                std::process::exit(1);
            }
        },
        other => {
            eprintln!(
                "uso: xtask [build|run|run-serial|run-whpx|run-usb|run-usb-serial|clean|cursor-assets] (recibi '{other}')"
            );
            std::process::exit(1);
        }
    }
}

fn build(root: &Path) {
    cursor_assets::generar(root).expect("no se pudieron generar los cursores binarios");
    let out = root.join("build");
    fs::create_dir_all(&out).expect("no se pudo crear build/");

    let objcopy = find_llvm_objcopy();

    // --- Bootloader ---
    let boot_elf = compile_crate(root, "boot", "boot", &root.join("boot/boot.ld"));
    let boot_bin = out.join("boot.bin");
    objcopy_binary(&objcopy, &boot_elf, &boot_bin);
    let boot_len = fs::metadata(&boot_bin).expect("sin boot.bin").len() as usize;
    if boot_len != BOOT_SECTOR_SIZE {
        panic!("el sector de arranque mide {boot_len} bytes (deberian ser 512)");
    }

    // --- Kernel ---
    let kernel_elf = compile_crate(root, "kernel", "rocky", &root.join("kernel/kernel.ld"));
    let kernel_bin = out.join("kernel.bin");
    objcopy_binary(&objcopy, &kernel_elf, &kernel_bin);
    let kernel = fs::read(&kernel_bin).expect("sin kernel.bin");
    if kernel.len() > MAX_KERNEL_SIZE {
        panic!(
            "el kernel mide {} KiB y no cabe antes de la ESP ({} KiB)",
            kernel.len() / 1024,
            MAX_KERNEL_SIZE / 1024
        );
    }

    // --- Segundo tramo del arranque ---
    // Lo cargan el MBR del pendrive y el sector de arranque del disquete, los
    // dos en 0x8000. Es el que carga el kernel y entra en modo largo.
    let tramo2 = compilar_tramo2(root, &objcopy);

    let kernel_sectores = (kernel.len() as u64).div_ceil(SECTOR as u64);
    if kernel_sectores > MAX_KERNEL_SECTORES {
        panic!(
            "el kernel ocupa {kernel_sectores} sectores y solo caben {MAX_KERNEL_SECTORES} antes de la ESP"
        );
    }

    // --- Imagen de disquete de 1.44 MB ---
    // Mismo reparto que el pendrive: LBA 0 es el sector de arranque, LBA 1..4
    // el segundo tramo y el kernel a partir del LBA 5.
    let mut image = vec![0u8; FLOPPY_SIZE];
    let boot = fs::read(&boot_bin).unwrap();
    image[..BOOT_SECTOR_SIZE].copy_from_slice(&boot);
    // El disquete no lleva volumen Estrato: el kernel lo busca en el disco
    // ATA, que es donde lo deja el disco de trabajo.
    let mut tramo2_disquete = tramo2.clone();
    parchear_cfg(&mut tramo2_disquete, kernel_sectores as u16, 0, 0);
    let toff = BOOT_SECTOR_SIZE;
    image[toff..toff + tramo2_disquete.len()].copy_from_slice(&tramo2_disquete);
    let koff = toff + tramo2_disquete.len();
    image[koff..koff + kernel.len()].copy_from_slice(&kernel);

    let img_path = root.join("rocky.img");
    fs::write(&img_path, &image).expect("no se pudo escribir rocky.img");

    // --- Disco duro virtual para el sistema de ficheros Estrato ---
    let disk_path = construir_disco(root);

    // --- Pendrive USB arrancable (MBR de BIOS + ESP para UEFI) ---
    let usb_path = build_usb(root, &objcopy, &kernel, &disk_path, tramo2);

    println!();
    println!("  boot.bin   : {} bytes", boot_len);
    println!(
        "  kernel.bin : {} bytes ({} KiB)",
        kernel.len(),
        kernel.len() / 1024
    );
    println!("  imagen     : {}", img_path.display());
    println!(
        "  disco      : {} ({} MiB)",
        disk_path.display(),
        fs::metadata(&disk_path).map(|m| m.len()).unwrap_or(0) / (1024 * 1024)
    );
    println!(
        "  pendrive   : {} ({} MiB)",
        usb_path.display(),
        fs::metadata(&usb_path).map(|m| m.len()).unwrap_or(0) / (1024 * 1024)
    );
    println!();
}

/// Construye `rocky-usb.img`: la imagen en crudo del pendrive arrancable que
/// escribe el script `rocky-usb.ps1`.
///
/// El arranque va en **dos tramos**: el sector de arranque tiene que caber el
/// solo hasta el offset 446 (la tabla de particiones) y no habia sitio ni
/// para un mensaje de error; si la BIOS fallaba al leer, el codigo seguia
/// adelante y saltaba a memoria sin cargar. Ahora el MBR (LBA 0) solo lleva
/// el BPB, carga el segundo tramo y salta a el; el segundo tramo (LBA 1)
/// carga el kernel, lo cuenta por pantalla y entra en modo largo.
///
/// Layout (todos los LBA en sectores de 512 bytes):
///
/// ```text
///   LBA 0             primer tramo: sector de arranque BIOS (MBR) + tabla
///   LBA 1 .. 4        segundo tramo (bootusb2.bin): carga el kernel
///   LBA 5 .. 2047     kernel.bin (hueco antes de la ESP)
///   LBA 2048 ..       particion 1: ESP (reservada; aun sin cargador UEFI)
///   LBA 133120 ..     particion 2: volumen Estrato (copia de la particion 1
///                     del disco de trabajo)
///   LBA 198656 ..     particion 3: logs FAT32 (COMANDOS.LOG), para Windows
/// ```
fn build_usb(
    root: &Path,
    objcopy: &Path,
    kernel: &[u8],
    disk_path: &Path,
    mut tramo2: Vec<u8>,
) -> PathBuf {
    let out = root.join("build");

    // --- Primer tramo: el sector de arranque (512 bytes) ---
    let bootusb_elf = compile_crate(root, "boot", "bootusb", &root.join("boot/bootusb.ld"));
    let bootusb_bin = out.join("bootusb.bin");
    objcopy_binary(objcopy, &bootusb_elf, &bootusb_bin);
    let mut boot = fs::read(&bootusb_bin).expect("sin bootusb.bin");
    if boot.len() != BOOT_SECTOR_SIZE {
        panic!(
            "el sector de arranque USB mide {} bytes (deberian ser 512)",
            boot.len()
        );
    }

    // --- Segundo tramo: el que carga el kernel y lo cuenta por pantalla ---
    let kernel_sectores = (kernel.len() as u64).div_ceil(SECTOR as u64);
    if kernel_sectores > u16::MAX as u64 {
        panic!("el kernel ocupa {kernel_sectores} sectores y no cabe en 16 bits");
    }
    if USB_KERNEL_LBA as u64 + kernel_sectores > USB_ESP_LBA as u64 {
        panic!(
            "el kernel ({kernel_sectores} sectores desde el LBA {USB_KERNEL_LBA}) invade la particion ESP (LBA {USB_ESP_LBA})"
        );
    }

    parchear_cfg(
        &mut tramo2,
        kernel_sectores as u16,
        USB_VOLUMEN_LBA,
        USB_VOLUMEN_SECTORES,
    );
    escribir_tabla(
        &mut boot,
        &[
            Particion {
                activa: true,
                tipo: 0xEF,
                lba: USB_ESP_LBA,
                sectores: USB_ESP_SECTORES,
            },
            Particion {
                activa: false,
                tipo: 0x0C,
                lba: USB_LOG_LBA,
                sectores: LOG_SECTORES,
            },
        ],
    );

    let total_sectores = USB_LOG_LBA as u64 + LOG_SECTORES as u64;
    let mut img = vec![0u8; total_sectores as usize * SECTOR];

    // LBA 0: primer tramo (ya lleva la tabla de particiones).
    img[..SECTOR].copy_from_slice(&boot);
    // LBA 1..: segundo tramo (el que carga el kernel).
    let toff = USB_TRAMO2_LBA as usize * SECTOR;
    img[toff..toff + tramo2.len()].copy_from_slice(&tramo2);
    // LBA 3..: el kernel, tal cual.
    let koff = USB_KERNEL_LBA as usize * SECTOR;
    img[koff..koff + kernel.len()].copy_from_slice(kernel);
    // Volumen Estrato: copia de la particion 1 del disco de trabajo (no de la
    // imagen entera, que ahora lleva delante su propio MBR y, detras, los logs).
    let eoff = USB_VOLUMEN_LBA as usize * SECTOR;
    if let Ok(data) = fs::read(disk_path) {
        let ini = DISCO_VOLUMEN_LBA as usize * SECTOR;
        let n = (DISCO_VOLUMEN_SECTORES as usize * SECTOR).min(data.len().saturating_sub(ini));
        if n > 0 {
            img[eoff..eoff + n].copy_from_slice(&data[ini..ini + n]);
        }
    }
    // Particion de logs: un volumen FAT32 recien formateado.
    let loff = USB_LOG_LBA as usize * SECTOR;
    let llen = LOG_SECTORES as usize * SECTOR;
    fat32::formatear(&mut img[loff..loff + llen], USB_LOG_LBA, ETIQUETA_LOG);

    let path = root.join("rocky-usb.img");
    fs::write(&path, &img).expect("no se pudo escribir rocky-usb.img");
    path
}

/// Compila el segundo tramo del arranque (`bootusb2.bin`) y lo devuelve con el
/// tamano que le reservan los cargadores (cuatro sectores).
///
/// Lo usan las dos imagenes: el MBR del pendrive y el sector de arranque del
/// disquete. Los dos lo cargan en 0x8000 y los dos esperan que el kernel
/// este en el LBA 5.
fn compilar_tramo2(root: &Path, objcopy: &Path) -> Vec<u8> {
    let out = root.join("build");
    let tramo2_elf = compile_crate(root, "boot", "bootusb2", &root.join("boot/bootusb2.ld"));
    let tramo2_bin = out.join("bootusb2.bin");
    objcopy_binary(objcopy, &tramo2_elf, &tramo2_bin);
    let mut tramo2 = fs::read(&tramo2_bin).expect("sin bootusb2.bin");
    let bytes = USB_TRAMO2_SECTORES as usize * SECTOR;
    if tramo2.len() > bytes {
        panic!(
            "el segundo tramo mide {} bytes y los cargadores solo leen {bytes} ({USB_TRAMO2_SECTORES} sectores)",
            tramo2.len()
        );
    }
    tramo2.resize(bytes, 0);
    tramo2
}

/// Escribe `kernel_sectors` (u16), `estrato_lba` (u32) y `estrato_sectores`
/// (u32) justo detras de la marca `RKYCFG` del segundo tramo de arranque.
fn parchear_cfg(sector: &mut [u8], kernel_sectors: u16, estrato_lba: u32, estrato_sectores: u32) {
    let pos = sector
        .windows(MARCA_CFG.len())
        .position(|w| w == &MARCA_CFG[..])
        .expect("no se encontro la marca RKYCFG en el segundo tramo del arranque USB");
    let mut off = pos + MARCA_CFG.len();
    sector[off..off + 2].copy_from_slice(&kernel_sectors.to_le_bytes());
    off += 2;
    sector[off..off + 4].copy_from_slice(&estrato_lba.to_le_bytes());
    off += 4;
    sector[off..off + 4].copy_from_slice(&estrato_sectores.to_le_bytes());
}

/// Una entrada de la tabla de particiones del MBR.
struct Particion {
    /// Marca de arranque (`0x80`).
    activa: bool,
    /// Tipo de particion (`0xEF` ESP, `0x83` Linux/Estrato, `0x0C` FAT32 LBA).
    tipo: u8,
    /// LBA del primer sector.
    lba: u32,
    /// Sectores que ocupa.
    sectores: u32,
}

/// Escribe la tabla de particiones del MBR (cuatro entradas, bytes 446..510).
///
/// Solo toca la tabla y la firma: el resto del sector (el codigo de arranque)
/// se respeta, que es justo lo que necesita `bootusb.s`.
fn escribir_tabla(sector: &mut [u8], partes: &[Particion]) {
    const P: usize = 446;
    assert!(
        partes.len() <= 4,
        "el MBR solo tiene sitio para cuatro particiones"
    );
    for byte in &mut sector[P..P + 64] {
        *byte = 0;
    }
    for (i, p) in partes.iter().enumerate() {
        let o = P + i * 16;
        sector[o] = if p.activa { 0x80 } else { 0x00 };
        sector[o + 1..o + 4].copy_from_slice(&chs(p.lba));
        sector[o + 4] = p.tipo;
        sector[o + 5..o + 8].copy_from_slice(&chs(p.lba + p.sectores - 1));
        sector[o + 8..o + 12].copy_from_slice(&p.lba.to_le_bytes());
        sector[o + 12..o + 16].copy_from_slice(&p.sectores.to_le_bytes());
    }
    sector[510] = 0x55;
    sector[511] = 0xAA;
}

/// CHS aproximado de una LBA (el que pondria cualquier herramienta).
///
/// Los discos grandes se salen del CHS, que solo llega a 1023 cilindros; en
/// ese caso se pone el tope (`0xFE 0xFF 0xFF`), que es lo que hacen los
/// formateadores: la LBA es la que manda.
fn chs(lba: u32) -> [u8; 3] {
    let cil = lba / (255 * 63);
    let resto = lba % (255 * 63);
    let cab = resto / 63;
    let sec = (resto % 63) + 1;
    if cil > 1023 {
        return [0xFE, 0xFF, 0xFF];
    }
    [
        cab as u8,
        ((sec & 0x3F) | ((cil >> 8) << 6)) as u8,
        (cil & 0xFF) as u8,
    ]
}

/// Crea el disco duro virtual si no existe todavia y devuelve su ruta. El
/// kernel lo formatea la primera vez que arranca (o lo usa si ya lo esta).
///
/// Crea `rockydisk.img` con un volumen **ext4 ya formateado** y con los
/// ficheros que rocky debe encontrar nada mas arrancar. Al hacerlo desde el
/// anfitrion, los recursos grandes (aqui `isma.png`, de 2,4 MiB) no tienen que
/// caber dentro del kernel. El escritor de `ext4.rs` vuelca el mismo formato
/// que produce `mke2fs` de Linux.
fn construir_disco(root: &Path) -> PathBuf {
    let path = root.join("rockydisk.img");
    let total = DISK_SIZE / SECTOR as u64;
    let mut vol = ext4::Volumen::formatear(total, "ROCKY");

    let home = vol.crear_dirs("/home").expect("no se pudo crear /home");
    let escritorio = vol
        .crear_dirs("/home/escritorio")
        .expect("no se pudo crear /home/escritorio");
    let acceso_directo_apps = root.join("desktop").join("Apps.tobylink");
    if let Ok(datos) = fs::read(&acceso_directo_apps) {
        vol.escribir_fichero(escritorio, "Apps.tobylink", &datos)
            .expect("no se pudo crear el acceso directo de Apps");
    }
    // Las apps viven SOLO aqui: importamos los bundles .app del directorio
    // fuente del proyecto a /home/apps dentro del volumen Estrato.
    vol.crear_dirs("/home/apps")
        .expect("no se pudo crear /home/apps");
    vol.crear_dirs("/home/papelera")
        .expect("no se pudo crear /home/papelera");
    importar_apps(root, &mut vol);
    vol.crear_dirs("/system/apps")
        .expect("no se pudo crear /system/apps");
    importar_apps_sistema(root, &mut vol);
    let fuentes = vol
        .crear_dirs("/usr/share/fonts")
        .expect("no se pudo crear /usr/share/fonts");
    importar_fuentes(root, &mut vol, fuentes);
    let fondos = vol
        .crear_dirs("/usr/share/backgrounds")
        .expect("no se pudo crear /usr/share/backgrounds");
    importar_archivos_share(
        &root.join("AQUA").join("backgrounds"),
        &mut vol,
        fondos,
        "/usr/share/backgrounds",
    );
    let bin = vol.crear_dirs("/bin").expect("no se pudo crear /bin");
    // Recursos del tema del sistema (documentos base y, mas adelante, PNG).
    let tema = vol
        .crear_dirs("/usr/share/tobyui")
        .expect("no se pudo crear /usr/share/tobyui");

    // Fichero de datos que el programa `fd` abre, lee y cierra (open/read/close).
    let mensaje = b"rocky: este texto lo ha leido un proceso de anillo 3 con open/read.\n";
    vol.escribir_fichero(bin, "mensaje", mensaje)
        .expect("no se pudo guardar /bin/mensaje");
    println!(
        "  {:<11} : {} bytes -> /bin/mensaje",
        "mensaje",
        mensaje.len()
    );

    let png = root.join("kernel").join("isma.png");
    match fs::read(&png) {
        Ok(datos) => {
            vol.escribir_fichero(home, "isma.png", &datos)
                .expect("no se pudo guardar isma.png en el volumen");
            println!("  isma.png   : {} bytes -> /home/isma.png", datos.len());
        }
        Err(_) => println!("  (aviso) no encontre kernel/isma.png"),
    }

    // El tema del sistema: la demo de Toby UI, para que `App::desde_fs` tenga
    // algo real que montar desde el disco.
    let demo = root
        .join("kernel")
        .join("src")
        .join("toby_ui")
        .join("demo.toby");
    match fs::read(&demo) {
        Ok(datos) => {
            vol.escribir_fichero(tema, "demo.toby", &datos)
                .expect("no se pudo guardar demo.toby");
            println!(
                "  demo.toby  : {} bytes -> /usr/share/tobyui/demo.toby",
                datos.len()
            );
        }
        Err(_) => println!("  (aviso) no encontre kernel/src/toby_ui/demo.toby"),
    }

    // Kit AQUA: las piezas PNG del kit (botones de 3 slices por variante de
    // color) tal cual vienen, en la carpeta que autoriza el lenguaje. El tema
    // las ensambla en tiempo de pintado (extremos + cuerpo estirado).
    let tema_default = vol
        .crear_dirs("/usr/share/tobyui/themes/default")
        .expect("no se pudo crear /usr/share/tobyui/themes/default");
    let aqua = root.join("AQUA");
    let prefigos = ["left_button", "body_button", "right_button"];
    let colores = [
        "blue", "green", "red", "orange", "yellow", "magenta", "white",
    ];
    let mut n_piezas = 0usize;
    for color in colores {
        // Las tres piezas azules son obligatorias y se copian abajo junto con
        // los demás recursos que necesita AppKit; los otros colores son extras.
        if color == "blue" {
            continue;
        }
        for pref in prefigos {
            let nombre = format!("{}_{}.png", pref, color);
            if let Ok(datos) = fs::read(aqua.join(&nombre)) {
                if vol
                    .escribir_fichero(tema_default, &nombre, &datos)
                    .is_some()
                {
                    n_piezas += 1;
                }
            }
        }
    }
    for nombre in RECURSOS_APPKIT_OBLIGATORIOS {
        let ruta = aqua.join(nombre);
        let datos = fs::read(&ruta)
            .unwrap_or_else(|e| panic!("falta el recurso AppKit {}: {e}", ruta.display()));
        vol.escribir_fichero(tema_default, nombre, &datos)
            .unwrap_or_else(|| panic!("no se pudo instalar el recurso AppKit {nombre}"));
        n_piezas += 1;
    }
    println!(
        "  kit AQUA   : {} piezas -> /usr/share/tobyui/themes/default/",
        n_piezas
    );

    for (nombre, datos) in compilar_programas(root) {
        vol.escribir_fichero(bin, &nombre, &datos)
            .expect("no se pudo guardar un programa en /bin");
    }

    // La imagen lleva una tabla de particiones de verdad para que Windows la
    // pueda abrir: la 1 es el sistema Estrato y la 2 el volumen de logs FAT32.
    // El kernel no da por hecho el LBA 0: lee esta tabla al arrancar.
    let total = DISCO_VOLUMEN_LBA as u64 + DISCO_VOLUMEN_SECTORES as u64 + LOG_SECTORES as u64;
    let mut img = vec![0u8; total as usize * SECTOR];
    escribir_tabla(
        &mut img[..SECTOR],
        &[
            Particion {
                activa: true,
                tipo: 0x83,
                lba: DISCO_VOLUMEN_LBA,
                sectores: DISCO_VOLUMEN_SECTORES,
            },
            Particion {
                activa: false,
                tipo: 0x0C,
                lba: DISCO_LOG_LBA,
                sectores: LOG_SECTORES,
            },
        ],
    );

    let vo = DISCO_VOLUMEN_LBA as usize * SECTOR;
    let estrato = vol.bytes();
    assert_eq!(
        estrato.len(),
        DISCO_VOLUMEN_SECTORES as usize * SECTOR,
        "el volumen Estrato no mide lo que ocupa su particion"
    );
    img[vo..vo + estrato.len()].copy_from_slice(estrato);

    let lo = DISCO_LOG_LBA as usize * SECTOR;
    let ln = LOG_SECTORES as usize * SECTOR;
    let fmt = fat32::formatear(&mut img[lo..lo + ln], DISCO_LOG_LBA, ETIQUETA_LOG);
    println!(
        "  logs       : {} en la particion 2 (LBA {}, {} sectores, {} clusters, FAT de {} sectores, datos en {})",
        fat32::FICHERO_LOG,
        DISCO_LOG_LBA,
        fmt.sectores,
        fmt.clusters,
        fmt.fat_sectores,
        fmt.offset_datos
    );

    fs::write(&path, &img).expect("no se pudo escribir rockydisk.img");
    path
}

/// Copia los bundles de `apps/*.app` al volumen para que el kernel pueda
/// abrirlos desde `/home/apps`. Se excluyen symlinks y artefactos de build.
fn importar_apps(root: &Path, vol: &mut ext4::Volumen) {
    importar_apps_desde(root, "apps", "/home/apps", vol);
}

/// Copia los bundles de `system_apps/*.app` a `/system/apps`.
fn importar_apps_sistema(root: &Path, vol: &mut ext4::Volumen) {
    importar_apps_desde(root, "system_apps", "/system/apps", vol);
}

/// Instala fuentes originales, cachés bitmap RKF1 y cachés tipográficas RKA3.
fn importar_fuentes(root: &Path, vol: &mut ext4::Volumen, destino: u32) {
    let origen = root.join("Fonts");
    let Ok(entradas) = fs::read_dir(&origen) else {
        println!("  (aviso) no encontre la carpeta {}", origen.display());
        return;
    };
    let mut cantidad = 0usize;
    let mut bytes_total = 0usize;
    for entrada in entradas.flatten() {
        let ruta = entrada.path();
        if !entrada.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let extension = ruta
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "ttf" | "otf" | "rkf" | "rka" | "txt") {
            continue;
        }
        let nombre = entrada.file_name().to_string_lossy().to_ascii_lowercase();
        let datos = fs::read(&ruta)
            .unwrap_or_else(|e| panic!("no se pudo leer la fuente {}: {e}", ruta.display()));
        vol.escribir_fichero(destino, &nombre, &datos)
            .unwrap_or_else(|| panic!("no se pudo instalar {} en /usr/share/fonts", nombre));
        bytes_total += datos.len();
        cantidad += 1;
    }
    println!(
        "  fuentes    : {} archivos ({} bytes) -> /usr/share/fonts/",
        cantidad, bytes_total
    );
}

/// Copia los recursos de fondo de AQUA a una carpeta compartida del sistema.
fn importar_archivos_share(
    origen: &Path,
    vol: &mut ext4::Volumen,
    destino: u32,
    ruta_destino: &str,
) -> (usize, usize) {
    let Ok(entradas) = fs::read_dir(origen) else {
        println!("  (aviso) no encontre la carpeta {}", origen.display());
        return (0, 0);
    };
    let mut entradas: Vec<_> = entradas
        .flatten()
        .filter(|entrada| {
            entrada.file_type().is_ok_and(|tipo| tipo.is_file())
                && !entrada.file_name().to_string_lossy().starts_with('.')
        })
        .collect();
    entradas.sort_by_key(|entrada| entrada.file_name());

    let mut cantidad = 0usize;
    let mut bytes_total = 0usize;
    for entrada in entradas {
        let nombre = entrada.file_name().to_string_lossy().into_owned();
        let ruta = entrada.path();
        let datos = fs::read(&ruta)
            .unwrap_or_else(|e| panic!("no se pudo leer el recurso {}: {e}", ruta.display()));
        vol.escribir_fichero(destino, &nombre, &datos)
            .unwrap_or_else(|| panic!("no se pudo instalar {nombre} en {ruta_destino}"));
        bytes_total += datos.len();
        cantidad += 1;
    }
    println!(
        "  fondos AQUA: {} archivos ({} bytes) -> {}/",
        cantidad,
        bytes_total,
        ruta_destino.trim_end_matches('/')
    );
    (cantidad, bytes_total)
}

#[cfg(test)]
mod pruebas_recursos_share {
    use super::importar_archivos_share;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn importa_fondos_con_espacios_en_el_nombre() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let origen = std::env::temp_dir().join(format!("rocky-backgrounds-{nonce}"));
        fs::create_dir_all(&origen).unwrap();
        fs::write(origen.join("flower intense blue.png"), b"png uno").unwrap();
        fs::write(origen.join("flower.png"), b"png dos").unwrap();
        fs::write(origen.join(".DS_Store"), b"oculto").unwrap();

        let mut volumen = crate::ext4::Volumen::formatear(8192, "TEST");
        let destino = volumen.crear_dirs("/usr/share/backgrounds").unwrap();
        let resultado =
            importar_archivos_share(&origen, &mut volumen, destino, "/usr/share/backgrounds");

        assert_eq!(resultado, (2, 14));
        drop(volumen);
        fs::remove_dir_all(origen).unwrap();
    }

    #[test]
    fn empaqueta_los_fondos_aqua_del_proyecto_en_usr_share() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let origen = root.join("AQUA").join("backgrounds");
        let archivos: Vec<_> = fs::read_dir(&origen)
            .unwrap()
            .flatten()
            .filter(|entrada| entrada.file_type().is_ok_and(|tipo| tipo.is_file()))
            .collect();
        let bytes_esperados: usize = archivos
            .iter()
            .map(|entrada| fs::metadata(entrada.path()).unwrap().len() as usize)
            .sum();

        let mut volumen = crate::ext4::Volumen::formatear(32_768, "TEST");
        let destino = volumen.crear_dirs("/usr/share/backgrounds").unwrap();
        let (cantidad, bytes) =
            importar_archivos_share(&origen, &mut volumen, destino, "/usr/share/backgrounds");

        assert_eq!(cantidad, archivos.len());
        assert_eq!(bytes, bytes_esperados);
    }
}

#[cfg(test)]
mod pruebas_recursos_appkit {
    use super::RECURSOS_APPKIT_OBLIGATORIOS;
    use std::path::PathBuf;

    #[test]
    fn todos_los_recursos_appkit_obligatorios_estan_en_el_kit() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        for recurso in RECURSOS_APPKIT_OBLIGATORIOS {
            assert!(
                root.join("AQUA").join(recurso).is_file(),
                "falta el recurso AppKit obligatorio: {recurso}"
            );
        }
    }
}

fn importar_apps_desde(root: &Path, carpeta: &str, destino_base: &str, vol: &mut ext4::Volumen) {
    let origen = root.join(carpeta);
    let Ok(entradas) = fs::read_dir(&origen) else {
        return;
    };
    for entrada in entradas.flatten() {
        let ruta = entrada.path();
        let nombre = entrada.file_name().to_string_lossy().into_owned();
        if !nombre.ends_with(".app") || !entrada.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if !nombre_bundle_valido(&nombre) {
            eprintln!("  (aviso) se omite bundle con nombre no válido: {nombre}");
            continue;
        }
        let destino = format!("{destino_base}/{nombre}");
        vol.crear_dirs(&destino)
            .unwrap_or_else(|| panic!("no se pudo crear {destino} en Estrato"));
        copiar_arbol_app(vol, &ruta, &destino);
        println!("  app        : {nombre} -> {destino}");
    }
}

fn nombre_bundle_valido(carpeta: &str) -> bool {
    let Some(nombre) = carpeta.strip_suffix(".app") else {
        return false;
    };
    let bytes = nombre.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

fn copiar_arbol_app(vol: &mut ext4::Volumen, origen: &Path, destino: &str) {
    let entradas = fs::read_dir(origen)
        .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", origen.display()));
    for entrada in entradas.flatten() {
        let ruta = entrada.path();
        let nombre = entrada.file_name().to_string_lossy().into_owned();
        if nombre.starts_with('.') || nombre == "target" {
            continue;
        }
        let tipo = match entrada.file_type() {
            Ok(t) if t.is_dir() => 1,
            Ok(t) if t.is_file() => 2,
            _ => continue,
        };
        if nombre.is_empty() || nombre.contains('/') || nombre.contains('\\') {
            continue;
        }
        let ruta_destino = format!("{destino}/{nombre}");
        if tipo == 1 {
            vol.crear_dirs(&ruta_destino)
                .unwrap_or_else(|| panic!("no se pudo crear {ruta_destino} en Estrato"));
            copiar_arbol_app(vol, &ruta, &ruta_destino);
        } else {
            let datos = fs::read(&ruta)
                .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", ruta.display()));
            let padre = vol
                .crear_dirs(destino)
                .unwrap_or_else(|| panic!("no se pudo abrir {destino} en Estrato"));
            vol.escribir_fichero(padre, &nombre, &datos)
                .unwrap_or_else(|| panic!("no hay espacio para {ruta_destino} en Estrato"));
        }
    }
}

#[cfg(test)]
mod pruebas_apps {
    use super::{importar_apps, nombre_bundle_valido};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn solo_importa_nombres_app_seguros() {
        assert!(nombre_bundle_valido("reloj.app"));
        assert!(nombre_bundle_valido("_herramientas-2.app"));
        assert!(!nombre_bundle_valido("Reloj.app"));
        assert!(!nombre_bundle_valido("../reloj.app"));
        assert!(!nombre_bundle_valido("reloj.rapp"));
        assert!(!nombre_bundle_valido(".app"));
    }

    #[test]
    fn importa_el_arbol_de_un_bundle_a_estrato() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let raiz = std::env::temp_dir().join(format!("rocky-appkit-import-{nonce}"));
        let app = raiz.join("apps/reloj.app");
        fs::create_dir_all(app.join("ui")).unwrap();
        fs::create_dir_all(app.join("code/src")).unwrap();
        fs::write(app.join("manifest.xml"), b"<app/>").unwrap();
        fs::write(app.join("ui/main.toby"), b"<toby-app/>").unwrap();
        fs::write(app.join("code/src/main.rs"), b"fn main() {}").unwrap();

        let mut volumen = crate::ext4::Volumen::formatear(8192, "TEST");
        volumen.crear_dirs("/home/apps").unwrap();
        importar_apps(&raiz, &mut volumen);
        drop(volumen);
        fs::remove_dir_all(raiz).unwrap();
    }
}

#[cfg(test)]
mod pruebas_fuentes {
    use std::fs;

    #[test]
    fn cada_fuente_tiene_caches_bitmap_y_antialias_validas() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let directorio = root.join("Fonts");
        let mut originales = 0;
        let mut caches = 0;
        let mut caches_suaves = 0;
        for entrada in fs::read_dir(&directorio).unwrap().flatten() {
            let ruta = entrada.path();
            if !entrada.file_type().unwrap().is_file() {
                continue;
            }
            let extension = ruta
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(extension.as_str(), "ttf" | "otf") {
                originales += 1;
                assert!(
                    ruta.with_extension("rkf").is_file(),
                    "falta cache para {}",
                    ruta.display()
                );
                assert!(
                    ruta.with_extension("rka").is_file(),
                    "falta cache antialias para {}",
                    ruta.display()
                );
            } else if extension == "rkf" {
                caches += 1;
                let bytes = fs::read(&ruta).unwrap();
                assert_eq!(bytes.get(..8), Some(&b"RKF1\x08\x10\x00\x01"[..]));
                assert_eq!(bytes.len(), 4104);
                assert!(bytes[8 + 32 * 16..8 + 33 * 16].iter().all(|b| *b == 0));
                assert!(bytes[8 + b'A' as usize * 16..8 + (b'A' as usize + 1) * 16]
                    .iter()
                    .any(|b| *b != 0));
            } else if extension == "rka" {
                let bytes = fs::read(&ruta).unwrap();
                let nombre = ruta
                    .file_name()
                    .and_then(|nombre| nombre.to_str())
                    .unwrap_or("");
                if nombre.eq_ignore_ascii_case("times-textedit.rka") {
                    let inicio_avances = 8;
                    let inicio_glifos = inicio_avances + 256;
                    assert_eq!(bytes.get(..8), Some(&b"RKA4\x14\x18\x00\x01"[..]));
                    assert_eq!(bytes.len(), inicio_glifos + 20 * 24 * 256);
                    assert!(
                        bytes[inicio_avances + b'i' as usize]
                            < bytes[inicio_avances + b'W' as usize],
                        "Times TextEdit debe conservar avances proporcionales"
                    );
                    let glifo_a = &bytes[inicio_glifos + b'A' as usize * 20 * 24
                        ..inicio_glifos + (b'A' as usize + 1) * 20 * 24];
                    assert!(glifo_a.iter().any(|b| *b != 0));
                    assert!(glifo_a.iter().any(|b| *b > 0 && *b < 255));
                } else if nombre.eq_ignore_ascii_case("about-title.rka") {
                    assert_eq!(bytes.get(..4), Some(&b"RKT1"[..]));
                    let palabra = |inicio: usize| {
                        u16::from_le_bytes([bytes[inicio], bytes[inicio + 1]]) as usize
                    };
                    let alto = palabra(4);
                    let anchos = [palabra(6), palabra(8), palabra(10)];
                    assert!(alto > 0 && alto <= 96);
                    assert!(anchos.iter().all(|ancho| *ancho > 0 && *ancho <= 512));
                    let pixeles: usize = anchos.iter().map(|ancho| ancho * alto).sum();
                    assert_eq!(bytes.len(), 12 + pixeles);
                    assert!(bytes[12..].iter().any(|pixel| *pixel > 0 && *pixel < 255));
                } else {
                    caches_suaves += 1;
                    assert_eq!(bytes.get(..4), Some(&b"RKA3"[..]));
                    assert_eq!(bytes.get(6..8), Some(&[0, 1][..]));
                    let ancho = bytes[4] as usize;
                    let alto = bytes[5] as usize;
                    assert!(ancho > 0 && ancho <= 32 && alto > 0 && alto <= 48);
                    let inicio_avances = 8;
                    let inicio_kerning = inicio_avances + 256;
                    let inicio_glifos = inicio_kerning + 256 * 256;
                    assert_eq!(bytes.len(), inicio_glifos + ancho * alto * 256);
                    assert!(bytes[inicio_avances..inicio_kerning].iter().any(|b| *b > 0));
                    if nombre.eq_ignore_ascii_case("SFPRODISPLAYREGULAR.rka") {
                        assert_eq!((ancho, alto), (12, 24));
                        let av = inicio_kerning + b'A' as usize * 256 + b'V' as usize;
                        assert!(
                            (bytes[av] as i8) < 0,
                            "SF Pro Regular debe ajustar el par AV"
                        );
                        assert!(
                            bytes[inicio_avances + b'i' as usize]
                                < bytes[inicio_avances + b'W' as usize],
                            "SF Pro Regular debe conservar avances proporcionales"
                        );
                    }
                    let tamano_glifo = ancho * alto;
                    let glifo_a = &bytes[inicio_glifos + b'A' as usize * tamano_glifo
                        ..inicio_glifos + (b'A' as usize + 1) * tamano_glifo];
                    assert!(glifo_a.iter().any(|b| *b != 0));
                    assert!(glifo_a.iter().any(|b| *b > 0 && *b < 255));
                }
            }
        }
        assert!(originales > 0);
        assert_eq!(caches, originales);
        assert_eq!(caches_suaves, originales);
    }
}

fn compile_crate(root: &Path, dir: &str, bin: &str, linker: &Path) -> PathBuf {
    let crate_dir = root.join(dir);
    let manifest = crate_dir.join("Cargo.toml");

    let rustflags = format!(
        "-Crelocation-model=static -Cpanic=abort -Ccode-model=kernel \
         -Clink-arg=-T -Clink-arg={}",
        linker.display()
    );

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(TARGET)
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--bin")
        .arg(bin)
        .env("RUSTFLAGS", rustflags)
        .status()
        .expect("no se pudo ejecutar cargo");

    if !status.success() {
        panic!("fallo al compilar '{dir}'");
    }

    let elf = crate_dir
        .join("target")
        .join(TARGET)
        .join("release")
        .join(bin);
    if !elf.exists() {
        panic!("no se encontro el ELF esperado en {}", elf.display());
    }
    elf
}

fn compilar_programas(root: &Path) -> Vec<(String, Vec<u8>)> {
    let crate_dir = root.join("programas");
    let manifest = crate_dir.join("Cargo.toml");
    // El SDK y los programas incluidos comparten una sola ABI de enlace.
    let linker = root.join("kernel/src/libraries/toby_appkit/rocky.ld");
    let rustflags = format!(
        "-Crelocation-model=pic -Cpanic=abort -Clink-arg=-T -Clink-arg={} -Clink-arg=-static",
        linker.display()
    );
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(TARGET)
        .arg("--manifest-path")
        .arg(&manifest)
        .env("RUSTFLAGS", rustflags)
        .status()
        .expect("no se pudo ejecutar cargo");
    if !status.success() {
        panic!("fallo al compilar los programas de usuario");
    }
    let dir = crate_dir.join("target").join(TARGET).join("release");
    let mut out = Vec::new();
    for &nombre in PROGRAMAS {
        let elf = dir.join(nombre);
        let datos = fs::read(&elf).expect("no se pudo leer el ELF del programa");
        println!("  {:<9} : {} bytes -> /bin/{}", nombre, datos.len(), nombre);
        out.push((nombre.to_string(), datos));
    }
    out
}

fn objcopy_binary(objcopy: &Path, elf: &Path, out: &Path) {
    let status = Command::new(objcopy)
        .arg("-O")
        .arg("binary")
        .arg(elf)
        .arg(out)
        .status()
        .expect("no se pudo ejecutar llvm-objcopy");
    if !status.success() {
        panic!("llvm-objcopy fallo para {}", elf.display());
    }
}

fn find_llvm_objcopy() -> PathBuf {
    let sysroot = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("no se pudo ejecutar rustc");
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());

    let rustlib = sysroot.join("lib").join("rustlib");
    if let Ok(entries) = fs::read_dir(&rustlib) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("bin").join("llvm-objcopy.exe");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // Ultimo recurso: confiar en el PATH.
    PathBuf::from("llvm-objcopy")
}

fn find_qemu() -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Instalacion por usuario. Deja aqui un QEMU portable y no hace falta tocar
    // el PATH:  %LOCALAPPDATA%\Programs\qemu\qemu-system-x86_64.exe
    if let Ok(local) = env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Programs")
                .join("qemu")
                .join("qemu-system-x86_64.exe"),
        );
    }
    // Instalacion via scoop (por comodidad).
    if let Ok(profile) = env::var("USERPROFILE") {
        candidates.push(
            PathBuf::from(&profile)
                .join("scoop")
                .join("apps")
                .join("qemu")
                .join("current")
                .join("qemu-system-x86_64.exe"),
        );
    }

    // Instalaciones clasicas del sistema.
    candidates.push(PathBuf::from(
        r"C:\Program Files\qemu\qemu-system-x86_64.exe",
    ));
    candidates.push(PathBuf::from(
        r"C:\Program Files (x86)\qemu\qemu-system-x86_64.exe",
    ));

    // Junto al proyecto: ./qemu/qemu-system-x86_64.exe (QEMU portable).
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("qemu")
            .join("qemu-system-x86_64.exe"),
    );

    for c in candidates {
        if c.exists() {
            return c;
        }
    }

    // Ultimo recurso: confiar en el PATH.
    PathBuf::from("qemu-system-x86_64")
}

/// Arranca QEMU.
///
/// `usb` añade el banco de pruebas USB del plan (fase 8): un controlador
/// xHCI (`qemu-xhci`, clase PCI `0C/03/30`) con un teclado y un ratón USB
/// colgando de él. Es la forma de ejercitar el HCD xHCI de rocky en QEMU,
/// porque la máquina `pc` de QEMU no trae ningún controlador USB por
/// defecto.
fn run_qemu(root: &Path, window: bool, whpx: bool, usb: bool) {
    let qemu = find_qemu();
    let img = root.join("rocky.img");
    let disk = root.join("rockydisk.img");

    let mut cmd = Command::new(&qemu);
    if whpx {
        // Acelera la ejecucion de instrucciones del invitado usando el
        // hipervisor de Windows; no convierte la VGA emulada en una GPU.
        cmd.arg("-accel").arg("whpx");
    }
    cmd.arg("-m")
        .arg("256")
        .arg("-boot")
        .arg("order=a")
        .arg("-drive")
        .arg(format!("if=floppy,format=raw,file={}", img.display()))
        .arg("-drive")
        .arg(format!(
            "if=ide,index=0,media=disk,format=raw,file={}",
            disk.display()
        ))
        .arg("-serial")
        .arg("stdio")
        .arg("-no-reboot");

    // Altavoz interno (PC speaker) enrutado al audio del anfitrion, para que
    // BEEP/PLAY y la melodia de la demo se oigan de verdad.
    if cfg!(windows) {
        cmd.arg("-audiodev").arg("dsound,id=snd0");
    } else {
        cmd.arg("-audiodev").arg("sdl,id=snd0");
    }
    cmd.arg("-machine").arg("pcspk-audiodev=snd0");

    if window {
        cmd.arg("-vga").arg("std");
    } else {
        cmd.arg("-display").arg("none");
    }

    if usb {
        // El HCD xHCI de QEMU: se anuncia como clase 0C, subclase 03,
        // prog_if 30, que es exactamente como lo despacha rocky. Los dos
        // dispositivos de entrada son HID de boot (teclado y raton), que es lo
        // que la fase 4 del plan tiene que hacer funcionar.
        cmd.arg("-device").arg("qemu-xhci,id=xhci");
        cmd.arg("-device").arg("usb-kbd,bus=xhci.0");
        cmd.arg("-device").arg("usb-mouse,bus=xhci.0");
    }

    if whpx {
        println!("Modo WHPX solicitado: acelera CPU invitada; la VGA sigue emulada.");
    }
    if usb {
        println!("Banco USB: xHCI de QEMU con teclado y raton USB.");
    }
    println!("Ejecutando: {} (Ctrl+C para salir)", qemu.display());
    let status = match cmd.status() {
        Ok(status) => status,
        Err(err) => {
            eprintln!();
            eprintln!("No se pudo ejecutar QEMU ({}).", err);
            eprintln!("Instala QEMU o deja el ejecutable en uno de estos sitios:");
            eprintln!("  - %LOCALAPPDATA%\\Programs\\qemu\\qemu-system-x86_64.exe");
            eprintln!("  - C:\\Program Files\\qemu\\qemu-system-x86_64.exe");
            eprintln!("  - en el PATH");
            std::process::exit(1);
        }
    };
    if whpx && !status.success() {
        eprintln!(
            "WHPX no pudo iniciar. Comprueba que la caracteristica Windows Hypervisor Platform este habilitada y que QEMU incluya WHPX."
        );
    }
    std::process::exit(status.code().unwrap_or(0));
}

fn clean(root: &Path) {
    for dir in ["boot", "kernel"] {
        let _ = fs::remove_dir_all(root.join(dir).join("target"));
    }
    let _ = fs::remove_dir_all(root.join("build"));
    let _ = fs::remove_file(root.join("rocky.img"));
    let _ = fs::remove_file(root.join("rocky-usb.img"));
    let _ = fs::remove_file(root.join("rockydisk.img"));
    println!("limpio.");
}
