# Botón Aqua · montado con 3 slices

Botón tipo **pastilla** ensamblado a partir de tres imágenes PNG (técnica *3-slice*).
Los extremos conservan su proporción y el cuerpo central se estira, así el botón crece
con el texto sin deformarse. Incluye además **radio** (botón de opción), **circular
button** (botón redondo: la esfera del radio sin marcar), **check** (casilla),
**segmented** (control segmentado al estilo macOS), **popup menu** (lista de
opciones desplegable), **nested box** (contenedor con título que admite cajas
anidadas), **text field** (entrada de texto) y **slider** (barra con pulgar de
esfera o gota contenido en la pista), y **progress indeterminado** (patrón en
movimiento sin porcentaje) con el mismo material *glossy*.

Abre **`index.html`**: demo mínima —sólo fondo blanco, sin adornos— con cada
componente en cuatro tamaños (18 · 26 · 36 · 48 px; el slider se muestra sólo en
el mayor), las dos variantes de pulgar del slider (esfera y gota), el **popup
menu** (que despliega las opciones), un ejemplo de **nested box** y el **text
field** (con marcador).

## Ficheros

```
index.html              · página demo (cada control en 4 tamaños + una fila con los 6 colores)
button.css              · componente reutilizable  ←  esto es el botón
radio.css               · componente de radio (botón de opción)
circular.css            · componente de botón redondo (la esfera del radio sin marcar)
check.css               · componente de check (casilla)
segmented.css           · componente de control segmentado
popup.css               · componente de popup menu (lista de opciones)
box.css                 · componente demo de cajas con título anidables
field.css               · componente de text field (entrada de texto)
slider.css              · componente de slider (pulgar centrado dentro de la pista)
progress.css            · progreso indeterminado y porcentual
indeterminate.png       · mosaico animado de progreso       32 × 16 px
loader_back.png         · pista gris del progreso porcentual 17 × 16 px
loader.png              · relleno azul del progreso         17 × 16 px
loader_center.png       · fragmento central para animarlo    9 × 16 px
left_button_blue.png    · pieza izquierda      258 × 409 px
body_button_blue.png    · pieza central (slice) 30 × 409 px
right_button_blue.png   · pieza derecha        258 × 409 px
<color>_*.png           · variantes de color de TODAS las piezas azules:
                          left/right/body_button_<color>.png,
                          <color>_radio.png, <color>_check.png,
                          segmented_<color>_left/body/right.png y
                          slider_thumb_<color>.png   (ver "Colores")
recolorear.py           · script Python que genera esas variantes
blue_radio.png          · esfera marcada       682 × 653 px
gray_radio.png          · esfera sin marcar    675 × 653 px
dot.png                 · punto central del radio   1000 × 1000 px
blue_check.png          · casilla marcada      635 × 649 px
gray_check.png          · casilla sin marcar   635 × 649 px
check_v.png             · tick del check       1000 × 1000 px
graybar_ left.png       · pista, extremo izquierdo  168 × 298 px   (el nombre lleva espacio)
graybar_body.png        · pista, franja estirable    20 × 298 px
graybar_ right.png      · pista, extremo derecho     168 × 298 px  (el nombre lleva espacio)
slider_thumb.png        · pulgar variante gota      404 × 371 px
segmented_withe_left.png    · pieza blanca, extremo o divisor  170 × 416 px
segmented_withe_body.png    · pieza blanca, franja estirable   20 × 418 px
segmented_withe_right.png   · pieza blanca, extremo derecho    170 × 418 px
segmented_blue_left.png     · pieza azul, extremo redondeado   170 × 418 px
segmented_blue_body.png     · pieza azul, franja estirable     20 × 418 px
segmented_blue_right.png    · pieza azul, extremo o divisor    170 × 417 px
arrow up.png            · flecha ▲ del popup menu   631 × 431 px   (el nombre lleva espacio)
arrow_down.png          · flecha ▼ del popup menu   631 × 431 px
```

El progreso indeterminado reutiliza `indeterminate.png` como mosaico horizontal
de 30 × 16 px. CSS lo desplaza de forma lineal cada 900 ms; no se declara
`value`, así que no representa ni calcula un porcentaje:

```html
<div class="aqua-progress" role="progressbar" aria-label="Cargando"
     aria-valuetext="En curso"></div>
```

