//! Tareas: multitarea apropiativa dentro del anillo 0 (el propio nucleo).
//!
//! Cada tarea tiene su **propia pila** y una "foto" de sus registros guardada en
//! ella. La foto es ni mas ni menos que el marco que deja el stub de
//! interrupciones ([`crate::idt::InterruptStack`]): por eso el cambio de tarea es
//! tan barato: se cambia el puntero de pila y el `iretq` final de
//! `interrupts.s` devuelve el control a la tarea elegida.
//!
//! El planificador es de **turno circular** y apropiativo: la IRQ0 (100 Hz)
//! entra por [`planificar`], guarda la tarea en curso y elige la siguiente de la
//! lista. Una tarea tambien puede dejar el turno a mano con [`ceder`] (vector
//! 48, `int 0x30`), que es lo que hacen [`dormir`] y [`terminar`].
//!
//! Aun no hay aislamiento: todas las tareas comparten el espacio de direcciones
//! del nucleo y corren en anillo 0 (como el propio nucleo). La separacion de
//! procesos (anillo 3, espacio propio) es el paso siguiente, y por eso todo el
//! estado de una tarea vive en su [`Tarea`]: manana esa estructura se lleva las
//! tablas de paginas del proceso.

use core::arch::asm;

use alloc::string::String;
use alloc::vec::Vec;

use crate::cpu::{self, sin_interrupciones};
use crate::gdt;
use crate::idt::InterruptStack;
use crate::paging;
use crate::pit;
use crate::proceso::Proceso;
use crate::sync::SyncCell;

/// Igual que [`sin_interrupciones`], pero sin obligar a cada llamante a abrir un
/// bloque `unsafe`.
fn sin_irq<F: FnOnce() -> R, R>(f: F) -> R {
    unsafe { sin_interrupciones(f) }
}

/// Tamano de la pila de cada tarea (64 KiB). Da de sobra para el interprete de
/// ordenes y para dibujar, y no es tanto como para que no quepan unas cuantas.
const PILA_BYTES: usize = 64 * 1024;
/// Estado de una tarea dentro del planificador.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// Puede correr en cuanto le toque el turno.
    Listo,
    /// Es la que esta corriendo ahora mismo.
    Ejecutando,
    /// No correra hasta que el reloj llegue a su hora de despertar.
    Durmiendo,
    /// Ha terminado: el recolector la sacara de la lista y liberara su pila.
    Muerto,
    /// Proceso terminado cuyo padre aun no lo ha recogido con `waitpid`. No se
    /// planifica; el padre lo saca de la lista al recogerlo (o el recolector si
    /// se queda huerfano).
    Zombi,
}

/// Relleno alineado a 16 bytes.
///
/// Una `Vec<Taco>` reserva la memoria alineada a 16, que es justo lo que la ABI
/// de x86_64 supone para la pila (el compilador puede emitir instrucciones SSE
/// que exigen esa alineacion).
#[repr(align(16))]
#[derive(Clone, Copy)]
struct Taco(#[allow(dead_code)] u8);

/// Cuantos `Taco` forman una pila (la alineacion obliga a que cada uno ocupe 16
/// bytes, asi que hay que dividir por su tamano real, no usarlo como bytes).
const TACOS: usize = PILA_BYTES / core::mem::size_of::<Taco>();

/// La pila de una tarea.
struct Pila(Vec<Taco>);

impl Pila {
    fn nueva() -> Pila {
        Pila(alloc::vec![Taco(0); TACOS])
    }

    /// Vacia: la usa la tarea del nucleo, que ya trae la suya.
    const fn vacia() -> Pila {
        Pila(Vec::new())
    }

    fn base(&self) -> u64 {
        self.0.as_ptr() as u64
    }

    fn tope(&self) -> u64 {
        self.base() + (self.0.len() * core::mem::size_of::<Taco>()) as u64
    }
}

/// Una tarea viva.
pub struct Tarea {
    /// La pila propia (la del nucleo esta vacia: es la de arranque). El campo
    /// no se lee nunca: solo existe para que la memoria se libere al morir la
    /// tarea (Drop).
    #[allow(dead_code)]
    pila: Pila,
    /// Puntero al marco de interrupciones guardado en su pila.
    rsp: u64,
    pub id: u64,
    pub nombre: String,
    estado: Estado,
    /// Tarea idle del procesador. Existe siempre, pero no aparece como proceso
    /// ni compite por un turno mientras haya trabajo normal listo.
    es_idle: bool,
    /// Tick del reloj en el que la tarea debe despertar.
    despertar: u64,
    /// Espacio de direcciones del proceso, si la tarea es de anillo 3. Al
    /// soltarse libera su PML4, sus tablas y sus paginas.
    proceso: Option<Proceso>,
    /// PID del padre (0 = sin padre: tarea del nucleo o proceso raiz). Lo fija
    /// `fork` en los hijos.
    padre: u64,
    /// Codigo con el que termino el proceso (lo devuelve `waitpid`).
    codigo_salida: i64,
    /// Si la tarea esta bloqueada en `waitpid`, PID del hijo que espera (o
    /// [`ESPERA_CUALQUIERA`]). 0 = no esta esperando a nadie.
    esperando_a: u64,
    /// Si la tarea esta bloqueada en una tuberia (Fase 6), identificador de esa
    /// tuberia. 0 = no espera a ninguna. Solo la despierta un cambio en ella
    /// (datos nuevos, hueco nuevo o el cierre de un extremo); nunca el reloj.
    esperando_tuberia: u32,
    /// Tabla de descriptores de fichero (Linux 0.01: el `filp[]` de
    /// `task_struct`). Cada entrada es un indice a la tabla global de
    /// [`crate::fd`], o [`crate::fd::LIBRE`]. `fork` la copia y `exit` la cierra.
    pub fds: [u32; crate::fd::NR_OPEN],
    /// Estado de senales (Fase 7): pendientes, mascara de bloqueo, acciones
    /// instaladas y si la tarea esta en `pause` o parada. Lo gestiona
    /// [`crate::senal`].
    pub senales: crate::senal::Senales,
    /// **Grupo de procesos** (y sesion) de la tarea, el `pgrp` de Linux 0.01
    /// (Fase 8). El terminal dirige las senales de control al grupo en primer
    /// plano, y `setsid` estrena uno nuevo (el del propio PID). 0 = sin grupo
    /// (las tareas del nucleo).
    pub pgrp: u64,
    /// **Directorio de trabajo** de la tarea (Fase 9): el registro (inodo) desde
    /// el que se resuelven las rutas **relativas** de `open`, `execve` y `chdir`.
    /// El hijo lo **hereda** en `fork` y `execve` lo conserva; lo cambia `chdir`.
    /// 0 = sin volumen.
    pub cwd: u32,
    // --- Fase 3 Marea: nodos intrusivos del planificador --------------------
    /// Clase de prioridad; decide en que cola de listos entra (Fase 3).
    clase: Clase,
    /// Enlace al siguiente/anterior en la cola de listos de su clase
    /// ([`NINGUNA`] = no esta en ninguna cola). Son **indices** al vector de
    /// tareas, no punteros: el vector puede realojarse sin invalidarlos, y
    /// borrar una tarea reajusta los indices (ver `quitar_tarea`).
    sig_listo: usize,
    ant_listo: usize,
    /// Enlace al siguiente/anterior en la **lista de vencimientos** (solo
    /// tareas con plazo de reloj finito), ordenada por `despertar`. Permite
    /// despertar solo a quien vence en vez de recorrer todas las tareas.
    sig_vence: usize,
    ant_vence: usize,
}

/// Clase de prioridad de una tarea (Fase 3 Marea).
///
/// Dos clases bastan para el objetivo del plan: que la interfaz (escritorio,
/// Kira, interprete de ordenes) responda aunque haya trabajo de fondo, sin que
/// esa misma clase pueda monopolizar el procesador. El reparto lo impone el
/// presupuesto de [`PRESUPUESTO_EQUIDAD`], no la clase por si sola.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Clase {
    /// Trabajo con el que el usuario interactua: escritorio, Kira, shell.
    Interactiva,
    /// Todo lo demas: procesos de usuario y servicios de fondo.
    Normal,
}

impl Clase {
    /// Cuantas clases hay (tamano de los vectores de cabezas de cola).
    const CUANTAS: usize = 2;

    const fn indice(self) -> usize {
        match self {
            Clase::Interactiva => 0,
            Clase::Normal => 1,
        }
    }

    /// Clase por defecto segun el tipo de tarea: los procesos de anillo 3
    /// (los que lanza el usuario) son trabajo normal; las tareas del nucleo
    /// son las que dan servicio a la interfaz.
    const fn por_defecto(es_proceso: bool) -> Clase {
        if es_proceso {
            Clase::Normal
        } else {
            Clase::Interactiva
        }
    }

    const fn etiqueta(self) -> &'static str {
        match self {
            Clase::Interactiva => "interactiva",
            Clase::Normal => "normal",
        }
    }
}

/// Valor de [`Tarea::esperando_a`] que significa "a cualquier hijo".
pub const ESPERA_CUALQUIERA: u64 = u64::MAX;

/// Presupuesto de equidad entre clases (Fase 3 Marea).
///
/// Como maximo se despachan `PRESUPUESTO_EQUIDAD` turnos seguidos de la clase
/// interactiva mientras haya trabajo normal esperando; el siguiente turno es
/// para la clase normal. Asi la interfaz manda (responde antes) pero no puede
/// dejar sin CPU al resto: la espera maxima de una tarea normal es de
/// `PRESUPUESTO_EQUIDAD + 1` turnos. El plan pide exactamente esto: "evitar que
/// la clase interactiva monopolice CPU".
const PRESUPUESTO_EQUIDAD: u32 = 8;

struct Planificador {
    tareas: Vec<Tarea>,
    /// Indice de la tarea en curso; [`NINGUNA`] mientras no haya ninguna.
    actual: usize,
    siguiente_id: u64,
    activo: bool,
    // --- Fase 3 Marea: colas de listos y vencimientos -----------------------
    /// Cabeza y cola de la cola de listos de cada clase (`NINGUNA` = vacia).
    /// Son listas intrusivas doblemente enlazadas por indices: insertar y
    /// quitar es O(1) y **no reserva memoria**, que es la condicion del plan
    /// para poder hacerlo desde la IRQ del reloj.
    cabeza_listo: [usize; Clase::CUANTAS],
    cola_listo: [usize; Clase::CUANTAS],
    /// Cabeza de la lista de vencimientos: tareas durmiendo con plazo de reloj
    /// finito, ordenadas de menor a mayor `despertar`. Despertar a quien vence
    /// cuesta O(vencidos) en vez de O(tareas).
    cabeza_vence: usize,
    /// Turnos consecutivos que lleva la clase interactiva con trabajo normal
    /// esperando (lo consume el presupuesto de [`PRESUPUESTO_EQUIDAD`]).
    racha_interactiva: u32,
}

