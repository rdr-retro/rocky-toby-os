//! Reserva, validacion y liberacion de memoria para DMA.
//!
//! Un dispositivo DMA no lee la memoria del kernel: escribe en direcciones
//! fisicas que el kernel le pasa. Por eso una zona de DMA tiene que cumplir
//! tres cosas a la vez, y las tres son responsabilidad del kernel:
//!
//! 1. **Fisica y contigua.** El dispositivo no sabe translating: lo que ve es
//!    una direccion fisica. La region son marcos contiguos.
//! 2. **Alineada.** Las estructuras de un controlador (anillos, TRB, colas)
//!    piden una alineacion concreta, a veces mayor que el marco.
//! 3. **Por debajo del limite fisico.** En x86_64 el mapa por identidad que
//!    prepara el arranque solo llega a 4 GiB. Una region de DMA por encima de
//!    eso no se puede ni leer ni escribir desde el kernel sin ampliar la
//!    paginacion.
//!
//! [`DmaRegion`] es la forma de pedir todo eso de una vez, con la region ya
//! puesta a cero y con la propiedad de la region marcada, para que no se
//! devuelva al gestor de memoria mientras el dispositivo pueda seguir
//! usandola.
//!
//! La libreria no conoce el gestor de marcos del kernel: usa el trait
//! [`Marcos`], que en el kernel implementa `pmm.rs`. Asi las reglas se
//! pueden probar en el anfitrion, sin hardware.
//!
//! Codigo propio de rocky.

/// Tamano de marco de pagina (4 KiB).
pub const FRAME_SIZE: usize = 4096;

/// Tope fisico por defecto: 4 GiB, **exclusivo**.
///
/// El arranque mapea por identidad los primeros 4 GiB, asi que una region por
/// encima de este limite no seria direccionable desde el kernel sin tocar la
/// paginacion. `LIMITE_4GIB` deja la region completa justo por debajo.
pub const LIMITE_4GIB: u64 = 1 << 32;

/// Gestion de marcos fisicos de la que depende [`DmaRegion`].
///
/// La implementacion real es la de `pmm.rs` en el kernel. Lo que la libreria
/// necesita es solo el contrato: contiguedad, alineacion y limite fisico.
pub trait Marcos {
    /// Reserva `count` marcos **contiguos** cuya direccion base este alineada a
    /// `align` y cuya ventana completa quepa **por debajo** de `limite_fisico`
    /// (tope exclusivo).
    ///
    /// Devuelve la direccion fisica base, o `None` si no hay memoria libre
    /// que cumpla las condiciones.
    fn reservar(&mut self, count: usize, align: usize, limite_fisico: u64) -> Option<usize>;

    /// Devuelve al gestor los `count` marcos que empiezan en `addr`.
    ///
    /// Solo se llama cuando la region ya no la puede usar el dispositivo.
    fn devolver(&mut self, addr: usize, count: usize);

    /// Pone a cero `count` bytes que empiezan en `addr`.
    fn poner_cero(&mut self, addr: usize, count: usize);
}

/// Por que una region DMA no se puede crear, usar o liberar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorDma {
    /// Se pidio una region de cero marcos.
    TamanoCero,
    /// La alineacion pedida no es valida: cero, no potencia de dos o menor que
    /// el tamano de marco. Tambien se usa si el gestor devuelve una direccion
    /// que no respeta la alineacion pedida.
    AlineacionInvalida,
    /// El gestor no pudo reservar los marcos pedidos.
    SinMarcos,
    /// La region se sale del limite fisico admitido.
    LimiteFisico,
    /// El tamano de la region no cabe en el rango de direcciones.
    Desbordamiento,
    /// La region ya se habia liberado antes.
    DobleLiberacion,
    /// El dispositivo puede seguir usando la region: no se devuelve al gestor.
    EnUso,
    /// El acceso pedido (offset y longitud) se sale de la region.
    FueraDeRegion,
}

