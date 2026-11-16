# -*- coding: utf-8 -*-
"""Muestreo sin ojos de `controles.toby` (F10.1).

Comprueba que cada control nuevo se PINTA y en el sitio que le toca, sin
mirar la imagen: sólo coordenadas, colores sueltos y diferencias entre dos
renderizados (marcado / sin marcar), que es la única prueba honesta de que
una marca se pinta encima de su casilla.
"""
import sys
from pathlib import Path
from PIL import Image

CAPTURAS = Path(__file__).resolve().parents[1] / "capturas"
ruta = sys.argv[1] if len(sys.argv) > 1 else str(CAPTURAS / "controles_actual.png")
ruta_sin = sys.argv[2] if len(sys.argv) > 2 else str(CAPTURAS / "controles_sinmarca.png")
im = Image.open(ruta).convert("RGB")
sin = Image.open(ruta_sin).convert("RGB")

malos = []


def px(x, y, img=None):
    return (img or im).getpixel((x, y))


def no_blanco(x, y, donde):
    r, g, b = px(x, y)
    if r > 245 and g > 245 and b > 245:
        malos.append(f"{donde}: ({x},{y}) sigue en blanco (no se pintó)")


def cambia(x0, y0, x1, y1, donde):
    """¿Diferen esas dos cajas entre el render marcado y el sin marcar?"""
    for y in range(y0, y1):
        for x in range(x0, x1):
            if px(x, y) != px(x, y, sin):
                return
    malos.append(f"{donde}: la casilla no cambia al marcarla")


def igual(x0, y0, x1, y1, donde):
    for y in range(y0, y1):
        for x in range(x0, x1):
            if px(x, y) != px(x, y, sin):
                malos.append(f"{donde}: cambia aunque no debía")
                return


AZUL = (0x0E, 0x5C, 0xB8)
ROJO = (0xD6, 0x45, 0x45)
GRIS = (0x8E, 0x96, 0x9E)

# --- check ---------------------------------------------------------------
# c1 (20,52) marcado en el doc A y sin marcar en el B: su casilla cambia.
cambia(20, 55, 38, 73, "check c1")
# c2 (170,52) sin marcar en AMBOS: no cambia nada.
igual(170, 55, 188, 73, "check c2")

# --- radio ---------------------------------------------------------------
# r1 (20,84) marcado en A, sin marcar en B: cambia su punto interior.
cambia(20, 87, 38, 105, "radio r1")

# --- circular ------------------------------------------------------------
cerca = px(36, 132)
if abs(cerca[0] - ROJO[0]) + abs(cerca[1] - ROJO[1]) + abs(cerca[2] - ROJO[2]) > 150:
    malos.append(f"circular k1: centro {cerca} no es rojo")
# k2 (64,116) apagado: gris, no su acento verde
no_blanco(80, 132, "circular k2")
gris_centro = px(80, 132)
if abs(gris_centro[0] - GRIS[0]) + abs(gris_centro[1] - GRIS[1]) + abs(gris_centro[2] - GRIS[2]) > 90:
    malos.append(f"circular k2: {gris_centro} no es gris")

# --- segmented -----------------------------------------------------------
# sg (120,116) 244x24 con «Mediano» activo: su tercio medio está pintado.
no_blanco(247, 128, "segmented activo")
# sg2 (160,266) con «No» activo: su segunda mitad.
no_blanco(200, 278, "segmented sg2 activo")

# --- popup ---------------------------------------------------------------
no_blanco(336, 128, "popup disparador magenta")
no_blanco(24, 278, "popup blanco (usa piel del kit)")

# --- field ---------------------------------------------------------------
no_blanco(240, 168, "field con texto: carril")
no_blanco(30, 168, "field con placeholder: carril")
# el texto del field f2 empieza en x=236: algo pintado ahí
no_blanco(238, 164, "field texto")

# --- slider --------------------------------------------------------------
# s1 (20,196) 200x24, valor 30 -> pulgar en x=20+199*30/100=79, y=208.
no_blanco(79, 208, "slider pulgar")
no_blanco(190, 208, "slider pista lejos del pulgar")
# s2 gota (250,196) 190x24, valor 70 -> x=250+189*70/100=382.
no_blanco(382, 200, "slider-gota pulgar de gota")
# s3 apagado (250,236) 20%: pulgar gris en x=250+189*20/100=287.
gris_pulgar = px(287, 248)
if abs(gris_pulgar[0] - GRIS[0]) + abs(gris_pulgar[1] - GRIS[1]) + abs(gris_pulgar[2] - GRIS[2]) > 120:
    malos.append(f"slider s3 apagado: pulgar {gris_pulgar} no es gris")

print(f"muestreados sobre {im.size[0]}x{im.size[1]}")
if malos:
    print("MAL:")
    for m in malos:
        print("  -", m)
    sys.exit(1)
print("OK: todos los controles nuevos se pintan donde deben")
