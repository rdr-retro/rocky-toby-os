//! Primitivas temporales y configuracion de transiciones para Toby AppKit.
//!
//! El reloj pertenece al anfitrion: `inicio`, `duracion` y `ahora` usan las
//! mismas unidades (ticks PIT, milisegundos u otra base monotona). Asi el
//! mismo modelo sirve en Kira, en un anfitrion de escritorio o en una app.

/// Duración Genie predeterminada compartida por AppKit y Kira.
pub const GENIE_DURATION_MS: u32 = 500;
/// Duración predeterminada del rebote de un icono del Dock.
pub const BOUNCE_DURATION_MS: u32 = 900;
/// Periodo predeterminado para repetir un salto mientras siga activo.
pub const BOUNCE_REPEAT_PERIOD_MS: u32 = 500;
/// Duración de un ciclo del patrón de progreso indeterminado.
pub const INDETERMINATE_PROGRESS_PERIOD_MS: u32 = 900;
/// Ancho de la textura de referencia usada para mantener la velocidad visual.
pub const INDETERMINATE_PROGRESS_TILE_WIDTH_PX: u32 = 32;

/// Fase cíclica para el patrón móvil compartido por las barras de progreso.
/// No asigna memoria ni depende del reloj del kernel: el anfitrión entrega los
/// ticks actuales al pedir el desplazamiento del mosaico.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndeterminateProgress {
    inicio: u64,
    periodo: u64,
}

/// Estado y cálculo del relleno de una barra porcentual.
///
/// El porcentaje no se interpola: el host lo cambia cuando avanza el trabajo.
/// La textura puede animarse aparte sin alterar el valor ni el ancho del
/// relleno, que se calcula con aritmética entera y se limita a `[0, 100]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeterminateProgress {
    value: u8,
}

impl DeterminateProgress {
    /// Crea el progreso para `value`, limitado al rango porcentual válido.
    pub const fn new(value: u8) -> Self {
        Self {
            value: if value > 100 { 100 } else { value },
        }
    }

    /// Porcentaje mostrado por el control.
    pub const fn value(self) -> u8 {
        self.value
    }

    /// Ancho entero del relleno, redondeado hacia abajo y sin desbordamiento.
    pub const fn filled_width(self, total_width: u32) -> u32 {
        ((total_width as u64 * self.value as u64) / 100) as u32
    }

    /// Fase cíclica de la textura animada del relleno, usando el mismo ritmo
    /// que la barra indeterminada. El host conserva el reloj y el renderizador
    /// recorta el mosaico al relleno para animar también su borde izquierdo.
    pub fn texture_offset_at(
        self,
        ahora: u64,
        unidades_por_segundo: u64,
        ancho_referencia: u32,
        ancho_mosaico: u32,
    ) -> u32 {
        if self.value == 0 {
            return 0;
        }
        IndeterminateProgress::from_millis(0, unidades_por_segundo).offset_al_mismo_ritmo(
            ahora,
            ancho_referencia,
            ancho_mosaico,
        )
    }
}

impl IndeterminateProgress {
    /// Crea el ciclo con unidades temporales elegidas por el anfitrión.
    pub const fn new(inicio: u64, periodo: u64) -> Self {
        Self { inicio, periodo }
    }

    /// Convierte el periodo predeterminado de milisegundos a las unidades del
    /// reloj anfitrión (por ejemplo, 100 ticks por segundo para el PIT).
    pub fn from_millis(inicio: u64, unidades_por_segundo: u64) -> Self {
        let periodo = ((INDETERMINATE_PROGRESS_PERIOD_MS as u128 * unidades_por_segundo as u128
            + 999)
            / 1000)
            .min(u64::MAX as u128) as u64;
        Self::new(inicio, periodo.max(1))
    }

    /// Desplazamiento del patrón en `0..ancho_mosaico`, listo para repetirlo
    /// horizontalmente dentro del clip de la barra.
    pub fn offset(self, ahora: u64, ancho_mosaico: u32) -> u32 {
        if self.periodo == 0 || ancho_mosaico == 0 {
            return 0;
        }
        let fase = ahora.wrapping_sub(self.inicio) % self.periodo;
        ((fase as u128 * ancho_mosaico as u128) / self.periodo as u128) as u32
    }

