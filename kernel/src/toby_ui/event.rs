//! Eventos, foco y estados de Toby UI (Fase 6).
//!
//! Convierte la entrada cruda del kernel en eventos normalizados, mantiene el
//! estado de interacción (hover, pulsado, foco) y resuelve sobre qué nodo cae
//! cada evento.
//!
//! No ejecuta nada: sólo describe la intención del usuario. El registro de
//! acciones (Fase 7) es quien decide qué hacer con un clic.
//!
//! Nota sobre el teclado: `input.rs` entrega pulsaciones ya resueltas
//! (imprimibles, `\n`, `\x08`, `\t`, `\x1b`, CP437 >= 0x80) y además las
//! teclas de navegación, que antes se perdían. Aquí Tabulador y las flechas
//! abajo/derecha pasan al siguiente control, y Mayús+Tabulador o las flechas
//! arriba/izquierda al anterior. El resto de teclas especiales (Inicio, Fin,
//! Re/Av Pág, Insert, Supr) se ignoran: esta interfaz no las usa.
//!
//! Todo el módulo es puro y comprobable salvo [`sondear`], que es el único
//! punto que toca el hardware.

use alloc::vec::Vec;

use super::document::{Nodo, ValidatedDocument};
use super::layout::Disposicion;
use super::theme::Estado;

/// Botón del ratón (bits del PS/2: 1 izquierdo, 2 derecho, 4 central).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boton {
    /// Botón izquierdo.
    Izquierdo,
    /// Botón derecho.
    Derecho,
    /// Botón central.
    Central,
}

impl Boton {
    /// Bit que le corresponde en la máscara de botones.
    pub fn bit(&self) -> u8 {
        match self {
            Boton::Izquierdo => 1,
            Boton::Derecho => 2,
            Boton::Central => 4,
        }
    }

    /// Botón a partir de su bit.
    pub fn de_bit(bit: u8) -> Option<Boton> {
        match bit {
            1 => Some(Boton::Izquierdo),
            2 => Some(Boton::Derecho),
            4 => Some(Boton::Central),
            _ => None,
        }
    }
}

/// Tecla normalizada.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tecla {
    /// Intro (0x0A o 0x0D).
    Intro,
    /// Retroceso (0x08).
    Retroceso,
    /// Tabulador (0x09).
    Tabulador,
    /// Escape (0x1B).
    Escape,
    /// Espacio.
    Espacio,
    /// Byte CP437 imprimible tal cual (p.ej. 0x87 = «ç»).
    Caracter(u8),
    /// Foco al siguiente control (flecha abajo o derecha).
    FocoSiguiente,
    /// Foco al control anterior (flecha arriba o izquierda, o Mayús+Tabulador).
    FocoAnterior,
}

impl Tecla {
    /// Traduce el byte que devuelve `input::poll()`.
    ///
    /// `input.rs` ya ha resuelto los desplazamientos y las teclas muertas, así
    /// que aquí sólo se clasifican los bytes de control.
    pub fn del_byte(b: u8) -> Tecla {
        match b {
            0x0A | 0x0D => Tecla::Intro,
            0x08 => Tecla::Retroceso,
            0x09 => Tecla::Tabulador,
            0x1B => Tecla::Escape,
            b' ' => Tecla::Espacio,
            c => Tecla::Caracter(c),
        }
    }

    /// ¿Es un carácter imprimible (candidato a texto)?
    pub fn es_caracter(&self) -> bool {
        matches!(self, Tecla::Caracter(_))
    }
}

/// Evento normalizado, con coordenadas absolutas en píxeles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Evento {
    /// El puntero se ha movido.
    RatonMueve {
        /// Posición absoluta en X.
        x: i32,
        /// Posición absoluta en Y.
        y: i32,
        /// Desplazamiento aplicado en X.
        dx: i32,
        /// Desplazamiento aplicado en Y.
        dy: i32,
    },
    /// Botón pulsado.
    RatonAbajo {
        /// Posición absoluta en X.
        x: i32,
        /// Posición absoluta en Y.
        y: i32,
        /// Botón afectado.
        boton: Boton,
    },
    /// Botón soltado.
    RatonArriba {
        /// Posición absoluta en X.
        x: i32,
        /// Posición absoluta en Y.
        y: i32,
        /// Botón afectado.
        boton: Boton,
    },
    /// Pulsación de tecla.
    Tecla(Tecla),
}

