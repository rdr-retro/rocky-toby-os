//! Decodificacion del registro de estado de un puerto raiz (`PORTSC`).
//!
//! El `PORTSC` mezcla estado y banderas de cambio. Este modulo las separa y
//! las traduce a un lenguaje que el resto de la pila puede usar: hay algo
//! conectado, a que velocidad, y que ha cambiado desde la ultima vez que se
//! miro.
//!
//! Referencia: Intel, "eXtensible Host Controller Interface for Universal
//! Serial Bus (xHCI) Requirements Specification", Rev 1.2, §5.4.8.
//!
//! Codigo propio de rocky.

use crate::xhci::regs::*;

/// La velocidad a la que trabaja un dispositivo en un puerto.
///
/// El campo `Port Speed` de `PORTSC` (bits 13:10) informa del identificador de
/// velocidad de protocolo (§5.4.8 y §7.2.1). El valor cero significa
/// "velocidad indefinida": en un puerto USB 2.0 el campo no es valido hasta
/// que el puerto se ha reiniciado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Velocidad {
    /// Sin velocidad conocida.
    Indefinida,
    /// USB 1.0, baja velocidad (1,5 Mb/s).
    Baja,
    /// USB 1.1, velocidad completa (12 Mb/s).
    Completa,
    /// USB 2.0, velocidad alta (480 Mb/s).
    Alta,
    /// USB 3.0, SuperSpeed (5 Gb/s).
    Super,
    /// USB 3.2, SuperSpeed Plus (10 Gb/s).
    SuperPlus,
}

impl Velocidad {
    /// Traduce el valor del campo `Port Speed`.
    ///
    /// En la revision 1.2 el campo `Port Speed` de `PORTSC` (bits 13:10) no
    /// lleva la codificacion clasica (1 = baja, 2 = completa, 3 = alta,
    /// 4 = SuperSpeed), sino un **identificador de velocidad de protocolo**
    /// (`PSI`, §7.2.1). Cuando el controlador no define palabras `PSI`
    /// (`PSIC = 0`) aparecen los valores por defecto de la tabla 7-13:
    ///
    /// | Valor | Velocidad            |
    /// |-------|----------------------|
    /// | 1     | completa (12 Mb/s)   |
    /// | 2     | baja (1,5 Mb/s)      |
    /// | 3     | alta (480 Mb/s)      |
    /// | 4     | SuperSpeed (5 Gb/s)  |
    /// | 5     | SuperSpeed Plus      |
    ///
    /// Ojo: el uno y el dos estan **cambiados** respecto de la revision 1.0:
    /// el uno es velocidad completa y el dos es baja velocidad.
    ///
    /// Si el controlador define palabras `PSI` propias, el valor del campo es
    /// un identificador y habria que buscarlo en la tabla de palabras `PSI`
    /// (§7.2.1) para saber la velocidad real: eso no se hace todavia.
    pub fn de_codigo(codigo: u32) -> Velocidad {
        match codigo {
            1 => Velocidad::Completa,
            2 => Velocidad::Baja,
            3 => Velocidad::Alta,
            4 => Velocidad::Super,
            5 | 6 | 7 => Velocidad::SuperPlus,
            _ => Velocidad::Indefinida,
        }
    }

    /// Nombre corto, para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            Velocidad::Indefinida => "velocidad indefinida",
            Velocidad::Baja => "baja velocidad (USB 1.0)",
            Velocidad::Completa => "velocidad completa (USB 1.1)",
            Velocidad::Alta => "velocidad alta (USB 2.0)",
            Velocidad::Super => "SuperSpeed (USB 3.0)",
            Velocidad::SuperPlus => "SuperSpeed Plus (USB 3.2)",
        }
    }

    /// ¿Es una velocidad de USB 3.x o superior?
    pub fn es_superspeed(self) -> bool {
        matches!(self, Velocidad::Super | Velocidad::SuperPlus)
    }
}

