//! Soporte base de CPU x86-64, identificación Intel/Haswell, E/S e interrupciones.

use core::arch::asm;

/// Perfil detectado mediante CPUID. No habilitamos AVX hasta que el cambio de
/// tarea guarde/restaure el estado YMM de cada proceso.
#[derive(Clone, Copy)]
pub struct Perfil {
    pub vendor: [u8; 12],
    pub marca: [u8; 48],
    pub family: u16,
    pub model: u16,
    pub stepping: u8,
    pub nucleos: u8,
    pub hilos: u8,
    pub haswell: bool,
    pub avx2: bool,
}

/// Detecta e informa el procesador que ejecuta rocky. El i7-4712MQ es Haswell
/// (familia 6, modelo 0x3C); esta ruta no escribe MSR ni altera voltajes/relojes.
pub fn init() -> Perfil {
    let vendor = fabricante();
    let marca = marca();
    let max_leaf = unsafe { cpuid(0, 0) }.0;
    let leaf1 = if max_leaf >= 1 {
        unsafe { cpuid(1, 0) }
    } else {
        (0, 0, 0, 0)
    };
    let raw_family = ((leaf1.0 >> 8) & 0x0F) as u16;
    let ext_family = ((leaf1.0 >> 20) & 0xFF) as u16;
    let family = if raw_family == 0x0F {
        raw_family + ext_family
    } else {
        raw_family
    };
    let raw_model = ((leaf1.0 >> 4) & 0x0F) as u16;
    let ext_model = ((leaf1.0 >> 16) & 0x0F) as u16;
    let model = if matches!(raw_family, 0x06 | 0x0F) {
        (ext_model << 4) | raw_model
    } else {
        raw_model
    };
    let stepping = (leaf1.0 & 0x0F) as u8;
    let hilos = ((leaf1.1 >> 16) & 0xFF) as u8;
    let nucleos = if max_leaf >= 4 && vendor == *b"GenuineIntel" {
        (((unsafe { cpuid(4, 0) }.0 >> 26) & 0x3F) + 1) as u8
    } else {
        hilos.max(1)
    };
    let haswell =
        vendor == *b"GenuineIntel" && family == 6 && matches!(model, 0x3C | 0x3F | 0x45 | 0x46);
    let avx2 = max_leaf >= 7 && unsafe { cpuid(7, 0) }.1 & (1 << 5) != 0;
    let perfil = Perfil {
        vendor,
        marca,
        family,
        model,
        stepping,
        nucleos,
        hilos: hilos.max(1),
        haswell,
        avx2,
    };

    let fin = perfil
        .marca
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(perfil.marca.len());
    let nombre = core::str::from_utf8(&perfil.marca[..fin])
        .unwrap_or("CPU x86-64")
        .trim();
    let vendor = core::str::from_utf8(&perfil.vendor).unwrap_or("desconocido");
    crate::println!(
        "rocky: CPU {} [{}], familia {} modelo {:02X} stepping {}, {} nucleos/{} hilos.",
        nombre,
        vendor,
        perfil.family,
        perfil.model,
        perfil.stepping,
        perfil.nucleos,
        perfil.hilos
    );
    if perfil.haswell {
        crate::println!(
            "rocky: perfil Intel Haswell activo; AVX2 {} (no se usa hasta guardar estado YMM por tarea).",
            if perfil.avx2 { "disponible" } else { "no disponible" }
        );
    }
    perfil
}

#[inline(always)]
pub unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value,
         options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port,
         options(nomem, nostack, preserves_flags));
    value
}

#[inline(always)]
pub unsafe fn outw(port: u16, value: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") value,
         options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    asm!("in ax, dx", out("ax") value, in("dx") port,
         options(nomem, nostack, preserves_flags));
    value
}

#[inline(always)]
pub unsafe fn outl(port: u16, value: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") value,
         options(nomem, nostack, preserves_flags));
}

#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    asm!("in eax, dx", out("eax") value, in("dx") port,
         options(nomem, nostack, preserves_flags));
    value
}

/// Pequena pausa de E/S (usada con el PIC).
#[inline(always)]
pub unsafe fn io_wait() {
    outb(0x80, 0);
}

