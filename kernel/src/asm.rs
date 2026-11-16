//! El ensamblador de rocky: traduce un fuente de texto a un ejecutable `.rok`.
//!
//! El lenguaje es deliberadamente **pequeno, regular y sin sorpresas** para que
//! resulte comodo de escribir tanto a una persona como a una IA. Una linea, una
//! instruccion; las etiquetas terminan en `:`; los textos van entre comillas y
//! las variables y etiquetas tienen nombre propio (el ensamblador reparte los
//! huecos de memoria por ti).
//!
//! ```text
//! ; hola.rka - el "Hola, mundo" de rocky
//!     PUSHS "Hola, mundo!"
//!     CALL println
//!     PUSHI 0
//!     CALL exit
//! ```
//!
//! Un bucle con variable:
//!
//! ```text
//!     PUSHI 1
//!     STORE i
//! bucle:
//!     LOAD i
//!     PUSHI 5
//!     LE
//!     JZ fin
//!     PUSHS "vuelta "
//!     LOAD i
//!     ADD
//!     CALL println
//!     LOAD i
//!     PUSHI 1
//!     ADD
//!     STORE i
//!     JMP bucle
//! fin:
//!     PUSHI 0
//!     CALL exit
//! ```
//!
//! Consulta el README para la lista completa de instrucciones y de funciones de
//! la API.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::rok::Programa;
use crate::vm;

/// Que tipo de operando lleva una instruccion.
enum Operando {
    /// Ninguno.
    Ninguno,
    /// Un numero entero inmediato (`PUSHI`).
    Inmediato,
    /// Un texto entre comillas (`PUSHS`).
    Texto,
    /// Una etiqueta (`JMP`, `JZ`, `JNZ`, `CALLF`).
    Etiqueta,
    /// El nombre de una variable (`STORE`, `LOAD`).
    Variable,
    /// El nombre de una funcion de la API (`CALL`).
    Funcion,
}

/// Una instruccion ya analizada y lista para emitir.
enum Item {
    Simple(u8),
    Inmediato(u8, i64),
    Texto(u8, u32),
    Etiqueta(u8, String),
    Variable(u8, u32),
    Funcion(u8),
}

impl Item {
    /// Tamano en bytes que ocupara al emitirse.
    fn tamano(&self) -> usize {
        match self {
            Item::Simple(_) => 1,
            Item::Inmediato(_, _) => 9,
            Item::Texto(_, _) => 5,
            Item::Etiqueta(_, _) => 5,
            Item::Variable(_, _) => 5,
            Item::Funcion(_) => 2,
        }
    }
}

