#![no_std]
#![no_main]

use toby_appkit::{app_entry, println};

fn terminal() {
    println!("Rocky Terminal - ASCII");
    println!("La consola se muestra en una ventana de Kira.");
}

app_entry!(terminal);
