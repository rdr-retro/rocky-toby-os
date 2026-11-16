//! Analizador sintáctico de Toby UI (léxico + sintáctico).
//!
//! Formato XML-like mínimo y estricto:
//!
//! ```text
//! <toby-app name="demo">
//!   <window title="Hola" width="640" height="400">
//!     <column spacing="8" padding="16">
//!       <button id="ok" title="Aceptar" action="app.ok" />
//!     </column>
//!   </window>
//! </toby-app>
//! ```
//!
//! El parser NO valida componentes, propiedades, tipos ni jerarquía: eso es
//! trabajo de `validation.rs`. Aquí sólo se comprueba la forma (balance de
//! etiquetas, sintaxis de atributos, límites de tamaño).
//!
//! No hay expresiones ni texto suelto. Los valores entre comillas dobles
//! aceptan las cinco entidades XML y referencias numéricas de caracteres.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::document::{Document, Sintaxis};

/// Límite de tamaño del documento fuente (bytes).
pub const MAX_BYTES: usize = 64 * 1024;
/// Límite de profundidad de anidamiento de etiquetas.
pub const MAX_PROFUNDIDAD: usize = 32;
/// Límite de número de nodos del documento.
pub const MAX_NODOS: usize = 4096;
/// Límite de atributos por etiqueta.
pub const MAX_ATRIBUTOS: usize = 32;

/// Error de análisis sintáctico, con posición.
#[derive(Clone, Debug)]
pub struct ParserError {
    /// Línea (1-based).
    pub linea: usize,
    /// Columna (1-based).
    pub columna: usize,
    /// Descripción breve del problema.
    pub detalle: &'static str,
}

impl ParserError {
    /// Mensaje legible con posición.
    pub fn mensaje(&self) -> String {
        format!(
            "línea {}, columna {}: {}",
            self.linea, self.columna, self.detalle
        )
    }
}

impl core::fmt::Display for ParserError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.mensaje())
    }
}

/// Analizador sobre el texto fuente del documento.
struct Analizador<'a> {
    fuente: &'a [u8],
    pos: usize,
    linea: usize,
    columna: usize,
    nodos: usize,
}

impl<'a> Analizador<'a> {
    fn nuevo(fuente: &'a [u8]) -> Self {
        Analizador {
            fuente,
            pos: 0,
            linea: 1,
            columna: 1,
            nodos: 0,
        }
    }

    fn error(&self, detalle: &'static str) -> ParserError {
        ParserError {
            linea: self.linea,
            columna: self.columna,
            detalle,
        }
    }

    fn actual(&self) -> Option<u8> {
        self.fuente.get(self.pos).copied()
    }

    fn adelante(&self) -> Option<u8> {
        self.fuente.get(self.pos + 1).copied()
    }

    fn avanzar(&mut self) {
        if let Some(b) = self.actual() {
            self.pos += 1;
            if b == b'\n' {
                self.linea += 1;
                self.columna = 1;
            } else {
                self.columna += 1;
            }
        }
    }

    fn es_espacio(b: u8) -> bool {
        b == b' ' || b == b'\t' || b == b'\r' || b == b'\n'
    }

