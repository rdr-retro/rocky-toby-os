# Toby AppKit

Toby AppKit es el kit central para crear aplicaciones Rocky: editor visual de
interfaces Toby, formato `.app`, validador de paquetes y compilador de código
Rust `no_std`. El motor Toby UI que valida y dibuja los `.toby` es el mismo
código compartido con el kernel y el intérprete.

```powershell
cargo run --manifest-path appkit/Cargo.toml -- nuevo apps\reloj.app
cargo run --manifest-path appkit/Cargo.toml -- nuevo-sistema system_apps\mi_servicio.app
cargo run --manifest-path appkit/Cargo.toml -- validar apps\reloj.app
cargo run --manifest-path appkit/Cargo.toml -- compilar apps\reloj.app
```

Cada app es una carpeta con un `manifest.xml`, una o más vistas `.toby` y su
código Rust. `nuevo` crea la estructura y un ejemplo ejecutable; `compilar`
usa el SDK central `kernel/src/libraries/toby_appkit` y produce `code/app.elf`.
Al ejecutar `xtask build`, las carpetas `apps/*.app` se copian a
`/home/apps` dentro del volumen Estrato; el kernel abre el bundle con
`toby /home/apps/reloj.app`.

Los paquetes creados con `nuevo-sistema` se guardan en `system_apps/*.app` y
`xtask build` los instala en `/system/apps`. El manifiesto puede definir su
clase y política de inicio:

```xml
<inicio clase="sistema" automatico="true" prioridad="100"/>
```

Los servicios gráficos del kernel (incluido Kira) se preparan antes de lanzar
apps. Después se inician primero las apps automáticas de sistema, ordenadas
por prioridad descendente, y luego las de usuario. La prioridad admitida va de
`-1000` a `1000`; el valor predeterminado es usuario, inicio manual y prioridad
`0`.

Desde la consola del kernel, `inicio` muestra el orden y la política; por
ejemplo, `inicio activar usuario/reloj 80` habilita el inicio automático y
fija su prioridad. `gestor-inicio` abre la interfaz gráfica para activar o
desactivar apps y ajustar su prioridad. Los cambios se guardan en
`/system/config/startup.conf`.

El escritorio de Kira usa un fondo liso sin imagen de wallpaper, una barra
superior con la misma textura degradada que la cabecera de ventana y un
rectángulo de selección al arrastrar con el botón izquierdo. Las etiquetas de
la barra son visuales por ahora; todavía no despliegan menús.

Estructura creada:

```text
reloj.app/
├── manifest.xml
├── ui/
│   └── main.toby
└── code/
    ├── Cargo.toml
    ├── src/main.rs
    └── app.elf          # aparece al compilar
```

Las vistas adicionales se registran en `<ui>` con `<vista nombre="..."
archivo="ui/otra.toby"/>`. El manifiesto impide rutas que escapen del bundle.
El código nativo se compila fuera del kernel; Toby AppKit no compila Rust en
tiempo de ejecución.

## Editor visual Toby

Editor visual del lenguaje **Toby UI** para Windows.

Se abre con un **lienzo en blanco**, a la izquierda está el **panel** con los
**tipos de botón** del kit AQUA y con los **ocho controles** del lenguaje
(`check`, `radio`, `circular`, `segmented`, `popup`, `field`, `slider` y
`slider-gota`), y abajo el botón **Generar .toby**. Todo se coloca **donde se
quiera**: se **arrastra desde el panel** hasta la hoja y se suelta.

La vista de cada entrada del panel es una **miniatura pintada por el motor** con
una pieza de verdad, así que el panel enseña exactamente lo que se va a colocar.

```
arrancar.bat
```

o, desde esta carpeta:

```
cargo run --release
```

## Cómo se usa

| Paso | Qué se hace |
|------|-------------|
| 1 | Pinchar una entrada del panel de la izquierda **sin soltar**. |
| 2 | Llevarla hasta la hoja: el control aparece bajo el puntero mientras el puntero esté dentro (mientras no esté, no se coloca nada). |
| 3 | Soltar dentro de la hoja: ahí se queda, centrado donde se soltó. |
| 4 | Pinchar encima de un control y arrastrar para moverlo; se queda dentro de la hoja, no se puede sacar. |
| 5 | **Generar .toby** escribe el documento y lo imprime en la consola. |

Un clic suelto (sin arrastrar) no coloca nada: para colocar hay que arrastrar
desde el panel. La entrada que se coge queda marcada en naranja en el panel.

Con un control seleccionado (marco naranja y asas en las esquinas):

| Tecla / botón | Efecto |
|---------------|--------|
| escribir | añade texto al final del `title`, `label`, `items` o `placeholder`, según el tipo |
| dígitos | en un `slider` o `slider-gota` fijan el `value` (0..100) |
| Retroceso | borra el último carácter (o baja un dígito del `value`) |
| flechas | afina la posición un píxel |
| Supr | quita el control |
| **Accion** | cicla la acción declarada: ninguna → `demo.pulsar` → `window.cerrar` |
| **Vaciar** | deja el lienzo otra vez en blanco |

