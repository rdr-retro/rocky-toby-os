"""Genera las variantes de color de las piezas azules del kit Aqua.

Sólo se tocan las piezas AZULES (el estado "marcado/seleccionado" y el pulgar).
Las grises, blancas y negras se quedan como están: no tienen color que girar.

    pieza original                 ->  salida por color
    left_button_blue.png               left_button_<color>.png
    right_button_blue.png              right_button_<color>.png
    body_button_blue.png               body_button_<color>.png
    blue_radio.png                     <color>_radio.png
    blue_check.png                     <color>_check.png
    segmented_blue_left.png            segmented_<color>_left.png
    segmented_blue_body.png            segmented_<color>_body.png
    segmented_blue_right.png           segmented_<color>_right.png
    slider_thumb.png (gota)            slider_thumb_<color>.png

El azul no se genera: ya existe y es el original. El blanco se genera sólo para
el botón y el circular button (un botón sin color): las demás piezas grises y
blancas del kit son el original y no se tocan.

El recoloreado es un giro de TONO: se conservan el brillo, el glossy, la forma y
el canal alfa, así que cada color sale con el mismo aspecto que el azul. A los
colores cálidos se les sube un poco la saturación para que no queden aguados.

Uso:  python recolorear.py
"""

import colorsys
import os
import sys

try:
    from PIL import Image
except ImportError:
    sys.exit("Hace falta Pillow:  python -m pip install pillow")

# El kit es la carpeta donde está este script.
CARPETA = os.path.dirname(os.path.abspath(__file__))

# (original, patrón de salida; %s es el nombre del color)
PIEZAS = (
    ("left_button_blue.png", "left_button_%s.png"),
    ("right_button_blue.png", "right_button_%s.png"),
    ("body_button_blue.png", "body_button_%s.png"),
    ("blue_radio.png", "%s_radio.png"),
    ("blue_check.png", "%s_check.png"),
    ("segmented_blue_left.png", "segmented_%s_left.png"),
    ("segmented_blue_body.png", "segmented_%s_body.png"),
    ("segmented_blue_right.png", "segmented_%s_right.png"),
    ("slider_thumb.png", "slider_thumb_%s.png"),
)

# Tono (en grados) alrededor del cual están pintadas las piezas azules.
TONO_BASE = 210.0

# Color -> (tono de destino en grados, cuánto subir la saturación, piezas).
# En "piezas", None = todas las piezas azules del kit; una tupla = sólo esas.
# El azul no aparece: es el original y se usa tal cual.
COLORES = {
    "green": (130.0, 1.15, None),
    "red": (0.0, 1.25, None),
    "orange": (30.0, 1.25, None),
    "yellow": (52.0, 1.35, None),
    "magenta": (305.0, 1.15, None),
    # El blanco es sólo para el botón y el circular button: mismo dibujo y mismo
    # glossy, pero sin color (la saturación casi a cero). El resto de controles
    # no lo tienen.
    "white": (
        210.0,
        0.06,
        (
            "left_button_blue.png",
            "right_button_blue.png",
            "body_button_blue.png",
            "blue_radio.png",
        ),
    ),
}

# Para comprobar el resultado: piezas donde se mira el tono conseguido.
CONTROL = (
    ("radio", "blue_radio.png", "%s_radio.png"),
    ("check", "blue_check.png", "%s_check.png"),
    ("segmentado", "segmented_blue_body.png", "segmented_%s_body.png"),
    ("pulgar", "slider_thumb.png", "slider_thumb_%s.png"),
    ("boton", "body_button_blue.png", "body_button_%s.png"),
)


def recolorear(imagen, tono, factor):
    """Copia con el tono girado: mismo brillo, mismo alfa, otro color."""
    imagen = imagen.convert("RGBA")
    ancho, alto = imagen.size
    px = imagen.load()
    destino = tono / 360.0
    rgb_a_hsv = colorsys.rgb_to_hsv
    hsv_a_rgb = colorsys.hsv_to_rgb
    base = TONO_BASE / 360.0

    for y in range(alto):
        for x in range(ancho):
            r, g, b, a = px[x, y]
            if not a:
                continue
            h, s, v = rgb_a_hsv(r / 255.0, g / 255.0, b / 255.0)
            if s < 0.015:            # gris o blanco: no se toca
                continue
            h = (h - base + destino) % 1.0
            s = min(1.0, s * factor)
            r2, g2, b2 = hsv_a_rgb(h, s, v)
            px[x, y] = (round(r2 * 255), round(g2 * 255), round(b2 * 255), a)

    return imagen


def resumen(ruta):
    """Tono dominante y saturación media, para comprobar el resultado."""
    imagen = Image.open(ruta).convert("RGBA")
    ancho, alto = imagen.size
    px = imagen.load()
    suma_s = 0.0
    suma_todo = 0.0
    todos = 0
    tonos = {}
    n = 0
    for y in range(0, alto, 3):
        for x in range(0, ancho, 3):
            r, g, b, a = px[x, y]
            if a < 128:
                continue
            h, s, v = colorsys.rgb_to_hsv(r / 255.0, g / 255.0, b / 255.0)
            suma_todo += s
            todos += 1
            if s < 0.15:
                continue
            suma_s += s
            tonos[int(h * 360)] = tonos.get(int(h * 360), 0) + 1
            n += 1
    if not n:
        return "sin color (s media %.2f)" % (suma_todo / todos if todos else 0.0)
    dominante = max(tonos.items(), key=lambda kv: kv[1])[0]
    return "%3d\u00b0  s=%.2f" % (dominante, suma_s / n)


# Para el resumen de la consola: (etiqueta, pieza original, patrón de color).
RESUMEN = (
    ("boton", "body_button_blue.png", "body_button_%s.png"),
    ("radio", "blue_radio.png", "%s_radio.png"),
    ("check", "blue_check.png", "%s_check.png"),
    ("segmented", "segmented_blue_body.png", "segmented_%s_body.png"),
    ("pulgar", "slider_thumb.png", "slider_thumb_%s.png"),
)


def main():
    for color, (tono, factor, solo) in COLORES.items():
        for original, patron in PIEZAS:
            if solo is not None and original not in solo:
                continue
            origen = os.path.join(CARPETA, original)
            if not os.path.exists(origen):
                print("falta %s  (se salta)" % original)
                continue
            salida = recolorear(Image.open(origen), tono, factor)
            salida.save(os.path.join(CARPETA, patron % color), "PNG")
        detalles = "  ".join(
            "%s %s" % (nombre, resumen(os.path.join(CARPETA, patron % color)))
            for nombre, _, patron in RESUMEN
            if os.path.exists(os.path.join(CARPETA, patron % color))
        )
        print("%-8s tono %4.0f  x%.2f   %s" % (color, tono, factor, detalles))

    print("\nAzul (original, no se genera):")
    for nombre, original, _ in RESUMEN:
        print("   %-10s %s" % (nombre, resumen(os.path.join(CARPETA, original))))
    print("\nListo.")


if __name__ == "__main__":
    main()
