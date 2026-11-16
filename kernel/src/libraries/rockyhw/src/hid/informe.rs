//! El **descriptor de informe** HID: que campos trae cada informe y como se
//! leen sus bits.
//!
//! El protocolo boot solo sirve para teclados y ratones que lo declaran. Un
//! HID moderno (o un teclado con teclas de consumo, luces, rueda, etc.) declara
//! su propio formato con un **descriptor de informe** (HID 1.11, seccion 6.2.2)
//! y manda los datos empaquetados como le da la gana: con o sin `Report ID`,
//! con campos de 1, 8, 12 o 16 bits, con valores con signo o sin el.
//!
//! Este modulo lo interpreta en dos pasos:
//!
//! 1. [`analizar`] recorre el descriptor (items cortos y largos, estado global
//!    con `Push`/`Pop`, estado local, `Report ID`, `Report Size`/`Count` y
//!    colecciones) y deja una lista acotada de [`Campo`].
//! 2. [`Dispositivo::teclado`] y [`Dispositivo::raton`] leen un informe real y
//!    lo convierten al mismo [`InformeTeclado`]/[`InformeRaton`] que usa el
//!    protocolo boot.
//!
//! Aqui solo hay logica pura y no se reserva memoria: el descriptor, el numero
//! de campos y el tamano de cada informe estan acotados, y cualquier cosa que
//! no encaje se rechaza con un [`Error`] en vez de leer fuera de rango. El
//! llamador puede entonces tratar la interfaz como no utilizable.

use super::boot::{InformeRaton, InformeTeclado, TECLAS_SIMULTANEAS, USO_ERROR_ROLLOVER};

/// Campos de entrada que se guardan como maximo.
///
/// Un teclado normal usa 8 modificadores + 1 matriz; un raton, 5 botones + 3
/// ejes. Con 48 hay margen para teclados con muchas teclas de funcion y para
/// dispositivos raros, sin que la tabla crezca sin limite.
pub const MAX_CAMPOS: usize = 48;

/// Usos que puede acumular un item local antes de su item principal.
pub const MAX_USOS: usize = 16;

/// Profundidad maxima de `Push`/`Pop` del estado global.
pub const MAX_PILA: usize = 4;

/// Bits maximos de un informe. El buffer de interrupcion es de 64 bytes y el
/// primer byte puede ser el `Report ID`.
pub const MAX_BITS_INFORME: u32 = 512;

/// Pagina de uso "Generic Desktop".
const PAGINA_ESCRITORIO: u16 = 0x01;
/// Pagina de uso "Keyboard/Keypad".
const PAGINA_TECLADO: u16 = 0x07;
/// Pagina de uso "Button".
const PAGINA_BOTONES: u16 = 0x09;

/// Uso de escritorio "Keyboard".
const USO_TECLADO: u32 = 0x06;
/// Uso de escritorio "Keypad".
const USO_TECLADO_NUMERICO: u32 = 0x07;
/// Uso de escritorio "Mouse".
const USO_RATON: u32 = 0x02;
/// Uso de escritorio "Pointer".
const USO_PUNTERO: u32 = 0x01;
/// Uso de escritorio "X".
const USO_X: u32 = 0x30;
/// Uso de escritorio "Y".
const USO_Y: u32 = 0x31;

/// El campo es una variable (un control por elemento).
const F_VARIABLE: u8 = 1 << 1;
/// El campo es relativo (solo tiene sentido en el raton).
const F_RELATIVO: u8 = 1 << 2;

/// Que clase de dispositivo describe el descriptor.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tipo {
    /// Teclado (pagina de teclado con modificadores o matriz de teclas).
    Teclado,
    /// Raton o puntero relativo.
    Raton,
    /// Cualquier otra cosa: no se usa como entrada.
    #[default]
    Desconocido,
}

impl Tipo {
    /// Nombre corto para la consola.
    pub fn nombre(self) -> &'static str {
        match self {
            Tipo::Teclado => "teclado",
            Tipo::Raton => "raton",
            Tipo::Desconocido => "desconocido",
        }
    }
}

/// Un campo de entrada del informe.
///
/// Un item `Input` **variable** se guarda como un campo por elemento (asi cada
/// modificador o boton tiene su uso), y un item **array** se guarda como un
/// solo campo con `cuenta` elementos y el rango `uso..=uso_fin` del que salen
/// los indices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Campo {
    /// Pagina de uso.
    pub pagina: u16,
    /// Primer uso del campo.
    pub uso: u32,
    /// Ultimo uso del campo (igual a `uso` si es un solo control).
    pub uso_fin: u32,
    /// `Report ID` al que pertenece (0 si el dispositivo no usa IDs).
    pub id: u8,
    /// Bits de [`F_VARIABLE`] y [`F_RELATIVO`].
    pub flags: u8,
    /// Desplazamiento en bits dentro del informe (sin contar el byte del ID).
    pub bit: u16,
    /// Bits de cada elemento.
    pub tam: u8,
    /// Elementos que ocupa el campo (1 en los campos variable).
    pub cuenta: u8,
    /// Minimo logico (si es negativo, el valor va con signo).
    pub minimo: i32,
    /// Maximo logico.
    pub maximo: i32,
}

