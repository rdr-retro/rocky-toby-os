//! La maquina virtual de rocky: el motor que ejecuta los programas `.rok`.
//!
//! Es una maquina de pila pequena, determinista y facil de entender. El
//! ensamblador (`asm.rs`) traduce un fuente de texto a las instrucciones que se
//! ejecutan aqui; el contenedor `.rok` lo define `rok.rs`.
//!
//! Las operaciones que tocan el sistema (consola, ficheros, reloj, sonido...) no
//! son instrucciones especiales: son llamadas a la **API** de funciones que se
//! describe mas abajo. Asi el repertorio de instrucciones se mantiene minimo y
//! ampliar la API no obliga a cambiar el lenguaje.
//!
//! # API (nombres en ingles)
//!
//! | Funcion                        | Devuelve      | Descripcion                        |
//! |--------------------------------|---------------|------------------------------------|
//! | `print(x)`                     | -             | escribe `x` sin salto de linea     |
//! | `println(x)`                   | -             | escribe `x` y un salto de linea    |
//! | `readline()`                   | texto         | lee una linea del teclado          |
//! | `readint()`                    | entero        | lee una linea y la convierte       |
//! | `readfile(ruta)`               | texto o nil   | contenido de un fichero            |
//! | `writefile(ruta, texto)`       | entero        | escribe (sobrescribe); -1 si falla |
//! | `appendfile(ruta, texto)`      | entero        | anade al final; -1 si falla        |
//! | `exists(ruta)`                 | 1 / 0         | si la ruta existe                  |
//! | `remove(ruta)`                 | 0 / -1        | borra fichero o directorio vacio   |
//! | `mkdir(ruta)`                  | 0 / -1        | crea un directorio                 |
//! | `listdir(ruta)`                | texto o nil   | nombres separados por `\n`         |
//! | `len(texto)`                   | entero        | numero de caracteres               |
//! | `str(x)`                       | texto         | convierte a texto                  |
//! | `int(texto)`                   | entero        | convierte a entero (0 si no puede) |
//! | `substr(texto, ini, n)`        | texto         | subcadena                          |
//! | `upper(texto)` / `lower(texto)`| texto         | mayusculas / minusculas            |
//! | `find(texto, sub)`             | entero        | posicion de `sub`, o -1            |
//! | `chr(codigo)` / `ord(texto)`   | texto/entero  | caracter <-> codigo                |
//! | `abs(n)`, `min(a,b)`, `max(a,b)`| entero       | matematicas basicas                |
//! | `random(max)`                  | entero        | entero en `[0, max)`               |
//! | `exit(codigo)`                 | -             | termina el programa                |
//! | `ticks()`                      | entero        | milisegundos desde el arranque     |
//! | `args()`                       | texto         | argumentos separados por espacios  |
//! | `argc()` / `arg(i)`            | entero/texto  | numero y acceso a argumentos       |
//! | `exec(orden)`                  | entero        | ejecuta una orden del shell        |
//! | `sleep(ms)`                    | 0             | espera                             |
//! | `time()` / `date()`            | texto         | hora `HH:MM:SS` y fecha `AAAA-MM-DD`|
//! | `beep(hz, ms)`                 | 0             | tono con el altavoz interno        |
//! | `msleep(ms)`                   | 0             | espera (milisegundos)              |

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::cmos;
use crate::console;
use crate::fs::{self, Error, Tipo};
use crate::input;
use crate::pit;
use crate::rok::Programa;
use crate::sound;
use crate::{print, println};

/// Repertorio de instrucciones (opcodes).
pub mod op {
    pub const NOP: u8 = 0x00;
    pub const PUSHN: u8 = 0x01;
    pub const PUSHI: u8 = 0x02;
    pub const PUSHS: u8 = 0x03;
    pub const POP: u8 = 0x04;
    pub const DUP: u8 = 0x05;
    pub const SWAP: u8 = 0x06;

