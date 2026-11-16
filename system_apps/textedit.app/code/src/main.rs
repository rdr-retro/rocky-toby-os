#![no_std]
#![no_main]

use toby_appkit::{app_entry, println};

fn textedit() {
    println!("TextEdit — editor de texto de Rocky");
    println!("La ventana de escritura la proporciona Kira.");
}

app_entry!(textedit);
