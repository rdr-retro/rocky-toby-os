//! Acceso a disco: ATA en modo PIO o, si arrancamos de un pendrive, el BIOS.
//!
//! Hay dos caminos para hablar con el almacenamiento:
//!
//! * **ATA PIO** (`0x1F0`-`0x1F7`, disco maestro de la placa). Es el modo que
//!   usa rocky cuando arranca del disco duro virtual del disquete. Solo
//!   implementamos lo que necesitamos: identificar la unidad y leer y escribir
//!   sectores de 512 bytes por LBA de 28/48 bits.
//! * **BIOS `INT 13h`**. Cuando se arranca del pendrive, el sector de arranque
//!   deja en `0x7E00` la unidad y el LBA donde empieza el volumen Estrato; el
//!   kernel usa entonces las lecturas LBA del BIOS (`AH=42h`/`43h`) para llegar
//!   a ese volumen, que vive detras de la particion ESP del pendrive.
//!
//! En ambos casos el resto del nucleo solo ve la interfaz comun
//! (`presente`, `sectores`, `leer`, `escribir`, `sincronizar`), que trabaja
//! **dentro del volumen Estrato**: los LBA son relativos a su inicio.
//!
//! El disco lleva una tabla de particiones (MBR) con dos entradas: el volumen
//! Estrato (tipo `0x83`) y una particion **FAT32** (tipo `0x0C`) para los logs
//! del sistema (ver [`crate::fat32`]). [`init`] lee el MBR y apunta donde
//! empieza cada una; para la de logs hay un acceso aparte por **LBA fisico**
//! ([`leer_crudo`]/[`escribir_crudo`]), porque no es el volumen.
//!
//! Codigo propio de rocky.

use crate::bios::{int13, Regs};
use crate::cpu::{inb, inw, outb, outw};
use crate::sync::SyncCell;

// Puertos del bus IDE primario.
const PUERTO_DATOS: u16 = 0x1F0;
const PUERTO_CONTEO: u16 = 0x1F2;
const PUERTO_LBA0: u16 = 0x1F3;
const PUERTO_LBA1: u16 = 0x1F4;
const PUERTO_LBA2: u16 = 0x1F5;
const PUERTO_SELECCION: u16 = 0x1F6;
const PUERTO_ESTADO: u16 = 0x1F7;
const PUERTO_ALTERNO: u16 = 0x3F6;

// Bits del registro de estado.
const OCUPADO: u8 = 0x80; // BSY
const FALLO: u8 = 0x20; // DF
const PETICION: u8 = 0x08; // DRQ
const ERROR: u8 = 0x01; // ERR

// Comandos.
const CMD_LEER: u8 = 0x20;
const CMD_ESCRIBIR: u8 = 0x30;
const CMD_IDENTIFICAR: u8 = 0xEC;
const CMD_SINCRONIZAR: u8 = 0xE7;

/// Datos que deja el sector de arranque del pendrive antes de saltar al kernel.
const BOOTINFO: usize = 0x7E00;
/// Magia del boot info: "CORK" (0x4B524F43) en little-endian.
const MAGIA_BOOT: u32 = 0x4B524F43;
/// Offset de la tabla de particiones dentro del MBR.
const MBR_TABLA: usize = 446;
/// Firma que cierra un MBR valido (los dos ultimos bytes del sector).
const MBR_FIRMA: [u8; 2] = [0x55, 0xAA];
/// Tipo de particion del volumen Estrato (el de Linux, por convenio).
const TIPO_ESTRATO: u8 = 0x83;
/// Tipos de particion FAT32 (LBA y el antiguo con CHS).
const TIPO_FAT32_LBA: u8 = 0x0C;
const TIPO_FAT32_CHS: u8 = 0x0B;
/// Disk Address Packet para las llamadas LBA del BIOS (memoria baja).
const DAP: usize = 0x7F00;
/// Buffer de rebote del BIOS: la unidad de transferencia vive por debajo de
/// 1 MiB, mientras que los buffers del kernel pueden estar en cualquier sitio.
const BUF_BIOS: usize = 0x8000;

/// Tamano de sector del disco.
pub const SECTOR: usize = 512;

/// De donde salen los sectores.
#[derive(Clone, Copy, PartialEq)]
enum Origen {
    /// No hay ningun disco disponible.
    Ninguno,
    /// Controlador ATA de la placa (PIO).
    Ata,
    /// BIOS `INT 13h` (pendrive arrancable).
    Bios,
}

#[derive(Clone, Copy)]
struct Estado {
    presente: bool,
    sectores: u64,
    origen: Origen,
    /// LBA fisico donde empieza el volumen Estrato.
    base: u32,
    /// Unidad del BIOS (0x80, 0x81, ...).
    unidad: u8,
    /// LBA fisico donde empieza la particion de logs FAT32 (0 = no hay).
    log_lba: u32,
    /// Sectores de la particion de logs.
    log_sectores: u32,
}

