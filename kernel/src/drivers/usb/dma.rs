//! Reserva de memoria para DMA de los controladores USB.
//!
//! Es el puente entre el gestor de marcos del kernel ([`crate::pmm`]) y el
//! trait [`rockyhw::dma::Marcos`], que es donde vive toda la logica de
//! reserva, validacion y liberacion (alineacion, tope fisico, region's en uso,
//! doble liberacion...). Aqui no hay reglas: solo la adaptacion.
//!
//! Codigo propio de rocky.

use rockyhw::dma::{DmaRegion, ErrorDma, Marcos, LIMITE_4GIB};

/// El gestor de marcos del kernel, visto como [`Marcos`].
pub struct MarcosDelKernel;

impl Marcos for MarcosDelKernel {
    fn reservar(&mut self, count: usize, align: usize, limite_fisico: u64) -> Option<usize> {
        crate::pmm::alloc_frames_aligned(count, align, limite_fisico)
    }

    fn devolver(&mut self, addr: usize, count: usize) {
        crate::pmm::free_frames(addr, count);
    }

    fn poner_cero(&mut self, addr: usize, count: usize) {
        crate::pmm::zero(addr, count);
    }
}

/// Reserva una region DMA para un controlador USB.
///
/// Usa el tope fisico por defecto (4 GiB), que es lo que cubre el mapa por
/// identidad del arranque: una region por encima de eso no se podria
/// direccionar sin ampliar la paginacion.
///
/// La region sale a cero y **no** se devuelve al gestor de memoria hasta que
/// se llame a [`DmaRegion::liberar`], que a su vez se niega mientras el
/// dispositivo la tenga marcada en uso.
pub fn reservar(marcos: usize, alineacion: usize) -> Result<DmaRegion, ErrorDma> {
    let region = DmaRegion::nueva(&mut MarcosDelKernel, marcos, alineacion, LIMITE_4GIB)?;
    // Los ceros tienen que estar visibles para el dispositivo antes de que se
    // le diga que use la region: de ahi la barrera de orden de escrituras.
    rockyhw::mmio::sfence();
    Ok(region)
}