/// Ensambla un fuente y devuelve el programa, o un error con la linea.
pub fn ensamblar(fuente: &str) -> Result<Programa, String> {
    let mut etiquetas: BTreeMap<String, u32> = BTreeMap::new();
    let mut variables: BTreeMap<String, u32> = BTreeMap::new();
    let mut textos: Vec<String> = Vec::new();
    let mut items: Vec<(Item, usize)> = Vec::new();
    let mut offset: u32 = 0;

    for (i, bruta) in fuente.lines().enumerate() {
        let nlinea = i + 1;
        let linea = quitar_comentario(bruta).trim().to_string();
        if linea.is_empty() {
            continue;
        }

        // ¿Es una etiqueta?
        if let Some(resto) = linea.strip_suffix(':') {
            let nombre = resto.trim();
            if nombre.is_empty() || nombre.contains(char::is_whitespace) {
                return Err(format!(
                    "linea {}: etiqueta no valida: '{}'",
                    nlinea, nombre
                ));
            }
            if etiquetas.contains_key(nombre) {
                return Err(format!(
                    "linea {}: la etiqueta '{}' ya existe",
                    nlinea, nombre
                ));
            }
            etiquetas.insert(nombre.to_string(), offset);
            continue;
        }

        let (mn, resto) = partir(&linea);
        let mn = mn.to_uppercase();
        let (op, clase) = match mnemonicos(&mn) {
            Some(x) => x,
            None => {
                return Err(format!(
                    "linea {}: instruccion desconocida: '{}'",
                    nlinea, mn
                ))
            }
        };

        let item = match clase {
            Operando::Ninguno => {
                if !resto.trim().is_empty() {
                    return Err(format!("linea {}: '{}' no lleva operando", nlinea, mn));
                }
                Item::Simple(op)
            }
            Operando::Inmediato => {
                let arg = resto.trim();
                let v = parsear_entero(arg)
                    .ok_or_else(|| format!("linea {}: numero no valido: '{}'", nlinea, arg))?;
                Item::Inmediato(op, v)
            }
            Operando::Texto => {
                let arg = resto.trim();
                let s = parsear_texto(arg)
                    .ok_or_else(|| format!("linea {}: texto no valido: {}", nlinea, arg))?;
                let idx = textos.len() as u32;
                textos.push(s);
                Item::Texto(op, idx)
            }
            Operando::Etiqueta => {
                let l = resto.trim();
                if l.is_empty() {
                    return Err(format!("linea {}: falta la etiqueta", nlinea));
                }
                Item::Etiqueta(op, l.to_string())
            }
            Operando::Variable => {
                let n = resto.trim();
                if n.is_empty() {
                    return Err(format!("linea {}: falta el nombre de la variable", nlinea));
                }
                let slot = match variables.get(n) {
                    Some(s) => *s,
                    None => {
                        let s = variables.len() as u32;
                        variables.insert(n.to_string(), s);
                        s
                    }
                };
                Item::Variable(op, slot)
            }
            Operando::Funcion => {
                let n = resto.trim().to_lowercase();
                match vm::api_codigo(&n) {
                    Some(c) => Item::Funcion(c),
                    None => {
                        return Err(format!(
                            "linea {}: funcion de API desconocida: '{}'",
                            nlinea, n
                        ))
                    }
                }
            }
        };

        offset += item.tamano() as u32;
        items.push((item, nlinea));
    }

    // Segunda pasada: emitir el bytecode resolviendo etiquetas.
    let mut codigo: Vec<u8> = Vec::with_capacity(offset as usize);
    let mut pendientes: Vec<(usize, String, usize)> = Vec::new();

    for (item, nlinea) in &items {
        match item {
            Item::Simple(op) => codigo.push(*op),
            Item::Inmediato(op, v) => {
                codigo.push(*op);
                codigo.extend_from_slice(&v.to_le_bytes());
            }
            Item::Texto(op, idx) => {
                codigo.push(*op);
                codigo.extend_from_slice(&idx.to_le_bytes());
            }
            Item::Variable(op, slot) => {
                codigo.push(*op);
                codigo.extend_from_slice(&slot.to_le_bytes());
            }
            Item::Funcion(c) => {
                codigo.push(vm::op::CALL);
                codigo.push(*c);
            }
            Item::Etiqueta(op, label) => {
                codigo.push(*op);
                pendientes.push((codigo.len(), label.clone(), *nlinea));
                codigo.extend_from_slice(&0u32.to_le_bytes());
            }
        }
    }

    for (pos, label, nlinea) in pendientes {
        let destino = etiquetas
            .get(label.as_str())
            .ok_or_else(|| format!("linea {}: etiqueta desconocida: '{}'", nlinea, label))?;
        codigo[pos..pos + 4].copy_from_slice(&destino.to_le_bytes());
    }

    Ok(Programa::nuevo(codigo, textos))
}