    pub const ADD: u8 = 0x10;
    pub const SUB: u8 = 0x11;
    pub const MUL: u8 = 0x12;
    pub const DIV: u8 = 0x13;
    pub const MOD: u8 = 0x14;
    pub const NEG: u8 = 0x15;

    pub const EQ: u8 = 0x20;
    pub const NE: u8 = 0x21;
    pub const LT: u8 = 0x22;
    pub const LE: u8 = 0x23;
    pub const GT: u8 = 0x24;
    pub const GE: u8 = 0x25;

    pub const AND: u8 = 0x30;
    pub const OR: u8 = 0x31;
    pub const NOT: u8 = 0x32;

    pub const JMP: u8 = 0x40;
    pub const JZ: u8 = 0x41;
    pub const JNZ: u8 = 0x42;

    pub const STORE: u8 = 0x50;
    pub const LOAD: u8 = 0x51;

    pub const CALL: u8 = 0x60;
    pub const CALLF: u8 = 0x61;
    pub const RET: u8 = 0x62;

    pub const HALT: u8 = 0x70;
}

/// Tabla de la API: nombre -> codigo. La usan el ensamblador y la maquina.
pub const APIS: &[(&str, u8)] = &[
    ("print", 0),
    ("println", 1),
    ("readline", 2),
    ("readint", 3),
    ("readfile", 4),
    ("writefile", 5),
    ("appendfile", 6),
    ("exists", 7),
    ("remove", 8),
    ("mkdir", 9),
    ("listdir", 10),
    ("len", 11),
    ("str", 12),
    ("int", 13),
    ("substr", 14),
    ("upper", 15),
    ("lower", 16),
    ("find", 17),
    ("chr", 18),
    ("ord", 19),
    ("abs", 20),
    ("min", 21),
    ("max", 22),
    ("random", 23),
    ("exit", 24),
    ("ticks", 25),
    ("args", 26),
    ("argc", 27),
    ("arg", 28),
    ("exec", 29),
    ("sleep", 30),
    ("time", 31),
    ("date", 32),
    ("beep", 33),
    ("msleep", 34),
];

/// Devuelve el codigo de una funcion de la API a partir de su nombre.
pub fn api_codigo(nombre: &str) -> Option<u8> {
    APIS.iter().find(|(n, _)| *n == nombre).map(|(_, c)| *c)
}

/// Numero maximo de instrucciones antes de rendirse (evita bucles infinitos).
const LIMITE_PASOS: u64 = 50_000_000;

/// Un valor de la pila de la maquina.
#[derive(Clone)]
pub enum Valor {
    /// Ausencia de valor.
    Nulo,
    /// Numero entero de 64 bits.
    Entero(i64),
    /// Cadena de texto.
    Texto(String),
}

impl Valor {
    /// Convierte el valor a texto para imprimirlo o concatenarlo.
    fn a_texto(&self) -> String {
        match self {
            Valor::Nulo => String::from("nil"),
            Valor::Entero(n) => n.to_string(),
            Valor::Texto(t) => t.clone(),
        }
    }

    /// Convierte el valor a entero (0 si no se puede).
    fn a_entero(&self) -> i64 {
        match self {
            Valor::Nulo => 0,
            Valor::Entero(n) => *n,
            Valor::Texto(t) => t.trim().parse().unwrap_or(0),
        }
    }

    /// Interpretacion booleana: 0, la cadena vacia y nil son falsos.
    fn verdad(&self) -> bool {
        match self {
            Valor::Nulo => false,
            Valor::Entero(n) => *n != 0,
            Valor::Texto(t) => !t.is_empty(),
        }
    }

    /// Simbolo de texto para los operadores de comparacion.
    fn a_simbolo(&self) -> String {
        match self {
            Valor::Entero(n) => n.to_string(),
            Valor::Texto(t) => t.clone(),
            Valor::Nulo => String::from("nil"),
        }
    }
}