impl Campo {
    /// Si el valor va en complemento a dos.
    pub fn firmado(&self) -> bool {
        self.minimo < 0
    }

    /// Si el campo es variable.
    pub fn variable(&self) -> bool {
        self.flags & F_VARIABLE != 0
    }
}

/// Un descriptor de informe ya interpretado y acotado.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dispositivo {
    /// Que clase de dispositivo es.
    pub tipo: Tipo,
    /// Si los informes empiezan por un byte de `Report ID`.
    pub usa_ids: bool,
    /// El `Report ID` que lleva los campos de este dispositivo (0 si no hay).
    pub id: u8,
    /// Cuantos campos hay en `campos`.
    pub n: u8,
    /// Bits que necesita el informe de este dispositivo (sin el byte del ID).
    pub necesita: u16,
    /// Los campos de entrada, en orden de descriptor.
    pub campos: [Campo; MAX_CAMPOS],
}

impl Dispositivo {
    /// Un descriptor sin campos.
    pub const VACIO: Dispositivo = Dispositivo {
        tipo: Tipo::Desconocido,
        usa_ids: false,
        id: 0,
        n: 0,
        necesita: 0,
        campos: [Campo {
            pagina: 0,
            uso: 0,
            uso_fin: 0,
            id: 0,
            flags: 0,
            bit: 0,
            tam: 0,
            cuenta: 0,
            minimo: 0,
            maximo: 0,
        }; MAX_CAMPOS],
    };

    /// Los campos validos.
    pub fn campos(&self) -> &[Campo] {
        &self.campos[..self.n as usize]
    }

    /// Si el informe empieza por el `Report ID` de este dispositivo.
    ///
    /// Un descriptor puede declarar varios IDs (teclado + teclas de consumo,
    /// por ejemplo); los informes de los otros se ignoran sin contarlos como
    /// error.
    pub fn es_suyo(&self, datos: &[u8]) -> bool {
        if !self.usa_ids {
            return true;
        }
        datos.first().copied() == Some(self.id)
    }

    /// El cuerpo del informe, sin el byte del `Report ID`.
    fn cuerpo<'a>(&self, datos: &'a [u8]) -> Option<&'a [u8]> {
        if !self.es_suyo(datos) {
            return None;
        }
        if self.usa_ids {
            datos.get(1..)
        } else {
            Some(datos)
        }
    }

    /// Lee un informe de teclado y lo deja en el formato del protocolo boot.
    ///
    /// Devuelve `None` si el informe es de otro `Report ID` o si le faltan
    /// bits. Si el descriptor no es de teclado tampoco se inventa nada.
    pub fn teclado(&self, datos: &[u8]) -> Option<InformeTeclado> {
        if self.tipo != Tipo::Teclado {
            return None;
        }
        let base = self.cuerpo(datos)?;
        if (base.len() as u32) * 8 < self.necesita as u32 {
            return None;
        }
        let mut r = InformeTeclado::default();
        let mut n = 0usize;
        let mut desbordado = false;

        for campo in self.campos() {
            if campo.pagina != PAGINA_TECLADO || campo.tam == 0 {
                continue;
            }
            if campo.variable() {
                for e in 0..campo.cuenta as u32 {
                    let Some(v) = leer(base, campo.bit as u32 + e * campo.tam as u32, campo.tam, false)
                    else {
                        continue;
                    };
                    if v == 0 {
                        continue;
                    }
                    // Los modificadores se declaran como ocho controles de un
                    // bit con usos E0..E7.
                    let uso = campo.uso.saturating_add(e);
                    if (0xE0..=0xE7).contains(&uso) {
                        r.modificadores |= 1 << (uso - 0xE0);
                    } else if uso >= 0x04 && uso <= 0xFF {
                        if !poner(&mut r, uso as u8, &mut n) {
                            desbordado = true;
                        }
                    }
                }
            } else {
                // Matriz de teclas: cada elemento es un indice dentro del
                // rango de usos; 0 es "hueco".
                for e in 0..campo.cuenta as u32 {
                    let Some(v) = leer(base, campo.bit as u32 + e * campo.tam as u32, campo.tam, false)
                    else {
                        continue;
                    };
                    if v <= 0 {
                        continue;
                    }
                    let uso = v as u64;
                    if uso < campo.uso as u64 || uso > campo.uso_fin as u64 {
                        continue;
                    }
                    if uso < 0x04 {
                        continue;
                    }
                    if !poner(&mut r, uso as u8, &mut n) {
                        desbordado = true;
                    }
                }
            }
        }

        if desbordado {
            // Mas teclas de las que caben: se avisa con `ErrorRollOver` para
            // que el consumidor suelte lo anterior en vez de inventar teclas.
            r.teclas = [0; TECLAS_SIMULTANEAS];
            r.teclas[0] = USO_ERROR_ROLLOVER;
        }
        Some(r)
    }

    /// Lee un informe de raton: botones, X e Y (la rueda se ignora).
    ///
    /// Solo se usan los tres primeros botones, que es lo que entiende
    /// [`InformeRaton`] y el resto del kernel.
    pub fn raton(&self, datos: &[u8]) -> Option<InformeRaton> {
        if self.tipo != Tipo::Raton {
            return None;
        }
        let base = self.cuerpo(datos)?;
        if (base.len() as u32) * 8 < self.necesita as u32 {
            return None;
        }
        let mut r = InformeRaton::default();
        let mut hay_eje = false;

        for campo in self.campos() {
            if campo.tam == 0 {
                continue;
            }
            if campo.pagina == PAGINA_BOTONES && campo.uso >= 1 {
                for e in 0..campo.cuenta as u32 {
                    let Some(v) = leer(base, campo.bit as u32 + e * campo.tam as u32, campo.tam, false)
                    else {
                        continue;
                    };
                    if v == 0 {
                        continue;
                    }
                    let boton = campo.uso.saturating_add(e);
                    if boton >= 1 && boton <= 3 {
                        r.botones |= 1 << (boton - 1);
                    }
                }
                continue;
            }
            if campo.pagina != PAGINA_ESCRITORIO {
                continue;
            }
            let Some(v) = leer(base, campo.bit as u32, campo.tam, campo.firmado()) else {
                continue;
            };
            match campo.uso {
                USO_X => {
                    r.dx = limitar(v);
                    hay_eje = true;
                }
                USO_Y => {
                    r.dy = limitar(v);
                    hay_eje = true;
                }
                _ => {}
            }
        }

        if hay_eje {
            Some(r)
        } else {
            None
        }
    }
}

