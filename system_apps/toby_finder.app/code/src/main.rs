#![no_std]
#![no_main]

use toby_appkit::{app_entry, println};

fn toby_finder() {
    println!("Toby Finder — gestor de archivos de Rocky");
    println!("La interfaz gráfica de Finder se abre desde el Dock.");
    println!("Navegación y operaciones de archivos: siguiente etapa.");
}

app_entry!(toby_finder);
