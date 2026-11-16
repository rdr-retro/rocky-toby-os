//! Segmentos de usuario (anillo 3) y TSS, sobre la GDT unificada del kernel.
//!
//! La GDT ya existia: la define `bios.s` (`rocky_gdt`) porque el trampolin al
//! BIOS la necesita para bajar a modo real, y `rocky_load_gdt` la recarga tras
//! cada viaje. Aqui **no** se levanta otra GDT (eso la pisaria): se completan las
//! entradas que faltaban para poder tener procesos:
//!
//! ```text
//!   0x08/0x10  codigo/datos 32 bits   (nucleo)
//!   0x18/0x20  codigo/datos 64 bits   (nucleo)
//!   0x28/0x30  codigo/datos 16 bits   (trampolin del BIOS)
//!   0x38/0x40  codigo/datos de **usuario** (DPL 3)   <- los anade el kernel
//!   0x48       TSS de 64 bits (doble entrada)         <- lo parchea el kernel
//! ```
//!
//! El descriptor del TSS no puede ser una constante en ensamblador porque su
//! base es la direccion de una variable de Rust; por eso la entrada 0x48 se
//! rellena aqui en tiempo de ejecucion, justo antes de `ltr`.

use core::arch::asm;

use crate::sync::SyncCell;

/// Selector del segmento de codigo del nucleo (anillo 0).
pub const KERNEL_CS: u64 = 0x18;
/// Selector del segmento de datos del nucleo (anillo 0).
pub const KERNEL_DS: u64 = 0x20;
/// Selector del segmento de codigo de usuario (anillo 3, RPL 3).
pub const USER_CS: u64 = 0x38 | 3; // 0x3B
/// Selector del segmento de datos de usuario (anillo 3, RPL 3).
pub const USER_DS: u64 = 0x40 | 3; // 0x43
/// Selector del TSS.
const TSS_SEL: u16 = 0x48;

/// Indice dentro del TSS de la pila de emergencia del doble fallo (#DF).
pub const IST_DF: u8 = 1;

/// Tamano de la pila de emergencia (`#DF`).
///
/// Es generosa a proposito (16 KiB): el manejador formatea el diagnostico y lo
/// pinta, y todo eso corre en esta pila. Vive en `.bss`, que `start.s` pone a
/// cero, asi que no ocupa espacio en la imagen.
const PILA_DF_BYTES: usize = 16 * 1024;

#[repr(C, align(16))]
struct PilaEmergencia([u8; PILA_DF_BYTES]);

/// Pila para el manejador del doble fallo.
///
/// Es la **unica** excepcion con IST propia. `#PF` se deja a proposito en la
/// pila normal: es una ruta recuperable (memoria bajo demanda), varias `#PF`
/// anidadas compartirian la misma IST, y si su manejador falla la escalada a
/// `#DF` ya cae aqui. El doble fallo, en cambio, es el ultimo aviso antes de la
/// triple falta: necesita una pila que no dependa de ninguna tarea.
static PILA_DF: SyncCell<PilaEmergencia> = SyncCell::new(PilaEmergencia([0; PILA_DF_BYTES]));

extern "C" {
    /// GDT unificada del kernel, definida en `bios.s` (11 entradas).
    static rocky_gdt: [u64; 11];
}

/// TSS de 64 bits: 104 bytes. Solo se usa `RSP0` (la pila del anillo 0 que el
/// procesador carga al entrar desde el anillo 3). El mapa de E/S se desactiva
/// (`I/O map base` = tamano).
#[repr(C, align(16))]
struct Tss([u8; 104]);

static TSS: SyncCell<Tss> = SyncCell::new(Tss([0; 104]));

impl Tss {
    /// Escribe `RSP0`, el campo que esta en el desplazamiento 4.
    fn poner_rsp0(&mut self, valor: u64) {
        unsafe {
            let p = self.0.as_mut_ptr().add(4) as *mut u64;
            core::ptr::write_unaligned(p, valor);
            // I/O map base = tamano del TSS -> sin mapa de E/S.
            let fin = self.0.as_mut_ptr().add(0x66) as *mut u16;
            core::ptr::write_unaligned(fin, 104);
        }
    }
}

/// Completa la GDT con el descriptor del TSS y carga el TSS.
///
/// Debe llamarse **despues** de `bios::init()` (que es quien carga `rocky_gdt`
/// con `lgdt`): el `ltr` lee el descriptor de la GDT que tiene cargada el
/// procesador.
pub fn init() {
    unsafe {
        // Descriptor del TSS: tipo 0x9 (TSS de 64 bits disponible), P = 1,
        // DPL = 0. En modo largo ocupa 16 bytes (dos entradas de 8).
        let tss = TSS.get() as u64;
        let limite: u64 = 103; // tamano del TSS - 1
        let bajo = (limite & 0xFFFF)
            | ((tss & 0x00FF_FFFF) << 16)
            | (0x89u64 << 40)
            | (((tss >> 24) & 0xFF) << 56);
        let alto = (tss >> 32) & 0xFFFF_FFFF;

        let gdt = rocky_gdt.as_ptr() as *mut u64;
        core::ptr::write_volatile(gdt.add(9), bajo);
        core::ptr::write_volatile(gdt.add(10), alto);

        // Al arrancar, el anillo 0 usa la pila del kernel (0x90000). Se deja
        // aqui para que una interrupcion desde el anillo 3 nunca caiga en una
        // RSP0 vacia mientras no haya ninguna tarea en marcha.
        (&mut *TSS.get()).poner_rsp0(0x90000);

        // La pila de emergencia para el doble fallo: la IST1 apunta a su tope
        // (mas alta de la pila; crece hacia abajo). Se fija **antes** del `ltr`
        // para que el TSS ya la tenga cuando se cargue.
        let tope_ist = PILA_DF.get() as u64 + PILA_DF_BYTES as u64;
        (&mut *TSS.get()).poner_ist1(tope_ist);

        asm!("ltr {0:x}", in(reg) TSS_SEL, options(nostack, preserves_flags));
    }
}

impl Tss {
    /// Fija la base de la IST1 (pila de emergencia del doble fallo).
    fn poner_ist1(&mut self, valor: u64) {
        unsafe {
            // IST1 vive en el desplazamiento 0x24 del TSS de 64 bits.
            core::ptr::write_volatile(self.0.as_mut_ptr().add(0x24) as *mut u64, valor);
        }
    }
}

/// Fija la pila del anillo 0 que el procesador cargara en la proxima
/// interrupcion desde el anillo 3 (campo `RSP0` del TSS).
///
/// La llama el planificador al cambiar a una tarea: cada tarea tiene su propia
/// pila de nucleo, y es la que debe usar el manejador de IRQ mientras el
/// programa corre en anillo 3.
pub fn poner_rsp0(valor: u64) {
    let tss = unsafe { &mut *TSS.get() };
    tss.poner_rsp0(valor);
}
