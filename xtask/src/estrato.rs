//! Escritor de volumenes Estrato para el anfitrion.
//!
//! Replica el formato que usa `kernel/src/fs.rs` para que `xtask` pueda crear la
//! imagen del disco con ficheros ya dentro. Asi los recursos grandes (como
//! `isma.png`, de 2,4 MiB) viven en el volumen del disco y no engordan el kernel
//! ni chocan con el limite de tamano de fichero de Estrato... que aqui tambien
//! se amplia con el mismo doble-indirecto que usa el kernel.
//!
//! Todo son bloques de 512 bytes. El volumen tiene la misma cabecera, el mismo
//! mapa de bloques, la misma tabla de registros y los mismos vinculos que
//! escribiria el propio rocky.

const BLOQUE: usize = 512;
const MAGIA: &[u8; 8] = b"ESTRATO\x01";
const NOMBRE_MAX: usize = 58;
const REGISTRO_TAM: usize = 64;
const VINCULO_TAM: usize = 64;
const VINCULOS_POR_BLOQUE: usize = BLOQUE / VINCULO_TAM;
const DIRECTOS: usize = 9;
const INDIRECTOS: usize = BLOQUE / 4;
const CON_INDIRECTO: usize = DIRECTOS + INDIRECTOS;

/// Tipo de entrada, con los mismos codigos que en el kernel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Fichero,
    Directorio,
}

impl Tipo {
    fn codigo(self) -> u8 {
        match self {
            Tipo::Fichero => 1,
            Tipo::Directorio => 2,
        }
    }
}

/// Un registro en memoria (la "ficha" de un fichero o directorio).
#[derive(Clone, Copy, Default)]
struct Registro {
    estado: u8,
    tam: u32,
    padre: u32,
    nbloques: u32,
    directos: [u32; DIRECTOS],
    indirecto: u32,
    indirecto2: u32,
}

/// Un volumen Estrato que se va construyendo en memoria.
pub struct Volumen {
    img: Vec<u8>,
    total: u64,
    bitmap_inicio: u64,
    tabla_inicio: u32,
    registros: u32,
    datos_inicio: u32,
    raiz: u32,
}

impl Volumen {
    /// Crea un volumen vacio de `total` bloques con la etiqueta indicada.
    pub fn formatear(total: u64, etiqueta: &str) -> Volumen {
        let bitmap_bytes = (total as usize + 7) / 8;
        let bitmap_bloques = ((bitmap_bytes + BLOQUE - 1) / BLOQUE) as u64;
        let registros = ((total / 64) as u32).clamp(64, 8192);
        let tabla_bloques =
            ((registros as usize * REGISTRO_TAM + BLOQUE - 1) / BLOQUE) as u64;
        let datos_inicio = 1 + bitmap_bloques + tabla_bloques;

        let mut img = vec![0u8; total as usize * BLOQUE];

        let mut etiq = [0u8; 16];
        let bytes = etiqueta.as_bytes();
        let n = bytes.len().min(16);
        etiq[..n].copy_from_slice(&bytes[..n]);

        img[0..8].copy_from_slice(MAGIA);
        img[8..16].copy_from_slice(&total.to_le_bytes());
        img[16..24].copy_from_slice(&1u64.to_le_bytes()); // bitmap_inicio
        img[24..32].copy_from_slice(&bitmap_bloques.to_le_bytes());
        img[32..40].copy_from_slice(&(1 + bitmap_bloques).to_le_bytes()); // tabla_inicio
        img[40..44].copy_from_slice(&registros.to_le_bytes());
        img[44..52].copy_from_slice(&datos_inicio.to_le_bytes());
        img[52..56].copy_from_slice(&0u32.to_le_bytes()); // raiz = registro 0
        img[72..88].copy_from_slice(&etiq);

        let mut v = Volumen {
            img,
            total,
            bitmap_inicio: 1,
            tabla_inicio: (1 + bitmap_bloques) as u32,
            registros,
            datos_inicio: datos_inicio as u32,
            raiz: 0,
        };

        // Marca como ocupada toda la zona de metadatos.
        for b in 0..datos_inicio as u32 {
            v.fijar_bit(b, true);
        }
        // La raiz es el registro 0, un directorio.
        let mut raiz = Registro::default();
        raiz.estado = Tipo::Directorio.codigo();
        v.guardar_registro(0, &raiz);
        v
    }