La variante porcentual monta `loader_back.png` y `loader.png` en 3-slice:
conserva los 4 px de cada extremo y estira sólo el fragmento central. El
componente conserva los 16 px de altura nativa de los PNG. El fragmento central
repetible de 9 px se desplaza dentro del tramo completado en
253,125 ms, manteniendo la misma velocidad lineal que la pieza de 32 px de la
variante indeterminada (900 ms). El porcentaje permanece centrado y visible.
El valor visible y accesible va de 0 a 100:

```html
<div class="aqua-progress aqua-progress--determinate" role="progressbar"
     aria-valuemin="0" aria-valuemax="100" aria-valuenow="68"
     style="--aqua-progress-value:68%">
  <span class="aqua-progress__fill" aria-hidden="true">
    <span class="aqua-progress__motion"></span>
  </span>
  <span class="aqua-progress__label">68%</span>
</div>
```

## Uso

```html
<link rel="stylesheet" href="button.css">

<button class="aqua-btn" type="button">
  <span class="aqua-btn__cap aqua-btn__cap--left"></span>
  <span class="aqua-btn__body">
    <span class="aqua-btn__label">Aceptar</span>
  </span>
  <span class="aqua-btn__cap aqua-btn__cap--right"></span>
</button>
```

Tamaño (alto en px; todo lo demás se deriva):

```html
<button class="aqua-btn" style="--aqua-size: 84px">…</button>
```

Clases auxiliares: `--xs` `--sm` `--md` `--lg` `--xl`, `--block` (ancho completo),
`--light-label` (etiqueta blanca).

Variables CSS: `--aqua-size` (alto), `--aqua-h`, `--aqua-cap-w`, `--aqua-label`,
`--aqua-cap-left`, `--aqua-cap-right`, `--aqua-body` (las tres piezas del botón) y
`--aqua-shadow` (tinte de la sombra, como trío `R, G, B`).

## Colores

Clases de color (las mismas en todos los controles que llevan azul):
`--green` `--red` `--orange` `--yellow` `--blue` `--magenta`.

| componente | clase | dónde se nota |
|---|---|---|
| botón | `aqua-btn--green` | las tres piezas del botón |
| radio | `aqua-radio--green` | la esfera marcada |
| circular button | `aqua-circular--green` | el círculo entero |
| check | `aqua-check--green` | la casilla marcada (el tick sigue blanco) |
| segmented | `aqua-seg--green` | el segmento seleccionado |
| popup menu | `aqua-pop--green` | la parte derecha (las flechas siguen negras) |
| slider | `aqua-slider--green` | el pulgar, esfera o gota |

El botón y el circular button tienen además su variante **blanca** —`aqua-btn--white`
y `aqua-circular--white`—: el mismo dibujo sin color (misma forma, mismo glossy y
misma sombra, pero en blanco/gris). Los demás controles llevan las seis clases de
color y no necesitan el blanco aparte: sus piezas grises y blancas ya son el original.

```html
<button class="aqua-btn aqua-btn--green" type="button">…</button>
<span class="aqua-slider aqua-slider--magenta">…</span>
```

El azul (`--blue`) es el original. Los demás son **las mismas piezas con el tono
girado**: se conservan brillo, glossy, silueta y transparencia, así que sólo
cambia el color. Lo gris (sin marcar), lo blanco (tick, punto del radio, fondo
del popup y text field) y lo negro (flechas) no se toca.

Las piezas de color las genera `recolorear.py` (Pillow) a partir de las azules:

```
python -m pip install pillow
python recolorear.py
```

Cada pieza se recolorea girando el tono desde `TONO_BASE` (el azul de origen,
210°) hasta el tono de destino del diccionario `COLORES`, subiendo además un
poco la saturación de los colores cálidos para que se lean igual de bien que el
azul. Para añadir otro color: se añade al diccionario `COLORES` (tono en grados
y factor de saturación), se ejecuta el script y se apunta la clase en el CSS que
toque.

## Uso del radio

Es un `<input type="radio">` real (accesible y navegable con teclado) con la esfera
como decoración:

```html
<link rel="stylesheet" href="radio.css">

<label class="aqua-radio" style="--aqua-radio-size: 26px">
  <input class="aqua-radio__input" type="radio" name="grupo" value="a" checked>
  <span class="aqua-radio__dot" aria-hidden="true"></span>
  <span class="aqua-radio__text">Opción A</span>
</label>
```

