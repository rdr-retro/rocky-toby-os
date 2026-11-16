//! Procesos de usuario: espacio de direcciones propio y programa en anillo 3.
//!
//! Este es el primer escalon de los "cuatro grandes" que le faltaban a rocky
//! frente a Linux 0.01: la **frontera usuario/nucleo**. Un proceso tiene:
//!
//! * su **propia PML4** (su propio espacio de direcciones), que copia las
//!   entradas del nucleo (la identidad y la ventana del heap) para que el anillo
//!   0 siga viendose igual mientras el proceso corre;
//! * una **ventana de usuario** con paginas de 4 KiB marcadas como accesibles
//!   desde el anillo 3 (`U/S = 1`), donde viven su codigo y su pila;
//! * un **programa**: unos pocos bytes de maquina que piden las llamadas al
//!   sistema (`int 0x80`) y terminan.
//!
//! Todo lo demas (identidad 0..4 GiB y ventana del heap) queda con `U/S = 0`,
//! asi que en anillo 3 solo se puede tocar la ventana propia: la separacion es
//! real, no un simple cambio de registro de segmento.
//!
//! La segunda pieza (Fase 2) es **duplicar** un proceso: [`Proceso::clonar`]
//! reserva marcos nuevos y copia byte a byte su ventana, de modo que el hijo de
//! `fork` tiene memoria propia aunque vea las mismas direcciones virtuales que
//! el padre.

use core::arch::asm;

use alloc::vec::Vec;

use crate::paging;
use crate::pmm;

/// Base de la ventana de usuario: 512 GiB (PML4[1]). Fuera de los 4 GiB que el
/// sector de arranque mapea por identidad con paginas de 2 MiB.
pub const USER_BASE: usize = 0x0000_0080_0000_0000;
/// Tamano de la ventana de usuario (1 GiB, dentro de la misma entrada de PML4).
///
/// La ventana no se mapea entera: solo las paginas que el proceso toca. El
/// **codigo** crece desde `USER_BASE`, el **heap** (`brk`) le sigue y la **pila**
/// vive pegada al tope, creciendo hacia abajo.
pub const USER_BYTES: usize = 1 << 30;
/// Tope de la ventana de usuario (exclusivo).
pub const USER_TOP: usize = USER_BASE + USER_BYTES;
/// Tamano maximo de la pila (8 MiB), contando desde el tope.
pub const PILA_MAX: usize = 8 << 20;
/// Tamano maximo del ejecutable (16 MiB).
pub const CODIGO_MAX: usize = 16 << 20;

/// Tipo de relocalizacion `R_X86_64_RELATIVE`: el enlazador PIC la usa para
/// decirle al cargador que valor debe poner en cada casilla del GOT.
const R_X86_64_RELATIVE: u32 = 8;

/// Bits de direccion fisica dentro de una entrada de pagina.
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Espacio de direcciones de un proceso. Al soltarse libera todos los marcos
/// que reservo (la PML4, las tablas intermedias y las paginas de usuario).
pub struct Proceso {
    /// PML4 propia del proceso.
    pub pml4: usize,
    /// Marcos de las tablas (la PML4 y los niveles intermedios): se liberan con
    /// [`crate::pmm::free_frame`].
    tablas: Vec<usize>,
    /// Paginas de usuario mapeadas, como `(direccion virtual, marco fisico)`.
    /// Pueden estar **compartidas** con otro proceso (copia-al-escribir): se
    /// sueltan con [`crate::pmm::frame_release`], no con `free_frame`.
    paginas: Vec<(usize, usize)>,
    /// Fin del segmento de codigo (direccion virtual absoluta).
    codigo_fin: usize,
    /// Inicio del heap (el `brk` minimo).
    heap_base: usize,
    /// Fin actual del heap (`brk`). Las paginas hasta aqui se crean bajo demanda.
    brk: usize,
    /// Tope de la pila (crece hacia abajo).
    pila_tope: usize,
}

#[inline]
fn entrada(tabla: usize, i: usize) -> u64 {
    unsafe { core::ptr::read_volatile((tabla + i * 8) as *const u64) }
}

#[inline]
fn poner(tabla: usize, i: usize, valor: u64) {
    unsafe { core::ptr::write_volatile((tabla + i * 8) as *mut u64, valor) };
}

/// Indica si `[buf, buf+len)` cabe entero dentro de la ventana de usuario.
///
/// Lo usan las llamadas al sistema para no leer ni escribir memoria del nucleo
/// a peticion de un programa de anillo 3. **No** garantiza que las paginas esten
/// mapeadas: de eso se encarga [`poblar_rango`], que las trae bajo demanda.
pub fn puntero_valido(buf: u64, len: u64) -> bool {
    let inicio = USER_BASE as u64;
    buf >= inicio && buf.checked_add(len).map_or(false, |d| d <= USER_TOP as u64)
}

