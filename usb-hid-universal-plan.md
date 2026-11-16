# Plan de implementación: USB universal + HID para Rocky

## Objetivo y alcance

Implementar en Rocky una pila USB propia, escrita en Rust `no_std`, que detecte
controladores USB PCI, enumere dispositivos y permita usar teclados y ratones
USB HID en Kira, la consola y las aplicaciones. Debe conservar el camino PS/2
actual como respaldo y no bloquear el arranque ni congelar la GUI cuando un
controlador o dispositivo no responda.

“Universal” significa una arquitectura común para USB y para clases de
dispositivo, con controladores de host separados. **No** significa un solo
driver capaz de hablar directamente con todos los chips ni soporte inmediato
para cualquier periférico USB. El primer objetivo funcional es HID teclado y
ratón; almacenamiento, audio, red, cámaras y hubs complejos quedan fuera de la
primera entrega.

Para el T440p (i7-4712MQ/Haswell) y portátiles modernos, el primer controlador
de host debe ser **xHCI**. Después se añaden EHCI y, solo si las máquinas de
destino lo requieren, UHCI/OHCI. Cada uno implementa la misma interfaz HCD y
usa el mismo núcleo USB/HID.

Un touchpad interno puede no ser USB: algunos se presentan como PS/2, SMBus o
I2C-HID. Esta pila arregla los que realmente enumeran en USB; no sustituye un
driver ACPI/I2C/SMBus para un touchpad que no aparezca en el bus USB.

## Estado del repositorio que la IA debe confirmar antes de editar

- El kernel es x86_64 `no_std`, arranca por BIOS/CSM y continúa en long mode.
  No hay servicios UEFI disponibles después del arranque.
- `kernel/src/pci.rs` enumera PCI y expone `Funcion`, `Bar`, acceso a config y
  `habilitar_memoria_y_bus_master`. Hay que confirmar si cubre los BAR de 64
  bits y cómo devuelve sus tamaños antes de usar registros de un HCD.
- `kernel/src/paging.rs` documenta un mapeo identidad de los primeros 4 GiB y
  tiene mapeo de páginas. Los MMIO deben marcarse como memoria de dispositivo;
  un BAR por encima de 4 GiB no se puede desreferenciar por identidad sin
  ampliar el mapeo.
- `kernel/src/pmm.rs` administra marcos físicos contiguos hasta 1 GiB y ofrece
  `alloc_frames`/`free_frames`; **no asumir** que sus resultados ya tienen la
  alineación o límites DMA que exige cada estructura xHCI.
- `kernel/src/drivers/mod.rs` solo arranca ahora GPU. PCI no es un HCD ni una
  pila USB.
- La entrada existente está repartida entre `kernel/src/input.rs`,
  `kernel/src/mouse.rs`, `kernel/src/ps2.rs`, `kernel/src/pic.rs` e
  `kernel/src/kira/comp.rs`. Kira consume teclado y ratón en su tarea de fondo.
- `kernel/src/tarea.rs` planifica tareas; una transferencia USB nunca debe
  mantener `tarea::critica` abierta mientras espera al hardware.
- El constructor oficial es `build.ps1` / `cargo run --manifest-path
  xtask/Cargo.toml -- build`. El kernel requiere las flags y el linker script
  de `xtask`; una compilación Cargo genérica puede fallar al enlazar y no
  equivale a validar el build oficial.

La IA debe comprobar el árbol actual, buscar instrucciones locales y leer los
módulos citados completos antes de implementar. No debe modificar ni reemplazar
`mareaplan.md`; este documento es el plan específico de USB/HID.

## Arquitectura objetivo

```text
PCI y mapeo MMIO/DMA
        ↓
USB HCD: xHCI primero; EHCI/UHCI/OHCI como drivers independientes
        ↓ interfaz común de transferencias y puertos
USB core: reset, velocidades, direcciones, descriptores, endpoints, hubs
        ↓
USB HID: boot protocol + parser acotado de report descriptors
        ↓ eventos normalizados, colas acotadas y sin asignación en IRQ
input común de Rocky → consola / Toby UI / Kira / Finder
```

