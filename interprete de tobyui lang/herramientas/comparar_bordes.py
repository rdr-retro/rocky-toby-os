"""Comprueba que los bordes de un boton AQUA salen suaves, sin escalones.

Las pieles del kit miden 258x409 y un boton mide 24 px de alto, asi que el
motor las reduce bastante. Si se muestrea con vecino mas proximo (se salta 16
de cada 17 pixeles de la fuente) la esquina redondeada sale a escalones de 255
a 0, que es el "diente de sierra" que se ve a simple vista. El motor la reduce
con media de area (`toby_ui/renderer.rs::muestra_escalada`), y entonces la
transicion pasa por pixeles semipintados.

Este script localiza el primer pixel azul de la piel dentro de una zona, mira
la esquina superior izquierda del boton y mide el mayor salto de color entre
pixeles vecinos: con el filtro sale ~20 y sin el filtro salia ~337. Sale con 0
si el borde es suave y con 1 si hay escalones, asi se comprueba sin mirar nada.

    python comparar_bordes.py salida.png [x0 y0 x1 y1]

El documento `documentos/ejemplo.toby` del interprete deja el primer boton en (26,17); la
hoja del ensamblador empieza en (360,168), por si se le pasa esa zona.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import leer_texto as lt  # noqa: E402

# Un pixel de la piel azul del kit (el cuerpo del boton).
def es_piel(p):
    return p[2] > 120 and p[0] < 90 and p[1] < 140


# Salto maximo admitido entre dos pixeles vecinos de la esquina. El fondo de
# la hoja es blanco y la piel es azul oscuro, asi que un borde sin filtrar
# supera los 300; el filtrado se queda por debajo de 30.
SALTO_MAXIMO = 80

# Ventana que se mira alrededor de la esquina.
ANCHO, ALTO = 14, 12


def esquina(f, ancho, alto, x0, y0, x1, y1):
    """(x, y) de la esquina superior izquierda del primer boton azul."""
    for y in range(y0, alto):
        for x in range(x0, ancho):
            if es_piel(lt.pixel(f, x, y)):
                break
        else:
            continue
        break
    else:
        return None
    while y > y0 and es_piel(lt.pixel(f, x, y - 1)):
        y -= 1
    while x > x0 and es_piel(lt.pixel(f, x - 1, y)):
        x -= 1
    return x, y


def mapa(f, x0, y0, fondo):
    """Mapa de la esquina: ' ' = fondo, '#' = piel, '+' = transicion."""
    lineas = []
    for y in range(y0, y0 + ALTO):
        s = ""
        for x in range(x0, x0 + ANCHO):
            p = lt.pixel(f, x, y)
            d = sum(abs(p[i] - fondo[i]) for i in range(3))
            s += " " if d < 12 else ("+" if d < 300 else "#")
        lineas.append(s)
    return lineas


def mayor_salto(f, x0, y0):
    """Mayor distancia de color entre dos pixeles vecinos de la esquina."""
    mejor, donde = 0, None
    for y in range(y0, y0 + ALTO):
        p0 = lt.pixel(f, x0, y)
        for x in range(x0 + 1, x0 + ANCHO):
            p1 = lt.pixel(f, x, y)
            d = sum(abs(p1[i] - p0[i]) for i in range(3))
            if d > mejor:
                mejor, donde = d, (x, y)
            p0 = p1
    return mejor, donde


def transiciones(f, x0, y0):
    """Pixeles de la esquina que no son ni fondo puro ni piel pura."""
    n = 0
    for y in range(y0, y0 + ALTO):
        for x in range(x0, x0 + ANCHO):
            p = lt.pixel(f, x, y)
            d = sum(abs(p[i] - 255) for i in range(3))
            if 12 <= d < 300:
                n += 1
    return n


def main(argv):
    if len(argv) < 2:
        print(__doc__.strip())
        return 2
    ancho, alto, f = lt.leer_png(argv[1])
    x0, y0 = 0, 0
    x1, y1 = ancho, alto
    if len(argv) >= 6:
        x0, y0, x1, y1 = (int(v) for v in argv[2:6])
    c = esquina(f, ancho, alto, x0, y0, min(x1, ancho), min(y1, alto))
    if c is None:
        print("no hay ningun pixel azul de la piel en (%d,%d)-(%d,%d)" % (x0, y0, x1, y1))
        return 2
    bx, by = c
    fondo = lt.pixel(f, bx - 1, by - 1) if bx > 0 and by > 0 else (255, 255, 255)
    print("%s: esquina del boton en (%d,%d)" % (argv[1], bx, by))
    for l in mapa(f, bx - 3, by - 3, fondo):
        print("   " + l)
    salto, donde = mayor_salto(f, bx - 3, by - 3)
    print("   pixeles de transicion: %d" % transiciones(f, bx - 3, by - 3))
    if salto <= SALTO_MAXIMO:
        print("OK    mayor salto %d (<= %d): borde suave, sin dientes de sierra"
              % (salto, SALTO_MAXIMO))
        return 0
    print("MAL   mayor salto %d (> %d) en %s: hay escalones"
          % (salto, SALTO_MAXIMO, donde))
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
