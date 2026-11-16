//! Tabla de descriptores de interrupciones (IDT) y gestion de excepciones/IRQ.

use core::arch::asm;

use crate::pic;
use crate::pit;
use crate::println;
use crate::sync::SyncCell;

/// Marco de pila que los stubs de `interrupts.s` construyen antes de llamar
/// a `isr_dispatch`. El orden debe coincidir con el de los `push`/`pop`.
#[repr(C)]
pub struct InterruptStack {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }

    fn new(handler: u64) -> Self {
        IdtEntry {
            offset_low: handler as u16,
            selector: 0x18,
            ist: 0,
            type_attr: 0x8E, // presente, anillo 0, puerta de interrupcion de 64 bits
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        }
    }

    /// Puerta accesible desde el anillo 3 (DPL = 3): la usa `int 0x80`.
    fn new_usuario(handler: u64) -> Self {
        IdtEntry {
            offset_low: handler as u16,
            selector: 0x18,
            ist: 0,
            type_attr: 0xEE, // presente, DPL 3, puerta de interrupcion de 64 bits
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        }
    }

    /// Igual que [`IdtEntry::new`], pero haciendo que la CPU cambie a la pila
    /// `ist` (1..7) antes de entrar. Se usa para el doble fallo.
    fn con_ist(handler: u64, ist: u8) -> Self {
        IdtEntry {
            offset_low: handler as u16,
            selector: 0x18,
            ist,
            type_attr: 0x8E, // presente, anillo 0, puerta de interrupcion de 64 bits
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        }
    }
}

#[repr(C, packed)]
struct IdtDescriptor {
    limit: u16,
    base: u64,
}

static IDT: SyncCell<[IdtEntry; 256]> = SyncCell::new([IdtEntry::missing(); 256]);

extern "C" {
    /// Direcciones de los stubs 0..48 (tabla definida en `interrupts.s`).
    static isr_vectors: [unsafe extern "C" fn(); 49];
    /// Stub generico para cualquier vector sin uno propio.
    fn isr_espurio();
    /// Stub de `int 0x80` (llamada al sistema desde el anillo 3).
    fn isr128();
}

pub fn init() {
    unsafe {
        let idt = &mut *IDT.get();
        // Todas las puertas tienen manejador: ninguna excepcion puede quedarse
        // sin puerta (eso acababa en #DF y ocultaba el fallo de verdad).
        for (vector, handler) in isr_vectors.iter().enumerate() {
            idt[vector] = IdtEntry::new(*handler as u64);
        }
        let generico = isr_espurio as *const () as u64;
        for vector in isr_vectors.len()..256 {
            idt[vector] = IdtEntry::new(generico);
        }

        // `int 0x80`: puerta accesible desde el anillo 3.
        idt[0x80] = IdtEntry::new_usuario(isr128 as *const () as u64);

        // El doble fallo (#DF) entra por su pila de emergencia (IST1), que no
        // depende de ninguna tarea: es el ultimo recurso si la pila del kernel
        // se agoto o se corrompio.
        idt[8] = IdtEntry::con_ist(isr_vectors[8] as u64, crate::gdt::IST_DF);

        let descriptor = IdtDescriptor {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: idt.as_ptr() as u64,
        };

        asm!("lidt [{}]", in(reg) &descriptor, options(readonly, nostack));
    }
}

/// Vectores que empujan codigo de error.
fn empuja_codigo_de_error(vector: u64) -> bool {
    matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17 | 21 | 29 | 30)
}

/// Si un vector con codigo de error lo dispara algo que no empuja codigo (una
/// IRQ, por ejemplo) el marco queda desplazado 8 bytes: lo que esperamos en
/// `error` es el RIP real. Se detecta porque el CS de verdad tiene que ser el
/// selector de codigo del kernel (0x18) y lo recolocamos.
fn normalizar_marco(f: &mut InterruptStack) {
    if !empuja_codigo_de_error(f.vector) || f.cs == 0x18 || f.rip != 0x18 {
        return;
    }
    f.ss = f.rsp;
    f.rsp = f.rflags;
    f.rflags = f.cs;
    f.cs = f.rip;
    f.rip = f.error;
    f.error = 0;
}

