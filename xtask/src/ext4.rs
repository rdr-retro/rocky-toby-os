//! Escritor de volumenes **ext4** para el anfitrion.
//!
//! Sustituye al antiguo `estrato.rs`: ahora el volumen del disco es un ext4 de
//! verdad, con el **mismo formato que produce `mke2fs` de Linux** para el
//! conjunto minimo de caracteristicas:
//!
//! ```text
//!   features: filetype extent large_file      (sin diario, sin sumas de
//!                                              comprobacion, sin 64 bits,
//!                                              sin flex_bg, sin dir_index)
//!   bloque:   1024 bytes
//!   inodo:    128 bytes
//! ```
//!
//! Asi el propio `e2fsck` de e2fsprogs puede comprobar (y Linux montar) el
//! volumen que genera `xtask`. El formato en disco es:
//!
//! ```text
//!   bloque 0              reservado (bloque de arranque; no pertenece a ningun grupo)
//!   en cada grupo g:      primero = 1 + g * 8192
//!       primero + 0       copia del superbloque
//!       primero + 1       tabla de descriptores de grupo
//!       primero + 2       mapa de bloques del grupo
//!       primero + 3       mapa de inodos del grupo
//!       primero + 4 ...   tabla de inodos (2048 inodos x 128 B = 256 bloques)
//!   resto                 zona de datos
//! ```
//!
//! El bit `i` del mapa de bloques de un grupo corresponde al bloque
//! `1 + g * 8192 + i`. El inodo `n` vive en la tabla del grupo `(n-1)/2048`.
//! Los inodos 1..10 estan reservados; el 2 es la raiz y el 11 `lost+found`.
//!
//! Solo se escriben los bloques que cambian: el volumen se construye en
//! memoria y al final se vuelca entero, como hacia `estrato.rs`.

const BS: usize = 1024;
const INODO_TAM: usize = 128;
const INODOS_POR_GRUPO: u32 = 2048;
const BLOQUES_POR_GRUPO: u32 = 8192;
/// Bloques que ocupa la tabla de inodos de un grupo.
const BLOQUES_TABLA_INODOS: u32 = INODOS_POR_GRUPO * INODO_TAM as u32 / BS as u32; // 256

/// Primer bloque con datos (para bloques de 1024 B).
const PRIMER_BLOQUE: u32 = 1;
/// Inodo de la raiz.
pub const RAIZ: u32 = 2;
/// Primer inodo asignable por el usuario.
const PRIMER_INODO: u32 = 11;
/// Inodo de `lost+found` (el primero asignable, como hace `mke2fs`).
const LOST_FOUND: u32 = 11;

/// `s_feature_incompat`: FILETYPE (0x2) + EXTENTS (0x40).
const FEAT_INCOMPAT: u32 = 0x0002 | 0x0040;
/// `s_feature_ro_compat`: LARGE_FILE (0x2).
const FEAT_RO_COMPAT: u32 = 0x0002;
/// `s_feature_compat`: ninguna.
const FEAT_COMPAT: u32 = 0x0000;

/// Marca del superbloque ext4.
const MAGIA: u16 = 0xEF53;
/// Marca de la cabecera de un arbol de extents.
const MAGIA_EXTENT: u16 = 0xF30A;
/// `i_flags`: el inodo usa extents.
const FLAG_EXTENTS: u32 = 0x0008_0000;

/// Tipo de una entrada, como lo entiende un directorio ext4.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Fichero,
    Directorio,
}

impl Tipo {
    /// Codigo de tipo de fichero de las entradas de directorio ext4.
    fn dir_codigo(self) -> u8 {
        match self {
            Tipo::Fichero => 1,
            Tipo::Directorio => 2,
        }
    }
    /// Bits de modo POSIX (S_IFREG / S_IFDIR) con los permisos por defecto.
    fn modo(self) -> u16 {
        match self {
            Tipo::Fichero => 0o100644,
            Tipo::Directorio => 0o040755,
        }
    }
}

/// Un volumen ext4 que se construye en memoria.
pub struct Volumen {
    img: Vec<u8>,
    bloques: u32,
    grupos: u32,
    inodos: u32,
    /// Numero de bloques que tiene cada grupo (el ultimo puede tener menos).
    por_grupo: Vec<u32>,
}

