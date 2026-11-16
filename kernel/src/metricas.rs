//! Contadores ligeros de telemetría del kernel (Fase 0 del plan Marea).
//!
//! Son solo `AtomicU64` incrementados en los puntos calientes: cambio de tarea,
//! syscall, heap y disco. El coste por evento es una sola instrucción
//! `lock xadd` (~20 ciclos), irrelevante frente al trabajo que mide.
//!
//! Se leen con el comando `metricas` del shell. No guardan timestamps ni
//! percentiles: eso será la Fase 1.

use core::sync::atomic::{AtomicU64, Ordering};

/// Cambios de contexto totales (IRQ0 + `int 0x30`).
pub static CAMBIOS_TAREA: AtomicU64 = AtomicU64::new(0);
/// Cambios de contexto en los que se eligió la tarea idle (no había trabajo).
pub static CAMBIOS_IDLE: AtomicU64 = AtomicU64::new(0);
/// Llamadas al sistema (`int 0x80`).
pub static SYSCALLS: AtomicU64 = AtomicU64::new(0);
/// Asignaciones de heap (`alloc`).
pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// Liberaciones de heap (`dealloc`).
pub static DEALLOCS: AtomicU64 = AtomicU64::new(0);
/// Bytes totales pedidos al heap (suma de `layout.size()`).
pub static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
/// Crecimientos del heap (mapeo de páginas nuevas)
pub static HEAP_GROWTHS: AtomicU64 = AtomicU64::new(0);
/// Sectores leídos del disco.
pub static LECTURAS: AtomicU64 = AtomicU64::new(0);
/// Sectores escritos al disco.
pub static ESCRITURAS: AtomicU64 = AtomicU64::new(0);

// --- Fase 2: disciplina de interrupciones ---------------------------------
/// Secciones con interrupciones desactivadas ejecutadas (vía `IrqOff`).
pub static IRQ_OFF_VECES: AtomicU64 = AtomicU64::new(0);
/// Duración acumulada de esas secciones, en ticks de PIT (10 ms).
pub static IRQ_OFF_TICKS: AtomicU64 = AtomicU64::new(0);
/// Sección con interrupciones desactivadas más larga vista, en ticks (10 ms).
pub static IRQ_OFF_MAX_TICKS: AtomicU64 = AtomicU64::new(0);
/// Secciones con la preempción inhibida (`tarea::critica`) ejecutadas.
pub static CRITICA_VECES: AtomicU64 = AtomicU64::new(0);
/// Duración máxima observada de una sección crítica, en ticks de PIT (10 ms).
pub static CRITICA_MAX_TICKS: AtomicU64 = AtomicU64::new(0);

// --- Fase 3: colas y equidad ----------------------------------------------
/// Turnos en que el presupuesto de equidad hizo ceder a la clase interactiva en
/// favor de la normal (prueba de que no hay inanición).
pub static EQUIDAD_CESIONES: AtomicU64 = AtomicU64::new(0);
/// Tareas despertadas por vencimiento de reloj (coste del camino O(vencidos)).
pub static DESPERTARES_RELOJ: AtomicU64 = AtomicU64::new(0);
/// Turnos despachados a la clase interactiva.
pub static TURNOS_INTERACTIVA: AtomicU64 = AtomicU64::new(0);
/// Turnos despachados a la clase normal.
pub static TURNOS_NORMAL: AtomicU64 = AtomicU64::new(0);

/// Incrementa el contador de cambios de contexto. `idle` indica si la tarea
/// elegida es la idle.
#[inline(always)]
pub fn cambio_tarea(idle: bool) {
    CAMBIOS_TAREA.fetch_add(1, Ordering::Relaxed);
    if idle {
        CAMBIOS_IDLE.fetch_add(1, Ordering::Relaxed);
    }
}

/// Incrementa el contador de syscalls.
#[inline(always)]
pub fn syscall() {
    SYSCALLS.fetch_add(1, Ordering::Relaxed);
}

