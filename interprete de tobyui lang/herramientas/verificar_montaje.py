# -*- coding: utf-8 -*-
"""Comprueba que controles_montaje.png monta los controles COMO EL CSS del kit.

La comparación es por MEDIA de una zona contra la pieza PNG del kit escalada
al mismo tamaño (los skins llevan degradado, así que un píxel suelto no sirve).
Además se miden sueltos el tick, el punto, las flechas, la raya y las tintas.
Sale PASS/FAIL por sonda y código de salida 1 si algo falla.
"""
import os
import sys
from PIL import Image

from pathlib import Path

RAIZ = str(Path(__file__).resolve().parents[2] / "AQUA")
PNG = str(Path(__file__).resolve().parents[1] / "capturas" / "controles_montaje.png")

fallos = 0


def abrir(nombre):
    return Image.open(os.path.join(RAIZ, nombre)).convert("RGBA")


def media_pieza(nombre, w, h):
    """Media de una pieza del kit ya escalada al tamaño del control."""
    im = abrir(nombre).resize((max(w, 1), max(h, 1)), Image.BILINEAR)
    px = [p[:3] for p in im.getdata() if p[3] >= 128 and max(p[:3]) >= 70]
    if not px:
        return (0, 0, 0)
    n = len(px)
    return tuple(sum(p[i] for p in px) // n for i in range(3))


def media_zona(im, caja, saltar_blanco=True):
    """Media de una zona del render, sin píxeles negros (texto/tick) ni del
    fondo blanco de la ventana."""
    x0, y0, x1, y1 = caja
    px = []
    for x in range(x0, x1):
        for y in range(y0, y1):
            r, g, b = im.getpixel((x, y))
            if max(r, g, b) < 70:
                continue
            if saltar_blanco and r > 250 and g > 250 and b > 250:
                continue
            px.append((r, g, b))
    if not px:
        return (0, 0, 0)
    n = len(px)
    return tuple(sum(p[i] for p in px) // n for i in range(3))


def cuenta(im, caja, prueba):
    x0, y0, x1, y1 = caja
    return sum(1 for x in range(x0, x1) for y in range(y0, y1)
               if prueba(im.getpixel((x, y))))


def dist(a, b):
    return max(abs(x - y) for x, y in zip(a, b))


def sonda(nombre, observado, esperado, tol=28):
    """PASS si la media observada está a `tol` de la de la pieza."""
    global fallos
    d = dist(observado, esperado)
    ok = d <= tol
    if not ok:
        fallos += 1
    print(f"{'PASS' if ok else 'FAIL'} {nombre:40s} {observado} vs {esperado} d={d}")


def exigir(nombre, valor, minimo):
    global fallos
    ok = valor >= minimo
    if not ok:
        fallos += 1
    print(f"{'PASS' if ok else 'FAIL'} {nombre:40s} {valor} (hace falta >={minimo})")


def prohibir(nombre, valor, maximo):
    global fallos
    ok = valor <= maximo
    if not ok:
        fallos += 1
    print(f"{'PASS' if ok else 'FAIL'} {nombre:40s} {valor} (hace falta <={maximo})")


im = Image.open(PNG).convert("RGB")

print("== check (check.css) ==")
# c1 (20,52): casilla 18x18 en y=55, marcada -> pieza azul.
sonda("c1 casilla marcada = pieza azul",
      media_zona(im, (21, 56, 37, 72)),
      media_pieza("blue_check.png", 18, 18))
# El tick mide 1,15 x el alto y se centra en la casilla: las puntas finas
# quedan semitransparentes al bajar a 21 px, así que FUERA de la casilla sólo
# se mira si el fondo blanco de la ventana deja de ser blanco (pisa el tick).
def filas_sobresalidas(y0, y1):
    filas = []
    for y in range(y0, y1):
        n = sum(1 for x in range(18, 42)
                if any(abs(c - 255) >= 8 for c in im.getpixel((x, y))))
        if n >= 1:
            filas.append(y)
    return filas


arriba = [y for y in filas_sobresalidas(50, 55) if y < 55]
abajo = [y for y in filas_sobresalidas(73, 79) if y > 72]
exigir(f"c1 tick sobresale por ARRIBA (filas {arriba})", len(arriba), 1)
exigir(f"c1 tick sobresale por ABAJO (filas {abajo})", len(abajo), 1)
sonda("c2 sin marcar = pieza gris (aunque variant=green)",
      media_zona(im, (171, 56, 187, 72)),
      media_pieza("gray_check.png", 18, 18))
sonda("c3 apagado = pieza gris",
      media_zona(im, (331, 56, 347, 72)),
      media_pieza("gray_check.png", 18, 18))
sonda("c4 marcado white->blue",
      media_zona(im, (21, 240, 37, 256)),
      media_pieza("blue_check.png", 18, 18))
# Etiqueta de check: #1d1d1f.
exigir("c1 etiqueta tinta oscura",
       cuenta(im, (46, 57, 110, 71),
              lambda p: dist(p, (0x1D, 0x1D, 0x1F)) < 18), 12)

print("== radio (radio.css) ==")
sonda("r1 esfera marcada = pieza azul",
      media_zona(im, (21, 88, 38, 104)),
      media_pieza("blue_radio.png", 19, 18))
exigir("r1 punto central (dot.png)",
       cuenta(im, (26, 90, 34, 102), lambda p: max(p) < 70), 12)
sonda("r2 sin marcar = esfera gris",
      media_zona(im, (171, 88, 188, 104)),
      media_pieza("gray_radio.png", 19, 18))
sonda("r3 apagado = esfera gris",
      media_zona(im, (331, 88, 348, 104)),
      media_pieza("gray_radio.png", 19, 18))
exigir("r1 etiqueta tinta oscura",
       cuenta(im, (47, 89, 111, 103),
              lambda p: dist(p, (0x1D, 0x1D, 0x1F)) < 18), 12)

print("== circular (circular.css) ==")
sonda("k1 cara = esfera roja del radio",
      media_zona(im, (22, 118, 51, 146)),
      media_pieza("red_radio.png", 33, 32))
prohibir("k1 sin punto central", cuenta(im, (32, 127, 41, 137),
                                        lambda p: max(p) < 70), 4)
sonda("k2 apagado = esfera gris",
      media_zona(im, (66, 118, 95, 146)),
      media_pieza("gray_radio.png", 33, 32))

print("== segmented (segmented.css) ==")
# sg (120,116) 244x24 -> opciones 120-205 / 205-290 / 290-364, activa la 2ª.
sonda("sg opcion 1 inactiva = pieza blanca",
      media_zona(im, (122, 118, 141, 138)),
      media_pieza("segmented_withe_body.png", 19, 20))
sonda("sg opcion 2 ACTIVA = pieza azul",
      media_zona(im, (260, 118, 288, 138)),
      media_pieza("segmented_blue_body.png", 28, 20))
sonda("sg opcion 3 inactiva = pieza blanca",
      media_zona(im, (330, 118, 362, 138)),
      media_pieza("segmented_withe_body.png", 32, 20))
exigir("sg texto activo en BLANCO",
       cuenta(im, (219, 124, 275, 132), lambda p: min(p) > 245), 20)
exigir("sg texto inactivo #3c3c3e",
       cuenta(im, (142, 124, 198, 132),
              lambda p: dist(p, (0x3C, 0x3C, 0x3E)) < 16), 12)
# sg2 (160,266) 84x24 -> 160-202 / 202-244, activa la 2ª (roja).
sonda("sg2 opcion 1 inactiva = blanca",
      media_zona(im, (162, 268, 176, 288)),
      media_pieza("segmented_withe_body.png", 14, 20))
sonda("sg2 opcion 2 ACTIVA = roja",
      media_zona(im, (225, 268, 242, 288)),
      media_pieza("segmented_red_body.png", 17, 20))

print("== popup (popup.css) ==")
# pp (330,116) 95x24: blanca 330-394, boton magenta 394-425 (cap 10).
sonda("pp parte blanca = pieza blanca",
      media_zona(im, (332, 118, 345, 138)),
      media_pieza("segmented_withe_body.png", 13, 20))
sonda("pp boton = capa magenta (extremo derecho volteado)",
      media_zona(im, (415, 117, 424, 139)),
      media_pieza("segmented_magenta_right.png", 9, 22))
exigir("pp DOS flechas negras",
       cuenta(im, (404, 120, 415, 136), lambda p: max(p) < 60), 25)
mag = abrir("segmented_magenta_body.png").getpixel((10, 209))[:3]
raya = tuple((f * 38 + d * 62) // 100 for f, d in zip((8, 28, 58), mag))
sonda("pp raya entre flechas rgba(8,28,58,.38)",
      im.getpixel((399, 128)), raya, 16)
# p2 (20,266) 79x24 variante blanca: el popup no tiene blanco -> azul.
sonda("pp2 boton (white->blue) = capa azul",
      media_zona(im, (90, 268, 97, 287)),
      media_pieza("segmented_blue_right.png", 7, 19))

print("== field (field.css) ==")
# f1 (20,156) 180x24: piel blanca del segmented (no el carril del slider).
sonda("f1 cuerpo = pieza blanca",
      media_zona(im, (131, 158, 198, 178)),
      media_pieza("segmented_withe_body.png", 67, 20))
sonda("f2 cuerpo = pieza blanca",
      media_zona(im, (351, 158, 408, 178)),
      media_pieza("segmented_withe_body.png", 57, 20))
exigir("f2 texto #3c3c3e",
       cuenta(im, (240, 164, 345, 172),
              lambda p: dist(p, (0x3C, 0x3C, 0x3E)) < 16), 30)
exigir("f1 placeholder #9a9a9e",
       cuenta(im, (30, 164, 126, 172),
              lambda p: dist(p, (0x9A, 0x9A, 0x9E)) < 14), 20)

print("== slider (slider.css) ==")
# s1 (20,196) 200x24: pista 22% abajo (y 215..220), gris y SIN tramo pintado.
sonda("s1 pista = carril gris",
      media_zona(im, (24, 216, 216, 220), saltar_blanco=False),
      media_pieza("graybar_body.png", 192, 4))
sonda("s1 pulgar = esfera azul",
      media_zona(im, (76, 203, 88, 214)),
      media_pieza("blue_radio.png", 12, 11))
# s2 gota (250,196) 190x24 valor 70 -> x = 250 + (190-14)*70/100 = 373.
sonda("s2 pulgar = GOTA roja (variant=red)",
      media_zona(im, (374, 203, 386, 214)),
      media_pieza("slider_thumb_red.png", 12, 11))
# s3 apagado (250,236) valor 10/50 -> x = 250 + (190-14)*20/100 = 285.
sonda("s3 pulgar apagado = esfera gris",
      media_zona(im, (286, 243, 298, 254)),
      media_pieza("gray_radio.png", 12, 11))

print("== boton (button.css) ==")
# La etiqueta va en --aqua-label #0a2e52, no en blanco. Se miran sólo los
# píxeles de tinta (los oscuros), no el cuerpo claro de la piel.
marinos = cuenta(im, (32, 24, 88, 32), lambda p: max(p) < 110 and p[2] > p[0])
blancos = cuenta(im, (32, 24, 88, 32), lambda p: min(p) > 245)
px_txt = [im.getpixel((x, y)) for x in range(32, 88) for y in range(24, 32)
          if max(im.getpixel((x, y))) < 110]
media_txt = tuple(sum(p[i] for p in px_txt) // len(px_txt)
                  for i in range(3)) if px_txt else (0, 0, 0)
exigir("etiqueta del botón en azul marino", marinos, 30)
prohibir("etiqueta sin blancos", blancos, 2)
sonda("etiqueta #0a2e52", media_txt, (0x0A, 0x2E, 0x52), 30)

print()
if fallos:
    print(f"{fallos} SONDA(S) FALLAN")
    sys.exit(1)
print("TODO CORRECTO")