El HCD posee exclusivamente sus registros, anillos y buffers DMA. USB core
posee el estado de enumeración. HID convierte informes en eventos. La capa
`input` publica esos eventos y mantiene compatibilidad con los consumidores
actuales. Ninguna capa superior debe leer registros PCI/MMIO ni conocer TRB,
descriptores de controlador o códigos de tecla PS/2.

## Estructura sugerida

Adaptarla a las convenciones existentes si ya hay código equivalente; no crear
duplicados por cumplir literalmente estos nombres.

```text
kernel/src/drivers/
  mod.rs
  usb/
    mod.rs             # registro e inicio de HCD
    pci.rs             # selección por clase/subclase/prog_if
    dma.rs             # asignación, liberación y validación de DMA
    mmio.rs            # acceso volatile y límites de BAR
    core/
      mod.rs
      descriptor.rs    # decodificación endian-safe y validada
      enumerate.rs     # dispositivo/configuración/interfaz/endpoint
      transfer.rs      # setup/control/interrupt y errores comunes
      hub.rs           # posterior a HID en root ports
    hcd/
      mod.rs           # interfaz HostController
      xhci/            # primer HCD funcional
      ehci.rs          # fase posterior
      uhci.rs          # opcional según hardware objetivo
      ohci.rs          # opcional según hardware objetivo
    hid/
      mod.rs
      boot.rs          # teclado/ratón boot protocol
      report.rs        # parser HID acotado
      keyboard.rs
      mouse.rs
  input.rs             # API común; puede seguir siendo kernel/src/input.rs
```

## Reglas que no se pueden violar

1. Mantener PS/2 y puerto serie funcionales. El fallo de USB nunca debe impedir
   que Rocky llegue a Kira o a la consola.
2. Inicializar USB como trabajo de fondo, con límites por paso. El arranque
   gráfico no espera a que un puerto sin dispositivo responda.
3. No reservar memoria, interpretar descriptores, hacer logging ni dibujar
   desde una interrupción. La IRQ (si se usa) solo reconoce el origen y publica
   una notificación acotada; el trabajo se hace en tarea/sondeo.
4. Cada espera tiene deadline monotónico y límite de iteraciones. Timeout,
   desconexión, reporte inválido o controlador ausente son errores recuperables,
   nunca `panic!`.
5. Las colas y buffers tienen tamaño máximo explícito, contadores de overflow y
   una política de descarte/coalescencia. No se permite corrupción silenciosa.
6. Validar toda longitud, offset, índice, tamaño de paquete, número de puerto,
   número de slot, longitud de descriptor y dirección DMA antes de leer/escribir.
7. No seguir punteros DMA devueltos por el dispositivo sin verificar que
   pertenecen a regiones reservadas por Rocky.
8. Respetar propiedad única y no guardar referencias `&mut` a estado global a
   través de puntos de planificación o espera.
9. No asumir un fabricante, ID PCI, resolución, cantidad de puertos o tamaño de
   descriptor fijo. Hacer despacho por clase PCI y capacidades anunciadas.
10. No entregar el controlador al sistema operativo ni desactivar la emulación
    USB heredada hasta que el HCD esté inicializado y se haya probado entrada.
    La secuencia xHCI BIOS/OS ownership debe tener timeout y una salida segura.

## Estado de las fases (2026-10-26)