/// El campo `Interval` del contexto de un endpoint de interrupcion (§6.2.3.8).
///
/// El controlador no guarda milisegundos: guarda el **exponente** del periodo
/// en microframes de 125 µs. La conversion desde `bInterval` depende de la
/// velocidad y es la misma que hace Linux (`xhci_get_endpoint_interval`):
///
/// - alta velocidad, SuperSpeed y SuperSpeed Plus: exponente `bInterval - 1`,
///   o sea un periodo de `2^(bInterval-1)` microframes, con `bInterval`
///   acotado a 1..16;
/// - velocidad completa o baja: `bInterval` son milisegundos, o sea
///   `bInterval * 8` microframes, redondeado **hacia abajo** a potencia de dos
///   y acotado a los exponentes 3..10 (de 8 a 1024 microframes), que es el
///   rango que la especificacion admite para un endpoint de interrupcion.
pub fn intervalo_interrupcion(velocidad: Velocidad, b_interval: u8) -> u8 {
    match velocidad {
        Velocidad::Alta | Velocidad::Super | Velocidad::SuperPlus => b_interval.clamp(1, 16) - 1,
        Velocidad::Baja | Velocidad::Completa => {
            // `bInterval` vale al menos 1 en un endpoint de interrupcion; si
            // viniera a cero se trata como 1 para no calcular log2(0).
            let microframes = b_interval.max(1) as u32 * 8;
            let exponente = 31 - microframes.leading_zeros();
            exponente.clamp(3, 10) as u8
        }
        Velocidad::Indefinida => 0,
    }
}

/// El estado de un puerto raiz, ya decodificado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EstadoPuerto {
    /// Hay un dispositivo conectado (`CCS`).
    pub conectado: bool,
    /// El puerto esta habilitado (`PED`).
    pub habilitado: bool,
    /// Hay sobretension en el puerto (`OCA`).
    pub sobretension: bool,
    /// Se ha pedido el reinicio del puerto (`PR`).
    pub reiniciando: bool,
    /// Estado del enlace del puerto (`PLS`, bits 8:5).
    pub enlace: u32,
    /// El puerto tiene alimentacion (`PP`).
    pub alimentado: bool,
    /// Velocidad del dispositivo conectado.
    pub velocidad: Velocidad,
    /// Cambio de estado de conexion (`CSC`).
    pub cambio_conexion: bool,
    /// Cambio de puerto habilitado (`PEC`).
    pub cambio_habilitado: bool,
    /// Cambio de reinicio en caliente (`WRC`).
    pub cambio_reinicio_caliente: bool,
    /// Cambio de sobretension (`OCC`).
    pub cambio_sobretension: bool,
    /// Cambio de reinicio del puerto (`PRC`).
    pub cambio_reinicio: bool,
    /// Cambio de estado del enlace (`PLC`).
    pub cambio_enlace: bool,
    /// Error de configuracion del puerto (`CEC`).
    pub error_configuracion: bool,
    /// Conexion en frio (`CAS`).
    pub conexion_en_frio: bool,
    /// El dispositivo es extraible (`DR`).
    pub extraible: bool,
}

impl EstadoPuerto {
    /// Decodifica el valor crudo de `PORTSC`.
    pub fn decodificar(valor: u32) -> EstadoPuerto {
        EstadoPuerto {
            conectado: valor & PORTSC_CCS != 0,
            habilitado: valor & PORTSC_PED != 0,
            sobretension: valor & PORTSC_OCA != 0,
            reiniciando: valor & PORTSC_PR != 0,
            enlace: (valor & PORTSC_PLS_MASCARA) >> PORTSC_PLS_DESPLAZAMIENTO,
            alimentado: valor & PORTSC_PP != 0,
            velocidad: Velocidad::de_codigo(
                (valor & PORTSC_VELOCIDAD_MASCARA) >> PORTSC_VELOCIDAD_DESPLAZAMIENTO,
            ),
            cambio_conexion: valor & PORTSC_CSC != 0,
            cambio_habilitado: valor & PORTSC_PEC != 0,
            cambio_reinicio_caliente: valor & PORTSC_WRC != 0,
            cambio_sobretension: valor & PORTSC_OCC != 0,
            cambio_reinicio: valor & PORTSC_PRC != 0,
            cambio_enlace: valor & PORTSC_PLC != 0,
            error_configuracion: valor & PORTSC_CEC != 0,
            conexion_en_frio: valor & PORTSC_CAS != 0,
            extraible: valor & PORTSC_DR != 0,
        }
    }

    /// ¿Ha cambiado algo del puerto desde la ultima lectura?
    ///
    /// Solo mira las banderas de cambio: son ellas las que el controlador
    /// pone a uno cuando algo pasa (§4.19.2).
    pub fn hay_cambio(&self) -> bool {
        self.cambio_conexion
            || self.cambio_habilitado
            || self.cambio_reinicio_caliente
            || self.cambio_sobretension
            || self.cambio_reinicio
            || self.cambio_enlace
            || self.error_configuracion
    }