- **Azul** = marcado, **gris** = sin marcar (se cambia solo con `:checked`).
- Tamaño con `--aqua-radio-size` (alto de la esfera); el ancho se deriva de la
  proporción nativa 682 : 653.
- Clase `--pip` añade un **punto central negro** al marcar (look de radio clásico,
  útil cuando el color no basta como única señal; color con `--aqua-radio-pip`).
  En la demo va **activado por defecto**.
- Estados: `:hover`, `:active`, `:focus-visible` (anillo de foco) y `:disabled`
  (atenuado + `not-allowed`), todo sin imágenes extra.
- Variables: `--aqua-radio-size`, `--aqua-radio-w`, `--aqua-radio-pip`, `--aqua-radio-text`.

## Uso del circular button

Botón **redondo** cuya cara es la **misma esfera del radio, pero sin selección**: la
esfera gris de un radio sin marcar. No lleva ni punto ni tick (que es lo único que
distingue a un radio marcado): es un botón, no una opción.

```html
<link rel="stylesheet" href="circular.css">

<button class="aqua-circular" type="button" aria-label="Cerrar">
  <span class="aqua-circular__face" aria-hidden="true"></span>
</button>
```

- **Piezas reutilizadas, ninguna nueva:** `gray_radio.png` (la esfera gris, la del
  radio sin marcar) y, en las variantes de color, las esferas `<color>_radio.png`
  —las mismas que usa el radio al marcarse— **sin** el `dot.png` que le pone
  `radio.css`.
- Clase **`aqua-circular--white`**: la esfera sin color (misma forma, mismo glossy y
  misma sombra, pero en blanco/gris); el PNG (`white_radio.png`) lo genera
  `recolorear.py` con la saturación casi a cero, igual que el botón blanco. Es la
  única variante que se genera aparte: las demás salen de girar el tono de la azul.
- Es un **`<button>` real** (foco, teclado y accesibilidad de serie) y **sin
  JavaScript**; la esfera la pinta la cara (`.aqua-circular__face`), que va debajo
  del texto.
- Diámetro con `--aqua-circular-size`; el ancho se deriva de la proporción nativa
  682 : 653, igual que en el radio.
- **Texto opcional:** `.aqua-circular__label` dentro del botón, centrado en el
  círculo (color con `--aqua-circular-ink`).
- Estados: `:hover` (brillo), `:active` (la esfera se encoge un 7 %), `:focus-visible`
  (anillo de foco redondo) y `:disabled` (atenuado + `not-allowed`), todo sin imágenes
  extra.
- Variables: `--aqua-circular-size`, `--aqua-circular-face`, `--aqua-circular-shadow`,
  `--aqua-circular-ink`.

## Uso del check

La pieza es un **cuadrado redondeado** (no una esfera) y al marcarse muestra un
**tick negro** encima, que sobresale un poco por los bordes:

```html
<link rel="stylesheet" href="check.css">

<label class="aqua-check" style="--aqua-check-size: 26px">
  <input class="aqua-check__input" type="checkbox" checked>
  <span class="aqua-check__box" aria-hidden="true"></span>
  <span class="aqua-check__text">Acepto</span>
</label>
```

- **Gris** = sin marcar (`gray_check.png`), **azul + tick** = marcada (`blue_check.png`).
- El tick es su **propio PNG** (`check_v.png`, dibujo negro sobre lienzo de
  1000 × 1000) puesto encima con `background-image`, sin máscaras ni SVG: se
  recentra con `background-position` y se escala con la casilla. Va tamaño
  1,15 × el alto de la casilla y su caja (`inset: -22%`) crece fuera de ella,
  así que las puntas **sobresalen** un poco, como en el Aqua clásico.
- Tamaño con `--aqua-check-size`; ancho derivado de la proporción 635 : 649.
- Mismos estados y mismos recursos que el radio.
- Variables: `--aqua-check-size`, `--aqua-check-w`, `--aqua-check-text`.

## Uso del segmented

Control segmentado de macOS: una pista blanca con extremos redondeados y un segmento
activo en azul. Cada segmento es un `<input type="radio">` real, así que se navega con
las flechas del teclado dentro del grupo:

