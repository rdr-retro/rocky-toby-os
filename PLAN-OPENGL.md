# Plan: aceleracion grafica completa en rocky

Meta: que la RX 550 Lexa haga el trabajo de rasterizacion y composicion de
Kira y de las aplicaciones graficas, en vez de que el kernel recorra los
pixeles con la CPU. La CPU seguira gestionando entrada, ventanas y envio de
trabajo; por tanto, «100 %» significa que las operaciones graficas soportadas
se ejecutan en la GPU, no que el sistema deje de usar la CPU. Se conserva un
camino por CPU/VBE como respaldo para no perder salida de video si el driver
no reconoce el hardware.

Como todo lo demas del proyecto, cada fase se cierra con su verificador
`verif-*.ps1`, su seccion en el `README.md` y su snapshot.

---

## 0. Decisiones ya tomadas

| # | decision | motivo |
|---|---|---|
| D1 | **Todo en Rust; sin toolchain de C; sin Mesa.** | el "softpipe" es nuestro: rasterizador y API OpenGL escritos en Rust. El arbol queda 100% Rust, sin clang, sin C dentro del kernel. |
| D2 | **Objetivo GL 2.1 + GLSL 1.20 en subconjunto.** | lo justo para que las apps hablen OpenGL de verdad. No se promete GLSL completo (ver riesgos). |
| D3 | **Aceleracion antes que escritorio.** | primero se valida el motor de la GPU; despues se migra Kira y las apps. |
| D4 | **La API de apps puede vivir en anillo 3.** | el envio seguro a SDMA/CP y la administracion de GPU requieren soporte privilegiado del kernel; el kernel necesariamente crece. |
| D5 | **Los pixeles van por `mmap` compartido**, no por `write()`. | 3 MB por frame no caben por mensajes de 128 bytes. |
| D6 | **Un contexto y un cliente cada vez** al principio. | no hay hilos en anillo 3; preemption es una fase posterior. |

---

## 1. Punto de partida verificado en el arbol actual

| pieza | estado | donde |
|---|---|---|
| GPU AMD | RX 550 Lexa identificada por PCI; acceso parcial a registros, cursor hardware y un camino ATOM de modeset. En Polaris se habilitan PCI Memory Space/Bus Master solo con BARs dentro del mapa actual. **No hay microcodigo cargado, colas SDMA/CP, envio de comandos ni shaders GPU** | `kernel/src/drivers/gpu/{amd,atom}.rs`, `kernel/src/pci.rs`, `kernel/src/gfx.rs` |
| Escritura de pantalla | `Canvas` rasteriza en CPU. Rellenos en RAM usan `REP STOSD`; copias al LFB usan pares `MOVNTI` cuando PAT WC se verifico. Es una ruta de memoria CPU optimizada, **no** SDMA ni rasterizacion GPU | `kernel/src/gfx.rs`, `kernel/src/paging.rs` |
| Kira | compositor y ventanas dibujados por CPU sobre framebuffer; el cursor hardware AMD es una excepcion | `kernel/src/kira/`, `kernel/src/drivers/gpu/amd.rs` |
| Tamano y carga | `build/kernel.bin` verificado: 475408 B; limite de `xtask`: 524288 B (47.7 KiB libres). Ambos cargadores copian desde la ventana baja `0x10000..0x90000` a `0x100000`; el linker limita kernel+BSS a 3 MiB y reserva pila en `[3,4) MiB` | `xtask/src/main.rs`, `boot/src/boot.s`, `boot/src/bin/bootusb2.s`, `kernel/kernel.ld`, `kernel/src/start.s`, `kernel/src/pmm.rs` |

Lo que falta es el camino completo de comandos y memoria GPU, aceleracion de
composicion 2D, aceleracion GFX/3D y la conexion del compositor a esos motores.
El layout ampliado ya deja margen para continuar, pero no implica que la GPU
este inicializada: Lexa necesita su secuencia de firmware, memoria DMA y colas.

