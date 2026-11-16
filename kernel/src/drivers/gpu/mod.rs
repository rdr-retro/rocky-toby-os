//! Tarjetas graficas: despacho por fabricante.
//!
//! La clase GPU agrupa a los controladores de tarjeta grafica. Cada fabricante
//! vive en su fichero ([`amd`] o [`intel`]) y el despacho de aqui
//! prueba los que haya, en orden: el primero que reconoce una tarjeta se queda
//! con la orden. Anadir un fabricante es anadir un fichero y una linea.

pub mod amd;
pub mod atom;
pub mod intel;

use crate::pci;

/// Arranca los controladores de GPU.
pub fn init() {
    amd::init();
    intel::init();
}

/// Orden `gpu`: identifica la tarjeta presente y ensena su estado.
pub fn mostrar() {
    if amd::mostrar() {
        return;
    }
    if intel::mostrar() {
        return;
    }

    // Ningun fabricante con controlador ha reclamado nada: se listan las
    // tarjetas que haya (por ejemplo, la VGA de Bochs de QEMU), que es lo util.
    crate::println!("gpu: no hay ninguna GPU AMD ni Intel en el bus PCI.");
    let mut hay = false;
    for f in pci::funciones() {
        if f.es_video() {
            if !hay {
                crate::println!("  dispositivos de video encontrados:");
                hay = true;
            }
            crate::println!(
                "    {:02X}:{:02X}.{}  {:04X}:{:04X}  {} {}",
                f.bus,
                f.dev,
                f.func,
                f.vendor,
                f.device,
                pci::fabricante_nombre(f.vendor),
                pci::clase_nombre(f.clase, f.subclase)
            );
        }
    }
    if !hay {
        crate::println!("  (no hay ningun dispositivo de video con clase 0x03)");
    }
}

/// Orden `cursor`: cursor por hardware de la GPU.
pub fn cursor(arg: &str) {
    if amd::cursor(arg) {
        return;
    }
    crate::println!("cursor: no hay ninguna GPU con cursor por hardware en el bus.");
}
