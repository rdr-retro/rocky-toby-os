//! Controlador de interrupciones programable 8259A (PIC).
//!
//! Se remapea para que las IRQ lleguen a los vectores 0x20..0x2F.

use crate::cpu::{inb, io_wait, outb};

pub unsafe fn init() {
    // ICW1: inicializacion
    outb(0x20, 0x11);
    io_wait();
    outb(0xA0, 0x11);
    io_wait();
    // ICW2: desplazamiento de vectores
    outb(0x21, 0x20);
    io_wait();
    outb(0xA1, 0x28);
    io_wait();
    // ICW3: conexion maestro/esclavo
    outb(0x21, 0x04);
    io_wait();
    outb(0xA1, 0x02);
    io_wait();
    // ICW4: modo 8086
    outb(0x21, 0x01);
    io_wait();
    outb(0xA1, 0x01);
    io_wait();
    // IRQ0 (reloj), IRQ1 (teclado), IRQ2 (cascada), IRQ4 (serie) e IRQ12
    // (raton PS/2) activos. Los dos canales PS/2 usan colas acotadas en sus
    // manejadores; no se leen desde el bucle de pintado de Kira.
    outb(0x21, 0xE8);
    outb(0xA1, 0xEF);

    // Consume cualquier estado pendiente del PIC.
    let _ = inb(0x21);
}

/// Enmascara temporalmente teclado y raton mientras se intercambian comandos
/// con el 8042. Asi los ACK quedan disponibles para el sondeo de inicializacion.
pub unsafe fn enmascarar_ps2() {
    crate::cpu::sin_interrupciones(|| {
        outb(0x21, inb(0x21) | (1 << 1));
        outb(0xA1, inb(0xA1) | (1 << 4));
    });
}

/// Reactiva IRQ1, IRQ12 y la cascada despues de configurar el 8042.
pub unsafe fn habilitar_ps2() {
    crate::cpu::sin_interrupciones(|| {
        outb(0x21, inb(0x21) & !(1 << 2) & !(1 << 1));
        outb(0xA1, inb(0xA1) & !(1 << 4));
    });
}

pub unsafe fn eoi(irq: u8) {
    if irq >= 8 {
        outb(0xA0, 0x20);
    }
    outb(0x20, 0x20);
}