| fase | estado | donde / prueba |
|---|---|---|
| 0 - Inventario y linea base | hecha | `kernel/src/drivers/usb/pci.rs`, orden `usb` |
| 1 - PCI, MMIO y DMA seguros | hecha | `usb/mmio.rs`, `usb/dma.rs`, validacion de BAR |
| 2 - xHCI: handoff y arranque | hecha | `usb/hcd/xhci/`; `verif-usb.ps1` |
| 3 - Nucleo USB y enumeracion | hecha | `usb/core/`, `xhci/enumeracion.rs`; `verif-usb.ps1` |
| 4 - HID boot teclado/raton | hecha | `xhci/hid.rs`, `rockyhw/src/hid/`; `verif-hid.ps1` |
| 5 - Entrada unificada y metricas | hecha | eventos a `input`/`mouse`, contadores en `usb` |
| 6 - HID generico (descriptor de informe) | hecha | rockyhw/src/hid/informe.rs + parser en xhci/hid.rs; cargo test (rockyhw) |
| 7 - USB 2.0 (EHCI) y hubs | parcial | hubs USB 2.0, route string/TT, hotplug y retirada segura en xHCI; faltan EHCI/UHCI/OHCI. `verif-hid.ps1` |
| 8 - Validacion fisica | parcial | matriz QEMU hecha, incluido hub y hotplug; falta el T440p y 30 min de estres |

La fase 5 se resolvio por la via del **adaptador**: los usos HID se traducen a
la posicion del juego 1 y entran por la misma cola que PS/2; no existe el tipo
neutral `Key`/`Pointer`/`DeviceChanged` que proponia el punto 1, pero tampoco
hay dos colas ni dos traducciones que se puedan desincronizar. Las fases 0 a 6
y los hubs de la fase 7 las comprueban `verif-usb.ps1`, `verif-hid.ps1` y las
pruebas de `rockyhw` en QEMU (xHCI, almacenamiento USB, teclado y raton, hubs
USB 2.0, conexion y desconexion en caliente).

## Fases de trabajo

### Fase 0 — Inventario y línea base

1. Confirmar que la rama de trabajo incluye los últimos cambios de `input`,
   PS/2, Kira y PIC; no restaurar snapshots ni sobrescribir cambios del usuario.
2. Añadir diagnóstico de solo lectura de funciones PCI USB: BDF, vendor/device,
   clase, subclase, `prog_if`, BAR y capacidad DMA. No habilitar bus mastering
   todavía ni escribir en el controlador durante el inventario.
3. Añadir una orden de diagnóstico (por ejemplo `usb`) que distinga claramente:
   `sin controlador`, `HCD detectado pero aún no soportado`, `HCD inicializado`,
   `puerto conectado`, `dispositivo enumerado` y `HID activo`.
4. Registrar el resultado de `lspci` en QEMU y en el T440p antes de programar
   hardware. Confirmar si el touchpad interno aparece como USB, PS/2 o ACPI/I2C.

**Salida:** mapa real de controladores del equipo objetivo y prueba de que el
arranque/PS2 actual sigue intacto.

### Fase 1 — Primitivas seguras de PCI, MMIO y DMA

1. Extender PCI con un predicado de HCD USB basado en class `0x0C`, subclass
   `0x03` y `prog_if`: UHCI `0x00`, OHCI `0x10`, EHCI `0x20`, xHCI `0x30`.
   No deducir el tipo por el nombre del fabricante.
2. Auditar y corregir el acceso a BAR: tipo I/O vs MMIO, BAR de 64 bits,
   dirección cero/no asignada, tamaño y desbordamiento. No medir un BAR activo
   escribiendo `0xFFFF_FFFF` después de que el dispositivo ya esté en uso.
3. Añadir una API `DmaRegion` fallible: reserva de marcos contiguos, alineación
   explícita, límite físico inclusivo (inicialmente <4 GiB), tamaño redondeado a
   páginas, acceso virtual válido, cero inicial, liberación y comprobaciones de
   rango. Si el PMM no puede expresar alineación/límite, extenderlo con pruebas.
4. Asegurar que las regiones DMA no se devuelven al PMM mientras el dispositivo
   pueda usarlas. Liberar anillos solo después de detener el HCD y confirmar que
   no hay transferencias en vuelo.