```html
<link rel="stylesheet" href="segmented.css">

<div class="aqua-seg" role="radiogroup" aria-label="Rango">
  <label class="aqua-seg__item">
    <input class="aqua-seg__input" type="radio" name="rango" value="dia" checked>
    <span class="aqua-seg__face">Día</span>
  </label>
  <label class="aqua-seg__item">
    <input class="aqua-seg__input" type="radio" name="rango" value="semana">
    <span class="aqua-seg__face">Semana</span>
  </label>
  <label class="aqua-seg__item">
    <input class="aqua-seg__input" type="radio" name="rango" value="mes">
    <span class="aqua-seg__face">Mes</span>
  </label>
</div>
```

- **Cada segmento se monta con 3 piezas**: `[izquierda] + [cuerpo estirable] + [derecha]`,
  y las piezas se **voltean** (`scaleX(-1)`) según dónde caiga el segmento, porque el
  divisor ya viene pintado en el borde de cada pieza — no hay ninguna línea dibujada con
  CSS:

  | Segmento | Izquierda | Cuerpo | Derecha |
  |---|---|---|---|
  | blanco, primero | `withe_right` **volteada** (redondea el control) | `withe_body` | `withe_left` **volteada** |
  | blanco, en medio | `withe_left` | `withe_body` | `withe_left` **volteada** |
  | blanco, último | `withe_left` | `withe_body` | `withe_right` (redondea el control) |
  | azul, primero | `blue_left` (redondea el control) | `blue_body` | `blue_right` |
  | azul, en medio | `blue_right` **volteada** | `blue_body` | `blue_right` |
  | azul, último | `blue_right` **volteada** | `blue_body` | `blue_left` **volteada** (redondea el control) |

  Los extremos redondeados del control son `blue_left` y `withe_right`: se usan
  (volteadas si hace falta) en cualquier segmento pegado a un borde exterior, sea
  del color que sea. Los bordes de en medio son los divisores rectos `withe_left`
  (filo a la izquierda) y `blue_right` (filo a la derecha), volteados para mirar
  hacia el lado que toca — así ninguna línea recta cae nunca en el borde del
  control ni sobra ninguna pieza.
- El contenedor **no lleva fondo ni recorte** (sin `border-radius`, sin `overflow`,
  sin `background`): cada segmento se pinta con sus propias piezas y su silueta es
  la del control. Con fondo estirado, los huecos transparentes de los extremos
  redondeados se rellenaban de pista y aparecían esquinas cuadradas con su filo,
  que no corresponden.
- La altura se fija con `--aqua-seg-h` (32 px por defecto); el ancho de las piezas se
  deriva de la proporción nativa (170 / 418).
- Los segmentos se dimensionan según su contenido, como en macOS. Clase `--block` para
  que ocupe todo el ancho y los segmentos se repartan el espacio.
- Clases de tamaño: `--sm` (24 px) y `--lg` (40 px).
- El grupo se atenúa entero si algún input está `:disabled`.
- Estados: `:hover`, `:focus-visible` (anillo de foco) y `:disabled`.
- Variables: `--aqua-seg-h`, `--aqua-seg-cap`, `--aqua-seg-ink`
  (texto sin seleccionar), `--aqua-seg-on` (texto seleccionado).

## Uso del popup menu

Popup menu **con la piel del segmented** —a la izquierda el texto de la opción elegida y
a la derecha un botón **azul** con las flechas **arriba / abajo**— que **despliega la
lista de opciones** al pulsarlo. Es un **`<select>` real**: el navegador abre la lista, la
cierra al elegir, la navega con el teclado y resuelve la accesibilidad, así que **no hace
falta JavaScript**. Las flechas son **sólo el indicador** de que hay una lista (como el
popup menu de macOS): **no son botones**.

```html
<link rel="stylesheet" href="popup.css">

<span class="aqua-pop" style="--aqua-pop-h: 48px">
  <select class="aqua-pop__input" aria-label="Zoom">
    <option>Ajustar a ventana</option>
    <option selected>100 %</option>
    <option>125 %</option>
    <option>150 %</option>
  </select>
  <span class="aqua-pop__skin" aria-hidden="true">
    <span class="aqua-pop__seg aqua-pop__seg--text"></span>
    <span class="aqua-pop__seg aqua-pop__seg--step">
      <span class="aqua-pop__pair">
        <span class="aqua-pop__arrow aqua-pop__arrow--up"></span>
        <span class="aqua-pop__arrow aqua-pop__arrow--down"></span>
      </span>
    </span>
  </span>
</span>
```