const NINGUNA: usize = usize::MAX;

impl Planificador {
    const fn nuevo() -> Self {
        Planificador {
            tareas: Vec::new(),
            actual: NINGUNA,
            siguiente_id: 1,
            activo: false,
            cabeza_listo: [NINGUNA; Clase::CUANTAS],
            cola_listo: [NINGUNA; Clase::CUANTAS],
            cabeza_vence: NINGUNA,
            racha_interactiva: 0,
        }
    }
}

static PLAN: SyncCell<Planificador> = SyncCell::new(Planificador::nuevo());

fn plan() -> &'static mut Planificador {
    // Seguro: todo acceso pasa por IRQ desactivadas o por el propio manejador
    // de IRQ (que ya corre con las interrupciones tapadas).
    unsafe { &mut *PLAN.get() }
}

/// Enciende la multitarea. A partir de aqui, el primer tic del reloj registra la
/// tarea en curso (el nucleo) como tarea 0. La tarea idle se crea antes de
// ---------------------------------------------------------------------------
// Fase 3 Marea: API pequena de espera/despertar sobre colas intrusivas
// ---------------------------------------------------------------------------
//
// Toda transicion `Listo -> ...` y `... -> Listo` pasa por aqui. Es la unica
// forma de garantizar que las colas y la lista de vencimientos siempre
// concuerdan con el estado de las tareas (lo comprueba `comprobar_invariantes`).
//
// Las colas son **intrusivas**: los enlaces viven dentro de `Tarea` y son
// indices, asi que encolar y desencolar no reserva memoria. Es obligatorio
// porque se hace desde la IRQ del reloj (punto 2 del plan).

/// Mete `i` al final de la cola de listos de su clase.
///
/// Idempotente: si ya estaba en una cola, no lo encola dos veces (dejarlo
/// duplicado haria que la tarea se despachase dos veces por vuelta).
fn encolar_listo(p: &mut Planificador, i: usize) {
    // `idle` no compite por turnos y una tarea ya encolada no se duplica (si
    // se duplicase, se despacharia dos veces por vuelta).
    if p.tareas[i].es_idle || esta_en_cola(p, i) {
        return;
    }
    let c = p.tareas[i].clase.indice();
    p.tareas[i].sig_listo = NINGUNA;
    p.tareas[i].ant_listo = p.cola_listo[c];
    if p.cola_listo[c] != NINGUNA {
        p.tareas[p.cola_listo[c]].sig_listo = i;
    }
    p.cola_listo[c] = i;
    if p.cabeza_listo[c] == NINGUNA {
        p.cabeza_listo[c] = i;
    }
}

/// `true` si `i` esta en la cola de listos de su clase.
fn esta_en_cola(p: &Planificador, i: usize) -> bool {
    let c = p.tareas[i].clase.indice();
    if p.cabeza_listo[c] == i || p.cola_listo[c] == i {
        return true;
    }
    p.tareas[i].sig_listo != NINGUNA || p.tareas[i].ant_listo != NINGUNA
}

/// Saca `i` de la cola de listos en la que este (si esta).
fn desencolar_listo(p: &mut Planificador, i: usize) {
    if !esta_en_cola(p, i) {
        return;
    }
    let c = p.tareas[i].clase.indice();
    let ant = p.tareas[i].ant_listo;
    let sig = p.tareas[i].sig_listo;
    if ant != NINGUNA {
        p.tareas[ant].sig_listo = sig;
    } else {
        p.cabeza_listo[c] = sig;
    }
    if sig != NINGUNA {
        p.tareas[sig].ant_listo = ant;
    } else {
        p.cola_listo[c] = ant;
    }
    p.tareas[i].sig_listo = NINGUNA;
    p.tareas[i].ant_listo = NINGUNA;
}

/// Saca de su cola la cabeza de `clase` y la devuelve ([`NINGUNA`] si vacia).
fn sacar_cabeza(p: &mut Planificador, clase: Clase) -> usize {
    let c = clase.indice();
    let i = p.cabeza_listo[c];
    if i != NINGUNA {
        desencolar_listo(p, i);
    }
    i
}

/// `true` si la cola de listos de `clase` tiene alguna tarea.
fn hay_listos(p: &Planificador, clase: Clase) -> bool {
    p.cabeza_listo[clase.indice()] != NINGUNA
}

/// Inserta `i` en la lista de vencimientos (ordenada por `despertar`).
///
/// Solo para tareas con plazo de reloj **finito**. Recorrer la lista para
/// colocarla es O(durmientes), no O(tareas): los durmientes por reloj suelen
/// ser pocos (el resto espera eventos, no tiempo).
fn encolar_vence(p: &mut Planificador, i: usize) {
    let clave = p.tareas[i].despertar;
    let mut prev = NINGUNA;
    let mut cur = p.cabeza_vence;
    while cur != NINGUNA && p.tareas[cur].despertar <= clave {
        prev = cur;
        cur = p.tareas[cur].sig_vence;
    }
    p.tareas[i].ant_vence = prev;
    p.tareas[i].sig_vence = cur;
    if cur != NINGUNA {
        p.tareas[cur].ant_vence = i;
    }
    if prev != NINGUNA {
        p.tareas[prev].sig_vence = i;
    } else {
        p.cabeza_vence = i;
    }
}

/// `true` si `i` esta en la lista de vencimientos.
fn esta_en_vence(p: &Planificador, i: usize) -> bool {
    if p.cabeza_vence == i {
        return true;
    }
    p.tareas[i].sig_vence != NINGUNA || p.tareas[i].ant_vence != NINGUNA
}

/// Saca `i` de la lista de vencimientos (si esta).
fn desencolar_vence(p: &mut Planificador, i: usize) {
    if !esta_en_vence(p, i) {
        return;
    }
    let ant = p.tareas[i].ant_vence;
    let sig = p.tareas[i].sig_vence;
    if ant != NINGUNA {
        p.tareas[ant].sig_vence = sig;
    } else {
        p.cabeza_vence = sig;
    }
    if sig != NINGUNA {
        p.tareas[sig].ant_vence = ant;
    }
    p.tareas[i].sig_vence = NINGUNA;
    p.tareas[i].ant_vence = NINGUNA;
}

/// Pone `i` a dormir hasta el tick `despertar` (`u64::MAX` = sin plazo: solo la
/// despertara un evento, y entonces no entra en la lista de vencimientos).
fn dormir_hasta(p: &mut Planificador, i: usize, despertar: u64) {
    p.tareas[i].estado = Estado::Durmiendo;
    p.tareas[i].despertar = despertar;
    if despertar != u64::MAX && !esta_en_vence(p, i) {
        encolar_vence(p, i);
    }
}

/// Despierta `i`: lo saca de donde este durmiendo y lo encola como listo.
///
/// Unico camino autorizado para pasar a `Listo`. Es idempotente y seguro
/// llamarlo sobre una tarea ya lista (no la encola dos veces), porque los
/// caminos de senales y tuberias pueden coincidir sobre la misma tarea.
fn despertar(p: &mut Planificador, i: usize) {
    if i == NINGUNA {
        return;
    }
    desencolar_vence(p, i);
    if p.tareas[i].estado == Estado::Listo {
        return;
    }
    if p.tareas[i].estado == Estado::Durmiendo {
        p.tareas[i].estado = Estado::Listo;
        // El plazo ya no aplica: se limpia para que la lista de vencimientos y
        // el estado no puedan contradecirse.
        p.tareas[i].despertar = 0;
    }
    if p.tareas[i].estado == Estado::Listo {
        encolar_listo(p, i);
    }
}

/// Borra la tarea `i` del vector reajustando colas, vencimientos e indices.
///
/// `Vec::remove` desplaza todos los indices posteriores, y las colas guardan
/// indices: hay que corregirlos o quedarian apuntando a otra tarea. Es O(cola +
/// durmientes), y solo ocurre al recoger o recolectar una tarea, nunca en el
/// camino del tic.
///
/// Devuelve la `Tarea` para que el llamante libere sus recursos.
fn quitar_tarea(p: &mut Planificador, i: usize) -> Tarea {
    desencolar_listo(p, i);
    desencolar_vence(p, i);

    let t = p.tareas.remove(i);

    // Un indice `j > i` pasa a valer `j - 1` tras el `remove`.
    let ajustar = |j: usize| -> usize {
        if j == NINGUNA {
            NINGUNA
        } else if j > i {
            j - 1
        } else {
            j
        }
    };

    for c in 0..Clase::CUANTAS {
        p.cabeza_listo[c] = ajustar(p.cabeza_listo[c]);
        p.cola_listo[c] = ajustar(p.cola_listo[c]);
    }
    p.cabeza_vence = ajustar(p.cabeza_vence);
    for k in 0..p.tareas.len() {
        p.tareas[k].sig_listo = ajustar(p.tareas[k].sig_listo);
        p.tareas[k].ant_listo = ajustar(p.tareas[k].ant_listo);
        p.tareas[k].sig_vence = ajustar(p.tareas[k].sig_vence);
        p.tareas[k].ant_vence = ajustar(p.tareas[k].ant_vence);
    }
    if p.actual != NINGUNA {
        p.actual = ajustar(p.actual);
    }

    t
}

/// Despierta a todas las tareas cuyo plazo de reloj ya vencio (Fase 3).
///
/// La lista esta ordenada por `despertar`, asi que basta con mirar la cabeza:
/// en cuanto la cabeza vence mas tarde que `ahora`, el resto tambien. Coste
/// O(vencidos), no O(tareas).
fn despertar_vencidos(p: &mut Planificador, ahora: u64) {
    while p.cabeza_vence != NINGUNA {
        let i = p.cabeza_vence;
        if p.tareas[i].despertar > ahora {
            break;
        }
        crate::metricas::despertar_reloj();
        despertar(p, i);
    }
}