5. Añadir acceso MMIO `read_volatile`/`write_volatile`, orden de memoria y
   verificación de límites de BAR. No convertir el BAR en una referencia Rust
   de tamaño ilimitado.
6. Habilitar PCI Memory Space y Bus Master solo después de validar BAR,
   reservar estructuras DMA y completar la secuencia de ownership del HCD.
7. Documentar la hipótesis de coherencia DMA x86. No añadir flushes arbitrarios;
   definir fences/sincronización y qué ocurre si se detecta IOMMU/VT-d no
   preparado. En caso inseguro, abortar ese HCD y mantener PS/2.

**Pruebas:** alineación, límite 32-bit, agotamiento, free sin doble liberación,
BAR MMIO truncado, BAR 64-bit, accesos fuera de rango. Estos tests deben poder
ejecutarse sin hardware USB.

### Fase 2 — xHCI: detección, handoff y arranque seguro

1. Empezar con un HCD xHCI genérico conforme a clase PCI, sin IDs específicos
   de Intel/AMD. Leer `CAPLENGTH`, versión, `HCSPARAMS`, `HCCPARAMS`, tamaño de
   página, número de slots/puertos, scratchpads y longitud de la lista de
   capacidades extendidas; validar cada campo antes de usarlo.
2. Implementar el BIOS/OS ownership handoff de xHCI con timeout. Si BIOS no
   cede, el BAR no es válido, el reset no termina o las capacidades son
   incoherentes: marcar ese controlador como fallido, registrar por serial y
   continuar con los demás dispositivos de entrada.
3. Implementar halt/reset/start según el estado observado, no con escrituras
   ciegas. Esperar con deadline a `Halted`, `Controller Not Ready` y reset.
4. Reservar/validar DCBAA, command ring, event ring y ERST con las alineaciones
   exigidas por xHCI; inicializar cycle bits, dequeue pointers, interrupter y
   doorbells. Calcular el número de scratchpad buffers desde las capacidades y
   no desde un valor fijo.
5. Primera versión: sondeo acotado del event ring desde una tarea USB. No
   depender de MSI/MSI-X ni del enrutamiento PCI de interrupciones todavía.
   Definir cómo se detecta `USBSTS.HSE`, errores de comando y timeouts.
6. Gestionar root ports: detectar conexión, reset, velocidad y cambio de estado;
   no tratar un puerto USB 2.0 como SuperSpeed por la forma del conector.
7. En cualquier reinicio/timeout, detener y vaciar el controlador antes de
   reutilizar DMA. Si no es posible confirmar el halt, no liberar esas páginas.

**Salida:** el controlador xHCI aparece como inicializado y los root ports
muestran conexión/velocidad; aún no se requiere reconocer teclado.

### Fase 3 — Núcleo USB común y enumeración

1. Definir interfaz HCD con operaciones explícitas y fallibles: iniciar/parar,
   inspeccionar puertos, resetear puerto, reservar slot, configurar endpoint,
   enviar control transfer y programar/consultar interrupt transfer. xHCI
   traduce estas operaciones a TRBs; USB core no manipula TRBs.
2. Crear tipos acotados para `UsbDevice`, `UsbConfiguration`, `UsbInterface`,
   `UsbEndpoint`, velocidad, dirección, tipo de transferencia y estado. Evitar
   `Vec` basado en longitudes no confiables; imponer topes de dispositivos,
   interfaces, endpoints, hubs y descriptor total.
3. Implementar enumeración estándar para xHCI: Enable Slot, Address Device,
   leer descriptor inicial de dispositivo, ajustar EP0, obtener descriptor
   completo, configuración y descriptores de interfaz/endpoint. Respetar que
   xHCI administra el direccionamiento mediante contextos/comandos propios.