- **El que recibe los clics es el `<select>`**: va encima y transparente, y el valor lo
  pinta él mismo. La **piel** (`.aqua-pop__skin`, debajo) lleva `pointer-events: none`, así
  que pulsar **cualquier punto del control** —la parte blanca, sobre todo— abre la lista.
- **Sin apariencia nativa**: `appearance: none` en el `<select>` (y `::-ms-expand` para
  navegadores antiguos); el texto se **centra en la parte blanca** con `text-align: center`
  y un `padding` derecho igual al botón azul más un extremo (los extremos de las piezas
  miden 170 / 418 del alto).
- **Piezas y volteos**: las mismas del segmented, «blanco primero + azul último».

  | Parte | Izquierda | Cuerpo | Derecha |
  |---|---|---|---|
  | blanca (texto) | `withe_right` **volteada** | `withe_body` | `withe_left` **volteada** |
  | azul (flechas) | `blue_right` **volteada** | `blue_body` | `blue_left` **volteada** |

- Las flechas (`arrow up.png`, `arrow_down.png`) son triángulos **negros y planos** y se
  usan **tal cual**, sin filtros: así quedan negras sobre el azul (medido: su único color
  opaco es `0,0,0`).
- **Pequeñas y centradas:** ocupan el **22 %** del alto del control y la pareja va
  **apretada y centrada entera** sobre la raya del medio, con márgenes iguales arriba y
  abajo y el eje en el centro exacto del botón azul (los dos PNG son isósceles: su
  vértice cae en el centro de su lienzo).
- La rayita que separa las dos flechas es lo único dibujado con CSS (no viene en ninguna
  pieza): se puede quitar borrando el `::after` de `.aqua-pop__pair`.
- Estados: `:hover` (el texto se oscurece), `:focus-visible` (anillo de foco) y `:disabled`
  (atenuado).
- Variables: `--aqua-pop-h` (alto del control), `--aqua-pop-cap` (extremo, 170 / 418 del
  alto), `--aqua-pop-w` (ancho del botón azul, 1,3 × el alto), `--aqua-pop-arrow-h` (alto de
  la flecha, 22 %; el ancho sale de la proporción 631 : 431) y `--aqua-pop-ink`.

## Uso del text field

Entrada de texto **sin JavaScript**: es un `<input type="text">` real —el teclado, la
selección, el pegado, el autocompletar y la accesibilidad vienen de serie— al que se le
quita la apariencia nativa y se le pone la piel con las **piezas blancas del segmented
pero SIN redondear**:

```html
<link rel="stylesheet" href="field.css">

<span class="aqua-field" style="--aqua-field-h: 26px">
  <input class="aqua-field__input" type="text" placeholder="Escribe aquí">
</span>
```

| Parte | Pieza | Volteo |
|---|---|---|
| extremo izquierdo | `segmented_withe_left.png` | — |
| cuerpo | `segmented_withe_body.png` | se estira |
| extremo derecho | `segmented_withe_left.png` | **el mismo PNG volteado** (`scaleX(-1)`) |

- **Sin redondear:** el extremo recto del juego blanco es `withe_left` (esquinas llenas),
  y a la derecha va ese **mismo** PNG volteado, así que el campo es un rectángulo. Las
  piezas traen su filo oscuro (`#11284F` en `x = 0–2`), que al escalarlas al alto del
  control queda como una **línea fina de borde** en los dos extremos.
- **El cuerpo se pinta en el propio `<input>`** (fondo estirado y centrado, libre de los
  extremos con `background-size: calc(100% - cap × 2)`) y los **extremos en una capa
  encima** (los pseudoelementos del contenedor, con `pointer-events: none`): el `<input>`
  recibe **todos** los clics, así que se escribe desde cualquier punto, **también desde
  la parte blanca de los extremos**.
- El marcado son **dos elementos**: el contenedor y el `<input>`.
- `--aqua-field-h` es el alto, `--aqua-field-w` el ancho (240 px por defecto) y
  `--aqua-field-cap` el extremo (170 / 418 del alto ≈ 0,41 × el alto).
