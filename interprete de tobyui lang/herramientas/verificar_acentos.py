"""Verifica por pixeles que los acentos salen dibujados en el render (F10.5).

La fuente del interprete (src/host/fuente.rs) es la receta: para cada texto del
documento `acentos.toby` se busca su secuencia de glifos en la region del
control y se exige que cada pixel coincida con la mascara del glifo (tinta
donde hay punto, sin tinta donde no lo hay). Si el parser contara bytes en
vez de caracteres, o el renderer dibujara `s.bytes()`, o la tabla Unicode→
glifos fallara, aqui saldria FALLO.

Uso:  python herramientas/verificar_acentos.py   (tras renderizar acentos.toby)
"""

import os
import re
import sys

from PIL import Image

AQUI = os.path.dirname(os.path.abspath(__file__))
RUTA_FUENTE = os.path.join(AQUI, "..", "src", "host", "fuente.rs")
RUTA_TEXTO = os.path.join(AQUI, "..", "..", "kernel", "src", "texto.rs")
RUTA_PNG = os.path.join(AQUI, "..", "capturas", "acentos.png")
ANCHO, ALTO = 8, 12

HAY = []


def carga_glifos():
    with open(RUTA_FUENTE, encoding="utf-8") as f:
        t = f.read()
    bloque = t.split("pub static GLIFOS: [u8; 3072] = [", 1)[1].split("];", 1)[0]
    b = bytes(int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]{2})", bloque))
    assert len(b) == 3072, "la tabla tiene %d bytes, no 3072" % len(b)
    return b


def carga_tabla():
    """La tabla Unicode→glifos se lee del propio kernel/src/texto.rs.

    No se copia aqui: si cambia la del motor, cambia tambien la que comprueba
    este guion (misma fuente de verdad).
    """
    with open(RUTA_TEXTO, encoding="utf-8") as f:
        t = f.read()
    cp437 = [
        (int(g, 16), int(u, 16))
        for g, u in re.findall(
            r"\((0x[0-9A-Fa-f]{2}), (0x[0-9A-Fa-f]{4})\)",
            t.split("pub const CP437", 1)[1].split("];", 1)[0],
        )
    ]
    comp = {
        int(u, 16)
        for u in re.findall(
            r"0x([0-9A-Fa-f]{4})", t.split("const COMPONER", 1)[1].split("];", 1)[0]
        )
    }
    assert len(cp437) == 53, "la tabla CP437 tiene %d entradas" % len(cp437)
    assert len(comp) == 23, "COMPONER tiene %d destinos" % len(comp)
    return cp437, comp


GLIFOS = carga_glifos()
CP437, COMPONER = carga_tabla()
IMAGEN = Image.open(RUTA_PNG).convert("RGB")
PX = IMAGEN.load()


def glifo_de(ch):
    """El mismo criterio que texto::glifo_de del motor."""
    u = ord(ch)
    if u < 0x80:
        return u
    for g, x in CP437:
        if x == u and g not in COMPONER:
            return g
    if u in COMPONER:
        return u
    return ord("?")


def mascara(ch):
    return [[(GLIFOS[ch * ALTO + f] >> (7 - c)) & 1 for c in range(ANCHO)] for f in range(ALTO)]


def oscura(t):
    """Tinta oscura (marino del boton #0a2e52, tinta de campo #3c3c3e)."""
    return max(t) < 128


def clara(t):
    """Tinta blanca (segmento activo del segmented)."""
    return min(t) > 200


def gris(t):
    """Tinta de placeholder (#9a9a9e)."""
    return abs(t[0] - 0x9A) < 50 and abs(t[1] - 0x9A) < 50 and abs(t[2] - 0x9E) < 50


def fallos(x, y, ch, es):
    m = mascara(glifo_de(ch))
    n = 0
    for f in range(ALTO):
        for c in range(ANCHO):
            if es(PX[x + c, y + f]) != bool(m[f][c]):
                n += 1
    return n


def secuencia(region, texto, es):
    """Primera posicion de la region donde `texto` coincide pixel a pixel."""
    x0, y0, x1, y1 = region
    for y in range(y0, y1):
        for x in range(x0, x1 - ANCHO * (len(texto) - 1)):
            if all(fallos(x + ANCHO * i, y, c, es) <= 1 for i, c in enumerate(texto)):
                return (x, y)
    return None


def comprueba(nombre, cond):
    HAY.append(cond)
    print(("PASS " if cond else "FALLO ") + nombre)
    return cond


def main():
    b = secuencia((20, 16, 100, 42), "ÁÉÍÓÚÑ", oscura)
    comprueba("boton: ÁÉÍÓÚÑ en tinta marino en %s" % (b,), b is not None)

    s = secuencia((20, 52, 101, 77), "Pequeño", oscura)
    comprueba("segmented: Pequeño (con ñ) en %s" % (s,), s is not None)

    f1 = secuencia((20, 88, 240, 112), "Ñandú ãõ ý", oscura)
    comprueba("campo con value: Ñandú ãõ ý en %s" % (f1,), f1 is not None)

    f2 = secuencia((20, 124, 240, 148), "¿Cómo estás?", gris)
    comprueba("placeholder: ¿Cómo estás? en %s" % (f2,), f2 is not None)

    r = secuencia((44, 160, 210, 184), "Ñoño Ágil", oscura)
    comprueba("etiqueta de radio: Ñoño Ágil en %s" % (r,), r is not None)

    c = secuencia((44, 192, 210, 216), "ÄÖÜ Çç", oscura)
    comprueba("etiqueta de check: ÄÖÜ Çç en %s" % (c,), c is not None)

    # El active del segmented lleva texto blanco (Mediano).
    w = secuencia((100, 52, 181, 77), "Mediano", clara)
    comprueba("segmento activo: Mediano en blanco en %s" % (w,), w is not None)

    print("TODO CORRECTO (%d/%d)" % (sum(HAY), len(HAY)))
    sys.exit(0 if all(HAY) else 1)


if __name__ == "__main__":
    main()