    /// Desplaza una textura de otro ancho con la misma velocidad en px/s que
    /// una textura de referencia. El desplazamiento se envuelve en el ancho
    /// del mosaico visible, por lo que las piezas pequeñas no ralentizan.
    pub fn offset_al_mismo_ritmo(
        self,
        ahora: u64,
        ancho_referencia: u32,
        ancho_mosaico: u32,
    ) -> u32 {
        if self.periodo == 0 || ancho_referencia == 0 || ancho_mosaico == 0 {
            return 0;
        }
        let transcurrido = ahora.wrapping_sub(self.inicio) as u128;
        let avance = transcurrido * ancho_referencia as u128 / self.periodo as u128;
        (avance % ancho_mosaico as u128) as u32
    }

    pub const fn period(self) -> u64 {
        self.periodo
    }
}

/// Curva de tiempo de una animacion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Easing {
    /// Avance constante.
    #[default]
    Lineal,
    /// Empieza despacio.
    Entrada,
    /// Termina despacio.
    Salida,
    /// Empieza y termina despacio.
    EntradaSalida,
}

/// Reloj parametrizado de una animacion. No consulta hardware ni reserva
/// memoria; cualquier anfitrion puede actualizarlo desde su bucle de eventos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Animation {
    inicio: u64,
    duracion: u64,
    easing: Easing,
}

impl Animation {
    /// Crea una animacion lineal con el reloj y las unidades del anfitrion.
    pub const fn new(inicio: u64, duracion: u64) -> Self {
        Self {
            inicio,
            duracion,
            easing: Easing::Lineal,
        }
    }

    /// Selecciona la curva de tiempo.
    pub const fn with_easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Progreso transformado por la curva, en Q16 `0..=65535`.
    pub fn progress(self, ahora: u64) -> u16 {
        let lineal = self.progress_linear(ahora) as i64;
        let unidad = u16::MAX as i64;
        let resultado = match self.easing {
            Easing::Lineal => lineal,
            Easing::Entrada => lineal * lineal / unidad,
            Easing::Salida => unidad - (unidad - lineal) * (unidad - lineal) / unidad,
            Easing::EntradaSalida if lineal < unidad / 2 => 2 * lineal * lineal / unidad,
            Easing::EntradaSalida => {
                let restante = unidad - lineal;
                unidad - 2 * restante * restante / unidad
            }
        };
        resultado.clamp(0, unidad) as u16
    }

    /// Progreso lineal, sin aplicar `easing`, en Q16 `0..=65535`.
    pub fn progress_linear(self, ahora: u64) -> u16 {
        if self.duracion == 0 {
            return u16::MAX;
        }
        let transcurrido = ahora.wrapping_sub(self.inicio).min(self.duracion);
        ((transcurrido as u128 * u16::MAX as u128) / self.duracion as u128) as u16
    }

    /// Progreso temporal en sentido inverso, util para cerrar/restaurar con la
    /// misma curva y la misma trayectoria.
    pub fn progress_reversed(self, ahora: u64) -> u16 {
        u16::MAX - self.progress(ahora)
    }

    /// Indica si se alcanzo el final, tolerando el desbordamiento del reloj.
    pub fn finished(self, ahora: u64) -> bool {
        self.duracion == 0 || ahora.wrapping_sub(self.inicio) >= self.duracion
    }

    pub const fn start(self) -> u64 {
        self.inicio
    }

    pub const fn duration(self) -> u64 {
        self.duracion
    }
}

/// Rebote vertical reutilizable, expresado en la misma unidad temporal que el
/// reloj del anfitrión. Ejecuta tres saltos amortiguados sin reservar memoria.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounce {
    timeline: Animation,
    altura_px: u16,
}

impl Bounce {
    /// Crea tres saltos amortiguados desde `inicio` durante `duracion` ticks.
    pub const fn new(inicio: u64, duracion: u64, altura_px: u16) -> Self {
        Self {
            timeline: Animation::new(inicio, duracion),
            altura_px,
        }
    }