/// Elige la siguiente tarea a la que dar el turno (Fase 3).
///
/// Respeta el **presupuesto de equidad**: la clase interactiva manda, pero como
/// mucho [`PRESUPUESTO_EQUIDAD`] turnos seguidos mientras haya trabajo normal
/// esperando; despues cede un turno a la clase normal. Si no hay trabajo normal,
/// la interactiva no gasta presupuesto (no hay a quien agraviar). Devuelve
/// [`NINGUNA`] si no hay ninguna tarea lista.
fn elegir_siguiente(p: &mut Planificador) -> usize {
    let hay_inter = hay_listos(p, Clase::Interactiva);
    let hay_normal = hay_listos(p, Clase::Normal);

    if hay_inter && hay_normal {
        // Racha agotada: le toca a la normal, se reinicia el contador.
        if p.racha_interactiva >= PRESUPUESTO_EQUIDAD {
            p.racha_interactiva = 0;
            crate::metricas::equidad_cede();
            crate::metricas::turno(false);
            return sacar_cabeza(p, Clase::Normal);
        }
        p.racha_interactiva += 1;
        crate::metricas::turno(true);
        return sacar_cabeza(p, Clase::Interactiva);
    }
    if hay_inter {
        // Sin rival normal, la racha no corre.
        p.racha_interactiva = 0;
        crate::metricas::turno(true);
        return sacar_cabeza(p, Clase::Interactiva);
    }
    if hay_normal {
        p.racha_interactiva = 0;
        crate::metricas::turno(false);
        return sacar_cabeza(p, Clase::Normal);
    }
    NINGUNA
}

/// habilitar el temporizador para que siempre haya un contexto al que cambiar
/// cuando todas las tareas de trabajo esten bloqueadas.
pub fn init() {
    sin_irq(|| {
        let p = plan();
        if p.activo {
            return;
        }
        if !p.tareas.iter().any(|t| t.es_idle) {
            let pila = Pila::nueva();
            let rsp = preparar_marco(&pila, tarea_idle as *const () as u64, 0);
            p.tareas.push(Tarea {
                pila,
                rsp,
                id: u64::MAX,
                nombre: String::from("idle"),
                estado: Estado::Listo,
                es_idle: true,
                despertar: 0,
                proceso: None,
                padre: u64::MAX,
                codigo_salida: 0,
                esperando_a: 0,
                esperando_tuberia: 0,
                fds: crate::fd::fds_iniciales(),
                senales: crate::senal::Senales::nuevo(),
                pgrp: 0,
                cwd: crate::fs::raiz(),
                clase: Clase::por_defecto(false),
                sig_listo: NINGUNA,
                ant_listo: NINGUNA,
                sig_vence: NINGUNA,
                ant_vence: NINGUNA,
            });
        }
        p.activo = true;
    });
}

/// No tiene trabajo que ejecutar: `sti; hlt` duerme hasta la siguiente IRQ sin
/// la ventana de carrera que tendría un `hlt` con IF aún desactivado.
extern "C" fn tarea_idle(_arg: u64) -> ! {
    loop {
        unsafe { asm!("sti; hlt", options(nomem, nostack)) };
    }
}

/// Indica si la multitarea esta en marcha.
pub fn activo() -> bool {
    sin_irq(|| plan().activo)
}

/// PID de la tarea en curso para el diagnostico de emergencia.
///
/// A diferencia de [`actual_id`], no da nada por bueno: si la multitarea no esta
/// activa, el indice no encaja o la tabla tiene un tamano absurdo, devuelve
/// `None` en vez de arriesgarse a leer memoria que un fallo pudo corromper.
pub fn pid_actual() -> Option<u64> {
    sin_irq(|| {
        let p = plan();
        if !p.activo || p.actual == NINGUNA {
            return None;
        }
        let n = p.tareas.len();
        if p.actual >= n || n > 4096 {
            return None;
        }
        if p.tareas[p.actual].es_idle {
            return None;
        }
        Some(p.tareas[p.actual].id)
    })
}

/// Profundidad de las secciones criticas en curso.
///
/// Dentro de una seccion critica el planificador **no cambia de tarea**: como
/// solo hay una CPU, eso basta para que quien este tocando una estructura
/// compartida (la lista de ventanas, por ejemplo) no se la encuentre cambiada a
/// mitad. A diferencia de apagar las interrupciones, el reloj y los demas
/// dispositivos siguen atendiendo sus IRQ; lo unico que se aplaza es el cambio
/// de tarea. Nunca hay que dormir ni ceder dentro de una seccion critica.
///
/// El incremento es `Acquire` y el decremento `Release` deliberadamente:
/// ademas de actualizar el contador, forman la barrera del compilador que
/// mantiene los accesos normales a `SyncCell` dentro de la seccion. `Relaxed`
/// solo garantizaba atomicidad y dejaba esos accesos libres para moverse a
/// traves del limite durante la optimizacion.
static CRITICA: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Ejecuta `f` sin que el planificador cambie de tarea.
pub fn critica<F: FnOnce() -> R, R>(f: F) -> R {
    use core::sync::atomic::Ordering;
    let inicio = crate::pit::ticks();
    CRITICA.fetch_add(1, Ordering::Acquire);
    crate::metricas::critica();
    let r = f();
    crate::metricas::critica_duracion(crate::pit::ticks().wrapping_sub(inicio));
    CRITICA.fetch_sub(1, Ordering::Release);
    r
}

/// Indica si la tarea en curso esta dentro de una seccion critica.
pub fn en_critica() -> bool {
    CRITICA.load(core::sync::atomic::Ordering::Acquire) > 0
}

/// Identificador de la tarea en curso.
pub fn actual_id() -> u64 {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA || p.tareas[p.actual].es_idle {
            0
        } else {
            p.tareas[p.actual].id
        }
    })
}

/// Cuantas tareas hay ahora mismo (incluida la del nucleo).
pub fn cuantas() -> usize {
    crate::tarea::critica(|| plan().tareas.iter().filter(|t| !t.es_idle).count())
}

/// Ejecuta `f` con la tabla de descriptores de la tarea en curso.
///
/// Devuelve `None` si no hay tarea en curso (durante el arranque temprano). El
/// prestamo dura solo lo que dure `f`: la capa [`crate::fd`] lo usa para leer o
/// asignar un descriptor y suelta enseguida, porque un cambio de tarea podria
/// mover la lista de tareas y dejar el prestamo colgando.
pub fn con_fds<F: FnOnce(&mut [u32; crate::fd::NR_OPEN]) -> R, R>(f: F) -> Option<R> {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return None;
        }
        Some(f(&mut p.tareas[p.actual].fds))
    })
}

/// Ejecuta `f` con el **estado de senales** de la tarea en curso (Fase 7).
///
/// Devuelve `None` si no hay tarea en curso. Vale para consultar y cambiar en
/// una sola operacion atomica (interrupciones tapadas) pendientes, mascara y
/// acciones, que es como lo usa [`crate::senal`].
pub fn con_senales<F: FnOnce(&mut crate::senal::Senales) -> R, R>(f: F) -> Option<R> {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return None;
        }
        Some(f(&mut p.tareas[p.actual].senales))
    })
}

/// Ejecuta `f` con el **grupo de procesos** (sesion) de la tarea en curso
/// (Fase 8). Devuelve `None` si no hay tarea en curso.
pub fn con_grupo<F: FnOnce(&mut u64) -> R, R>(f: F) -> Option<R> {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return None;
        }
        Some(f(&mut p.tareas[p.actual].pgrp))
    })
}

/// **Directorio de trabajo** de la tarea en curso (Fase 9), desde el que se
/// resuelven las rutas relativas. Si no hay tarea en curso (arranque temprano)
/// devuelve la raiz del volumen; si la tarea no tiene volumen, su `cwd` es 0 y
/// las rutas absolutas siguen funcionando.
pub fn cwd_actual() -> u32 {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return crate::fs::raiz();
        }
        p.tareas[p.actual].cwd
    })
}

/// Fija el **directorio de trabajo** de la tarea en curso (lo usa `chdir` y el
/// interprete, para que los procesos que lance hereden el `cd`). Devuelve
/// `false` si no hay tarea en curso.
pub fn fijar_cwd(cwd: u32) -> bool {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return false;
        }
        p.tareas[p.actual].cwd = cwd;
        true
    })
}

/// Indica si la tarea en curso tiene alguna senal pendiente **sin bloquear**
/// (la usa el TTY para que `read` vuelva y la senal se entregue: el `EINTR` de
/// Linux, que permite desbloquear con `kill` a un proceso que espera teclado).
pub fn senal_pendiente() -> bool {
    sin_irq(|| con_senales(|s| s.hay_desbloqueada()).unwrap_or(false))
}

/// Deja pendiente `signr` en una tarea y la despierta si procede (`pause`,
/// `SIGCONT`). Comun a [`enviar_senal`] y [`enviar_senal_grupo`].
///
/// Recibe el **indice** en el vector de tareas (no `&mut Tarea`) porque
/// despertarla exige tocar las colas del planificador (Fase 3).
fn aplicar_senal(p: &mut Planificador, i: usize, signr: u32) {
    p.tareas[i].senales.encolar(signr);
    let mut despierta = false;
    // `pause` vuelve con cualquier senal.
    if p.tareas[i].senales.pause {
        p.tareas[i].senales.pause = false;
        despierta = true;
    }
    // SIGCONT reanuda a un proceso parado.
    if signr == crate::senal::SIGCONT && p.tareas[i].senales.parado {
        p.tareas[i].senales.parado = false;
        despierta = true;
    }
    if despierta {
        despertar(p, i);
    }
}

/// Deja pendiente `signr` en la tarea `pid` (el `send_sig` de Linux) y la
/// despierta si estaba en `pause` o parada por `SIGCONT`. Devuelve `false` si no
/// existe esa tarea.
pub fn enviar_senal(pid: u64, signr: u32) -> bool {
    if pid == 0 {
        return false;
    }
    sin_irq(|| {
        let p = plan();
        for i in 0..p.tareas.len() {
            if p.tareas[i].id == pid && !p.tareas[i].es_idle && p.tareas[i].estado != Estado::Muerto
            {
                aplicar_senal(p, i, signr);
                return true;
            }
        }
        false
    })
}

/// Deja pendiente `signr` en **todas** las tareas del grupo `pgrp` (el
/// `kill(-pgrp, sig)` de Linux). Lo usa el terminal para las senales de control
/// (`^C`...) contra el grupo en primer plano. Devuelve cuantas alcanzo.
pub fn enviar_senal_grupo(pgrp: u64, signr: u32) -> u32 {
    sin_irq(|| {
        let p = plan();
        let mut n = 0;
        for i in 0..p.tareas.len() {
            if !p.tareas[i].es_idle
                && p.tareas[i].pgrp == pgrp
                && p.tareas[i].estado != Estado::Muerto
            {
                aplicar_senal(p, i, signr);
                n += 1;
            }
        }
        n
    })
}

/// Marca la tarea en curso como **parada** (accion por defecto de SIGSTOP o
/// SIGTSTP): el planificador deja de darle el turno hasta un SIGCONT.
pub fn parar_actual() {
    sin_irq(|| {
        let p = plan();
        if p.actual != NINGUNA {
            p.tareas[p.actual].senales.parado = true;
            // Sin plazo: solo SIGCONT la despierta (`aplicar_senal`).
            dormir_hasta(p, p.actual, u64::MAX);
        }
    });
}

