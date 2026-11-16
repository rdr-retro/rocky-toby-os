//! Controlador PS/2 (i8042): teclado y raton.
//!
//! rocky recoge teclado y raton por IRQ1/IRQ12 en colas acotadas, con sondeo de
//! respaldo para equipos cuyo firmware no enrute las interrupciones. El 8042 y
//! los dos dispositivos hay que ponerlos en marcha al arrancar.
//!
//! En QEMU (SeaBIOS) el 8042 viene listo para usar, y por eso alli el teclado y
//! el raton funcionan sin tocar nada. En un PC real el firmware lo deja en un
//! estado cualquiera: es frecuente que deshabilite el reloj del teclado, que el
//! teclado se quede con el envio de datos apagado (orden 0xF5) o que este
//! esperando el parametro de una orden a medias. En esas condiciones el
//! dispositivo no emite ni un byte y el sondeo no ve absolutamente nada.
//!
//! [`init`] deja el controlador en un estado conocido:
//!
//!   1. para los dos dispositivos y vacia lo que haya pendiente,
//!   2. activa los relojes, deja la traduccion set 2 -> set 1 en el 8042 y
//!      habilita las IRQ1/IRQ12,
//!   3. habilita los puertos de teclado (`0xAE`) y raton (`0xA8`),
//!   4. reactiva el barrido del teclado con el estado de protocolo compatible
//!      que dejo el firmware,
//!   5. pone el touchpad/raton en valores PS/2 relativos por defecto y activa
//!      su envio. No negocia modos privados SMBus/InterTouch.
//!
//! Todas las esperas tienen limite: si un dispositivo no esta o no contesta, el
//! arranque sigue y el problema queda registrado en el [`Informe`] que devuelve
//! `init` (el escritorio lo imprime en la ventana Terminal).

use crate::cpu::{inb, outb};

/// Puerto de datos del controlador.
const DATOS: u16 = 0x60;
/// Puerto de estado (lectura) y de ordenes (escritura) del controlador.
const ESTADO: u16 = 0x64;

/// Limite de espera, en lecturas del puerto de estado. Cada acceso a un puerto
/// de E/S cuesta del orden de un microsegundo, asi que 200.000 son unas decimas
/// de segundo: de sobra para un dispositivo presente y una salida rapida si no
/// lo esta.
const GUARDA: u32 = 200_000;

/// Como [`GUARDA`], pero para el reinicio del teclado, que puede tardar medio
/// segundo en contestar con el resultado de su autotest.
const GUARDA_LENTA: u32 = 1_500_000;

/// Resultado de [`init`]: lo que el firmware habia dejado hecho y lo que se ha
/// conseguido. El escritorio lo imprime como diagnostico, porque en un PC real
/// la unica forma de mirarlo suele ser leer la pantalla.
#[derive(Default)]
pub struct Informe {
    /// Byte de configuracion del controlador al llegar.
    pub cfg_antes: u8,
    /// Byte de configuracion que se ha escrito.
    pub cfg_despues: u8,
    /// El teclado acepto el juego de codigos 2.
    pub teclado_set2: bool,
    /// Respuesta del teclado a la orden 0xF4 (0xFA = correcto).
    pub teclado_eco: Option<u8>,
    /// Hubo que reiniciar el teclado (no contestaba).
    pub teclado_reinicio: bool,
    /// El teclado queda en condiciones de enviar datos.
    pub teclado_ok: bool,
    /// El raton contesta a las ordenes.
    pub raton_ack: bool,
    /// Identificador que devuelve el raton (0x00 = raton estandar).
    pub raton_id: Option<u8>,
    /// Puerto de estado al terminar (bit 0 = hay un byte pendiente).
    pub estado_final: u8,
}

impl Informe {
    /// Imprime el diagnostico.
    ///
    /// Cuando los dos dispositivos responden y el teclado queda activo, basta
    /// una linea. Si algo falla se detalla todo: en un PC real la unica forma de
    /// mirarlo suele ser leer la pantalla, y ahi cada dato cuenta.
    pub fn imprimir(&self) {
        if self.teclado_ok && self.raton_ack {
            crate::println!("entrada: teclado y dispositivo apuntador PS/2 listos.");
            return;
        }

        crate::println!("entrada (controlador PS/2):");
        crate::println!(
            "  configuracion   0x{:02X} -> 0x{:02X}   (estado final 0x{:02X})",
            self.cfg_antes,
            self.cfg_despues,
            self.estado_final
        );
        if self.teclado_ok {
            if self.teclado_reinicio {
                crate::println!("  teclado         activo tras reiniciarlo (0xF4 = 0xFA)");
            } else {
                crate::println!(
                    "  teclado         activo (0xF4 = {})",
                    hex(self.teclado_eco)
                );
            }
        } else {
            crate::println!(
                "  teclado         SIN RESPUESTA a 0xF4 ({})",
                hex(self.teclado_eco)
            );
            crate::println!("                  el firmware no lo entrega al 8042");
            crate::println!("                  revisa el soporte de teclado en la BIOS");
        }
        if self.raton_ack {
            crate::println!(
                "  raton           activo (identificador {})",
                hex(self.raton_id)
            );
            crate::println!("                  protocolo relativo estandar");
        } else {
            crate::println!("  raton           SIN RESPUESTA en el puerto auxiliar");
            crate::println!("                  (un raton USB solo aparece ahi si la");
            crate::println!("                   BIOS lo emula como PS/2)");
        }
    }
}