Los caracteres que romperían el documento (`< > & "`) se ignoran: el parser de
Toby UI no admite entidades ni texto suelto, así que el editor no los teclea.

**F11** (en cualquier momento, con o sin control seleccionado) alterna
pantalla completa y ventana normal; la ventana abre maximizada.

## Por qué el lienzo no puede mentir

La vista previa de la hoja **la pinta el motor real del lenguaje**
(`App` → parser → validación → layout → tema → renderer), el mismo
`kernel/src/toby_ui/` que compila rocky. Cada vez que se toca el diseño:

1. se regenera el texto `.toby` del diseño,
2. se vuelve a montar la `App` con ese texto,
3. el layout del motor da la posición de cada botón, y el editor la usa tal cual.

Así que lo que se ve en la hoja es, literalmente, lo que pintará el
intérprete; no es una maqueta que se le parezca. La autoprueba comprueba
justamente eso: que cada control del modelo cae en el mismo rectángulo que
calcula el motor al releer el `.toby` generado.

Lo mismo vale para el arrastre: mientras se trae una entrada desde el panel, el
control que sigue al puntero **ya es un control del diseño** (lo pinta el mismo
motor), no una silueta dibujada aparte; si el puntero sale de la hoja, ese
control provisional se quita. La autoprueba lo mira en el búfer de píxeles: el
mismo punto del lienzo cambia de color con el provisional puesto y sin él.

## El documento que genera

Un `<stack>` con las hojas en **posición libre** (`x`/`y`):

```xml
<toby-app name="diseno">
  <window title="Diseno" width="480" height="300" align="stretch">
    <stack width="480" height="300" grow="1" responsive="scale">
      <button id="c1" name="aceptar" variant="blue" title="Aceptar" action="demo.pulsar" x="40" y="40"/>
      <check id="c2" name="marcar" variant="blue" label="Marcar" x="40" y="100"/>
      <radio id="c3" name="si" variant="blue" label="Sí" group="g1" x="40" y="134"/>
      <segmented id="c4" name="uno-dos-tres" variant="blue" items="Uno|Dos|Tres" x="140" y="170"/>
      <field id="c5" name="texto" placeholder="Texto" x="40" y="220"/>
      <slider id="c6" name="slider" variant="blue" value="0" min="0" max="100" x="280" y="220"/>
    </stack>
  </window>
</toby-app>
```

Cada tipo escribe sus propias propiedades, con las reglas que exige el lenguaje:
`title` en el botón (opcional) y en el `circular` (obligatorio), `label` en
`check` y `radio`, `items` (opciones separadas por `|`) en `segmented` y
`popup`, `placeholder` en `field` y `value/min/max` en los sliders. El `radio`
escribe además `group="g1"`: todas las bolas del diseño van en el mismo grupo,
así que se excluyen entre sí como espera el motor. El `field` no escribe
`variant` porque el lenguaje no se la admite (sus piezas son las blancas del
kit).

### `name`: el nombre propio de cada control

Como en HTML, **cada tipo de elemento admite `name`**, y AppKit lo
escribe siempre, derivado del texto del control:

* `"Aceptar"` → `name="aceptar"`, `"Guardar todo"` → `name="guardar-todo"`;
* si el texto no aporta nada (un slider), se usa el tipo: `name="slider"`;
* si dos controles quedan con el mismo nombre, el segundo se numera:
  `boton`, `boton-2`, `boton-3`…;
* el nombre de la `<toby-app>` ocupa ese mismo espacio, así que también se
  esquiva.

El `name` es **único en el documento** (lo valida el lenguaje, igual que el
`id`) y no pinta nada: sirve para identificar el elemento en el listado que
vuelca el intérprete y en la petición que recibe cada acción
(`Peticion::nombre`), de forma que un mismo manejador puede atender varios
elementos sin depender del `id` técnico.

`x`/`y` son la forma que el lenguaje tiene ahora de decir «esto va aquí»:

* una hoja (`button`, los ocho controles, `image`) que declara `x` o `y` se
  coloca en `origen_del_contenido + (x, y)` y **queda fuera del flujo** de
  `row`/`column`;
* la que no declara ninguna sigue como siempre (colocada por el flujo);
* un contenedor que se ajusta a su contenido cuenta a las absolutas para
  medirse.

Es una propiedad más de la tabla central del lenguaje
(`kernel/src/toby_ui/component.rs`), validada como las demás, y tiene sus
pruebas en `kernel/src/toby_ui/layout.rs`.

