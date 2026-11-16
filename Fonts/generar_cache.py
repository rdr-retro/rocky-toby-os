"""Genera cachés RKF1 bitmap y RKA3 antialias/métricas/kerning para Kira.

Requiere Pillow: `python Fonts/generar_cache.py`. Las TTF/OTF originales se
instalan tambien en /usr/share/fonts; el cache permite rasterizarlas sin
incorporar un parser TrueType pesado al kernel. Times obtiene además una
cache RKA4 para TextEdit y máscaras RKT1 rasterizadas al tamaño final del
título de Acerca de.
"""

from pathlib import Path
import struct
import sys

from PIL import Image, ImageDraw, ImageFont


RAIZ = Path(__file__).resolve().parent
ANCHO = 8
ANCHO_SUAVE = 12
ALTO = 16
ALTO_SUAVE = 24
TAMANO = 14
TAMANO_SUAVE = 21
GLIFOS = 256
ANCHO_TEXTEDIT = 20
ALTO_TEXTEDIT = 24
TAMANO_TEXTEDIT = 18
TAMANO_ACERCA = 60
COMPOSICIONES = {
    0xC0, 0xC1, 0xC2, 0xC3, 0xC8, 0xC9, 0xCA, 0xCB,
    0xCC, 0xCD, 0xCE, 0xCF, 0xD2, 0xD3, 0xD4, 0xD5,
    0xD9, 0xDA, 0xDB, 0xDD, 0xE3, 0xF5, 0xFD,
}
CP437 = [
    (0x80, 0x00C7), (0x81, 0x00FC), (0x82, 0x00E9), (0x83, 0x00E2),
    (0x84, 0x00E4), (0x85, 0x00E0), (0x86, 0x00E5), (0x87, 0x00E7),
    (0x88, 0x00EA), (0x89, 0x00EB), (0x8A, 0x00E8), (0x8B, 0x00EF),
    (0x8C, 0x00EE), (0x8D, 0x00EC), (0x8E, 0x00C4), (0x8F, 0x00C5),
    (0x90, 0x00C9), (0x91, 0x00E6), (0x92, 0x00C6), (0x93, 0x00F4),
    (0x94, 0x00F6), (0x95, 0x00F2), (0x96, 0x00FB), (0x97, 0x00F9),
    (0x98, 0x00FF), (0x99, 0x00D6), (0x9A, 0x00DC), (0x9B, 0x00A2),
    (0x9C, 0x00A3), (0x9D, 0x00A5), (0xA0, 0x00E1), (0xA1, 0x00ED),
    (0xA2, 0x00F3), (0xA3, 0x00FA), (0xA4, 0x00F1), (0xA5, 0x00D1),
    (0xA6, 0x00AA), (0xA7, 0x00BA), (0xA8, 0x00BF), (0xAA, 0x00AC),
    (0xAB, 0x00BD), (0xAC, 0x00BC), (0xAD, 0x00A1), (0xAE, 0x00AB),
    (0xAF, 0x00BB), (0xE1, 0x00DF), (0xE6, 0x00B5), (0xF1, 0x00B1),
    (0xF6, 0x00F7), (0xF8, 0x00B0), (0xFA, 0x00B7), (0xFD, 0x00B2),
    (0xFF, 0x00A0),
]


def caracteres_por_indice():
    """Replica texto::glifo_de: ASCII, CP437 y los acentos compuestos."""
    chars = [None] * 256
    for code in range(0x20, 0x7F):
        chars[code] = chr(code)
    chars[0x7F] = chr(0x2302)  # casa de CP437
    for indice, unicode in CP437:
        if indice not in COMPOSICIONES:
            chars[indice] = chr(unicode)
    for indice in COMPOSICIONES:
        chars[indice] = chr(indice)
    return chars


