# interprete de tobyui lang

Anfitrion **de Windows** para **Toby UI**: abre una ventana basica en Rust y
pinta un documento `.toby` con el **motor real** del lenguaje (el mismo que
compila el kernel rocky). No lo copia: incluye los ficheros de
`kernel/src/toby_ui/` con `#[path]`.

Es la version en el host de lo que en rocky hara la orden `toby` (ver
`kernel/src/toby_ui/host.rs`): parser -> validacion -> layout -> tema ->
renderer -> eventos -> acciones. El documento **solo describe**; lo unico que
se ejecuta son los manejadores nativos que registra este programa.

Para **escribir** documentos `.toby` sin teclearlos a mano hay un companero,
**Toby AppKit** (`appkit\`): un editor visual con
panel de botones, lienzo de posiciones libres y boton *Generar .toby*. Genera
justo lo que este interprete sabe leer.

## Uso

Lo mas comodo es el lanzador, que vive junto al motor:
`kernel\src\toby_ui\arrancar.bat`. Doble clic y ya: localiza esta carpeta, compila
en release si hace falta y abre la ventana.

```bat
arrancar.bat                    :: documentos/ejemplo.toby (por defecto)
arrancar.bat otro.toby          :: un documento concreto
arrancar.bat --png salida.png   :: sin ventana, vuelca un PNG
arrancar.bat --ancho 1024 --alto 768   :: tamano de arranque
arrancar.bat recompilar         :: recompila antes de arrancar
```

Con `cargo`, lo mismo:

```powershell
# Abre la ventana con el ejemplo que trae (composicion de botones).
cargo run

# Con otro documento.
cargo run -- ruta\a\mi.toby

# Sin ventana: pinta una vez y guarda un PNG (util para ver/verificar).
cargo run -- --png capturas\salida.png

# Tamano de arranque (por defecto 480x300, centrada).
cargo run -- ruta\a\mi.toby --ancho 1024 --alto 768
```

La ventana se abre **centrada y en primer plano**, con area de cliente de
480x300 por defecto (o la que digan `--ancho`/`--alto`).

- El raton pasa por encima y pulsa los botones (hover/pulsado/foco de verdad).
- **Tab** recorre los botones interactivos; **ESC** o el boton *Cerrar* cierran.

### Pantalla completa (F10.8)

**F11** alterna entre **pantalla completa** (la ventana maximizada, area de
cliente igual a la pantalla) y la **ventana normal**. La regla esta en el motor
(`kernel/src/toby_ui/layout.rs::medir`): el tamano declarado por el documento
es un **minimo**, y el viewport manda. Asi que al maximizar el documento se
**estira** para llenar la pantalla y **centra** sus filas; al volver a la
ventana normal recupera exactamente el 480x300 declarado.

- `WM_SIZE` (al redimensionar o al pulsar F11) vuelve a medir y a pintar con el
  tamano real del area de cliente.
- `--ancho`/`--alto` fijan el tamano de arranque, por encima del declarado.
- Sin `WS_THICKFRAME`, al estar maximizada el marco llega al borde inferior,
  como en una F11 de verdad.

Al arrancar (y en modo `--png`) vuelca por consola la composicion del
documento, con una columna por elemento: tipo, `id` y `name`.

```
composicion (16 nodos):
   0  toby-app     -          diseno       (  0,  0)  480x300
   3  button       c1         aceptar      ( 40,  40)   80x24
   7  check        c5         marcar       ( 40, 100)   74x24
  14  slider       c12        slider       (280, 220)  160x24
```

`name` es el **nombre propio del elemento**, como en HTML: lo admite *todos*
los tipos, es **unico en el documento** (el validador lo rechaza si se
repite) y no pinta nada. Sirve para identificar el elemento en este listado y
en la peticion que recibe cada accion (`Peticion::nombre`), asi un mismo
manejador puede atender varios elementos sin depender del `id` tecnico. El
AppKit lo genera solo, derivado del texto de cada control.

### Transición Genie reutilizable

Una ventana Toby UI puede declarar el efecto y duración; el documento sigue
siendo declarativo y no ejecuta código:

```xml
<window width="480" height="300" transition="genie"
        transition-duration="500">
  ...
</window>
```

El anfitrión usa `App::transition_config()` para leer esa opción, crea el
reloj con `TransitionConfig::animation(inicio, unidades_por_segundo)`, captura
una vez con `App::capturar_transicion_genie(...)` y pinta cada frame mediante
`App::pintar_transicion_genie(...)` con un lienzo RAM reutilizable, los
rectángulos de origen/destino y el progreso. Esto permite que otras
apps Toby UI animen paneles, vistas o superficies a su geometría elegida; la
misma malla y el rasterizador sin asignaciones los usan Kira y el SDK AppKit.
Si el host no solicita la transición, el render normal sigue siendo
`App::pintar(...)`.

También se admite `transition="bounce"`. `App::transition_config()` lo
expone como `Effect::Bounce`; el host puede crear el movimiento reutilizable
con `TransitionConfig::bounce(inicio, unidades_por_segundo, altura_px)` y
aplicar `Bounce::offset_y(ahora)` a la posición vertical del elemento. Dura
900 ms por defecto y admite `transition-duration` explícito.
Para mantener un salto uniforme en bucle mientras una operación siga activa,
el host puede consultar `Bounce::offset_y_repeating(ahora)` y detenerlo cuando
corresponda; el período es la duración indicada al crear `Bounce`.

## Estructura del crate

```text
interprete de tobyui lang/
├── src/
│   ├── main.rs             anfitrion Windows y CLI
│   ├── toby_ui.rs          puente al motor compartido del kernel
│   └── host/               shims Windows (Win32, fuente, PNG, entrada, FS)
├── documentos/             ejemplos y documentos `.toby`
├── capturas/               PNG generados y evidencias visuales
└── herramientas/           scripts de generación y verificación
```

El documento por defecto es `documentos/ejemplo.toby`. Las salidas de las
pruebas visuales deben guardarse en `capturas/` para no mezclar código con
artefactos.

## Que ensena `documentos/ejemplo.toby`

Sin una linea de codigo, el documento declara:

1. Las **tres piezas PNG** que forman un boton AQUA (extremo izquierdo + cuerpo
   estirable + extremo derecho), mostradas con `<image>`.
2. Esos mismos botones **ensamblados**, uno por color del kit (variant): azul,
   verde, rojo, naranja, amarillo, magenta, blanco y uno *apagado*.
3. Un boton *Cerrar* ligado a la accion `window.cerrar`.

## Herramientas

En `herramientas\` hay dos scripts de Python (solo biblioteca estandar):

- `gen_fuente.py`: **genera** `src/host/fuente.rs` desde
  `C:\Windows\Fonts\vgaoem.fon`. Ojo con un detalle del `.fon`: es un ejecutable
  NE con un recurso RT_FONT y **su bloque de glifos empieza en el caracter 1**
  (`dfFirstChar = 0x01`). Copiarlo con "indice = caracter" desplaza toda la tabla
  un caracter y el texto sale con el caracter siguiente (`Hola` se dibuja
  `Ipmb`). El fichero generado lleva tests que detectan justo eso.
- `leer_texto.py`: **reconoce el texto dibujado en un PNG** (el que saca
  `--png`) comparando cada celda de 8x12 con la tabla de glifos. Es la forma de
  verificar el texto sin ojos:

  ```powershell
  python herramientas\leer_texto.py capturas\salida.png buscar 6 6 300 28 "Toby AppKit"
  # OK  ... dibujado en (12,12), pixeles mal=0
  ```

- `comparar_bordes.py`: **comprueba que los bordes de un boton AQUA salen
  suaves**. Las pieles del kit miden 258x409 y el boton 24 px de alto, asi que
  el motor las reduce con media de area (`toby_ui/renderer.rs`); con vecino mas
  proximo la esquina salia a escalones de 255 a 0. El script mide el mayor salto
  de color entre pixeles vecinos de la esquina y sale con 0 si es suave y con 1
  si hay escalones (20 con el filtro, 337 sin el):

  ```powershell
  python herramientas\comparar_bordes.py capturas\salida.png            # OK borde suave
  python herramientas\comparar_bordes.py capturas\editor.png 360 168 840 468   # zona
  ```

## Como encaja con el kernel

Los modulos `src/host/{bios,gfx,fs,sync,mouse,input}` son **shims** del
anfitrion; en rocky viven en la raiz del crate del kernel. Los recursos del tema
(`/usr/share/tobyui/themes/default/*.png`) se leen del kit **AQUA** del
repositorio (`../AQUA/`), con el descodificador PNG propio de rocky
(`libraries/png`, sin `std`). La fuente del texto (`src/host/fuente.rs`) se extrajo de
`C:\Windows\Fonts\vgaoem.fon` (8x12, un byte por fila) y se **regenera** con
`python herramientas/gen_fuente.py` (el fichero no se edita a mano).

Usa el descodificador PNG del kernel y el SDK local `toby-appkit` para el
timeline y el rasterizador compartidos de Genie.
