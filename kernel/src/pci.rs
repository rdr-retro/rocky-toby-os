//! Bus PCI: enumeracion, identificacion de dispositivos y lectura de sus BARs.
//!
//! Es la base para hablar con el hardware real que no se controla por puertos de
//! E/S "conocidos" (la GPU, por ejemplo). Se usa el mecanismo clasico de
//! configuracion **#1** (puertos `0xCF8`/`0xCFC`) para barrer el bus y leer,
//! ademas del identificador de cada funcion, sus **BAR** (*Base Address
//! Register*): la direccion donde vive el bloque de registros (MMIO) y la
//! abertura de memoria del dispositivo.
//!
//! El bootloader ya mapea por identidad los primeros 4 GiB, asi que un BAR de
//! MMIO por debajo de esa frontera se puede leer y escribir directamente, sin
//! tocar la paginacion.
//!
//! Codigo propio de rocky.

//! El bus en si (enumeracion, BAR, espacio de configuracion) **no** es un
//! controlador: vive en [`crate::pci`], que solo *describe* los dispositivos.
//! Aqui esta lo que se *hace* con ellos.

use crate::cpu::{inl, outl};
use crate::sync::SyncCell;

// Los tipos del bus PCI viven en la libreria `rockyhw`, que es `no_std` y se
// puede probar en el anfitrion. Aqui solo se reexportan, para que el codigo
// del kernel siga usando `pci::Bar` y `pci::Funcion` como antes.
pub use rockyhw::pci::{Bar, BarLeido, Funcion, TipoHcd};

/// Puerto de direccion del mecanismo de configuracion PCI #1.
const CONFIG_ADDR: u16 = 0xCF8;
/// Puerto de datos del mecanismo de configuracion PCI #1.
const CONFIG_DATA: u16 = 0xCFC;

/// Funciones que se guardan como maximo al enumerar.
///
/// Un PC real puede pasar de 64 con holgura (puentes PCIe + dispositivos
/// multifuncion), asi que se deja sitio de sobra.
const MAX: usize = 256;

/// Bit de "espacio de E/S" del registro Command.
const CMD_IO_SPACE: u16 = 1 << 0;
/// Bit de "espacio de memoria" del registro Command.
const CMD_MEMORY_SPACE: u16 = 1 << 1;
/// Bit de "bus master" del registro Command (el dispositivo puede hacer DMA).
const CMD_BUS_MASTER: u16 = 1 << 2;

/// Habilita decodificacion de memoria y bus mastering en una funcion PCI.
///
/// Es el requisito PCI para que un dispositivo pueda leer/escribir RAM por
/// DMA. Escribe solo los 16 bits del registro Command; los 16 bits altos son
/// Status y algunos son W1C, por lo que copiar el dword leido podria borrar
/// estados de error pendientes.
///
/// **No se llama hasta que el controlador este preparado** (BAR validado y
/// estructuras DMA reservadas): encender el bus mastering antes de tiempo deja
/// al dispositivo con permiso para escribir en memoria que todavia no es suya.
pub fn habilitar_memoria_y_bus_master(f: Funcion) -> bool {
    let command = leer_comando(f);
    let solicitado = command | CMD_MEMORY_SPACE | CMD_BUS_MASTER;
    escribir_comando(f, solicitado);

    let actual = leer_comando(f);
    actual & (CMD_MEMORY_SPACE | CMD_BUS_MASTER) == (CMD_MEMORY_SPACE | CMD_BUS_MASTER)
}

/// Habilita solo la decodificacion de memoria de la funcion.
pub fn habilitar_memoria(f: Funcion) -> bool {
    let command = leer_comando(f);
    escribir_comando(f, command | CMD_MEMORY_SPACE);
    leer_comando(f) & CMD_MEMORY_SPACE != 0
}

/// Apaga el bus mastering de la funcion.
///
/// Se usa antes de tocar el dispositivo (por ejemplo, antes de medir sus BAR)
/// para que no pueda hacer DMA mientras se esta tocando su configuracion.
///
/// El bus mastering se vuelve a encender con
/// [`habilitar_memoria_y_bus_master`], y solo cuando el controlador este
/// preparado: con estructuras DMA reservadas y el HCD inicializado.
pub fn deshabilitar_bus_master(f: Funcion) -> bool {
    let command = leer_comando(f);
    escribir_comando(f, command & !CMD_BUS_MASTER);
    leer_comando(f) & CMD_BUS_MASTER == 0
}