def rasterizar_celda(font, caracter, ancho_celda, alto_celda=ALTO, alinear_izquierda=False):
    if caracter is None:
        return Image.new("L", (ancho_celda, alto_celda), 0)

    # Una línea de base compartida mantiene alineadas mayúsculas, minúsculas,
    # acentos y descendentes dentro de la celda antialias de mayor resolución.
    alto_lienzo = alto_celda + 2
    linea_base = round(alto_celda * 0.875)
    izquierda, _, derecha, _ = font.getbbox(caracter, anchor="ls")
    ancho_glifo = max(1, derecha - izquierda)
    x = 2 - izquierda
    imagen = Image.new("L", (32, alto_lienzo), 0)
    ImageDraw.Draw(imagen).text(
        (x, linea_base), caracter, font=font, fill=255, anchor="ls"
    )
    glifo = imagen.crop((2, 0, 2 + ancho_glifo, alto_lienzo))
    ancho = min(ancho_celda, ancho_glifo)
    glifo = glifo.resize((ancho, alto_celda), Image.Resampling.LANCZOS)

    celda = Image.new("L", (ancho_celda, alto_celda), 0)
    x_celda = 0 if alinear_izquierda else (ancho_celda - ancho) // 2
    celda.paste(glifo, (x_celda, 0))
    return celda


def rasterizar(font, caracter):
    """Cache binaria 8x16 para mantener compatibilidad con el renderer antiguo."""
    celda = rasterizar_celda(font, caracter, ANCHO)
    filas = bytearray(ALTO)
    for y in range(ALTO):
        for col in range(ANCHO):
            if celda.getpixel((col, y)) >= 72:
                filas[y] |= 0x80 >> col
    return bytes(filas)


def rasterizar_suave(font, caracter):
    """Cobertura en escala de grises de mayor resolución para texto antialias."""
    return rasterizar_celda(
        font,
        caracter,
        ANCHO_SUAVE,
        alto_celda=ALTO_SUAVE,
        alinear_izquierda=True,
    ).tobytes()


def generar_cache_textedit(ruta):
    """Crea Times antialias 20x24 sin tabla densa de kerning (123 KiB)."""
    chars = caracteres_por_indice()
    font = ImageFont.truetype(str(ruta), TAMANO_TEXTEDIT)
    ascent, descent = font.getmetrics()
    baseline = (ALTO_TEXTEDIT - ascent - descent) // 2 + ascent
    glifos = bytearray()
    avances = bytearray(GLIFOS)

    for indice, caracter in enumerate(chars):
        celda = Image.new("L", (ANCHO_TEXTEDIT + 4, ALTO_TEXTEDIT), 0)
        if caracter is not None:
            izquierda, _, _, _ = font.getbbox(caracter, anchor="ls")
            dibujo_celda = ImageDraw.Draw(celda)
            dibujo_celda.text(
                (2 - izquierda, baseline),
                caracter,
                font=font,
                fill=255,
                anchor="ls",
            )
            glifo = celda.crop((2, 0, 2 + ANCHO_TEXTEDIT, ALTO_TEXTEDIT))
            glifos.extend(glifo.tobytes())
            avances[indice] = max(1, min(ANCHO_TEXTEDIT, round(font.getlength(caracter))))
        else:
            glifos.extend(bytes(ANCHO_TEXTEDIT * ALTO_TEXTEDIT))

    salida = RAIZ / "times-textedit.rka"
    salida.write_bytes(
        b"RKA4"
        + bytes((ANCHO_TEXTEDIT, ALTO_TEXTEDIT))
        + struct.pack("<H", GLIFOS)
        + avances
        + glifos
    )
    print(f"Times TextEdit: {salida.name} ({salida.stat().st_size} bytes)")


def generar_cache_titulo_acerca(ruta):
    """Rasteriza el título al tamaño final para que el kernel no lo amplíe."""
    font = ImageFont.truetype(str(ruta), TAMANO_ACERCA)
    lineas = ["Rocky ", "&", "Toby OS"]
    bounds = [font.getbbox(texto, anchor="ls") for texto in lineas]
    top = min(caja[1] for caja in bounds)
    bottom = max(caja[3] for caja in bounds)
    alto = bottom - top
    mascaras = []
    anchos = []
    for texto in lineas:
        ancho = max(1, int(font.getlength(texto)) + 2)
        celda = Image.new("L", (ancho, alto), 0)
        ImageDraw.Draw(celda).text(
            (1, -top), texto, font=font, fill=255, anchor="ls"
        )
        anchos.append(ancho)
        mascaras.append(celda.tobytes())

    salida = RAIZ / "about-title.rka"
    cabecera = b"RKT1" + struct.pack("<4H", alto, *anchos)
    salida.write_bytes(cabecera + b"".join(mascaras))
    print(f"Times Acerca de: {salida.name} ({salida.stat().st_size} bytes)")