static ESTADO: SyncCell<Estado> = SyncCell::new(Estado {
    presente: false,
    sectores: 0,
    origen: Origen::Ninguno,
    base: 0,
    unidad: 0,
    log_lba: 0,
    log_sectores: 0,
});

// --------------------------------------------------------------------------
// Backend ATA (PIO)
// --------------------------------------------------------------------------

/// Pequena espera de E/S: 4 lecturas del puerto alterno (~400 ns).
fn retardo() {
    unsafe {
        for _ in 0..4 {
            let _ = inb(PUERTO_ALTERNO);
        }
    }
}

fn estado() -> u8 {
    unsafe { inb(PUERTO_ESTADO) }
}

/// Espera a que el disco no este ocupado.
fn esperar_libre() -> bool {
    for i in 0..2_000_000u32 {
        if estado() & OCUPADO == 0 {
            return true;
        }
        // Aprovecha la espera para no perder bytes del puerto serie.
        if i % 256 == 0 {
            crate::input::recoger();
        }
    }
    false
}

/// Espera a que el disco tenga datos listos (DRQ).
fn esperar_datos() -> bool {
    for i in 0..2_000_000u32 {
        let s = estado();
        if s & (OCUPADO | PETICION) == PETICION {
            return true;
        }
        if s & (ERROR | FALLO) != 0 {
            return false;
        }
        if i % 256 == 0 {
            crate::input::recoger();
        }
    }
    false
}

/// Selecciona el disco maestro en modo LBA y deja la marca de la parte alta.
fn seleccionar(lba: u32) {
    unsafe {
        outb(PUERTO_SELECCION, 0xE0 | (((lba >> 24) as u8) & 0x0F));
    }
    retardo();
}

/// Detecta el disco ATA maestro y su capacidad. Devuelve `true` si esta.
fn init_ata() -> bool {
    if !esperar_libre() {
        return false;
    }
    unsafe {
        outb(PUERTO_SELECCION, 0xA0);
    }
    retardo();
    unsafe {
        outb(PUERTO_CONTEO, 0);
        outb(PUERTO_LBA0, 0);
        outb(PUERTO_LBA1, 0);
        outb(PUERTO_LBA2, 0);
        outb(PUERTO_ESTADO, CMD_IDENTIFICAR);
    }
    // Si el registro de estado es cero, no hay unidad conectada.
    if estado() == 0 {
        return false;
    }
    if !esperar_libre() {
        return false;
    }
    unsafe {
        // Si no es un dispositivo ATA, LBA1/LBA2 no seran cero.
        if inb(PUERTO_LBA1) != 0 || inb(PUERTO_LBA2) != 0 {
            return false;
        }
    }
    if !esperar_datos() {
        return false;
    }

    let mut ident = [0u16; 256];
    unsafe {
        for word in ident.iter_mut() {
            *word = inw(PUERTO_DATOS);
        }
    }

    let lba28 = ident[60] as u64 | ((ident[61] as u64) << 16);
    let lba48 = ident[100] as u64
        | ((ident[101] as u64) << 16)
        | ((ident[102] as u64) << 32)
        | ((ident[103] as u64) << 48);
    // Bit 10 de la palabra 83 = soporta LBA de 48 bits.
    let sectores = if ident[83] & 0x0400 != 0 && lba48 > 0 {
        lba48
    } else {
        lba28
    };

    let e = unsafe { &mut *ESTADO.get() };
    e.presente = true;
    e.sectores = sectores;
    e.origen = Origen::Ata;
    true
}

fn leer_ata(lba: u32, buf: &mut [u8; SECTOR]) -> Result<(), ()> {
    if !esperar_libre() {
        return Err(());
    }
    seleccionar(lba);
    unsafe {
        outb(PUERTO_CONTEO, 1);
        outb(PUERTO_LBA0, lba as u8);
        outb(PUERTO_LBA1, (lba >> 8) as u8);
        outb(PUERTO_LBA2, (lba >> 16) as u8);
        outb(PUERTO_ESTADO, CMD_LEER);
    }
    if !esperar_datos() {
        // --- Depuracion temporal (quitar) ---
        crate::println!("disk: sin DRQ leyendo el sector {}", lba);
        return Err(());
    }
    unsafe {
        for i in 0..SECTOR / 2 {
            let word = inw(PUERTO_DATOS);
            buf[i * 2] = word as u8;
            buf[i * 2 + 1] = (word >> 8) as u8;
        }
    }
    retardo();
    Ok(())
}

