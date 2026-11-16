//! Servicio de disco serializado (Fase 5 Marea).
//!
//! Propietario único de las operaciones de disco. Las peticiones se serializan
//! con una sección crítica breve para adquirir el disco; el I/O real se hace
//! FUERA de `tarea::critica` para permitir la preempción (Kira/cursor siguen
//! respondiendo mientras el disco espera).

use crate::sync::SyncCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tipo {
    Leer,
    Escribir,
}

struct Estado {
    activo: bool,
    ocupado: bool,
    duenio: u64,
    desde: u64,
    lba_actual: u32,
}

static ESTADO: SyncCell<Estado> = SyncCell::new(Estado {
    activo: false,
    ocupado: false,
    duenio: 0,
    desde: 0,
    lba_actual: 0,
});



fn estado() -> &'static mut Estado {
    unsafe { &mut *ESTADO.get() }
}

/// Inicializa el servicio de disco (idempotente).
pub fn init() {
    let e = estado();
    if e.activo {
        return;
    }
    e.activo = true;
    e.ocupado = false;
}

/// Encola/ejecuta una petición de disco de forma serializada.
/// El I/O se realiza fuera de `tarea::critica` para no bloquear la preempción.
pub fn pedir(tipo: Tipo, lba: u32, buf: &mut [u8], len: u32) -> bool {
    if len == 0 || (len as usize) > buf.len() || len > 512 {
        return false;
    }
    // Adquirir el disco con sección crítica breve
    let mut precio = 0u64;
    let mut proximo_aviso = 500u64;
    loop {
        let adquirido = crate::tarea::critica(|| {
            let e = estado();
            if !e.ocupado {
                e.ocupado = true;
                e.duenio = crate::tarea::actual_id();
                e.desde = crate::pit::ticks();
                e.lba_actual = lba;
                true
            } else {
                false
            }
        });
        if adquirido {
            break;
        }
        // --- Depuracion temporal (quitar) ---
        {
            let (duenio, desde, lba_duenio) = crate::tarea::critica(|| {
                let e = estado();
                (e.duenio, e.desde, e.lba_actual)
            });
            let ahora = crate::pit::ticks();
            let esperando = ahora.wrapping_sub(desde);
            if esperando > proximo_aviso {
                proximo_aviso = esperando + 500;
                crate::println!(
                    "disco: bloqueado por tarea {} (lba {}) desde hace {} ticks; espera tarea {} (lba {}, {} vueltas)",
                    duenio,
                    lba_duenio,
                    esperando,
                    crate::tarea::actual_id(),
                    lba,
                    precio
                );
                crate::tarea::volcar_pilas();
            }
        }
        precio = precio.wrapping_add(1);
        // Esperar al disco dentro de una sección crítica es un error: el
        // planificador no cambia de tarea y el dueño del disco no podría
        // terminar de liberarlo, congelando el sistema. Se cancela la petición
        // en vez de bloquear el planificador para siempre.
        if crate::tarea::en_critica() {
            crate::println!("disco: peticion dentro de una seccion critica; se cancela");
            return false;
        }
        // Ceder para que otra tarea/libere el disco
        unsafe {
            crate::tarea::ceder();
        }
    }
    // Hacer I/O FUERA de critica para permitir preempción
    let res = if tipo == Tipo::Leer {
        let mut tmp = [0u8; 512];
        match crate::disk::leer(lba, &mut tmp) {
            Ok(()) => {
                buf[..len as usize].copy_from_slice(&tmp[..len as usize]);
                true
            }
            Err(()) => false,
        }
    } else {
        let mut tmp = [0u8; 512];
        tmp[..len as usize].copy_from_slice(&buf[..len as usize]);
        crate::disk::escribir(lba, &tmp).is_ok()
    };
    // Liberar disco con sección crítica breve
    crate::tarea::critica(|| {
        let e = estado();
        e.ocupado = false;
    });
    res
}