fn u16_en(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_en(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn poner_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn poner_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

impl Volumen {
    /// Crea un volumen ext4 vacio de `total_sectores` sectores de 512 bytes.
    pub fn formatear(total_sectores: u64, etiqueta: &str) -> Volumen {
        let bloques = (total_sectores / 2) as u32; // 1024 / 512 = 2 sectores por bloque
        let grupos = bloques.div_ceil(BLOQUES_POR_GRUPO);
        let inodos = grupos * INODOS_POR_GRUPO;

        let img = vec![0u8; bloques as usize * BS];
        let mut v = Volumen {
            img,
            bloques,
            grupos,
            inodos,
            por_grupo: Vec::new(),
        };
        // Cuantos bloques tiene cada grupo (el ultimo puede quedarse corto).
        for g in 0..grupos {
            let inicio = PRIMER_BLOQUE + g * BLOQUES_POR_GRUPO;
            v.por_grupo.push(BLOQUES_POR_GRUPO.min(bloques - inicio));
        }

        // --- Metadatos de cada grupo: marcar bloques e inodos ocupados ---
        for g in 0..grupos {
            let inicio = v.inicio_grupo(g);
            // superbloque, descriptores, mapa de bloques, mapa de inodos, tabla.
            for k in 0..(4 + BLOQUES_TABLA_INODOS) {
                v.fijar_bit_bloque(inicio + k, true);
            }
        }
        // Inodos 1..10 reservados; 11 sera lost+found.
        for n in 1..=LOST_FOUND {
            v.fijar_bit_inodo(n, true);
        }

        // Relleno de los mapas de bits: ext4 marca a 1 los bits sobrantes (los
        // bloques que no existen en el ultimo grupo y los inodos que no caben
        // en el mapa), igual que hace `mke2fs`.
        for g in 0..grupos {
            let nb = v.por_grupo[g as usize];
            v.fijar_bits_bloque(g, nb, BLOQUES_POR_GRUPO, true);
            v.fijar_bits_inodo(g, INODOS_POR_GRUPO, 8192, true);
        }

        v.escribir_superbloque(etiqueta);
        v.escribir_descriptores();

        // --- Raiz (inodo 2) ---
        let raiz_bloque = v.asignar_bloque();
        v.escribir_bloque_de_datos(raiz_bloque, &[0u8; BS]);
        let mut raiz = v.inodo_nuevo(Tipo::Directorio, 3);
        v.poner_extents(&mut raiz, &[(0, raiz_bloque, 1)]);
        poner_u32(&mut raiz, 4, BS as u32); // tamano = 1 bloque
        poner_u32(&mut raiz, 0x1C, 2); // i_blocks en sectores de 512 B
        v.escribir_inodo(RAIZ, &raiz);

        // --- lost+found (inodo 11) ---
        let lf_bloque = v.asignar_bloque();
        v.escribir_bloque_de_datos(lf_bloque, &[0u8; BS]);
        let mut lf = v.inodo_nuevo(Tipo::Directorio, 2);
        v.poner_extents(&mut lf, &[(0, lf_bloque, 1)]);
        poner_u32(&mut lf, 4, BS as u32);
        poner_u32(&mut lf, 0x1C, 2);
        v.escribir_inodo(LOST_FOUND, &lf);

        // Poner '.' y '..' en los dos directorios y enlazarlos desde la raiz.
        v.rellenar_punto(raiz_bloque, RAIZ, RAIZ);
        v.rellenar_punto(lf_bloque, LOST_FOUND, RAIZ);
        v.anadir_entrada(RAIZ, "lost+found", LOST_FOUND, Tipo::Directorio);

        v.actualizar_resumen();
        v
    }

    /// Contenido binario del volumen, listo para escribir a disco.
    ///
    /// Antes de devolverlo recalcula los contadores de bloques e inodos libres
    /// y reescribe los descriptores de grupo y el superbloque, de modo que el
    /// volumen quede coherente despues de cualquier cambio.
    pub fn bytes(&mut self) -> &[u8] {
        self.actualizar_resumen();
        &self.img
    }

    // ----------------------------------------------------------------------
    // Geometria
    // ----------------------------------------------------------------------

    /// Primer bloque (superbloque) del grupo `g`.
    fn inicio_grupo(&self, g: u32) -> u32 {
        PRIMER_BLOQUE + g * BLOQUES_POR_GRUPO
    }

    fn grupo_de(&self, bloque: u32) -> u32 {
        (bloque - PRIMER_BLOQUE) / BLOQUES_POR_GRUPO
    }

    /// Desplazamiento del bloque `b` dentro de la imagen.
    fn off_bloque(&self, b: u32) -> usize {
        b as usize * BS
    }

    fn leer_bloque(&self, b: u32) -> &[u8] {
        let o = self.off_bloque(b);
        &self.img[o..o + BS]
    }

    fn escribir_bloque_de_datos(&mut self, b: u32, datos: &[u8]) {
        let o = self.off_bloque(b);
        self.img[o..o + BS].copy_from_slice(&datos[..BS]);
    }

    // ----------------------------------------------------------------------
    // Superbloque y descriptores
    // ----------------------------------------------------------------------

    fn escribir_superbloque(&mut self, etiqueta: &str) {
        let mut sb = [0u8; BS];
        poner_u32(&mut sb, 0x00, self.inodos);
        poner_u32(&mut sb, 0x04, self.bloques);
        poner_u32(&mut sb, 0x08, 0); // bloques reservados
        poner_u32(&mut sb, 0x0C, 0); // bloques libres (se recalcula al final)
        poner_u32(&mut sb, 0x10, self.inodos - LOST_FOUND); // inodos libres
        poner_u32(&mut sb, 0x14, PRIMER_BLOQUE); // first_data_block
        poner_u32(&mut sb, 0x18, 0); // log_block_size = 0 -> 1024 B
        poner_u32(&mut sb, 0x1C, 0); // log_cluster_size
        poner_u32(&mut sb, 0x20, BLOQUES_POR_GRUPO);
        poner_u32(&mut sb, 0x24, BLOQUES_POR_GRUPO); // clusters por grupo
        poner_u32(&mut sb, 0x28, INODOS_POR_GRUPO);
        poner_u32(&mut sb, 0x2C, 0); // mtime
        poner_u32(&mut sb, 0x30, 0); // wtime
        poner_u16(&mut sb, 0x34, 0); // mnt_count
        poner_u16(&mut sb, 0x36, 0xFFFF); // max_mnt_count
        poner_u16(&mut sb, 0x38, MAGIA);
        poner_u16(&mut sb, 0x3A, 1); // estado: limpio
        poner_u16(&mut sb, 0x3C, 1); // errores: continuar
        poner_u16(&mut sb, 0x3E, 0); // revision menor
        poner_u32(&mut sb, 0x40, 0); // lastcheck
        poner_u32(&mut sb, 0x44, 0); // checkinterval
        poner_u32(&mut sb, 0x48, 0); // creator_os = Linux
        poner_u32(&mut sb, 0x4C, 1); // rev_level = dinamica
        poner_u16(&mut sb, 0x50, 0); // def_resuid
        poner_u16(&mut sb, 0x52, 0); // def_resgid
        poner_u32(&mut sb, 0x54, PRIMER_INODO);
        poner_u16(&mut sb, 0x58, INODO_TAM as u16);
        poner_u16(&mut sb, 0x5A, 0); // block_group_nr
        poner_u32(&mut sb, 0x5C, FEAT_COMPAT);
        poner_u32(&mut sb, 0x60, FEAT_INCOMPAT);
        poner_u32(&mut sb, 0x64, FEAT_RO_COMPAT);
        // uuid fijo (no se usa con estas caracteristicas, pero ext4 lo trae)
        sb[0x68..0x78].copy_from_slice(&[
            0x52, 0x4F, 0x43, 0x4B, 0x59, 0x2D, 0x00, 0x01, 0x91, 0x8B, 0x77, 0x1A, 0x0C, 0xE4,
            0x5A, 0x39,
        ]);
        let mut nom = [0u8; 16];
        let bytes = etiqueta.as_bytes();
        let n = bytes.len().min(16);
        nom[..n].copy_from_slice(&bytes[..n]);
        sb[0x78..0x88].copy_from_slice(&nom);
        poner_u8(&mut sb, 0xFC, 1); // def_hash_version
        poner_u16(&mut sb, 0xFE, 0); // sin descriptores de 64 bytes

        // Copia del superbloque al principio de cada grupo.
        let copia = sb;
        for g in 0..self.grupos {
            let b = self.inicio_grupo(g);
            // El bloque de arranque (bloque 0) queda a cero; el superbloque va
            // despues, a 1024 bytes del principio del volumen.
            let o = self.off_bloque(b);
            self.img[o..o + BS].copy_from_slice(&copia);
        }
    }

    fn escribir_bloque_descriptores(&mut self, g: u32) {
        let inicio = self.inicio_grupo(g);
        let b = inicio + 1;
        let mut blk = [0u8; BS];
        for otro in 0..self.grupos {
            let o = otro as usize * 32;
            let base = self.inicio_grupo(otro);
            poner_u32(&mut blk, o, base + 2); // mapa de bloques
            poner_u32(&mut blk, o + 4, base + 3); // mapa de inodos
            poner_u32(&mut blk, o + 8, base + 4); // tabla de inodos
            poner_u16(&mut blk, o + 0x0C, self.grupo_libre_bloques(otro) as u16);
            poner_u16(&mut blk, o + 0x0E, self.grupo_libre_inodos(otro) as u16);
            poner_u16(&mut blk, o + 0x10, self.grupo_dirs(otro) as u16);
            poner_u16(&mut blk, o + 0x12, 0); // sin flags
                                              // `bg_itable_unused` solo tiene sentido con la caracteristica
                                              // `uninit_bg` (que no usamos): debe quedar a cero.
            poner_u16(&mut blk, o + 0x1C, 0);
        }
        // El bloque de descriptores puede ocupar mas de un bloque: se escribe
        // entero en el grupo indicado (los 4 grupos caben en 128 bytes).
        let o = self.off_bloque(b);
        self.img[o..o + BS].copy_from_slice(&blk);
    }

    /// (Re)escribe la tabla de descriptores en todos los grupos.
    fn escribir_descriptores(&mut self) {
        for g in 0..self.grupos {
            self.escribir_bloque_descriptores(g);
        }
    }

    // ----------------------------------------------------------------------
    // Mapas de bits
    // ----------------------------------------------------------------------

    fn off_bitmap_bloques(&self, g: u32) -> usize {
        self.off_bloque(self.inicio_grupo(g) + 2)
    }

    fn off_bitmap_inodos(&self, g: u32) -> usize {
        self.off_bloque(self.inicio_grupo(g) + 3)
    }

    fn bit_bloque(&self, b: u32) -> bool {
        let g = self.grupo_de(b);
        let i = b - PRIMER_BLOQUE - g * BLOQUES_POR_GRUPO;
        let o = self.off_bitmap_bloques(g) + (i / 8) as usize;
        self.img[o] & (1u8 << (i % 8)) != 0
    }

    fn fijar_bit_bloque(&mut self, b: u32, usado: bool) {
        let g = self.grupo_de(b);
        let i = b - PRIMER_BLOQUE - g * BLOQUES_POR_GRUPO;
        let o = self.off_bitmap_bloques(g) + (i / 8) as usize;
        let m = 1u8 << (i % 8);
        if usado {
            self.img[o] |= m;
        } else {
            self.img[o] &= !m;
        }
    }

    fn bit_inodo(&self, n: u32) -> bool {
        let g = (n - 1) / INODOS_POR_GRUPO;
        let i = (n - 1) % INODOS_POR_GRUPO;
        let o = self.off_bitmap_inodos(g) + (i / 8) as usize;
        self.img[o] & (1u8 << (i % 8)) != 0
    }

    fn fijar_bit_inodo(&mut self, n: u32, usado: bool) {
        let g = (n - 1) / INODOS_POR_GRUPO;
        let i = (n - 1) % INODOS_POR_GRUPO;
        let o = self.off_bitmap_inodos(g) + (i / 8) as usize;
        let m = 1u8 << (i % 8);
        if usado {
            self.img[o] |= m;
        } else {
            self.img[o] &= !m;
        }
    }

    fn grupo_libre_bloques(&self, g: u32) -> u32 {
        let mut n = 0;
        for i in 0..self.por_grupo[g as usize] {
            let b = self.inicio_grupo(g) + i;
            if !self.bit_bloque(b) {
                n += 1;
            }
        }
        n
    }

    fn grupo_libre_inodos(&self, g: u32) -> u32 {
        let mut n = 0;
        for i in 0..INODOS_POR_GRUPO {
            let ino = g * INODOS_POR_GRUPO + 1 + i;
            if !self.bit_inodo(ino) {
                n += 1;
            }
        }
        n
    }

    fn grupo_dirs(&self, g: u32) -> u32 {
        let mut n = 0;
        for i in 0..INODOS_POR_GRUPO {
            let ino = g * INODOS_POR_GRUPO + 1 + i;
            let b = self.leer_inodo(ino);
            if u16_en(&b, 0) & 0o170000 == 0o040000 && u16_en(&b, 0) != 0 {
                n += 1;
            }
        }
        n
    }

    /// Inodos sin usar al final de la tabla del grupo.
    #[allow(dead_code)]
    fn grupo_itable_unused(&self, g: u32) -> u32 {
        let mut n = 0;
        for i in (0..INODOS_POR_GRUPO).rev() {
            let ino = g * INODOS_POR_GRUPO + 1 + i;
            if self.bit_inodo(ino) {
                break;
            }
            n += 1;
        }
        n
    }

    /// Marca un tramo de bits del mapa de bloques del grupo `g` (indices dentro
    /// del grupo, no numeros de bloque).
    fn fijar_bits_bloque(&mut self, g: u32, desde: u32, hasta: u32, usado: bool) {
        for i in desde..hasta {
            let o = self.off_bitmap_bloques(g) + (i / 8) as usize;
            let m = 1u8 << (i % 8);
            if usado {
                self.img[o] |= m;
            } else {
                self.img[o] &= !m;
            }
        }
    }

    /// Marca un tramo de bits del mapa de inodos del grupo `g`.
    fn fijar_bits_inodo(&mut self, g: u32, desde: u32, hasta: u32, usado: bool) {
        for i in desde..hasta {
            let o = self.off_bitmap_inodos(g) + (i / 8) as usize;
            let m = 1u8 << (i % 8);
            if usado {
                self.img[o] |= m;
            } else {
                self.img[o] &= !m;
            }
        }
    }

    // ----------------------------------------------------------------------
    // Reserva de bloques e inodos
    // ----------------------------------------------------------------------

    /// Reserva el primer bloque libre y lo devuelve.
    fn asignar_bloque(&mut self) -> u32 {
        for g in 0..self.grupos {
            for i in 0..self.por_grupo[g as usize] {
                let b = self.inicio_grupo(g) + i;
                if !self.bit_bloque(b) {
                    self.fijar_bit_bloque(b, true);
                    return b;
                }
            }
        }
        panic!("ext4: no quedan bloques libres");
    }

    /// Reserva el primer inodo libre y lo devuelve.
    fn asignar_inodo(&mut self) -> u32 {
        for n in PRIMER_INODO..=self.inodos {
            if !self.bit_inodo(n) {
                self.fijar_bit_inodo(n, true);
                return n;
            }
        }
        panic!("ext4: no quedan inodos libres");
    }

    #[allow(dead_code)]
    fn liberar_bloque(&mut self, b: u32) {
        self.fijar_bit_bloque(b, false);
    }

    // ----------------------------------------------------------------------
    // Inodos
    // ----------------------------------------------------------------------

    fn off_inodo(&self, n: u32) -> usize {
        let g = (n - 1) / INODOS_POR_GRUPO;
        let i = (n - 1) % INODOS_POR_GRUPO;
        let tabla = self.inicio_grupo(g) + 4;
        self.off_bloque(tabla) + i as usize * INODO_TAM
    }

    fn leer_inodo(&self, n: u32) -> [u8; INODO_TAM] {
        let o = self.off_inodo(n);
        let mut b = [0u8; INODO_TAM];
        b.copy_from_slice(&self.img[o..o + INODO_TAM]);
        b
    }

    fn escribir_inodo(&mut self, n: u32, b: &[u8; INODO_TAM]) {
        let o = self.off_inodo(n);
        let mut inodo = *b;
        let mtime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(u32::MAX as u64) as u32;
        poner_u32(&mut inodo, 0x10, mtime);
        self.img[o..o + INODO_TAM].copy_from_slice(&inodo);
    }

    fn inodo_nuevo(&self, t: Tipo, enlaces: u16) -> [u8; INODO_TAM] {
        let mut b = [0u8; INODO_TAM];
        poner_u16(&mut b, 0x00, t.modo());
        poner_u16(&mut b, 0x1A, enlaces);
        poner_u32(&mut b, 0x20, FLAG_EXTENTS);
        b
    }

    // ----------------------------------------------------------------------
    // Extents
    // ----------------------------------------------------------------------

    /// Escribe el arbol de extents de un inodo a partir de los tramos
    /// `(bloque logico, bloque fisico, longitud)`. Soporta profundidad 0 y 1.
    fn poner_extents(&mut self, inodo: &mut [u8; INODO_TAM], tramos: &[(u32, u32, u32)]) {
        const OFS: usize = 0x28; // i_block
        if tramos.len() <= 4 {
            let mut e = [0u8; 60];
            poner_u16(&mut e, 0, MAGIA_EXTENT);
            poner_u16(&mut e, 2, tramos.len() as u16);
            poner_u16(&mut e, 4, 4);
            poner_u16(&mut e, 6, 0);
            for (k, &(lb, fb, ln)) in tramos.iter().enumerate() {
                let o = 12 + k * 12;
                poner_u32(&mut e, o, lb);
                poner_u16(&mut e, o + 4, ln as u16);
                poner_u16(&mut e, o + 6, 0); // parte alta del bloque (bloques de 32 bits)
                poner_u32(&mut e, o + 8, fb);
            }
            inodo[OFS..OFS + 60].copy_from_slice(&e);
            return;
        }

        // Mas de cuatro tramos: arbol de profundidad 1. Los extents se reparten
        // en bloques hoja (84 por bloque) y el inodo guarda los indices.
        let mut indices: Vec<(u32, u32)> = Vec::new(); // (bloque logico, bloque hoja)
        for trozo in tramos.chunks(84) {
            let hoja = self.asignar_bloque();
            let mut e = [0u8; BS];
            poner_u16(&mut e, 0, MAGIA_EXTENT);
            poner_u16(&mut e, 2, trozo.len() as u16);
            poner_u16(&mut e, 4, 84);
            poner_u16(&mut e, 6, 0);
            for (k, &(lb, fb, ln)) in trozo.iter().enumerate() {
                let o = 12 + k * 12;
                poner_u32(&mut e, o, lb);
                poner_u16(&mut e, o + 4, ln as u16);
                poner_u16(&mut e, o + 6, 0);
                poner_u32(&mut e, o + 8, fb);
            }
            self.escribir_bloque_de_datos(hoja, &e);
            indices.push((trozo[0].0, hoja));
        }
        assert!(
            indices.len() <= 4,
            "ext4: arbol de extents demasiado grande"
        );

        let mut e = [0u8; 60];
        poner_u16(&mut e, 0, MAGIA_EXTENT);
        poner_u16(&mut e, 2, indices.len() as u16);
        poner_u16(&mut e, 4, 4);
        poner_u16(&mut e, 6, 1); // profundidad 1
        for (k, &(lb, hoja)) in indices.iter().enumerate() {
            let o = 12 + k * 12;
            poner_u32(&mut e, o, lb);
            poner_u32(&mut e, o + 4, hoja);
            poner_u16(&mut e, o + 8, 0);
        }
        inodo[OFS..OFS + 60].copy_from_slice(&e);
    }

    /// Devuelve los `(bloque logico, bloque fisico)` de un inodo (profundidad 0
    /// y 1). Lo usa el escritor para retocar directorios ya creados.
    fn bloques_de(&self, inodo: &[u8; INODO_TAM]) -> Vec<(u32, u32)> {
        const OFS: usize = 0x28;
        let mut out = Vec::new();
        let e = &inodo[OFS..OFS + 60];
        if u16_en(e, 0) != MAGIA_EXTENT {
            return out;
        }
        let ent = u16_en(e, 2) as usize;
        let prof = u16_en(e, 6);
        if prof == 0 {
            for k in 0..ent {
                let o = 12 + k * 12;
                let lb = u32_en(e, o);
                let ln = u16_en(e, o + 4) as u32;
                let fisico = u32_en(e, o + 8);
                for j in 0..ln {
                    out.push((lb + j, fisico + j));
                }
            }
        } else {
            for k in 0..ent {
                let o = 12 + k * 12;
                let hoja = u32_en(e, o + 4);
                let h = self.leer_bloque(hoja);
                let hent = u16_en(h, 2) as usize;
                for j in 0..hent {
                    let ho = 12 + j * 12;
                    let lb = u32_en(h, ho);
                    let ln = u16_en(h, ho + 4) as u32;
                    let fisico = u32_en(h, ho + 8);
                    for m in 0..ln {
                        out.push((lb + m, fisico + m));
                    }
                }
            }
        }
        out.sort();
        out
    }

    // ----------------------------------------------------------------------
    // Directorios
    // ----------------------------------------------------------------------

    /// Rellena un bloque de directorio con '.' y '..' (el resto queda libre).
    fn rellenar_punto(&mut self, bloque: u32, propio: u32, padre: u32) {
        let mut b = [0u8; BS];
        // '.' -> el propio directorio
        poner_u32(&mut b, 0, propio);
        poner_u16(&mut b, 4, 12);
        poner_u8(&mut b, 6, 1);
        poner_u8(&mut b, 7, 2);
        b[8] = b'.';
        // '..' -> el padre, ocupando el resto del bloque
        poner_u32(&mut b, 12, padre);
        poner_u16(&mut b, 16, (BS - 12) as u16);
        poner_u8(&mut b, 18, 2);
        poner_u8(&mut b, 19, 2);
        b[20] = b'.';
        b[21] = b'.';
        self.escribir_bloque_de_datos(bloque, &b);
    }

    /// Añade una entrada `nombre -> ino` dentro del directorio `dir`.
    fn anadir_entrada(&mut self, dir: u32, nombre: &str, ino: u32, t: Tipo) {
        let mut dinodo = self.leer_inodo(dir);
        let tam = u32_en(&dinodo, 4) as usize;
        let name = nombre.as_bytes();
        let necesario = ((8 + name.len() + 3) / 4) * 4;
        assert!(necesario <= BS, "ext4: nombre demasiado largo");

        let bloques = self.bloques_de(&dinodo);
        for &(_lb, fisico) in &bloques {
            let blk = self.leer_bloque(fisico).to_vec();
            let mut off = 0usize;
            while off < BS {
                let e_ino = u32_en(&blk, off);
                let rec = u16_en(&blk, off + 4) as usize;
                let nl = blk[off + 6] as usize;
                let minimo = ((8 + nl + 3) / 4) * 4;
                if e_ino == 0 && rec >= necesario {
                    let mut b = blk.clone();
                    poner_u32(&mut b, off, ino);
                    poner_u16(&mut b, off + 4, rec as u16);
                    poner_u8(&mut b, off + 6, name.len() as u8);
                    poner_u8(&mut b, off + 7, t.dir_codigo());
                    b[off + 8..off + 8 + name.len()].copy_from_slice(name);
                    self.escribir_bloque_de_datos(fisico, &b);
                    return;
                }
                if rec >= minimo + necesario {
                    let mut b = blk.clone();
                    poner_u16(&mut b, off + 4, minimo as u16);
                    let n_off = off + minimo;
                    poner_u32(&mut b, n_off, ino);
                    poner_u16(&mut b, n_off + 4, (rec - minimo) as u16);
                    poner_u8(&mut b, n_off + 6, name.len() as u8);
                    poner_u8(&mut b, n_off + 7, t.dir_codigo());
                    b[n_off + 8..n_off + 8 + name.len()].copy_from_slice(name);
                    self.escribir_bloque_de_datos(fisico, &b);
                    return;
                }
                if rec == 0 {
                    break;
                }
                off += rec;
            }
        }

        // No habia hueco: el directorio crece un bloque.
        let nuevo = self.asignar_bloque();
        let mut b = [0u8; BS];
        poner_u32(&mut b, 0, ino);
        poner_u16(&mut b, 4, BS as u16);
        poner_u8(&mut b, 6, name.len() as u8);
        poner_u8(&mut b, 7, t.dir_codigo());
        b[8..8 + name.len()].copy_from_slice(name);
        self.escribir_bloque_de_datos(nuevo, &b);

        let mut tramos = self.bloques_de(&dinodo);
        tramos.push((tramos.len() as u32, nuevo));
        let tramos: Vec<(u32, u32, u32)> = tramos.iter().map(|&(lb, fb)| (lb, fb, 1)).collect();
        self.poner_extents(&mut dinodo, &tramos);
        poner_u32(&mut dinodo, 4, (tam + BS) as u32);
        poner_u32(&mut dinodo, 0x1C, (tramos.len() as u32) * 2);
        self.escribir_inodo(dir, &dinodo);
    }

    // ----------------------------------------------------------------------
    // API publica (la que usa `main.rs`)
    // ----------------------------------------------------------------------

    /// Crea (si hace falta) toda la jerarquia de directorios de una ruta
    /// absoluta y devuelve el inodo del ultimo.
    pub fn crear_dirs(&mut self, ruta: &str) -> Option<u32> {
        let mut actual = RAIZ;
        for parte in ruta.split('/').filter(|p| !p.is_empty()) {
            actual = match self.buscar_en(actual, parte) {
                Some(h) => h,
                None => self.crear(actual, parte, Tipo::Directorio)?,
            };
        }
        Some(actual)
    }

    /// Busca un nombre dentro de un directorio.
    fn buscar_en(&self, dir: u32, nombre: &str) -> Option<u32> {
        let dinodo = self.leer_inodo(dir);
        for (_lb, fisico) in self.bloques_de(&dinodo) {
            let blk = self.leer_bloque(fisico).to_vec();
            let mut off = 0usize;
            while off + 8 <= BS {
                let e_ino = u32_en(&blk, off);
                let rec = u16_en(&blk, off + 4) as usize;
                let nl = blk[off + 6] as usize;
                if e_ino != 0
                    && nl == nombre.len()
                    && &blk[off + 8..off + 8 + nl] == nombre.as_bytes()
                {
                    return Some(e_ino);
                }
                if rec == 0 {
                    break;
                }
                off += rec;
            }
        }
        None
    }

    /// Crea un fichero o directorio dentro de `padre`.
    pub fn crear(&mut self, padre: u32, nombre: &str, t: Tipo) -> Option<u32> {
        if nombre.is_empty() || nombre.len() > 255 {
            return None;
        }
        let ino = self.asignar_inodo();
        match t {
            Tipo::Fichero => {
                let b = self.inodo_nuevo(Tipo::Fichero, 1);
                self.escribir_inodo(ino, &b);
            }
            Tipo::Directorio => {
                let bloque = self.asignar_bloque();
                self.escribir_bloque_de_datos(bloque, &[0u8; BS]);
                let mut b = self.inodo_nuevo(Tipo::Directorio, 2);
                self.poner_extents(&mut b, &[(0, bloque, 1)]);
                poner_u32(&mut b, 4, BS as u32);
                poner_u32(&mut b, 0x1C, 2);
                self.escribir_inodo(ino, &b);
                self.rellenar_punto(bloque, ino, padre);
                // El padre gana un enlace por el subdirectorio.
                let mut p = self.leer_inodo(padre);
                let l = u16_en(&p, 0x1A);
                poner_u16(&mut p, 0x1A, l + 1);
                self.escribir_inodo(padre, &p);
            }
        }
        self.anadir_entrada(padre, nombre, ino, t);
        Some(ino)
    }

    /// Escribe un fichero (lo crea) dentro del directorio `dir`.
    pub fn escribir_fichero(&mut self, dir: u32, nombre: &str, datos: &[u8]) -> Option<u32> {
        let ino = self.crear(dir, nombre, Tipo::Fichero)?;
        self.escribir_contenido(ino, datos);
        Some(ino)
    }

    /// Vuelca `datos` en el inodo `ino`, reservando los bloques que hagan falta.
    pub fn escribir_contenido(&mut self, ino: u32, datos: &[u8]) {
        let nbloques = datos.len().div_ceil(BS);
        let mut tramos: Vec<(u32, u32, u32)> = Vec::new();
        let mut fisicos: Vec<u32> = Vec::new();
        for b in 0..nbloques {
            let fisico = self.asignar_bloque();
            fisicos.push(fisico);
            let ini = b * BS;
            let fin = (ini + BS).min(datos.len());
            let mut blk = [0u8; BS];
            blk[..fin - ini].copy_from_slice(&datos[ini..fin]);
            self.escribir_bloque_de_datos(fisico, &blk);
        }
        // Agrupa bloques fisicos consecutivos en tramos.
        let mut i = 0usize;
        while i < fisicos.len() {
            let mut n = 1u32;
            while i + (n as usize) < fisicos.len() && fisicos[i + n as usize] == fisicos[i] + n {
                n += 1;
            }
            tramos.push((i as u32, fisicos[i], n));
            i += n as usize;
        }
        let mut b = self.leer_inodo(ino);
        self.poner_extents(&mut b, &tramos);
        poner_u32(&mut b, 4, datos.len() as u32);
        poner_u32(&mut b, 0x1C, (nbloques as u32) * 2);
        self.escribir_inodo(ino, &b);
    }

    /// Recalcula los contadores globales y reescribe los descriptores y el
    /// superbloque con los valores finales.
    fn actualizar_resumen(&mut self) {
        let mut libres_bloques = 0u32;
        for g in 0..self.grupos {
            libres_bloques += self.grupo_libre_bloques(g);
        }
        let mut libres_inodos = 0u32;
        for n in 1..=self.inodos {
            if !self.bit_inodo(n) {
                libres_inodos += 1;
            }
        }
        // Superbloque: solo el campo de bloques libres (inodos libres se fijo en
        // `formatear`; se recalcula aqui por si acaso).
        let b = self.inicio_grupo(0);
        let o = self.off_bloque(b);
        poner_u32(&mut self.img, o + 0x0C, libres_bloques);
        poner_u32(&mut self.img, o + 0x10, libres_inodos);
        let copia: Vec<u8> = self.img[o..o + BS].to_vec();
        for g in 1..self.grupos {
            let ob = self.off_bloque(self.inicio_grupo(g));
            self.img[ob..ob + BS].copy_from_slice(&copia);
        }
        self.escribir_descriptores();
    }
}

fn poner_u8(b: &mut [u8], o: usize, v: u8) {
    b[o] = v;
}