fn escribir_ata(lba: u32, buf: &[u8; SECTOR]) -> Result<(), ()> {
    if !esperar_libre() {
        return Err(());
    }
    seleccionar(lba);
    unsafe {
        outb(PUERTO_CONTEO, 1);
        outb(PUERTO_LBA0, lba as u8);
        outb(PUERTO_LBA1, (lba >> 8) as u8);
        outb(PUERTO_LBA2, (lba >> 16) as u8);
        outb(PUERTO_ESTADO, CMD_ESCRIBIR);
    }
    if !esperar_datos() {
        return Err(());
    }
    unsafe {
        for i in 0..SECTOR / 2 {
            let word = (buf[i * 2] as u16) | ((buf[i * 2 + 1] as u16) << 8);
            outw(PUERTO_DATOS, word);
        }
    }
    retardo();
    // Espera a que la unidad termine de volcar el sector antes de la siguiente
    // orden; si no, las escrituras seguidas se pisan.
    let _ = esperar_libre();
    Ok(())
}

// --------------------------------------------------------------------------
// Backend BIOS (INT 13h, funciones extendidas de LBA)
// --------------------------------------------------------------------------

fn leer_u32(dir: usize) -> u32 {
    unsafe { core::ptr::read_volatile(dir as *const u32) }
}

/// Rellena la DAP con una transferencia de un sector a `BUF_BIOS`.
fn preparar_dap(lba: u32) {
    unsafe {
        let d = DAP as *mut u8;
        core::ptr::write_volatile(d, 0x10); // tamano de la DAP
        core::ptr::write_volatile(d.add(1), 0); // reservado
        core::ptr::write_volatile(d.add(2) as *mut u16, 1); // 1 sector
        core::ptr::write_volatile(d.add(4) as *mut u16, 0); // offset = 0
        core::ptr::write_volatile(d.add(6) as *mut u16, (BUF_BIOS >> 4) as u16); // segmento
        core::ptr::write_volatile(d.add(8) as *mut u32, lba); // LBA bajo
        core::ptr::write_volatile(d.add(12) as *mut u32, 0); // LBA alto
    }
}

/// Ejecuta `INT 13h` con `AH` = `comando` sobre el sector `lba` del volumen.
fn bios_sector(comando: u8, lba: u32, unidad: u8) -> bool {
    preparar_dap(lba);
    let r = int13(Regs {
        eax: ((comando as u32) << 8) | 1, // AH=comando, AL=1 sector
        edx: unidad as u32,               // DL = unidad
        esi: DAP as u32,                  // DS:SI = DAP (DS=0 en el trampolin)
        ..Default::default()
    });
    !r.carry
}

fn leer_bios(lba: u32, buf: &mut [u8; SECTOR]) -> Result<(), ()> {
    let e = unsafe { *ESTADO.get() };
    if !bios_sector(0x42, lba, e.unidad) {
        return Err(());
    }
    unsafe {
        core::ptr::copy_nonoverlapping(BUF_BIOS as *const u8, buf.as_mut_ptr(), SECTOR);
    }
    Ok(())
}

fn escribir_bios(lba: u32, buf: &[u8; SECTOR]) -> Result<(), ()> {
    let e = unsafe { *ESTADO.get() };
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), BUF_BIOS as *mut u8, SECTOR);
    }
    if !bios_sector(0x43, lba, e.unidad) {
        return Err(());
    }
    Ok(())
}

// --------------------------------------------------------------------------
// API publica
// --------------------------------------------------------------------------

/// Prepara el disco. Primero mira si el pendrive dejo su configuracion en
/// `0x7E00` (backend BIOS); si no, busca un disco ATA de la placa. En ambos
/// casos lee la tabla de particiones del MBR (si la hay) para saber donde
/// empiezan el volumen Estrato y la particion de logs.
pub fn init() -> bool {
    if leer_u32(BOOTINFO) == MAGIA_BOOT {
        let unidad = leer_u32(BOOTINFO + 4);
        let base = leer_u32(BOOTINFO + 8);
        let sectores = leer_u32(BOOTINFO + 12) as u64;
        if unidad <= 0xFF && sectores > 0 {
            let e = unsafe { &mut *ESTADO.get() };
            e.presente = true;
            e.sectores = sectores;
            e.origen = Origen::Bios;
            e.base = base;
            e.unidad = unidad as u8;
            buscar_particiones();
            return true;
        }
    }
    if !init_ata() {
        return false;
    }
    buscar_particiones();
    true
}