/// Guarda un uso en la lista de teclas sin repetirlo.
///
/// Devuelve `false` cuando ya no cabe otro.
fn poner(r: &mut InformeTeclado, uso: u8, n: &mut usize) -> bool {
    if r.teclas[..*n].contains(&uso) {
        return true;
    }
    if *n >= TECLAS_SIMULTANEAS {
        return false;
    }
    r.teclas[*n] = uso;
    *n += 1;
    true
}

/// Recorta un valor a lo que cabe en un `i8`.
fn limitar(v: i64) -> i8 {
    if v < i8::MIN as i64 {
        i8::MIN
    } else if v > i8::MAX as i64 {
        i8::MAX
    } else {
        v as i8
    }
}

/// Lee `tam` bits (1..32) que empiezan en `bit`, en orden little-endian.
///
/// Devuelve `None` si el campo se sale del informe o el tamano no es valido.
fn leer(base: &[u8], bit: u32, tam: u8, firmado: bool) -> Option<i64> {
    let tam = tam as u32;
    if tam == 0 || tam > 32 {
        return None;
    }
    let fin = bit.checked_add(tam)?;
    if (fin as usize + 7) / 8 > base.len() {
        return None;
    }
    let mut v = 0u32;
    for i in 0..tam {
        let p = bit + i;
        let b = (base[(p / 8) as usize] >> (p % 8)) & 1;
        v |= (b as u32) << i;
    }
    if !firmado {
        return Some(v as i64);
    }
    if tam == 32 {
        return Some(v as i32 as i64);
    }
    let signo = 1u32 << (tam - 1);
    if v & signo != 0 {
        Some(v as i64 - (1i64 << tam))
    } else {
        Some(v as i64)
    }
}

/// Estado global HID (el que `Push`/`Pop` guarda y restaura).
#[derive(Clone, Copy)]
struct Global {
    pagina: u16,
    minimo: i32,
    maximo: i32,
    tam: u32,
    cuenta: u32,
    id: u8,
}

impl Default for Global {
    fn default() -> Self {
        Global {
            pagina: 0,
            minimo: 0,
            maximo: 0,
            tam: 0,
            cuenta: 0,
            id: 0,
        }
    }
}

/// Un uso con su pagina (los items de 4 bytes la llevan dentro).
#[derive(Clone, Copy, Default)]
struct Uso {
    pagina: u16,
    uso: u32,
}

/// Por que no se puede usar un descriptor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// El descriptor termina a mitad de un item.
    Truncado,
    /// Un item declara un tamano que no se puede interpretar.
    TamanoInvalido,
    /// No hay ningun campo de entrada.
    SinCampos,
    /// Hay mas campos que [`MAX_CAMPOS`].
    DemasiadosCampos,
    /// Un informe ocupa mas de [`MAX_BITS_INFORME`].
    InformeDemasiadoGrande,
    /// Un item local acumula mas usos que [`MAX_USOS`].
    DemasiadosUsos,
    /// `Push` con la pila llena.
    PilaLlena,
    /// `Pop` con la pila vacia.
    PilaVacia,
    /// `End Collection` sin coleccion abierta.
    ColeccionSinCerrar,
    /// Queda una coleccion abierta al final.
    ColeccionAbierta,
    /// `Report ID` cero o fuera de rango.
    IdInvalido,
    /// `Report Count` mayor de 255.
    CuentaInvalida,
}

