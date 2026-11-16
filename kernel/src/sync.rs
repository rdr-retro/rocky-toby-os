//! Celda global sincronizada para estado mutable en `static`.
//!
//! rocky es mononucleo y monotarea, por lo que basta con accesos no atomicos
//! protegidos por el hecho de que solo hay un hilo de ejecucion.
//!
//! # Fase 4 Marea: reglas de propiedad del estado global
//!
//! Todo el estado global mutable del nucleo pasa por [`SyncCell`] o por
//! `Atomic*`; **no queda ningun `static mut`** (auditado en la Fase 4). Cada
//! parametro global cumple una de estas cuatro reglas:
//!
//! 1. **Escribir una vez, leer siempre** (`IDT`, `PILA_DF`, `COPIA` de BIOS,
//!    `PCI::TABLA`): se rellena en el arranque y despues solo se lee. No hay
//!    carrera porque el arranque es anterior a las IRQ.
//! 2. **Un unico dueno por tarea** (`PLAN`, `VOL`, `COMP`, `KIRA`, `REG`...): la
//!    referencia `&'static mut` vive lo justo para una operacion y **nunca se
//!    mantiene a traves de una llamada a otro global ni a traves de un punto que
//!    pueda cambiar de tarea** (`dormir`, `ceder`, `int 0x30`, `int 0x80`,
//!    `planificar`). Cuando hace falta tocar dos subsistemas, primero se cierra
//!    el prestamo del primero (ver `fd::soltar`).
//! 3. **Compartido con IRQ** (`TICKS`, `CRITICA`, `RING`, `EN_PANICO` y todos los
//!    contadores de `metricas`): **debe** ser `Atomic*`, porque lo escribe o lo
//!    lee un manejador. `input::RING` ademas se protege con `IrqOff` en
//!    `recoger`.
//! 4. **Estado de una sola ruta reentrante** (`Console`/`TEXTO`, `BACKEND`): la
//!    impresion puede reentrar desde la ruta de panico/excepcion. El contrato es
//!    que las secciones que lo toman son cortas y que **ninguna ruta de IRQ0/IRQ4
//!    imprime**. Con varias tareas imprimiendo (la del nucleo y la del USB) ya
//!    no basta: `console::Turno` serializa los mensajes con una bandera atomica
//!    y la suelta con `Drop`. Si se anade otra consola de salida, tiene que
//!    pasar por el mismo turno.
//!
//! ## Deteccion de alias mutables
//!
//! El patron peligroso es: `let a = estado_mut(); ... f()` donde `f()` vuelve a
//! llamar a `estado_mut()` sobre el **mismo** parametro. En la Fase 4 se
//! encontraron y corrigieron dos:
//!
//! - `kira::dev::escribir_kira` tenia `estado_mut()` vivo mientras
//!   `connection_setup_reply` (que llamaba a `kira::opcion()`) y
//!   `req::dispatch` (que via `wm::ensure_root` llamaba a `estado_mut()`)
//!   reentraban en KIRA. Ahora la geometria de la raiz y los XID viajan como
//!   parametros.
//! - `fd::soltar` mantenia `&mut ...TABLA` a traves de
//!   `pipe::soltar_extremo` / `kira::dev::cerrar_conexion`. Ahora cierra el
//!   prestamo antes de avisar al subsistema.

use core::cell::UnsafeCell;

/// Celda global para estado mutable en `static` (ver las reglas de arriba).
pub struct SyncCell<T>(UnsafeCell<T>);

unsafe impl<T> Sync for SyncCell<T> {}

impl<T> SyncCell<T> {
    pub const fn new(value: T) -> Self {
        SyncCell(UnsafeCell::new(value))
    }

    /// Devuelve un puntero crudo al valor interior.
    ///
    /// # Seguridad
    /// El llamante debe garantizar que no crea dos referencias mutables a la vez.
    #[inline(always)]
    pub unsafe fn get(&self) -> *mut T {
        self.0.get()
    }
}