/// Trae a memoria la pagina que contiene `virt` si pertenece a una region
/// legitima del proceso en curso (codigo, heap por debajo de `brk` o pila), y
/// devuelve si quedo accesible. El nucleo la usa antes de tocar memoria del
/// usuario desde una llamada al sistema.
pub fn poblar(virt: u64) -> bool {
    let v = virt as usize;
    if v < USER_BASE || v >= USER_TOP {
        return false;
    }
    crate::tarea::con_proceso(|p| p.poblar(v)).unwrap_or(false)
}

/// Como [`poblar`], para todo el rango `[buf, buf+len)`.
pub fn poblar_rango(buf: u64, len: u64) -> bool {
    let inicio = buf as usize;
    let fin = match inicio.checked_add(len as usize) {
        Some(f) => f,
        None => return false,
    };
    if inicio < USER_BASE || fin > USER_TOP {
        return false;
    }
    let mut p = inicio & !(pmm::FRAME_SIZE - 1);
    while p < fin {
        if !poblar(p as u64) {
            return false;
        }
        p += pmm::FRAME_SIZE;
    }
    true
}

/// Como [`poblar`], pero garantiza ademas que la pagina se pueda **escribir**:
/// si esta compartida tras `fork` (copia-al-escribir) se rompe la copia. La usa
/// el nucleo antes de dejar datos en memoria del usuario.
pub fn poblar_escritura(virt: u64) -> bool {
    let v = virt as usize;
    if v < USER_BASE || v >= USER_TOP {
        return false;
    }
    crate::tarea::con_proceso(|p| p.poblar_escritura(v)).unwrap_or(false)
}

/// Como [`poblar_escritura`], para todo el rango `[buf, buf+len)`.
pub fn poblar_rango_escritura(buf: u64, len: u64) -> bool {
    let inicio = buf as usize;
    let fin = match inicio.checked_add(len as usize) {
        Some(f) => f,
        None => return false,
    };
    if inicio < USER_BASE || fin > USER_TOP {
        return false;
    }
    let mut p = inicio & !(pmm::FRAME_SIZE - 1);
    while p < fin {
        if !poblar_escritura(p as u64) {
            return false;
        }
        p += pmm::FRAME_SIZE;
    }
    true
}

/// Atiende un fallo de pagina (#PF) de la tarea en curso.
///
/// `error` es el codigo que empuja el procesador. Devuelve `true` si el fallo se
/// resolvio (copia-al-escribir o pagina bajo demanda) y hay que reintentar la
/// instruccion; `false` si es un fallo grave (lo informa la IDT).
pub fn fallo_pagina(error: u64) -> bool {
    // El bit USER (2) del codigo **no** basta para descartar el fallo: el nucleo
    // tambien lee y escribe la memoria del usuario desde anillo 0 (por ejemplo,
    // al montar el bloque de argumentos de `execve`). Quien decide si el fallo es
    // legitimo es la region a la que apunta `cr2`: `Proceso::tratar_fallo` solo
    // lo atiende si cae en el heap o la pila del proceso en curso; cualquier otra
    // direccion (memoria del nucleo, procesos ajenos) sigue siendo un fallo grave.
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)) };
    crate::tarea::con_proceso(|p| p.tratar_fallo(cr2 as usize, error)).unwrap_or(false)
}

impl Proceso {
    /// Crea el espacio de direcciones de un proceso nuevo.
    ///
    /// Reserva la PML4 del proceso y copia las 512 entradas de la PML4 del
    /// nucleo: asi el proceso comparte la identidad de la memoria baja y la
    /// ventana del heap (donde viven las estructuras del propio kernel), y el
    /// anillo 0 funciona igual con la PML4 del proceso cargada.
    pub fn nuevo() -> Proceso {
        let pml4 = pmm::alloc_frame().expect("proceso: sin marco para la PML4");
        pmm::zero(pml4, pmm::FRAME_SIZE);
        unsafe {
            let src = paging::PML4_NUCLEO as *const u64;
            let dst = pml4 as *mut u64;
            for i in 0..512 {
                core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
            }
        }
        Proceso {
            pml4,
            tablas: alloc::vec![pml4],
            paginas: Vec::new(),
            codigo_fin: USER_BASE,
            heap_base: USER_BASE,
            brk: USER_BASE,
            pila_tope: USER_TOP,
        }
    }