impl Error {
    /// Explicacion corta para la consola.
    pub fn motivo(self) -> &'static str {
        match self {
            Error::Truncado => "descriptor truncado",
            Error::TamanoInvalido => "item con tamano invalido",
            Error::SinCampos => "sin campos de entrada",
            Error::DemasiadosCampos => "demasiados campos",
            Error::InformeDemasiadoGrande => "informe demasiado grande",
            Error::DemasiadosUsos => "demasiados usos en un item",
            Error::PilaLlena => "Push con la pila llena",
            Error::PilaVacia => "Pop con la pila vacia",
            Error::ColeccionSinCerrar => "End Collection sin coleccion",
            Error::ColeccionAbierta => "coleccion sin cerrar",
            Error::IdInvalido => "Report ID invalido",
            Error::CuentaInvalida => "Report Count invalido",
        }
    }
}

/// Interpreta un descriptor de informe.
///
/// Solo se guardan los campos de **entrada**; los de salida y las
/// caracteristicas se recorren para validar el descriptor pero no ocupan
/// sitio. Un campo de mas de 32 bits se ignora (no se puede leer con esta
/// implementacion) sin invalidar el resto.
pub fn analizar(desc: &[u8]) -> Result<Dispositivo, Error> {
    if desc.is_empty() {
        return Err(Error::Truncado);
    }
    let mut d = Dispositivo::VACIO;
    let mut g = Global::default();
    let mut pila = [Global::default(); MAX_PILA];
    let mut n_pila = 0usize;
    let mut usos = [Uso::default(); MAX_USOS];
    let mut n_usos = 0usize;
    let mut uso_min: Option<Uso> = None;
    let mut uso_max: Option<Uso> = None;
    let mut aplicacion: Option<Uso> = None;
    // Profundidad de colecciones (Main Collection / End Collection).
    let mut prof = 0u32;
    let mut bits_sin_id = 0u32;
    let mut bits_con_id = [0u32; 256];

    let mut i = 0usize;
    while i < desc.len() {
        let prefijo = desc[i];
        i += 1;

        // Los items largos empiezan por 0xFE y no se interpretan: se saltan
        // con su longitud para no desincronizar el resto.
        if prefijo == 0xFE {
            if i + 2 > desc.len() {
                return Err(Error::Truncado);
            }
            let largo = desc[i] as usize;
            i += 2;
            if i + largo > desc.len() {
                return Err(Error::Truncado);
            }
            i += largo;
            continue;
        }

        let tam = match prefijo & 0x03 {
            0 => 0usize,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if i + tam > desc.len() {
            return Err(Error::Truncado);
        }
        let datos = leer_crudo(&desc[i..i + tam]);
        let firmado = leer_firmado(&desc[i..i + tam]);
        i += tam;

        let tipo = (prefijo >> 2) & 0x03;
        let etiqueta = prefijo >> 4;
        match tipo {
            // Main
            0 => match etiqueta {
                0x8 => {
                    let total = g
                        .tam
                        .checked_mul(g.cuenta)
                        .ok_or(Error::InformeDemasiadoGrande)?;
                    let base = if g.id == 0 {
                        bits_sin_id
                    } else {
                        bits_con_id[g.id as usize]
                    };
                    let fin = base
                        .checked_add(total)
                        .filter(|f| *f <= MAX_BITS_INFORME)
                        .ok_or(Error::InformeDemasiadoGrande)?;

                    let constante = datos & 0x01 != 0;
                    if !constante && g.cuenta > 0 {
                        if g.tam == 0 {
                            return Err(Error::TamanoInvalido);
                        }
                        // Un campo mayor de 32 bits no se puede leer con
                        // `leer`; se cuenta el hueco pero no se guarda.
                        if g.tam <= 32 {
                            agregar(
                                &mut d,
                                &g,
                                &usos[..n_usos],
                                uso_min,
                                uso_max,
                                base,
                                datos as u8,
                            )?;
                        }
                    }
                    if g.id == 0 {
                        bits_sin_id = fin;
                    } else {
                        bits_con_id[g.id as usize] = fin;
                    }
                    limpiar(&mut n_usos, &mut uso_min, &mut uso_max);
                }
                // Output (0x9), Feature (0xB): se recorren pero no ocupan
                // espacio en la tabla de entrada.
                0x9 | 0xB => {
                    limpiar(&mut n_usos, &mut uso_min, &mut uso_max);
                }
                0xA => {
                    // Collection: el tipo esta en los dos bits bajos del dato.
                    if datos & 0xFF == 0x01 && prof == 0 && aplicacion.is_none() {
                        aplicacion = usos.first().copied().or(uso_min);
                    }
                    prof = prof.checked_add(1).ok_or(Error::ColeccionAbierta)?;
                    limpiar(&mut n_usos, &mut uso_min, &mut uso_max);
                }
                0xC => {
                    if prof == 0 {
                        return Err(Error::ColeccionSinCerrar);
                    }
                    prof -= 1;
                    limpiar(&mut n_usos, &mut uso_min, &mut uso_max);
                }
                _ => {
                    limpiar(&mut n_usos, &mut uso_min, &mut uso_max);
                }
            },
            // Global
            1 => match etiqueta {
                0x0 => g.pagina = datos as u16,
                0x1 => g.minimo = firmado,
                0x2 => g.maximo = firmado,
                // Fisico, unidad, exponente: no hacen falta para leer bits.
                0x3..=0x6 => {}
                0x7 => g.tam = datos,
                0x8 => {
                    if datos == 0 || datos > 0xFF {
                        return Err(Error::IdInvalido);
                    }
                    g.id = datos as u8;
                }
                0x9 => {
                    if datos > 0xFF {
                        return Err(Error::CuentaInvalida);
                    }
                    g.cuenta = datos;
                }
                0xA => {
                    if n_pila >= MAX_PILA {
                        return Err(Error::PilaLlena);
                    }
                    pila[n_pila] = g;
                    n_pila += 1;
                }
                0xB => {
                    if n_pila == 0 {
                        return Err(Error::PilaVacia);
                    }
                    n_pila -= 1;
                    g = pila[n_pila];
                }
                _ => {}
            },
            // Local
            2 => match etiqueta {
                0x0 => {
                    if n_usos >= MAX_USOS {
                        return Err(Error::DemasiadosUsos);
                    }
                    usos[n_usos] = leer_uso(datos, tam, g.pagina);
                    n_usos += 1;
                }
                0x1 => uso_min = Some(leer_uso(datos, tam, g.pagina)),
                0x2 => uso_max = Some(leer_uso(datos, tam, g.pagina)),
                // Designadores, cadenas y delimitador: no hacen falta.
                _ => {}
            },
            // Reservado
            _ => {}
        }
    }

    if prof != 0 {
        return Err(Error::ColeccionAbierta);
    }
    if d.n == 0 {
        return Err(Error::SinCampos);
    }
    d.usa_ids = d.campos().iter().any(|c| c.id != 0);
    d.tipo = clasificar(aplicacion, &d);
    d.id = id_principal(&d);
    d.necesita = necesita(&d);
    Ok(d)
}

/// Anade los campos de un item `Input` a la tabla.
fn agregar(
    d: &mut Dispositivo,
    g: &Global,
    usos: &[Uso],
    uso_min: Option<Uso>,
    uso_max: Option<Uso>,
    base: u32,
    flags: u8,
) -> Result<(), Error> {
    let variable = flags & F_VARIABLE != 0;
    if variable {
        for e in 0..g.cuenta {
            let uso = if (e as usize) < usos.len() {
                usos[e as usize]
            } else if let (Some(min), Some(max)) = (uso_min, uso_max) {
                let Some(u) = min.uso.checked_add(e).filter(|u| *u <= max.uso) else {
                    break;
                };
                Uso {
                    pagina: min.pagina,
                    uso: u,
                }
            } else if let Some(ultimo) = usos.last() {
                *ultimo
            } else {
                Uso::default()
            };
            let bit = base + e * g.tam;
            if bit + g.tam > MAX_BITS_INFORME {
                return Err(Error::InformeDemasiadoGrande);
            }
            let campo = Campo {
                pagina: uso.pagina,
                uso: uso.uso,
                uso_fin: uso.uso,
                id: g.id,
                flags,
                bit: bit as u16,
                tam: g.tam as u8,
                cuenta: 1,
                minimo: g.minimo,
                maximo: g.maximo,
            };
            meter(d, campo)?;
        }
    } else {
        let (ini, fin) = if let (Some(min), Some(max)) = (uso_min, uso_max) {
            (min, max)
        } else if let (Some(primero), Some(ultimo)) = (usos.first(), usos.last()) {
            (*primero, *ultimo)
        } else {
            (Uso::default(), Uso::default())
        };
        let campo = Campo {
            pagina: ini.pagina,
            uso: ini.uso,
            uso_fin: fin.uso.max(ini.uso),
            id: g.id,
            flags,
            bit: base as u16,
            tam: g.tam as u8,
            cuenta: g.cuenta as u8,
            minimo: g.minimo,
            maximo: g.maximo,
        };
        meter(d, campo)?;
    }
    Ok(())
}

/// Guarda un campo si cabe.
fn meter(d: &mut Dispositivo, campo: Campo) -> Result<(), Error> {
    if d.n as usize >= MAX_CAMPOS {
        return Err(Error::DemasiadosCampos);
    }
    d.campos[d.n as usize] = campo;
    d.n += 1;
    Ok(())
}

/// Limpia el estado local despues de un item principal.
fn limpiar(n_usos: &mut usize, uso_min: &mut Option<Uso>, uso_max: &mut Option<Uso>) {
    *n_usos = 0;
    *uso_min = None;
    *uso_max = None;
}

/// Lee el dato de un item corto (0 a 4 bytes).
fn leer_crudo(b: &[u8]) -> u32 {
    let mut v = 0u32;
    for (i, byte) in b.iter().enumerate() {
        v |= (*byte as u32) << (8 * i);
    }
    v
}

/// Lee un item con signo (Logical Minimum/Maximum).
fn leer_firmado(b: &[u8]) -> i32 {
    match b.len() {
        1 => b[0] as i8 as i32,
        2 => i16::from_le_bytes([b[0], b[1]]) as i32,
        4 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        _ => 0,
    }
}

/// Lee un uso: con 4 bytes, los 16 altos son la pagina.
fn leer_uso(datos: u32, tam: usize, pagina: u16) -> Uso {
    if tam == 4 {
        Uso {
            pagina: (datos >> 16) as u16,
            uso: datos & 0xFFFF,
        }
    } else {
        Uso {
            pagina,
            uso: datos,
        }
    }
}

/// Decide de que clase es el dispositivo.
fn clasificar(aplicacion: Option<Uso>, d: &Dispositivo) -> Tipo {
    if let Some(app) = aplicacion {
        if app.pagina == PAGINA_ESCRITORIO {
            match app.uso {
                USO_TECLADO | USO_TECLADO_NUMERICO => return Tipo::Teclado,
                // Un raton manda desplazamientos relativos. Una tableta o un
                // puntero absoluto usa los mismos usos X/Y pero con valores de
                // posicion: tratarlo como raton moveria el cursor a saltos.
                USO_RATON | USO_PUNTERO => {
                    return if relativo(d) {
                        Tipo::Raton
                    } else {
                        Tipo::Desconocido
                    };
                }
                _ => {}
            }
        }
    }

    let teclado = d.campos().iter().any(|c| {
        c.pagina == PAGINA_TECLADO
            && c.tam > 0
            && (c.uso >= 0x04 || c.uso_fin >= 0x04)
    });
    if teclado {
        return Tipo::Teclado;
    }
    if relativo(d) {
        return Tipo::Raton;
    }
    Tipo::Desconocido
}

/// Si el descriptor declara X o Y como relativos.
fn relativo(d: &Dispositivo) -> bool {
    d.campos().iter().any(|c| {
        c.pagina == PAGINA_ESCRITORIO
            && (c.uso == USO_X || c.uso == USO_Y)
            && c.flags & F_RELATIVO != 0
    })
}

/// El `Report ID` que lleva los campos utiles (el de la matriz o el de X).
fn id_principal(d: &Dispositivo) -> u8 {
    for c in d.campos() {
        if c.id == 0 {
            return 0;
        }
        if d.tipo == Tipo::Teclado && c.pagina == PAGINA_TECLADO && !c.variable() {
            return c.id;
        }
        if d.tipo == Tipo::Raton && c.pagina == PAGINA_ESCRITORIO && c.uso == USO_X {
            return c.id;
        }
    }
    d.campos().first().map(|c| c.id).unwrap_or(0)
}

/// Bits que necesita el informe mas pequeno util para este dispositivo.
///
/// Si el informe que llega es mas corto, no se puede leer la tecla o el eje
/// que falta: se descarta en vez de devolver un informe a medias.
fn necesita(d: &Dispositivo) -> u16 {
    let mut bits = 0u32;
    for c in d.campos() {
        if d.usa_ids && c.id != d.id {
            continue;
        }
        let relevante = match d.tipo {
            Tipo::Teclado => c.pagina == PAGINA_TECLADO,
            Tipo::Raton => {
                c.pagina == PAGINA_ESCRITORIO || c.pagina == PAGINA_BOTONES
            }
            Tipo::Desconocido => false,
        };
        if !relevante {
            continue;
        }
        let fin = c.bit as u32 + c.tam as u32 * c.cuenta as u32;
        bits = bits.max(fin);
    }
    bits.min(MAX_BITS_INFORME) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    // Teclado boot de QEMU, sin Report ID.
    const TECLADO_BOOT: &[u8] = &[
        0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15, 0x00, 0x25,
        0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x06,
        0x75, 0x08, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x05, 0x07, 0x19, 0x00, 0x2A, 0xFF, 0x00, 0x81,
        0x00, 0xC0,
    ];

    // Teclado con Report ID 1 y un segundo informe (ID 2) de teclas de consumo.
    const TECLADO_CON_ID: &[u8] = &[
        0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15,
        0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01,
        0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x05, 0x07, 0x19, 0x00, 0x2A, 0xFF,
        0x00, 0x81, 0x00, 0x85, 0x02, 0x05, 0x0C, 0x19, 0x00, 0x2A, 0x3C, 0x02, 0x95, 0x01, 0x75,
        0x10, 0x81, 0x00, 0xC0,
    ];

    // Raton con Report ID 2: 3 botones, X/Y/Wheel de 8 bits con signo.
    const RATON: &[u8] = &[
        0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x02, 0x09, 0x01, 0xA1, 0x00, 0x05, 0x09, 0x19,
        0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01,
        0x75, 0x05, 0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7F, 0x75,
        0x08, 0x95, 0x02, 0x81, 0x06, 0x09, 0x38, 0x15, 0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x01,
        0x81, 0x06, 0xC0, 0xC0,
    ];

    #[test]
    fn teclado_boot_sin_id() {
        let d = analizar(TECLADO_BOOT).unwrap();
        assert_eq!(d.tipo, Tipo::Teclado);
        assert!(!d.usa_ids);
        assert_eq!(d.id, 0);
        let r = d.teclado(&[0x02, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(r.modificadores, 0x02);
        assert_eq!(r.teclas, [0x04, 0, 0, 0, 0, 0]);
        assert!(!r.hay_rollover());
    }

    #[test]
    fn teclado_boot_varias_teclas_y_suelta() {
        let d = analizar(TECLADO_BOOT).unwrap();
        let r = d
            .teclado(&[0x00, 0x00, 0x04, 0x05, 0x06, 0x00, 0x00, 0x00])
            .unwrap();
        assert_eq!(r.teclas, [0x04, 0x05, 0x06, 0, 0, 0]);
        let vacio = d.teclado(&[0; 8]).unwrap();
        assert_eq!(vacio.teclas, [0; 6]);
        assert_eq!(vacio.modificadores, 0);
    }

    #[test]
    fn teclado_con_report_id_elige_el_suyo() {
        let d = analizar(TECLADO_CON_ID).unwrap();
        assert_eq!(d.tipo, Tipo::Teclado);
        assert!(d.usa_ids);
        assert_eq!(d.id, 1);
        let r = d
            .teclado(&[0x01, 0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00])
            .unwrap();
        assert_eq!(r.modificadores, 0x01);
        assert_eq!(r.teclas[0], 0x04);
        // Un informe del otro Report ID no se interpreta como teclado.
        assert!(d.teclado(&[0x02, 0x00, 0x00]).is_none());
        assert!(!d.es_suyo(&[0x02, 0x00]));
        assert!(d.es_suyo(&[0x01, 0x00]));
    }

    #[test]
    fn raton_lee_botones_y_signos() {
        let d = analizar(RATON).unwrap();
        assert_eq!(d.tipo, Tipo::Raton);
        assert!(d.usa_ids);
        assert_eq!(d.id, 2);
        let r = d.raton(&[0x02, 0x05, 0xF6, 0x0A, 0x00]).unwrap();
        assert_eq!(r.botones, 0x05);
        assert_eq!(r.dx, -10);
        assert_eq!(r.dy, 10);
        // La rueda y los botones de mas no cambian el resultado.
        let r = d.raton(&[0x02, 0x07, 0x01, 0xFF, 0x7F]).unwrap();
        assert_eq!(r.botones, 0x07);
        assert_eq!(r.dx, 1);
        assert_eq!(r.dy, -1);
    }

    #[test]
    fn raton_de_otro_id_no_vale() {
        let d = analizar(RATON).unwrap();
        assert!(d.raton(&[0x01, 0x00, 0x00, 0x00]).is_none());
    }

    #[test]
    fn puntero_absoluto_no_es_raton() {
        // Usage Pointer + X/Y absolutos (sin flag relativo).
        let d = analizar(&[
            0x05, 0x01, 0x09, 0x01, 0xA1, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x00, 0x26, 0xFF,
            0x0F, 0x75, 0x10, 0x95, 0x02, 0x81, 0x02, 0xC0,
        ])
        .unwrap();
        assert_eq!(d.tipo, Tipo::Desconocido);
        assert!(d.raton(&[0x00, 0x01, 0x00, 0x02]).is_none());

        // Aunque se anuncie como "Mouse", si X/Y son absolutos no se mueve el
        // cursor con ellos: es una tableta.
        let d = analizar(&[
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x00, 0x26, 0xFF,
            0x0F, 0x75, 0x10, 0x95, 0x02, 0x81, 0x02, 0xC0,
        ])
        .unwrap();
        assert_eq!(d.tipo, Tipo::Desconocido);
    }

    #[test]
    fn push_pop_restaura_la_pagina() {
        // Sin el Pop, los botones caerian en la pagina de escritorio; con el,
        // vuelven a la pagina de botones.
        let d = analizar(&[
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0xA4, 0x05, 0x09, 0x19, 0x01, 0x29, 0x03,
            0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x03, 0x81, 0x02, 0xB4, 0x05, 0x01,
            0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x02, 0x81,
            0x06, 0xC0,
        ])
        .unwrap();
        assert_eq!(d.tipo, Tipo::Raton);
        let r = d.raton(&[0x01, 0x01, 0x00]).unwrap();
        assert_eq!(r.botones, 1);
    }

    #[test]
    fn item_largo_se_salta() {
        let mut desc = [0u8; 64];
        let mut n = 0;
        // Logical Maximum 0xFF (26 FF 00) y un item largo de 3 bytes.
        for b in [0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15,
                  0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02] {
            desc[n] = b;
            n += 1;
        }
        for b in [0xFE, 0x03, 0x10, 0xAA, 0xBB, 0xCC] {
            desc[n] = b;
            n += 1;
        }
        for b in [0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x19, 0x00, 0x2A, 0xFF,
                  0x00, 0x81, 0x00, 0xC0] {
            desc[n] = b;
            n += 1;
        }
        let d = analizar(&desc[..n]).unwrap();
        assert_eq!(d.tipo, Tipo::Teclado);
        let r = d.teclado(&[0x00, 0x04, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(r.teclas[0], 0x04);
    }

    #[test]
    fn mas_de_seis_teclas_avisa_de_rollover() {
        // Matriz de 8 teclas, como la de algunos teclados.
        let desc = [
            0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0x00, 0x2A, 0xFF, 0x00, 0x15,
            0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x08, 0x81, 0x00, 0xC0,
        ];
        let d = analizar(&desc).unwrap();
        let r = d
            .teclado(&[0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B])
            .unwrap();
        assert!(r.hay_rollover());
        assert_eq!(r.teclas[0], USO_ERROR_ROLLOVER);
    }

    #[test]
    fn informes_cortos_se_descartan() {
        let d = analizar(TECLADO_BOOT).unwrap();
        assert!(d.teclado(&[0x00, 0x01]).is_none());
        assert!(d.teclado(&[]).is_none());
        let m = analizar(RATON).unwrap();
        assert!(m.raton(&[0x02, 0x00]).is_none());
    }

    #[test]
    fn descriptor_truncado_no_vale() {
        assert_eq!(analizar(&[]), Err(Error::Truncado));
        assert_eq!(analizar(&[0x05]), Err(Error::Truncado));
        assert_eq!(analizar(&[0x26, 0xFF]), Err(Error::Truncado));
        assert_eq!(analizar(&[0xFE, 0x05, 0x01, 0x00]), Err(Error::Truncado));
    }

    #[test]
    fn colecciones_descuadradas_no_valen() {
        assert_eq!(analizar(&[0xC0]), Err(Error::ColeccionSinCerrar));
        assert_eq!(
            analizar(&[
                0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0x00, 0x29, 0x01, 0x75,
                0x01, 0x95, 0x01, 0x81, 0x02,
            ]),
            Err(Error::ColeccionAbierta)
        );
    }

    #[test]
    fn pila_descuadrada_no_vale() {
        assert_eq!(analizar(&[0xB4]), Err(Error::PilaVacia));
        let mut desc = [0u8; 24];
        let mut n = 0;
        for _ in 0..5 {
            desc[n] = 0xA4;
            n += 1;
        }
        assert_eq!(analizar(&desc[..n]), Err(Error::PilaLlena));
    }

    #[test]
    fn report_id_cero_no_vale() {
        assert_eq!(
            analizar(&[0x85, 0x00, 0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0xC0]),
            Err(Error::IdInvalido)
        );
    }

    #[test]
    fn demasiados_campos_no_valen() {
        // 49 botones de un bit.
        let desc = [
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x05, 0x09, 0x19, 0x01, 0x29, 0x31, 0x15, 0x00,
            0x25, 0x01, 0x75, 0x01, 0x95, 0x31, 0x81, 0x02, 0xC0,
        ];
        assert_eq!(analizar(&desc), Err(Error::DemasiadosCampos));
    }

    #[test]
    fn informe_demasiado_grande_no_vale() {
        // 8 bits x 200 = 1600 bits > 512.
        let desc = [
            0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0x00, 0x2A, 0xFF, 0x00, 0x75,
            0x08, 0x95, 0xC8, 0x81, 0x00, 0xC0,
        ];
        assert_eq!(analizar(&desc), Err(Error::InformeDemasiadoGrande));
    }

    #[test]
    fn tamano_cero_no_vale() {
        let desc = [
            0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x09, 0x04, 0x75, 0x00, 0x95, 0x01,
            0x81, 0x02, 0xC0,
        ];
        assert_eq!(analizar(&desc), Err(Error::TamanoInvalido));
    }

    #[test]
    fn sin_campos_de_entrada_no_vale() {
        let desc = [0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0xC0];
        assert_eq!(analizar(&desc), Err(Error::SinCampos));
    }

    #[test]
    fn campo_de_32_bits_se_lee_con_signo() {
        // Un eje X de 32 bits con signo.
        let desc = [
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x09, 0x30, 0x09, 0x31, 0x16, 0x00, 0x80, 0x26,
            0xFF, 0x7F, 0x75, 0x20, 0x95, 0x02, 0x81, 0x06, 0xC0,
        ];
        let d = analizar(&desc).unwrap();
        assert_eq!(d.tipo, Tipo::Raton);
        let r = d
            .raton(&[0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0x00, 0x00, 0x00])
            .unwrap();
        assert_eq!(r.dx, -1);
        assert_eq!(r.dy, 2);
    }
}