- Estados: `:focus-visible` (anillo por **fuera**, para que los extremos no lo tapen) y
  `:disabled` (atenuado). El marcador lleva un gris más claro (`::placeholder`).
- Variables: `--aqua-field-h`, `--aqua-field-w`, `--aqua-field-cap`, `--aqua-field-ink`.

## Uso del slider

Slider **sin JavaScript**: es un `<input type="range">` real —el navegador mueve el
pulgar y resuelve el teclado y la accesibilidad— con la piel puesta por `slider.css`:

```html
<link rel="stylesheet" href="slider.css">

<span class="aqua-slider" style="--aqua-slider-h: 26px">
  <input class="aqua-slider__input" type="range" value="30">
</span>
```

- **Pista** montada con 3 piezas, como el segmented:
  `[graybar_ left] + [graybar_body estirable] + [graybar_ right]` (los nombres de
  los extremos llevan un espacio tras el guion).
- **El pulgar queda contenido en la pista**: barra y pulgar tienen el mismo alto
  (55 % del control) y comparten el centro vertical. La esfera o la gota no
  sobresalen por arriba ni por abajo.
- `--aqua-slider-h` es el **alto total del control**. La pista y el pulgar se
  derivan con `--aqua-slider-track` y `--aqua-slider-thumb`: ambos miden 55 % del
  alto; el ancho sale de la proporción nativa de cada PNG. `--aqua-slider-w`
  fija el ancho (240 px por defecto).
- **Dos variantes de pulgar**, misma geometría, sólo cambia la pieza:

  | Variante | Pieza | Clase | Proporción |
  |---|---|---|---|
  | esfera (por defecto) | `blue_radio.png` | — | 682 : 653 |
  | gota | `slider_thumb.png` | `.aqua-slider--drop` | 404 : 371 |

- El `input` conserva la altura total del control y el navegador centra la pista
  dentro de ese espacio; el pulgar usa ese mismo centro, sin salirse del carril.
- Estados: `:hover` (brillo), `:active` (pulgar a escala 0,94), `:focus-visible`
  (anillo de foco alrededor del control) y `:disabled` (desaturado + atenuado).
- Firefox lleva sus `::-moz-range-*` (con el pulgar centrado dentro de la
  pista igual que en WebKit); la demo va dirigida a Blink.
- Variables: `--aqua-slider-h`, `--aqua-slider-w`, `--aqua-slider-track`,
  `--aqua-slider-cap`, `--aqua-slider-thumb` (tamaño del pulgar), `--aqua-slider-thumb-h`,
  `--aqua-slider-thumb-w`.

## Datos medidos de los assets

| Pieza | Nativo | Contenido |
|---|---|---|
| `left_button_blue.png` | 258 × 409 | extremo izquierdo redondeado + tramo plano (53,5 px) |
| `body_button_blue.png` | 30 × 409 | franja estirable (degradado vertical) |
| `right_button_blue.png` | 258 × 409 | extremo derecho redondeado (espejo) |

- **Forma:** pastilla con radio = alto / 2 (204,5 px en el nativo). Ancho nativo total: 546 × 409.
- **Costuras:** el salto de color en las uniones es de 0–10 sobre 765 → imperceptible.
- **Borde exterior:** filo claro casi blanco (~`#E2E9F5`), de 1–3 px.
- **Degradado vertical** (de arriba a abajo, sobre 409 px):

| % | Color | |
|---|---|---|
| 0 % | `#9CABD0` | filo superior |
| 5 % | `#D6E4F4` | brillo máximo |
| 16 % | `#93B2DC` | |
| **32 %** | **`#538BC6`** | **zona más oscura** |
| 47 % | `#7FADDB` | |
| 63 % | `#A3CCF2` | |
| **89 %** | **`#C4EFFD`** | **reflejo *aqua* máximo** |
| 100 % | `#AFD5F7` | filo inferior |

## Assets de radio y check

Cuatro PNG del mismo material *aqua* que la pastilla: dos **esferas** (radio) y dos
**cuadrados redondeados** (casilla). Ya están montados en la demo con `radio.css` y
`check.css`.