def tabla_avances(font, caracteres):
    """Ancho natural de cada glifo, limitado al ancho de la celda rasterizada."""
    avances = bytearray(GLIFOS)
    for indice, caracter in enumerate(caracteres):
        if caracter is not None:
            avances[indice] = max(1, min(ANCHO_SUAVE, round(font.getlength(caracter))))
    return bytes(avances)


def u16(datos, offset):
    return struct.unpack_from(">H", datos, offset)[0]


def s16(datos, offset):
    return struct.unpack_from(">h", datos, offset)[0]


def tablas_fuente(datos):
    cantidad = u16(datos, 4)
    tablas = {}
    for i in range(cantidad):
        registro = 12 + i * 16
        tag = datos[registro:registro + 4].decode("latin1")
        offset, longitud = struct.unpack_from(">II", datos, registro + 8)
        tablas[tag] = (offset, longitud)
    return tablas


def mapa_glifos(datos, caracteres, tablas):
    """Resuelve los caracteres del mapa CP437 a sus IDs OpenType."""
    if "cmap" not in tablas:
        return {}
    cmap, _ = tablas["cmap"]
    cantidad = u16(datos, cmap + 2)
    candidatos = []
    for i in range(cantidad):
        registro = cmap + 4 + i * 8
        plataforma, codificacion = struct.unpack_from(">HH", datos, registro)
        sub = cmap + struct.unpack_from(">I", datos, registro + 4)[0]
        formato = u16(datos, sub)
        if formato == 4:
            prioridad = 0 if plataforma == 3 and codificacion in (1, 10) else 1
            candidatos.append((prioridad, sub))
    if not candidatos:
        return {}

    sub = min(candidatos)[1]
    segmentos = u16(datos, sub + 6) // 2
    fin_base = sub + 14
    inicio_base = fin_base + segmentos * 2 + 2
    delta_base = inicio_base + segmentos * 2
    rango_base = delta_base + segmentos * 2
    salida = {}
    for indice, caracter in enumerate(caracteres):
        if caracter is None:
            continue
        cp = ord(caracter)
        if cp >= 0xFFFF:
            continue
        for segmento in range(segmentos):
            inicio = u16(datos, inicio_base + segmento * 2)
            fin = u16(datos, fin_base + segmento * 2)
            if not inicio <= cp <= fin:
                continue
            delta = s16(datos, delta_base + segmento * 2)
            rango_pos = rango_base + segmento * 2
            rango = u16(datos, rango_pos)
            if rango == 0:
                glifo = (cp + delta) & 0xFFFF
            else:
                glifo_pos = rango_pos + rango + (cp - inicio) * 2
                glifo = u16(datos, glifo_pos)
                if glifo:
                    glifo = (glifo + delta) & 0xFFFF
            salida[indice] = glifo
            break
    return salida


def glifos_cobiertos(datos, offset):
    formato = u16(datos, offset)
    if formato == 1:
        cantidad = u16(datos, offset + 2)
        return set(struct.unpack_from(">" + "H" * cantidad, datos, offset + 4))
    if formato == 2:
        cantidad = u16(datos, offset + 2)
        cubiertos = set()
        for i in range(cantidad):
            inicio, fin, _ = struct.unpack_from(">HHH", datos, offset + 4 + i * 6)
            cubiertos.update(range(inicio, fin + 1))
        return cubiertos
    return set()


def valor_clase(datos, offset, glifo):
    formato = u16(datos, offset)
    if formato == 1:
        inicio = u16(datos, offset + 2)
        cantidad = u16(datos, offset + 4)
        if inicio <= glifo < inicio + cantidad:
            return u16(datos, offset + 6 + (glifo - inicio) * 2)
    elif formato == 2:
        cantidad = u16(datos, offset + 2)
        for i in range(cantidad):
            inicio, fin, clase = struct.unpack_from(">HHH", datos, offset + 4 + i * 6)
            if inicio <= glifo <= fin:
                return clase
    return 0


def tamano_valor(formato):
    return 2 * sum(bool(formato & (1 << bit)) for bit in range(8))


def leer_avance(datos, offset, formato):
    avance = 0
    for bit in range(8):
        mascara = 1 << bit
        if not formato & mascara:
            continue
        valor = s16(datos, offset) if bit < 4 else u16(datos, offset)
        if mascara == 4:
            avance = valor
        offset += 2
    return avance


