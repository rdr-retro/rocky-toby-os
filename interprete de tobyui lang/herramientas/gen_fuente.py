"""Genera `src/host/fuente.rs` a partir de la fuente OEM de la VGA de Windows.

`vgaoem.fon` (la fuente "Terminal") es un ejecutable NE que lleva un recurso
RT_FONT en formato FNT. La fuente es de 8 px de ancho y 12 de alto, un byte
por fila (MSB a la izquierda) y ancho fijo (`dfPixWidth = 8`).

OJO con el detalle que costo un rato encontrar: en este FNT
`dfFirstChar = 0x01`, o sea que el bloque de glifos que empieza en
`dfBitsOffset` arranca en el caracter 1, no en el 0. Si se copia ese bloque
con "indice = caracter" (`GLIFOS[ch] = bloque[ch]`), toda la tabla queda
desplazada un caracter y el texto sale con el caracter siguiente (`Hola` se
dibuja como `Ipmb`).

El `CanvasRenderer` de Toby UI hace `glifo = addr + caracter * bytes_por_glifo`
(ver `renderer.rs`), asi que aqui hay que dejar `GLIFOS[caracter]` con el
dibujo de ese caracter. Los caracteres sin glifo en el FNT (el 0 y los que
queden fuera de [dfFirstChar, dfLastChar]) se rellenan en blanco.

Uso:  python herramientas/gen_fuente.py [ruta_del_fon]

Ademas de copiar la ROM, compone los 23 acentos de `bios::COMPONER` (À Á Â Ã
... ã õ ý), que son los que el kernel anade a su copia RAM de la fuente: así
los `.toby` pueden escribirse en UTF-8 con acentos en mayuscula en ambos
anfitriones.
"""

import os
import struct
import sys

AQUI = os.path.dirname(os.path.abspath(__file__))
SALIDA = os.path.join(AQUI, "..", "src", "host", "fuente.rs")
FON = sys.argv[1] if len(sys.argv) > 1 else r"C:\Windows\Fonts\vgaoem.fon"

ANCHO = 8
ALTO = 12


def recurso_fnt(datos):
    """Devuelve el primer recurso RT_FONT (tipo 8) del ejecutable NE."""
    lfanew = struct.unpack_from("<I", datos, 0x3C)[0]
    assert datos[lfanew:lfanew + 2] == b"NE", "no es un ejecutable NE"
    tabla = lfanew + struct.unpack_from("<H", datos, lfanew + 0x24)[0]
    alineacion = struct.unpack_from("<H", datos, tabla)[0]
    p = tabla + 2
    while True:
        tipo, cuenta = struct.unpack_from("<HH", datos, p)
        if tipo == 0:
            break
        p += 8
        for _ in range(cuenta):
            off, largo, _flags, _rid, _h1, _h2 = struct.unpack_from("<HHHHHH", datos, p)
            p += 12
            if (tipo & 0x7FFF) == 8:  # RT_FONT
                off <<= alineacion
                largo <<= alineacion
                return datos[off:off + largo]
    raise SystemExit("el .fon no lleva ningun recurso RT_FONT")


