"""Reconoce el TEXTO dibujado en un PNG de Toby UI comparando cada celda de
8x12 con la tabla de glifos incrustada.

Es la unica forma de comprobar el texto en este entorno (sin ojos): se saca la
tinta de cada celda y se compara con `fuente.rs`. Si la tabla estuviera
desplazada, el reconocimiento devolveria el caracter siguiente.

Uso:
  python leer_texto.py <png> buscar <x0> <y0> <x1> <y1> "<texto esperado>"
  python leer_texto.py <png> leer <x> <y> <n>
"""

import re
import struct
import sys
import zlib

ANCHO, ALTO = 8, 12
import os
AQUI = os.path.dirname(os.path.abspath(__file__))
FUENTE = os.path.join(AQUI, "..", "src", "fuente.rs")


# --- PNG ------------------------------------------------------------------

def leer_png(ruta):
    datos = open(ruta, "rb").read()
    assert datos[:8] == b"\x89PNG\r\n\x1a\n", "no es PNG"
    pos, ancho, alto, idat = 8, None, None, b""
    while pos < len(datos):
        (lon,) = struct.unpack(">I", datos[pos:pos + 4])
        tipo = datos[pos + 4:pos + 8]
        cuerpo = datos[pos + 8:pos + 8 + lon]
        pos += 12 + lon
        if tipo == b"IHDR":
            ancho, alto, prof, color, _c, _f, _e = struct.unpack(">IIBBBBB", cuerpo)
            assert prof == 8 and color == 6, (prof, color)
        elif tipo == b"IDAT":
            idat += cuerpo
        elif tipo == b"IEND":
            break
    crudo = zlib.decompress(idat)
    bpp, paso = 4, ancho * 4
    filas, previo, off = [], bytearray(paso), 0
    for _ in range(alto):
        f = crudo[off]
        off += 1
        linea = bytearray(crudo[off:off + paso])
        off += paso
        if f == 1:
            for i in range(bpp, paso):
                linea[i] = (linea[i] + linea[i - bpp]) & 0xFF
        elif f == 2:
            for i in range(paso):
                linea[i] = (linea[i] + previo[i]) & 0xFF
        elif f == 3:
            for i in range(paso):
                a = linea[i - bpp] if i >= bpp else 0
                linea[i] = (linea[i] + ((a + previo[i]) >> 1)) & 0xFF
        elif f == 4:
            for i in range(paso):
                a = linea[i - bpp] if i >= bpp else 0
                b = previo[i]
                c = previo[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                linea[i] = (linea[i] + pr) & 0xFF
        filas.append(bytes(linea))
        previo = linea
    return ancho, alto, filas


def pixel(filas, x, y):
    i = x * 4
    return tuple(filas[y][i:i + 3])


# --- Fuente ---------------------------------------------------------------

def leer_glifos(ruta=FUENTE):
    txt = open(ruta, "r", encoding="utf-8", errors="replace").read()
    i = txt.index("GLIFOS")
    i = txt.index("= [", i) + 2
    j = txt.index("];", i)
    vals = [int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]{2})", txt[i:j])]
    assert len(vals) == 256 * ALTO, "la tabla no tiene 256 glifos de %d bytes" % ALTO
    return {ch: tuple(vals[ch * ALTO:(ch + 1) * ALTO]) for ch in range(256)}


# --- Reconocimiento -------------------------------------------------------

def mascara(filas, x, y, fondo):
    """Que pixeles de la celda (x,y) son tinta: los que no son del color de fondo."""
    m = []
    for f in range(ALTO):
        for c in range(ANCHO):
            m.append(0 if pixel(filas, x + c, y + f) == fondo else 1)
    return tuple(m)


def glifo_mascara(glifos, ch):
    """La misma forma que `mascara` para el glifo del caracter `ch`."""
    m = []
    for b in glifos[ch]:
        for c in range(ANCHO):
            m.append((b >> (7 - c)) & 1)
    return tuple(m)


def color_fondo(filas, x0, y0, x1, y1):
    cuenta = {}
    for y in range(y0, y1):
        for x in range(x0, x1):
            p = pixel(filas, x, y)
            cuenta[p] = cuenta.get(p, 0) + 1
    return max(cuenta.items(), key=lambda kv: kv[1])[0]


def reconocer(filas, glifos, x, y, n, fondo):
    """Devuelve (texto, fallos): reconoce `n` celdas desde (x,y)."""
    texto, fallos = "", 0
    invertido = {v: k for k, v in glifos.items()}
    for i in range(n):
        m = mascara(filas, x + i * ANCHO, y, fondo)
        if not any(m):
            texto += " "  # glifo en blanco: el del espacio
            continue
        ch = invertido.get(m)
        if ch is None:
            mejor, dist = None, 1e9
            for cand in range(256):
                d = sum(a != b for a, b in zip(m, glifo_mascara(glifos, cand)))
                if d < dist:
                    mejor, dist = cand, d
            fallos += dist
            ch = mejor
        texto += chr(ch) if 32 <= ch < 127 else "?"
    return texto, fallos


def verificar(filas, glifos, x, y, esperado, fondo):
    motivos = 0
    for i, ch in enumerate(esperado):
        m = mascara(filas, x + i * ANCHO, y, fondo)
        g = glifo_mascara(glifos, ord(ch))
        motivos += sum(a != b for a, b in zip(m, g))
    return motivos


def buscar(filas, glifos, x0, y0, x1, y1, esperado):
    """Busca donde esta dibujado `esperado` dentro de la region y comprueba que
    coincide glifo a glifo. Devuelve (x, y, pixeles_mal)."""
    fondo = color_fondo(filas, x0, y0, x1, y1)
    n = len(esperado)
    mejor = (None, None, 1e9)
    for y in range(y0, y1 - ALTO + 1):
        for x in range(x0, x1 - n * ANCHO + 1):
            mal = verificar(filas, glifos, x, y, esperado, fondo)
            if mal < mejor[2]:
                mejor = (x, y, mal)
            if mal == 0:
                return mejor
    return mejor


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    png, modo = sys.argv[1], sys.argv[2]
    glifos = leer_glifos()
    ancho, alto, filas = leer_png(png)
    print("PNG %dx%d" % (ancho, alto))

    if modo == "buscar":
        x0, y0, x1, y1 = (int(v) for v in sys.argv[3:7])
        esperado = sys.argv[7]
        x, y, mal = buscar(filas, glifos, x0, y0, x1, y1, esperado)
        estado = "OK" if mal == 0 else "MAL"
        print("%s  buscar %r en (%d,%d)-(%d,%d): dibujado en (%s,%s), pixeles mal=%s"
              % (estado, esperado, x0, y0, x1, y1, x, y, mal))
        if x is not None:
            texto, _ = reconocer(filas, glifos, x, y, len(esperado),
                                 color_fondo(filas, x0, y0, x1, y1))
            print("      leido: %r" % texto)
        return 0 if mal == 0 else 1

    if modo == "leer":
        x, y, n = (int(v) for v in sys.argv[3:6])
        fondo = pixel(filas, x, y + ALTO - 1)
        texto, fallos = reconocer(filas, glifos, x, y, n, fondo)
        print("leer (%d,%d) x%d  fondo=%s  fallos por celda=%d" % (x, y, n, fondo, fallos))
        print("      %r" % texto)
        return 0

    print("modo desconocido: %s" % modo)
    return 2


if __name__ == "__main__":
    sys.exit(main())
