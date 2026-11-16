//! Cola circular de capacidad fija.
//!
//! Las colas del kernel estan **acotadas por diseno**: si una fuente produce
//! mas rapido de lo que el consumidor drena hay que decidir y contarlo, no
//! crecer sin limite. Esta cola guarda `N` elementos exactos (no sacrifica un
//! hueco para distinguir llena de vacia) y ofrece [`Cola::empujar_todo`], que
//! es todo o nada: el conjunto de eventos de un informe de teclado no se parte
//! a la mitad.
//!
//! Es logica pura (solo `core`), asi que se prueba en el anfitrion.

/// Cola circular de `N` elementos de tipo `T`.
///
/// `T` tiene que ser `Copy` para poder rellenar el almacenamiento con `vacio`;
/// ese valor solo sirve de inicializacion y nunca se devuelve.
#[derive(Clone, Copy)]
pub struct Cola<T: Copy, const N: usize> {
    datos: [T; N],
    cabeza: usize,
    cola: usize,
    largo: usize,
}

impl<T: Copy, const N: usize> Cola<T, N> {
    /// Crea una cola vacia rellenando el almacenamiento con `vacio`.
    pub const fn nueva(vacio: T) -> Self {
        Self {
            datos: [vacio; N],
            cabeza: 0,
            cola: 0,
            largo: 0,
        }
    }

    /// Elementos que caben todavia.
    pub fn libres(&self) -> usize {
        N - self.largo
    }

    /// Elementos que hay ahora mismo.
    pub fn largo(&self) -> usize {
        self.largo
    }

    /// No queda ningun elemento.
    pub fn vacia(&self) -> bool {
        self.largo == 0
    }

    /// Encola un elemento si cabe. Devuelve si entro.
    pub fn empujar(&mut self, valor: T) -> bool {
        if self.largo == N {
            return false;
        }
        self.datos[self.cabeza] = valor;
        self.cabeza = if self.cabeza + 1 == N {
            0
        } else {
            self.cabeza + 1
        };
        self.largo += 1;
        true
    }

    /// Encola la secuencia **entera** o no encola nada.
    ///
    /// Asi una liberacion de teclas no se queda a medias: o entra el grupo
    /// completo o se descarta y el llamante lo cuenta.
    pub fn empujar_todo(&mut self, valores: &[T]) -> bool {
        if valores.len() > self.libres() {
            return false;
        }
        for valor in valores {
            let entro = self.empujar(*valor);
            debug_assert!(entro, "se comprobo el hueco antes de empujar");
        }
        true
    }

    /// Saca el elemento mas antiguo.
    pub fn sacar(&mut self) -> Option<T> {
        if self.largo == 0 {
            return None;
        }
        let valor = self.datos[self.cola];
        self.cola = if self.cola + 1 == N { 0 } else { self.cola + 1 };
        self.largo -= 1;
        Some(valor)
    }

    /// Primer elemento sin sacarlo.
    pub fn frente(&self) -> Option<&T> {
        if self.largo == 0 {
            None
        } else {
            Some(&self.datos[self.cola])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respeta_el_orden_y_los_limites() {
        let mut c = Cola::<u8, 4>::nueva(0);
        assert!(c.vacia());
        assert_eq!(c.libres(), 4);
        assert!(c.empujar(1));
        assert!(c.empujar(2));
        assert!(c.empujar(3));
        assert!(c.empujar(4));
        assert_eq!(c.largo(), 4);
        assert_eq!(c.libres(), 0);
        assert!(!c.empujar(5), "no cabe un quinto");
        assert_eq!(c.frente(), Some(&1));
        for esperado in 1..=4 {
            assert_eq!(c.sacar(), Some(esperado));
        }
        assert_eq!(c.sacar(), None);
        assert!(c.vacia());
    }

    #[test]
    fn da_la_vuelta_sin_perder_nada() {
        let mut c = Cola::<u8, 3>::nueva(0);
        // Tres ciclos completos con huecos distintos para forzar el cruce.
        for vuelta in 0..3u8 {
            assert!(c.empujar(vuelta));
            assert!(c.empujar(vuelta + 10));
            assert_eq!(c.sacar(), Some(vuelta));
            assert!(c.empujar(vuelta + 20));
            assert_eq!(c.sacar(), Some(vuelta + 10));
            assert_eq!(c.sacar(), Some(vuelta + 20));
        }
        assert!(c.vacia());
    }

    #[test]
    fn empujar_todo_es_todo_o_nada() {
        let mut c = Cola::<u8, 4>::nueva(0);
        assert!(c.empujar_todo(&[1, 2, 3]));
        // Solo cabe uno: el grupo de dos no entra y no deja restos.
        assert!(!c.empujar_todo(&[4, 5]));
        assert_eq!(c.largo(), 3);
        assert_eq!(c.sacar(), Some(1));
        assert!(c.empujar_todo(&[4, 5]));
        assert_eq!(c.sacar(), Some(2));
        assert_eq!(c.sacar(), Some(3));
        assert_eq!(c.sacar(), Some(4));
        assert_eq!(c.sacar(), Some(5));
        // Un grupo vacio siempre entra y no cambia nada.
        assert!(c.empujar_todo(&[]));
        assert!(c.vacia());
    }
}
