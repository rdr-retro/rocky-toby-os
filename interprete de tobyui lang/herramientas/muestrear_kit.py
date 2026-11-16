# -*- coding: utf-8 -*-
"""Muestrea el kit AQUA: tamano y color central de las piezas."""
import os
from pathlib import Path
from PIL import Image

RAIZ = str(Path(__file__).resolve().parents[2] / "AQUA")
PIEZAS = [
    "body_button_blue.png", "left_button_blue.png",
    "segmented_blue_body.png", "segmented_blue_left.png", "segmented_blue_right.png",
    "segmented_withe_body.png", "segmented_withe_left.png", "segmented_withe_right.png",
    "graybar_body.png", "graybar_ left.png",
    "gray_check.png", "blue_check.png", "check_v.png",
    "gray_radio.png", "blue_radio.png", "dot.png",
    "slider_thumb.png", "arrow_down.png", "arrow up.png",
    "green_check.png", "red_check.png",
]

for nombre in PIEZAS:
    ruta = os.path.join(RAIZ, nombre)
    if not os.path.exists(ruta):
        print(f"{nombre:30s} FALTA")
        continue
    im = Image.open(ruta).convert("RGBA")
    w, h = im.size
    cx, cy = w // 2, h // 2
    centro = im.getpixel((cx, cy))
    # color medio de la mitad central (para saber si es claro u oscuro)
    total = [0, 0, 0]
    cuenta = 0
    for x in range(w // 4, 3 * w // 4, max(1, w // 40)):
        for y in range(h // 4, 3 * h // 4, max(1, h // 40)):
            r, g, b, a = im.getpixel((x, y))
            if a > 200:
                total[0] += r; total[1] += g; total[2] += b
                cuenta += 1
    media = tuple(t // max(cuenta, 1) for t in total)
    print(f"{nombre:30s} {w:4d}x{h:<4d} centro={centro} media={media}")