**QEMU:** `xtask run-whpx` usa WHPX para que las instrucciones del invitado se
ejecuten con aceleracion de virtualizacion de Windows; no acelera el dibujo ni
entrega la RX 550 al invitado. El QEMU instalado ofrece `virtio-vga-gl`, pero
rocky aun no implementa el driver VirtIO-GPU ni su protocolo de comandos, por
lo que seleccionarlo ahora no daria aceleracion a Kira. La via de GPU virtual
requiere primero un driver VirtIO PCI/virtqueue y despues un backend GL/virgl
validado en el anfitrion; es independiente del driver nativo Polaris de la RX
550 y no sustituye sus pruebas en hardware real.

**Avance de esta build:** ambos cargadores BIOS ya reubican el kernel a 1 MiB y
se probaron en QEMU. El `Canvas` tambien usa `REP STOSD`/`MOVNTI` cuando es
seguro, pero Kira todavia compone en CPU; no se debe anunciar esto como GPU.

---

## 2. Fases

### Fase 15 — Motor de render en Rust (el "softpipe" nuestro, sobre CPU)

El corazon: todo lo demas se apoya aqui. Va como **libreria** (crate `rogl-core`,
`no_std`) que enlaza el programa de anillo 3; no toca el kernel.

**Trabajo**
- Rasterizador de triangulos por **funciones de borde** (edge functions), con
  viewport, scissor y correccion de area.
- **Z-buffer** (16 o 24 bits) y interpolacion lineal de varyings (profundidad,
  UV, color) sobre el area cubierta.
- **Texturas** RGBA con muestreo NEAREST y LINEAR, clamping y repeating.
- **Blending** (alfa source-over) y `glClear`.
- Formato de salida `Vec<u32>` (0x00RRGGBB), igual que el `Canvas` de `gfx.rs`,
  para poder volcarlo al framebuffer tal cual.

**Verificacion**: `verif-rogl-triangulo.ps1` — un binario de anillo 3
rasteriza dos triangulos con solape y z-buffer; el screendump comprueba los
pixeles (reutilizando el analizador PPM de `verif-fondo.ps1`).

**Riesgo**: medio. **Tamano**: grande (es el nucleo que reutiliza todo).

---

### Fase 16 — API OpenGL en Rust (`rogl`): GL 2.1 sobre el motor

**Trabajo**
- Estado GL: `glClearColor`, `glClear`, `glEnable/Disable`, `glViewport`,
  `glScissor`, matrices (`glMatrixMode`, `glLoadMatrix`, `glTranslate`, `glRotate`,
  `glOrtho`), `glTexImage2D`, `glTexParameteri`, `glBlendFunc`,
  `glVertexPointer/glTexCoordPointer/glColorPointer`, `glDrawArrays`,
  `glDrawElements` (camino **fixed-function**, estilo GL 1.x, ya funcional).
- Shaders: `glCreateShader`/`glCompileShader`/`glUseProgram` con un **parser de
  subconjunto de GLSL 1.20** (vec/mat, operadores, `attribute`/`varying`/
  `uniform`, `texture2D`, `gl_FragColor`, unas pocas funciones built-in) → IR
  propio → interpretado por el motor de la fase 15. Donde no hay shader, corre
  el fixed-function.
- Contexto: `rogl::Context` con sus buffers y texturas; un contexto por
  programa (D6).

**Verificacion**: `verif-gl-fixed.ps1` (fixed-function: cuadrito texturizado
y blending) y `verif-gl-shader.ps1` (un vertex + fragment shader que tiñe un
triangulo por vertice).

**Riesgo**: medio-alto (el parser de GLSL es lo delicado). **Tamano**: grande.

---

### Fase 17 — Syscalls que faltan: `mmap` y reloj

**Trabajo**
- `mmap`/`munmap` anonimos para anillo 3 (frames del PMM en la ventana DATA).
- `mmap` **compartido**: region cuyos frames estan tambien mapeados en el
  kernel, para que el compositor lea el frame del programa sin copias (D5).
