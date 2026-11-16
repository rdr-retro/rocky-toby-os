//! rocky - un sistema operativo x86_64 escrito 100% en Rust.
//!
//! Kernel bare-metal para x86_64 en modo largo. No usa Linux ni Unix y el
//! diseÃ±o es propio: gestion de memoria fisica, un heap paginado, el sistema
//! de ficheros Estrato sobre un disco ATA y un interprete de ordenes.
//!
//! Arranque: el cargador BIOS prepara el modo largo, reubica la imagen desde
//! 0x10000 a 1 MiB y salta a `rust_main`.

#![no_std]
#![no_main]

extern crate alloc;

use core::arch::global_asm;
use core::panic::PanicInfo;

mod acpi;
mod asm;
mod bios;
mod cmos;
mod console;
mod cpu;
mod cursor;
mod demo;
mod disk;
mod disk_srv;
mod dock;
mod drivers;
mod espacio;
mod fat32;
mod fd;
mod fs;
mod fuentes;
mod gdt;
mod gfx;
mod heap;
mod idt;
mod inicio_apps;
mod input;
mod kira;
mod metricas;
mod mouse;
mod paging;
mod panico;
mod pantalla;
mod pci;
mod pic;
mod pipe;
mod pit;
mod pmm;
mod proceso;
mod ps2;
mod rok;
mod senal;
mod serial;
mod servicios;
mod shell;
mod sound;
mod sync;
mod syscall;
mod tarea;
mod texto;
mod toby_ui;
mod tty;
mod vga;
mod visor;
mod vm;

global_asm!(include_str!("start.s"));
global_asm!(include_str!("interrupts.s"));

/// El asignador global del kernel: heap propio sobre la ventana dinamica.
#[global_allocator]
static ALLOCATOR: heap::Heap = heap::Heap;

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    console::init();
    crate::println!("rocky: cargando kernel (x86_64, modo largo)...");
    let _cpu = cpu::init();

    idt::init();

    // Los servicios del BIOS (mapa de memoria, VESA...) se piden ANTES de tocar
    // el PIC ni el PIT. En cuanto el kernel reubica el PIC en 0x20 y reprograma
    // el temporizador, un manejador del firmware que reactive las interrupciones
    // saltaria a la entrada de la IVT equivocada (las IRQ del BIOS viven en 0x08
    // y las nuestras en 0x20), y en un PC real eso cuelga la maquina. Dejando el
    // PIC y el PIT tal y como los dejo el firmware, el firmware sigue siendo
    // coherente aunque encienda las interrupciones por su cuenta.
    //
    // (Los mensajes "[n/4]" siguen el orden de arranque y sirven para saber
    //  donde se queda si alguna vez falla en un PC real.)
    crate::println!("rocky: [1/4] trampolin del BIOS y mapa de memoria...");
    unsafe { bios::init() };
    pmm::init();
    // Con el trampolin y la memoria fisica en marcha, se barre el bus PCI: da el
    // identificador de la maquina (y de la GPU) y deja los BAR listos para las
    // fases de aceleracion.
    pci::init();
    // Con los BAR ya localizados, se arrancan los controladores: hoy solo hay
    // GPU (fase 14), que deja abierta la ventana de registros de la tarjeta,
    // marcada como no cacheable, para leerla y para el cursor por hardware.
    drivers::init();

    // Con la GDT del BIOS ya cargada (`rocky_gdt`), se completan los segmentos
    // de usuario (0x38/0x40) y el TSS (0x48), y se carga el TSS con `ltr`. A
    // partir de aqui el nucleo puede saltar al anillo 3 y volver.
    gdt::init();

    crate::println!("rocky: [2/4] paginacion del kernel...");
    paging::init();

    crate::println!("rocky: [3/4] heap del kernel...");
    heap::init();
    // Con el heap en marcha ya se puede reservar la tabla de referencias de
    // marcos, que sostiene la copia-al-escribir de la memoria virtual (Fase 5).
    pmm::init_refs();
    crate::println!(
        "rocky: memoria {} KiB libres de {} KiB (heap {} KiB).",
        pmm::free_bytes() / 1024,
        pmm::total_bytes() / 1024,
        heap::mapped_bytes() / 1024
    );

    crate::println!("rocky: [4/4] temporizador e interrupciones...");
    // La tabla de ficheros abiertos arranca con la consola (descriptores 0/1/2).
    fd::init();
    unsafe {
        pic::init();
        pit::init(100);
        // A partir de aqui el reloj reparte el turno entre tareas. Mientras
        // solo exista la tarea del nucleo, el planificador no cambia nada.
        tarea::init();
        cpu::enable_interrupts();
    }
    // A partir de aqui el PIC y el PIT son del kernel: cada llamada al BIOS los
    // volvera a dejar como estan (el firmware los reprograma, sobre todo al
    // activar el modo VESA, y sin esto la IRQ0 puede dejar de llegar).
    bios::tomar_interrupciones();
    crate::println!("rocky: temporizador e interrupciones activos.");

    // Fase 1 Marea: smoke-test de invariantes del planificador. Se ejecuta
    // una vez al arranque para verificar que las transiciones de estado son
    // correctas antes de arrancar la GUI. Los resultados van por serial.
    crate::println!("rocky: probando invariantes del planificador...");
    match crate::tarea::test_plan() {
        Ok(n) => crate::println!("rocky: test_plan OK ({} pruebas).", n),
        Err(msg) => crate::println!("rocky: test_plan FALLO: {}", msg),
    }

    // Fase 2 USB: arrancar la inicializacion del controlador xHCI en segundo
    // plano (con plazos por paso). PS/2 y el puerto serie siguen siendo la
    // entrada del sistema hasta que haya un HID funcional.
    crate::drivers::usb::arrancar();

    shell::main_loop();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    console::panic_report(info)
}