/// Estado del puntero tal y como lo ve Toby UI.
///
/// El kernel sólo entrega desplazamientos (`mouse::tomar()`), así que la
/// posición absoluta se acumula aquí a partir de ellos.
#[derive(Clone, Copy, Debug)]
pub struct Entrada {
    /// Posición absoluta en X.
    pub x: i32,
    /// Posición absoluta en Y.
    pub y: i32,
    botones: u8,
}

impl Entrada {
    /// Puntero en una posición inicial, sin botones pulsados.
    pub fn nueva(x: i32, y: i32) -> Entrada {
        Entrada { x, y, botones: 0 }
    }

    /// Máscara de botones actual.
    pub fn botones(&self) -> u8 {
        self.botones
    }

    /// ¿Está pulsado ese botón?
    pub fn esta_pulsado(&self, b: Boton) -> bool {
        self.botones & b.bit() != 0
    }

    /// Aplica un desplazamiento, acotado al área indicada.
    pub fn mover(&mut self, dx: i32, dy: i32, ancho: i32, alto: i32) -> Evento {
        let max_x = (ancho - 1).max(0);
        let max_y = (alto - 1).max(0);
        self.x = (self.x + dx).clamp(0, max_x);
        self.y = (self.y + dy).clamp(0, max_y);
        Evento::RatonMueve {
            x: self.x,
            y: self.y,
            dx,
            dy,
        }
    }

    /// Aplica la máscara de botones y devuelve los cambios (abajo/arriba).
    ///
    /// Se comparan los bits uno a uno para no perder un abajo y un arriba del
    /// mismo botón entre dos sondeos.
    pub fn botones_nuevos(&mut self, mascara: u8) -> Vec<Evento> {
        let viejo = self.botones;
        self.botones = mascara & 0x07;
        let mut evs = Vec::new();
        for bit in [1u8, 2, 4] {
            let antes = viejo & bit != 0;
            let ahora = self.botones & bit != 0;
            if antes == ahora {
                continue;
            }
            let boton = match Boton::de_bit(bit) {
                Some(b) => b,
                None => continue,
            };
            let ev = if ahora {
                Evento::RatonAbajo {
                    x: self.x,
                    y: self.y,
                    boton,
                }
            } else {
                Evento::RatonArriba {
                    x: self.x,
                    y: self.y,
                    boton,
                }
            };
            evs.push(ev);
        }
        evs
    }

    /// Evento de tecla a partir de la pulsación neutral del kernel.
    ///
    /// Devuelve `None` para las teclas que esta interfaz no usa. El estado de
    /// Mayús lo resuelve `input`; aquí sólo decide entre avanzar y retroceder.
    pub fn tecla(p: &crate::input::Pulsacion) -> Option<Evento> {
        use crate::input::{Especial, Tecla as TeclaLibre};
        let tecla = match p.tecla {
            TeclaLibre::Caracter(b'\t') if p.shift => Tecla::FocoAnterior,
            TeclaLibre::Caracter(b) => Tecla::del_byte(b),
            TeclaLibre::Especial(Especial::Arriba | Especial::Izquierda) => Tecla::FocoAnterior,
            TeclaLibre::Especial(Especial::Abajo | Especial::Derecha) => Tecla::FocoSiguiente,
            TeclaLibre::Especial(_) => return None,
        };
        Some(Evento::Tecla(tecla))
    }
}

/// Lee el hardware y devuelve los eventos pendientes.
///
/// Es el ÚNICO punto del módulo que depende del kernel; el resto es puro.
/// El orden es determinista: movimiento, botones, teclas.
pub fn sondear(entrada: &mut Entrada, ancho: i32, alto: i32) -> Vec<Evento> {
    let mut evs = Vec::new();
    crate::mouse::poll();
    let (dx, dy, bot) = crate::mouse::tomar();
    if dx != 0 || dy != 0 {
        evs.push(entrada.mover(dx, dy, ancho, alto));
    }
    evs.extend(entrada.botones_nuevos(bot));
    while let Some(p) = crate::input::poll_tecla() {
        if let Some(ev) = Entrada::tecla(&p) {
            evs.push(ev);
        }
    }
    evs
}