/// Devuelve el opcode y el tipo de operando de un mnemonico (en mayusculas).
fn mnemonicos(mn: &str) -> Option<(u8, Operando)> {
    use vm::op::*;
    let x = match mn {
        "NOP" => (NOP, Operando::Ninguno),
        "PUSHN" | "PUSHNIL" => (PUSHN, Operando::Ninguno),
        "PUSHI" | "PUSH" => (PUSHI, Operando::Inmediato),
        "PUSHS" | "PUSHT" => (PUSHS, Operando::Texto),
        "POP" | "DROP" => (POP, Operando::Ninguno),
        "DUP" => (DUP, Operando::Ninguno),
        "SWAP" => (SWAP, Operando::Ninguno),
        "ADD" | "SUM" => (ADD, Operando::Ninguno),
        "SUB" => (SUB, Operando::Ninguno),
        "MUL" => (MUL, Operando::Ninguno),
        "DIV" => (DIV, Operando::Ninguno),
        "MOD" => (MOD, Operando::Ninguno),
        "NEG" => (NEG, Operando::Ninguno),
        "EQ" => (EQ, Operando::Ninguno),
        "NE" => (NE, Operando::Ninguno),
        "LT" => (LT, Operando::Ninguno),
        "LE" => (LE, Operando::Ninguno),
        "GT" => (GT, Operando::Ninguno),
        "GE" => (GE, Operando::Ninguno),
        "AND" => (AND, Operando::Ninguno),
        "OR" => (OR, Operando::Ninguno),
        "NOT" => (NOT, Operando::Ninguno),
        "JMP" | "GOTO" => (JMP, Operando::Etiqueta),
        "JZ" => (JZ, Operando::Etiqueta),
        "JNZ" => (JNZ, Operando::Etiqueta),
        "STORE" | "SET" => (STORE, Operando::Variable),
        "LOAD" | "GET" => (LOAD, Operando::Variable),
        "CALL" | "SYS" => (CALL, Operando::Funcion),
        "CALLF" | "CALLFUNC" => (CALLF, Operando::Etiqueta),
        "RET" => (RET, Operando::Ninguno),
        "HALT" | "END" | "STOP" => (HALT, Operando::Ninguno),
        _ => return None,
    };
    Some(x)
}

/// Parte una linea en (primera palabra, resto).
fn partir(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// Elimina un comentario `;` o `#` (respetando comillas y escapes).
fn quitar_comentario(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut en_comilla = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if escape {
            escape = false;
            i += 1;
            continue;
        }
        match c {
            b'\\' if en_comilla => escape = true,
            b'"' => en_comilla = !en_comilla,
            b';' | b'#' if !en_comilla => return &s[..i],
            _ => {}
        }
        i += 1;
    }
    s
}

/// Analiza un numero entero (decimal, hexadecimal `0x`, binario `0b`, octal `0o`).
fn parsear_entero(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (neg, cuerpo) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    };
    let valor = if let Some(h) = cuerpo
        .strip_prefix("0x")
        .or_else(|| cuerpo.strip_prefix("0X"))
    {
        i64::from_str_radix(h, 16).ok()?
    } else if let Some(b) = cuerpo
        .strip_prefix("0b")
        .or_else(|| cuerpo.strip_prefix("0B"))
    {
        i64::from_str_radix(b, 2).ok()?
    } else if let Some(o) = cuerpo
        .strip_prefix("0o")
        .or_else(|| cuerpo.strip_prefix("0O"))
    {
        i64::from_str_radix(o, 8).ok()?
    } else {
        cuerpo.parse::<i64>().ok()?
    };
    Some(if neg { -valor } else { valor })
}

/// Analiza un texto entre comillas dobles, con los escapes `\n \t \r \0 \\ \"`.
fn parsear_texto(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut escape = false;
    let mut cierre = None;
    for (i, c) in s.char_indices().skip(1) {
        if escape {
            match c {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                otro => {
                    out.push('\\');
                    out.push(otro);
                }
            }
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if c == '"' {
            cierre = Some(i);
            break;
        }
        out.push(c);
    }
    let cierre = cierre?;
    if !s[cierre + 1..].trim().is_empty() {
        return None;
    }
    Some(out)
}