4. Validar `bLength >= 2`, `bLength <= bytes restantes`, descriptores truncados,
   interfaces repetidas, endpoint inválido, `wTotalLength`, `wMaxPacketSize`,
   dirección y velocidad. Un descriptor malicioso solo desconecta/rechaza ese
   dispositivo.
5. Implementar control transfers con setup/data/status, dirección correcta,
   toggle/cycle atendido por HCD, stalls, short packets, cancelación y timeout.
6. Negociar configuración sin asumir que la primera interfaz es HID. Recorrer
   descriptores de configuración/interfaz y enlazar por clase de interfaz.
7. Implementar inicialmente dispositivos conectados directamente a root ports.
   Detectar hubs y reportarlos como “no soportados aún”; no recorrerlos a ciegas.

**Pruebas puras:** parser de descriptores con ejemplos válidos, truncados,
longitudes cero/uno, longitudes gigantes, múltiples interfaces, múltiples
endpoints, HID no primera interfaz, short packet y STALL.

### Fase 4 — HID boot: teclado y ratón USB

1. Encontrar interfaces HID por códigos de clase de interfaz y validar
   subclass/protocol. No basarse solo en la clase declarada por el dispositivo.
2. Para boot keyboard/mouse, obtener el HID descriptor y el report descriptor;
   seleccionar configuración, interfaz alterna y endpoint interrupt-IN reales.
   Solicitar `SET_PROTOCOL(Boot)` cuando el dispositivo lo soporte y comprobar
   resultado; no asumir que todo HID admite boot protocol.
3. Teclado boot: validar longitud de reporte, traducir usos HID (no scan codes
   PS/2), procesar cambios entre reportes, modificadores y hasta el límite de
   teclas simultáneas. Tratar `ErrorRollOver` sin convertirlo en pulsaciones
   falsas. La distribución española/CP437, teclas muertas y Ctrl deben compartir
   una capa de traducción comprobable con la entrada actual.
4. Ratón boot: validar reporte de botones y ejes con signo, respetar límites del
   endpoint, acumular deltas sin overflow y mapear botones/wheel que existan.
   No asumir siempre paquetes de 3 bytes PS/2.
5. Mantener solicitudes interrupt-IN activas según el intervalo del endpoint.
   Reponer la transferencia después de cada completado; manejar desconexión,
   Stall y cambio de endpoint sin bucle ocupado.
6. Publicar eventos desde bottom-half/tarea a colas fijas compartidas con
   `input`. Nunca llamar a Kira ni dibujar desde un evento USB.

**Salida:** teclado y ratón USB boot-class funcionan en QEMU xHCI, manteniendo
PS/2 y serie a la vez.

### Fase 5 — API unificada de entrada y Kira

1. Diseñar un evento neutral, por ejemplo `Key { usage/key, pressed,
   modifiers }`, `Pointer { dx, dy, buttons, wheel }` y `DeviceChanged`. Si para
   conservar compatibilidad se mantiene `poll_event() -> (u8, bool)`, que sea
   un adaptador de la cola neutral, no una segunda cola independiente.
2. Conservar `input::poll()` para shell/TTY existentes hasta migrarlos; no
   introducir una ruta que consuma los eventos USB y deje sin eventos PS/2.
3. Integrar el movimiento USB por la misma API de puntero que usa
   `mouse::tomar()`, con sincronización y límites claramente definidos. No
   sobrescribir botones o deltas PS/2 con estado USB ni a la inversa.
4. Actualizar Kira, TTY, shell, demo y Toby UI para manejar eventos sin perder
   Escape, navegación, Ctrl, texto ni ratón. Mantener el orden temporal entre
   teclado y puntero y definir foco/dispositivo cuando hay varios.
5. Desconexión de teclado no debe dejar Shift/Ctrl/Alt virtuales “pegados”:
   sintetizar liberación segura de las teclas/modificadores mantenidos por ese
   dispositivo.