/// Qué ha cambiado al aplicar un evento.
///
/// El estado actual siempre se puede consultar con los accesores de
/// [`Interaccion`]; esto sólo dice qué merece la pena repintar.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Cambios {
    /// Ha cambiado el nodo bajo el puntero.
    pub hover: bool,
    /// Ha cambiado el foco de teclado.
    pub foco: bool,
    /// Ha cambiado el nodo pulsado.
    pub pulsado: bool,
    /// Clic completado sobre este nodo (abajo y arriba sobre él).
    pub click: Option<usize>,
    /// Ha cambiado el VALOR de un componente (un `check` marcado, un
    /// `slider` movido, un `field` tecleado...). No viene del sistema de
    /// eventos: lo pone `app::App` al aplicar el valor, y sólo sirve para
    /// saber que merece la pena repintar.
    pub valor: bool,
}

impl Cambios {
    /// Nada ha cambiado.
    pub const NADA: Cambios = Cambios {
        hover: false,
        foco: false,
        pulsado: false,
        click: None,
        valor: false,
    };

    /// ¿Hay algo que repintar?
    pub fn algo(&self) -> bool {
        self.hover || self.foco || self.pulsado || self.click.is_some() || self.valor
    }

    /// Acumula otro cambio (para resumir varios eventos en un fotograma).
    ///
    /// El clic no se pierde: si el otro trae uno, gana el más reciente.
    pub fn unir(&mut self, otro: Cambios) {
        self.hover |= otro.hover;
        self.foco |= otro.foco;
        self.pulsado |= otro.pulsado;
        self.valor |= otro.valor;
        if otro.click.is_some() {
            self.click = otro.click;
        }
    }
}

/// Estado de interacción que sobrevive entre fotogramas.
///
/// [`Interaccion`] toma prestados el documento y la disposición, así que no se
/// puede guardar junto a ellos (sería una estructura autorreferente). Lo que sí
/// viaja de un fotograma a otro es esto: quién tiene el foco, quién está bajo el
/// puntero y quién está pulsado.
///
/// Uso: `Interaccion::retomar(...)` al empezar a atender eventos y
/// `Interaccion::suspender()` al terminar.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Sesion {
    /// Nodo con el foco de teclado.
    pub foco: Option<usize>,
    /// Nodo bajo el puntero.
    pub hover: Option<usize>,
    /// Nodo pulsado con el ratón.
    pub pulsado: Option<usize>,
}

/// Estado de interacción de un documento ya dispuesto.
///
/// Mantiene qué nodo tiene el foco, cuál está bajo el puntero y cuál está
/// pulsado, y los traduce a [`Estado`] para el tema.
pub struct Interaccion<'a> {
    pre: Vec<&'a Nodo>,
    disp: &'a Disposicion,
    foco: Option<usize>,
    hover: Option<usize>,
    pulsado: Option<usize>,
}