    /// Reserva y pone a cero un marco de **tabla**, apuntandolo para liberarlo al
    /// morir.
    fn marco_tabla(&mut self) -> usize {
        let f = pmm::alloc_frame().expect("proceso: sin marcos libres");
        pmm::zero(f, pmm::FRAME_SIZE);
        self.tablas.push(f);
        f
    }

    /// Reserva una **pagina de usuario**: ademas de apuntarla para liberarla al
    /// morir, suma su referencia (para la copia-al-escribir).
    fn marco_pagina(&mut self) -> usize {
        let f = pmm::alloc_frame().expect("proceso: sin marcos libres");
        pmm::zero(f, pmm::FRAME_SIZE);
        pmm::frame_retain(f);
        f
    }

    /// Devuelve la tabla siguiente bajo `tabla[i]`, creandola si falta.
    fn tabla(&mut self, tabla: usize, i: usize) -> usize {
        let e = entrada(tabla, i);
        if e & paging::PRESENT != 0 {
            return (e & ADDR_MASK) as usize;
        }
        let f = self.marco_tabla();
        poner(
            tabla,
            i,
            f as u64 | paging::PRESENT | paging::WRITABLE | paging::USER,
        );
        f
    }

    /// Baja por la jerarquia de tablas hasta la tabla de paginas de `virt`,
    /// creando los niveles que falten.
    fn tabla_para(&mut self, virt: usize) -> usize {
        let pml4 = self.pml4;
        let p3 = self.tabla(pml4, (virt >> 39) & 0x1FF);
        let p2 = self.tabla(p3, (virt >> 30) & 0x1FF);
        self.tabla(p2, (virt >> 21) & 0x1FF)
    }

    /// Tabla de paginas de `virt` si la jerarquia ya existe (sin crearla).
    fn tabla_de(&self, virt: usize) -> Option<usize> {
        let e4 = entrada(self.pml4, (virt >> 39) & 0x1FF);
        if e4 & paging::PRESENT == 0 {
            return None;
        }
        let p3 = (e4 & ADDR_MASK) as usize;
        let e3 = entrada(p3, (virt >> 30) & 0x1FF);
        if e3 & paging::PRESENT == 0 {
            return None;
        }
        let p2 = (e3 & ADDR_MASK) as usize;
        let e2 = entrada(p2, (virt >> 21) & 0x1FF);
        if e2 & paging::PRESENT == 0 {
            return None;
        }
        Some((e2 & ADDR_MASK) as usize)
    }

    /// Entrada de pagina de `virt`, si la jerarquia existe.
    fn pte(&self, virt: usize) -> Option<u64> {
        self.tabla_de(virt)
            .map(|t| entrada(t, (virt >> 12) & 0x1FF))
    }

    /// Indica si la pagina de `virt` esta mapeada.
    fn presente(&self, virt: usize) -> bool {
        self.pte(virt).map_or(false, |e| e & paging::PRESENT != 0)
    }

    /// Escribe la entrada de pagina de `virt` (creando la jerarquia si falta) e
    /// invalida la traduccion que hubiera en la TLB.
    fn fijar(&mut self, virt: usize, valor: u64) {
        let t = self.tabla_para(virt);
        poner(t, (virt >> 12) & 0x1FF, valor);
        paging::flush(virt);
    }

    /// Mapea la pagina `virt -> phys` con `flags` y la registra.
    fn mapear(&mut self, virt: usize, phys: usize, flags: u64) {
        self.fijar(virt, phys as u64 | flags);
        if let Some(p) = self.paginas.iter_mut().find(|p| p.0 == virt) {
            p.1 = phys;
        } else {
            self.paginas.push((virt, phys));
        }
    }

    /// Trae a memoria la pagina de `virt` si es una region legitima (heap por
    /// debajo de `brk`, o pila). Devuelve si quedo accesible.
    pub fn poblar(&mut self, virt: usize) -> bool {
        let pagina = virt & !(pmm::FRAME_SIZE - 1);
        if self.presente(pagina) {
            return true;
        }
        let en_heap = pagina >= self.heap_base && pagina < self.brk;
        let en_pila = pagina >= self.pila_tope.saturating_sub(PILA_MAX) && pagina < self.pila_tope;
        if !en_heap && !en_pila {
            return false;
        }
        let f = match pmm::alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        pmm::zero(f, pmm::FRAME_SIZE);
        pmm::frame_retain(f);
        self.mapear(pagina, f, paging::PRESENT | paging::WRITABLE | paging::USER);
        true
    }