    /// Devuelve el contenido binario del volumen, listo para escribir a disco.
    pub fn bytes(&self) -> &[u8] {
        &self.img
    }

    // --- Bloques ---

    fn bloque(&self, lba: u32) -> &[u8] {
        let o = lba as usize * BLOQUE;
        &self.img[o..o + BLOQUE]
    }

    fn bloque_mut(&mut self, lba: u32) -> &mut [u8] {
        let o = lba as usize * BLOQUE;
        &mut self.img[o..o + BLOQUE]
    }

    fn fijar_bit(&mut self, bloque: u32, usado: bool) {
        let byte = self.bitmap_inicio as usize * BLOQUE + (bloque / 8) as usize;
        let mascara = 1u8 << (bloque % 8);
        if usado {
            self.img[byte] |= mascara;
        } else {
            self.img[byte] &= !mascara;
        }
    }

    fn bit_usado(&self, bloque: u32) -> bool {
        let byte = self.bitmap_inicio as usize * BLOQUE + (bloque / 8) as usize;
        self.img[byte] & (1u8 << (bloque % 8)) != 0
    }

    fn asignar_bloque(&mut self) -> Option<u32> {
        for b in self.datos_inicio..self.total as u32 {
            if !self.bit_usado(b) {
                self.fijar_bit(b, true);
                return Some(b);
            }
        }
        None
    }

    // --- Registros ---

    fn leer_registro(&self, idx: u32) -> Registro {
        if idx >= self.registros {
            return Registro::default();
        }
        let o = self.tabla_inicio as usize * BLOQUE + idx as usize * REGISTRO_TAM;
        let b = &self.img[o..o + REGISTRO_TAM];
        let u32en = |p: usize| u32::from_le_bytes(b[p..p + 4].try_into().unwrap());
        let mut directos = [0u32; DIRECTOS];
        for (i, d) in directos.iter_mut().enumerate() {
            *d = u32en(20 + i * 4);
        }
        Registro {
            estado: b[0],
            tam: u32en(4),
            padre: u32en(8),
            nbloques: u32en(16),
            directos,
            indirecto: u32en(56),
            indirecto2: u32en(60),
        }
    }

    fn guardar_registro(&mut self, idx: u32, r: &Registro) {
        let o = self.tabla_inicio as usize * BLOQUE + idx as usize * REGISTRO_TAM;
        let b = &mut self.img[o..o + REGISTRO_TAM];
        for x in b.iter_mut() {
            *x = 0;
        }
        b[0] = r.estado;
        b[4..8].copy_from_slice(&r.tam.to_le_bytes());
        b[8..12].copy_from_slice(&r.padre.to_le_bytes());
        b[16..20].copy_from_slice(&r.nbloques.to_le_bytes());
        for (i, d) in r.directos.iter().enumerate() {
            b[20 + i * 4..24 + i * 4].copy_from_slice(&d.to_le_bytes());
        }
        b[56..60].copy_from_slice(&r.indirecto.to_le_bytes());
        b[60..64].copy_from_slice(&r.indirecto2.to_le_bytes());
    }

    fn asignar_registro(&mut self) -> Option<u32> {
        for i in 0..self.registros {
            if self.leer_registro(i).estado == 0 {
                return Some(i);
            }
        }
        None
    }

    // --- Mapeo de bloques de datos ---

    fn puntero(blk: &[u8], i: usize) -> Option<u32> {
        let off = i * 4;
        if off + 4 > BLOQUE {
            return None;
        }
        let x = u32::from_le_bytes(blk[off..off + 4].try_into().unwrap());
        if x == 0 {
            None
        } else {
            Some(x)
        }
    }