impl<'a> Interaccion<'a> {
    /// Prepara el estado de interacción de un documento.
    ///
    /// Exige el mismo `ValidatedDocument` con el que se calculó `disp`: los
    /// índices de nodo son los del preorden.
    pub fn nueva(doc: &'a ValidatedDocument, disp: &'a Disposicion) -> Interaccion<'a> {
        Interaccion::retomar(doc, disp, Sesion::default())
    }

    /// Reanuda la interacción de un fotograma anterior.
    ///
    /// Es la forma de conservar foco/hover/pulsado sin guardar el
    /// `Interaccion` entero (que presta el documento y la disposición).
    pub fn retomar(
        doc: &'a ValidatedDocument,
        disp: &'a Disposicion,
        sesion: Sesion,
    ) -> Interaccion<'a> {
        Interaccion {
            pre: doc.preorden(),
            disp,
            foco: sesion.foco,
            hover: sesion.hover,
            pulsado: sesion.pulsado,
        }
    }

    /// Lo que hay que conservar para el siguiente fotograma.
    pub fn suspender(&self) -> Sesion {
        Sesion {
            foco: self.foco,
            hover: self.hover,
            pulsado: self.pulsado,
        }
    }

    /// Nodo con el foco de teclado.
    pub fn foco(&self) -> Option<usize> {
        self.foco
    }

    /// Nodo bajo el puntero.
    pub fn hover(&self) -> Option<usize> {
        self.hover
    }

    /// Nodo pulsado con el ratón.
    pub fn pulsado(&self) -> Option<usize> {
        self.pulsado
    }

    /// Estado visual de un nodo, para el tema y el renderer.
    ///
    /// Prioridad: deshabilitado > pulsado > hover > enfocado > normal.
    pub fn estado_de(&self, i: usize) -> Estado {
        if !self.habilitado(i) {
            return Estado::Disabled;
        }
        if self.pulsado == Some(i) {
            return Estado::Pressed;
        }
        if self.hover == Some(i) {
            return Estado::Hover;
        }
        if self.foco == Some(i) {
            return Estado::Focused;
        }
        Estado::Normal
    }

    /// Clave de acción declarada en un nodo (`action="..."`), si la tiene.
    ///
    /// No la ejecuta: sólo la lee para que el registro de acciones (Fase 7)
    /// decida. Una clave inexistente en el registro será un error controlado.
    pub fn accion_de(&self, i: usize) -> Option<&'a str> {
        self.nodo(i)
            .and_then(|n| n.prop("action"))
            .and_then(|v| v.texto())
    }

    /// `id` declarado en un nodo, si lo tiene.
    ///
    /// Va con la petición de la acción para que un mismo manejador pueda
    /// distinguir desde qué nodo se ha disparado.
    pub fn id_de(&self, i: usize) -> Option<&'a str> {
        self.nodo(i).and_then(|n| n.id.as_deref())
    }

    /// `name` declarado en un nodo, si lo tiene (como en HTML).
    ///
    /// También viaja en la petición de la acción: un mismo manejador puede
    /// atender varios elementos y distinguirlos por su nombre, que es
    /// humano y estable, sin depender del `id` técnico.
    pub fn nombre_de(&self, i: usize) -> Option<&'a str> {
        self.nodo(i).and_then(|n| n.nombre())
    }

    /// Mueve el foco (sólo si el nodo es interactivo). Devuelve si cambió.
    pub fn enfocar(&mut self, i: Option<usize>) -> bool {
        let nuevo = match i {
            Some(k) if self.interactivo(k) => Some(k),
            _ => None,
        };
        if self.foco != nuevo {
            self.foco = nuevo;
            true
        } else {
            false
        }
    }

    /// Pasa el foco al siguiente nodo interactivo (con vuelta al principio).
    ///
    /// Lo dispara Tabulador y las flechas abajo/derecha.
    pub fn enfocar_siguiente(&mut self) {
        let lista = self.interactivos();
        if lista.is_empty() {
            self.foco = None;
            return;
        }
        let siguiente = match self.foco.and_then(|a| lista.iter().position(|&i| i == a)) {
            Some(p) => lista[(p + 1) % lista.len()],
            None => lista[0],
        };
        self.foco = Some(siguiente);
    }

    /// Pasa el foco al nodo interactivo anterior (con vuelta al final).
    ///
    /// Lo dispara Mayus+Tabulador y las flechas arriba/izquierda.
    pub fn enfocar_anterior(&mut self) {
        let lista = self.interactivos();
        if lista.is_empty() {
            self.foco = None;
            return;
        }
        let anterior = match self.foco.and_then(|a| lista.iter().position(|&i| i == a)) {
            Some(0) => lista[lista.len() - 1],
            Some(p) => lista[p - 1],
            None => lista[lista.len() - 1],
        };
        self.foco = Some(anterior);
    }

    /// Nodo interactivo bajo un punto; sube por los padres si el más profundo
    /// no es interactivo (p.ej. un texto dentro de un botón).
    pub fn diana(&self, x: i32, y: i32) -> Option<usize> {
        let mut actual = self.disp.indice_bajo(x, y);
        while let Some(i) = actual {
            if self.interactivo(i) {
                return Some(i);
            }
            actual = self.disp.nodos.get(i).and_then(|c| c.indice_padre);
        }
        None
    }

    /// Aplica un evento y describe qué ha cambiado.
    pub fn aplicar(&mut self, ev: &Evento) -> Cambios {
        let mut c = Cambios::NADA;
        match *ev {
            Evento::RatonMueve { x, y, .. } => {
                let nuevo = self.diana(x, y);
                if nuevo != self.hover {
                    self.hover = nuevo;
                    c.hover = true;
                }
            }
            Evento::RatonAbajo { x, y, boton } => {
                if boton != Boton::Izquierdo {
                    return c;
                }
                let diana = self.diana(x, y);
                if self.pulsado != diana {
                    self.pulsado = diana;
                    c.pulsado = true;
                }
                // Pulsar sobre algo le da el foco; pulsar en el fondo lo quita.
                if self.foco != diana {
                    self.foco = diana;
                    c.foco = true;
                }
            }
            Evento::RatonArriba { x, y, boton } => {
                if boton != Boton::Izquierdo {
                    return c;
                }
                let diana = self.diana(x, y);
                let pulsado = self.pulsado;
                if pulsado.is_some() {
                    self.pulsado = None;
                    c.pulsado = true;
                }
                // Clic = abajo y arriba sobre el MISMO nodo.
                if let (Some(p), Some(d)) = (pulsado, diana) {
                    if p == d {
                        c.click = Some(p);
                    }
                }
            }
            Evento::Tecla(Tecla::Tabulador | Tecla::FocoSiguiente) => {
                self.enfocar_siguiente();
                c.foco = true;
            }
            Evento::Tecla(Tecla::FocoAnterior) => {
                self.enfocar_anterior();
                c.foco = true;
            }
            Evento::Tecla(Tecla::Escape) => {
                if self.foco.is_some() {
                    self.foco = None;
                    c.foco = true;
                }
            }
            Evento::Tecla(_) => {}
        }
        c
    }

    fn nodo(&self, i: usize) -> Option<&'a Nodo> {
        self.pre.get(i).copied()
    }

    /// ¿El nodo está visible y habilitado?
    fn habilitado(&self, i: usize) -> bool {
        match self.nodo(i) {
            Some(n) => n.booleano("visible", true) && n.booleano("enabled", true),
            None => false,
        }
    }

    /// ¿El nodo recibe eventos del usuario?
    fn interactivo(&self, i: usize) -> bool {
        match self.nodo(i) {
            Some(n) => n.def.interactivo && self.habilitado(i),
            None => false,
        }
    }

    /// Índices de los nodos interactivos, en orden de pintado.
    fn interactivos(&self) -> Vec<usize> {
        (0..self.pre.len())
            .filter(|&i| self.interactivo(i))
            .collect()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::toby_ui::layout;
    use crate::toby_ui::parser::analizar;
    use crate::toby_ui::validation::validar;

    const DOC: &str = concat!(
        "<toby-app name=\"demo\">",
        "<window title=\"demo\" width=\"320\" height=\"240\">",
        "<column padding=\"10\" spacing=\"4\">",
        "<button id=\"a\" title=\"Aceptar\" height=\"24\"/>",
        "<button id=\"b\" title=\"Cancelar\" height=\"24\"/>",
        "<button id=\"c\" title=\"Salir\" height=\"24\" enabled=\"false\"/>",
        "</column>",
        "</window>",
        "</toby-app>"
    );

    fn montar(src: &str) -> (ValidatedDocument, layout::Disposicion) {
        let d = analizar(src).expect("parser");
        let v = validar(&d).expect("validación");
        let disp = layout::calcular_en(&v, 320, 240);
        (v, disp)
    }

    // Índices: 0 app, 1 window, 2 column, 3 botón a, 4 botón b, 5 botón c.
    // Botón a: (10,10,80,24); b: (10,38,88,24); c: (10,66,72,24).

    #[test]
    fn la_entrada_acumula_y_acota() {
        let mut e = Entrada::nueva(50, 50);
        let ev = e.mover(10, -20, 320, 240);
        assert_eq!(
            ev,
            Evento::RatonMueve {
                x: 60,
                y: 30,
                dx: 10,
                dy: -20
            }
        );
        // Se acota a los bordes.
        e.mover(-1000, -1000, 320, 240);
        assert_eq!((e.x, e.y), (0, 0));
        e.mover(1000, 1000, 320, 240);
        assert_eq!((e.x, e.y), (319, 239));
    }

    #[test]
    fn los_botones_generan_abajo_y_arriba() {
        let mut e = Entrada::nueva(10, 20);
        assert!(e.botones_nuevos(0).is_empty());
        let abajo = e.botones_nuevos(1);
        assert_eq!(
            abajo,
            alloc::vec![Evento::RatonAbajo {
                x: 10,
                y: 20,
                boton: Boton::Izquierdo
            }]
        );
        assert!(e.esta_pulsado(Boton::Izquierdo));
        // Sin cambios no hay eventos repetidos.
        assert!(e.botones_nuevos(1).is_empty());
        let arriba = e.botones_nuevos(0);
        assert_eq!(
            arriba,
            alloc::vec![Evento::RatonArriba {
                x: 10,
                y: 20,
                boton: Boton::Izquierdo
            }]
        );
        assert!(!e.esta_pulsado(Boton::Izquierdo));
    }

    #[test]
    fn clasificacion_de_teclas() {
        assert_eq!(Tecla::del_byte(0x0A), Tecla::Intro);
        assert_eq!(Tecla::del_byte(0x0D), Tecla::Intro);
        assert_eq!(Tecla::del_byte(0x08), Tecla::Retroceso);
        assert_eq!(Tecla::del_byte(0x09), Tecla::Tabulador);
        assert_eq!(Tecla::del_byte(0x1B), Tecla::Escape);
        assert_eq!(Tecla::del_byte(b' '), Tecla::Espacio);
        assert_eq!(Tecla::del_byte(b'a'), Tecla::Caracter(b'a'));
        assert_eq!(Tecla::del_byte(0x87), Tecla::Caracter(0x87));
        for byte in 0x20..=0x7E {
            assert_eq!(Tecla::del_byte(byte), Tecla::Caracter(byte));
        }
        assert!(Tecla::Caracter(b'x').es_caracter());
        assert!(!Tecla::Escape.es_caracter());
    }

    #[test]
    fn hover_sigue_al_nodo_bajo_el_puntero() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        // Fuera de todo.
        let c = it.aplicar(&Evento::RatonMueve {
            x: 300,
            y: 220,
            dx: 0,
            dy: 0,
        });
        assert!(!c.hover && it.hover().is_none());
        // Dentro del botón b.
        let c = it.aplicar(&Evento::RatonMueve {
            x: 20,
            y: 45,
            dx: 0,
            dy: 0,
        });
        assert!(c.hover);
        assert_eq!(it.hover(), Some(4));
        assert_eq!(it.estado_de(4), Estado::Hover);
        // Mover dentro del mismo botón no cambia nada.
        let c = it.aplicar(&Evento::RatonMueve {
            x: 25,
            y: 50,
            dx: 5,
            dy: 5,
        });
        assert!(!c.hover);
        // Un contenedor no captura el hover.
        let _ = it.aplicar(&Evento::RatonMueve {
            x: 300,
            y: 100,
            dx: 0,
            dy: 0,
        });
        assert!(it.hover().is_none());
    }

    #[test]
    fn clic_solo_si_abajo_y_arriba_sobre_el_mismo_nodo() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        // Abajo en el botón a (10,10,80,24).
        let c = it.aplicar(&Evento::RatonAbajo {
            x: 20,
            y: 20,
            boton: Boton::Izquierdo,
        });
        assert!(c.pulsado && c.foco && c.click.is_none());
        assert_eq!(it.pulsado(), Some(3));
        assert_eq!(it.foco(), Some(3));
        assert_eq!(it.estado_de(3), Estado::Pressed);
        // Arriba en el mismo nodo: clic.
        let c = it.aplicar(&Evento::RatonArriba {
            x: 25,
            y: 22,
            boton: Boton::Izquierdo,
        });
        assert_eq!(c.click, Some(3));
        assert_eq!(it.pulsado(), None);
    }

    #[test]
    fn arrastrar_fuera_no_es_clic() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        it.aplicar(&Evento::RatonAbajo {
            x: 20,
            y: 20,
            boton: Boton::Izquierdo,
        });
        // Soltar sobre otro botón: ni clic ni pulsación.
        let c = it.aplicar(&Evento::RatonArriba {
            x: 20,
            y: 45,
            boton: Boton::Izquierdo,
        });
        assert_eq!(c.click, None);
        assert_eq!(it.pulsado(), None);
    }

    #[test]
    fn el_derecho_no_interactua() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        let c = it.aplicar(&Evento::RatonAbajo {
            x: 20,
            y: 20,
            boton: Boton::Derecho,
        });
        assert!(!c.algo());
        assert_eq!(it.pulsado(), None);
    }

    #[test]
    fn el_fondo_no_es_diana_y_quita_el_foco() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        it.aplicar(&Evento::RatonAbajo {
            x: 20,
            y: 20,
            boton: Boton::Izquierdo,
        });
        assert_eq!(it.foco(), Some(3));
        it.aplicar(&Evento::RatonArriba {
            x: 20,
            y: 20,
            boton: Boton::Izquierdo,
        });
        // Pulsar en la zona vacía de la ventana.
        let c = it.aplicar(&Evento::RatonAbajo {
            x: 300,
            y: 200,
            boton: Boton::Izquierdo,
        });
        assert!(c.foco);
        assert_eq!(it.foco(), None);
        assert_eq!(it.pulsado(), None);
    }

    #[test]
    fn tabulador_recorre_los_interactivos_y_salta_los_deshabilitados() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        // Interactivos: 3 y 4 (el 5 está deshabilitado).
        it.enfocar_siguiente();
        assert_eq!(it.foco(), Some(3));
        it.enfocar_siguiente();
        assert_eq!(it.foco(), Some(4));
        // Vuelve al principio, saltándose el deshabilitado.
        it.enfocar_siguiente();
        assert_eq!(it.foco(), Some(3));
        // Y hacia atrás (API disponible aunque no haya tecla que la dispare).
        it.enfocar_anterior();
        assert_eq!(it.foco(), Some(4));

        // Con Tab desde el principio, sin foco previo.
        let mut it2 = Interaccion::nueva(&v, &d);
        let c = it2.aplicar(&Evento::Tecla(Tecla::Tabulador));
        assert!(c.foco);
        assert_eq!(it2.foco(), Some(3));
        // Escape lo suelta.
        let c = it2.aplicar(&Evento::Tecla(Tecla::Escape));
        assert!(c.foco);
        assert_eq!(it2.foco(), None);
    }

    #[test]
    fn no_se_puede_enfocar_un_deshabilitado() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        assert!(!it.enfocar(Some(5)));
        assert_eq!(it.foco(), None);
        assert!(!it.enfocar(Some(0))); // toby-app no es interactivo
        assert_eq!(it.foco(), None);
        assert!(it.enfocar(Some(4)));
        assert_eq!(it.foco(), Some(4));
    }

    #[test]
    fn estado_de_respeta_la_prioridad() {
        let (v, d) = montar(DOC);
        let mut it = Interaccion::nueva(&v, &d);
        assert_eq!(it.estado_de(3), Estado::Normal);
        assert_eq!(it.estado_de(5), Estado::Disabled); // c está deshabilitado
        it.enfocar(Some(3));
        assert_eq!(it.estado_de(3), Estado::Focused);
        it.aplicar(&Evento::RatonMueve {
            x: 20,
            y: 20,
            dx: 0,
            dy: 0,
        });
        assert_eq!(it.estado_de(3), Estado::Hover); // hover manda sobre focused
        it.aplicar(&Evento::RatonAbajo {
            x: 20,
            y: 20,
            boton: Boton::Izquierdo,
        });
        assert_eq!(it.estado_de(3), Estado::Pressed); // pulsado manda sobre hover
    }

    #[test]
    fn accion_de_lee_la_clave_declarada() {
        let src = concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<column>",
            "<button id=\"a\" title=\"Guardar\" action=\"settings.save\"/>",
            "<button id=\"b\" title=\"Cerrar\"/>",
            "</column>",
            "</window>",
            "</toby-app>"
        );
        let (v, d) = montar(src);
        let it = Interaccion::nueva(&v, &d);
        assert_eq!(it.accion_de(3), Some("settings.save"));
        assert_eq!(it.accion_de(4), None);
    }

    #[test]
    fn un_invisible_no_es_diana() {
        let src = concat!(
            "<toby-app name=\"demo\">",
            "<window width=\"320\" height=\"240\">",
            "<stack>",
            "<button id=\"fondo\" title=\"Fondo\"/>",
            "<button id=\"alto\" title=\"Alto\" visible=\"false\"/>",
            "</stack>",
            "</window>",
            "</toby-app>"
        );
        let (v, d) = montar(src);
        let it = Interaccion::nueva(&v, &d);
        // El invisible se dibuja encima pero no recibe nada: gana el de abajo.
        assert_eq!(it.diana(10, 10), Some(3));
    }
}