/// Bloquea la tarea en curso hasta que tenga una senal entregable (la llamada
/// `pause`). Solo vuelve cuando hay algo pendiente sin bloquear; la entrega la
/// hace [`crate::senal::comprobar`] al regresar a anillo 3.
pub fn pausar_hasta_senal() {
    loop {
        let hay = sin_irq(|| con_senales(|s| s.hay_desbloqueada()).unwrap_or(false));
        if hay {
            return;
        }
        sin_irq(|| {
            let p = plan();
            if p.actual != NINGUNA {
                p.tareas[p.actual].senales.pause = true;
                // Sin plazo: la despierta la llegada de una senal.
                dormir_hasta(p, p.actual, u64::MAX);
            }
        });
        unsafe { ceder() };
    }
}

/// Programa `SIGALRM` dentro de `segundos` segundos (la llamada `alarm`) y
/// devuelve los segundos que quedaban de la alarma anterior.
pub fn poner_alarma(segundos: u64) -> u64 {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return 0;
        }
        let ahora = pit::ticks();
        let t = &mut p.tareas[p.actual];
        let previo = if t.senales.alarma > ahora {
            // Redondea hacia arriba: los ticks van a 100 Hz.
            (t.senales.alarma - ahora + 99) / 100
        } else {
            0
        };
        t.senales.alarma = if segundos == 0 {
            0
        } else {
            ahora + segundos * 100
        };
        previo
    })
}

/// Revisa las alarmas de todas las tareas y deja `SIGALRM` pendiente en las que
/// hayan vencido. La llama el tic del reloj (IRQ0).
pub fn comprobar_alarmas(ahora: u64) {
    // La IRQ puede llegar mientras una tarea tiene prestado `Plan` dentro de
    // `critica`. En ese caso la alarma espera al siguiente tic: tocar el Vec de
    // tareas desde aqui crearia un segundo `&mut Plan` sobre el de la tarea.
    if en_critica() {
        return;
    }
    sin_irq(|| {
        let p = plan();
        for i in 0..p.tareas.len() {
            if p.tareas[i].senales.alarma != 0 && ahora >= p.tareas[i].senales.alarma {
                p.tareas[i].senales.alarma = 0;
                aplicar_senal(p, i, crate::senal::SIGALRM);
            }
        }
    });
}

/// Ejecuta `f` con el **espacio de direcciones** de la tarea en curso.
///
/// Devuelve `None` si no hay tarea en curso. Igual que [`con_fds`], el prestamo
/// dura solo lo que dure `f`: quien lo use no debe dejar que un cambio de tarea
/// ocurra dentro (las paginas que traiga bajo demanda no ceden el turno).
pub fn con_proceso<F: FnOnce(&mut Proceso) -> R, R>(f: F) -> Option<R> {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return None;
        }
        p.tareas[p.actual].proceso.as_mut().map(f)
    })
}

/// Ajusta el `brk` del proceso en curso (llamada `SYS_BRK`).
pub fn ajustar_brk(addr: u64) -> i64 {
    con_proceso(|p| p.ajustar_brk(addr as usize)).unwrap_or(-1)
}

/// Prepara la pila de una tarea nueva para que `iretq` la deje corriendo en
/// `entrada` con `arg` como primer argumento (registro `rdi`, ABI C).
fn preparar_marco(pila: &Pila, entrada: u64, arg: u64) -> u64 {
    // El marco ocupa 22 palabras (176 bytes): 15 registros + vector + error +
    // rip + cs + rflags + rsp + ss. Se coloca cerca del tope, dejando toda la
    // pila libre por debajo (la pila crece hacia abajo).
    //
    // Despues del `iretq`, `rsp` queda en `marco + 160`; la ABI quiere que al
    // entrar en una funcion sea congruente con 8 (mod 16), asi que el marco se
    // deja en una direccion congruente con 8 (mod 16).
    let marco = ((pila.tope() - 176) & !15) - 8;

    // Indices dentro de `InterruptStack`, en palabras de 8 bytes.
    const RDI: usize = 8;
    const RIP: usize = 17;
    const CS: usize = 18;
    const RFLAGS: usize = 19;
    const RSP: usize = 20;
    const SS: usize = 21;
    const PALABRAS: usize = 22;

    unsafe {
        let p = marco as *mut u64;
        for i in 0..PALABRAS {
            core::ptr::write_volatile(p.add(i), 0);
        }
        core::ptr::write_volatile(p.add(RDI), arg);
        core::ptr::write_volatile(p.add(RIP), entrada);
        core::ptr::write_volatile(p.add(CS), gdt::KERNEL_CS); // segmento de codigo del kernel
        core::ptr::write_volatile(p.add(RFLAGS), 0x202); // IF=1: la tarea ve las IRQ
                                                         // El `iretq` restaura tambien RSP y SS (el marco de interrupcion los
                                                         // trae). La pila de la tarea empieza justo por encima del marco y crece
                                                         // hacia abajo, asi que aprovecha todo el bloque.
        core::ptr::write_volatile(p.add(RSP), marco + 8 * PALABRAS as u64);
        core::ptr::write_volatile(p.add(SS), gdt::KERNEL_DS); // segmento de datos del kernel
    }
    marco
}

/// Igual que [`preparar_marco`], pero para una tarea de **anillo 3**: el `iretq`
/// deja al procesador con los segmentos de usuario y la pila de usuario.
///
/// El marco se coloca en la **pila del nucleo** de la tarea (la que el TSS
/// usara como `RSP0` mientras el programa corre), porque es donde el procesador
/// apilara el estado al entrar por una interrupcion.
fn preparar_marco_usuario(pila: &Pila, entrada: u64, arg: u64, rsp_usuario: u64) -> u64 {
    let marco = ((pila.tope() - 176) & !15) - 8;

    const RDI: usize = 8;
    const RIP: usize = 17;
    const CS: usize = 18;
    const RFLAGS: usize = 19;
    const RSP: usize = 20;
    const SS: usize = 21;
    const PALABRAS: usize = 22;

    unsafe {
        let p = marco as *mut u64;
        for i in 0..PALABRAS {
            core::ptr::write_volatile(p.add(i), 0);
        }
        core::ptr::write_volatile(p.add(RDI), arg);
        core::ptr::write_volatile(p.add(RIP), entrada);
        core::ptr::write_volatile(p.add(CS), gdt::USER_CS);
        // IF = 1 (la tarea ve el reloj) e IOPL = 0 (no puede tocar puertos).
        core::ptr::write_volatile(p.add(RFLAGS), 0x202);
        core::ptr::write_volatile(p.add(RSP), rsp_usuario);
        core::ptr::write_volatile(p.add(SS), gdt::USER_DS);
    }
    marco
}

/// Crea una tarea nueva y la deja lista para correr.
///
/// `entrada` recibe `arg` como unico argumento y **no debe volver**: al acabar
/// tiene que llamar a [`terminar`].
pub fn crear(nombre: &str, entrada: extern "C" fn(u64) -> !, arg: u64) -> u64 {
    sin_irq(|| {
        let pila = Pila::nueva();
        let rsp = preparar_marco(&pila, entrada as u64, arg);
        let p = plan();
        let id = p.siguiente_id;
        p.siguiente_id += 1;
        p.tareas.push(Tarea {
            pila,
            rsp,
            id,
            nombre: String::from(nombre),
            estado: Estado::Listo,
            es_idle: false,
            despertar: 0,
            proceso: None,
            padre: 0,
            codigo_salida: 0,
            esperando_a: 0,
            esperando_tuberia: 0,
            fds: crate::fd::fds_iniciales(),
            senales: crate::senal::Senales::nuevo(),
            pgrp: 0,
            cwd: crate::fs::raiz(),
            clase: Clase::por_defecto(false),
            sig_listo: NINGUNA,
            ant_listo: NINGUNA,
            sig_vence: NINGUNA,
            ant_vence: NINGUNA,
        });
        // La tarea nace `Listo`: hay que encolarla o nunca recibiria turno.
        let i = p.tareas.len() - 1;
        encolar_listo(p, i);
        id
    })
}

/// Crea una tarea nueva con un **padre concreto** (para pruebas y para el
/// plan Marea Fase 1). El padre puede recogerla con [`esperar`] cuando salga.
pub fn crear_con_padre(
    nombre: &str,
    entrada: extern "C" fn(u64) -> !,
    arg: u64,
    padre: u64,
) -> u64 {
    sin_irq(|| {
        let pila = Pila::nueva();
        let rsp = preparar_marco(&pila, entrada as u64, arg);
        let p = plan();
        let id = p.siguiente_id;
        p.siguiente_id += 1;
        p.tareas.push(Tarea {
            pila,
            rsp,
            id,
            nombre: String::from(nombre),
            estado: Estado::Listo,
            es_idle: false,
            despertar: 0,
            proceso: None,
            padre,
            codigo_salida: 0,
            esperando_a: 0,
            esperando_tuberia: 0,
            fds: crate::fd::fds_iniciales(),
            senales: crate::senal::Senales::nuevo(),
            pgrp: 0,
            cwd: crate::fs::raiz(),
            clase: Clase::por_defecto(false),
            sig_listo: NINGUNA,
            ant_listo: NINGUNA,
            sig_vence: NINGUNA,
            ant_vence: NINGUNA,
        });
        let i = p.tareas.len() - 1;
        encolar_listo(p, i);
        id
    })
}

/// Crea una tarea de nucleo con **padre y clase** explicitos (pruebas de la
/// Fase 3: equidad entre clases). No la usa el camino normal del nucleo.
#[allow(dead_code)]
pub fn crear_con_clase(
    nombre: &str,
    entrada: extern "C" fn(u64) -> !,
    arg: u64,
    clase: Clase,
    padre: u64,
) -> u64 {
    sin_irq(|| {
        let pila = Pila::nueva();
        let rsp = preparar_marco(&pila, entrada as u64, arg);
        let p = plan();
        let id = p.siguiente_id;
        p.siguiente_id += 1;
        p.tareas.push(Tarea {
            pila,
            rsp,
            id,
            nombre: String::from(nombre),
            estado: Estado::Listo,
            es_idle: false,
            despertar: 0,
            proceso: None,
            padre,
            codigo_salida: 0,
            esperando_a: 0,
            esperando_tuberia: 0,
            fds: crate::fd::fds_iniciales(),
            senales: crate::senal::Senales::nuevo(),
            pgrp: 0,
            cwd: crate::fs::raiz(),
            clase,
            sig_listo: NINGUNA,
            ant_listo: NINGUNA,
            sig_vence: NINGUNA,
            ant_vence: NINGUNA,
        });
        let i = p.tareas.len() - 1;
        encolar_listo(p, i);
        id
    })
}