fn hex(valor: Option<u8>) -> alloc::string::String {
    use alloc::format;
    match valor {
        Some(v) => format!("0x{:02X}", v),
        None => alloc::string::String::from("sin dato"),
    }
}

/// Estado del controlador (puerto 0x64).
fn estado() -> u8 {
    unsafe { inb(ESTADO) }
}

/// Espera a que el bufer de entrada del controlador quede libre.
unsafe fn esperar_entrada() -> bool {
    let mut guarda = 0u32;
    while inb(ESTADO) & 0x02 != 0 {
        guarda += 1;
        if guarda > GUARDA {
            return false;
        }
    }
    true
}

/// Envia una orden al controlador (puerto 0x64).
unsafe fn orden(cmd: u8) -> bool {
    if !esperar_entrada() {
        return false;
    }
    outb(ESTADO, cmd);
    true
}

/// Escribe un byte en el puerto de datos del controlador (0x60).
unsafe fn escribir_dato(dato: u8) -> bool {
    if !esperar_entrada() {
        return false;
    }
    outb(DATOS, dato);
    true
}

/// Lee un byte del puerto de datos, esperando (con limite) a que lo haya.
unsafe fn leer_dato_espera(guarda_max: u32) -> Option<u8> {
    let mut guarda = 0u32;
    while inb(ESTADO) & 0x01 == 0 {
        guarda += 1;
        if guarda > guarda_max {
            return None;
        }
    }
    Some(inb(DATOS))
}

/// Como [`leer_dato_espera`] con la espera normal.
unsafe fn leer_dato() -> Option<u8> {
    leer_dato_espera(GUARDA)
}

/// Descarta los bytes pendientes en el controlador.
unsafe fn drenar() {
    let mut guarda = 0u32;
    while inb(ESTADO) & 0x01 != 0 && guarda < 256 {
        let _ = inb(DATOS);
        guarda += 1;
    }
}

/// Envia al teclado la mascara de sus luces (0xED).
///
/// Bit 0 = Bloq Despl, bit 1 = Bloq Num, bit 2 = Bloq Mayus. El teclado pide un
/// ACK (0xFA) antes del dato y otro despues; si no contesta en un rato, se
/// devuelve sin mas: la luz es un detalle, no algo que deba parar el teclado.
pub fn leds_teclado(mascara: u8) {
    unsafe {
        if !escribir_dato(0xED) || !esperar_ack() {
            return;
        }
        if !escribir_dato(mascara) {
            return;
        }
        esperar_ack();
    }
}

/// Espera el ACK del teclado, descartando las pulsaciones que lleguen en medio.
///
/// Ignora bytes auxiliares mientras espera, para que un movimiento del touchpad
/// no se consuma como si fuera un ACK de los LEDs.
unsafe fn esperar_ack() -> bool {
    for _ in 0..GUARDA {
        let estado = inb(ESTADO);
        if estado & 0x01 == 0 {
            continue;
        }
        let byte = inb(DATOS);
        if estado & 0x20 != 0 {
            crate::mouse::alimentar(byte);
        } else {
            match byte {
                0xFA => return true,
                0xFE => return false,
                _ => crate::input::guardar_scancode_ps2(byte),
            }
        }
    }
    false
}

/// Lee el byte de configuracion del controlador (orden 0x20).
unsafe fn leer_config() -> u8 {
    if !orden(0x20) {
        return 0;
    }
    leer_dato().unwrap_or(0)
}

/// Escribe el byte de configuracion del controlador (orden 0x60).
unsafe fn escribir_config(cfg: u8) -> bool {
    orden(0x60) && escribir_dato(cfg)
}

/// Envia una orden al teclado y espera su ACK real.
///
/// El teclado puede dejar scancodes pendientes durante la inicializacion.
/// Leer solo un byte hacia que una tecla pulsada se tomara por el ACK y
/// desincronizaba todos los comandos siguientes (F0/F4). Se descartan esos
/// bytes hasta recibir 0xFA; si el dispositivo pide reenvio (0xFE), se reintenta.
unsafe fn orden_teclado(cmd: u8) -> Option<u8> {
    for _ in 0..3 {
        if !escribir_dato(cmd) {
            return None;
        }
        let mut reintentar = false;
        for _ in 0..GUARDA {
            let estado = inb(ESTADO);
            if estado & 0x01 == 0 {
                continue;
            }
            let byte = inb(DATOS);
            if estado & 0x20 != 0 {
                // No interpretar informes auxiliares como respuesta del
                // teclado; si ya se movio, conservar ese paquete.
                crate::mouse::alimentar(byte);
                continue;
            }
            match byte {
                0xFA => return Some(byte),
                0xFE => {
                    reintentar = true;
                    break;
                }
                _ => crate::input::guardar_scancode_ps2(byte),
            }
        }
        if !reintentar {
            return None;
        }
    }
    None
}