/// Una region de memoria reservada para DMA.
///
/// Se crea con [`DmaRegion::nueva`], que reserva los marcos, comprueba el
/// limite fisico y pone la region a cero. Mientras exista, los marcos son
/// propiedad de rocky: **no** se devuelven al gestor de memoria.
///
/// La region tiene dos banderas de estado:
///
/// - **en uso**: el dispositivo puede estar tocando la region. Mientras lo
///   este, [`DmaRegion::liberar`] se niega: es la regla de la fase 1 de no
///   devolver al PMM memoria que el dispositivo pueda seguir usando.
/// - **liberada**: los marcos ya han vuelto al gestor. No se puede volver a
///   liberar.
///
/// Codigo propio de rocky.
///
/// No es `Copy`: una region DMA es de propiedad exclusiva, y copiarla
/// permitiria devolver sus marcos dos veces al gestor de memoria.
#[derive(Debug, PartialEq, Eq)]
pub struct DmaRegion {
    /// Direccion fisica base (y virtual, por el mapa por identidad del arranque).
    base: usize,
    /// Numero de marcos reservados.
    marcos: usize,
    /// Alineacion de la direccion base.
    alineacion: usize,
    /// Tope fisico exclusivo que se pidio y que se cumplio.
    limite: u64,
    /// El dispositivo puede estar usando la region.
    en_uso: bool,
    /// Los marcos ya han vuelto al gestor.
    liberada: bool,
}

impl DmaRegion {
    /// Reserva una region DMA de `marcos` marcos, alineada a `alineacion`, por
    /// debajo de `limite_fisico` (tope exclusivo).
    ///
    /// La region sale puesta a cero. Si algo falla despues de reservar (por
    /// ejemplo, que la region se pase del limite fisico), los marcos se
    /// devuelven al gestor antes de devolver el error: no queda memoria
    /// perdida.
    pub fn nueva(
        gestor: &mut impl Marcos,
        marcos: usize,
        alineacion: usize,
        limite_fisico: u64,
    ) -> Result<DmaRegion, ErrorDma> {
        if marcos == 0 {
            return Err(ErrorDma::TamanoCero);
        }
        if !alineacion_valida(alineacion) {
            return Err(ErrorDma::AlineacionInvalida);
        }
        let bytes = marcos
            .checked_mul(FRAME_SIZE)
            .ok_or(ErrorDma::Desbordamiento)?;

        let base = gestor
            .reservar(marcos, alineacion, limite_fisico)
            .ok_or(ErrorDma::SinMarcos)?;

        // El gestor prometio la alineacion; se comprueba igualmente, porque
        // una region mal alineada rompe el controlador de forma silenciosa.
        if base % alineacion != 0 {
            gestor.devolver(base, marcos);
            return Err(ErrorDma::AlineacionInvalida);
        }
        // Y el limite: un gestor que lo ignore no puede dejar que la region
        // se pase, porque el kernel no la podria direccionar.
        let fin = (base as u64)
            .checked_add(bytes as u64)
            .ok_or(ErrorDma::Desbordamiento)?;
        if fin > limite_fisico {
            gestor.devolver(base, marcos);
            return Err(ErrorDma::LimiteFisico);
        }

        gestor.poner_cero(base, bytes);

        Ok(DmaRegion {
            base,
            marcos,
            alineacion,
            limite: limite_fisico,
            en_uso: false,
            liberada: false,
        })
    }

    /// Direccion fisica base de la region.
    ///
    /// Con el mapa por identidad del arranque, esta direccion es tambien
    /// una direccion virtual valida para el kernel.
    pub fn base(&self) -> usize {
        self.base
    }

    /// Numero de marcos de la region.
    pub fn marcos(&self) -> usize {
        self.marcos
    }

    /// Tamano de la region en bytes (siempre multiplo del tamano de marco).
    pub fn bytes(&self) -> usize {
        self.marcos * FRAME_SIZE
    }

    /// Alineacion de la direccion base.
    pub fn alineacion(&self) -> usize {
        self.alineacion
    }

    /// Tope fisico exclusivo que se pidio para la region.
    pub fn limite(&self) -> u64 {
        self.limite
    }

    /// Â¿El dispositivo puede seguir usando la region?
    pub fn esta_en_uso(&self) -> bool {
        self.en_uso
    }

    /// Â¿Los marcos ya han vuelto al gestor de memoria?
    pub fn esta_liberada(&self) -> bool {
        self.liberada
    }

    /// Puntero **constante** al inicio de la region.
    ///
    /// El puntero es valido mientras la region no se libere.
    pub fn puntero(&self) -> *const u8 {
        self.base as *const u8
    }