def sumar_par(pares, primero, segundo, avance):
    if avance:
        clave = (primero, segundo)
        pares[clave] = pares.get(clave, 0) + avance


def leer_pair_pos(datos, sub, pares, ids_glifo):
    formato = u16(datos, sub)
    cobertura = glifos_cobiertos(datos, sub + u16(datos, sub + 2))
    formato1, formato2 = struct.unpack_from(">HH", datos, sub + 4)
    if formato == 1:
        cantidad = u16(datos, sub + 8)
        for i, primero in enumerate(sorted(cobertura)[:cantidad]):
            conjunto = sub + u16(datos, sub + 10 + i * 2)
            pares_cantidad = u16(datos, conjunto)
            cursor = conjunto + 2
            for _ in range(pares_cantidad):
                segundo = u16(datos, cursor)
                cursor += 2
                avance = leer_avance(datos, cursor, formato1)
                cursor += tamano_valor(formato1) + tamano_valor(formato2)
                sumar_par(pares, primero, segundo, avance)
    elif formato == 2:
        clase1_off, clase2_off, cuenta1, cuenta2 = struct.unpack_from(">HHHH", datos, sub + 8)
        clase1_pos = sub + clase1_off
        clase2_pos = sub + clase2_off
        matriz = sub + 16
        salto = tamano_valor(formato1) + tamano_valor(formato2)
        glifos = set(ids_glifo.values())
        for primero in glifos & cobertura:
            clase1 = valor_clase(datos, clase1_pos, primero)
            if clase1 >= cuenta1:
                continue
            for segundo in glifos:
                clase2 = valor_clase(datos, clase2_pos, segundo)
                if clase2 >= cuenta2:
                    continue
                cursor = matriz + (clase1 * cuenta2 + clase2) * salto
                avance = leer_avance(datos, cursor, formato1)
                sumar_par(pares, primero, segundo, avance)


def kerning_gpos(datos, offset, pares, ids_glifo):
    lista_features = offset + u16(datos, offset + 6)
    lista_lookups = offset + u16(datos, offset + 8)
    cantidad_features = u16(datos, lista_features)
    indices = set()
    for i in range(cantidad_features):
        registro = lista_features + 2 + i * 6
        if datos[registro:registro + 4] != b"kern":
            continue
        feature = lista_features + u16(datos, registro + 4)
        cantidad = u16(datos, feature + 2)
        for j in range(cantidad):
            indices.add(u16(datos, feature + 4 + j * 2))

    cantidad_lookups = u16(datos, lista_lookups)
    for indice in indices:
        if indice >= cantidad_lookups:
            continue
        lookup = lista_lookups + u16(datos, lista_lookups + 2 + indice * 2)
        tipo = u16(datos, lookup)
        cantidad = u16(datos, lookup + 4)
        for i in range(cantidad):
            sub = lookup + u16(datos, lookup + 6 + i * 2)
            tipo_sub = tipo
            if tipo == 9 and u16(datos, sub) == 1:
                tipo_sub = u16(datos, sub + 2)
                sub += struct.unpack_from(">I", datos, sub + 4)[0]
            if tipo_sub == 2:
                leer_pair_pos(datos, sub, pares, ids_glifo)


def kerning_clasico(datos, offset, longitud, pares):
    if u16(datos, offset) != 0:
        return
    cantidad = u16(datos, offset + 2)
    sub = offset + 4
    for _ in range(cantidad):
        version, largo, cobertura = struct.unpack_from(">HHH", datos, sub)
        formato = cobertura >> 8
        if version == 0 and formato == 0 and cobertura & 1 and largo >= 14:
            cuenta = u16(datos, sub + 6)
            cursor = sub + 14
            for _ in range(cuenta):
                primero, segundo, ajuste = struct.unpack_from(">HHh", datos, cursor)
                sumar_par(pares, primero, segundo, ajuste)
                cursor += 6
        sub += largo
        if sub > offset + longitud:
            break