/// Lee el MBR (sector fisico 0) y anota donde estan el volumen Estrato y la
/// particion de logs.
///
/// Si no hay tabla de particiones (un disco formateado "a pelo", como el que
/// usaba rocky antes), todo se queda como estaba y el volumen empieza en el
/// LBA 0. Es a proposito: asi un disco ya hecho sigue funcionando.
fn buscar_particiones() {
    let mut mbr = [0u8; SECTOR];
    if leer_crudo(0, &mut mbr).is_err() || mbr[510..512] != MBR_FIRMA {
        return;
    }
    let mut estrato = None;
    let mut log = None;
    for i in 0..4 {
        let p = MBR_TABLA + i * 16;
        let tipo = mbr[p + 4];
        let lba = u32::from_le_bytes([mbr[p + 8], mbr[p + 9], mbr[p + 10], mbr[p + 11]]);
        let n = u32::from_le_bytes([mbr[p + 12], mbr[p + 13], mbr[p + 14], mbr[p + 15]]);
        if lba == 0 || n == 0 {
            continue;
        }
        match tipo {
            TIPO_ESTRATO => estrato = Some((lba, n)),
            TIPO_FAT32_LBA | TIPO_FAT32_CHS => log = Some((lba, n)),
            _ => {}
        }
    }
    let e = unsafe { &mut *ESTADO.get() };
    if let Some((lba, n)) = estrato {
        // Con el backend BIOS el sector de arranque ya dejo el inicio del
        // volumen en `0x7E00`; el MBR es la referencia para el disco ATA, que
        // se direcciona por LBA absoluto.
        if e.origen == Origen::Ata {
            e.base = lba;
            e.sectores = n as u64;
        }
    }
    if let Some((lba, n)) = log {
        e.log_lba = lba;
        e.log_sectores = n;
    }
}

/// LBA y sectores de la particion de logs FAT32, si el disco tiene una.
pub fn particion_log() -> Option<(u32, u32)> {
    let e = unsafe { *ESTADO.get() };
    if e.log_lba != 0 && e.log_sectores != 0 {
        Some((e.log_lba, e.log_sectores))
    } else {
        None
    }
}

/// Hay un disco disponible.
pub fn presente() -> bool {
    unsafe { (*ESTADO.get()).presente }
}

/// `true` si el disco es el volumen Estrato de un pendrive arrancable.
pub fn es_pendrive() -> bool {
    unsafe { (*ESTADO.get()).origen == Origen::Bios }
}

/// Numero de sectores de 512 bytes del volumen.
pub fn sectores() -> u64 {
    unsafe { (*ESTADO.get()).sectores }
}

/// Lee un sector de 512 bytes del **volumen** (el LBA es relativo a su inicio).
pub fn leer(lba: u32, buf: &mut [u8; SECTOR]) -> Result<(), ()> {
    if !presente() {
        return Err(());
    }
    let base = unsafe { (*ESTADO.get()).base };
    let fisico = base.checked_add(lba).ok_or(())?;
    leer_crudo(fisico, buf)
}

/// Escribe un sector de 512 bytes en el **volumen**.
pub fn escribir(lba: u32, buf: &[u8; SECTOR]) -> Result<(), ()> {
    if !presente() {
        return Err(());
    }
    let base = unsafe { (*ESTADO.get()).base };
    let fisico = base.checked_add(lba).ok_or(())?;
    escribir_crudo(fisico, buf)
}

/// Lee un sector por su **LBA fisico**, sin sumarle el inicio del volumen.
///
/// Es lo que usa la particion de logs: el sector 0 de un disco con MBR es el
/// MBR, y cada particion se direcciona por su LBA, tal cual.
pub fn leer_crudo(lba: u32, buf: &mut [u8; SECTOR]) -> Result<(), ()> {
    let r = match unsafe { (*ESTADO.get()).origen } {
        Origen::Ata => leer_ata(lba, buf),
        Origen::Bios => leer_bios(lba, buf),
        Origen::Ninguno => Err(()),
    };
    if r.is_ok() {
        crate::metricas::lectura();
    }
    r
}

/// Escribe un sector por su **LBA fisico**, sin sumarle el inicio del volumen.
pub fn escribir_crudo(lba: u32, buf: &[u8; SECTOR]) -> Result<(), ()> {
    let r = match unsafe { (*ESTADO.get()).origen } {
        Origen::Ata => escribir_ata(lba, buf),
        Origen::Bios => escribir_bios(lba, buf),
        Origen::Ninguno => Err(()),
    };
    if r.is_ok() {
        crate::metricas::escritura();
    }
    r
}

/// Fuerza el volcado de las escrituras pendientes a la superficie del disco.
pub fn sincronizar() {
    if !presente() {
        return;
    }
    match unsafe { (*ESTADO.get()).origen } {
        Origen::Ata => {
            if !esperar_libre() {
                return;
            }
            unsafe {
                outb(PUERTO_ESTADO, CMD_SINCRONIZAR);
            }
            let _ = esperar_libre();
        }
        // Las escrituras del BIOS ya son sincronas.
        Origen::Bios => {}
        Origen::Ninguno => {}
    }
}