    /// Como [`Proceso::poblar`], pero dejando ademas la pagina **escribible**:
    /// si quedo compartida tras `fork` (copia-al-escribir) se rompe la copia. La
    /// usa el nucleo antes de escribir en memoria del usuario.
    pub fn poblar_escritura(&mut self, virt: usize) -> bool {
        if !self.poblar(virt) {
            return false;
        }
        let pagina = virt & !(pmm::FRAME_SIZE - 1);
        if let Some(e) = self.pte(pagina) {
            if e & paging::COW != 0 {
                // "presente + escritura": `tratar_fallo` rompe la copia.
                return self.tratar_fallo(pagina, 0b011);
            }
        }
        true
    }

    /// Resuelve un fallo de pagina: copia-al-escribir (si la pagina estaba
    /// compartida) o traida bajo demanda (si aun no existia).
    pub fn tratar_fallo(&mut self, virt: usize, error: u64) -> bool {
        let presente = error & 1 != 0;
        let escritura = error & 2 != 0;
        if !presente {
            return self.poblar(virt);
        }
        if !escritura {
            return false; // lectura de una pagina presente que no es nuestra
        }
        let pagina = virt & !(pmm::FRAME_SIZE - 1);
        let viejo = match self.pte(pagina) {
            Some(e) if e & paging::COW != 0 => (e & ADDR_MASK) as usize,
            _ => return false, // no es copia-al-escribir
        };
        if pmm::frame_refs(viejo) <= 1 {
            // Ya no lo comparte nadie: basta con permitir la escritura.
            let flags = (self.pte(pagina).unwrap_or(0) | paging::WRITABLE) & !paging::COW;
            self.fijar(pagina, viejo as u64 | (flags & !ADDR_MASK));
            return true;
        }
        let nuevo = match pmm::alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        unsafe {
            core::ptr::copy_nonoverlapping(viejo as *const u8, nuevo as *mut u8, pmm::FRAME_SIZE);
        }
        pmm::frame_retain(nuevo);
        pmm::frame_release(viejo);
        self.mapear(
            pagina,
            nuevo,
            paging::PRESENT | paging::WRITABLE | paging::USER,
        );
        true
    }

    /// Ajusta el fin del heap (`brk`). Devuelve el `brk` resultante.
    pub fn ajustar_brk(&mut self, addr: usize) -> i64 {
        if addr == 0 {
            return self.brk as i64;
        }
        let tope = self.pila_tope.saturating_sub(PILA_MAX);
        if addr < self.heap_base || addr > tope {
            return self.brk as i64; // peticion fuera de rango: sin cambios
        }
        if addr < self.brk {
            // Encoge: las paginas que quedan por encima se devuelven.
            let desde = (addr + pmm::FRAME_SIZE - 1) & !(pmm::FRAME_SIZE - 1);
            let mut p = desde;
            while p < self.brk {
                self.desmapear(p);
                p += pmm::FRAME_SIZE;
            }
        }
        self.brk = addr;
        self.brk as i64
    }

    /// Desmapea una pagina de usuario (si existe) y suelta su marco.
    fn desmapear(&mut self, virt: usize) {
        let e = match self.pte(virt) {
            Some(e) if e & paging::PRESENT != 0 => e,
            _ => return,
        };
        self.fijar(virt, 0);
        let phys = (e & ADDR_MASK) as usize;
        if let Some(pos) = self.paginas.iter().position(|p| p.0 == virt) {
            self.paginas.swap_remove(pos);
        }
        pmm::frame_release(phys);
    }

    /// Duplica el espacio de direcciones (`fork`) con **copia-al-escribir**.
    ///
    /// Ni el padre ni el hijo copian nada: sus tablas apuntan a los mismos
    /// marcos, marcados de solo lectura y con el bit [`paging::COW`]. El primero
    /// que intente escribir una pagina se llevara una copia privada (ver
    /// [`Proceso::tratar_fallo`]).
    pub fn clonar(&mut self) -> Option<Proceso> {
        let mut nuevo = Proceso::nuevo();
        nuevo.codigo_fin = self.codigo_fin;
        nuevo.heap_base = self.heap_base;
        nuevo.brk = self.brk;
        nuevo.pila_tope = self.pila_tope;

        let paginas: Vec<(usize, usize)> = self.paginas.clone();
        for (virt, phys) in paginas {
            // El hijo comparte el marco (solo lectura + COW)...
            pmm::frame_retain(phys);
            nuevo.mapear(virt, phys, paging::PRESENT | paging::USER | paging::COW);
            // ...y en el padre se quita la escritura y se marca COW.
            self.fijar(
                virt,
                phys as u64 | paging::PRESENT | paging::USER | paging::COW,
            );
        }
        Some(nuevo)
    }