def fuente_del_fnt(fnt):
    """Devuelve los 256 glifos con el glifo de cada caracter en su indice."""
    # Cabecera FNT (los desplazamientos estan fijados por el formato).
    pix_ancho = struct.unpack_from("<H", fnt, 0x56)[0]
    pix_alto = struct.unpack_from("<H", fnt, 0x58)[0]
    primero = fnt[0x5F]
    ultimo = fnt[0x60]
    bits_offset = struct.unpack_from("<I", fnt, 0x71)[0]

    assert pix_ancho == ANCHO and pix_alto == ALTO, \
        "la fuente no es %dx%d (es %dx%d)" % (ANCHO, ALTO, pix_ancho, pix_alto)

    bytes_por_glifo = ((pix_ancho + 7) // 8) * pix_alto
    glifos = bytearray(256 * ALTO)  # todo en blanco por defecto
    for ch in range(primero, ultimo + 1):
        origen = bits_offset + (ch - primero) * bytes_por_glifo
        trozo = fnt[origen:origen + bytes_por_glifo]
        assert len(trozo) == bytes_por_glifo, "el glifo 0x%02X se sale del recurso" % ch
        glifos[ch * ALTO:(ch + 1) * ALTO] = trozo
    return glifos, primero, ultimo


def arte(glifos, ch):
    filas = []
    for f in range(ALTO):
        b = glifos[ch * ALTO + f]
        filas.append("".join("#" if (b >> (7 - c)) & 1 else "." for c in range(ANCHO)))
    return filas


def comprobar(glifos):
    """Comprueba lo que se puede comprobar sin ojos: el espacio va en blanco y
    esta en su sitio, y las letras de la interfaz no se repiten."""
    assert all(b == 0 for b in glifos[0x20 * ALTO:0x21 * ALTO]), "el espacio no esta en blanco"
    blancos = [c for c in range(0x20, 0x7F) if all(b == 0 for b in glifos[c * ALTO:(c + 1) * ALTO])]
    assert blancos == [0x20], "el unico glifo en blanco de 0x20..0x7E tiene que ser el espacio"

    def g(ch):
        return bytes(glifos[ch * ALTO:(ch + 1) * ALTO])

    for a, b in [(0x41, 0x42), (0x4D, 0x4E), (0x30, 0x4F), (0x61, 0x6F)]:
        assert g(a) != g(b), "'%s' y '%s' tienen el mismo glifo" % (chr(a), chr(b))

    # La 'A' empieza estrecha por arriba y la 'B' tiene el asta a la izquierda:
    # es una forma barata de detectar un desplazamiento de un caracter.
    primera_a = next(f for f in range(ALTO) if glifos[0x41 * ALTO + f])
    primera_b = next(f for f in range(ALTO) if glifos[0x42 * ALTO + f])
    col_a = min(c for c in range(ANCHO) if (glifos[0x41 * ALTO + primera_a] >> (7 - c)) & 1)
    assert col_a > 0, "el primer pixel de la 'A' tendria que estar sangrado (empieza en punta)"
    assert primera_b >= primera_a, "la 'B' no arranca antes que la 'A'"


# Los acentos que compone bios::COMPONER en la copia RAM de la fuente del
# kernel (16 px). La tabla tiene que ser LA MISMA que la de bios.rs: cada
# destino ocupa su sitio Latin-1, y texto::COMPONER depende de ese orden.
ACENTO_AGUDO = [0x18, 0x30, 0x60]
ACENTO_GRAVE = [0x60, 0x30, 0x18]
ACENTO_CIRCUNFLEJO = [0x10, 0x38, 0x6C]
ACENTO_DIERESIS = [0xC6]
ACENTO_VIRGULILLA = [0x76, 0xDC]

COMPONER = [
    (0xC0, ord("A"), ACENTO_GRAVE, True),        # À
    (0xC1, ord("A"), ACENTO_AGUDO, True),        # Á
    (0xC2, ord("A"), ACENTO_CIRCUNFLEJO, True),  # Â
    (0xC3, ord("A"), ACENTO_VIRGULILLA, True),   # Ã
    (0xC8, ord("E"), ACENTO_GRAVE, True),        # È
    (0xC9, ord("E"), ACENTO_AGUDO, True),        # É
    (0xCA, ord("E"), ACENTO_CIRCUNFLEJO, True),  # Ê
    (0xCB, ord("E"), ACENTO_DIERESIS, True),     # Ë
    (0xCC, ord("I"), ACENTO_GRAVE, True),        # Ì
    (0xCD, ord("I"), ACENTO_AGUDO, True),        # Í
    (0xCE, ord("I"), ACENTO_CIRCUNFLEJO, True),  # Î
    (0xCF, ord("I"), ACENTO_DIERESIS, True),     # Ï
    (0xD2, ord("O"), ACENTO_GRAVE, True),        # Ò
    (0xD3, ord("O"), ACENTO_AGUDO, True),        # Ó
    (0xD4, ord("O"), ACENTO_CIRCUNFLEJO, True),  # Ô
    (0xD5, ord("O"), ACENTO_VIRGULILLA, True),   # Õ
    (0xD9, ord("U"), ACENTO_GRAVE, True),        # Ù
    (0xDA, ord("U"), ACENTO_AGUDO, True),        # Ú
    (0xDB, ord("U"), ACENTO_CIRCUNFLEJO, True),  # Û
    (0xDD, ord("Y"), ACENTO_AGUDO, True),        # Ý
    (0xE3, ord("a"), ACENTO_VIRGULILLA, False),  # ã
    (0xF5, ord("o"), ACENTO_VIRGULILLA, False),  # õ
    (0xFD, ord("y"), ACENTO_AGUDO, False),       # ý
]


def componer_acento(glifos, destino, letra, acento, mayus):
    """Compone un acento sobre una letra, como bios::COMPONER del kernel.

    En los 16 px del kernel la letra baja lo justo para dejar una fila en
    blanco entre el acento y ella, recortandose por abajo para no mover la
    linea base. Aqui no cabe (una mayuscula ocupa ya 9 de las 12 filas), y la
    propia ROM no deja ese hueco en sus acentuados (É 0x90: acento en 0-2 y
    'E' desde la 3; ñ 0xA4: virgulilla en 1-2, hueco en 3 y 'n' desde la 4).
    Asi que el mismo criterio de la ROM: en mayusculas la letra va justo
    debajo del acento y en minusculas con su hueco.
    """
    n = len(acento)
    # Fila de la ultima linea del acento.
    ultima = (2 if n >= 3 else 1) if mayus else 2
    primera = ultima + 1 - n
    # Fila donde arranca la letra.
    deseada = 3 if mayus else 4
    d = letra * ALTO
    t = destino * ALTO
    tope = next((f for f in range(ALTO) if glifos[d + f]), ALTO)
    fin = next((f for f in range(ALTO - 1, -1, -1) if glifos[d + f]), None)
    if fin is None:
        return
    # La letra no se mueve nunca mas de lo que cabe: si no llega, recorta.
    baja = min(max(0, deseada - tope), ALTO - 1 - fin)
    for f in range(ALTO):
        src = f - baja
        glifos[t + f] = glifos[d + src] if 0 <= src <= fin else 0
    for i, px in enumerate(acento):
        glifos[t + primera + i] |= px


def comprobar_acentos(glifos):
    """Lo compuesto tiene que cumplir lo que texto.rs y la GUI esperan."""
    for destino, letra, acento, mayus in COMPONER:
        g = bytes(glifos[destino * ALTO:(destino + 1) * ALTO])
        assert any(g), "el compuesto 0x%02X salio en blanco" % destino
        assert g != bytes(glifos[letra * ALTO:(letra + 1) * ALTO]), \
            "el compuesto 0x%02X quedo como la letra sola" % destino
        # El acento puro (la letra empieza por debajo) tiene que salir entero.
        ultima = (2 if len(acento) >= 3 else 1) if mayus else 2
        primera = ultima + 1 - len(acento)
        for i, px in enumerate(acento):
            assert g[primera + i] & px == px, \
                "falta parte del acento de 0x%02X en la fila %d" % (destino, primera + i)
    # La ROM pone la virgulilla de la 'ñ' en las filas 1-2 con la 3 en blanco:
    # 'ã' y 'õ' la imitan (son minusculas, mismo criterio).
    a3 = glifos[0xE3 * ALTO:0xE4 * ALTO]
    assert a3[1] == 0x76 and a3[2] == 0xDC, "la virgulilla de ã no va como la de ñ"
    assert a3[3] == 0, "entre la virgulilla y la 'a' tiene que quedar la fila 3"


def filas_rust(glifos):
    """Los glifos como lineas de Rust, 12 bytes por linea (un glifo por linea)."""
    lineas = []
    for ch in range(256):
        trozo = glifos[ch * ALTO:(ch + 1) * ALTO]
        lineas.append("    " + " ".join("0x%02X," % b for b in trozo))
    return "\n".join(lineas)


def leer_final_de_linea(ruta):
    try:
        with open(ruta, "rb") as f:
            return "\r\n" if b"\r\n" in f.read() else "\n"
    except OSError:
        return "\n"


PLANTILLA = '''//! Fuente de la VGA (extraida de `{fon}`).
//!
//! 8 px de ancho, un byte por fila (MSB a la izquierda) y `ALTO` filas por
//! glifo. `GLIFOS[caracter]` es el dibujo de ese caracter, que es como lo lee
//! el `CanvasRenderer` de Toby UI (`glifo = addr + caracter * bytes_por_glifo`)
//! y como lo publica `bios::get_font`.
//!
//! Fichero generado: no se edita a mano. Se regenera con
//! `python herramientas/gen_fuente.py`.

/// Pixeles de ancho de cada glifo.
pub const ANCHO: usize = 8;
/// Filas (bytes) que ocupa cada glifo.
pub const ALTO: usize = 12;

/// Los 256 glifos, uno tras otro y en el indice de su caracter.
pub static GLIFOS: [u8; 3072] = [
{glifos}
];

#[cfg(test)]
mod pruebas {{
    use super::*;

    fn glifo(ch: u8) -> &'static [u8] {{
        let i = ch as usize * ALTO;
        &GLIFOS[i..i + ALTO]
    }}

    /// `GLIFOS[caracter]` tiene que ser el dibujo de ese caracter.
    ///
    /// Si la tabla se genera desplazada (p. ej. copiando el bloque de glifos
    /// con indice 0 cuando la fuente arranca en el caracter 1), todo el texto
    /// sale con el caracter siguiente y `Hola` se dibuja como `Ipmb`. El unico
    /// glifo en blanco de la zona imprimible es el del espacio, asi que el
    /// despiste se detecta mirando donde esta ese blanco.
    #[test]
    fn el_indice_de_la_tabla_es_el_caracter() {{
        let blancos: Vec<u8> = (0x20u8..=0x7E).filter(|&c| glifo(c).iter().all(|&b| b == 0)).collect();
        assert_eq!(blancos, vec![0x20], "la tabla de glifos no esta indexada por caracter");
    }}

    /// Las letras que salen en la interfaz no pueden compartir glifo.
    #[test]
    fn letras_distintas() {{
        assert_ne!(glifo(b'A'), glifo(b'B'));
        assert_ne!(glifo(b'M'), glifo(b'N'));
        assert_ne!(glifo(b'0'), glifo(b'O'));
        assert_ne!(glifo(b'a'), glifo(b'o'));
        // La 'A' empieza en punta (sangrada); la 'B' tiene el asta a la izquierda.
        let primera_a = (0..ALTO).find(|&f| glifo(b'A')[f] != 0).unwrap();
        let primera_b = (0..ALTO).find(|&f| glifo(b'B')[f] != 0).unwrap();
        assert!(glifo(b'A')[primera_a] & 0x80 == 0, "el primer pixel de la 'A' tendria que ir sangrado");
        assert!(primera_b >= primera_a, "la 'B' no arranca antes que la 'A'");
    }}

    /// Un texto largo no puede tener ningun glifo en blanco de mas: si la
    /// fuente estuviera desplazada, los espacios saldrian con el dibujo del
    /// '!' (una barra con un punto).
    #[test]
    fn el_espacio_esta_en_blanco() {{
        assert!(glifo(b' ').iter().all(|&b| b == 0));
        assert!(glifo(b'!').iter().any(|&b| b != 0));
    }}

    /// Los 23 acentos que compone `bios::COMPONER` en el kernel estan aqui ya
    /// compuestos, cada uno en su sitio Latin-1.
    #[test]
    fn los_acentos_compuestos_estan_compuestos() {{
        // La virgulilla de la 'ã' va en las filas 1-2, como la de la 'ñ' de
        // la ROM, y entre medias queda la fila 3 en blanco.
        let a = glifo(0xE3);
        assert_eq!(&a[1..=2], &[0x76, 0xDC]);
        assert_eq!(a[3], 0);
        // La 'É' compuesta (0xC9) lleva su agudo en las filas 0-2 y la 'E'
        // justo debajo, como la 'É' de la ROM (0x90).
        let e = glifo(0xC9);
        assert_eq!(&e[0..=2], &[0x18, 0x30, 0x60]);
        assert_ne!(glifo(0xC9), glifo(b'E'), "tiene que llevar el acento");
    }}
}}
'''


def main():
    with open(FON, "rb") as f:
        datos = f.read()
    fnt = recurso_fnt(datos)
    glifos, primero, ultimo = fuente_del_fnt(fnt)
    comprobar(glifos)

    # bios::COMPONER: los 23 acentos que el kernel compone en su copia RAM,
    # aqui directamente sobre la tabla que se emite.
    for destino, base, acento, mayus in COMPONER:
        componer_acento(glifos, destino, base, acento, mayus)
    comprobar_acentos(glifos)

    print("%s: recurso FNT de %d bytes, caracteres 0x%02X..0x%02X" % (FON, len(fnt), primero, ultimo))
    for ch, nombre in [(0x20, "espacio"), (0x41, "A"), (0x42, "B"), (0x61, "a"), (0x30, "0"), (0x7A, "z"),
                       (0xA4, "ñ"), (0xE3, "ã compuesto"), (0x90, "É de la ROM"), (0xC9, "É compuesto"),
                       (0xA5, "Ñ"), (0xFD, "ý compuesto")]:
        print("  0x%02X (%s):" % (ch, nombre))
        for linea in arte(glifos, ch):
            print("    " + linea)

    eol = leer_final_de_linea(SALIDA)
    texto = PLANTILLA.format(fon=FON, glifos=filas_rust(glifos))
    with open(SALIDA, "w", encoding="utf-8", newline=eol) as f:
        f.write(texto)
    print("escrito %s (%d bytes)" % (os.path.normpath(SALIDA), len(texto)))


if __name__ == "__main__":
    main()