- `clock_gettime` + `nanosleep` para anillo 3: sin ellos el bucle de render
  quema el CPU y no hay cadencia.

**Verificacion**: `verif-mmap.ps1`, `verif-reloj.ps1`.

**Riesgo**: bajo (MMU, PMM y COW ya estan). **Tamano**: pequena.
**Independiente**: se puede hacer en paralelo a la 15 y 16.

---

### Fase 18 — Fundamentos del driver y aceleracion 2D real

Lo primero que **se nota** como aceleracion, y **no depende de la API GL**.

**Trabajo**
- Ampliar con seguridad la capacidad de carga y ubicar kernel, pila y buffers
  sin pisar EBDA/VGA; conservar compatibilidad BIOS legacy y probar el USB.
- Crear asignacion de buffers GPU y acceso DMA visible para la RX 550; manejar
  timeout/reset para que un atasco de GPU no congele Kira.
- Inicializar SDMA con el firmware correspondiente a Lexa y enviar copias y
  rellenos de rectangulos. SDMA no se debe presentar como compositor alfa ni
  como rasterizador 3D.
- Migrar operaciones compatibles de `Canvas` a SDMA con fallback CPU. Para
  transparencia, escalado y mezcla de ventanas se necesita el motor GFX/CP,
  no solo una cola de copia.
- Mantener VBE/CPU operativo como recuperacion si el firmware o el motor no
  arrancan. No activar el driver automaticamente hasta validar en la RX 550.

**Verificacion**: **solo PC real** (el `ati-vga` de QEMU es una tarjeta antigua
que ni alcanza el bloque DCE). En QEMU se verifica contra la implementacion por
CPU como referencia de pixeles; en el PC real se mide modo activo y tiempo de
blit.

**Riesgo**: alto. **Tamano**: grande. La validacion final requiere el PC real;
QEMU no sustituye pruebas de SDMA en Lexa.

---

### Fase 19 — Composicion y rasterizacion GPU: CP/PM4 + shaders GCN

**Trabajo**
- Ring buffer **PM4**, firmware y memoria de comandos/buffers con validacion y
  limites; inicializacion del CP y sus microcontroladores.
- Shaders GCN para composicion alfa, escalado, texturas, triangulos y texto;
  sustituir las rutas CPU de Kira una por una, manteniendo referencia CPU.
- Backend `gpu` para las operaciones graficas soportadas; reportar claramente
  por operacion si se usa GPU o fallback, nunca anunciar aceleracion total si
  queda una ruta de dibujo CPU activa.

**Verificacion**: **PC real**: triangulo con z-buffer dibujado por la GPU, y
comparacion de pixes contra el backend CPU de la fase 15 (que hace de
referencia).

**Riesgo**: muy alto (trabajo de largo recorrido). **Tamano**: muy grande.
Completar solo SDMA no equivale a aceleracion grafica completa: las operaciones
de composicion y rasterizacion deben pasar tambien por GFX/CP.

---

### Fase 20 — Integracion completa con Kira y las aplicaciones

Kira ya existe y dibuja por CPU; esta fase cambia su backend, no crea el
compositor desde cero.

**Trabajo**
- Redirigir el pintado de escritorio, ventanas, iconos, texto y cursor al
  backend GPU, incluyendo actualizacion parcial y superficies con alfa.
- Conectar las apps Toby/AppKit y las aplicaciones Rust a superficies GPU con
  limites de memoria y sincronizacion; mantener fallback CPU.
- Medir latencia, FPS, carga CPU y exactitud visual en QEMU y en la RX 550 real.

**Verificacion**: `verif-gl-ventana.ps1` (cliente dibuja, se arrastra la
ventana con QMP y el screendump sigue siendo el triangulo en la nueva
posicion).

**Riesgo**: medio. **Tamano**: media.