    /// Reemplaza la **imagen** del proceso (`execve`): libera las paginas de
    /// usuario del programa anterior y monta `prog` como codigo nuevo, con una
    /// pila nueva.
    ///
    /// La PML4 y las tablas intermedias se **conservan** (el proceso sigue con
    /// la misma CR3 ya cargada); solo cambia lo que hay mapeado en su ventana.
    /// Los marcos que se liberan dejan su entrada vacia antes de volver al
    /// gestor, para que un despiste no deje accesible memoria reutilizada.
    pub fn ejecutar(&mut self, prog: &[u8]) -> Option<(usize, usize)> {
        if prog.is_empty() || prog.len() > CODIGO_MAX {
            return None;
        }
        // Libera las paginas de usuario del programa anterior.
        let viejas: Vec<(usize, usize)> = self.paginas.drain(..).collect();
        for (virt, phys) in viejas {
            let t = self.tabla_para(virt);
            poner(t, (virt >> 12) & 0x1FF, 0);
            paging::flush(virt);
            pmm::frame_release(phys);
        }
        // Detectar formato: ELF64 si empieza por 0x7F 'E' 'L' 'F'
        if prog.len() >= 4
            && prog[0] == 0x7F
            && prog[1] == b'E'
            && prog[2] == b'L'
            && prog[3] == b'F'
        {
            return self.cargar_elf(prog);
        }
        self.cargar(prog)
    }

    /// Monta `prog` como segmento de codigo (todas sus paginas) y fija el inicio
    /// del heap y el tope de pila. Devuelve `(entrada, tope de pila)`.
    ///
    /// La pila **no** se reserva aqui: se trae bajo demanda (el programa al
    /// tocarla, o el nucleo via [`Proceso::poblar_escritura`] al montar `argv`).
    fn cargar(&mut self, prog: &[u8]) -> Option<(usize, usize)> {
        let n = (prog.len() + pmm::FRAME_SIZE - 1) / pmm::FRAME_SIZE;
        for i in 0..n {
            let f = self.marco_pagina();
            let ini = i * pmm::FRAME_SIZE;
            let fin = (ini + pmm::FRAME_SIZE).min(prog.len());
            unsafe {
                core::ptr::copy_nonoverlapping(prog.as_ptr().add(ini), f as *mut u8, fin - ini);
            }
            self.mapear(
                USER_BASE + ini,
                f,
                paging::PRESENT | paging::WRITABLE | paging::USER,
            );
        }
        self.codigo_fin = USER_BASE + prog.len();
        self.heap_base = (self.codigo_fin + pmm::FRAME_SIZE - 1) & !(pmm::FRAME_SIZE - 1);
        self.brk = self.heap_base;
        self.pila_tope = USER_TOP;

        // La pila se trae **bajo demanda**, como el resto: el nucleo asegura las
        // paginas del bloque de argumentos antes de escribir en ellas (ver
        // `tarea::ejecutar`) y el programa nuevo trae las suyas al tocarlas.
        Some((USER_BASE, self.pila_tope - 16))
    }

