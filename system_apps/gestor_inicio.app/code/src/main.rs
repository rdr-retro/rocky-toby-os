#![no_std]
#![no_main]

fn gestor_inicio() {
    toby_appkit::println!("Gestor de inicio de Rocky");
    toby_appkit::println!("Usa 'inicio' para ver las aplicaciones instaladas.");
    toby_appkit::println!("inicio activar sistema/<app>");
    toby_appkit::println!("inicio desactivar usuario/<app>");
    toby_appkit::println!("inicio prioridad usuario/<app> <0..1000>");
}

toby_appkit::app_entry!(gestor_inicio);