6. Añadir métricas: controladores detectados/inicializados, dispositivos,
   transferencias, errores/timeouts, máximo de cola, eventos descartados y
   latencia aproximada. Presentarlas en `usb`/`input`, sin log por cada reporte.

**Criterio:** la GUI sigue respondiendo durante enumeración y durante un
controlador ausente; ninguna espera USB ocurre dentro de una sección crítica de
Kira.

### Fase 6 — HID genérico (no solo boot protocol)

1. Implementar parser HID acotado: items cortos/largos, estado global/local,
   `Report ID`, `Report Size`, `Report Count`, `Push/Pop`, colecciones y campos
   Input. Limitar profundidad, cantidad de campos, tamaño de reporte y bytes
   totales; detectar overflow de bits.
2. Interpretar solo las usages necesarias para primera versión genérica:
   Generic Desktop Keyboard/Keypad, Mouse, X/Y/Wheel y Button page. Ignorar
   usages desconocidas con seguridad.
3. Soportar varios report IDs y offsets de bits no alineados a bytes. Usar
   signo correcto según `Logical Minimum/Maximum`; no tratar todos los campos
   como `u8`.
4. Si descriptor es inválido o excede límites, intentar Boot protocol si la
   interfaz lo permite; si tampoco, rechazar esa interfaz sin afectar otras.
5. Añadir tests de report descriptors reales y sintéticos: teclado compacto,
   teclado con LEDs/output report, ratón de 3/5 botones, wheel, report ID,
   campo de 12 bits, campos desconocidos, long item, truncado y anidamiento
   inválido.

**Salida:** HID teclado/ratón estándar no boot que describe sus reportes puede
funcionar sin IDs por fabricante.

### Fase 7 — Compatibilidad USB 2.0 y hubs

1. Implementar EHCI detrás de la misma interfaz HCD para controladores PCI
   `prog_if 0x20`, con schedules y DMA propios. Tener en cuenta companion
   controllers/ownership del firmware; no apagar EHCI si no hay reemplazo activo.
2. Añadir hubs USB 2.0 y root-hub port status/change con límites y recursión
   acotados. Un hub desconectado o que no responde no detiene los demás puertos.
3. Solo después, priorizar UHCI (`0x00`) u OHCI (`0x10`) según las máquinas que
   realmente se quieran soportar. No inventar una API pública separada para
   cada HCD: todos deben pasar el mismo conjunto de pruebas del USB core.
4. Añadir hot-plug, reset/re-enumeración y retirada segura de endpoints/slots.
   Definir estado de dispositivo (`Attached → Default → Addressed → Configured
   → Suspended/Disconnected/Failed`) y transiciones legales.

### Fase 8 — Validación física y endurecimiento

1. Añadir configuraciones QEMU reproducibles sin sustituir las actuales:
   `qemu-xhci + usb-kbd + usb-mouse`; EHCI/USB2 con sus companion controllers;
   PS/2-only; sin dispositivo; dispositivo desconectado durante uso. La prueba
   debe comprobar que se mueve Kira y que la tecla llega a TTY, no solo que el
   PCI class-code aparece.
2. Validar en T440p: teclado integrado, TrackPoint/touchpad según bus real,
   teclado USB externo y ratón USB; registrar `lspci`, árbol USB, velocidad,
   descriptor HID y contadores. Repetir con USB Legacy Support activado y, si
   el firmware lo permite, desactivado para probar el HCD nativo.
3. Validar al menos un xHCI Intel y uno AMD, USB 2.0 y 3.x, hubs alimentados y
   no alimentados. QEMU no sustituye esta matriz física.
4. Probar durante 30 minutos: mover ratón continuamente, escribir en Terminal,
   abrir/cerrar Finder, desconectar/reconectar, llenar colas y enviar reportes
   malformados. Criterios: sin kernel panic, sin freeze, sin tecla/modificador
   pegado, sin cursor perdido y sin crecimiento de memoria por cada reporte.