/// Registro Command de la funcion (los 16 bits que importan).
///
/// Los 16 bits altos son el registro Status y algunos son de escribir-para-
/// limpiar, asi que no se tocan nunca.
pub fn leer_comando(f: Funcion) -> u16 {
    leer32(f.bus, f.dev, f.func, 0x04) as u16
}

/// Escribe el registro Command de la funcion (solo los 16 bits bajos).
pub fn escribir_comando(f: Funcion, valor: u16) {
    escribir32(f.bus, f.dev, f.func, 0x04, (valor as u32) & 0xFFFF);
}

/// ¿La funcion tiene el bus mastering encendido?
pub fn bus_master_activo(f: Funcion) -> bool {
    leer_comando(f) & CMD_BUS_MASTER != 0
}

/// ¿La funcion tiene la decodificacion de memoria encendida?
pub fn memoria_activa(f: Funcion) -> bool {
    leer_comando(f) & CMD_MEMORY_SPACE != 0
}

static TABLA: SyncCell<([Funcion; MAX], usize)> = SyncCell::new(([Funcion::VACIA; MAX], 0));

/// Marca de las funciones que ya estan en uso por algun controlador.
///
/// Es el registro que impide medir los BAR de un dispositivo que ya esta
/// funcionando: escribir `0xFFFF_FFFF` en un registro de un controlador
/// vivo puede dejarlo sin estado.
static EN_USO: SyncCell<[bool; MAX]> = SyncCell::new([false; MAX]);

fn tabla() -> &'static mut ([Funcion; MAX], usize) {
    // Seguro: lo unico que comparte esta tabla es el interprete de ordenes, que
    // corre en la tarea del nucleo (una a la vez) y solo la lee.
    unsafe { &mut *TABLA.get() }
}

fn marcas() -> &'static mut [bool; MAX] {
    unsafe { &mut *EN_USO.get() }
}

/// Indice de una funcion dentro de la tabla del bus.
fn indice_de(f: Funcion) -> Option<usize> {
    funciones()
        .iter()
        .position(|x| x.bus == f.bus && x.dev == f.dev && x.func == f.func)
}

/// Marca la funcion como **en uso**: su configuracion no se vuelve a tocar.
///
/// Devuelve `false` si la funcion no esta en la tabla del bus.
pub fn marcar_en_uso(f: Funcion) -> bool {
    match indice_de(f) {
        Some(i) => {
            marcas()[i] = true;
            true
        }
        None => false,
    }
}

/// Quita la marca de uso de la funcion.
pub fn desmarcar_en_uso(f: Funcion) {
    if let Some(i) = indice_de(f) {
        marcas()[i] = false;
    }
}

/// ¿La funcion esta marcada como en uso?
pub fn esta_en_uso(f: Funcion) -> bool {
    match indice_de(f) {
        Some(i) => marcas()[i],
        None => false,
    }
}

// --- Acceso al espacio de configuracion ---

/// Direccion de configuracion de 32 bits de una funcion.
fn direccion(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | (((dev as u32) & 0x1F) << 11)
        | (((func as u32) & 0x07) << 8)
        | ((offset as u32) & 0xFC)
}

/// Lee 32 bits del espacio de configuracion.
pub fn leer32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    unsafe {
        outl(CONFIG_ADDR, direccion(bus, dev, func, offset));
        inl(CONFIG_DATA)
    }
}

/// Escribe 32 bits en el espacio de configuracion.
pub fn escribir32(bus: u8, dev: u8, func: u8, offset: u8, valor: u32) {
    unsafe {
        outl(CONFIG_ADDR, direccion(bus, dev, func, offset));
        outl(CONFIG_DATA, valor);
    }
}

fn leer8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    let shift = ((offset & 3) * 8) as u32;
    (leer32(bus, dev, func, offset & 0xFC) >> shift) as u8
}

// --- Enumeracion ---

fn leer_funcion(bus: u8, dev: u8, func: u8) -> Funcion {
    let id = leer32(bus, dev, func, 0x00);
    let clase = leer32(bus, dev, func, 0x08);
    Funcion {
        bus,
        dev,
        func,
        vendor: id as u16,
        device: (id >> 16) as u16,
        revision: clase as u8,
        clase: (clase >> 24) as u8,
        subclase: (clase >> 16) as u8,
        prog_if: (clase >> 8) as u8,
    }
}