/// Crea una tarea de **usuario** (anillo 3) a partir de un proceso ya cargado.
///
/// `entrada` es la direccion virtual del programa y `pila` la pila inicial del
/// usuario; `proc` es su espacio de direcciones (que se libera al morir). El
/// proceso **hereda el directorio de trabajo** de la tarea que lo lanza (Fase 9).
pub fn crear_usuario(nombre: &str, entrada: u64, pila: u64, proc: Proceso) -> u64 {
    sin_irq(|| {
        let pila_nucleo = Pila::nueva();
        let rsp = preparar_marco_usuario(&pila_nucleo, entrada, 0, pila);
        let p = plan();
        let id = p.siguiente_id;
        p.siguiente_id += 1;
        // El nuevo proceso arranca en el directorio de trabajo de quien lo lanza
        // (el interprete de ordenes), como el hijo de un `fork` en Linux.
        let cwd = if p.actual == NINGUNA {
            crate::fs::raiz()
        } else {
            p.tareas[p.actual].cwd
        };

        p.tareas.push(Tarea {
            pila: pila_nucleo,
            rsp,
            id,
            nombre: String::from(nombre),
            estado: Estado::Listo,
            es_idle: false,
            despertar: 0,
            proceso: Some(proc),
            padre: 0,
            codigo_salida: 0,
            esperando_a: 0,
            esperando_tuberia: 0,
            fds: crate::fd::fds_iniciales(),
            senales: crate::senal::Senales::nuevo(),
            pgrp: 0,
            cwd,
            clase: Clase::por_defecto(true),
            sig_listo: NINGUNA,
            ant_listo: NINGUNA,
            sig_vence: NINGUNA,
            ant_vence: NINGUNA,
        });
        let i = p.tareas.len() - 1;
        encolar_listo(p, i);
        id
    })
}

/// Duplica el proceso de la tarea en curso (`fork`).
///
/// Copia su espacio de direcciones (una copia **ansiosa**: de momento no hay
/// paginas compartidas ni copia-al-escribir) y su marco de interrupcion, para
/// que el hijo vuelva a anillo 3 justo donde estaba el padre pero con `rax = 0`.
/// Devuelve el PID del hijo (que es lo que vera el padre como valor de `fork`),
/// o `-1` si la tarea en curso no es un proceso de usuario.
pub fn bifurcar(marco: *const InterruptStack) -> u64 {
    let clon = sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return None;
        }
        // `clonar` necesita `&mut` porque marca tambien las paginas del padre
        // como copia-al-escribir (solo lectura + bit COW).
        p.tareas[p.actual]
            .proceso
            .as_mut()
            .and_then(|pr| pr.clonar())
    });
    match clon {
        Some(proc) => crear_hijo(marco, proc),
        None => (-1i64) as u64,
    }
}

/// Crea la tarea del hijo de `fork` a partir del marco del padre y de una copia
/// de su espacio de direcciones.
fn crear_hijo(marco: *const InterruptStack, proc: Proceso) -> u64 {
    sin_irq(|| {
        // La foto del hijo es una copia exacta de la del padre (mismos RIP, RSP
        // de usuario, RFLAGS y segmentos), salvo `rax`, que vuelve a 0.
        const RAX: usize = 14;
        const PALABRAS: usize = 22;

        let pila = Pila::nueva();
        let destino = ((pila.tope() - 176) & !15) - 8;
        unsafe {
            core::ptr::copy_nonoverlapping(marco as *const u8, destino as *mut u8, PALABRAS * 8);
            core::ptr::write_volatile((destino as *mut u64).add(RAX), 0);
        }

        let p = plan();
        let id = p.siguiente_id;
        p.siguiente_id += 1;
        let padre = if p.actual != NINGUNA {
            p.tareas[p.actual].id
        } else {
            0
        };
        // El hijo hereda los descriptores del padre: cada fichero abierto gana
        // una referencia (los dos comparten la posicion, como en Unix).
        let mut fds = crate::fd::fds_iniciales();
        if p.actual != NINGUNA {
            fds = p.tareas[p.actual].fds;
            for &d in fds.iter() {
                if d != crate::fd::LIBRE {
                    crate::fd::retener(d);
                }
            }
        }
        // El hijo hereda las acciones de senal y la mascara de bloqueo, pero no
        // las senales pendientes (como el `copy_process` de Linux).
        let senales = if p.actual != NINGUNA {
            p.tareas[p.actual].senales.heredar()
        } else {
            crate::senal::Senales::nuevo()
        };
        // El hijo hereda el grupo de procesos (sesion) del padre.
        let pgrp = if p.actual != NINGUNA {
            p.tareas[p.actual].pgrp
        } else {
            0
        };
        // Y tambien su directorio de trabajo (Fase 9).
        let cwd = if p.actual != NINGUNA {
            p.tareas[p.actual].cwd
        } else {
            crate::fs::raiz()
        };
        p.tareas.push(Tarea {
            pila,
            rsp: destino,
            id,
            nombre: String::from("hijo"),
            estado: Estado::Listo,
            es_idle: false,
            despertar: 0,
            proceso: Some(proc),
            padre,
            codigo_salida: 0,
            esperando_a: 0,
            esperando_tuberia: 0,
            fds,
            senales,
            pgrp,
            cwd,
            clase: Clase::por_defecto(true),
            sig_listo: NINGUNA,
            ant_listo: NINGUNA,
            sig_vence: NINGUNA,
            ant_vence: NINGUNA,
        });
        let i = p.tareas.len() - 1;
        encolar_listo(p, i);
        id
    })
}

/// Reemplaza la imagen de la **tarea en curso** (`execve`).
///
/// Conserva la identidad del proceso (`id`, `padre`, `esperando_a`) y su espacio
/// de direcciones: la PML4 sigue siendo la misma (y la misma CR3 que ya esta
/// cargada en el procesador), solo cambia lo que hay mapeado en su ventana. El
/// ejecutable `prog` lo ha leido antes el nucleo desde el sistema de ficheros;
/// `argv` y `envp` son copias en memoria del nucleo de lo que el proceso viejo
/// dejo en su pila (ya se pueden leer aunque su imagen se vaya a liberar).
///
/// El marco de `int 0x80` se reescribe en el sitio para que el `iretq` final
/// devuelva a anillo 3 en la **entrada del programa nuevo**, con la pila nueva
/// (que lleva `argc`, `argv[]`, `envp[]` y las cadenas). Por eso `execve` **no
/// vuelve** al programa que lo llamo.
///
/// Devuelve `0` si fue bien, o `-1` si la tarea en curso no es un proceso de
/// usuario, el ejecutable no cabe en la ventana o los argumentos no caben en la
/// pila.
pub fn ejecutar(
    marco: *mut InterruptStack,
    prog: &[u8],
    argv: &[Vec<u8>],
    envp: &[Vec<u8>],
) -> i64 {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return -1;
        }
        // Si los argumentos no caben en la pila de usuario, se falla **antes**
        // de tocar la imagen del proceso: asi el programa que llamo sigue vivo.
        if crate::proceso::tamano_pila(argv, envp) > crate::pmm::FRAME_SIZE / 2 {
            return -1;
        }
        let t = &mut p.tareas[p.actual];
        let proc = match t.proceso.as_mut() {
            Some(pr) => pr,
            None => return -1,
        };
        match proc.ejecutar(prog) {
            Some((entrada, tope)) => {
                // Un `execve` con exito deja las senales como al principio: sin
                // pendientes y con los manejadores a la accion por defecto (los
                // \"ignorar\" se conservan, como en Linux).
                t.senales.reiniciar();
                // El nucleo escribe el bloque de argumentos en la pila del
                // usuario **en anillo 0**: si tropezara con una pagina sin traer
                // no habria un #PF "de usuario" que reanudar. Se aseguran las
                // paginas del rango que ocupara el bloque (que `tamano_pila`
                // acota) antes de escribir nada.
                let tam = crate::proceso::tamano_pila(argv, envp);
                let mut pag = tope.saturating_sub(tam) & !(crate::pmm::FRAME_SIZE - 1);
                while pag < tope {
                    if !proc.poblar_escritura(pag) {
                        return -1;
                    }
                    pag += crate::pmm::FRAME_SIZE;
                }
                // Bloque de argumentos en el tope de la pila nueva.
                let rsp = crate::proceso::montar_pila(tope, argv, envp).unwrap_or(tope);
                // El `iretq` tomara de aqui el destino en anillo 3. Los demas
                // registros del marco son de la llamada anterior: se limpian
                // para que el programa nuevo no herede nada del viejo.
                unsafe {
                    const RAX: usize = 14;
                    let m = marco as *mut u64;
                    for i in 0..=RAX {
                        core::ptr::write_volatile(m.add(i), 0);
                    }
                    (*marco).rip = entrada as u64;
                    (*marco).rsp = rsp as u64;
                }
                0
            }
            None => -1,
        }
    })
}

/// Termina la tarea en curso con un codigo de salida (llamada `exit`).
///
/// Si el proceso tiene padre, queda como **zombi** hasta que este lo recoja con
/// `waitpid`; si no lo tiene, se marca muerto para que [`recolectar`] lo libere.
/// En ambos casos despierta al padre si estaba bloqueado esperandolo.
pub fn salir(codigo: i64) {
    sin_irq(|| {
        let p = plan();
        if p.actual == NINGUNA {
            return;
        }
        let id = p.tareas[p.actual].id;
        let padre = p.tareas[p.actual].padre;

        p.tareas[p.actual].codigo_salida = codigo;
        // Si el padre existe en la tabla, queda como zombi (el padre lo
        // recoge con `waitpid`); si no, se marca muerto para que
        // `recolectar` lo libere. Se comprueba la existencia real del padre
        // (no solo `padre != 0`) porque la tarea del nucleo tiene id 0.
        let padre_existe = p.tareas.iter().any(|t| t.id == padre);
        p.tareas[p.actual].estado = if padre_existe {
            Estado::Zombi
        } else {
            Estado::Muerto
        };
        // Al terminar se cierran los ficheros abiertos (como `do_exit` en Linux).
        let mut fds = p.tareas[p.actual].fds;
        crate::fd::cerrar_todos(&mut fds);
        p.tareas[p.actual].fds = fds;
        for i in 0..p.tareas.len() {
            if p.tareas[i].estado == Estado::Durmiendo
                && (p.tareas[i].esperando_a == id || p.tareas[i].esperando_a == ESPERA_CUALQUIERA)
            {
                p.tareas[i].esperando_a = 0;
                despertar(p, i);
            }
            // El padre recibe `SIGCHLD` (Fase 7). Si no tiene manejador, la
            // accion por defecto es ignorarlo: no cambia nada.
            if p.tareas[i].id == padre && padre_existe {
                p.tareas[i].senales.encolar(crate::senal::SIGCHLD);
            }
        }
    });
}