    fn mapear(&self, r: &Registro, b: u32) -> Option<u32> {
        let b = b as usize;
        if b < DIRECTOS {
            let x = r.directos[b];
            return if x == 0 { None } else { Some(x) };
        }
        if b < CON_INDIRECTO {
            if r.indirecto == 0 {
                return None;
            }
            let ind = self.bloque(r.indirecto);
            return Self::puntero(ind, b - DIRECTOS);
        }
        if r.indirecto2 == 0 {
            return None;
        }
        let d = b - CON_INDIRECTO;
        let ind2 = self.bloque(r.indirecto2);
        let medio = Self::puntero(ind2, d / INDIRECTOS)?;
        let ind = self.bloque(medio);
        Self::puntero(ind, d % INDIRECTOS)
    }

    /// Garantiza que existe el `b`-esimo bloque de datos del registro `idx`.
    fn bloque_para(&mut self, idx: u32, b: u32) -> Option<u32> {
        let mut r = self.leer_registro(idx);
        let mut cambiado = false;

        let lba = if (b as usize) < DIRECTOS {
            if r.directos[b as usize] == 0 {
                let nb = self.asignar_bloque()?;
                r.directos[b as usize] = nb;
                cambiado = true;
            }
            r.directos[b as usize]
        } else if (b as usize) < CON_INDIRECTO {
            if r.indirecto == 0 {
                r.indirecto = self.asignar_bloque()?;
                cambiado = true;
            }
            let i = b as usize - DIRECTOS;
            let mut ind = self.bloque(r.indirecto).to_vec();
            let mut ptr = Self::puntero(&ind, i).unwrap_or(0);
            if ptr == 0 {
                ptr = self.asignar_bloque()?;
                ind[i * 4..i * 4 + 4].copy_from_slice(&ptr.to_le_bytes());
                self.bloque_mut(r.indirecto).copy_from_slice(&ind);
            }
            ptr
        } else {
            if r.indirecto2 == 0 {
                r.indirecto2 = self.asignar_bloque()?;
                cambiado = true;
            }
            let d = b as usize - CON_INDIRECTO;
            let nivel1 = d / INDIRECTOS;
            let mut ind2 = self.bloque(r.indirecto2).to_vec();
            let medio = match Self::puntero(&ind2, nivel1) {
                Some(x) => x,
                None => {
                    let nb = self.asignar_bloque()?;
                    ind2[nivel1 * 4..nivel1 * 4 + 4].copy_from_slice(&nb.to_le_bytes());
                    self.bloque_mut(r.indirecto2).copy_from_slice(&ind2);
                    nb
                }
            };
            let nivel2 = d % INDIRECTOS;
            let mut ind = self.bloque(medio).to_vec();
            let mut ptr = Self::puntero(&ind, nivel2).unwrap_or(0);
            if ptr == 0 {
                ptr = self.asignar_bloque()?;
                ind[nivel2 * 4..nivel2 * 4 + 4].copy_from_slice(&ptr.to_le_bytes());
                self.bloque_mut(medio).copy_from_slice(&ind);
            }
            ptr
        };

        if r.nbloques < b + 1 {
            r.nbloques = b + 1;
            cambiado = true;
        }
        if cambiado {
            self.guardar_registro(idx, &r);
        }
        Some(lba)
    }

    // --- Directorios ---

