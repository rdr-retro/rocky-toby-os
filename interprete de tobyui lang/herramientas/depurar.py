# -*- coding: utf-8 -*-
"""Volcado rapido de zonas para depurar las sondas."""
from PIL import Image

import os

PNG = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "capturas", "controles_montaje.png")
im = Image.open(PNG).convert("RGB")


def fila(y, x0, x1, paso=4):
    return " ".join(f"{im.getpixel((x, y))[0]:3d},{im.getpixel((x, y))[1]:3d},{im.getpixel((x, y))[2]:3d}"
                    for x in range(x0, x1, paso))


print("== radio r1 (20,84) 91x24: columna x=29 ==")
for y in range(84, 108, 2):
    print(f"  y={y:3d} {im.getpixel((29, y))}")
print("== radio r1: fila y=96 ==")
print("  " + fila(96, 18, 60, 3))

print("== segmented sg: fila y=135 ==")
print("  " + fila(135, 120, 366, 8))
print("== segmented sg: fila y=120 (arriba, sin texto) ==")
print("  " + fila(120, 120, 366, 8))
print("== segmented sg: columna x=250 ==")
for y in range(116, 141, 2):
    print(f"  y={y:3d} {im.getpixel((250, y))}")

print("== boton etiqueta (20,16) 80x24: pixeles no claros ==")
osc = []
for x in range(30, 90):
    for y in range(22, 34):
        r, g, b = im.getpixel((x, y))
        if r < 200 and g < 200:
            osc.append((x, y, (r, g, b)))
print(f"  {len(osc)} px oscuros; primeros 12: {osc[:12]}")

print("== check c3 casilla (330,55) 18x18: muestra ==")
for y in range(55, 74, 3):
    print(f"  y={y:3d} " + " ".join(f"{im.getpixel((x, y))}" for x in (332, 336, 340, 344)))

print("== popup pp boton (394..425,116..140): muestra ==")
for y in range(117, 140, 3):
    print(f"  y={y:3d} " + " ".join(f"{im.getpixel((x, y))}" for x in (396, 404, 410, 418, 423)))

print("== slider s2 gota: zona pulgar (373..387, 202..215) ==")
for y in range(202, 216, 2):
    print(f"  y={y:3d} " + " ".join(f"{im.getpixel((x, y))}" for x in (374, 378, 380, 384, 386)))