/// Incrementa los contadores de alloc.
#[inline(always)]
pub fn alloc(bytes: usize) {
    ALLOCS.fetch_add(1, Ordering::Relaxed);
    ALLOC_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

/// Incrementa contador de crecimiento de heap
#[inline(always)]
pub fn heap_grow() {
    HEAP_GROWTHS.fetch_add(1, Ordering::Relaxed);
}

/// Incrementa el contador de dealloc.
#[inline(always)]
pub fn dealloc() {
    DEALLOCS.fetch_add(1, Ordering::Relaxed);
}

/// Incrementa el contador de lecturas de sector.
#[inline(always)]
pub fn lectura() {
    LECTURAS.fetch_add(1, Ordering::Relaxed);
}

/// Incrementa el contador de escrituras de sector.
#[inline(always)]
pub fn escritura() {
    ESCRITURAS.fetch_add(1, Ordering::Relaxed);
}

/// Registra una sección con interrupciones desactivadas que duró `ticks`
/// ticks de PIT (Fase 2). Llama a esto quien crea una `IrqOff` y mide el
/// lapso hasta que se suelta.
#[inline(always)]
pub fn irq_off(ticks: u64) {
    IRQ_OFF_VECES.fetch_add(1, Ordering::Relaxed);
    IRQ_OFF_TICKS.fetch_add(ticks, Ordering::Relaxed);
    // Actualiza el máximo sin bloqueos: en mononucleo basta con comparar.
    let mut max = IRQ_OFF_MAX_TICKS.load(Ordering::Relaxed);
    while ticks > max {
        match IRQ_OFF_MAX_TICKS.compare_exchange_weak(
            max,
            ticks,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => max = actual,
        }
    }
}

/// Registra una sección con la preempción inhibida (Fase 2).
#[inline(always)]
pub fn critica() {
    CRITICA_VECES.fetch_add(1, Ordering::Relaxed);
}

/// Registra la duración de una sección crítica. Es útil para detectar renders
/// o lecturas accidentales que bloqueen el planificador durante demasiado.
#[inline(always)]
pub fn critica_duracion(ticks: u64) {
    let mut max = CRITICA_MAX_TICKS.load(Ordering::Relaxed);
    while ticks > max {
        match CRITICA_MAX_TICKS.compare_exchange_weak(
            max,
            ticks,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => max = actual,
        }
    }
}

/// Registra una cesión del presupuesto de equidad (Fase 3).
#[inline(always)]
pub fn equidad_cede() {
    EQUIDAD_CESIONES.fetch_add(1, Ordering::Relaxed);
}

/// Registra un turno despachado a una clase (Fase 3). `interactiva` distingue
/// las dos clases.
#[inline(always)]
pub fn turno(interactiva: bool) {
    if interactiva {
        TURNOS_INTERACTIVA.fetch_add(1, Ordering::Relaxed);
    } else {
        TURNOS_NORMAL.fetch_add(1, Ordering::Relaxed);
    }
}

/// Registra un despertar por vencimiento de reloj (Fase 3).
#[inline(always)]
pub fn despertar_reloj() {
    DESPERTARES_RELOJ.fetch_add(1, Ordering::Relaxed);
}

/// Escribe la instantánea en pantalla (orden `metricas` del shell).
pub fn imprimir() {
    let tareas = CAMBIOS_TAREA.load(Ordering::Relaxed);
    let idle = CAMBIOS_IDLE.load(Ordering::Relaxed);
    let sys = SYSCALLS.load(Ordering::Relaxed);
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let deallocs = DEALLOCS.load(Ordering::Relaxed);
    let alloc_bytes = ALLOC_BYTES.load(Ordering::Relaxed);
    let lect = LECTURAS.load(Ordering::Relaxed);
    let escr = ESCRITURAS.load(Ordering::Relaxed);

    let uptime = crate::pit::ticks();
    let seg = uptime / 100;
    let cs = uptime % 100;

    crate::println!("Metricas del kernel (Fase 0 Marea)");
    crate::println!("  uptime:             {:>3}.{:02} s", seg, cs);
    crate::println!();
    crate::println!("  Cambios de tarea:   {}", tareas);
    if tareas > 0 {
        crate::println!(
            "  ... a idle:         {} ({}.{:01}%)",
            idle,
            idle * 100 / tareas,
            (idle * 1000 / tareas) % 10
        );
    }
    crate::println!();
    crate::println!("  Syscalls:           {}", sys);
    if seg > 0 {
        crate::println!("  ... por segundo:    {}", sys / seg);
    }
    crate::println!();
    crate::println!("  Heap allocs:        {}", allocs);
    crate::println!("  Heap deallocs:      {}", deallocs);
    crate::println!(
        "  Heap bytes pedidos: {} ({} KiB)",
        alloc_bytes,
        alloc_bytes / 1024
    );
    if allocs > 0 {
        crate::println!("  Media por alloc:    {} bytes", alloc_bytes / allocs);
    }
    crate::println!();
    crate::println!("  Sectores leidos:    {}", lect);
    crate::println!("  Sectores escritos:  {}", escr);
    if seg > 0 {
        crate::println!("  Lecturas/seg:       {}", lect / seg);
        crate::println!("  Escrituras/seg:     {}", escr / seg);
    }

    // --- Fase 2: disciplina de interrupciones (medidas) ---
    let irq_veces = IRQ_OFF_VECES.load(Ordering::Relaxed);
    let irq_ticks = IRQ_OFF_TICKS.load(Ordering::Relaxed);
    let irq_max = IRQ_OFF_MAX_TICKS.load(Ordering::Relaxed);
    let crit_veces = CRITICA_VECES.load(Ordering::Relaxed);
    let crit_max = CRITICA_MAX_TICKS.load(Ordering::Relaxed);

    crate::println!();
    crate::println!("  Fase 2: interrupciones");
    crate::println!("  Secciones IRQ off:  {}", irq_veces);
    crate::println!(
        "  ... acumuladas:     {} ticks ({} ms)",
        irq_ticks,
        irq_ticks * 10
    );
    if irq_veces > 0 {
        crate::println!(
            "  ... media:          {} ticks ({} ms)",
            irq_ticks / irq_veces,
            (irq_ticks / irq_veces) * 10
        );
    }
    crate::println!(
        "  ... maxima:         {} ticks ({} ms)",
        irq_max,
        irq_max * 10
    );
    if uptime > 0 {
        crate::println!(
            "  ... % del tiempo:   {}.{:01}%",
            irq_ticks * 100 / uptime,
            (irq_ticks * 1000 / uptime) % 10
        );
    }
    crate::println!("  Secciones criticas: {}", crit_veces);
    crate::println!(
        "  ... maxima:         {} ticks ({} ms)",
        crit_max,
        crit_max * 10
    );

    // --- Fase 3: colas y equidad (medidas) ---
    let cesiones = EQUIDAD_CESIONES.load(Ordering::Relaxed);
    let despiertos = DESPERTARES_RELOJ.load(Ordering::Relaxed);
    let t_inter = TURNOS_INTERACTIVA.load(Ordering::Relaxed);
    let t_normal = TURNOS_NORMAL.load(Ordering::Relaxed);

    crate::println!();
    crate::println!("  Fase 3: colas y equidad");
    crate::println!("  Turnos interactiva: {}", t_inter);
    crate::println!("  Turnos normal:      {}", t_normal);
    crate::println!("  Cesiones de turno:  {}", cesiones);
    crate::println!("  Despertares reloj:  {}", despiertos);
}