def tabla_kerning(ruta, caracteres, tamano=TAMANO):
    """Rasteriza las métricas kern/GPOS a la escala solicitada de la fuente."""
    datos = ruta.read_bytes()
    tablas = tablas_fuente(datos)
    ids_glifo = mapa_glifos(datos, caracteres, tablas)
    pares = {}
    if "kern" in tablas:
        offset, longitud = tablas["kern"]
        kerning_clasico(datos, offset, longitud, pares)
    if "GPOS" in tablas:
        offset, _ = tablas["GPOS"]
        kerning_gpos(datos, offset, pares, ids_glifo)

    unidades = u16(datos, tablas["head"][0] + 18) if "head" in tablas else 1000
    tabla = bytearray(GLIFOS * GLIFOS)
    for anterior in range(GLIFOS):
        primero = ids_glifo.get(anterior, 0)
        if not primero:
            continue
        for actual in range(GLIFOS):
            segundo = ids_glifo.get(actual, 0)
            if not segundo:
                continue
            ajuste = round(pares.get((primero, segundo), 0) * tamano / unidades)
            limite = max(2, round(2 * tamano / TAMANO))
            ajuste = max(-limite, min(limite, ajuste))
            tabla[anterior * GLIFOS + actual] = ajuste & 0xFF
    return bytes(tabla)


def generar_cache_suave(ruta, caracteres):
    font_suave = ImageFont.truetype(str(ruta), TAMANO_SUAVE)
    glifos_suaves = b"".join(rasterizar_suave(font_suave, c) for c in caracteres)
    avances = tabla_avances(font_suave, caracteres)
    kerning = tabla_kerning(ruta, caracteres, TAMANO_SUAVE)
    salida_suave = ruta.with_suffix(".rka")
    salida_suave.write_bytes(
        b"RKA3"
        + bytes((ANCHO_SUAVE, ALTO_SUAVE))
        + struct.pack("<H", GLIFOS)
        + avances
        + kerning
        + glifos_suaves
    )
    print(f"  antialias: {salida_suave.name} ({salida_suave.stat().st_size} bytes)")


def main():
    chars = caracteres_por_indice()
    if sys.argv[1:] == ["--textedit-times"]:
        ruta = RAIZ / "times.ttf"
        if not ruta.is_file():
            raise SystemExit(f"falta la fuente Times: {ruta}")
        generar_cache_textedit(ruta)
        return
    if len(sys.argv) > 1 and sys.argv[1] == "--acerca-times":
        ruta = RAIZ / "times.ttf"
        if not ruta.is_file():
            raise SystemExit(f"falta la fuente Times: {ruta}")
        generar_cache_titulo_acerca(ruta)
        return
    if len(sys.argv) > 1 and sys.argv[1] == "--suave-only":
        fuentes = [Path(arg) if Path(arg).is_absolute() else RAIZ / arg
                   for arg in sys.argv[2:]]
        if not fuentes or any(not p.is_file() or p.suffix.lower() not in (".ttf", ".otf")
                              for p in fuentes):
            raise SystemExit("indica una o más rutas TTF/OTF existentes")
        for ruta in fuentes:
            generar_cache_suave(ruta, chars)
        return
    if len(sys.argv) > 1:
        fuentes = sorted(
            (Path(arg) if Path(arg).is_absolute() else RAIZ / arg)
            for arg in sys.argv[1:]
        )
        if any(not p.is_file() or p.suffix.lower() not in (".ttf", ".otf") for p in fuentes):
            raise SystemExit("indica rutas existentes a fuentes TTF u OTF")
    else:
        fuentes = sorted(
            p for p in RAIZ.iterdir()
            if p.is_file() and p.suffix.lower() in (".ttf", ".otf")
        )
    if not fuentes:
        raise SystemExit("Fonts/: no se encontraron archivos TTF u OTF")

    for ruta in fuentes:
        font = ImageFont.truetype(str(ruta), TAMANO)
        glifos = b"".join(rasterizar(font, c) for c in chars)
        salida = ruta.with_suffix(".rkf")
        salida.write_bytes(b"RKF1" + bytes((ANCHO, ALTO)) + struct.pack("<H", GLIFOS) + glifos)
        print(f"{font.getname()[0]} {font.getname()[1]}: {salida.name} ({salida.stat().st_size} bytes)")

        generar_cache_suave(ruta, chars)
        if ruta.stem.casefold() == "times":
            generar_cache_textedit(ruta)
            generar_cache_titulo_acerca(ruta)


if __name__ == "__main__":
    main()