`grow` reparte el espacio sobrante del eje principal entre los hijos de `row`
y `column` (su valor es un peso; `grow="2"` recibe el doble de espacio extra
que `grow="1"`). Si la ventana se hace más pequeña, el motor también reparte
la contracción para mantener los elementos dentro del área. En un `stack`,
`responsive="scale"` conserva las coordenadas libres del diseño y escala
uniformemente posiciones y controles al nuevo tamaño, centrando el diseño
cuando cambia la proporción de la ventana. AppKit lo incluye en los `.toby`
generados, y Kira usa el mismo layout al maximizar o restaurar.

## Modos sin ventana

Útiles para comprobar cosas sin pantalla (y para las pruebas de este repo):

```
cargo run -- --png editor.png        # vuelca la interfaz del editor a un PNG
cargo run -- --generar diseno.toby   # escribe el .toby de un diseño de ejemplo
cargo run -- --prueba                # autoprueba de la lógica (0 = todo bien)
cargo run -- --salida otro.toby      # cambia el fichero del botón Generar
```

El fichero por defecto del botón **Generar** es `salida.toby`, en esta carpeta.

## Pantalla completa (F11)

La ventana **abre maximizada**, para que la hoja se vea tan grande como
permita la tarjeta, y **F11 alterna** entre esa pantalla completa y la ventana
normal de 1000×660:

| Tecla | Efecto |
|-------|--------|
| **F11** | pantalla completa ↔ ventana normal (también el botón de la barra de título) |

Al redimensionar (por F11 o por el sistema) la geometría de todo se recalcula
sobre el tamaño real de la ventana, de modo que la barra de acciones, el panel,
el pie y la hoja siguen en su sitio y a escala. Sin `WS_THICKFRAME`, al estar
maximizada el marco llega al borde inferior, como en una F11 de verdad.

El diseño conserva un tamaño lógico (`width`/`height`, 480×300 por defecto)
para que las coordenadas sigan siendo estables mientras editas. La aplicación
generada adapta uniformemente sus controles al tamaño real de la ventana; la
vista previa del lienzo muestra la composición en ese tamaño lógico.

## El `name`, a la vista en el pie

Cada control lleva su `name` (ver más abajo), y el editor lo **enseña**: con un
control seleccionado, el pie rotula `name=…` con el nombre de ese control, en
el hueco libre de la izquierda; sin selección dice `nada seleccionado`. Es el
mismo texto que se escribe en el `.toby`, de forma que no hace falta mirar el
documento para saber cómo se llama un elemento.

La lista entera de nombres la da `Diseno::nombres()`, que es **la única fuente
de verdad**: la usan a la vez el documento generado y el rótulo del pie, así
que el nombre que se ve es exactamente el que se escribe.

## Cómo está montado

No se copia nada del motor: los ficheros de `kernel/src/toby_ui/` y los shims
del intérprete de Windows se incluyen con `#[path]`. Hay **una sola fuente de
verdad** para el lenguaje y para el anfitrión:

```
src/main.rs      el editor (modelo, pintado, ratón/teclado, ventana Win32)
src/../..        por #[path]:
                   kernel/src/toby_ui/*      el motor del lenguaje
                   interprete de tobyui lang/src/host/{win32,fuente,bios,gfx,fs,
                                                 sync,mouse,input,
                                                 png_escribir,toby_ui}.rs
```

La única dependencia externa es `png`, el descodificador del propio rocky, que
usan las pieles AQUA del tema. Los PNG del kit se leen de `../AQUA/` a través
del shim de `fs` (en el contrato del lenguaje son `/usr/share/tobyui/themes/...`).

Las pieles del kit miden 258×409 y un botón mide 24 px de alto, así que el
motor las reduce mucho. Por eso el escalado se **filtra**: media de área al
reducir e interpolación bilineal al ampliar, nunca vecino más próximo, que
dejaba la esquina redondeada a escalones de 255 a 0 («dientes de sierra»). El
muestreo está en `kernel/src/toby_ui/renderer.rs::muestra` (con cinco pruebas
guardianas; `muestra_escalada` es el envoltorio para una `Imagen`), y lo
reutiliza también el visor de imágenes del escritorio: hay una sola
implementación. Se comprueba sin mirar nada con la herramienta del intérprete:

```
python "../interprete de tobyui lang/herramientas/comparar_bordes.py" editor.png 360 168 840 468
# OK    mayor salto 20 (<= 80): borde suave, sin dientes de sierra
```

Por eso este programa sólo compila junto al resto del repositorio: los dos
crates y el kernel comparten los mismos ficheros.

## Lo que todavía no hace

* El tamaño de la hoja está fijo en 480×300 (el mismo `<window>` que usa el
  intérprete por defecto). No hay campo para cambiarlo.
* El texto se escribe al final; no hay cursor ni selección de texto.
* No inserta `<image>` ni contenedores: sólo botones y los ocho controles.
* El `checked` de `check`/`radio` y el `selected` de `segmented`/`popup` no se
  editan aquí: se escriben a mano en el `.toby` (por eso sus miniaturas del
  panel salen sin marcar, como los pinta el motor).
* No hay deshacer.