/// Estado de una ejecucion.
struct Maquina<'a> {
    prog: &'a Programa,
    pila: Vec<Valor>,
    globales: Vec<Valor>,
    pila_ret: Vec<usize>,
    pc: usize,
    salida: Option<i64>,
    pasos: u64,
    cwd: u32,
    args: Vec<String>,
}

impl<'a> Maquina<'a> {
    fn byte(&mut self) -> u8 {
        let b = self.prog.codigo.get(self.pc).copied().unwrap_or(0);
        self.pc += 1;
        b
    }

    fn u32(&mut self) -> u32 {
        let mut a = [0u8; 4];
        for (i, x) in a.iter_mut().enumerate() {
            *x = self.prog.codigo.get(self.pc + i).copied().unwrap_or(0);
        }
        self.pc += 4;
        u32::from_le_bytes(a)
    }

    fn i64(&mut self) -> i64 {
        let mut a = [0u8; 8];
        for (i, x) in a.iter_mut().enumerate() {
            *x = self.prog.codigo.get(self.pc + i).copied().unwrap_or(0);
        }
        self.pc += 8;
        i64::from_le_bytes(a)
    }

    fn pop(&mut self) -> Valor {
        self.pila.pop().unwrap_or(Valor::Nulo)
    }

    fn pop_texto(&mut self) -> String {
        self.pop().a_texto()
    }

    fn pop_entero(&mut self) -> i64 {
        self.pop().a_entero()
    }

    fn push(&mut self, v: Valor) {
        self.pila.push(v);
    }

    fn set_global(&mut self, s: usize, v: Valor) {
        if self.globales.len() <= s {
            self.globales.resize(s + 1, Valor::Nulo);
        }
        self.globales[s] = v;
    }

    fn get_global(&self, s: usize) -> Valor {
        self.globales.get(s).cloned().unwrap_or(Valor::Nulo)
    }

    fn correr(&mut self, exec: &mut dyn FnMut(&str)) -> i64 {
        while self.pc < self.prog.codigo.len() {
            self.pasos += 1;
            if self.pasos > LIMITE_PASOS {
                println!("vm: limite de pasos superado (posible bucle infinito)");
                return -1;
            }
            let op = self.byte();
            match op {
                op::NOP => {}
                op::PUSHN => self.push(Valor::Nulo),
                op::PUSHI => {
                    let v = self.i64();
                    self.push(Valor::Entero(v));
                }
                op::PUSHS => {
                    let i = self.u32() as usize;
                    let t = self.prog.textos.get(i).cloned().unwrap_or_default();
                    self.push(Valor::Texto(t));
                }
                op::POP => {
                    self.pop();
                }
                op::DUP => {
                    let v = self.pila.last().cloned().unwrap_or(Valor::Nulo);
                    self.push(v);
                }
                op::SWAP => {
                    let n = self.pila.len();
                    if n >= 2 {
                        self.pila.swap(n - 1, n - 2);
                    }
                }
                op::ADD => {
                    let b = self.pop();
                    let a = self.pop();
                    if let (Valor::Entero(x), Valor::Entero(y)) = (&a, &b) {
                        self.push(Valor::Entero(x.wrapping_add(*y)));
                    } else {
                        let s = format!("{}{}", a.a_texto(), b.a_texto());
                        self.push(Valor::Texto(s));
                    }
                }
                op::SUB => self.binaria(|a, b| a.wrapping_sub(b)),
                op::MUL => self.binaria(|a, b| a.wrapping_mul(b)),
                op::DIV => self.binaria(|a, b| if b == 0 { 0 } else { a.wrapping_div(b) }),
                op::MOD => self.binaria(|a, b| if b == 0 { 0 } else { a.wrapping_rem(b) }),
                op::NEG => {
                    let a = self.pop_entero();
                    self.push(Valor::Entero(a.wrapping_neg()));
                }
                op::EQ => self.comparar(|o| o == core::cmp::Ordering::Equal),
                op::NE => self.comparar(|o| o != core::cmp::Ordering::Equal),
                op::LT => self.comparar(|o| o == core::cmp::Ordering::Less),
                op::LE => self.comparar(|o| o != core::cmp::Ordering::Greater),
                op::GT => self.comparar(|o| o == core::cmp::Ordering::Greater),
                op::GE => self.comparar(|o| o != core::cmp::Ordering::Less),
                op::AND => {
                    let b = self.pop().verdad();
                    let a = self.pop().verdad();
                    self.push(Valor::Entero((a && b) as i64));
                }
                op::OR => {
                    let b = self.pop().verdad();
                    let a = self.pop().verdad();
                    self.push(Valor::Entero((a || b) as i64));
                }
                op::NOT => {
                    let a = self.pop().verdad();
                    self.push(Valor::Entero((!a) as i64));
                }
                op::JMP => {
                    let t = self.u32() as usize;
                    self.pc = t;
                }
                op::JZ => {
                    let t = self.u32() as usize;
                    let v = self.pop();
                    if !v.verdad() {
                        self.pc = t;
                    }
                }
                op::JNZ => {
                    let t = self.u32() as usize;
                    let v = self.pop();
                    if v.verdad() {
                        self.pc = t;
                    }
                }
                op::STORE => {
                    let s = self.u32() as usize;
                    let v = self.pop();
                    self.set_global(s, v);
                }
                op::LOAD => {
                    let s = self.u32() as usize;
                    let v = self.get_global(s);
                    self.push(v);
                }
                op::CALL => {
                    let c = self.byte();
                    self.api(c, exec);
                    if let Some(code) = self.salida {
                        return code;
                    }
                }
                op::CALLF => {
                    let t = self.u32() as usize;
                    self.pila_ret.push(self.pc);
                    self.pc = t;
                }
                op::RET => {
                    if let Some(t) = self.pila_ret.pop() {
                        self.pc = t;
                    } else {
                        return self.pop_entero();
                    }
                }
                op::HALT => return self.pop_entero(),
                _ => {
                    println!("vm: instruccion desconocida 0x{:02X}", op);
                    return -1;
                }
            }
        }
        0
    }