---

### Opcional — Fase 21: LLVM (GLSL completo + backend AMDGPU)

Si se quiere GLSL completo y shaders en la GPU sin escribir un assembler GCN
propio, la salida es enlazar **LLVM** (y con el el backend AMDGPU). Eso
implicaria relajar D1 (sin C). Se valora cuando las fases 15-19 esten verdes.

---

## 3. Orden y dependencias

```
Layout/carga del kernel -> memoria GPU + firmware -> SDMA (copias/rellenos)
                                                 -> GFX/CP (alfa/texturas/dibujo)
                                                 -> Kira + apps (GPU por defecto)
Fases 15/16: renderer CPU y API GL de usuario como fallback/trabajo paralelo
```

- **Layout/carga del kernel: completado.** Ambos caminos BIOS se probaron en
  QEMU y el binario actual cabe en 512 KiB. Quedan 47.7 KiB en la ventana de
  carga; volver a ampliarla exige otro cambio probado en los dos cargadores.
- Luego: buffers/DMA + SDMA (copias y rellenos) y pruebas en Lexa.
- Despues: CP/PM4 + shaders para composicion, compositor Kira, texto/recursos
  y superficies GPU para apps.
  La meta completa solo se considera alcanzada cuando Kira no usa rasterizacion
  CPU para sus operaciones graficas soportadas y el fallback sigue funcional.

---

## 4. Riesgos globales

- **Sin C y sin Mesa**: el driver y las colas GPU se escriben en Rust. El
  kernel tiene un limite de carga de 512 KiB; el binario actual deja unos
  47.7 KiB libres.
- **GLSL es subconjunto**: hay que documentar bien en el README lo que no
  soporta (no prometer GLSL completo).
- **GPU 3D (fase 19) es el riesgo mayor**: requiere firmware, colas, memoria y
  shaders GCN; no se puede certificar sin el PC real.
- **Verificabilidad**: fases 15-17 y 20 enteras en QEMU; la 18 y la 19 solo en
  el PC real cuando se pruebe SDMA/GFX en la RX 550. QEMU puede probar un
  backend VirtIO/virgl separado cuando exista su driver de invitado, pero eso
  no valida los registros ni firmware Polaris.
- **Límite de carga de 512 KiB**: ya se verificó la reubicación, la RAM baja y
  la pila en QEMU. Cualquier aumento futuro debe volver a probar arranque BIOS
  legacy y USB.

---

## 5. Que tocar primero (arranque inmediato)

Dos caminos, y se pueden atacar a la vez porque son independientes:

**A. Layout de carga — completado en QEMU:**
1. Ambos cargadores copian hasta 512 KiB y reubican la imagen a 1 MiB.
2. El linker limita kernel+BSS a 3 MiB; la pila persistente ocupa `[3,4) MiB`.
3. El binario actual mide 475.408 B; el arranque físico legacy aún debe
   confirmarse en el PC del usuario.

**B. Aceleracion 2D real (fase 18)** — sobre la RX 550:
1. Integrar y licenciar firmware SDMA para Polaris 12/Lexa.
2. Crear buffers DMA accesibles por GPU y una cola acotada con timeout/fallback.
3. Copiar/rellenar primero; medir frente a CPU en el PC real y conectar Kira.
4. implementar alfa/escalado en GFX/CP; no adjudicarlos a SDMA.

**C. Motor de render (fase 15)** — backend CPU de referencia/fallback:
1. `rogl-core` (crate nuevo, `no_std`, en anillo 3): rasterizador de triangulos.
2. Z-buffer + interpolacion de varyings.
3. Texturas + blending.
4. `verif-rogl-triangulo.ps1`.

Las fases 15/16 (OpenGL por CPU) son el backend de referencia y pueden avanzar
en paralelo; no sustituyen la ruta SDMA+GFX. Tras validar A y B en hardware, se
implementa la fase 19 y se migra todo Kira/apps en la fase 20.