    /// Desplazamiento vertical para pintar el objeto; los valores negativos
    /// hacen que suba. El valor vuelve exactamente a cero al terminar.
    pub fn offset_y(self, ahora: u64) -> i32 {
        self.offset_y_progreso(self.timeline.progress_linear(ahora))
    }

    /// Repite indefinidamente un salto de altura uniforme. El host decide
    /// cuándo parar de consultarlo (por ejemplo, al terminar una carga).
    pub fn offset_y_repeating(self, ahora: u64) -> i32 {
        let duracion = self.timeline.duration();
        if duracion == 0 {
            return 0;
        }
        let transcurrido = ahora.wrapping_sub(self.timeline.start()) % duracion;
        let progreso = ((transcurrido as u128 * u16::MAX as u128) / duracion as u128) as u16;
        let unidad = u16::MAX as u64;
        let fase = progreso as u64;
        if self.altura_px == 0 {
            return 0;
        }
        let arco_q16 = 4 * fase * (unidad - fase) / unidad;
        let altura = (self.altura_px as u64 * arco_q16 + unidad / 2) / unidad;
        -(altura as i32)
    }

    fn offset_y_progreso(self, progreso: u16) -> i32 {
        let unidad = u16::MAX as u64;
        let progreso = progreso as u64;
        if progreso == unidad || self.altura_px == 0 {
            return 0;
        }
        let recorrido = progreso * 3;
        let salto = (recorrido / unidad).min(2);
        let fase = recorrido % unidad;
        let arco_q16 = 4 * fase * (unidad - fase) / unidad;
        let denominador = unidad * 3;
        let numerador = self.altura_px as u64 * arco_q16 * (3 - salto);
        let altura = (numerador + denominador / 2) / denominador;
        -(altura as i32)
    }

    pub fn finished(self, ahora: u64) -> bool {
        self.timeline.finished(ahora)
    }

    pub const fn duration(self) -> u64 {
        self.timeline.duration()
    }
}

/// Efecto declarable en un documento Toby UI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    /// Pintado estatico, sin transicion.
    #[default]
    None,
    /// Deformacion de malla Genie entre dos rectangulos.
    Genie,
    /// Rebote vertical amortiguado para una superficie o elemento UI.
    Bounce,
}

/// Ajustes declarados en `<window transition="...">`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionConfig {
    pub effect: Effect,
    /// Duracion en milisegundos para documentos Toby UI.
    pub duration_ms: u32,
}

impl TransitionConfig {
    /// Convierte la duracion declarada en ms a unidades del reloj del host.
    /// `unidades_por_segundo` vale 1000 para ms, 100 para ticks PIT, etc.
    pub fn duration_in_units(self, unidades_por_segundo: u64) -> u64 {
        if unidades_por_segundo == 0 {
            return 0;
        }
        (((self.duration_ms as u128 * unidades_por_segundo as u128 + 999) / 1000)
            .min(u64::MAX as u128) as u64)
            .max(1)
    }

    /// Crea un timeline con la duracion convertida al reloj indicado.
    pub fn animation(self, inicio: u64, unidades_por_segundo: u64) -> Animation {
        Animation::new(inicio, self.duration_in_units(unidades_por_segundo))
    }