    #[inline]
    fn u16le(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    #[inline]
    fn u32le(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }
    #[inline]
    fn u64le(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes([
            b[o],
            b[o + 1],
            b[o + 2],
            b[o + 3],
            b[o + 4],
            b[o + 5],
            b[o + 6],
            b[o + 7],
        ])
    }

    /// Carga un ejecutable **ELF64** en la ventana de usuario, como haria un
    /// cargador de Linux: mira los **program headers**, no las secciones.
    ///
    /// Acepta cualquier `e_type` (el `rust-lld` de Windows, por ejemplo, emite
    /// `ET_DYN` aunque se enlace `-static`): lo que importa es que los `PT_LOAD`
    /// caigan dentro de la ventana y que quepan. Como los binarios salen en
    /// modelo PIC, el ultimo paso aplica sus `R_X86_64_RELATIVE` (ver
    /// [`Proceso::relocalizar`]); no se enlaza nada dinamicamente.
    ///
    /// Varios segmentos pueden **compartir pagina** (texto, rodata y datos del
    /// mismo binario caben juntos). Por eso se mapea primero la envolvente
    /// completa —una pagina a cero por direccion— y despues se copia la parte de
    /// fichero de cada segmento en su sitio.
    fn cargar_elf(&mut self, elf: &[u8]) -> Option<(usize, usize)> {
        // ELF64, little-endian, x86-64.
        if elf.len() < 64 || elf[4] != 2 || elf[5] != 1 {
            return None;
        }
        let e_entry = Self::u64le(elf, 24) as usize;
        let e_phoff = Self::u64le(elf, 32) as usize;
        let e_phentsize = Self::u16le(elf, 54) as usize;
        let e_phnum = Self::u16le(elf, 56) as usize;
        if e_phentsize != 56 || e_phoff + e_phnum * e_phentsize > elf.len() {
            return None;
        }

        // 1) Envolvente virtual de todos los PT_LOAD.
        let mut vmin = usize::MAX;
        let mut vmax = 0usize;
        for i in 0..e_phnum {
            let off = e_phoff + i * e_phentsize;
            if Self::u32le(elf, off) != 1 {
                continue; // solo PT_LOAD
            }
            let p_offset = Self::u64le(elf, off + 8) as usize;
            let p_vaddr = Self::u64le(elf, off + 16) as usize;
            let p_filesz = Self::u64le(elf, off + 32) as usize;
            let p_memsz = Self::u64le(elf, off + 40) as usize;
            if p_filesz > p_memsz || p_offset + p_filesz > elf.len() {
                return None;
            }
            if p_memsz == 0 {
                continue;
            }
            if p_vaddr < USER_BASE || p_vaddr.checked_add(p_memsz)? > USER_TOP {
                return None;
            }
            let ini = p_vaddr & !(pmm::FRAME_SIZE - 1);
            let fin = (p_vaddr + p_memsz + pmm::FRAME_SIZE - 1) & !(pmm::FRAME_SIZE - 1);
            if ini < vmin {
                vmin = ini;
            }
            if fin > vmax {
                vmax = fin;
            }
        }
        if vmax <= vmin || vmax - vmin > CODIGO_MAX {
            return None;
        }
        if e_entry < USER_BASE || e_entry >= vmax {
            return None;
        }

        // 2) Mapea la envolvente entera, una pagina a cero por direccion.
        let mut v = vmin;
        while v < vmax {
            let f = self.marco_pagina(); // ya viene a cero
            self.mapear(v, f, paging::PRESENT | paging::WRITABLE | paging::USER);
            v += pmm::FRAME_SIZE;
        }

        // 3) Copia la parte de fichero de cada segmento a su direccion virtual
        //    (escribiendo en el marco fisico, porque el nucleo corre con su
        //    propia PML4 y no ve la ventana de usuario).
        for i in 0..e_phnum {
            let off = e_phoff + i * e_phentsize;
            if Self::u32le(elf, off) != 1 {
                continue;
            }
            let p_offset = Self::u64le(elf, off + 8) as usize;
            let p_vaddr = Self::u64le(elf, off + 16) as usize;
            let p_filesz = Self::u64le(elf, off + 32) as usize;
            let mut restante = p_filesz;
            let mut va = p_vaddr;
            let mut so = p_offset;
            while restante > 0 {
                let pagina = va & !(pmm::FRAME_SIZE - 1);
                let dentro = va - pagina;
                let trozo = (pmm::FRAME_SIZE - dentro).min(restante);
                let phys = match self.paginas.iter().find(|p| p.0 == pagina) {
                    Some(p) => p.1,
                    None => return None,
                };
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        elf.as_ptr().add(so),
                        (phys + dentro) as *mut u8,
                        trozo,
                    );
                }
                va += trozo;
                so += trozo;
                restante -= trozo;
            }
        }

        self.codigo_fin = vmax;
        self.heap_base = vmax; // la envolvente ya queda alineada a pagina
        self.brk = self.heap_base;
        self.pila_tope = USER_TOP;

        // 4) Relocalizaciones del binario PIC. Sin este paso, las casillas del
        //    GOT quedan a cero y el programa salta a la direccion 0 en cuanto
        //    llama a `memset`, `memcmp` o a cualquier direccion de un `static`.
        self.relocalizar(elf, e_phoff, e_phnum, e_phentsize)?;

