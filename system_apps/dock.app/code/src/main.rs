#![no_std]
#![no_main]

use toby_appkit::{app_entry, println};

fn dock() {
    println!("Dock del escritorio Rocky");
    println!("Kira mantiene visible el contenedor azul anclado abajo.");
    println!("Toby Finder es un anclaje permanente del sistema.");
}

app_entry!(dock);
