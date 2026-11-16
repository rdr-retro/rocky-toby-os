//! Adaptador de secciones críticas para los anfitriones de escritorio.
//!
//! La UI de estos anfitriones se ejecuta en el hilo de mensajes; se conserva
//! la misma interfaz del kernel y la clausura se ejecuta sin cambio de tarea.

/// Ejecuta la operación de forma síncrona, sin ceder el hilo de UI.
#[inline]
pub fn critica<F: FnOnce() -> R, R>(f: F) -> R {
    f()
}
