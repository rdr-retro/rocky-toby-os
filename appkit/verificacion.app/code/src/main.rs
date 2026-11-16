#![no_std]
#![no_main]

use toby_appkit::{app_entry, println};

fn mi_app() {
    println!("Hola desde Toby AppKit");
}

app_entry!(mi_app);