    /// Salta espacios en blanco y comentarios `<!-- ... -->`.
    fn saltar_ruido(&mut self) -> Result<(), ParserError> {
        loop {
            match self.actual() {
                Some(b) if Self::es_espacio(b) => self.avanzar(),
                Some(b'<')
                    if self.adelante() == Some(b'!')
                        && self.fuente.get(self.pos + 2).copied() == Some(b'-')
                        && self.fuente.get(self.pos + 3).copied() == Some(b'-') =>
                {
                    // Comentario: avanza "<!--" y busca el cierre "-->".
                    let inicio_linea = self.linea;
                    for _ in 0..4 {
                        self.avanzar();
                    }
                    loop {
                        match self.actual() {
                            None => {
                                return Err(ParserError {
                                    linea: inicio_linea,
                                    columna: self.columna,
                                    detalle: "comentario sin cerrar",
                                })
                            }
                            Some(b'-')
                                if self.adelante() == Some(b'-')
                                    && self.fuente.get(self.pos + 2).copied() == Some(b'>') =>
                            {
                                for _ in 0..3 {
                                    self.avanzar();
                                }
                                break;
                            }
                            Some(_) => self.avanzar(),
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn leer_nombre(&mut self) -> Result<String, ParserError> {
        let mut s = String::new();
        while let Some(b) = self.actual() {
            let ok = b.is_ascii_alphabetic()
                || b.is_ascii_digit()
                || b == b'-'
                || b == b'_'
                || b == b':'
                || b == b'.';
            if !ok {
                break;
            }
            s.push(b as char);
            self.avanzar();
        }
        if s.is_empty() {
            return Err(self.error("se esperaba un nombre de etiqueta"));
        }
        Ok(s)
    }

    /// Nombre de atributo: `[a-zA-Z_][a-zA-Z0-9_.-]*`.
    fn leer_nombre_attr(&mut self) -> Result<String, ParserError> {
        let mut s = String::new();
        while let Some(b) = self.actual() {
            let ok = b.is_ascii_alphabetic()
                || b == b'_'
                || (b.is_ascii_digit() && !s.is_empty())
                || b == b'-'
                || b == b'.';
            if !ok {
                break;
            }
            s.push(b as char);
            self.avanzar();
        }
        if s.is_empty() {
            return Err(self.error("se esperaba un nombre de atributo"));
        }
        Ok(s)
    }

    /// Valor entre comillas dobles. No admite `<` ni comillas dentro.
    fn leer_valor(&mut self) -> Result<String, ParserError> {
        if self.actual() != Some(b'"') {
            return Err(self.error("el valor del atributo debe ir entre comillas dobles"));
        }
        self.avanzar();
        let mut s = String::new();
        loop {
            match self.actual() {
                None => return Err(self.error("falta la comilla de cierre")),
                Some(b'"') => {
                    self.avanzar();
                    return Ok(s);
                }
                Some(b'<') => return Err(self.error("no se permite '<' dentro de un valor")),
                Some(b'&') => s.push(self.leer_entidad()?),
                Some(b) if b >= 0x80 => {
                    // UTF-8 en el valor (lo permitimos para etiquetas visibles).
                    match crate::texto::utf8_en(self.fuente, self.pos) {
                        Some((cp, n)) => {
                            match char::from_u32(cp) {
                                Some(c) => s.push(c),
                                None => {
                                    return Err(self.error("secuencia UTF-8 no válida en el valor"))
                                }
                            }
                            for _ in 0..n {
                                self.avanzar();
                            }
                        }
                        None => return Err(self.error("secuencia UTF-8 no válida en el valor")),
                    }
                }
                Some(b) => {
                    s.push(b as char);
                    self.avanzar();
                }
            }
        }
    }

    /// Lee una entidad XML básica o una referencia numérica de carácter.
    fn leer_entidad(&mut self) -> Result<char, ParserError> {
        self.avanzar(); // &
        let mut nombre = String::new();
        loop {
            match self.actual() {
                Some(b';') => {
                    self.avanzar();
                    break;
                }
                Some(b)
                    if (b.is_ascii_alphanumeric() || (b == b'#' && nombre.is_empty()))
                        && nombre.len() < 10 =>
                {
                    nombre.push(b as char);
                    self.avanzar();
                }
                _ => return Err(self.error("entidad XML no válida")),
            }
        }
        let caracter = match nombre.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if nombre.starts_with("#x") || nombre.starts_with("#X") => {
                u32::from_str_radix(&nombre[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if nombre.starts_with('#') => {
                nombre[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        caracter.ok_or_else(|| self.error("entidad XML desconocida o inválida"))
    }

    /// Atributos `nombre="valor"` hasta que aparezca `>` o `/>`.
    /// Devuelve la lista y si la etiqueta se autocerró.
    fn leer_atributos(&mut self) -> Result<(Vec<(String, String)>, bool), ParserError> {
        let mut out: Vec<(String, String)> = Vec::new();
        loop {
            self.saltar_ruido()?;
            match self.actual() {
                Some(b'>') => {
                    self.avanzar();
                    return Ok((out, false));
                }
                Some(b'/') if self.adelante() == Some(b'>') => {
                    self.avanzar();
                    self.avanzar();
                    return Ok((out, true));
                }
                None => return Err(self.error("la etiqueta no se cierra")),
                Some(_) => {
                    if out.len() >= MAX_ATRIBUTOS {
                        return Err(self.error("demasiados atributos en la etiqueta"));
                    }
                    let nombre = self.leer_nombre_attr()?;
                    self.saltar_ruido()?;
                    if self.actual() != Some(b'=') {
                        return Err(self.error("se esperaba '=' después del nombre del atributo"));
                    }
                    self.avanzar();
                    self.saltar_ruido()?;
                    let valor = self.leer_valor()?;
                    out.push((nombre, valor));
                }
            }
        }
    }

    /// Analiza un nodo `<nombre ...>...</nombre>` o `<nombre ... />`.
    fn analizar_nodo(&mut self, prof: usize) -> Result<Sintaxis, ParserError> {
        if prof > MAX_PROFUNDIDAD {
            return Err(self.error("demasiada profundidad de anidamiento"));
        }
        let linea = self.linea;
        if self.actual() != Some(b'<') {
            return Err(self.error("se esperaba '<'"));
        }
        self.avanzar();
        if self.actual() == Some(b'/') {
            return Err(self.error("etiqueta de cierre inesperada"));
        }
        let nombre = self.leer_nombre()?;
        self.nodos += 1;
        if self.nodos > MAX_NODOS {
            return Err(self.error("demasiados nodos en el documento"));
        }
        let (atributos, autocerrada) = self.leer_atributos()?;
        if autocerrada {
            return Ok(Sintaxis {
                etiqueta: nombre,
                linea,
                atributos,
                hijos: Vec::new(),
            });
        }
        // Contenido: sólo más etiquetas, nunca texto suelto.
        let mut hijos: Vec<Sintaxis> = Vec::new();
        loop {
            self.saltar_ruido()?;
            match self.actual() {
                None => return Err(self.error("falta la etiqueta de cierre")),
                Some(b'<') if self.adelante() == Some(b'/') => {
                    let linea_cierre = self.linea;
                    self.avanzar();
                    self.avanzar();
                    let cierre = self.leer_nombre()?;
                    self.saltar_ruido()?;
                    if self.actual() != Some(b'>') {
                        return Err(self.error("la etiqueta de cierre debe ser '</nombre>'"));
                    }
                    self.avanzar();
                    if cierre != nombre {
                        return Err(ParserError {
                            linea: linea_cierre,
                            columna: self.columna,
                            detalle: "las etiquetas no están balanceadas",
                        });
                    }
                    return Ok(Sintaxis {
                        etiqueta: nombre,
                        linea,
                        atributos,
                        hijos,
                    });
                }
                Some(b'<') => {
                    hijos.push(self.analizar_nodo(prof + 1)?);
                }
                Some(_) => {
                    return Err(self.error("texto suelto no permitido; usa atributos"));
                }
            }
        }
    }
}

/// Analiza un documento XML-like genérico: **un** elemento raíz con cualquier
/// nombre, etiquetas balanceadas, atributos `nombre="valor"` y los mismos
/// límites que Toby UI (`MAX_BYTES`, `MAX_PROFUNDIDAD`, `MAX_NODOS`,
/// `MAX_ATRIBUTOS`).
///
/// Devuelve la raíz en crudo. Sobre esta base se construyen tanto el documento
/// de Toby UI (`analizar_bytes`, que exige `<toby-app>`) como otros formatos
/// declarativos de Rocky con la misma sintaxis (p.ej. el manifiesto de una app
/// `.app`, en `manifest.rs`). El léxico no se duplica.
pub fn analizar_xml(bytes: &[u8]) -> Result<Sintaxis, ParserError> {
    if bytes.len() > MAX_BYTES {
        return Err(ParserError {
            linea: 1,
            columna: 1,
            detalle: "documento demasiado grande",
        });
    }
    let mut a = Analizador::nuevo(bytes);
    a.saltar_ruido()?;
    if a.actual() != Some(b'<') {
        return Err(a.error("el documento está vacío"));
    }
    let linea_raiz = a.linea;
    a.avanzar();
    let raiz = a.leer_nombre()?;
    a.nodos = 1;
    let (atributos, autocerrada) = a.leer_atributos()?;
    let mut hijos: Vec<Sintaxis> = Vec::new();
    if !autocerrada {
        loop {
            a.saltar_ruido()?;
            match a.actual() {
                None => return Err(a.error("falta la etiqueta de cierre")),
                Some(b'<') if a.adelante() == Some(b'/') => {
                    let linea_cierre = a.linea;
                    a.avanzar();
                    a.avanzar();
                    let cierre = a.leer_nombre()?;
                    a.saltar_ruido()?;
                    if a.actual() != Some(b'>') {
                        return Err(a.error("la etiqueta de cierre debe ser '</nombre>'"));
                    }
                    a.avanzar();
                    if cierre != raiz {
                        return Err(ParserError {
                            linea: linea_cierre,
                            columna: a.columna,
                            detalle: "las etiquetas no están balanceadas",
                        });
                    }
                    break;
                }
                Some(b'<') => hijos.push(a.analizar_nodo(1)?),
                Some(_) => return Err(a.error("texto suelto no permitido; usa etiquetas")),
            }
        }
        // Sólo puede quedar ruido final.
        a.saltar_ruido()?;
        if a.actual().is_some() {
            return Err(a.error("contenido inesperado después de la etiqueta de cierre"));
        }
    }
    Ok(Sintaxis {
        etiqueta: raiz,
        linea: linea_raiz,
        atributos,
        hijos,
    })
}

/// Analiza un documento Toby UI completo.
///
/// Comprueba la forma: raíz `toby-app`, balance de etiquetas, sintaxis de
/// atributos y límites de tamaño. La validación semántica es aparte.
pub fn analizar(texto: &str) -> Result<Document, ParserError> {
    analizar_bytes(texto.as_bytes())
}

/// Igual que `analizar`, pero aceptando bytes ya leídos del disco.
pub fn analizar_bytes(bytes: &[u8]) -> Result<Document, ParserError> {
    let raiz = analizar_xml(bytes)?;
    if raiz.etiqueta != "toby-app" {
        return Err(ParserError {
            linea: raiz.linea,
            columna: 1,
            detalle: "la raíz del documento debe ser <toby-app>",
        });
    }
    // El nombre es sólo un espejo del atributo `name` de la raíz: quien manda
    // es la propiedad tipada (`validation`).
    let nombre = raiz
        .atributos
        .iter()
        .find(|(k, _)| k == "name")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    Ok(Document { nombre, raiz })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn documento_minimo() {
        let d = analizar("<toby-app name=\"demo\"><window width=\"10\" height=\"10\"/></toby-app>")
            .expect("debe analizar");
        assert_eq!(d.raiz.etiqueta, "toby-app");
        assert_eq!(d.raiz.hijos.len(), 1);
        assert_eq!(d.raiz.hijos[0].etiqueta, "window");
        assert_eq!(d.raiz.hijos[0].atributos.len(), 2);
        assert_eq!(d.nombre, "demo");
    }

    #[test]
    fn raiz_incorrecta() {
        let e = analizar("<html></html>").expect_err("debe fallar");
        assert!(e.detalle.contains("toby-app"));
    }

    #[test]
    fn desbalanceadas() {
        assert!(analizar("<toby-app><box></toby-app>").is_err());
    }

    #[test]
    fn texto_suelto_rechazado() {
        assert!(analizar("<toby-app>hola</toby-app>").is_err());
    }

    #[test]
    fn entidades_xml_en_atributos() {
        let d = analizar(
            "<toby-app name=\"&lt;&gt;&amp;&quot;&apos;&#65;&#x42;\"><window width=\"10\" height=\"10\"/></toby-app>",
        )
        .expect("entidades XML válidas");
        assert_eq!(d.nombre, "<>&\"'AB");
        assert!(analizar("<toby-app name=\"&nope;\"></toby-app>").is_err());
    }

    #[test]
    fn referencia_numerica_admite_salto_de_linea() {
        let d = analizar(
            "<toby-app name=\"demo\"><window width=\"10\" height=\"10\"><label text=\"uno&#10;dos\"/></window></toby-app>",
        )
        .expect("la referencia numérica debe decodificarse");
        assert_eq!(d.raiz.hijos[0].hijos[0].atributos[0].1, "uno\ndos");
    }

    #[test]
    fn profundidad_limitada() {
        let mut s = String::from("<toby-app>");
        for _ in 0..40 {
            s.push_str("<column>");
        }
        for _ in 0..40 {
            s.push_str("</column>");
        }
        s.push_str("</toby-app>");
        assert!(analizar(&s).is_err());
    }

    #[test]
    fn comentarios_permitidos() {
        let d = analizar("<toby-app><!-- prueba --><box/></toby-app>").expect("debe analizar");
        assert_eq!(d.raiz.hijos.len(), 1);
    }

    #[test]
    fn los_valores_admiten_utf8() {
        // ñ son DOS bytes UTF-8 pero UN caracter: si el parser los contara por
        // bytes, las medidas de layout y el pintado se desalinearian.
        let d = analizar(concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"10\" height=\"10\">",
            "<segmented items=\"Pequeño|Mediano\"/>",
            "</window>",
            "</toby-app>"
        ))
        .expect("debe analizar");
        let seg = &d.raiz.hijos[0].hijos[0];
        let items = &seg
            .atributos
            .iter()
            .find(|(k, _)| k == "items")
            .expect("items")
            .1;
        assert_eq!(items, "Pequeño|Mediano");
        assert_eq!(items.chars().count(), 15, "ñ es un solo caracter");
    }

    #[test]
    fn una_secuencia_utf8_cortada_es_error_controlado() {
        // 0xC3 abre 'ñ' y justo despues esta la comilla de cierre: falta el
        // byte de continuacion. Se va por la base generica, que analiza bytes.
        let e = analizar_xml(b"<x><check label=\"a\xC3\"/></x>").expect_err("debe fallar");
        assert!(e.detalle.contains("UTF-8"));
    }

    #[test]
    fn raiz_libre_con_analizar_xml() {
        // La base genérica acepta cualquier raíz: la usa el manifiesto.
        let raiz = analizar_xml(b"<rapp nombre=\"demo\"><ui/></rapp>").expect("debe analizar");
        assert_eq!(raiz.etiqueta, "rapp");
        assert_eq!(raiz.hijos.len(), 1);
        assert_eq!(raiz.hijos[0].etiqueta, "ui");
    }
}
