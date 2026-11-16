//! Servicios: subsistemas que pueden estar **activos** o **inactivos**.
//!
//! Un servicio tiene un nombre, una descripcion y un estado. Las etiquetas del
//! estado van en ingles a proposito (asi se pidio): `active` / `inactive`.
//!
//! Al arrancar rocky se registran los servicios de serie y se pone en marcha el
//! reloj (servicio `time`). Desde el interprete de ordenes, `servicio` los lista
//! y `servicio start|stop <nombre>` los arranca o los para.
//!
//! Un servicio puede tener una **tarea de fondo** en el planificador
//! ([`crate::tarea`]): arrancarlo la crea y pararlo la mata. El servicio `time`
//! mantiene asi la hora y la fecha en la barra superior de TobyUI, y el
//! servicio `kira` su compositor (escritorio, menu y cursor del raton).
//!
//! Codigo propio de rocky.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cmos;
use crate::sync::SyncCell;
use crate::tarea;

/// Estado de un servicio.
///
/// Las etiquetas visibles van en ingles (`active` / `inactive`), como se pidio.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// En marcha.
    Activo,
    /// Parado.
    Inactivo,
}

impl Estado {
    /// Etiqueta del estado, en ingles: `active` o `inactive`.
    pub fn etiqueta(self) -> &'static str {
        match self {
            Estado::Activo => "active",
            Estado::Inactivo => "inactive",
        }
    }
}

/// Un servicio registrado.
struct Servicio {
    nombre: &'static str,
    descripcion: &'static str,
    estado: Estado,
    /// Tarea de fondo asociada (0 = ninguna).
    tarea: u64,
}

/// La tabla de servicios.
struct Registro {
    servicios: Vec<Servicio>,
    iniciado: bool,
}

static REG: SyncCell<Registro> = SyncCell::new(Registro {
    servicios: Vec::new(),
    iniciado: false,
});

fn reg() -> &'static mut Registro {
    // Seguro: todos los accesos van dentro de una seccion critica de tareas
    // (`tarea::critica`), que impide que el planificador cambie de tarea a mitad.
    unsafe { &mut *REG.get() }
}

/// Hora y fecha que mantiene el servicio `time`, para la barra del escritorio.
/// `None` mientras el servicio esta parado.
static RELOJ: SyncCell<Option<String>> = SyncCell::new(None);

/// Registra los servicios de serie y arranca el reloj. Es idempotente.
pub fn init() {
    tarea::critica(|| {
        let r = reg();
        if r.iniciado {
            return;
        }
        r.iniciado = true;
        r.servicios.push(Servicio {
            nombre: "time",
            descripcion: "hora y fecha del sistema",
            estado: Estado::Inactivo,
            tarea: 0,
        });
        r.servicios.push(Servicio {
            nombre: "kira",
            descripcion: "escritorio y servidor de ventanas Kira",
            estado: Estado::Inactivo,
            tarea: 0,
        });
    });
    // El reloj es un servicio de aspecto: arranca solo, pero se puede parar.
    let _ = arrancar("time");
}

/// Lista `(nombre, estado_en_ingles, descripcion)` de todos los servicios.
pub fn listar() -> Vec<(&'static str, &'static str, &'static str)> {
    tarea::critica(|| {
        reg()
            .servicios
            .iter()
            .map(|s| (s.nombre, s.estado.etiqueta(), s.descripcion))
            .collect()
    })
}

/// Etiqueta del estado de un servicio (`active`/`inactive`), o `None` si no
/// existe.
pub fn estado_etiqueta(nombre: &str) -> Option<&'static str> {
    indice(nombre).map(estado_de).map(Estado::etiqueta)
}

/// Arranca un servicio. Si ya estaba activo no hace nada. Devuelve error si el
/// servicio no existe.
pub fn arrancar(nombre: &str) -> Result<(), String> {
    let i = indice(nombre).ok_or_else(|| format!("no existe el servicio '{}'", nombre))?;
    if estado_de(i) == Estado::Activo {
        return Ok(());
    }
    // Servicios con tarea de fondo o con estado propio.
    let tarea_id = match nombre {
        "time" => {
            // Deja el reloj ya puesto, para que no parpadee hasta el primer tic.
            fijar_reloj(Some(texto_reloj()));
            tarea::crear("servicio time", tarea_time, 0)
        }
        "kira" => {
            // Carga la fuente instalada antes del primer repintado; si no
            // existe, el compositor conserva la fuente del BIOS.
            crate::fuentes::inicializar();
            // El servidor de ventanas Kira, con su compositor (escritorio,
            // barra de menú y cursor del ratón): la tarea sigue al
            // raton y refresca la pantalla mientras el interprete este en ella.
            crate::kira::iniciar(1024, 768);
            crate::kira::comp::arrancar();
            tarea::crear("servicio kira", crate::kira::comp::tarea_kira, 0)
        }
        _ => 0,
    };
    tarea::critica(|| {
        let s = &mut reg().servicios[i];
        s.estado = Estado::Activo;
        s.tarea = tarea_id;
    });
    Ok(())
}

/// Para un servicio. Si ya estaba inactivo no hace nada. Devuelve error si el
/// servicio no existe.
pub fn parar(nombre: &str) -> Result<(), String> {
    let i = indice(nombre).ok_or_else(|| format!("no existe el servicio '{}'", nombre))?;
    if estado_de(i) == Estado::Inactivo {
        return Ok(());
    }
    let tarea_id = tarea::critica(|| {
        let s = &mut reg().servicios[i];
        let id = s.tarea;
        s.estado = Estado::Inactivo;
        s.tarea = 0;
        id
    });
    if tarea_id != 0 {
        tarea::matar(tarea_id);
    }
    if nombre == "time" {
        fijar_reloj(None);
    }
    if nombre == "kira" {
        crate::kira::parar();
        // El compositor deja el video: el interprete vuelve al modo texto.
        crate::kira::comp::parar();
    }
    Ok(())
}

/// Texto del reloj para la barra del escritorio, si el servicio `time` esta
/// activo (y por tanto lo ha puesto).
///
/// Lo leia la barra del escritorio grafico antiguo; se conserva como acceso al
/// estado del servicio.
#[allow(dead_code)]
pub fn reloj() -> Option<String> {
    tarea::critica(|| unsafe { (*RELOJ.get()).clone() })
}

fn indice(nombre: &str) -> Option<usize> {
    tarea::critica(|| reg().servicios.iter().position(|s| s.nombre == nombre))
}

fn estado_de(i: usize) -> Estado {
    tarea::critica(|| {
        reg()
            .servicios
            .get(i)
            .map(|s| s.estado)
            .unwrap_or(Estado::Inactivo)
    })
}

fn fijar_reloj(texto: Option<String>) {
    tarea::critica(|| unsafe { *RELOJ.get() = texto });
}

/// Formatea la hora y la fecha actuales (p. ej. `16/10/2026 14:07:33`).
fn texto_reloj() -> String {
    let (d, m, a) = cmos::date();
    let (h, mi, s) = cmos::time();
    format!("{:02}/{:02}/{:04} {:02}:{:02}:{:02}", d, m, a, h, mi, s)
}

/// Tarea de fondo del servicio `time`: refresca el reloj dos veces por segundo.
extern "C" fn tarea_time(_arg: u64) -> ! {
    loop {
        fijar_reloj(Some(texto_reloj()));
        tarea::dormir(500);
    }
}