5. Ejecutar `cargo fmt --check`, tests disponibles y el build oficial de `xtask`.
   Crear imagen solo cuando no haya QEMU montando esa imagen; comprobar tamaño
   de carga, imagen BIOS y USB. No declarar “funciona en todos los portátiles”
   basándose solo en compilación o QEMU.

## Definition of Done

- xHCI genérico por clase PCI funciona en QEMU y en el T440p sin IDs de
  fabricante cableados.
- Al menos un teclado y un ratón USB HID boot protocol generan eventos reales
  en Kira y TTY; PS/2 y serie siguen funcionando simultáneamente.
- USB core reconoce, valida y enumera dispositivos/interfaces/endpoints, y
  rechaza descriptores inválidos de forma recuperable.
- Un descriptor HID report-protocol válido de teclado/ratón pasa el parser y
  genera eventos; las pruebas negativas no panic ni leen fuera del buffer.
- Las transferencias tienen timeout, el trabajo es planificable y no hay
  asignación ni renderizado en IRQ.
- No se vende soporte de EHCI/UHCI/OHCI ni touchpad I2C hasta que cada driver
  correspondiente pase pruebas en hardware/QEMU.
- Diagnóstico puede explicar por qué no funciona: falta de controlador, HCD no
  soportado, fallo de handoff, timeout, descriptor rechazado o clase sin driver.

## Instrucciones para la IA que implemente este plan

1. Lee primero este documento, `mareaplan.md`, `README.md` y cualquier
   `AGENTS.md` vigente. Vuelve a inspeccionar los archivos fuente: el estado del
   repositorio puede haber cambiado desde que se escribió este plan.
2. No intentes completar todas las fases en una sola modificación. Empieza por
   Fase 0 y Fase 1: inventario observable, APIs de DMA/MMIO seguras y pruebas.
   Compila con el flujo oficial y presenta los hallazgos antes de iniciar xHCI.
3. Después implementa xHCI en incrementos que arranquen: detección PCI;
   handoff/reset; rings/eventos; puertos; control transfers/enumeración; HID
   boot; integración de eventos; parser report-protocol; EHCI/hubs.
4. No dejes módulos vacíos, funciones que devuelvan éxito ficticio ni
   “drivers” que solo impriman el ID PCI. Cada fase debe entregar una función
   observable y tests del comportamiento declarado.
5. Si no puede probar hardware, indícalo exactamente. Separa siempre:
   compilado, probado en test unitario, probado en QEMU y probado físicamente.
6. Si una modificación amenaza el arranque, el PS/2 o los datos de disco,
   conserva el último camino funcional, documenta el bloqueo y no deshabilites
   la ruta de respaldo para aparentar que el nuevo driver está listo.
7. En cada entrega enumera archivos cambiados, pruebas ejecutadas y fallos
   pendientes; no declares compatibilidad universal si solo se probó un HCD o
   un modelo de portátil.

## Referencias normativas primarias

- [USB-IF: USB 2.0 Specification](https://www.usb.org/document-library/usb-20-specification)
- [Intel: índice de especificaciones USB/xHCI (incluye la revisión xHCI más reciente)](https://www.intel.com/content/www/us/en/products/docs/io/universal-serial-bus/universal-serial-bus-specifications.html)
- [Intel: xHCI Requirements Specification, Revision 1.2c (octubre de 2025)](https://cdrdv2.intel.com/v1/dl/getContent/868295)
- [USB-IF: HID Device Class Definition 1.11](https://www.usb.org/document-library/device-class-definition-hid-111)
- [USB-IF: códigos de clase de dispositivo e interfaz](https://www.usb.org/defined-class-codes)

Usar las especificaciones como autoridad para layouts de registros, TRBs,
ownership, solicitudes y descriptores. No copiar código de Linux como si fuera
una especificación ni portar trozos de un driver sin entender sus invariantes,
dependencias, licencias y modelo de DMA.