/// Enumera el bus, guarda las funciones encontradas e imprime un resumen.
///
/// Se llama una sola vez, al arrancar. A partir de ahi `lspci` y los
/// controladores leen la tabla guardada (con [`funciones`]), sin volver a
/// barrer el bus.
pub fn init() {
    let (lista, cuenta) = tabla();
    *cuenta = 0;

    for bus in 0..=255u8 {
        for dev in 0..32u8 {
            if leer32(bus, dev, 0, 0x00) as u16 == 0xFFFF {
                continue;
            }
            // Una funcion 0 presente puede ser multifuncion (bit 7 del tipo de
            // cabecera); entonces hay que mirar tambien las funciones 1..7.
            let multifuncion = leer8(bus, dev, 0, 0x0E) & 0x80 != 0;
            let n_func = if multifuncion { 8 } else { 1 };
            for func in 0..n_func {
                if leer32(bus, dev, func, 0) as u16 != 0xFFFF && *cuenta < MAX {
                    lista[*cuenta] = leer_funcion(bus, dev, func);
                    *cuenta += 1;
                }
            }
        }
    }

    if *cuenta == MAX {
        crate::println!(
            "rocky: PCI: aviso: la lista se ha truncado a {} funciones.",
            MAX
        );
    }
    crate::println!("rocky: PCI: {} dispositivos.", *cuenta);
}

/// Las funciones guardadas al enumerar el bus.
///
/// Es el inventario que consultan los controladores (por ejemplo, el de AMD
/// busca aqui su GPU). Solo lectura: la tabla no cambia tras [`init`].
pub fn funciones() -> &'static [Funcion] {
    let (lista, cuenta) = tabla();
    let n = *cuenta;
    let p: *const Funcion = lista.as_ptr();
    // Seguro: la tabla es estatica y solo se escribe en `init`, una sola vez.
    unsafe { core::slice::from_raw_parts(p, n) }
}

/// Lee los seis BAR **sin escribir nada** en la funcion.
///
/// Es la lectura que puede hacer el inventario: solo lee el espacio de
/// configuracion, no toca el dispositivo. El tamano de la ventana no se puede
/// saber asi (habria que escribir en el registro), asi que la decodificacion
/// devuelve la base y el tipo, pero no el tamano.
pub fn bares_solo_lectura(f: Funcion) -> [Option<BarLeido>; 6] {
    let mut bares: [Option<BarLeido>; 6] = [None; 6];
    let mut i = 0u8;
    while i < 6 {
        let off = 0x10 + i * 4;
        let lo = leer32(f.bus, f.dev, f.func, off);
        if lo == 0 {
            i += 1;
            continue;
        }
        // Un BAR de 64 bits ocupa dos registros: el bajo y su hermano.
        let es_64 = lo & 1 == 0 && (lo >> 1) & 0x3 == 0x2;
        let hi = if es_64 {
            Some(leer32(f.bus, f.dev, f.func, off + 4))
        } else {
            None
        };
        bares[i as usize] = rockyhw::pci::decodificar_bar(lo, hi);
        i += if es_64 { 2 } else { 1 };
    }
    bares
}

/// Mide los seis BAR de `f`.
///
/// Para medir un BAR hay que escribirle todo a unos y leer la mascara, asi que
/// mientras dura el sondeo se apaga la decodificacion de memoria/E/S **y el bus
/// mastering** de la funcion, y se restaura despues, para no dejar el
/// dispositivo a medias. La hermana alta de un BAR de 64 bits se deja como
/// `None`.
///
/// Si la funcion ya esta marcada como **en uso**, no se mide nada: escribir en
/// los registros de un controlador vivo puede dejarlo sin estado. En ese caso
/// se devuelven seis `None`.
pub fn medir_bares(f: Funcion) -> [Option<Bar>; 6] {
    if esta_en_uso(f) {
        return [None; 6];
    }
    medir_bares_siempre(f)
}

