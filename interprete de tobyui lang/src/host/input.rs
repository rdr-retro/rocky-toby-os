//! Shim de `input` para el anfitrion.
//!
//! Igual que `mouse`: solo rellena lo que `event.rs` espera del kernel. El
//! teclado lo entrega Win32 (WM_KEYDOWN) y se llama a `App::procesar`.

/// No hay cola de teclado propia del kernel que vaciar.
pub fn poll() -> Option<u8> {
    None
}