/// Espera a que termine un hijo y lo recoge (llamada `waitpid`).
///
/// `pid` es el hijo concreto, o <= 0 para "cualquier hijo". Devuelve
/// `(pid, codigo)` del hijo recogido, o `None` si no hay hijos que encajen
/// (el nucleo de `waitpid` lo traduce a `-1`).
///
/// Bloquea y cede el turno mientras no haya un hijo terminado; el hijo que
/// termina despierta al padre (ver [`salir`]).
pub fn esperar(pid: i64) -> Option<(u64, i64)> {
    loop {
        if let Some(recogido) = sin_irq(|| recoger(pid)) {
            return Some(recogido);
        }
        if sin_irq(|| sin_hijos(pid)) {
            return None;
        }
        sin_irq(|| {
            let p = plan();
            if p.actual != NINGUNA {
                p.tareas[p.actual].esperando_a = if pid > 0 {
                    pid as u64
                } else {
                    ESPERA_CUALQUIERA
                };
                // Nunca despierta por reloj: solo la salida de un hijo lo hace.
                dormir_hasta(p, p.actual, u64::MAX);
            }
        });
        unsafe { ceder() };
    }
}

/// Busca un hijo terminado que encaje con `pid` y lo saca de la lista (lo que
/// libera su espacio de direcciones). Devuelve `(pid, codigo)`.
fn recoger(pid: i64) -> Option<(u64, i64)> {
    let p = plan();
    if p.actual == NINGUNA {
        return None;
    }
    let padre = p.tareas[p.actual].id;
    let mut i = 0;
    while i < p.tareas.len() {
        let t = &p.tareas[i];
        if t.padre == padre && t.estado == Estado::Zombi && (pid <= 0 || t.id == pid as u64) {
            let hijo = t.id;
            let codigo = t.codigo_salida;

            // `quitar_tarea` reajusta colas, vencimientos e indices tras el
            // `remove` (incluido `p.actual`), que es justo lo que aqui se hacia
            // a mano antes de la Fase 3.
            let _ = quitar_tarea(p, i);
            return Some((hijo, codigo));
        }
        i += 1;
    }
    None
}

/// Indica si la tarea en curso no tiene ningun hijo que `pid` pudiera recoger.
fn sin_hijos(pid: i64) -> bool {
    let p = plan();
    if p.actual == NINGUNA {
        return true;
    }
    let padre = p.tareas[p.actual].id;
    !p.tareas
        .iter()
        .any(|t| t.padre == padre && t.estado != Estado::Muerto && (pid <= 0 || t.id == pid as u64))
}

/// Elige la siguiente tarea y devuelve el marco desde el que debe continuar.
///
/// Es lo que llama `isr_dispatch` en cada tic del reloj (y en cada `int 0x30`).
/// Lo que devuelve puede ser el marco de **otra** tarea: por eso
/// `interrupts.s` reajusta `rsp` con el valor de retorno.
pub fn planificar(marco: *mut InterruptStack) -> *mut InterruptStack {
    // Dentro de una seccion critica no se toca `PLAN` en absoluto. La tarea
    // interrumpida puede tener un `&mut Plan` vivo; incluso actualizar su `rsp`
    // aqui (o incluso leer `activo`) crearia un alias mutable desde la IRQ y
    // podria corromper la tabla de tareas y otros Vec globales. No se pierde el
    // marco: como no hubo cambio de tarea, se guardara en el siguiente punto de
    // planificacion una vez que la seccion critica haya terminado.
    if en_critica() {
        return marco;
    }
    let p = plan();
    if !p.activo {
        return marco;
    }

    // La primera vez, la tarea en curso (el nucleo) pasa a ser una tarea mas.
    // Su pila es la de arranque (ver `start.s`), que se reserva en el gestor de
    // marcos: es la que usan para siempre el interprete de ordenes y el
    // escritorio.
    if p.actual == NINGUNA {
        p.tareas.push(Tarea {
            pila: Pila::vacia(),
            rsp: marco as u64,
            id: 0,
            nombre: String::from("sistema"),
            estado: Estado::Ejecutando,
            es_idle: false,
            despertar: 0,
            proceso: None,
            padre: 0,
            codigo_salida: 0,
            esperando_a: 0,
            esperando_tuberia: 0,
            fds: crate::fd::fds_iniciales(),
            senales: crate::senal::Senales::nuevo(),
            pgrp: 0,
            cwd: crate::fs::raiz(),
            clase: Clase::por_defecto(false),
            sig_listo: NINGUNA,
            ant_listo: NINGUNA,
            sig_vence: NINGUNA,
            ant_vence: NINGUNA,
        });
        p.actual = p.tareas.len() - 1;
        return marco;
    }

    // Guarda la foto de la tarea que deja el turno.
    {
        let actual = &mut p.tareas[p.actual];
        actual.rsp = marco as u64;
        if actual.estado == Estado::Ejecutando {
            actual.estado = Estado::Listo;
        }
    }
    // Si sigue lista, vuelve al final de su cola: es el turno circular.
    let saliente = p.actual;
    if p.tareas[saliente].estado == Estado::Listo {
        encolar_listo(p, saliente);
    }

    // Despierta a quien ya le toque: solo los vencimientos de reloj, sacados de
    // la lista ordenada por plazo. Antes esto recorria **todas** las tareas en
    // cada tic (Fase 3 del plan: coste O(vencidos) en vez de O(tareas)).
    despertar_vencidos(p, pit::ticks());

    // Elige el siguiente turno con equidad entre clases.
    let mut elegida = elegir_siguiente(p);

    if elegida == NINGUNA {
        // No revivir a la tarea actual: puede estar dormida o ser un zombi que
        // acaba de salir. En ambos casos se pasa al contexto idle hasta que una
        // IRQ o una señal haga que vuelva a estar lista.
        elegida = p.tareas.iter().position(|t| t.es_idle).unwrap_or(NINGUNA);
        if elegida == NINGUNA {
            // Invariante defensiva para una inicialización incompleta. No se
            // convierte una tarea bloqueada en ejecutable para ocultar el fallo.
            return marco;
        }
    }

    p.tareas[elegida].estado = Estado::Ejecutando;
    p.actual = elegida;

    crate::metricas::cambio_tarea(p.tareas[elegida].es_idle);

    // Si la tarea elegida es de anillo 3, hay que dejar el TSS apuntando a su
    // pila de nucleo (es la que el procesador cargara al entrar por IRQ o por
    // `int 0x80`) y activar su espacio de direcciones.
    let t = &p.tareas[elegida];
    let tope = t.pila.tope();
    if tope != 0 {
        gdt::poner_rsp0(tope);
    }
    let pml4 = t.proceso.as_ref().map_or(paging::PML4_NUCLEO, |x| x.pml4);
    paging::activar(pml4);

    t.rsp as *mut InterruptStack
}

/// Termina **solo** la tarea en curso por una excepcion de anillo 3 y devuelve
/// el marco desde el que debe continuar la tarea que tome el relevo.
///
/// Lo usa [`crate::idt`]: una excepcion cometida por un proceso de usuario no
/// debe tumbar el sistema, solo al proceso. Reutiliza [`salir`] (que cierra sus
/// descriptores, marca el estado y despierta al padre) y luego pide el turno.
///
/// # Seguridad
/// Espera el mismo marco que [`planificar`]: el de la interrupcion en curso.
pub fn abortar(marco: *mut InterruptStack, codigo: i64) -> *mut InterruptStack {
    salir(codigo);
    planificar(marco)
}

/// Deja el turno voluntariamente (vector 48). El planificador decidira si
/// vuelve a esta tarea o pasa a otra.
///
/// # Seguridad
/// Es lo mismo que una interrupcion: el estado de los registros queda tal cual
/// estaba cuando el control regrese.
pub unsafe fn ceder() {
    // Dentro de una seccion critica el planificador no cambia de tarea, asi que
    // ceder no serviria de nada y, en un bucle de espera, colgaria el sistema.
    if en_critica() {
        return;
    }
    asm!("int 0x30");
}

/// Duerme la tarea actual al menos `ms` milisegundos, dejando correr a las
/// demas mientras tanto.
pub fn dormir(ms: u64) {
    if ms == 0 {
        return;
    }
    // Nunca se duerme dentro de una seccion critica: el cambio de tarea esta
    // bloqueado y la espera solo retendria el planificador.
    if en_critica() {
        return;
    }
    let despertar = pit::ticks().saturating_add((ms + 9) / 10);
    while pit::ticks() < despertar {
        sin_irq(|| {
            let p = plan();
            if p.actual != NINGUNA {
                dormir_hasta(p, p.actual, despertar);
            }
        });
        unsafe { ceder() };
    }
}

/// Duerme la tarea actual hasta que cambie la tuberia `id` (Fase 6).
///
/// Es el `sleep_on(&inode->i_wait)` de Linux 0.01, pero sin cola de espera: la
/// tarea se marca dormida apuntando a la tuberia, y [`despertar_tuberia`] la
/// volvera a poner lista cuando haya datos, hueco o se cierre un extremo.
/// Nunca despierta por reloj (`despertar = u64::MAX`): hay que llamarla desde
/// dentro de la misma seccion critica que comprobo que habia que esperar, o se
/// perdera el aviso.
pub fn bloquear_en_tuberia(id: u32) {
    sin_irq(|| {
        let p = plan();
        if p.actual != NINGUNA {
            p.tareas[p.actual].esperando_tuberia = id;
            // Sin plazo: la despierta `despertar_tuberia`.
            dormir_hasta(p, p.actual, u64::MAX);
        }
    });
}

/// Despierta a las tareas que esperan a la tuberia `id` (el `wake_up` de Linux).
///
/// Se llama tras una lectura, una escritura o el cierre de un extremo. Despierta
/// a **todas** las que esperan a esa tuberia (lectores y escritores): cada una
/// volvera a mirar si su condicion se cumple y, si no, se dormira otra vez.
pub fn despertar_tuberia(id: u32) {
    if id == 0 {
        return;
    }
    sin_irq(|| {
        let p = plan();
        for i in 0..p.tareas.len() {
            if p.tareas[i].esperando_tuberia == id {
                p.tareas[i].esperando_tuberia = 0;
                // Idempotente: si ya estaba lista, no la encola dos veces.
                despertar(p, i);
            }
        }
    });
}

/// Termina la tarea actual. No vuelve.
///
/// La usaba la tarea que lanzaba un programa `.rok` desde el escritorio; hoy no
/// tiene quien la llame.
#[allow(dead_code)]
pub fn terminar() -> ! {
    sin_irq(|| {
        let p = plan();
        if p.actual != NINGUNA {
            p.tareas[p.actual].estado = Estado::Muerto;
        }
    });
    // Deja el turno: el planificador no volvera a elegirla.
    loop {
        unsafe { ceder() };
    }
}