/// Mide los seis BAR de `f` sin mirar la marca de uso.
///
/// Es la parte interna de [`medir_bares`], para quien ya ha marcado la funcion
/// como en uso (la propia sonda, que no quiere que nadie se meta).
pub fn medir_bares_siempre(f: Funcion) -> [Option<Bar>; 6] {
    let cmd = leer32(f.bus, f.dev, f.func, 0x04);
    // Se apagan los bits 0 (E/S), 1 (memoria) y 2 (bus master): durante el
    // sondeo el dispositivo no debe decodificar sus ventanas ni hacer DMA.
    escribir32(
        f.bus,
        f.dev,
        f.func,
        0x04,
        cmd & !((CMD_IO_SPACE | CMD_MEMORY_SPACE | CMD_BUS_MASTER) as u32),
    );

    let mut bares = [None; 6];
    let mut i = 0u8;
    while i < 6 {
        bares[i as usize] = leer_bar(f, i);
        i += match bares[i as usize] {
            Some(b) if b.es_64 => 2,
            _ => 1,
        };
    }

    escribir32(f.bus, f.dev, f.func, 0x04, cmd);
    bares
}

// --- Ordenes del interprete ---

/// Orden `lspci`: lista las funciones del bus guardadas al arrancar.
pub fn listar() {
    let (lista, cuenta) = tabla();
    let n = *cuenta;
    if n == 0 {
        crate::println!("lspci: no se ha encontrado ningun dispositivo PCI.");
        return;
    }
    crate::println!("Bus PCI: {} funciones", n);
    for f in &lista[..n] {
        crate::print!(
            "  {:02X}:{:02X}.{}  {:04X}:{:04X}  [{:02X}{:02X}]  {}",
            f.bus,
            f.dev,
            f.func,
            f.vendor,
            f.device,
            f.clase,
            f.subclase,
            fabricante_nombre(f.vendor)
        );
        // Para las de video se prefiere el nombre del modelo; para el resto, la
        // clase basta.
        match (f.es_video(), producto_nombre(f.vendor, f.device)) {
            (true, Some(nombre)) => crate::println!(" {}", nombre),
            _ => crate::println!(" {}", clase_nombre(f.clase, f.subclase)),
        }
    }
    crate::println!(
        "(usa 'gpu' para los BAR de la tarjeta grafica y 'cursor' para el cursor por hardware)"
    );
}

/// Lee y descodifica el BAR de indice `idx` (o `None` si no hay ninguno).
fn leer_bar(f: Funcion, idx: u8) -> Option<Bar> {
    let off = 0x10 + idx * 4;
    let raw = leer32(f.bus, f.dev, f.func, off);
    if raw == 0 {
        return None;
    }
    if raw & 1 != 0 {
        return Some(Bar {
            base: (raw & !0x3) as u64,
            size: tam(f, off, false),
            es_io: true,
            es_64: false,
        });
    }
    let es_64 = (raw >> 1) & 0x3 == 0x2;
    let mut base = (raw & !0xF) as u64;
    if es_64 {
        base |= (leer32(f.bus, f.dev, f.func, off + 4) as u64) << 32;
    }
    Some(Bar {
        base,
        size: tam(f, off, es_64),
        es_io: false,
        es_64,
    })
}

/// Mide el tamano de un BAR (se le escribe todo a unos y se lee la mascara).
fn tam(f: Funcion, off: u8, es_64: bool) -> u64 {
    let es_io = leer32(f.bus, f.dev, f.func, off) & 1 != 0;
    let memo_lo = leer32(f.bus, f.dev, f.func, off);
    let memo_hi = if es_64 {
        leer32(f.bus, f.dev, f.func, off + 4)
    } else {
        0
    };

    escribir32(f.bus, f.dev, f.func, off, 0xFFFF_FFFF);
    if es_64 {
        escribir32(f.bus, f.dev, f.func, off + 4, 0xFFFF_FFFF);
    }
    let leido_lo = leer32(f.bus, f.dev, f.func, off);
    let leido_hi = if es_64 {
        leer32(f.bus, f.dev, f.func, off + 4)
    } else {
        0
    };

    // Se restaura el valor original (y el hermano alto si es de 64 bits).
    escribir32(f.bus, f.dev, f.func, off, memo_lo);
    if es_64 {
        escribir32(f.bus, f.dev, f.func, off + 4, memo_hi);
    }

    let mascara = if es_io {
        (leido_lo & !0x3) as u64
    } else if es_64 {
        (leido_lo as u64 & !0xF) | ((leido_hi as u64) << 32)
    } else {
        (leido_lo & !0xF) as u64
    };
    if mascara == 0 {
        return 0;
    }
    let size = mascara.wrapping_neg();
    if es_io {
        size & 0xFFFF
    } else if es_64 {
        size
    } else {
        size & 0xFFFF_FFFF
    }
}