    /// Puntero **mutable** al inicio de la region.
    ///
    /// # Seguridad
    ///
    /// Solo debe usarse mientras la region no este en uso por el dispositivo: si
    /// el controlador esta escribiendo en la region a la vez, esto es una
    /// condicion de carrera de verdad.
    pub unsafe fn puntero_mut(&self) -> *mut u8 {
        self.base as *mut u8
    }

    /// Â¿La direccion `addr` con longitud `len` cae dentro de la region?
    pub fn contiene(&self, addr: usize, len: usize) -> bool {
        let fin = match (addr as u64).checked_add(len as u64) {
            Some(v) => v,
            None => return false,
        };
        addr >= self.base && fin <= (self.base + self.bytes()) as u64
    }

    /// Comprueba que un acceso (offset y longitud) cae dentro de la region.
    ///
    /// Es la comprobacion que hay que hacer **antes** de tocar la region con
    /// un dato que venga del dispositivo: un offset o una longitud inventados
    /// por el controlador no pueden salirse de la region.
    pub fn comprobar(&self, offset: usize, len: usize) -> Result<(), ErrorDma> {
        if !self.contiene(self.base.wrapping_add(offset), len) {
            return Err(ErrorDma::FueraDeRegion);
        }
        Ok(())
    }

    /// Vista de solo lectura de la region completa.
    pub fn bytes_region(&self) -> &[u8] {
        // Seguro: la region sigue reservada mientras exista este valor.
        unsafe { core::slice::from_raw_parts(self.base as *const u8, self.bytes()) }
    }

    /// Vista de escritura de la region completa.
    ///
    /// # Seguridad
    ///
    /// El llamante garantiza que el dispositivo no esta escribiendo en la
    /// region mientras dura la vista.
    pub unsafe fn bytes_region_mut(&mut self) -> &mut [u8] {
        core::slice::from_raw_parts_mut(self.base as *mut u8, self.bytes())
    }

    /// Marca la region como en uso por el dispositivo.
    ///
    /// Se llama en cuanto el controlador puede tener transferencias en vuelo.
    /// A partir de aqui [`DmaRegion::liberar`] se niega.
    pub fn marcar_en_uso(&mut self) {
        self.en_uso = true;
    }

    /// Quita la marca de uso: el dispositivo ya no toca la region.
    ///
    /// Hay que llamarlo **solo** cuando se ha parado el controlador y se ha
    /// confirmado que no hay transferencias en vuelo.
    pub fn liberar_uso(&mut self) {
        self.en_uso = false;
    }

    /// Devuelve los marcos al gestor de memoria.
    ///
    /// Falla con [`ErrorDma::EnUso`] si el dispositivo puede seguir tocando la
    /// region, y con [`ErrorDma::DobleLiberacion`] si ya se habia liberado.
    pub fn liberar(&mut self, gestor: &mut impl Marcos) -> Result<(), ErrorDma> {
        if self.liberada {
            return Err(ErrorDma::DobleLiberacion);
        }
        if self.en_uso {
            return Err(ErrorDma::EnUso);
        }
        gestor.devolver(self.base, self.marcos);
        self.liberada = true;
        Ok(())
    }
}

/// Â¿La alineacion pedida sirve para una region de marcos?
///
/// Tiene que ser una potencia de dos y al menos un marco: por debajo del
/// marco no se puede alinear nada.
pub fn alineacion_valida(alineacion: usize) -> bool {
    alineacion >= FRAME_SIZE && alineacion.is_power_of_two()
}