/// Ejecuta `CPUID` con `leaf`/`sub` y devuelve `(EAX, EBX, ECX, EDX)`.
///
/// Solo se usa para identificar el procesador (ver [`fabricante`] y [`marca`]).
#[inline(always)]
pub unsafe fn cpuid(leaf: u32, sub: u32) -> (u32, u32, u32, u32) {
    let (a, b, c, d);
    // `rbx` lo reserva LLVM (no admite restricciones sobre el), asi que se
    // intercambia con un registro libre antes y despues de `CPUID`.
    //
    // El temporal es `inout` a proposito: al ser una entrada ademas de salida, el
    // asignador le da un registro propio y no puede reutilizar `eax` ni `ecx`
    // (que llevan la hoja), cosa que si podria pasar con un `out` que se escribe
    // antes de leer las entradas.
    let mut _salvo: u64 = 0;
    asm!(
        "xchg {salvo}, rbx",
        "cpuid",
        "mov {b:e}, ebx",
        "xchg {salvo}, rbx",
        salvo = inout(reg) _salvo,
        inlateout("eax") leaf => a,
        b = lateout(reg) b,
        inlateout("ecx") sub => c,
        out("edx") d,
        options(nomem, nostack, preserves_flags),
    );
    (a, b, c, d)
}

/// Fabricante de la CPU: 12 bytes ASCII, p. ej. `GenuineIntel`.
pub fn fabricante() -> [u8; 12] {
    let (_, b, c, d) = unsafe { cpuid(0, 0) };
    let mut out = [0u8; 12];
    // El fabricante va en EBX, EDX, ECX (en ese orden).
    out[0..4].copy_from_slice(&b.to_le_bytes());
    out[4..8].copy_from_slice(&d.to_le_bytes());
    out[8..12].copy_from_slice(&c.to_le_bytes());
    out
}