    fn escribir_vinculo(&mut self, dir: u32, nombre: &str, reg: u32, tipo: Tipo) -> bool {
        let r = self.leer_registro(dir);
        for b in 0..r.nbloques {
            let lba = match self.mapear(&r, b) {
                Some(x) => x,
                None => continue,
            };
            let mut blk = self.bloque(lba).to_vec();
            for v in 0..VINCULOS_POR_BLOQUE {
                let off = v * VINCULO_TAM;
                let ocupado =
                    u32::from_le_bytes(blk[off..off + 4].try_into().unwrap()) != 0;
                if !ocupado {
                    blk[off..off + 4].copy_from_slice(&reg.to_le_bytes());
                    blk[off + 4] = tipo.codigo();
                    blk[off + 5] = nombre.len() as u8;
                    blk[off + 6..off + 6 + nombre.len()].copy_from_slice(nombre.as_bytes());
                    self.bloque_mut(lba).copy_from_slice(&blk);
                    return true;
                }
            }
        }
        // No habia hueco: bloque nuevo del directorio.
        let nuevo = r.nbloques;
        let lba = match self.bloque_para(dir, nuevo) {
            Some(x) => x,
            None => return false,
        };
        let mut blk = vec![0u8; BLOQUE];
        blk[0..4].copy_from_slice(&reg.to_le_bytes());
        blk[4] = tipo.codigo();
        blk[5] = nombre.len() as u8;
        blk[6..6 + nombre.len()].copy_from_slice(nombre.as_bytes());
        self.bloque_mut(lba).copy_from_slice(&blk);
        let mut rr = self.leer_registro(dir);
        rr.tam = rr.nbloques * BLOQUE as u32;
        self.guardar_registro(dir, &rr);
        true
    }

    fn buscar_en(&self, dir: u32, nombre: &str) -> Option<u32> {
        let r = self.leer_registro(dir);
        for b in 0..r.nbloques {
            let lba = self.mapear(&r, b)?;
            let blk = self.bloque(lba).to_vec();
            for v in 0..VINCULOS_POR_BLOQUE {
                let off = v * VINCULO_TAM;
                let x = u32::from_le_bytes(blk[off..off + 4].try_into().unwrap());
                if x == 0 {
                    continue;
                }
                let largo = (blk[off + 5] as usize).min(NOMBRE_MAX);
                if &blk[off + 6..off + 6 + largo] == nombre.as_bytes() {
                    return Some(x);
                }
            }
        }
        None
    }

    /// Crea una entrada dentro de `padre`.
    pub fn crear(&mut self, padre: u32, nombre: &str, tipo: Tipo) -> Option<u32> {
        if nombre.is_empty() || nombre.len() > NOMBRE_MAX {
            return None;
        }
        let idx = self.asignar_registro()?;
        let mut r = Registro::default();
        r.estado = tipo.codigo();
        r.padre = padre;
        self.guardar_registro(idx, &r);
        if !self.escribir_vinculo(padre, nombre, idx, tipo) {
            return None;
        }
        Some(idx)
    }

    /// Crea (si hace falta) toda la jerarquia de directorios de una ruta
    /// absoluta y devuelve el registro del ultimo.
    pub fn crear_dirs(&mut self, ruta: &str) -> Option<u32> {
        let mut actual = self.raiz;
        for parte in ruta.split('/').filter(|p| !p.is_empty()) {
            actual = match self.buscar_en(actual, parte) {
                Some(h) => h,
                None => self.crear(actual, parte, Tipo::Directorio)?,
            };
        }
        Some(actual)
    }

    /// Escribe un fichero (lo crea) dentro del directorio `dir`.
    pub fn escribir_fichero(&mut self, dir: u32, nombre: &str, datos: &[u8]) -> Option<u32> {
        let idx = self.crear(dir, nombre, Tipo::Fichero)?;
        let nbloques = (datos.len() + BLOQUE - 1) / BLOQUE;
        for b in 0..nbloques {
            let lba = self.bloque_para(idx, b as u32)?;
            let ini = b * BLOQUE;
            let fin = (ini + BLOQUE).min(datos.len());
            let dst = self.bloque_mut(lba);
            dst[..fin - ini].copy_from_slice(&datos[ini..fin]);
        }
        let mut r = self.leer_registro(idx);
        r.tam = datos.len() as u32;
        self.guardar_registro(idx, &r);
        Some(idx)
    }
}