| Fichero | Tamaño | Forma | Uso |
|---|---|---|---|
| `blue_radio.png` | 682 × 653 | esfera (elipse) | radio marcado |
| `gray_radio.png` | 675 × 653 | esfera (elipse) | radio sin marcar |
| `blue_check.png` | 635 × 649 | cuadrado redondeado | casilla marcada |
| `gray_check.png` | 635 × 649 | cuadrado redondeado | casilla sin marcar |

- **Esferas de radio:** elipse inscrita en el lienzo (relación ≈ 1,04 → prácticamente un
  círculo). ~21 % de los píxeles son transparentes (todo lo que queda fuera de la elipse),
  con alfa limpio y borde de 1–2 px: funciona bien con `drop-shadow`.
- **Casillas de check:** **cuadrado redondeado** que llena el lienzo; radio de esquina
  ≈ 103 px (≈ 16 % del ancho), fila superior opaca 429 / 635 = 68 %. Solo las esquinas son
  transparentes (~2,8 % del total), así que también va bien con `drop-shadow`. **No traen
  tick**: el tick lo pone encima `check.css` con `check_v.png`.
- **Estructura común** (la de la pastilla, enrollada): filo fino oscuro →
  **brillo especular ≈ 5 %** → banda oscura **≈ 29 %** → **reflejo *aqua* ≈ 84 %** →
  filo inferior oscuro. Las esferas tienen además un realce claro pegado al contorno
  lateral (rim light).
- **Sin marca interior:** el perfil radial es continuo en los cuatro; el estado
  “marcado” se distingue por el color (radio) o por color + tick (check).
- **Escala:** el diámetro nativo de la esfera (653) es ≈ 1,6 × el alto del botón (409).

Degradado medido en la columna central (arriba → abajo):

| % | `blue_radio` | `gray_radio` | `blue_check` | `gray_check` |
|---|---|---|---|---|
| 0 % | `#111219` | `#131413` | `#364373` | `#413F40` |
| **5–10 %** | **`#D9E2F1`** | **`#DFDEDD`** | **`#9EB8E1`** | **`#AFAFB0`** |
| 12–20 % | `#A7BEE1` | `#BABABA` | `#5689CB` | `#807F80` |
| 21–30 % | `#6391CB` | `#8C8C8C` | `#3675BE` | `#626263` |
| 47–50 % | `#6DA0D4` | `#969696` | `#6CA0D5` | `#929191` |
| 61–60 % | `#8DBBE8` | `#BBBCBB` | `#82B2E5` | `#A9A8A8` |
| 70–80 % | `#9ECAF6` | `#C0C0C0` | `#96C3F2` | `#C0C0C0` |
| **84–90 %** | **`#ACD9FB`** | **`#E4E3E2`** | **`#B2DCFC`** | **`#E1E0E1`** |
| 100 % | `#4F738F` | `#717271` | `#314F60` | `#3D3D3C` |

Color base dominante: azul **`#548FCD`** (saturación ≈ 0,59) · gris **`#ACABAB`**
(neutro, saturación ≈ 0,01).

## Assets del segmented

Seis PNG del segmented: los tres de la pista blanca y los tres del azul del segmento
activo (más los dos de las flechas, al final). Los extremos son cuadros altos (degradado
vertical, con las esquinas cortadas donde hay redondeo) y los cuerpos son franjas
estirables.

| Fichero | Tamaño | Forma | Uso |
|---|---|---|---|
| `segmented_withe_left.png` | 170 × 416 | filo oscuro `#11284F` en `x = 0–2` + bisele claro | extremo izquierdo / divisor · extremo del text field |
| `segmented_withe_body.png` | 20 × 418 | franja estirable | centro blanco (segmented y text field) |
| `segmented_withe_right.png` | 170 × 418 | esquinas **derechas** redondeadas + hueco `#444449` | extremo derecho del control |
| `segmented_blue_left.png` | 170 × 418 | esquinas **izquierdas** redondeadas | extremo izquierdo del control |
| `segmented_blue_body.png` | 20 × 418 | franja estirable | centro azul |
| `segmented_blue_right.png` | 170 × 417 | filo oscuro `#192C54` en `x = 152–169` | extremo derecho / divisor |
| `arrow up.png` | 631 × 431 | triángulo macizo negro (apunta arriba) | flecha del popup menu |
| `arrow_down.png` | 631 × 431 | triángulo macizo negro (apunta abajo) | flecha del popup menu |