/// Cadena de marca del procesador (48 bytes, rellena con espacios o nulos).
///
/// Vacia si la CPU no la soporta; entonces conviene usar [`fabricante`].
pub fn marca() -> [u8; 48] {
    let mut out = [0u8; 48];
    let max = unsafe { cpuid(0x8000_0000, 0) }.0;
    if max < 0x8000_0004 {
        return out;
    }
    for (i, leaf) in [0x8000_0002u32, 0x8000_0003, 0x8000_0004]
        .iter()
        .enumerate()
    {
        let (a, b, c, d) = unsafe { cpuid(*leaf, 0) };
        let base = i * 16;
        for (j, w) in [a, b, c, d].iter().enumerate() {
            out[base + j * 4..base + j * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
    out
}

#[inline(always)]
pub unsafe fn enable_interrupts() {
    asm!("sti", options(nomem, nostack));
}

/// ¿Estan las interrupciones activas ahora mismo? (bit `IF` de `RFLAGS`).
///
/// Lo usa la consola para no esperar por otra tarea cuando una espera no
/// podria terminar: con `IF` a cero no hay cambio de tarea y el que tiene el
/// turno de impresion no podria soltarlo.
#[inline(always)]
pub fn interrupciones_activas() -> bool {
    let flags: u64;
    // `pushfq`/`pop` deja `RFLAGS` en un registro sin tocar la pila de forma
    // asimetrica; `nomem` porque solo se lee un registro del procesador.
    unsafe {
        asm!("pushfq; pop {}", out(reg) flags, options(nomem));
    }
    flags & (1 << 9) != 0
}

/// Ejecuta `f` con las interrupciones desactivadas y restaura el estado previo.
///
/// Sirve para tocar estructuras compartidas con los manejadores de IRQ (por
/// ejemplo, el bufer circular del puerto serie) sin que una interrupcion se
/// cuele a mitad de la operacion.
#[inline(always)]
pub unsafe fn sin_interrupciones<F: FnOnce() -> R, R>(f: F) -> R {
    let flags: u64;
    // Guarda RFLAGS en un registro (sin tocar la pila de forma asimetrica) y
    // desactiva las interrupciones.
    asm!("pushfq; pop {}", out(reg) flags, options(nomem));
    asm!("cli", options(nomem, nostack));
    let resultado = f();
    if flags & (1 << 9) != 0 {
        asm!("sti", options(nomem, nostack));
    }
    resultado
}

/// Guardia de ambito que mantiene las interrupciones desactivadas y las
/// restaura al soltarse (Fase 2 del plan Marea).
///
/// Es la forma estructurada de [`sin_interrupciones`] para bloques largos: al
/// basarse en `Drop`, el estado previo de `IF` se restaura tambien en los
/// caminos de error (`?`, `return` dentro del bloque), que es lo que la forma
/// manual con `cli`/`sti` no garantiza. **No se puede usar** dentro de un
/// manejador de IRQ: las interrupciones ya estan tapadas alli y restaurarlas
/// abriera IRQ anidadas.
///
/// ```ignore
/// let _irq = IrqOff::nuevo();   // IF = 0 mientras viva la guardia
/// // ... seccion que no debe ser interrumpida ...
/// // IF se restaura aqui, pase lo que pase
/// ```
pub struct IrqOff {
    /// RFLAGS guardados al crear la guardia; se devuelven al soltarse.
    flags: u64,
    /// Tic del PIT al entrar (para la medicion de la Fase 2). 0 = sin medir.
    inicio: u64,
}

impl IrqOff {
    /// Desactiva las interrupciones y devuelve la guardia que las restaurara.
    ///
    /// # Seguridad
    /// El llamante no debe volver a habilitar interrupciones manualmente
    /// mientras viva la guardia (se restaurarian dos veces). Tampoco debe
    /// usarse dentro de un manejador de IRQ.
    #[inline(always)]
    pub unsafe fn nuevo() -> IrqOff {
        let flags: u64;
        asm!("pushfq; pop {}", out(reg) flags, options(nomem));
        asm!("cli", options(nomem, nostack));
        IrqOff { flags, inicio: 0 }
    }

    /// Igual que [`nuevo`], pero deja registrada la duracion en las metricas
    /// de la Fase 2 cuando se suelta la guardia.
    ///
    /// Usa el PIT (100 Hz, 10 ms por tic), ya calibrado y disponible; no toca
    /// TSC asi que no hay que comprobar CPUID. La resolucion es de 10 ms, de
    /// sobra para detectar secciones largas.
    ///
    /// # Seguridad
    /// Igual que [`nuevo`].
    #[inline(never)]
    pub unsafe fn medir() -> IrqOff {
        let mut g = IrqOff::nuevo();
        g.inicio = crate::pit::ticks();
        g
    }
}

impl Drop for IrqOff {
    #[inline(always)]
    fn drop(&mut self) {
        // Registra la duracion antes de restaurar IF (Fase 2).
        if self.inicio != 0 {
            let fin = crate::pit::ticks();
            crate::metricas::irq_off(fin.saturating_sub(self.inicio));
        }
        // Solo repone IF si el llamante la tenia encendida; si entraba con
        // interrupciones ya tapadas, no se abren aqui.
        if self.flags & (1 << 9) != 0 {
            unsafe { asm!("sti", options(nomem, nostack)) };
        }
    }
}

#[inline(always)]
pub unsafe fn hlt() {
    asm!("hlt", options(nomem, nostack));
}

/// Suspensión ligera: detiene el planificador y espera a que teclado o ratón
/// PS/2 generen una IRQ. No es ACPI S3; conserva la pantalla encendida y vuelve
/// al mismo punto al recibir la interrupción de despertar.
pub fn suspender() {
    crate::println!("rocky: suspendido; pulsa una tecla o mueve el raton para continuar.");
    unsafe {
        let flags: u64;
        asm!("pushfq; pop {}; cli", out(reg) flags, options(nomem));
        let mascara_maestro = inb(0x21);
        let mascara_esclavo = inb(0xA1);

        // Sin IRQ0 no hay turnos del PIT; solo se dejan pasar teclado IRQ1,
        // cascada IRQ2 y ratón PS/2 IRQ12 para poder despertar.
        outb(0x21, 0xF9);
        outb(0xA1, 0xEF);
        asm!("sti; hlt; cli", options(nomem, nostack));

        outb(0x21, mascara_maestro);
        outb(0xA1, mascara_esclavo);
        if flags & (1 << 9) != 0 {
            asm!("sti", options(nomem, nostack));
        }
    }
    crate::println!("rocky: reanudado.");
}

/// Reinicia la máquina por el controlador de chipset/8042 y, como último
/// recurso, provoca un triple fallo para solicitar el reset de la CPU.
pub fn reiniciar() -> ! {
    crate::println!("rocky: reiniciando equipo.");
    unsafe {
        asm!("cli", options(nomem, nostack));

        // Reset de plataforma usado por muchos chipsets x86 (incluido QEMU).
        outb(0xCF9, 0x02);
        io_wait();
        outb(0xCF9, 0x06);
        io_wait();

        // Pulso RESET tradicional del controlador de teclado si sigue activo.
        outb(0x64, 0xFE);
        io_wait();

        // IDT nula: INT3 provoca #GP y el fallo de entrega de #DF reinicia la CPU.
        let idt_nula = [0u8; 10];
        asm!("lidt [{}]", in(reg) idt_nula.as_ptr(), options(readonly, nostack));
        asm!("int3", options(nomem, nostack));
    }
    halt_loop()
}

/// Bucle de parada definitiva (tras un fallo irrecuperable).
pub fn halt_loop() -> ! {
    loop {
        unsafe { hlt() };
    }
}

/// Apaga la maquina.
///
/// Primero ACPI de verdad ([`crate::acpi::preparar`]): las tablas del firmware
/// dan el puerto de control y el `SLP_TYP` del estado S5, que es lo unico que
/// funciona cuando el chipset no vive en los puertos de siempre. Si el firmware
/// no publica ACPI utilizable, se prueba `SLP_EN` en los puertos `PM1a_CNT`
/// habituales y, en ultimo lugar, APM (`INT 15h` AX=5307h), que es lo que apaga
/// los PCs antiguos que arrancan por BIOS.
///
/// Devuelve `false` si la maquina sigue encendida (no debe apagarse a la
/// fuerza: el escritorio sigue vivo y en la Terminal queda el diagnostico).
pub fn shutdown() -> bool {
    // Se informa de lo que se va a intentar *antes* de tocar los puertos: si la
    // maquina no responde, en la pantalla queda el porque. Cuando funciona ya no
    // hay ocasion de escribir nada, asi que primero el mensaje.
    match crate::acpi::preparar() {
        Some(s5) => {
            crate::println!(
                "Apagando por ACPI: PM1a=0x{:04X} SLP_TYP={}{}.",
                s5.pm1a,
                s5.typ_a,
                if s5.del_dsdt { "" } else { " (por defecto)" }
            );
            crate::acpi::encender(&s5);
        }
        None => {
            crate::println!("Apagando: este firmware no publica ACPI utilizable.");
            // Plan B: `SLP_EN` en los puertos `PM1a_CNT` de siempre. Solo aqui:
            // si el firmware *si* publica ACPI, su puerto y su `SLP_TYP` son los
            // buenos, y escribir un 5 a ciegas podria ser otro estado (dormir).
            apagar_acpi();
        }
    }

    // Plan C: APM, el apagado clasico de la BIOS.
    match apagar_apm() {
        Some((mayor, menor)) => crate::println!("APM {}.{} tampoco responde.", mayor, menor),
        None => crate::println!("Este BIOS no tiene APM."),
    }
    crate::println!("No he podido apagar el equipo: pulsa el boton.");
    false
}

/// Enciende `SLP_EN` en los puertos `PM1a_CNT` habituales (plan B).
fn apagar_acpi() {
    // 0x604 PIIX/ICH (QEMU), 0xB004 Bochs, 0x4004 VirtualBox, 0x1804 y 0x8004
    // (chipsets Intel y AMD) y 0x404 (algunos ICH).
    const PUERTOS: [u16; 6] = [0x604, 0xB004, 0x4004, 0x1804, 0x8004, 0x404];
    for puerto in PUERTOS {
        for valor in [0x2000u16, 0x3400u16] {
            unsafe {
                outw(puerto, valor);
                io_wait();
            }
        }
    }
}

/// Apagado por APM (`INT 15h` AX=5307h).
///
/// Devuelve la version de APM `(mayor, menor)` si el BIOS la ofrece (y se le
/// pidio el apagado), o `None` si no hay APM en absoluto.
fn apagar_apm() -> Option<(u8, u8)> {
    use crate::bios::{self, Regs};
    unsafe {
        // La llamada al BIOS corre en modo real con IF encendido, y el PIC ya
        // esta reubicado en 0x20: una IRQ caeria en la tabla del firmware. Se
        // tapan todas mientras dura la llamada.
        let imr = inb(0x21);
        outb(0x21, 0xFF);

        // ¿Hay APM? (AX=5300h: el BIOS devuelve la version en AH:AL).
        let r = bios::int15(Regs {
            eax: 0x5300,
            ..Default::default()
        });
        let mayor = ((r.eax >> 8) & 0xFF) as u8;
        let menor = (r.eax & 0xFF) as u8;
        if r.carry || mayor == 0 {
            outb(0x21, imr);
            return None;
        }

        // Conectar la interfaz de 16 bits (AX=5301h, ES:BX = 0:0).
        let _ = bios::int15(Regs {
            eax: 0x5301,
            ..Default::default()
        });
        // Pedir el apagado (AX=5307h): BX=0001h (todo el equipo) y
        // CX=0003h (estado: apagado).
        let _ = bios::int15(Regs {
            eax: 0x5307,
            ebx: 0x0001,
            ecx: 0x0003,
            ..Default::default()
        });

        outb(0x21, imr);
        Some((mayor, menor))
    }
}
