#![no_std]
#![no_main]

use toby_appkit::{app_entry, println};

fn system_preferences() {
    println!("System Preferences de Rocky");
    println!("Aplicacion de sistema preparada para sus paneles de configuracion.");
}

app_entry!(system_preferences);
