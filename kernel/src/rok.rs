//! El formato de ejecutable de rocky: los ficheros `.rok`.
//!
//! Un `.rok` es el equivalente al `.exe` de Windows o a un binario ELF en Linux,
//! pero con un contenedor propio, minimalista y estable. Cualquier persona o
//! programa (incluidas las IA) puede generarlo con el ensamblador de `asm.rs`.
//!
//! La cabecera ocupa 32 bytes y todo va en little-endian:
//!
//! ```text
//!   offset  tam  campo
//!   0       4    magia = "ROK1"
//!   4       2    version del formato (1)
//!   6       2    tamano de la cabecera (32)
//!   8       4    punto de entrada (offset dentro del codigo)
//!   12      4    offset del codigo
//!   16      4    longitud del codigo
//!   20      4    offset de la tabla de textos
//!   24      4    longitud de la tabla de textos
//!   28      4    reservado (0)
//! ```
//!
//! Tras la cabecera van, en este orden, la **seccion de codigo** (el bytecode que
//! ejecuta `vm.rs`) y la **tabla de textos**, una sucesion de entradas
//! `[u32 longitud][bytes]` que se recorre hasta agotar la seccion.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Firma que identifica un ejecutable `.rok`.
pub const MAGIA: &[u8; 4] = b"ROK1";
/// Version actual del formato.
pub const VERSION: u16 = 1;
/// Extension habitual de estos ficheros.
pub const EXTENSION: &str = "rok";
/// Tamano de la cabecera, en bytes.
pub const CABECERA: usize = 32;

/// Un programa `.rok` cargado en memoria, listo para ejecutar.
pub struct Programa {
    /// El bytecode.
    pub codigo: Vec<u8>,
    /// La tabla de textos constantes.
    pub textos: Vec<String>,
    /// Punto de entrada (offset dentro de `codigo`).
    pub entrada: u32,
}

impl Programa {
    /// Crea un programa a partir de su codigo y sus textos.
    pub fn nuevo(codigo: Vec<u8>, textos: Vec<String>) -> Programa {
        Programa {
            codigo,
            textos,
            entrada: 0,
        }
    }

    /// Comprueba si unos bytes empiezan por la firma `.rok`.
    pub fn es_rok(datos: &[u8]) -> bool {
        datos.len() >= 4 && &datos[0..4] == MAGIA
    }

    /// Serializa el programa al formato `.rok`.
    pub fn serializar(&self) -> Vec<u8> {
        // La tabla de textos: [u32 longitud][bytes] por cada texto.
        let mut tabla: Vec<u8> = Vec::new();
        for t in &self.textos {
            tabla.extend_from_slice(&(t.len() as u32).to_le_bytes());
            tabla.extend_from_slice(t.as_bytes());
        }

        let off_textos = (CABECERA + self.codigo.len()) as u32;
        let mut datos = Vec::with_capacity(CABECERA + self.codigo.len() + tabla.len());
        datos.extend_from_slice(MAGIA);
        datos.extend_from_slice(&VERSION.to_le_bytes());
        datos.extend_from_slice(&(CABECERA as u16).to_le_bytes());
        datos.extend_from_slice(&self.entrada.to_le_bytes());
        datos.extend_from_slice(&(CABECERA as u32).to_le_bytes());
        datos.extend_from_slice(&(self.codigo.len() as u32).to_le_bytes());
        datos.extend_from_slice(&off_textos.to_le_bytes());
        datos.extend_from_slice(&(tabla.len() as u32).to_le_bytes());
        datos.extend_from_slice(&0u32.to_le_bytes());
        datos.extend_from_slice(&self.codigo);
        datos.extend_from_slice(&tabla);
        datos
    }

    /// Carga un programa desde bytes; devuelve un mensaje legible si falla.
    ///
    /// Todo se valida con cuidado (tamanos, offsets y UTF-8) para que un fichero
    /// corrupto no pueda tumbar el kernel.
    pub fn cargar(datos: &[u8]) -> Result<Programa, String> {
        if datos.len() < CABECERA {
            return Err(String::from("demasiado pequeno para ser un .rok"));
        }
        if &datos[0..4] != MAGIA {
            return Err(String::from("no empieza por la firma ROK1"));
        }
        let version = u16::from_le_bytes([datos[4], datos[5]]);
        if version != VERSION {
            return Err(format!("version de .rok no soportada: {}", version));
        }
        let entrada = u32::from_le_bytes(datos[8..12].try_into().unwrap());
        let off_cod = u32::from_le_bytes(datos[12..16].try_into().unwrap()) as usize;
        let len_cod = u32::from_le_bytes(datos[16..20].try_into().unwrap()) as usize;
        let off_txt = u32::from_le_bytes(datos[20..24].try_into().unwrap()) as usize;
        let len_txt = u32::from_le_bytes(datos[24..28].try_into().unwrap()) as usize;

        let dentro =
            |off: usize, len: usize| off.checked_add(len).map_or(false, |e| e <= datos.len());
        if !dentro(off_cod, len_cod) || !dentro(off_txt, len_txt) {
            return Err(String::from("una seccion apunta fuera del fichero"));
        }

        let codigo = datos[off_cod..off_cod + len_cod].to_vec();

        let mut textos = Vec::new();
        let mut p = off_txt;
        let fin = off_txt + len_txt;
        while p + 4 <= fin {
            let n = u32::from_le_bytes(datos[p..p + 4].try_into().unwrap()) as usize;
            p += 4;
            if p + n > fin {
                return Err(String::from("tabla de textos corrupta"));
            }
            let texto = core::str::from_utf8(&datos[p..p + n])
                .map_err(|_| String::from("un texto no es UTF-8 valido"))?
                .into();
            textos.push(texto);
            p += n;
        }

        Ok(Programa {
            codigo,
            textos,
            entrada,
        })
    }
}