/// Redondea `valor` hacia arriba a multiplo de `alineacion`.
///
/// Devuelve `None` si el redondeo no cabe en el rango de direcciones.
pub fn alinear(valor: usize, alineacion: usize) -> Option<usize> {
    if alineacion == 0 || !alineacion.is_power_of_two() {
        return None;
    }
    let mask = alineacion - 1;
    valor.checked_add(mask).map(|v| v & !mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tope fisico sin limite: para los mocks, cuya direccion depende de donde
    /// el anfitrion coloque el buffer.
    const SIN_LIMITE: u64 = u64::MAX;

    /// Gestor de marcos de mentira, pero con **memoria de verdad**.
    ///
    /// El buffer es real (un array en la pila del test), asi que poner a cero
    /// y leer la region funcionan de verdad; lo unico que cambia respecto al
    /// gestor del kernel es que las direcciones son las del proceso.
    struct MemoriaFalsa {
        /// Memoria que reparte el gestor.
        datos: [u8; 128 * 1024],
        /// Offset del siguiente marco libre.
        siguiente: usize,
        /// Bytes que se han puesto a cero (para comprobarlo).
        ceros: usize,
        /// Marcos devueltos al gestor.
        devueltos: usize,
    }

    impl MemoriaFalsa {
        fn nueva() -> MemoriaFalsa {
            MemoriaFalsa {
                datos: [0; 128 * 1024],
                siguiente: 0,
                ceros: 0,
                devueltos: 0,
            }
        }

        fn base(&self) -> usize {
            self.datos.as_ptr() as usize
        }
    }

    impl Marcos for MemoriaFalsa {
        fn reservar(&mut self, count: usize, align: usize, limite_fisico: u64) -> Option<usize> {
            let bytes = count.checked_mul(FRAME_SIZE)?;
            let dir = alinear(self.base() + self.siguiente, align)?;
            let fin = dir.checked_add(bytes)?;
            if fin > self.base() + self.datos.len() {
                return None;
            }
            if fin as u64 > limite_fisico {
                return None;
            }
            self.siguiente = fin - self.base();
            Some(dir)
        }

        fn devolver(&mut self, addr: usize, count: usize) {
            assert_eq!(addr % FRAME_SIZE, 0);
            assert!(count > 0);
            self.devueltos += count;
        }

        fn poner_cero(&mut self, addr: usize, count: usize) {
            let off = addr - self.base();
            assert!(off + count <= self.datos.len());
            for b in &mut self.datos[off..off + count] {
                *b = 0;
            }
            self.ceros += count;
        }
    }

    /// Gestor que miente: da una direccion por encima del limite pedido.
    ///
    /// No toca memoria: solo sirve para comprobar que [`DmaRegion`] verifica
    /// el limite por su cuenta.
    struct MemoriaMentirosa {
        base: usize,
    }

    impl Marcos for MemoriaMentirosa {
        fn reservar(&mut self, _count: usize, _align: usize, _limite_fisico: u64) -> Option<usize> {
            Some(self.base)
        }
        fn devolver(&mut self, _addr: usize, _count: usize) {}
        fn poner_cero(&mut self, _addr: usize, _count: usize) {}
    }

    #[test]
    fn reserva_una_region_alineada_y_a_cero() {
        let mut mem = MemoriaFalsa::nueva();
        let r = DmaRegion::nueva(&mut mem, 4, 4096, SIN_LIMITE).expect("reserva");
        assert_eq!(r.marcos(), 4);
        assert_eq!(r.bytes(), 4 * FRAME_SIZE);
        assert_eq!(r.alineacion(), 4096);
        assert!(!r.esta_en_uso());
        assert!(!r.esta_liberada());
        // Salida a cero: el controlador ve una estructura limpia.
        assert_eq!(mem.ceros, 4 * FRAME_SIZE);
        for b in r.bytes_region() {
            assert_eq!(*b, 0);
        }
    }

    #[test]
    fn respeta_la_alineacion_pedida() {
        let mut mem = MemoriaFalsa::nueva();
        // Pide 64 KiB de alineacion: la base tiene que ser multiplo de 65536.
        let r = DmaRegion::nueva(&mut mem, 1, 65536, SIN_LIMITE).expect("reserva");
        assert_eq!(r.base() % 65536, 0);
    }

    #[test]
    fn rechaza_una_alineacion_invalida() {
        let mut mem = MemoriaFalsa::nueva();
        // No es potencia de dos.
        assert_eq!(
            DmaRegion::nueva(&mut mem, 1, 3000, SIN_LIMITE),
            Err(ErrorDma::AlineacionInvalida)
        );
        // Menor que el marco.
        assert_eq!(
            DmaRegion::nueva(&mut mem, 1, 512, SIN_LIMITE),
            Err(ErrorDma::AlineacionInvalida)
        );
        // Cero.
        assert_eq!(
            DmaRegion::nueva(&mut mem, 1, 0, SIN_LIMITE),
            Err(ErrorDma::AlineacionInvalida)
        );
    }

    #[test]
    fn rechaza_una_region_de_cero_marcos() {
        let mut mem = MemoriaFalsa::nueva();
        assert_eq!(
            DmaRegion::nueva(&mut mem, 0, 4096, SIN_LIMITE),
            Err(ErrorDma::TamanoCero)
        );
    }

    #[test]
    fn informa_del_agotamiento_de_memoria() {
        let mut mem = MemoriaFalsa::nueva();
        // El buffer son 128 KiB: caben 32 marcos. Se piden 33.
        assert_eq!(
            DmaRegion::nueva(&mut mem, 33, 4096, SIN_LIMITE),
            Err(ErrorDma::SinMarcos)
        );
    }

    #[test]
    fn no_reserva_una_region_por_encima_del_limite_fisico() {
        // Un limite por debajo de la primera pagina utilizable: no hay nada
        // que reservar por debajo de el.
        let mut mem = MemoriaFalsa::nueva();
        assert_eq!(
            DmaRegion::nueva(&mut mem, 1, 4096, 0x1000),
            Err(ErrorDma::SinMarcos)
        );
    }

    #[test]
    fn comprueba_el_limite_fisico_aunque_el_gestor_no_lo_haga() {
        // El gestor miente y devuelve memoria por encima de 4 GiB.
        let mut mem = MemoriaMentirosa {
            base: 0x1_0000_0000,
        };
        assert_eq!(
            DmaRegion::nueva(&mut mem, 1, 4096, LIMITE_4GIB),
            Err(ErrorDma::LimiteFisico)
        );
    }

    #[test]
    fn comprueba_el_rango_de_los_accesos() {
        let mut mem = MemoriaFalsa::nueva();
        let r = DmaRegion::nueva(&mut mem, 4, 4096, SIN_LIMITE).expect("reserva");
        // Dentro: vale.
        assert_eq!(r.comprobar(0, 4096), Ok(()));
        assert_eq!(r.comprobar(4096, 4096), Ok(()));
        assert_eq!(r.comprobar(0, 4 * FRAME_SIZE), Ok(()));
        // Se sale por la derecha: 4 bytes validos y 4 de mas.
        assert_eq!(
            r.comprobar(4 * FRAME_SIZE - 4, 8),
            Err(ErrorDma::FueraDeRegion)
        );
        assert_eq!(r.comprobar(4 * FRAME_SIZE, 1), Err(ErrorDma::FueraDeRegion));
        // Y por la izquierda (offset que desborda hacia atras).
        assert_eq!(r.comprobar(usize::MAX, 2), Err(ErrorDma::FueraDeRegion));
    }

    #[test]
    fn no_libera_una_region_que_el_dispositivo_usa() {
        let mut mem = MemoriaFalsa::nueva();
        let mut r = DmaRegion::nueva(&mut mem, 4, 4096, SIN_LIMITE).expect("reserva");
        r.marcar_en_uso();
        assert!(r.esta_en_uso());
        // Con el dispositivo usando la region, la liberacion se niega.
        assert_eq!(r.liberar(&mut mem), Err(ErrorDma::EnUso));
        assert!(!r.esta_liberada());
        assert_eq!(mem.devueltos, 0);
        // Cuando el dispositivo termina, se puede devolver al gestor.
        r.liberar_uso();
        assert!(!r.esta_en_uso());
        assert_eq!(r.liberar(&mut mem), Ok(()));
        assert!(r.esta_liberada());
        assert_eq!(mem.devueltos, 4);
    }

    #[test]
    fn avisa_de_la_doble_liberacion() {
        let mut mem = MemoriaFalsa::nueva();
        let mut r = DmaRegion::nueva(&mut mem, 4, 4096, SIN_LIMITE).expect("reserva");
        assert_eq!(r.liberar(&mut mem), Ok(()));
        assert_eq!(r.liberar(&mut mem), Err(ErrorDma::DobleLiberacion));
    }

    #[test]
    fn alinea_por_encima_() {
        assert_eq!(alinear(0x1001, 4096), Some(0x2000));
        assert_eq!(alinear(0x1000, 4096), Some(0x1000));
        assert_eq!(alinear(0, 4096), Some(0));
        assert_eq!(alinear(0x1000, 0), None);
        assert_eq!(alinear(0x1000, 3000), None);
        // Un redondeo que no cabe no se disimula.
        assert_eq!(alinear(usize::MAX - 10, 4096), None);
    }

    #[test]
    fn la_alineacion_valida_es_potencia_de_dos_y_al_menos_un_marco() {
        assert!(alineacion_valida(4096));
        assert!(alineacion_valida(65536));
        assert!(!alineacion_valida(2048));
        assert!(!alineacion_valida(3000));
        assert!(!alineacion_valida(0));
    }
}