- **Anchos de extremo:** 170 px nativos ≈ 0,41 × el alto → a `--aqua-seg-h: 32px` son
  ≈ 13 px. Se derivan con `--aqua-seg-cap`.
- **Radio de esquina** nativo ≈ 128–148 px sobre 418 de alto (≈ 31 % del alto).
- **Volteo (`scaleX(-1)`):** `withe_left` y `blue_right` llevan el divisor pintado en un
  solo lado, así que los segmentos que no están pegados a un extremo del control los
  voltean para que el divisor caiga en el límite de cada segmento. `blue_left` y
  `withe_right` son los extremos redondeados y no se voltean.
- **Anatomía blanca:** desde arriba, filo claro `#FBFAFA` → gris `#DADADC` en el centro →
  `#96959A` abajo.
- **Anatomía azul:** `#4D87F7` arriba → azul vivo `#3782ED` → `#125BB6` abajo.
- El contenedor **no** lleva `border-radius` ni `overflow: hidden`: recortarlo se
  comía las esquinas de las piezas. El redondeo de los extremos lo pone la propia
  imagen (`blue_left` a la izquierda y `withe_right` a la derecha).
- **Flechas del popup menu:** llenan su lienzo (la base del triángulo es todo el
  ancho) y son **negro plano**, sin brillo; se usan sin filtro, tal cual.
- **Text field:** reutiliza `withe_left` —el extremo **recto**, sin redondear— en los dos
  lados (el derecho volteado) con el mismo cuerpo blanco; ver «Uso del text field».

## Assets del slider

Cuatro PNG: los tres de la pista y el pulgar de la variante gota (la otra variante
usa `blue_radio.png`, ya documentado arriba).

| Fichero | Tamaño | Forma | Uso |
|---|---|---|---|
| `graybar_ left.png` | 168 × 298 | extremo izquierdo de píldora | tope izquierdo de la pista |
| `graybar_body.png` | 20 × 298 | franja estirable | centro de la pista |
| `graybar_ right.png` | 168 × 298 | extremo derecho (espejo) | tope derecho de la pista |
| `slider_thumb.png` | 404 × 371 | cuerpo arriba + punta centrada abajo | pulgar gota |

- **Pista:** píldora gris de la que sólo se estira la franja central; cada casquillo
  conserva su escala con `--aqua-slider-cap` = alto de la pista × 168 / 298 (los dos
  extremos se comprimen juntos hasta `max(0px, …)`, así que nunca pisan al cuerpo).
- **Costura casquillo ↔ cuerpo:** salto de color Δ2–7 sobre 765 en casi toda la
  unión, salvo una franja arriba (y ≈ 13–20) con Δ≤64 → 1 px o menos a los tamaños
  de la demo (aceptado; la alternativa sería recortar el cuerpo del propio casquillo).
- **Pulgar gota:** la punta es su parte baja y queda justo sobre el borde de la
  pista — con el alto de control completo, la esfera y la gota tocan la pista en el
  mismo sitio.

## Notas

- **Contraste:** la etiqueta por defecto es azul marino `#0a2e52` (≈5,2:1 sobre el centro
  del botón). La variante `--light-label` (blanco) queda por debajo de AA; se ofrece por
  fidelidad al estilo *glossy* clásico.
- **Segmented:** el texto del segmento activo es blanco (fiel a macOS) sobre azul
  `#3782ED`, ≈3,8:1 — válido para texto grande pero justo por debajo de AA para texto
  pequeño. Si necesitas AA, baja el azul o usa `--aqua-seg-on: #0b0d12` (≈5,6:1).
- **Popup menu:** las flechas negras sobre el azul `#3782ED` dan ≈5,6:1 (AA), así que
  se leen bien. La **lista de opciones la dibuja el navegador**: vestirla exigiría
  `appearance: base-select` + `::picker(select)`, todavía sólo en Blink.
- Los estados (`:hover`, `:active`, `:disabled`, `:focus-visible`) se aplican con
  `filter` y `transform`, sin necesidad de imágenes adicionales.
- Para botones que necesiten conservar el brillo al 100 %, usa una imagen de cuerpo más
  ancha; con 30 px el estirado es suave porque el degradado horizontal es mínimo.