    fn binaria(&mut self, f: impl Fn(i64, i64) -> i64) {
        let b = self.pop_entero();
        let a = self.pop_entero();
        self.push(Valor::Entero(f(a, b)));
    }

    fn comparar(&mut self, f: impl Fn(core::cmp::Ordering) -> bool) {
        let b = self.pop();
        let a = self.pop();
        self.push(Valor::Entero(f(ordenar(&a, &b)) as i64));
    }

    /// Ejecuta una llamada a la API.
    fn api(&mut self, c: u8, exec: &mut dyn FnMut(&str)) {
        match c {
            0 => {
                let v = self.pop();
                print!("{}", v.a_texto());
            }
            1 => {
                let v = self.pop();
                println!("{}", v.a_texto());
            }
            2 => {
                let s = leer_linea();
                self.push(Valor::Texto(s));
            }
            3 => {
                let s = leer_linea();
                let n = s.trim().parse::<i64>().unwrap_or(0);
                self.push(Valor::Entero(n));
            }
            4 => {
                let ruta = self.pop_texto();
                match leer_fichero(self.cwd, &ruta) {
                    Some(t) => self.push(Valor::Texto(t)),
                    None => self.push(Valor::Nulo),
                }
            }
            5 => {
                let texto = self.pop_texto();
                let ruta = self.pop_texto();
                let r = escribir_fichero(self.cwd, &ruta, texto.as_bytes(), false);
                self.push(Valor::Entero(r));
            }
            6 => {
                let texto = self.pop_texto();
                let ruta = self.pop_texto();
                let r = escribir_fichero(self.cwd, &ruta, texto.as_bytes(), true);
                self.push(Valor::Entero(r));
            }
            7 => {
                let ruta = self.pop_texto();
                let v = fs::resolver(self.cwd, &ruta).is_ok() as i64;
                self.push(Valor::Entero(v));
            }
            8 => {
                let ruta = self.pop_texto();
                let v = match fs::borrar(self.cwd, &ruta) {
                    Ok(()) => 0,
                    Err(_) => -1,
                };
                self.push(Valor::Entero(v));
            }
            9 => {
                let ruta = self.pop_texto();
                let v = match fs::crear(self.cwd, &ruta, Tipo::Directorio) {
                    Ok(_) => 0,
                    Err(_) => -1,
                };
                self.push(Valor::Entero(v));
            }
            10 => {
                let ruta = self.pop_texto();
                match listar_dir(self.cwd, &ruta) {
                    Some(t) => self.push(Valor::Texto(t)),
                    None => self.push(Valor::Nulo),
                }
            }
            11 => {
                let t = self.pop_texto();
                self.push(Valor::Entero(t.chars().count() as i64));
            }
            12 => {
                let v = self.pop();
                self.push(Valor::Texto(v.a_texto()));
            }
            13 => {
                let t = self.pop_texto();
                self.push(Valor::Entero(t.trim().parse().unwrap_or(0)));
            }
            14 => {
                let n = self.pop_entero().max(0) as usize;
                let ini = self.pop_entero().max(0) as usize;
                let t = self.pop_texto();
                let sub: String = t.chars().skip(ini).take(n).collect();
                self.push(Valor::Texto(sub));
            }
            15 => {
                let t = self.pop_texto();
                self.push(Valor::Texto(t.to_uppercase()));
            }
            16 => {
                let t = self.pop_texto();
                self.push(Valor::Texto(t.to_lowercase()));
            }
            17 => {
                let sub = self.pop_texto();
                let t = self.pop_texto();
                let pos = match t.find(&sub) {
                    Some(b) => t[..b].chars().count() as i64,
                    None => -1,
                };
                self.push(Valor::Entero(pos));
            }
            18 => {
                let n = self.pop_entero();
                let s = char::from_u32(n as u32)
                    .map(String::from)
                    .unwrap_or_default();
                self.push(Valor::Texto(s));
            }
            19 => {
                let t = self.pop_texto();
                let n = t.chars().next().map(|c| c as i64).unwrap_or(0);
                self.push(Valor::Entero(n));
            }
            20 => {
                let n = self.pop_entero();
                self.push(Valor::Entero(n.wrapping_abs()));
            }
            21 => {
                let b = self.pop_entero();
                let a = self.pop_entero();
                self.push(Valor::Entero(a.min(b)));
            }
            22 => {
                let b = self.pop_entero();
                let a = self.pop_entero();
                self.push(Valor::Entero(a.max(b)));
            }
            23 => {
                let max = self.pop_entero();
                self.push(Valor::Entero(azar(max)));
            }
            24 => {
                let code = self.pop_entero();
                self.salida = Some(code);
            }
            25 => {
                let ms = pit::ticks().wrapping_mul(10) as i64;
                self.push(Valor::Entero(ms));
            }
            26 => {
                self.push(Valor::Texto(self.args.join(" ")));
            }
            27 => {
                self.push(Valor::Entero(self.args.len() as i64));
            }
            28 => {
                let i = self.pop_entero();
                let s = if i >= 0 {
                    self.args.get(i as usize).cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                self.push(Valor::Texto(s));
            }
            29 => {
                let cmd = self.pop_texto();
                if !cmd.trim().is_empty() {
                    exec(&cmd);
                }
                self.push(Valor::Entero(0));
            }
            30 => {
                let ms = self.pop_entero().max(0).min(60_000) as u32;
                sound::sleep_ms(ms);
                self.push(Valor::Entero(0));
            }
            31 => {
                let (h, m, s) = cmos::time();
                self.push(Valor::Texto(format!("{:02}:{:02}:{:02}", h, m, s)));
            }
            32 => {
                let (d, m, a) = cmos::date();
                self.push(Valor::Texto(format!("{:04}-{:02}-{:02}", a, m, d)));
            }
            33 => {
                let ms = self.pop_entero().max(0).min(60_000) as u32;
                let hz = self.pop_entero().max(0).min(20_000) as u32;
                sound::beep(hz, ms);
                self.push(Valor::Entero(0));
            }
            34 => {
                let ms = self.pop_entero().clamp(0, 60_000) as u32;
                crate::sound::sleep_ms(ms);
                self.push(Valor::Entero(0));
            }
            _ => {
                println!("vm: funcion de API desconocida ({})", c);
            }
        }
    }
}

/// Compara dos valores: numericamente si ambos son enteros, como texto si no.
fn ordenar(a: &Valor, b: &Valor) -> core::cmp::Ordering {
    match (a, b) {
        (Valor::Entero(x), Valor::Entero(y)) => x.cmp(y),
        _ => a.a_simbolo().cmp(&b.a_simbolo()),
    }
}

/// Ejecuta un programa `.rok`. Devuelve su codigo de salida.
///
/// `cwd` es el directorio de trabajo (para las rutas relativas) y `exec` permite
/// que el programa lance ordenes del interprete con `exec(...)`.
pub fn ejecutar(prog: &Programa, cwd: u32, args: &[String], exec: &mut dyn FnMut(&str)) -> i64 {
    let mut m = Maquina {
        prog,
        pila: Vec::new(),
        globales: Vec::new(),
        pila_ret: Vec::new(),
        pc: prog.entrada as usize,
        salida: None,
        pasos: 0,
        cwd,
        args: args.to_vec(),
    };
    m.correr(exec)
}

/// Lee una linea del teclado con eco y retroceso.
fn leer_linea() -> String {
    let mut s = String::new();
    loop {
        let c = input::wait();
        match c {
            b'\n' => {
                println!();
                return s;
            }
            0x08 => {
                if s.pop().is_some() {
                    console::put(0x08);
                    console::put(b' ');
                    console::put(0x08);
                }
            }
            c if (0x20..0x7F).contains(&c) => {
                s.push(c as char);
                console::put(c);
            }
            _ => {}
        }
    }
}

/// Lee un fichero completo como texto.
fn leer_fichero(cwd: u32, ruta: &str) -> Option<String> {
    let reg = fs::resolver(cwd, ruta).ok()?;
    if fs::tipo(reg) != Tipo::Fichero {
        return None;
    }
    let tam = fs::tamano(reg) as usize;
    let mut buf = vec![0u8; tam];
    let n = fs::leer(reg, 0, &mut buf);
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Escribe (o anade) texto en un fichero. Devuelve los bytes escritos o -1.
fn escribir_fichero(cwd: u32, ruta: &str, datos: &[u8], anexar: bool) -> i64 {
    let reg = match fs::resolver(cwd, ruta) {
        Ok(r) => {
            if fs::tipo(r) != Tipo::Fichero {
                return -1;
            }
            if !anexar {
                fs::truncar(r);
            }
            r
        }
        Err(Error::NoExiste) => match fs::crear(cwd, ruta, Tipo::Fichero) {
            Ok(r) => r,
            Err(_) => return -1,
        },
        Err(_) => return -1,
    };
    let offset = if anexar { fs::tamano(reg) as usize } else { 0 };
    match fs::escribir(reg, offset, datos) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Lista un directorio con un nombre por linea.
fn listar_dir(cwd: u32, ruta: &str) -> Option<String> {
    let reg = fs::resolver(cwd, ruta).ok()?;
    if fs::tipo(reg) != Tipo::Directorio {
        return None;
    }
    let mut out = String::new();
    fs::listar(reg, |e| {
        out.push_str(&e.nombre);
        out.push('\n');
    });
    Some(out)
}

/// Generador de numeros aleatorios (xorshift) sembrado con el reloj.
static SEMILLA: AtomicU64 = AtomicU64::new(0);

fn azar(max: i64) -> i64 {
    if max <= 0 {
        return 0;
    }
    let mut x = SEMILLA.load(Ordering::Relaxed);
    if x == 0 {
        x = pit::ticks() | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    SEMILLA.store(x, Ordering::Relaxed);
    (x % (max as u64)) as i64
}