/// Descompone un tamano en la unidad mas comoda (sin asignar memoria).
pub fn tam_txt(mut b: u64) -> (u64, &'static str) {
    let mut unidad = "bytes";
    if b != 0 && b % 1024 == 0 {
        b /= 1024;
        unidad = "KiB";
        if b % 1024 == 0 {
            b /= 1024;
            unidad = "MiB";
            if b % 1024 == 0 {
                b /= 1024;
                unidad = "GiB";
            }
        }
    }
    (b, unidad)
}

// --- Nombres conocidos (para que la salida sea legible) ---

/// Nombre del fabricante a partir de su identificador.
pub fn fabricante_nombre(id: u16) -> &'static str {
    match id {
        0x1002 => "AMD/ATI",
        0x1022 => "AMD",
        0x8086 => "Intel",
        0x1234 => "QEMU/Bochs",
        0x1AF4 | 0x1B36 => "Red Hat",
        0x10DE => "NVIDIA",
        0x10EC => "Realtek",
        0x14E4 => "Broadcom",
        0x1B21 => "ASMedia",
        0x1106 => "VIA",
        _ => "?",
    }
}

pub fn clase_nombre(clase: u8, subclase: u8) -> &'static str {
    match clase {
        0x00 => "sin clasificar",
        0x01 => match subclase {
            0x01 => "almacenamiento IDE",
            0x06 => "almacenamiento SATA",
            0x08 => "almacenamiento NVMe",
            _ => "almacenamiento",
        },
        0x02 => "red",
        0x03 => match subclase {
            0x00 => "display/VGA",
            0x02 => "display 3D",
            _ => "display",
        },
        0x04 => "multimedia",
        0x05 => "memoria",
        0x06 => match subclase {
            0x00 => "puente host",
            0x01 => "puente ISA",
            0x04 => "puente PCI",
            _ => "puente",
        },
        0x07 => "comunicaciones",
        0x08 => "sistema",
        0x09 => "entrada",
        0x0A => "acoplamiento",
        0x0B => "procesador",
        0x0C => match subclase {
            0x03 => "bus USB",
            0x05 => "bus SMBus",
            _ => "bus serie",
        },
        0x0D => "inalambrico",
        0x0E => "inteligente",
        0x0F => "satelite",
        0x10 => "cifrado",
        0x11 => "procesador de senales",
        0xFF => "sin definir",
        _ => "dispositivo",
    }
}

/// Nombre comercial de un dispositivo conocido (un mini `pci.ids`).
///
/// Es solo un catalogo para que la salida sea legible: el *comportamiento* de
/// cada dispositivo vive en [`crate::drivers`], no aqui. Devuelve `None` para
/// los fabricantes que no tenemos fichados.
pub fn producto_nombre(vendor: u16, device: u16) -> Option<&'static str> {
    match vendor {
        0x8086 => Some(match device {
            0x0412 => "HD Graphics 4600 (Haswell GT2)",
            0x0416 => "HD Graphics 4600 Mobile (Haswell GT2)",
            0x041A | 0x041B | 0x041E => "HD Graphics Haswell GT2",
            _ => "GPU Intel",
        }),
        // AMD/ATI: los modelos de Radeon que nos vamos encontrando.
        0x1002 => Some(match device {
            0x5046 => "Rage 128 Pro (QEMU ati-vga)",
            0x67FF => "Baffin [Radeon RX 550 640SP / RX 640]",
            0x67DF => "Ellesmere [Radeon RX 470/480/570/580/590]",
            0x67C0 => "Baffin [Radeon PRO WX 4100]",
            0x699F => "Lexa PRO [Radeon RX 550/550X]",
            0x6995 | 0x6997 => "Lexa [Radeon 540/540X/550/550X]",
            0x6987 | 0x6981 => "Lexa XT [Radeon PRO WX 3100/2100]",
            0x6FDF => "Polaris 20 [Radeon RX 580/590]",
            0x731F => "Navi 10 [Radeon RX 5600/5700]",
            0x73FF => "Navi 23 [Radeon RX 6600/6600 XT/6600M]",
            0x743F => "Navi 24 [Radeon RX 6400/6500 XT]",
            _ => "GPU AMD",
        }),
        _ => None,
    }
}