/// Marca una tarea como muerta: el planificador deja de darle el turno y
/// `recolectar` acabara liberando su pila.
///
/// La usa, por ejemplo, el boton de cerrar de la ventana de un programa. Devuelve
/// `true` si existia. La tarea del nucleo (PID 0) no se puede matar.
pub fn matar(id: u64) -> bool {
    if id == 0 || id == u64::MAX {
        return false;
    }
    sin_irq(|| {
        let p = plan();
        for t in p.tareas.iter_mut() {
            if t.id == id && t.estado != Estado::Muerto {
                t.estado = Estado::Muerto;
                return true;
            }
        }
        false
    })
}

/// Saca de la lista las tareas terminadas y libera sus pilas.
///
/// La llama el bucle del escritorio de vez en cuando. Libera:
/// - las tareas marcadas [`Estado::Muerto`] (programas del nucleo terminados);
/// - los **zombis huerfanos** (procesos terminados cuyo padre ya no esta).
///
/// Un zombi con el padre vivo se queda: es el padre quien lo recoge con
/// `waitpid`. Al liberar una tarea, sus hijos (si los tenia) pasan a no tener
/// padre. Nunca libera la tarea en curso.
pub fn recolectar() {
    sin_irq(|| {
        let p = plan();

        let actual = p.actual;
        let mut i = 0;
        while i < p.tareas.len() {
            let libera = {
                let t = &p.tareas[i];
                match t.estado {
                    Estado::Muerto => true,
                    Estado::Zombi => !p.tareas.iter().any(|o| o.id == t.padre),
                    _ => false,
                }
            };
            if libera && i != actual {
                let id = p.tareas[i].id;

                // Cierra los descriptores que le quedaran (tareas del nucleo que
                // mueren sin pasar por `exit`). Para un proceso ya cerrado en
                // `salir` es inofensivo: su tabla esta toda a `LIBRE`.
                let mut fds = p.tareas[i].fds;
                crate::fd::cerrar_todos(&mut fds);
                p.tareas[i].fds = fds;

                for t in p.tareas.iter_mut() {
                    if t.padre == id {
                        t.padre = 0;
                    }
                }
                let _ = quitar_tarea(p, i);
                // `quitar_tarea` ya dejo `p.actual` ajustado; no hay que
                // corregirlo aqui (antes se hacia a mano).
            } else {
                i += 1;
            }
        }
    });
}

/// Escribe en pantalla la lista de tareas (orden `ps`).
pub fn imprimir() {
    let (filas, actual) = sin_irq(|| {
        let p = plan();
        let filas: Vec<(u64, String, &'static str, &'static str)> = p
            .tareas
            .iter()
            .filter(|t| !t.es_idle)
            .map(|t| {
                let estado = match t.estado {
                    Estado::Listo => "listo",
                    Estado::Ejecutando => "en curso",
                    Estado::Durmiendo => "dormido",
                    Estado::Muerto => "terminado",
                    Estado::Zombi => "zombi",
                };
                (t.id, t.nombre.clone(), estado, t.clase.etiqueta())
            })
            .collect();
        let actual = p.tareas.get(p.actual).filter(|t| !t.es_idle).map(|t| t.id);
        (filas, actual)
    });
    crate::println!(
        "  PID  ESTADO     CLASE        NOMBRE   ({} tareas)",
        cuantas()
    );
    for (id, nombre, estado, clase) in filas.iter() {
        let marca = if Some(*id) == actual { "*" } else { " " };
        crate::println!(
            "  {}{:>3}  {:<9}  {:<11}  {}",
            marca,
            id,
            estado,
            clase,
            nombre
        );
    }
}

/// --- Depuracion temporal (quitar) ---
pub fn volcar_pilas() {
    let datos: Vec<(usize, u64, u64, bool, bool, u64, u64)> = sin_irq(|| {
        let p = plan();
        p.tareas
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let rip = if t.rsp != 0 {
                    unsafe { core::ptr::read_volatile((t.rsp as *const u64).add(17)) }
                } else {
                    0
                };
                (
                    i,
                    t.id,
                    t.rsp,
                    t.es_idle,
                    t.estado == Estado::Ejecutando,
                    rip,
                    t.rsp,
                )
            })
            .collect()
    });
    let actual = sin_irq(|| plan().actual);
    for (i, id, _, idle, curso, rip, _) in datos {
        let marca = if curso { "*" } else { " " };
        crate::println!(
            "  t{} id={} idle={} {}rip={:#x}",
            i,
            id,
            idle,
            marca,
            rip
        );
    }
    crate::println!("  actual={}", actual);
}

/// Aparta el procesador hasta la proxima interrupcion (sin dejar de ser una
/// tarea: otra podra correr cuando llegue el tic).
///
/// Lo usaba el bucle de espera del escritorio grafico antiguo.
#[allow(dead_code)]
pub fn holgar() {
    unsafe { cpu::hlt() };
}

// ---------------------------------------------------------------------------
// Fase 1 del plan Marea: invariantes del planificador
// ---------------------------------------------------------------------------

/// Marca `len` posiciones como `false` (evita depender del macro `vec!`, que no
/// esta en el preludio de `no_std`).
fn marcas_vacias(len: usize) -> Vec<bool> {
    let mut v = Vec::with_capacity(len);
    for _ in 0..len {
        v.push(false);
    }
    v
}

/// Comprueba los invariantes de estado del planificador (Fase 1 Marea).
///
/// Transiciones autorizadas:
///
/// ```text
///   Listo       -> Ejecutando   (planificar lo elige)
///   Ejecutando  -> Listo        (planificar guarda el turno)
///   Ejecutando  -> Durmiendo    (dormir, pause, esperar, tuberia, parar)
///   Ejecutando  -> Zombi/Muerto (salir/abortar)
///   Durmiendo   -> Listo        (despertar por reloj, senal, tuberia, hijo)
///   Zombi       -> (recolectado) (padre hace waitpid o es huerfano)
///   Muerto      -> (recolectado) (recolectar lo libera)
/// ```
///
/// Invariantes verificados:
///
/// 1. A lo sumo una tarea esta `Ejecutando` (mononucleo).
/// 2. Si hay una `Ejecutando`, es la que apunta `actual`.
/// 3. Ningun `Zombi` tiene `despertar` valido para el reloj (solo se saca con
///    `waitpid`, nunca por timeout).
/// 4. La tarea idle nunca esta `Zombi` ni `Muerto` (existe siempre).
/// 5. No hay IDs duplicados.
/// 6. El indice `actual` es valido.
/// 7. (Fase 3) Solo las tareas `Listo` estan en una cola de listos, toda tarea
///    `Listo` no idle esta en su cola, y una tarea no esta en dos colas.
/// 8. (Fase 3) La lista de vencimientos concuerda con las tareas: solo contiene
///    `Durmiendo` con plazo finito, esta ordenada por `despertar`, sin ciclos y
///    sin indices fuera de rango.
///
/// Devuelve `Ok(())` o un mensaje de error estatico.
pub fn comprobar_invariantes() -> Result<(), &'static str> {
    sin_irq(|| {
        let p = plan();
        let mut ejecutando = 0;
        let mut ids = Vec::new();

        for (i, t) in p.tareas.iter().enumerate() {
            // 1 y 2: a lo sumo una Ejecutando y es la actual.
            if t.estado == Estado::Ejecutando {
                ejecutando += 1;
                if i != p.actual {
                    return Err("hay una tarea Ejecutando que no es la actual");
                }
            }

            // 5: sin IDs duplicados.
            if ids.contains(&t.id) {
                return Err("dos tareas comparten el mismo ID");
            }
            ids.push(t.id);

            // 4: idle nunca en estado terminal.
            if t.es_idle && (t.estado == Estado::Zombi || t.estado == Estado::Muerto) {
                return Err("la tarea idle esta en estado terminal");
            }

            // 3: un zombi no debe ser despertado por el reloj.
            if t.estado == Estado::Zombi && t.despertar != u64::MAX && t.despertar != 0 {
                return Err("un zombi tiene despertar por reloj activo");
            }

            // Un zombi o muerto no debe tener tuberia pendiente.
            if (t.estado == Estado::Zombi || t.estado == Estado::Muerto) && t.esperando_tuberia != 0
            {
                return Err("una tarea terminal espera a una tuberia");
            }
        }

        if ejecutando > 1 {
            return Err("hay mas de una tarea Ejecutando");
        }

        // 6: indice actual valido.
        if p.actual != NINGUNA && p.actual >= p.tareas.len() {
            return Err("el indice actual apunta fuera de la lista de tareas");
        }

        // 7 (Fase 3): coherencia entre `Listo` y las colas de listos.
        // Primero, que ninguna tarea aparezca en las dos clases a la vez ni
        // repetida dentro de una cola (una cola con un indice fuera de rango
        // tambien seria catastrófica).
        let mut visto_en_cola = marcas_vacias(p.tareas.len());
        for c in 0..Clase::CUANTAS {
            let mut cur = p.cabeza_listo[c];
            let mut pasos = 0;
            let mut ant = NINGUNA;
            while cur != NINGUNA {
                if cur >= p.tareas.len() {
                    return Err("una cola de listos apunta fuera de la lista de tareas");
                }
                if visto_en_cola[cur] {
                    return Err("una tarea esta en dos colas de listos o repetida en una");
                }
                visto_en_cola[cur] = true;
                if p.tareas[cur].ant_listo != ant {
                    return Err("los enlaces hacia atras de una cola no concuerdan");
                }
                ant = cur;
                cur = p.tareas[cur].sig_listo;
                pasos += 1;
                if pasos > p.tareas.len() {
                    return Err("ciclo en una cola de listos");
                }
            }
            if p.cola_listo[c] != ant {
                return Err("la cola declarada no es el ultimo elemento de la cola");
            }
        }
        for (i, t) in p.tareas.iter().enumerate() {
            if t.es_idle {
                // `idle` nunca entra en las colas.
                if visto_en_cola[i] {
                    return Err("la tarea idle esta en una cola de listos");
                }
                continue;
            }
            if t.estado == Estado::Listo {
                if !visto_en_cola[i] {
                    return Err("una tarea Listo no esta en ninguna cola");
                }
            } else if visto_en_cola[i] {
                return Err("una tarea no Listo esta en una cola de listos");
            }
        }

        // 8 (Fase 3): coherencia de la lista de vencimientos.
        let mut visto_vence = marcas_vacias(p.tareas.len());
        let mut cur = p.cabeza_vence;
        let mut ant = NINGUNA;
        let mut clave_ant = 0u64;
        let mut pasos = 0;
        while cur != NINGUNA {
            if cur >= p.tareas.len() {
                return Err("la lista de vencimientos apunta fuera de la lista de tareas");
            }
            if visto_vence[cur] {
                return Err("ciclo en la lista de vencimientos");
            }
            visto_vence[cur] = true;
            if p.tareas[cur].ant_vence != ant {
                return Err("los enlaces hacia atras de los vencimientos no concuerdan");
            }
            if p.tareas[cur].estado != Estado::Durmiendo {
                return Err("una tarea que no esta Durmiendo aparece en los vencimientos");
            }
            if p.tareas[cur].despertar == u64::MAX || p.tareas[cur].despertar == 0 {
                return Err("una tarea con vencimiento no tiene plazo finito");
            }
            if ant != NINGUNA && p.tareas[cur].despertar < clave_ant {
                return Err("la lista de vencimientos no esta ordenada");
            }
            clave_ant = p.tareas[cur].despertar;
            ant = cur;
            cur = p.tareas[cur].sig_vence;
            pasos += 1;
            if pasos > p.tareas.len() {
                return Err("ciclo en la lista de vencimientos");
            }
        }
        // Toda tarea Durmiendo con plazo finito debe estar en la lista.
        for (i, t) in p.tareas.iter().enumerate() {
            if t.estado == Estado::Durmiendo
                && t.despertar != u64::MAX
                && t.despertar != 0
                && !visto_vence[i]
            {
                return Err("una tarea Durmiendo con plazo no esta en los vencimientos");
            }
        }

        Ok(())
    })
}

