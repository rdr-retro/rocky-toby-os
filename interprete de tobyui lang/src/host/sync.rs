//! Celda global sincronizada para estado mutable en `static`.
//!
//! Mismo tipo que `kernel/src/sync.rs`: `kit.rs` lo usa para su cache de pieles.
//! Aqui el anfitrion es monotarea, asi que basta con accesos no atomicos.

use core::cell::UnsafeCell;

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