/// Envia una orden al dispositivo auxiliar (el raton) y espera su ACK.
///
/// El protocolo es el del controlador: primero la orden 0xD4 (siguiente byte
/// para el dispositivo auxiliar) y despues el byte. Si en medio aparece un
/// scancode del teclado, se descarta y se sigue esperando.
unsafe fn orden_aux(cmd: u8) -> Option<u8> {
    if !orden(0xD4) {
        return None;
    }
    if !escribir_dato(cmd) {
        return None;
    }
    for _ in 0..GUARDA {
        let estado = inb(ESTADO);
        if estado & 0x01 == 0 {
            continue;
        }
        let byte = inb(DATOS);
        if estado & 0x20 != 0 {
            match byte {
                0xFA => return Some(0xFA),
                0xFE => return None,
                _ => continue, // bytes de respuesta del dispositivo auxiliar
            }
        } else {
            crate::input::guardar_scancode_ps2(byte);
        }
    }
    None
}

/// Reinicia el teclado (orden 0xFF): contesta ACK (0xFA) y despues 0xAA si su
/// autotest ha ido bien. Un teclado reiniciado queda en el juego de codigos 2
/// con el envio de datos activado, que es justo lo que queremos.
unsafe fn reiniciar_teclado() -> bool {
    if !escribir_dato(0xFF) {
        return false;
    }
    for _ in 0..4 {
        match leer_dato_espera(GUARDA_LENTA) {
            Some(0xAA) => return true,
            Some(_) => continue,
            None => return false,
        }
    }
    false
}

/// Prepara el controlador y los dos dispositivos. Ver el modulo.
pub fn init() -> Informe {
    let mut inf = Informe::default();
    unsafe {
        // 1) Evitar que los ACK de configuración entren en las colas de IRQ.
        crate::pic::enmascarar_ps2();
        // Los dos dispositivos se quedan quietos mientras se configura.
        orden(0xAD); // teclado fuera
        orden(0xA7); // raton fuera
        drenar();

        // 2) Reactivar los relojes y fijar el formato que espera input.rs.
        //    El teclado interno del T440p está conectado al 8042 por el
        //    firmware/EC; mantener el protocolo PS/2 estándar evita cambiarlo
        //    a un modo propietario. El 8042 traduce el set 2 normal a set 1.
        inf.cfg_antes = leer_config();
        let mut cfg = inf.cfg_antes;
        cfg &= !0x10; // reloj del teclado habilitado
        cfg &= !0x20; // reloj del raton habilitado
        cfg |= 0x03; // IRQ1/IRQ12; el PIC se mantiene enmascarado hasta acabar
        cfg |= 0x40; // traducir set 2 -> set 1 para el decodificador existente
        escribir_config(cfg);
        inf.cfg_despues = leer_config();
        inf.teclado_set2 = inf.cfg_despues & 0x40 != 0;

        // 3) Habilitar los dos puertos.
        orden(0xAE); // teclado
        orden(0xA8); // auxiliar (raton)

        // 4) F4 es idempotente y reactiva un teclado que el firmware dejo con
        //    scanning apagado. No enviamos F0/F5: no hace falta cambiar el
        //    juego de barridos por defecto y algunos EC de portatiles no
        //    implementan esa negociacion como un teclado de sobremesa.
        inf.teclado_eco = orden_teclado(0xF4); // Enable Scanning
        inf.teclado_ok = inf.teclado_eco == Some(0xFA);
        if !inf.teclado_ok {
            drenar();
            inf.teclado_reinicio = reiniciar_teclado();
            if inf.teclado_reinicio {
                inf.teclado_eco = orden_teclado(0xF4);
                inf.teclado_ok = inf.teclado_eco == Some(0xFA);
            }
        }

        // 5) Dispositivo apuntador en modo relativo PS/2 estándar. Se omiten
        //    las secuencias privadas de Synaptics/InterTouch: el UltraNav del
        //    T440p debe conservar la compatibilidad PS/2; el lector consume
        //    paquetes relativos de tres bytes.
        inf.raton_ack = orden_aux(0xF6).is_some(); // Set Defaults
        inf.raton_ack &= orden_aux(0xF4).is_some(); // Enable Data Reporting

        // A partir de aquí los eventos se capturan aunque Kira esté en medio
        // de un repintado largo. El sondeo del bucle sigue como respaldo.
        crate::pic::habilitar_ps2();

        // 6) Como queda el controlador (bit 0 = hay algo pendiente).
        inf.estado_final = estado();
    }
    inf
}
