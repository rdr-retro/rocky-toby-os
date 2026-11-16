//! Shim de `mouse` para el anfitrion.
//!
//! `event::sondear` (la unica parte del motor que toca hardware) no se usa
//! aqui: la ventana Win32 entrega el raton ya normalizado y se llama a
//! `App::procesar`. Este modulo solo existe para que `event.rs` compile sin
//! cambios.

/// En el anfitrion no hay sondeo de la PS/2.
pub fn poll() {}

/// Sin desplazamiento ni botones (no se usa).
pub fn tomar() -> (i32, i32, u8) {
    (0, 0, 0)
}