        Some((e_entry, self.pila_tope - 16))
    }

    /// Aplica las relocalizaciones `R_X86_64_RELATIVE` de un ELF estatico PIC.
    ///
    /// `rust-lld` enlaza los binarios de usuario como `ET_DYN` con
    /// `-Crelocation-model=pic`: las llamadas a `memset`, `memcmp` o a la
    /// direccion de cualquier `static` pasan por el GOT, y cada casilla trae su
    /// `R_X86_64_RELATIVE` diciendo que valor debe llevar. Esto **no** es
    /// enlazado dinamico (no hay `PT_INTERP`, `.so`, `PLT` ni relocalizaciones
    /// de simbolos): es lo minimo que le falta al cargador para que arranque un
    /// ejecutable estatico PIC. El enlazado dinamico de verdad queda para una
    /// fase aparte.
    ///
    /// Los segmentos se cargan en su propia `p_vaddr`, sin sesgo, de modo que
    /// `R_X86_64_RELATIVE` se resuelve poniendo el propio `addend`.
    ///
    /// Un tipo de relocalizacion que no se entienda **rechaza el programa**
    /// entero (`None`): mas vale no arrancar que correr con una direccion a
    /// cero y morir con un fallo de pagina imposible de explicar.
    fn relocalizar(&self, elf: &[u8], phoff: usize, phnum: usize, phentsize: usize) -> Option<()> {
        // 1) Localizar PT_DYNAMIC dentro del fichero.
        let mut d_off = 0usize;
        let mut d_len = 0usize;
        for i in 0..phnum {
            let off = phoff + i * phentsize;
            if Self::u32le(elf, off) == 2 {
                // PT_DYNAMIC
                d_off = Self::u64le(elf, off + 8) as usize;
                d_len = Self::u64le(elf, off + 32) as usize;
                break;
            }
        }
        if d_len == 0 {
            return Some(()); // sin `.dynamic` no hay nada que relocalizar
        }
        if d_off.checked_add(d_len)? > elf.len() {
            return None;
        }

        // 2) Recorrer las etiquetas hasta DT_NULL buscando las tablas.
        let mut rela = 0usize;
        let mut relasz = 0usize;
        let mut relaent = 24usize;
        let mut rel = 0usize;
        let mut relsz = 0usize;
        let mut relent = 16usize;
        let mut o = d_off;
        while o + 16 <= d_off + d_len {
            let tag = Self::u64le(elf, o) as i64;
            let val = Self::u64le(elf, o + 8) as usize;
            o += 16;
            match tag {
                0 => break,         // DT_NULL
                7 => rela = val,    // DT_RELA
                8 => relasz = val,  // DT_RELASZ
                9 => relaent = val, // DT_RELAENT
                17 => rel = val,    // DT_REL
                18 => relsz = val,  // DT_RELSZ
                19 => relent = val, // DT_RELENT
                _ => {}
            }
        }

        // 3) Las tablas estan en direcciones virtuales: hay que traducirlas a
        //    desplazamiento dentro del fichero por los `PT_LOAD`.
        if rela != 0 && relasz != 0 {
            if relaent < 24 || relasz % relaent != 0 {
                return None;
            }
            let ini = Self::va_a_off(elf, phoff, phnum, phentsize, rela as u64, relasz)?;
            for k in 0..relasz / relaent {
                let e = ini + k * relaent;
                let destino = Self::u64le(elf, e) as usize;
                let tipo = (Self::u64le(elf, e + 8) & 0xFFFF_FFFF) as u32;
                let addend = Self::u64le(elf, e + 16);
                if tipo != R_X86_64_RELATIVE {
                    return None; // relocalizar un simbolo no es enlazar en caliente
                }
                // Una escritura de 8 bytes alineada nunca cruza de marco; una
                // tabla malformada con el offset desalineado, si.
                if destino & 7 != 0 {
                    return None;
                }
                unsafe {
                    *(self.dir_fisica(destino)? as *mut u64) = addend;
                }
            }
        }
        // Las entradas `REL` (sin `addend`) guardan el valor ya en memoria, y
        // al cargar sin sesgo no hay nada que sumar: solo se valida el tipo.
        if rel != 0 && relsz != 0 {
            if relent < 16 || relsz % relent != 0 {
                return None;
            }
            let ini = Self::va_a_off(elf, phoff, phnum, phentsize, rel as u64, relsz)?;
            for k in 0..relsz / relent {
                let e = ini + k * relent;
                if (Self::u64le(elf, e + 8) & 0xFFFF_FFFF) as u32 != R_X86_64_RELATIVE {
                    return None;
                }
            }
        }
        Some(())
    }

    /// Traduce la direccion virtual `va` (dentro de un `PT_LOAD`) a su
    /// desplazamiento correspondiente dentro del fichero ELF.
    fn va_a_off(
        elf: &[u8],
        phoff: usize,
        phnum: usize,
        phentsize: usize,
        va: u64,
        tam: usize,
    ) -> Option<usize> {
        for i in 0..phnum {
            let off = phoff + i * phentsize;
            if Self::u32le(elf, off) != 1 {
                continue; // solo PT_LOAD
            }
            let p_offset = Self::u64le(elf, off + 8);
            let p_vaddr = Self::u64le(elf, off + 16);
            let p_filesz = Self::u64le(elf, off + 32);
            let fin = p_vaddr.checked_add(p_filesz)?;
            if va >= p_vaddr && va.checked_add(tam as u64)? <= fin {
                return Some((p_offset + (va - p_vaddr)) as usize);
            }
        }
        None
    }

    /// Direccion **fisica** (marco mas desplazamiento) donde esta la direccion
    /// virtual `va`, que tiene que estar ya mapeada.
    ///
    /// El nucleo corre con su propia PML4 y no ve la ventana de usuario, asi
    /// que todo lo que el cargador escribe va por aqui.
    fn dir_fisica(&self, va: usize) -> Option<usize> {
        let pagina = va & !(pmm::FRAME_SIZE - 1);
        self.paginas
            .iter()
            .find(|p| p.0 == pagina)
            .map(|p| p.1 + (va - pagina))
    }
}