    /// Crea un rebote si esta transición declara `effect="bounce"`.
    pub fn bounce(self, inicio: u64, unidades_por_segundo: u64, altura_px: u16) -> Option<Bounce> {
        (self.effect == Effect::Bounce).then(|| {
            Bounce::new(
                inicio,
                self.duration_in_units(unidades_por_segundo),
                altura_px,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_clamps_wraps_and_reverses() {
        let a = Animation::new(100, 40);
        assert_eq!(a.progress(100), 0);
        assert_eq!(a.progress(120), 32_767);
        assert_eq!(a.progress_reversed(100), u16::MAX);
        assert_eq!(a.progress_reversed(140), 0);
        assert!(a.finished(140));
        assert_eq!(Animation::new(10, 0).progress(10), u16::MAX);
        assert_eq!(Animation::new(u64::MAX - 9, 20).progress(0), 32_767);
    }

    #[test]
    fn easing_has_exact_endpoints_and_monotonic_samples() {
        for easing in [
            Easing::Lineal,
            Easing::Entrada,
            Easing::Salida,
            Easing::EntradaSalida,
        ] {
            let a = Animation::new(0, 100).with_easing(easing);
            assert_eq!(a.progress(0), 0);
            assert_eq!(a.progress(100), u16::MAX);
            let mut previo = 0;
            for tick in 1..=100 {
                let actual = a.progress(tick);
                assert!(actual >= previo);
                previo = actual;
            }
        }
    }

    #[test]
    fn bounce_has_three_amortiguated_hops_and_exact_endpoints() {
        let bounce = Bounce::new(0, 90, 18);
        assert_eq!(bounce.offset_y(0), 0);
        assert_eq!(bounce.offset_y(15), -18);
        assert_eq!(bounce.offset_y(45), -12);
        assert_eq!(bounce.offset_y(75), -6);
        assert_eq!(bounce.offset_y(90), 0);
        assert!(bounce.finished(90));
    }

    #[test]
    fn bounce_can_repeat_while_the_host_keeps_it_active() {
        let bounce = Bounce::new(100, 90, 18);
        assert_eq!(bounce.offset_y_repeating(100), 0);
        assert_eq!(bounce.offset_y_repeating(145), -18);
        assert_eq!(bounce.offset_y_repeating(190), 0);
        assert_eq!(bounce.offset_y_repeating(235), -18);
        assert_eq!(bounce.offset_y_repeating(280), 0);
    }

    #[test]
    fn indeterminate_progress_cycles_without_a_percentage() {
        let progress = IndeterminateProgress::new(10, 90);
        assert_eq!(progress.offset(10, 30), 0);
        assert_eq!(progress.offset(55, 30), 15);
        assert_eq!(progress.offset(99, 30), 29);
        assert_eq!(progress.offset(100, 30), 0);
        assert_eq!(progress.offset(55, 0), 0);
        assert_eq!(IndeterminateProgress::from_millis(0, 100).period(), 90);
    }

    #[test]
    fn progreso_de_otro_tamano_conserva_la_velocidad_en_pixeles() {
        let progress = IndeterminateProgress::from_millis(0, 100);
        assert_eq!(progress.offset_al_mismo_ritmo(25, 32, 9), 8);
        assert_eq!(progress.offset_al_mismo_ritmo(26, 32, 9), 0);
        assert_eq!(progress.offset_al_mismo_ritmo(90, 32, 9), 5);
        assert_eq!(progress.offset_al_mismo_ritmo(55, 32, 32), 19);
        assert_eq!(progress.offset_al_mismo_ritmo(55, 0, 9), 0);
    }

    #[test]
    fn determinate_progress_clamps_and_scales_without_overflow() {
        assert_eq!(DeterminateProgress::new(0).filled_width(180), 0);
        assert_eq!(DeterminateProgress::new(1).filled_width(180), 1);
        assert_eq!(DeterminateProgress::new(50).filled_width(181), 90);
        assert_eq!(DeterminateProgress::new(100).filled_width(180), 180);
        assert_eq!(DeterminateProgress::new(255).value(), 100);
        assert_eq!(
            DeterminateProgress::new(100).filled_width(u32::MAX),
            u32::MAX
        );
    }

    #[test]
    fn determinate_progress_texture_shares_the_common_animation_rhythm() {
        let progress = DeterminateProgress::new(68);
        assert_eq!(progress.texture_offset_at(0, 100, 32, 9), 0);
        assert_eq!(progress.texture_offset_at(25, 100, 32, 9), 8);
        assert_eq!(progress.texture_offset_at(26, 100, 32, 9), 0);
        assert_eq!(
            DeterminateProgress::new(0).texture_offset_at(25, 100, 32, 9),
            0
        );
        assert_eq!(progress.texture_offset_at(25, 100, 32, 0), 0);
    }

    #[test]
    fn transition_config_creates_bounce_only_for_bounce_effect() {
        let config = TransitionConfig {
            effect: Effect::Bounce,
            duration_ms: BOUNCE_DURATION_MS,
        };
        assert_eq!(config.bounce(0, 100, 20).unwrap().duration(), 90);
        assert!(TransitionConfig {
            effect: Effect::Genie,
            duration_ms: GENIE_DURATION_MS,
        }
        .bounce(0, 100, 20)
        .is_none());
    }
}
