//! Intel HD Graphics integradas.
//!
//! El perfil objetivo es Haswell GT2 Mobile (8086:0416), la Intel HD Graphics
//! 4600 del ThinkPad T440p con i7-4712MQ. Rocky conserva el scanout que prepara
//! el firmware y dibuja por su compositor sobre el framebuffer VBE; no programa
//! registros de pantalla Intel ni activa el motor 3D sin GTT/firmware/gestión de
//! energía completos.

use crate::pci::{self, Funcion};

const FABRICANTE: u16 = 0x8086;
const HASWELL_GT2: &[u16] = &[0x0412, 0x0416, 0x041A, 0x041B, 0x041E];

/// Busca una Intel HD Graphics Haswell GT2 conocida.
pub fn buscar() -> Option<Funcion> {
    pci::funciones()
        .iter()
        .copied()
        .find(|f| f.vendor == FABRICANTE && f.es_video() && HASWELL_GT2.contains(&f.device))
}

/// Reconoce la iGPU sin reinicializarla ni interrumpir el modo que dejó la BIOS.
pub fn init() {
    if let Some(gpu) = buscar() {
        crate::println!(
            "rocky: Intel Haswell GT2 {:04X}:{:04X} detectada: {}.",
            gpu.vendor,
            gpu.device,
            pci::producto_nombre(gpu.vendor, gpu.device).unwrap_or("HD Graphics")
        );
        crate::println!(
            "rocky: Intel display conserva el modo VBE del firmware; aceleración 3D nativa aún no habilitada."
        );
    }
}

/// Orden `gpu`: informa de la iGPU Intel identificada.
pub fn mostrar() -> bool {
    let Some(gpu) = buscar() else { return false };

    let subsistema = pci::leer32(gpu.bus, gpu.dev, gpu.func, 0x2C);
    let comando = pci::leer32(gpu.bus, gpu.dev, gpu.func, 0x04) as u16;
    crate::println!(
        "GPU Intel {:04X}:{:04X} - {} - en {:02X}:{:02X}.{}",
        gpu.vendor,
        gpu.device,
        pci::producto_nombre(gpu.vendor, gpu.device).unwrap_or("Intel HD Graphics"),
        gpu.bus,
        gpu.dev,
        gpu.func
    );
    crate::println!(
        "  revision 0x{:02X}, subsistema {:04X}:{:04X}, PCI memoria {} bus-master {}",
        gpu.revision,
        (subsistema >> 16) as u16,
        subsistema as u16,
        comando & (1 << 1) != 0,
        comando & (1 << 2) != 0
    );
    crate::println!(
        "  salida activa: framebuffer VBE configurado por BIOS; composición de Rocky por CPU."
    );
    crate::println!(
        "  nota: no hay aún aceleración Intel 3D/GTT; no se alteró el estado de la GPU."
    );
    true
}