/// Crea un proceso de anillo 3 a partir de los bytes de un **ELF** en memoria
/// y lo deja listo (pila, espacio de direcciones) para correr. `nombre` es el
/// con el que `ps` mostrara la tarea.
pub fn lanzar_elf(datos: &[u8], nombre: &str) -> Option<u64> {
    let mut p = Proceso::nuevo();
    let (entrada, pila) = match p.cargar_elf(datos) {
        Some(v) => v,
        None => return None,
    };
    Some(crate::tarea::crear_usuario(
        nombre,
        entrada as u64,
        pila as u64,
        p,
    ))
}

impl Drop for Proceso {
    fn drop(&mut self) {
        // Las tablas vuelven al gestor tal cual; las paginas de usuario pueden
        // estar compartidas (copia-al-escribir), asi que se sueltan con
        // `frame_release`, que solo libera el marco cuando queda sin duenos.
        for &f in &self.tablas {
            pmm::free_frame(f);
        }
        for &(_, phys) in &self.paginas {
            pmm::frame_release(phys);
        }
    }
}

/// Escribe en la pila de usuario el bloque de argumentos que vera el programa
/// nuevo: `argc`, `argv[]`, `envp[]` (cada tabla terminada en NULL) y las
/// cadenas, al estilo de Linux.
pub fn montar_pila(tope: usize, argv: &[Vec<u8>], envp: &[Vec<u8>]) -> Option<usize> {
    // El nucleo solo garantiza mapeada la pagina superior de la pila; el bloque
    // de argumentos cabe de sobra en ella (ver [`tamano_pila`]).
    let limite = tope - pmm::FRAME_SIZE;
    let mut sp = tope;

    // Las cadenas van arriba del todo.
    let mut argv_ptr: Vec<usize> = Vec::new();
    for s in argv {
        sp = empujar_cadena(sp, s, limite)?;
        argv_ptr.push(sp);
    }
    let mut envp_ptr: Vec<usize> = Vec::new();
    for s in envp {
        sp = empujar_cadena(sp, s, limite)?;
        envp_ptr.push(sp);
    }

    // La ABI pide `rsp` alineado a 16 al entrar en una funcion.
    sp &= !15;

    // envp[] (termina en NULL), argv[] (termina en NULL) y argc.
    sp = empujar(sp, 0, limite)?;
    for p in envp_ptr.iter().rev() {
        sp = empujar(sp, *p as u64, limite)?;
    }
    sp = empujar(sp, 0, limite)?;
    for p in argv_ptr.iter().rev() {
        sp = empujar(sp, *p as u64, limite)?;
    }
    sp = empujar(sp, argv.len() as u64, limite)?;
    Some(sp)
}

/// Espacio que ocupara el bloque de argumentos en la pila (cadenas, tablas,
/// `argc` y un margen de alineacion). Sirve para comprobar que cabe **antes**
/// de reemplazar la imagen del proceso.
pub fn tamano_pila(argv: &[Vec<u8>], envp: &[Vec<u8>]) -> usize {
    let cadenas: usize = argv.iter().chain(envp.iter()).map(|s| s.len() + 1).sum();
    let tablas = 8 * (1 + argv.len() + 1 + envp.len() + 1);
    cadenas + tablas + 16
}

/// Baja `sp` ocho bytes y escribe `v`; falla si se sale de la pagina de pila.
fn empujar(sp: usize, v: u64, limite: usize) -> Option<usize> {
    let sp = sp.checked_sub(8)?;
    if sp < limite {
        return None;
    }
    unsafe { core::ptr::write_volatile(sp as *mut u64, v) };
    Some(sp)
}

/// Copia una cadena terminada en NUL a la pila, bajando `sp`.
fn empujar_cadena(sp: usize, s: &[u8], limite: usize) -> Option<usize> {
    let sp = sp.checked_sub(s.len() + 1)?;
    if sp < limite {
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), sp as *mut u8, s.len());
        core::ptr::write_volatile((sp + s.len()) as *mut u8, 0);
    }
    Some(sp)
}
