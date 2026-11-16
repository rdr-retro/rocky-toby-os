//! Acceso acotado a los registros de un controlador xHCI.
//!
//! La ventana de registros de un xHCI esta repartida en **tres bases**, y
//! donde empieza cada una lo dice el propio controlador (el `CAPLENGTH` y los
//! dos registros de desplazamiento). Este modulo guarda las capacidades y
//! calcula los desplazamiento, de modo que el resto del driver solo pregunta
//! por nombres ("el registro de estado del puerto 3") y nunca por numeros.
//!
//! Cada acceso comprueba su rango contra el tamano del BAR: un registro que
//! no existe en el hardware no se puede ni leer ni escribir.
//!
//! Referencia: Intel, "eXtensible Host Controller Interface for Universal
//! Serial Bus (xHCI) Requirements Specification", Rev 1.2, §5.3, §5.4 y §5.5.
//!
//! Codigo propio de rocky.

use crate::drivers::usb::mmio::{ErrorMmio, Mmio};
use rockyhw::xhci::caps::{Capacidades, Registro};
use rockyhw::xhci::regs::*;

/// La ventana de registros de un controlador xHCI.
///
/// Encapsula la disposicion que dan las capacidades: donde esta la base
/// operacional, donde la de ejecucion y donde los puertos raiz.
#[derive(Clone, Copy)]
pub struct Registros {
    ventana: Mmio,
    caps: Capacidades,
}

impl Registros {
    /// Prepara el acceso a los registros de un controlador.
    pub fn nueva(ventana: Mmio, caps: Capacidades) -> Registros {
        Registros { ventana, caps }
    }

    /// Las capacidades del controlador.
    pub fn capacidades(&self) -> &Capacidades {
        &self.caps
    }

    /// La ventana de registros, para leer lo que no sea de una base concreta.
    pub fn ventana(&self) -> Mmio {
        self.ventana
    }

    /// Lee un dword de la base operacional.
    pub fn leer_op(&self, off: usize) -> Result<u32, ErrorMmio> {
        self.ventana.leer32(self.caps.base_operacional + off)
    }

    /// Escribe un dword de la base operacional.
    pub fn escribir_op(&self, off: usize, valor: u32) -> Result<(), ErrorMmio> {
        self.ventana
            .escribir32(self.caps.base_operacional + off, valor)
    }

    /// Escribe un registro de 64 bits de la base operacional.
    ///
    /// **El dword alto se escribe aparte, y despues del bajo.** No es
    /// capricho: el controlador solo se entera de que un registro de 64 bits
    /// ha cambiado cuando llega la escritura de la parte alta. En el `CRCR`
    /// la escritura del dword alto es la que hace que el controlador
    /// inicialice el anillo de comandos (§5.4.5); en el `ERSTBA` la que hace
    /// que lea la tabla de segmentos de evento (§5.5.2.3.2).
    ///
    /// Ademas hay emuladores (QEMU entre ellos) que atropellan el dword
    /// alto cuando la escritura es de 8 bytes de una vez: solo guardan la
    /// parte baja y el registro 64 bits se queda a medias.
    pub fn escribir64_op(&self, off: usize, valor: u64) -> Result<(), ErrorMmio> {
        self.escribir_op(off, valor as u32)?;
        self.escribir_op(off + 4, (valor >> 32) as u32)
    }

    /// Escribe un dword de la base de ejecucion.
    pub fn escribir_rt(&self, off: usize, valor: u32) -> Result<(), ErrorMmio> {
        self.ventana
            .escribir32(self.caps.base_ejecucion + off, valor)
    }

    /// Escribe un registro de 64 bits de la base de ejecucion.
    ///
    /// Igual que [`Registros::escribir64_op`]: el dword alto va aparte y
    /// despues del bajo.
    pub fn escribir64_rt(&self, off: usize, valor: u64) -> Result<(), ErrorMmio> {
        self.escribir_rt(off, valor as u32)?;
        self.escribir_rt(off + 4, (valor >> 32) as u32)
    }

    /// Lee `USBCMD`.
    pub fn usbcmd(&self) -> Result<u32, ErrorMmio> {
        self.leer_op(OP_USBCMD)
    }

    /// Escribe `USBCMD`.
    pub fn escribir_usbcmd(&self, valor: u32) -> Result<(), ErrorMmio> {
        self.escribir_op(OP_USBCMD, valor)
    }

    /// Lee `USBSTS`.
    pub fn usbsts(&self) -> Result<u32, ErrorMmio> {
        self.leer_op(OP_USBSTS)
    }

    /// Escribe `USBSTS`.
    ///
    /// Solo se usa para limpiar las banderas de escribir-1-para-limpiar
    /// (`HCE`, `HSE`, `EINT`, `PCD`): los demas bits del registro son de solo
    /// lectura y escribirlos no hace nada (§5.4.2).
    pub fn escribir_usbsts(&self, valor: u32) -> Result<(), ErrorMmio> {
        self.escribir_op(OP_USBSTS, valor)
    }

    /// El desplazamiento del registro de estado de un puerto.
    ///
    /// El numero de puerto va de 1 a `MaxPorts` (§5.4.8). Un puerto fuera de
    /// ese rango es un error: no se inventa la direccion.
    pub fn offset_puerto(&self, numero: u8) -> Result<usize, ErrorMmio> {
        if numero == 0 || numero > self.caps.max_ports {
            return Err(ErrorMmio::FueraDeRango { offset: 0, len: 4 });
        }
        Ok(self.caps.base_operacional + OP_PUERTOS + (numero as usize - 1) * PUERTO_PASO)
    }

    /// Lee el registro de estado de un puerto.
    pub fn leer_puerto(&self, numero: u8) -> Result<u32, ErrorMmio> {
        let off = self.offset_puerto(numero)?;
        self.ventana.leer32(off)
    }

    /// Escribe el registro de estado de un puerto.
    pub fn escribir_puerto(&self, numero: u8, valor: u32) -> Result<(), ErrorMmio> {
        let off = self.offset_puerto(numero)?;
        self.ventana.escribir32(off, valor)
    }

    /// Escribe un registro doorbell (base_campanas + slot*4)
    #[allow(dead_code)] // Se usa al implementar la cola de comandos y transferencias.
    pub fn escribir_doorbell(&self, slot: u8, valor: u32) -> Result<(), ErrorMmio> {
        if slot == 0 {
            self.ventana.escribir32(self.caps.base_campanas, valor)
        } else {
            self.ventana
                .escribir32(self.caps.base_campanas + (slot as usize) * 4, valor)
        }
    }
}

/// La ventana de registros vista como ventana de capacidades.
///
/// Se usa **antes** de conocer la disposicion: las capacidades se leen
/// siempre en la base del BAR, y el `CAPLENGTH` dice donde empieza lo demas.
#[derive(Clone, Copy)]
pub struct VentanaCapacidades(pub Mmio);

impl Registro for VentanaCapacidades {
    fn leer32(&self, off: usize) -> Option<u32> {
        self.0.leer32(off).ok()
    }
}
