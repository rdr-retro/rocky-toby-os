# Fuentes del sistema

Los archivos `.ttf` y `.otf` de esta carpeta se instalan en
`/usr/share/fonts/` al construir `rockydisk.img`. Kira carga por defecto
`sfprodisplayregular.rkf` y usa `times.rkf` como alternativa; si faltan ambas
caches, vuelve a la fuente del BIOS. El texto suavizado del shell usa
`sfprodisplaymedium.rka`, una caché de cobertura antialias 12x24 con ajustes de
kerning por pareja, filtrada a 16 px al dibujar. La fuente bitmap base sigue
siendo SF Pro Display Regular.

Kira conserva el renderer bitmap 8x16 como alternativa. Para no añadir un
parser TrueType grande al kernel, cada fuente original lleva una caché `.rkf`
para el renderer bitmap existente y una caché RKA3 `.rka` con avances
proporcionales, cobertura gris de mayor resolución y kerning OpenType para
texto suavizado. Usar la variante
Medium para el texto suavizado da algo más de peso al topbar sin cargar una
segunda caché tipográfica en memoria. Si añades o cambias un `.ttf`/`.otf`, genera
las cachés con Pillow instalado:

```powershell
python Fonts/generar_cache.py
```

Al volver a construir el disco, se instalan tanto las fuentes originales como
las caches generadas.

`CascadiaCode-VariableFont_wght.ttf` y sus caches RKF/RKA se usan en la
Terminal gráfica de Kira como fuente monoespaciada, con la cache RKA para
antialias. Se incluye también la variante cursiva y la licencia OFL de
Cascadia Code.

TextEdit usa `times-textedit.rka` como fuente predeterminada: rasterización
antialias a 20×24 píxeles y avances proporcionales. Esta caché compacta se carga
bajo demanda al abrir el editor para no reservar memoria adicional mientras no
se usa. Se genera junto con las demás cachés o de forma aislada con:

```powershell
python Fonts/generar_cache.py --textedit-times
```

El encabezado de «Acerca de» no escala esa caché pequeña: usa
`about-title.rka`, con máscaras Times rasterizadas directamente a 60 px. La
información inferior usa SF Pro Display Regular suavizada a 12×24 y se presenta
a 17 px. Para regenerar los recursos del título:

```powershell
python Fonts/generar_cache.py --acerca-times
```
