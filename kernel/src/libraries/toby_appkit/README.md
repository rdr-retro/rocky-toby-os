# Toby AppKit SDK

SDK común para el código Rust nativo que se guarda en `code/` dentro de cada
bundle `<nombre>.app`. Es `no_std` y usa la ABI de sistema `int 0x80` de Rocky.

```rust
#![no_std]
#![no_main]

use toby_appkit::{app_entry, println};

fn mi_app() {
    println!("Hola desde Rocky");
}

app_entry!(mi_app);
```

`app_entry!` genera `_start`, adapta la pila al ABI x86_64 y convierte el
retorno de la función en `exit(0)`. El SDK también expone `write`, `print!`,
`println!`, `exit` y `fs::{open, read, close, mkdir, unlink, rename}`. Las
llamadas del sistema devuelven el resultado crudo del ABI (los errores son
negativos). AppKit compila a `x86_64-unknown-none` con el linker script
`rocky.ld`; `xtask` usa ese mismo script para los programas incluidos en el
sistema.

La parte declarativa de la interfaz vive en `ui/*.toby` y la valida/renderiza
el motor compartido en `kernel/src/toby_ui`. Las dos capas forman Toby AppKit:
Toby UI para vistas y este SDK para los binarios Rust. No se compila código
Rust dentro del kernel.

## Cajas anidadas en Toby UI

`<box>` es un contenedor con borde y título opcional situado fuera del marco,
en una banda superior propia. Se mide con sus hijos, reserva 16 px de padding
interior por defecto y puede contener otras cajas hasta el límite de
profundidad del documento. Las cajas anidadas usan por defecto un panel gris y
el nivel interior, un gris Aqua ligeramente más oscuro:

```xml
<toby-app name="preferencias">
  <window title="Preferencias" width="480" height="320">
    <column padding="16" spacing="12">
      <box id="cuenta" title="Cuenta" spacing="8">
        <button id="perfil" title="Editar perfil"/>
        <box id="privacidad" title="Privacidad">
          <button id="permisos" title="Permisos"/>
        </box>
      </box>
    </column>
  </window>
</toby-app>
```

`padding`, `spacing`, `width`, `height` y `align` controlan el layout del
contenido; el título queda integrado en el borde superior.

## Stepper Control en Toby UI

Las vistas de AppKit pueden declarar el control como `<stepper>`:

```xml
<stepper id="zoom" items="75 %|100 %|125 %" selected="1"/>
```

El nombre identifica el control de opciones con flechas de la galería AQUA y
mantiene su comportamiento actual. La etiqueta `<popup>` sigue aceptada como
alias para que las apps existentes no dejen de funcionar.

## Progreso indeterminado en Toby UI

El indicador animado usa el recurso compartido `indeterminate.png` del tema
AQUA. No tiene valor ni porcentaje; Kira y el anfitrión de pantalla completa
repintan la fase a intervalos mientras la vista está visible:

```xml
<progress id="cargando" mode="indeterminate" width="180" height="18"/>
```

El atributo `mode` es opcional y admite `indeterminate` (el modo por defecto)
o `determinate`. El modo porcentual exige `value` entre 0 y 100 y usa los
recursos compartidos `loader_back.png` (pista) y `loader.png` (relleno). Ambos
se montan en 3-slice: conserva los extremos y estira únicamente el centro:

```xml
<progress id="instalacion" mode="determinate" value="68" width="180" height="16"/>
```

La variante porcentual usa la altura nativa de `loader.png` y `loader_back.png`
(16 px); sólo se divide horizontalmente en 3-slice para conservar los extremos.

Ambos modos solicitan fotogramas con la misma velocidad lineal: el modo
indeterminado recorre sus 32 px en 900 ms y el porcentual sus 9 px centrales en
253,125 ms. El fragmento central de `loader.png` se desplaza por todo el
relleno azul —incluido su borde izquierdo— y queda recortado al porcentaje,
sin ocultar la etiqueta. No hace falta declarar una opción de animación
adicional: Toby UI lo anima automáticamente mientras el control está visible.
La geometría y la fase se centralizan en
`toby_appkit::animation::DeterminateProgress` (`filled_width` y
`texture_offset_at`); otros hosts AppKit pueden usar la misma fase con su
propio renderer y reloj. En una app ya montada,
`App::actualizar_progreso(id, valor)` actualiza el porcentaje y devuelve `true`
si cambió para que Kira solicite un repintado.

## Animaciones compartidas

El efecto Genie está en `toby_appkit::genie`, no en el compositor de Kira.
`toby_appkit::animation::Animation` aporta timeline Q16, cuatro curvas,
reversa y fin de animación; `genie::rasterizar` convierte una superficie en
una malla de 50 bandas y llama a un callback del anfitrión por píxel. No
depende de `Canvas`, no reserva memoria por frame y sirve para cualquier
superficie o renderer propio.

Una interfaz Toby UI puede declarar una transición para su ventana:

```xml
<window title="Panel" width="480" height="300"
        transition="genie" transition-duration="500">
  ...
</window>
```

El anfitrión conserva el control del reloj y de la geometría (por ejemplo, el
rectángulo de un icono o de un panel). `App::transition_config()` lee la
declaración; `TransitionConfig::animation(inicio, unidades_por_segundo)` crea
un timeline compatible con el reloj elegido. Al empezar, se llama una vez a
`App::capturar_transicion_genie(...)` sobre un lienzo RAM reutilizable; durante
cada frame, `App::pintar_transicion_genie(...)` rasteriza esa captura hacia el
rectángulo destino. Si el documento no declara Genie, la captura/transición
devuelve `false` y se usa `App::pintar(...)` normal.

AppKit también ofrece `animation::Bounce`, un rebote vertical amortiguado de
tres saltos, independiente del framebuffer y reutilizable para iconos,
superficies y controles. Toby UI puede declararlo con
`transition="bounce"`; `App::transition_config()` devuelve `Effect::Bounce`
y `TransitionConfig::bounce(inicio, unidades_por_segundo, altura_px)` crea el
timeline. En cada frame, el host suma `Bounce::offset_y(ahora)` a la coordenada
vertical del elemento. La duración predeterminada es 900 ms y puede
sobrescribirse con `transition-duration`.

Kira usa `Bounce::offset_y_repeating(...)` para mantener un salto uniforme de
la misma altura cada 500 ms en el icono de una app cerrada hasta que termina
de cargarse; al enfocar una app abierta o restaurar una minimizada no activa
el rebote.

También se puede deformar cualquier bitmap sin una aplicación Toby UI, usando
`genie::rasterizar` directamente. El callback recibe `(x, y, source_x,
source_y, color)` para que cada host aplique su clip, mezcla alfa y backend de
presentación.