    /// Máscara de las banderas de cambio que hay que limpiar en `PORTSC`.
    pub fn mascara_cambio(&self) -> u32 {
        let mut m = 0;
        if self.cambio_conexion {
            m |= PORTSC_CSC;
        }
        if self.cambio_habilitado {
            m |= PORTSC_PEC;
        }
        if self.cambio_reinicio_caliente {
            m |= PORTSC_WRC;
        }
        if self.cambio_sobretension {
            m |= PORTSC_OCC;
        }
        if self.cambio_reinicio {
            m |= PORTSC_PRC;
        }
        if self.cambio_enlace {
            m |= PORTSC_PLC;
        }
        if self.error_configuracion {
            m |= PORTSC_CEC;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodifica_un_puerto_vacio() {
        // Solo con alimentacion (PP) y sin nada mas.
        let e = EstadoPuerto::decodificar(PORTSC_PP);
        assert!(e.alimentado);
        assert!(!e.conectado);
        assert!(!e.habilitado);
        assert_eq!(e.velocidad, Velocidad::Indefinida);
        assert!(!e.hay_cambio());
    }

    #[test]
    fn decodifica_un_dispositivo_usb2_alta_velocidad() {
        // CCS + PED + PP + velocidad alta (3) + PRC.
        let valor = PORTSC_CCS | PORTSC_PED | PORTSC_PP | (3 << 10) | PORTSC_PRC;
        let e = EstadoPuerto::decodificar(valor);
        assert!(e.conectado);
        assert!(e.habilitado);
        assert_eq!(e.velocidad, Velocidad::Alta);
        assert!(e.cambio_reinicio);
        assert!(e.hay_cambio());
        assert_eq!(e.mascara_cambio(), PORTSC_PRC);
    }

    #[test]
    fn decodifica_superspeed() {
        let valor = PORTSC_CCS | PORTSC_PED | PORTSC_PP | (4 << 10);
        let e = EstadoPuerto::decodificar(valor);
        assert_eq!(e.velocidad, Velocidad::Super);
        assert!(e.velocidad.es_superspeed());
    }

    #[test]
    fn la_velocidad_no_se_confunde_con_el_enlace() {
        // PLS = 7 (Polling) no debe confundirse con la velocidad.
        let valor = (7 << 5) | PORTSC_PP;
        let e = EstadoPuerto::decodificar(valor);
        assert_eq!(e.enlace, 7);
        assert_eq!(e.velocidad, Velocidad::Indefinida);
    }

    #[test]
    fn distingue_baja_de_completa() {
        // Tabla 7-13: el 1 es velocidad completa (12 Mb/s) y el 2 es baja
        // (1,5 Mb/s). En la revision 1.0 era al reves.
        let e = EstadoPuerto::decodificar(1 << 10);
        assert_eq!(e.velocidad, Velocidad::Completa);
        let e = EstadoPuerto::decodificar(2 << 10);
        assert_eq!(e.velocidad, Velocidad::Baja);
    }
}

#[cfg(test)]
mod tests_intervalo {
    use super::*;

    #[test]
    fn alta_velocidad_usa_el_exponente_del_descriptor() {
        // bInterval 10 en alta velocidad: 2^(10-1) = 512 microframes.
        assert_eq!(intervalo_interrupcion(Velocidad::Alta, 10), 9);
        assert_eq!(intervalo_interrupcion(Velocidad::Alta, 1), 0);
        assert_eq!(intervalo_interrupcion(Velocidad::Alta, 0), 0);
        assert_eq!(intervalo_interrupcion(Velocidad::Alta, 255), 15);
    }

    #[test]
    fn velocidad_completa_redondea_los_milisegundos() {
        // 10 ms son 80 microframes: la potencia de dos menor es 64 (2^6).
        assert_eq!(intervalo_interrupcion(Velocidad::Completa, 10), 6);
        // 1 ms son 8 microframes: exponente 3.
        assert_eq!(intervalo_interrupcion(Velocidad::Completa, 1), 3);
        // El tope superior son 1024 microframes (exponente 10).
        assert_eq!(intervalo_interrupcion(Velocidad::Completa, 255), 10);
        assert_eq!(intervalo_interrupcion(Velocidad::Baja, 10), 6);
    }

    #[test]
    fn superspeed_usa_el_mismo_exponente_que_linux() {
        assert_eq!(intervalo_interrupcion(Velocidad::Super, 5), 4);
        assert_eq!(intervalo_interrupcion(Velocidad::SuperPlus, 16), 15);
    }

    #[test]
    fn sin_velocidad_no_hay_intervalo() {
        assert_eq!(intervalo_interrupcion(Velocidad::Indefinida, 10), 0);
    }
}