/// Nombre legible de un vector, para el diagnostico.
pub fn vector_name(vector: u64) -> &'static str {
    match vector {
        0 => "division por cero (#DE)",
        1 => "depuracion (#DB)",
        2 => "interrupcion no enmascarable (NMI)",
        3 => "punto de ruptura (#BP)",
        4 => "desbordamiento (#OF)",
        5 => "limite de rango (#BR)",
        6 => "codigo invalido (#UD)",
        7 => "dispositivo no disponible (#NM)",
        8 => "doble fallo (#DF)",
        9 => "excepcion obsoleta",
        10 => "TSS invalido (#TS)",
        11 => "segmento ausente (#NP)",
        12 => "falta de pila (#SS)",
        13 => "fallo general de proteccion (#GP)",
        14 => "fallo de pagina (#PF)",
        15 => "vector reservado",
        16 => "error de coma flotante (#MF)",
        17 => "comprobacion de alineacion (#AC)",
        18 => "comprobacion de maquina (#MC)",
        19 => "error SIMD (#XM)",
        20 => "excepcion de virtualizacion (#VE)",
        21 => "proteccion de control (#CP)",
        29 => "excepcion de comunicacion (#VC)",
        30 => "excepcion de seguridad (#SX)",
        32 => "IRQ0: temporizador",
        33 => "IRQ1: teclado",
        34 => "IRQ2: cascada",
        35 => "IRQ3: COM2",
        36 => "IRQ4: COM1",
        37 => "IRQ5",
        38 => "IRQ6",
        39 => "IRQ7",
        40 => "IRQ8: reloj de tiempo real",
        41 => "IRQ9",
        42 => "IRQ10",
        43 => "IRQ11",
        44 => "IRQ12: raton",
        45 => "IRQ13: coprocesador",
        46 => "IRQ14: disco",
        47 => "IRQ15",
        48 => "ceder el turno (int 0x30)",
        0x1000 => "vector sin manejador (49..255)",
        0x1001 => "numero de vector invalido",
        _ => "excepcion desconocida",
    }
}

#[no_mangle]
pub extern "C" fn isr_dispatch(stack: *mut InterruptStack) -> *mut InterruptStack {
    let vector = unsafe { (*stack).vector };
    normalizar_marco(unsafe { &mut *stack });
    let salida = match vector {
        32 => {
            // Tic del reloj: se cuenta, se revisan las alarmas (`SIGALRM`) y,
            // ademas, se reparte el turno.
            pit::tick();
            crate::tarea::comprobar_alarmas(pit::ticks());
            unsafe { pic::eoi(0) };
            crate::tarea::planificar(stack)
        }
        48 => {
            // `int 0x30`: la tarea deja el turno a proposito.
            crate::tarea::planificar(stack)
        }
        128 => {
            // `int 0x80`: llamada al sistema desde el anillo 3.
            crate::syscall::dispatch(stack)
        }
        14 => {
            // `#PF`: fallo de pagina. Lo resuelve el espacio de direcciones del
            // proceso en curso (copia-al-escribir o traida bajo demanda). Si no
            // puede, el fallo es grave y se informa.
            if crate::proceso::fallo_pagina(unsafe { (*stack).error }) {
                stack
            } else {
                excepcion(14, stack)
            }
        }
        33 => {
            crate::input::on_ps2_irq();
            unsafe { pic::eoi(1) };
            stack
        }
        36 => {
            crate::input::on_serial_irq();
            unsafe { pic::eoi(4) };
            stack
        }
        44 => {
            crate::input::on_ps2_irq();
            unsafe { pic::eoi(12) };
            stack
        }
        // Cualquier otra IRQ (32..47, PIC ya remapeado): se atiende y se sigue.
        v if (32..48).contains(&v) => {
            unsafe { pic::eoi((v - 32) as u8) };
            stack
        }
        v => excepcion(v, stack),
    };
    // **Punto unico de entrega de senales** (el `do_signal` de Linux): al final
    // de toda interrupcion, si el marco que va a volver lleva al anillo 3, se
    // entrega la primera senal pendiente. Asi una senal se ve en cuanto el
    // proceso vuelve a correr, venga de una llamada al sistema, del reloj o de
    // una excepcion.
    crate::senal::comprobar(salida)
}

/// Atiende una excepcion de la CPU que no se ha podido resolver.
///
/// Si viene del anillo 3 y es de las que un programa puede provocar por si
/// mismo, se contiene terminando **solo ese proceso** y el sistema sigue. En
/// cualquier otro caso el fallo es del nucleo y se entra en la pantalla azul
/// (ver [`crate::panico`]).
fn excepcion(vector: u64, stack: *mut InterruptStack) -> *mut InterruptStack {
    let f = unsafe { &*stack };
    if f.cs & 3 == 3 && contenible_de_usuario(vector) {
        // Fase 7: si el proceso tiene instalado un manejador para la senal que
        // genera esta excepcion (#PF -> SIGSEGV, #DE -> SIGFPE...), se le
        // entrega y es el manejador quien decide. Si no, se termina el proceso
        // como antes (la convencion de "muerto por senal" de Unix).
        if let Some(signr) = crate::senal::desde_excepcion(vector) {
            if crate::senal::capturada(signr) {
                crate::senal::encolar_actual(signr);
                return stack;
            }
        }
        println!(
            "rocky: PID {} terminado por {} en RIP={:#X}",
            crate::tarea::pid_actual().unwrap_or(0),
            vector_name(vector),
            f.rip
        );
        // Codigo negativo: la convencion de "muerto por senal" de Unix.
        return crate::tarea::abortar(stack, -(vector as i64));
    }
    crate::panico::desde_excepcion(vector, f)
}

/// Excepciones que, cometidas desde el anillo 3, se contienen terminando el
/// proceso en vez de detener todo el sistema.
///
/// Se quedan fuera las que no tienen una recuperacion clara (#DF, #MC, #VE...):
/// esas senalan un problema del propio nucleo o del entorno, no del programa.
fn contenible_de_usuario(vector: u64) -> bool {
    matches!(vector, 0 | 3 | 4 | 5 | 6 | 7 | 13 | 14 | 16 | 17)
}