/// Estado observado de una tarea para las pruebas (snapshot inmutable).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum EstadoTest {
    /// El scheduler la tiene preparada.
    Listo,
    /// Es la que corre.
    Ejecutando,
    /// Bloqueada (dormida, pause, senal parada, tuberia o waitpid).
    Durmiendo,
    /// Zombi esperando a que el padre la recoja.
    Zombi,
    /// Muerta pendiente de recoleccion.
    Muerto,
}

impl From<Estado> for EstadoTest {
    fn from(e: Estado) -> Self {
        match e {
            Estado::Listo => EstadoTest::Listo,
            Estado::Ejecutando => EstadoTest::Ejecutando,
            Estado::Durmiendo => EstadoTest::Durmiendo,
            Estado::Zombi => EstadoTest::Zombi,
            Estado::Muerto => EstadoTest::Muerto,
        }
    }
}

/// Devuelve el estado actual de la tarea `pid` (para pruebas). `None` si no
/// existe.
#[allow(dead_code)]
pub fn estado_de(pid: u64) -> Option<EstadoTest> {
    sin_irq(|| {
        let p = plan();
        p.tareas
            .iter()
            .find(|t| t.id == pid)
            .map(|t| t.estado.into())
    })
}

/// Pruebas de invariantes del planificador (Fase 1 Marea).
///
/// Cada prueba verifica una transicion de estado y comprueba
/// [`comprobar_invariantes`] al final. Se ejecuta con el comando `test_plan`
/// del shell, o automaticamente al arranque. Devuelve `Ok(n)` con el numero de
/// pruebas pasadas, o `Err(mensaje)` en la primera que falle.
pub fn test_plan() -> Result<usize, &'static str> {
    let mut pasadas = 0;
    let mi_id = actual_id();

    // --- Prueba 1: invariantes en reposo -----------------------------------
    comprobar_invariantes()?;
    pasadas += 1;

    // --- Prueba 2: una tarea que duerme y despierta por reloj ---------------
    // `dormir` debe poner la tarea en Durmiendo y el reloj debe despertarla.
    let id2 = crear_con_padre("test_dormir", test_dormir_fn, 0, mi_id);
    // Espera a que termine (se durmio 50 ms y luego salio).
    esperar_hijo(id2)?;
    comprobar_invariantes()?;
    pasadas += 1;

    // --- Prueba 3: una tarea que sale y queda como zombi --------------------
    // `salir` con padre != 0 produce Zombi, que `esperar` recoge.
    let id3 = crear_con_padre("test_zombi", test_zombi_fn, 0, mi_id);
    let resultado = esperar_hijo(id3)?;
    if resultado.1 != 42 {
        return Err("el zombi no devolvio el codigo de salida esperado");
    }
    comprobar_invariantes()?;
    pasadas += 1;

    // --- Prueba 4: tarea bloqueada en tuberia se despierta ------------------
    // El hijo se bloquea en la tuberia 777; el padre la despierta tras 200 ms
    // y el hijo sale con codigo 0.
    let id4 = crear_con_padre("test_tuberia", test_tuberia_fn, 777, mi_id);
    // Espera ~200 ms a que el hijo llegue a `bloquear_en_tuberia`.
    dormir(200);
    // Despierta a todas las tareas que esperan la tuberia 777.
    despertar_tuberia(777);
    esperar_hijo(id4)?;
    comprobar_invariantes()?;
    pasadas += 1;

    // --- Prueba 5: pause + SIGALRM ------------------------------------------
    // La tarea se pone en pause, el padre le envia SIGALRM y la senal la
    // despierta; la tarea sale con codigo 0.
    let id5 = crear_con_padre("test_pause", test_pause_fn, 0, mi_id);
    // Espera ~200 ms (20 ticks) a que el hijo llegue a `pause` y se bloquee.
    dormir(200);
    // Envia SIGALRM: `aplicar_senal` despierta a la tarea que esta en pause.
    if !enviar_senal(id5, crate::senal::SIGALRM) {
        return Err("no se pudo enviar SIGALRM al hijo en pause");
    }
    esperar_hijo(id5)?;
    comprobar_invariantes()?;
    pasadas += 1;

    // --- Prueba 6 (Fase 3): varias tareas normales duermen a la vez ----------
    // Tres tareas con plazos distintos se duermen a la vez: la lista de
    // vencimientos debe despertarlas **en orden** y todas deben terminar. Las
    // tareas creadas con `crear_con_padre` son de clase interactiva; esta prueba
    // cubre el camino de la lista ordenada con varios nodos.
    let ida = crear_con_padre("test_cola_a", test_dormir_ms_fn, 30, mi_id);
    let idb = crear_con_padre("test_cola_b", test_dormir_ms_fn, 10, mi_id);
    let idc = crear_con_padre("test_cola_c", test_dormir_ms_fn, 20, mi_id);
    esperar_hijo(ida)?;
    esperar_hijo(idb)?;
    esperar_hijo(idc)?;
    comprobar_invariantes()?;
    pasadas += 1;

    // --- Prueba 7 (Fase 3): equidad entre clases -----------------------------
    // Ninguna tarea lista debe quedarse sin turno: dos tareas normales y una
    // interactiva compitiendo, todas deben llegar a terminar (sin inanicion).
    let normales = crear_con_clase("test_norm_a", test_contar_fn, 0, Clase::Normal, mi_id);
    let normal_b = crear_con_clase("test_norm_b", test_contar_fn, 0, Clase::Normal, mi_id);
    let inter = crear_con_clase("test_inter", test_contar_fn, 0, Clase::Interactiva, mi_id);
    esperar_hijo(normales)?;
    esperar_hijo(normal_b)?;
    esperar_hijo(inter)?;
    comprobar_invariantes()?;
    pasadas += 1;

    // --- Prueba 8 (Fase 3): una tarea parada no queda en ninguna cola ---------
    // Se para la tarea de prueba (Durmiendo sin plazo) y se comprueba que no
    // aparece en las colas de listos ni en los vencimientos.
    let id8 = crear_con_padre("test_parada", test_parada_fn, 0, mi_id);
    dormir(100);
    if !enviar_senal(id8, crate::senal::SIGCONT) {
        return Err("no se pudo reanudar la tarea parada");
    }
    esperar_hijo(id8)?;
    comprobar_invariantes()?;
    pasadas += 1;

    Ok(pasadas)
}

/// Tarea de prueba: duerme `arg` milisegundos y sale con codigo 0.
extern "C" fn test_dormir_ms_fn(arg: u64) -> ! {
    dormir(arg);
    salir(0);
    loop {
        unsafe { ceder() };
    }
}

/// Tarea de prueba: da unos cuantos turnos y sale (para la prueba de equidad).
extern "C" fn test_contar_fn(_arg: u64) -> ! {
    let mut i = 0;
    while i < 20 {
        unsafe { ceder() };
        i += 1;
    }
    salir(0);
    loop {
        unsafe { ceder() };
    }
}

/// Tarea de prueba: se para a si misma y espera SIGCONT (`test_parada_fn`).
extern "C" fn test_parada_fn(_arg: u64) -> ! {
    parar_actual();
    // Al reanudar con SIGCONT, sale.
    salir(0);
    loop {
        unsafe { ceder() };
    }
}

/// Espera a que termine la tarea `pid` y la recoge. Usa [`esperar`] que es la
/// misma ruta que `waitpid`.
fn esperar_hijo(pid: u64) -> Result<(u64, i64), &'static str> {
    match esperar(pid as i64) {
        Some(r) => Ok(r),
        None => Err("la tarea termino pero no se pudo recoger"),
    }
}

/// Tarea de prueba: se duerme 50 ms y sale con codigo 0.
extern "C" fn test_dormir_fn(_arg: u64) -> ! {
    dormir(50);
    salir(0);
    loop {
        unsafe { ceder() };
    }
}

/// Tarea de prueba: sale inmediatamente con codigo 42.
extern "C" fn test_zombi_fn(_arg: u64) -> ! {
    salir(42);
    loop {
        unsafe { ceder() };
    }
}

/// Tarea de prueba: se bloquea en una tuberia y espera a que la despierten.
extern "C" fn test_tuberia_fn(arg: u64) -> ! {
    let tuberia = arg as u32;
    // Publica la espera y cede: si nadie la despierta, queda bloqueada.
    bloquear_en_tuberia(tuberia);
    unsafe { ceder() };
    // Despierta: sale con codigo 0.
    salir(0);
    loop {
        unsafe { ceder() };
    }
}

/// Tarea de prueba: se pone en `pause` y espera a que una senal lo despierte.
/// El padre le envia SIGALRM tras crearla.
extern "C" fn test_pause_fn(_arg: u64) -> ! {
    pausar_hasta_senal();
    salir(0);
    loop {
        unsafe { ceder() };
    }
}

/// Imprime el resultado de las pruebas del planificador (orden `test_plan` del
/// shell).
pub fn test_plan_shell() {
    crate::println!("Probando invariantes del planificador (Fase 1 Marea)...");
    match test_plan() {
        Ok(n) => {
            crate::println!("OK: {} pruebas pasadas.", n);
        }
        Err(msg) => {
            crate::println!("FALLO: {}", msg);
        }
    }
}
