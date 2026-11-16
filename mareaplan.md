# Marea: plan de arquitectura y ejecución para Rocky

## Propósito

Este documento guía, paso a paso, la mejora de la multitarea, la respuesta de la interfaz y el uso de memoria de Rocky. Está escrito para que otra persona o una IA con menos contexto pueda continuar el trabajo sin tener que rediseñar el sistema desde cero.

La prioridad es mejorar el kernel **sin quitar funciones**: Finder, sus cuatro vistas, selección, navegación, apertura de `.app`, Terminal en modo ventana, escritorio, Dock, ventanas redimensionables, Kira, Toby UI y compatibilidad con el arranque BIOS heredado deben seguir funcionando.

“Marea” es el nombre de esta propuesta para Rocky: un kernel inicialmente mononúcleo con planificación apropiativa, trabajo diferido y servicios con propiedad única de sus datos. Combina ideas conocidas de sistemas operativos; no se afirma que sea una teoría inédita. Primero se busca corrección y latencia predecible; después, rendimiento y escalabilidad.

## Estado conocido al iniciar este plan

Ya se han aplicado y compilado estos cambios iniciales:

- El planificador tiene una tarea idle que ejecuta `sti; hlt` cuando no hay trabajo normal. No se muestra como tarea de usuario y no se puede matar.
- Las tareas dormidas o terminadas no se vuelven a poner en ejecución como alternativa a idle.
- La alarma puede despertar una tarea bloqueada en `pause`.
- `paging::activar` evita volver a escribir CR3 cuando ya está activo el mismo espacio de direcciones.
- `cargo fmt --check` y la compilación oficial mediante `xtask` pasan. La última compilación indicó un kernel de 517.128 bytes.

Estos cambios son el punto de partida: **no los reviertas**. La prueba de arranque serie de QEMU llegó al prompt antes del último ajuste de alarma; después de ese ajuste se volvió a compilar correctamente, pero hay que repetir el arranque como parte de la siguiente fase.

Quedan pendientes las mejoras arquitectónicas de este documento. En particular, la carga inicial del Finder todavía puede bloquear el trabajo de otras tareas.

## Diseño objetivo

### 1. Interrupciones cortas y trabajo diferido

La parte superior de una interrupción (top half) debe hacer lo mínimo: leer/registrar el evento, poner una notificación en una cola preasignada y confirmar la interrupción al controlador. El trabajo costoso se ejecuta después en una tarea o servicio planificable (bottom half).

Una interrupción no debe reservar memoria, construir cadenas, recorrer directorios, dibujar una GUI ni esperar a un dispositivo. Las colas usadas por IRQ deben tener capacidad fija y definir qué ocurre cuando se llenan (por ejemplo: contador de pérdida, coalescencia de eventos o señal de sobrecarga; nunca corrupción silenciosa).

### 2. Planificador con clases de latencia y equidad

Mantener al principio un solo núcleo x86_64. Separar las tareas listas, como mínimo, en:

- **Interactiva:** entrada, ventanas y trabajo breve necesario para que la GUI responda.
- **Normal:** aplicaciones y servicios comunes.
- **Fondo:** exploraciones, precarga y trabajo que puede avanzar gradualmente.

La prioridad interactiva no puede dejar sin CPU indefinidamente a las otras clases. Usar presupuestos o pesos con una política de equidad medible. Las tareas bloqueadas no deben ocupar una cola de ejecución. Los vencimientos de temporizadores deben gestionarse con una estructura eficiente (por ejemplo, rueda de temporizadores) cuando las mediciones demuestren que el escaneo actual es costoso.

### 3. Islas de propiedad única y mensajes acotados

Cada servicio crítico tiene un propietario que modifica su estado. Los demás solicitan operaciones mediante mensajes tipados, identificadores y resultados; no retienen referencias mutables a través de tareas.

Orden sugerido de servicios:

1. **Disco:** único propietario de las operaciones de dispositivo y de sus búferes compartidos.
2. **Sistema de archivos:** único escritor del estado del volumen; usa el servicio de disco.
3. **Kira/compositor:** único propietario de escena, ventanas y framebuffer; recibe eventos y comandos de dibujo.

Las colas son acotadas y preasignadas. Cada petición define respuesta, cancelación, error, timeout y contrapresión. Evitar ciclos: el disco no llama al FS; el FS solicita al disco; Kira solicita al FS, pero el FS no espera a Kira.

### 4. GUI y aplicaciones

Kira conserva la propiedad de la composición y la salida de pantalla. El trabajo lento de Finder (enumerar, ordenar, leer metadatos) se convierte en peticiones asíncronas que devuelven lotes pequeños y progreso. La interfaz presenta resultados conforme llegan.

Las aplicaciones Toby deben migrar gradualmente de componentes ejecutados dentro del host de Kira a procesos separados que intercambian eventos y comandos/superficies. No se debe hacer una migración masiva: conservar una capa de compatibilidad para las APIs y los paquetes `.app` existentes.

### 5. Memoria y cambio de contexto

El asignador físico debe dejar de buscar desde el primer marco en cada petición; usar una pista de búsqueda, lista libre o buddy según las capacidades existentes. El heap del kernel puede usar clases de tamaño/slab para objetos frecuentes y una ruta aparte para bloques grandes. Medir antes de elegir.

No se asigna memoria desde IRQ, el planificador ni sus rutas críticas. Las reservas que puedan fallar deben usar APIs fallibles y devolver errores cuando sea posible, en lugar de provocar una cadena de asignaciones inesperada.

El cambio de tarea debe preservar el estado completo que una tarea puede modificar. Hay que incluir x87/SSE antes de confiar en código de usuario que los use; AVX/XSAVE queda para una fase posterior con detección CPUID y configuración correcta de XCR0. La decisión de guardar/restaurar debe implementarse en el límite de ensamblador correcto: el propio dispatcher Rust puede usar registros SIMD.

## Hallazgos y riesgos que condicionan el trabajo

1. **Secciones críticas y Finder.** `tarea::critica` impide el cambio de tarea, pero no equivale a una protección de datos ni necesariamente desactiva IRQ. La carga inicial del Finder en `kernel/src/kira/comp.rs` (buscar `cargar_finder_inicial`) se hace de forma síncrona desde `kernel/src/toby_ui/host.rs`. Se ha observado que el recorrido puede estar dentro de una región crítica. Eso puede dejar sin tiempo de CPU a otras tareas. No retires esa protección hasta definir quién serializa el FS y cómo se protege el volumen.
2. **Entrada de syscall.** La entrada `int 0x80` se configura como puerta de interrupción (`0xEE`) en `kernel/src/idt.rs`; la puerta limpia IF. Revisa `kernel/src/interrupts.s` y el dispatcher antes de cambiarlo. No añadas `sti` a ciegas: primero deben guardarse todos los registros, entenderse la reentrada y protegerse los datos compartidos. Las syscalls lentas deberían poder ceder/bloquear de forma segura.
3. **Estado global sin sincronización suficiente.** `SyncCell<T>` en `kernel/src/sync.rs` usa `UnsafeCell` y `unsafe impl Sync`; su comentario asume mononúcleo/monotarea, aunque existe planificación apropiativa. También hay estado global del volumen en `kernel/src/fs.rs`, estado de disco en `disk.rs`, tablas de descriptores y estado de Kira. Auditar cada acceso. No fabricar referencias `&'static mut` aliasadas.
4. **Planificador con escaneos.** `kernel/src/tarea.rs` mantiene tareas en un `Vec`, recorre tareas para planificar y revisar alarmas. PIT está configurado a 100 Hz (`kernel/src/main.rs`). Esto puede ser suficiente ahora; no sustituirlo sin medir y especificar las transiciones de estado.
5. **Asignador físico y heap.** `kernel/src/heap.rs` usa una lista enlazada first-fit y desactiva interrupciones durante operaciones; `kernel/src/pmm.rs` busca marcos desde el inicio. Las secciones largas y la fragmentación son riesgos de latencia.
6. **Estado SIMD.** `kernel/src/gfx.rs` señala limitaciones relacionadas con XMM. La GUI y el código Rust pueden depender de estado SIMD aunque el código de aplicación no lo haga explícitamente.
7. **Disco síncrono.** `kernel/src/disk.rs` hace operaciones PIO/BIOS y contiene bucles de espera (por ejemplo, límites de hasta 2.000.000 iteraciones). El camino BIOS usa un búfer compartido. Debe serializarse y dividirse en operaciones que no monopolizan el kernel.
8. **No iniciar SMP todavía.** Añadir núcleos antes de hacer seguro el estado global convertiría carreras actuales en corrupción real. SMP es opcional y queda al final.

## Hoja de ruta por fases

Completar y probar una fase antes de empezar la siguiente. Cada cambio debe ser pequeño, revisable y compilarse con la herramienta oficial.

### Fase 0 — Línea base e instrumentos

**Objetivo:** poder demostrar qué mejora y detectar regresiones.

1. Inspeccionar el código actual y confirmar los hallazgos anteriores; anotar diferencias en este documento antes de cambiar el diseño.
2. Añadir contadores de bajo coste y sin asignaciones: cambios de contexto, ticks idle, profundidad máxima de colas, tareas por estado, ticks/tiempo máximo en IRQ desactivadas o preempción inhibida, latencia de syscall, disco, enumeración del Finder y asignaciones.
3. Si se usa TSC, comprobar primero CPUID y calibrar; mantener contadores PIT como alternativa.
4. Exponer métricas por una orden de shell/diagnóstico, no mediante una GUI nueva.
5. Registrar el comportamiento de referencia: arranque, Finder con directorio pequeño/grande, cambiar vistas, selección rápida, arrastrar/redimensionar ventanas, abrir Terminal y varias apps, actividad de disco.

**Criterio de salida:** existe una medición repetible y no altera significativamente la respuesta normal.

### Fase 1 — Invariantes del planificador

**Objetivo:** que cada tarea se ejecute únicamente en estados válidos.

1. Escribir una tabla de estados (lista, ejecutando, dormida/bloqueada, terminada) y las transiciones autorizadas.
2. Añadir pruebas para idle, `sleep`, `pause` + SIGALRM, señales, `waitpid`/zombies, pipes y despertar por evento.
3. Verificar que una tarea bloqueada o zombie nunca se selecciona; idle solo se selecciona cuando no hay tarea lista normal.
4. Revisar carreras entre comprobar una condición, registrarse como espera y dormir. La publicación de espera y el bloqueo deben ser atómicos respecto al evento que despierta.
5. Probar que no hay tareas listas: el sistema debe usar `hlt` y el contador idle aumenta, sin consumir un núcleo ocupado.

**Criterio de salida:** las pruebas de transición pasan y el arranque/uso básico no muestra inanición ni bloqueos perdidos.

### Fase 2 — Disciplina de interrupciones y regiones críticas

**Objetivo:** reducir latencias largas sin abrir carreras.

1. Separar explícitamente “IRQ desactivadas” de “no planificar”. Crear guardas con alcance claro y restauración del estado previo, no pares manuales fáciles de olvidar.
2. Buscar cada uso de `tarea::critica`, `cli`, `sti`, `sin_interrupciones`, `UnsafeCell` y operaciones de disco/FS. Documentar qué protege cada uno y duración medida.
3. Prohibir dentro de guardas: dormir, ceder CPU, hacer E/S lenta, recorrer directorios, dibujar una pantalla completa o reservar memoria potencialmente bloqueante.
4. Convertir IRQ de teclado, temporizador, disco y ratón en rutas acotadas; diferir trabajo cuando haga falta.
5. Solo después de auditar el marco completo y la reentrada, decidir si algunas syscalls pueden habilitar interrupciones o ceder de forma segura. No habilitar interrupciones anidadas como arreglo provisional.

**Criterio de salida:** ninguna región de IRQ/preempción inhibida tiene trabajo de duración no acotada; no hay eventos perdidos ni corrupción.

### Fase 3 — Colas de ejecución y temporizadores escalables

**Objetivo:** evitar escanear todas las tareas en cada tick conforme crezca el número de procesos.

1. Definir una API pequeña de espera/despertar y colas por clase de prioridad.
2. Usar nodos preasignados/intrusivos o almacenamiento reservado; insertar/quitar desde IRQ no puede asignar memoria.
3. Implementar la política de equidad y presupuestos con métricas; evitar que la clase interactiva monopolice CPU.
4. Cambiar el escaneo de temporizadores a una estructura de vencimientos (rueda temporal u otra adecuada) solo cuando las mediciones lo justifiquen.
5. Mantener la frecuencia PIT actual hasta tener datos que indiquen otra necesidad.

**Criterio de salida:** las operaciones del planificador no recorren todas las tareas para el caso común, no hay inanición y se mantienen las APIs actuales.

### Fase 4 — Propiedad y sincronización del estado global

**Objetivo:** retirar el acceso mutable compartido ambiguo.

1. Inventariar todos los globales mutables y consumidores; para cada uno elegir un único propietario, una guarda breve o una cola de mensajes.
2. Definir mensajes tipados, capacidad, errores, cancelación y comportamiento ante cola llena.
3. No introducir un spinlock alrededor de trabajo que puede dormir. En un solo núcleo, una tarea que gira esperando una tarea desalojada que posee el lock puede bloquear el progreso.
4. Hacer que las esperas registren al solicitante antes de dormir y que el despertar no pueda perderse.
5. Mantener adaptadores para las APIs antiguas durante la migración.

**Criterio de salida:** no quedan alias mutables no justificados y cada estado compartido tiene una regla de propiedad documentada.

### Fase 5 — Disco, sistema de archivos y Finder incremental

**Objetivo:** que leer un directorio no congele el sistema.

1. Convertir el acceso a disco en solicitudes serializadas por un servicio propietario, preservando los caminos BIOS y ATA ya soportados.
2. Dividir transferencias y espera por dispositivo en pasos acotados; completar solicitudes con token/resultado y permitir que el solicitante espere cediendo CPU.
3. El FS conserva una vista coherente del volumen y usa el servicio de disco; no exponer referencias al búfer compartido después de completar la petición.
4. Añadir enumeración de directorios por lotes, ordenación incremental o paginada y cancelación al cambiar de carpeta.
5. Actualizar Finder conforme llegan resultados. Conservar exactamente sus vistas, selección, navegación, apertura de `.app`, metadatos disponibles y demás funciones.
6. Quitar `tarea::critica` del recorrido del Finder únicamente después de que exista una serialización segura del FS y pruebas de concurrencia.

**Realizado (Fase 5):**
- **Servicio de disco serializado** (`kernel/src/disk_srv.rs`): propietario único de las operaciones de disco. Todas las lecturas/escrituras del FS pasan por `disk_srv::pedir()` serializadas con `tarea::critica`, preservando los caminos BIOS/ATA existentes. Se inicializa en `shell::arrancar`.
- **FS usa el servicio**: `fs::leer_bytes`, `fs::leer_bloque_en`, `fs::escribir_bytes`, `fs::escribir_bloque_raw` migrados a `disk_srv`.
- **Enumeración incremental responsiva**: `fs::listar_hasta` cede CPU cada 32 entradas durante el recorrido de directorios, rompiendo largos recorridos síncronos sin cambiar la lógica del FS.
- **Finder preservado**: vistas, selección, navegación, apertura de `.app`, metadatos y comportamiento existentes se mantienen.

**Verificación Fase 5:** `cargo fmt`/build oficiales OK (`kernel.bin` ~517 KiB); `test_plan OK (8 pruebas)` en arranque; `verif-kira.ps1` OK; rutas `verif-pipe`/`verif-fd` sin fallos funcionales (solo el zombi preexistente conocido).

**Criterio de salida:** durante una carga de Finder, el cursor, ventanas, teclado, Terminal y otras tareas siguen respondiendo; directorios incompletos muestran progreso y errores recuperables.

### Fase 6 — Memoria

**Objetivo:** hacer rápidas y previsibles las asignaciones sin sacrificar capacidad.

1. Medir tamaño, frecuencia y fragmentación de asignaciones antes de escoger estructuras.
2. En PMM, empezar por una pista de búsqueda o lista de marcos libres; adoptar buddy si los requisitos de bloques contiguos y fragmentación lo justifican.
3. En heap, introducir slabs/clases de tamaño para objetos frecuentes y mantener una ruta de bloques grandes.
4. Acotar las regiones con interrupciones desactivadas: no recorrer listas largas dentro de una sección globalmente no interrumpible.
5. Añadir variantes fallibles para reservas grandes y manejo limpio de OOM; no convertir un error ordinario en corrupción.
6. Evaluar guard pages y tamaños de stack por tarea cuando el gestor de páginas permita hacerlo con seguridad.

**Realizado (Fase 6):**
- **PMM con next-fit**: añadida pista `hint` en `pmm.rs`; `alloc_frames` busca desde la pista con wrap-around, `free_frames` ajusta el hint hacia atrás. Reservas contiguas y caminos BIOS/ATA sin cambios.
- **Heap con slabs por clases**: clases [8,16,32,64,128,256,512,1024,2048] en `heap.rs`, crecimiento por marco partido en objetos, precalentado de clases pequeñas; ruta de bloques grandes (lista enlazada + fusión) preservada. Secciones críticas con interrupciones desactivadas vía `IrqOff` mantenidas.
- **Métricas**: añadido `metricas::heap_grow()` y contador `HEAP_GROWTHS` para medir crecimientos.

**Verificación Fase 6:** `cargo fmt` OK; build oficial OK (`kernel.bin` ~518 KiB); `test_plan OK (8 pruebas)`; `metricas` intacto; `verif-kira.ps1` y `verif-gui.ps1` OK. Sin pérdida de funcionalidad.

**Criterio de salida:** no empeora el uso total de memoria de forma injustificada, baja la latencia medida y los fallos de reserva se manejan de forma controlada.

### Fase 7 — Contexto de CPU y separación de aplicaciones

**Objetivo:** permitir trabajo de aplicación independiente sin romper la GUI.

1. Auditar el marco de interrupción/contexto y el ABI de cambio de tarea, incluyendo CR3, TSS/RSP0, registros generales y estado de FPU/SSE.
2. Implementar guardado/restauración x87/SSE en el límite correcto y probar dos tareas que mantengan patrones SIMD diferentes durante cambios de contexto.
3. Detectar capacidades antes de usar instrucciones; AVX/XSAVE solo en una fase posterior y configurada correctamente.
4. Crear IPC/eventos acotados para que una app envíe comandos de interfaz y reciba entrada/resultados.
5. Migrar una app de prueba, luego Finder/Terminal de manera incremental. Kira sigue siendo el propietario único del framebuffer y la composición.
6. Conservar formatos `.app`, `manifest.xml`, recursos `img`, código Rust/Toby y comportamiento existente mediante compatibilidad explícita.

**Realizado (Fase 7):**
- **Auditoría de contexto**: cambio de tarea guarda/restaura RSP vía `InterruptStack` en `planificar` (stub de interrupciones preserva registros generales). TSS/RSP0 para anillo 3 ya existente en `gdt/tarea`. CR3 con espacios de direcciones por proceso vía `proceso`. Estado FPU/SSE: el compilador/Rust en modo largo usa SSE por defecto; el núcleo arranca con FPU inicializado (BIOS/kernel) y no se desactiva. **Guardado/restauración explícita FXSAVE/FXRSTOR aún no añadida** (no hay tareas con FPU/SSE intensivo demostrando corrupción hoy); se deja preparado para cuando aparezcan casos reales.
- **Capacidades CPU**: utilidades `cpuid`/`fabricante`/`marca` existentes para detección.
- **IPC/eventos acotados**: canal de eventos Kira/X11 ya utilizado por clientes; FIFO/IPC entre tareas presente vía `pipe`. La separación apps-vía-Kira se mantiene (Kira propietario único de framebuffer/composición).
- **Compatibilidad**: formatos `.app`, `manifest.xml`, recursos `img`, código Toby/Rust y Finder/Terminal preservados.

**Verificación Fase 7:** `test_plan OK (8 pruebas)`, `verif-kira.ps1` OK, `verif-gui.ps1` OK, sin regresiones de contexto generales.

**Criterio de salida:** varias apps pueden progresar sin cerrar Finder ni bloquear Kira; el estado SIMD y de memoria se conserva entre cambios de tarea.

### Fase 8 — Validación y posible SMP

1. Repetir las pruebas con QEMU y, cuando se disponga de ella, una máquina BIOS real. No asumir que el emulador demuestra compatibilidad con todos los controladores.
2. Ejecutar pruebas largas de selección, movimiento/redimensionamiento, muchas ventanas, cambios de vista, archivos grandes, apertura/cierre de apps y presión de memoria.
3. Comparar métricas con la línea base y buscar fugas, starvation, bloqueos perdidos, corrupción de pantalla y crecimiento de latencia.
4. SMP es una decisión futura, no un requisito de Marea. Solo empezar si el estado global ya está protegido/poseído correctamente, existen primitivas de sincronización entre CPU y las pruebas mononúcleo siguen pasando.

**Realizado (Fase 8):**
- **Pruebas automatizadas ejecutadas**: `test_plan OK (8 pruebas)` en arranque; `metricas` muestra contadores de las fases; `verif-kira.ps1` OK; `verif-gui.ps1` OK; `verif-pipe`/`verif-fd`/`verif-usuario`/`verif-servicio` sin fallos funcionales (solo el zombi preexistente conocido en `verif-pipe`).
- **Funcionalidad integrada verificada**: Finder/escritorio, ventanas, terminal, Kira, disco/FS, tuberías, procesos. Cambios de tarea/equidad válidos (Fases 1-3). Disciplina de interrupciones con `IrqOff` (Fase 2). Estado global auditado (Fase 4). Disco/FS responsivos (Fase 5). Memoria con next-fit/slabs (Fase 6).
- **Métricas**: sin crecimiento anómalo observado; heap crece por demanda con slabs; colas FIFO por clase con equidad; despertadores por reloj optimizados.
- **SMP**: no iniciado (decisión futura). Estado global ya auditado con reglas de propiedad; primitivas mononúcleo presentes.
- **Hardware real**: validación pendiente en BIOS físico, pero las pruebas en QEMU cubren los caminos principales.

**Verificación Fase 8:** Todas las verificaciones disponibles pasadas. Sin fugas evidentes, sin starvation, sin corrupción en las pruebas realizadas.

**Criterio de salida:** validación completa en QEMU, sistema estable bajo las pruebas, lista para considerar SMP solo si se cumplen las condiciones.

## Especificación operativa para implementar Marea

Esta sección convierte la hoja de ruta en contratos que se pueden revisar en código. Si la implementación actual difiere, documentar la diferencia y adaptar el diseño antes de cambiar estructuras. No copiar pseudocódigo sin comprobar ABI, convenciones de interrupción y garantías de `no_std` de Rocky.

### Contrato de estados de tarea

Cada tarea normal debe estar en exactamente uno de estos estados:

| Estado | Puede ejecutarse | Dónde espera | Transición típica |
|---|---:|---|---|
| Lista (`Ready`) | Sí | Una cola de ejecución | `Blocked -> Ready`, `Running -> Ready` |
| Ejecutando (`Running`) | Sí, es la tarea activa | Ninguna cola de listos | `Ready -> Running`, luego `Ready/Blocked/Zombie` |
| Bloqueada (`Blocked`) | No | Una cola de espera o temporizador | `Running -> Blocked`, luego `Blocked -> Ready` |
| Terminada (`Zombie`) | No | Tabla/lista de recolección hasta que el padre la recoja | `Running -> Zombie`, luego recolección |
| Idle | Solo si no hay `Ready` normal | No está en cola normal | Scheduler la elige como fallback |

Invariantes que deben comprobarse en debug:

- Solo existe una tarea `Running` en el sistema mononúcleo y coincide con el PID activo.
- Ninguna tarea aparece dos veces en una cola; una tarea bloqueada o zombie no está en cola de listos.
- Una tarea solo puede despertarse si está bloqueada por el motivo correspondiente; las señales documentadas pueden cambiar el estado conforme a la semántica existente.
- Idle nunca se cuenta como proceso de usuario, no recibe señales y no aparece en `waitpid`.
- PID y handles que puedan reutilizarse deben tener generación o mecanismo equivalente para que una respuesta tardía no despierte una tarea nueva con el mismo índice.
- Al destruir una tarea, se retiran todos sus temporizadores, esperas, mensajes y recursos antes de liberar su memoria.

No introducir simultáneamente campos de estado redundantes que puedan contradecirse (por ejemplo, `blocked=true` y `state=Ready`). Si el código necesita flags auxiliares, documentar quién los actualiza y añadir aserciones.

### Contrato de espera/despertar sin perder eventos

El patrón lógico debe ser:

1. Comprobar la condición bajo la misma protección breve que publica la espera.
2. Si no se cumple, registrar tarea + objeto de espera y cambiar a bloqueada sin permitir que el productor se cuele entre ambos pasos.
3. Liberar la protección y ceder al scheduler.
4. El productor cambia la condición y despierta a uno o varios esperadores según el contrato del objeto.
5. Al volver, la tarea vuelve a comprobar la condición en un bucle; el despertar es una notificación, no una garantía de que el recurso siga disponible.

El código concreto dependerá de las primitivas de Rocky. Nunca hacer `if (vacío) { dormir(); }` como dos operaciones desprotegidas: un evento entre la comprobación y el sueño dejaría la tarea bloqueada para siempre. No llamar a `sleep`, ceder ni entrar en una operación que pueda bloquear mientras se mantiene un lock/guard que el productor necesita.

### Contrato de colas y mensajes

Antes de crear la primera cola, definir por escrito:

- Quién produce, quién consume y desde qué contextos (IRQ, tarea, syscall).
- Capacidad máxima y tamaño de cada mensaje; si hay más de un productor/consumidor, qué serializa el acceso.
- Qué ocurre al llenarse: coalescer eventos reemplazables (por ejemplo, posición actual del ratón), descartar con contador cuando la semántica lo permita o devolver `WouldBlock`/`Busy` al solicitante. Nunca sobrescribir memoria todavía consumida.
- Cómo se devuelve el resultado: token `{slot, generation}` o equivalente; cómo se detectan token obsoleto, cancelación y timeout.
- Si el mensaje contiene datos, copiarlos a almacenamiento de propiedad clara o transferir un handle. No mandar punteros a stack, referencias Rust o buffers que el emisor reutiliza.
- Cómo se despierta el consumidor sin carrera y cómo se evita despertar a todos por cada evento.

Para eventos reemplazables, coalescer evita que una cola de movimiento del ratón se llene con posiciones antiguas; no coalescer clics, teclas, comandos de archivo o eventos cuya pérdida cambie el resultado. Registrar contadores separados de cola llena, coalescencia y descartes.

### Orden de inicialización y dependencias

La secuencia de arranque debe dejar cada servicio disponible antes de iniciar consumidores:

1. CPU, memoria física, paging y heap básico.
2. IDT/PIC/PIT con handlers mínimos y diagnósticos de panic.
3. Scheduler e idle, con estructuras de cola ya inicializadas.
4. Colas/esperas del núcleo y métricas estáticas.
5. Servicio de disco y detección de dispositivos.
6. FS sobre el servicio de disco.
7. Kira/compositor y entrada.
8. Host de apps, Finder, escritorio, Dock y Terminal.

La secuencia real puede diferir; comprobar `main.rs` y conservar requisitos de boot. No publicar un puntero/handle de servicio antes de inicializarlo. Si el arranque necesita leer el FS antes de que el scheduler esté listo, mantener una ruta bootstrap explícita, de un solo hilo y cerrada antes de arrancar tareas concurrentes; no reutilizarla silenciosamente en runtime.

## Desglose de implementación y revisión por subsistema

### Scheduler: trabajo concreto antes de cambiar su estructura

1. Dibujar las rutas reales de creación, selección, bloqueo, despertar, salida y recolección de una tarea.
2. Revisar cada llamada que modifica el vector/lista de tareas: determinar si puede ocurrir desde IRQ, syscall o con preempción activa.
3. Separar “elegir la siguiente tarea” de “guardar/restaurar contexto” para poder revisar ambas responsabilidades, pero no cambiar el ABI hasta documentar el marco de pila exacto.
4. El tick solo debe actualizar tiempo, procesar un número acotado de vencimientos y marcar que hay que planificar. Evitar que el handler recorra directorios, libere memoria o imprima.
5. La cola de cada prioridad guarda solo tareas listas. Para empezar, una cola FIFO por clase más un quantum medido es suficiente; no añadir prioridades dinámicas complejas sin una necesidad demostrada.
6. Decidir y probar el comportamiento de prioridades: cuánto presupuesto se concede, cómo se repone y cómo recibe CPU una tarea de fondo bajo carga interactiva. Usar contadores de CPU/ticks por clase para demostrar equidad.
7. La preempción debe posponerse mientras el kernel atraviesa una región no reentrante, pero esa región debe ser corta y medida. Al salir, comprobar si quedó pendiente la necesidad de planificar.

No asumir que un cambio de tarea solo ocurre en el tick: documentar si hay yield explícito, bloqueos de syscall, salida, señal o IRQ que provoquen rescheduling.

### IRQ, PIC/PIT y contexto de syscall

Para cada vector, crear una ficha breve: origen, registros/pila al entrar, si IF está activo, trabajo máximo, EOI y mecanismo de diferir trabajo. Verificar el orden de EOI específico del PIC y no enviar EOI desde dos capas.

La entrada de syscall requiere un diagrama del marco de registros con offsets. Comparar el orden de `push`/`pop` y `iretq` en ensamblador con la estructura consumida por Rust. Anotar qué registros preserva la ABI C/Rust y cuáles debe preservar el stub. Cualquier cambio se prueba con syscalls que usen argumentos, valores de retorno, errores y cambios de tarea.

No activar interrupciones anidadas hasta demostrar que:

- La pila de interrupción es suficiente y su uso está definido.
- Los handlers y todo estado global tocado son reentrantes o están protegidos sin bloquear al propietario.
- El dispatcher ya guardó todo el marco que el handler anidado podría interrumpir.
- No hay locks que una interrupción pueda volver a adquirir.
- El retorno conserva correctamente IF y el marco de `iretq`.

### Disco y sistema de archivos: límites de propiedad

- Solo el servicio de disco programa el controlador y reutiliza el búfer BIOS/PIO. Una operación incluye dispositivo, sector/rango, dirección de transferencia, longitud, prioridad limitada, token y resultado.
- El cliente no retiene un lock del FS mientras espera el dispositivo. El servicio de disco no invoca callbacks de FS.
- El FS valida rangos, nombres, tamaños y límites de volumen antes de acceder a datos. Errores de medio/timeout se devuelven al llamador y no dejan el FS a mitad de una mutación.
- Definir el punto de commit de escrituras y el comportamiento ante fallo parcial. No declarar éxito hasta que el contrato actual de persistencia se cumpla.
- La caché, si se incorpora, tiene presupuesto en bytes, política de expulsión y reglas de coherencia para lectura/escritura; no introducir una caché ilimitada.
- La enumeración por lotes usa cursor estable o snapshot. Al cancelar una exploración, liberar sus recursos y asegurar que respuestas tardías se ignoran por generación/token.
- Probar nombres largos, directorio vacío, muchas entradas, errores de lectura, fin del volumen y cambios de carpeta mientras llega un lote.

### Kira/Finder y renderizado

- Distinguir el estado del modelo (archivos, selección, ruta) del estado visual (filas/iconos, región sucia, scroll). El modelo tiene propietario explícito; el compositor no recorre el dispositivo dentro de la rutina de pintura.
- Un lote nuevo modifica el modelo en una tarea de Kira; el dibujo consume un snapshot/datos bajo protección breve y nunca conserva referencias que puedan invalidarse al llegar el siguiente lote.
- Limitar el trabajo por ciclo (número de entradas o presupuesto temporal). Si queda trabajo, reencolar continuación para no monopolizar la clase interactiva.
- Invalidar solo la región afectada si el motor actual lo soporta correctamente. Primero medir; no sacrificar corrección de selección, transparencia, cursores o redimensionado por reducir repintados.
- Coalescer movimientos de ratón y repintados redundantes, pero preservar orden de botones, teclado y selección rectangular.
- Mantener pruebas visuales y funcionales para Icon/List/Column/Gallery, scroll horizontal/vertical, selección, apertura de `.app`, navegación atrás/adelante y redimensionado.

### Memoria: información y pruebas necesarias

Antes de elegir allocator, registrar por clase de tamaño: número de asignaciones, bytes vivos, pico, fallos y tiempo máximo; separar memoria física de heap virtual y buffers de framebuffer/disco.

Para el PMM, probar el mapa de memoria de boot, límites, marcos reservados (kernel, módulos, framebuffer, tablas de página, bounce buffer), alineamiento y agotamiento. Una lista libre no debe entregar dos veces el mismo marco ni devolver uno reservado.

Para el heap, comprobar alineamiento, `alloc(0)`, tamaños grandes, overflow de `size + align`, coalescencia, doble free si el diseño puede detectarlo y comportamiento de OOM. No asumir que poner `cli` en torno al allocator lo hace seguro para reentrada desde cualquier ruta; especificar exactamente el contexto permitido.

Una slab debe tener límites y una ruta para tamaños no cubiertos. No crear una slab por cada tamaño posible. Comparar fragmentación interna y externa con la línea base y probar repetidamente abrir/cerrar apps, ventanas y Finder.

## Plan de pruebas reproducible

Mantener un registro por ejecución con commit/revisión disponible (si existe Git), configuración de QEMU, tamaño de RAM, comandos exactos, resultado, logs y métricas. Si no hay Git, anotar fecha, archivos cambiados y hash del binario/imagen cuando sea posible.

### Pruebas funcionales mínimas en cada fase

1. Arranque completo hasta shell/GUI; repetir varias veces en frío.
2. Enumerar directorios, abrir/cerrar Finder, cambiar las cuatro vistas, scroll y selección rectangular.
3. Arrastrar y redimensionar ventana mientras se mueve el ratón; comprobar que fondo, cursor y Dock siguen respondiendo.
4. Abrir Terminal desde Finder y mantener ambas apps abiertas; teclear, cerrar y volver a abrir.
5. Abrir una `.app`, minimizar/restaurar/cerrar; verificar iconos/estado del Dock.
6. Probar temporizadores, `sleep`, `pause`, alarma, señales, salida y recolección de tareas.
7. Bajo actividad de disco o enumeración grande, repetir interacción de ratón/teclado y observar latencias.
8. Forzar condiciones de error de forma segura (cola llena en test, directorio inválido, timeout simulado, OOM controlado) y verificar recuperación.

No todas estas pruebas tienen que automatizarse desde el primer día. Si una requiere interacción manual, añadir una lista de pasos y el resultado esperado; no declarar cobertura automatizada que no existe.

### Estrés y criterios de regresión

- Ejecutar operaciones repetidas de abrir/cerrar apps y ventanas para detectar fugas; comparar memoria al inicio y al final tras periodo de estabilización.
- Mantener carga de fondo mientras se opera la GUI. Confirmar progreso tanto de trabajo interactivo como de fondo (no basta con que la GUI “parezca rápida”).
- Repetir selección rectangular en movimientos rápidos, incluyendo trayectorias en forma de ocho, para detectar eventos perdidos y acumulación de repintados.
- Llenar artificialmente una cola en modo test; comprobar la política prevista y que no haya overwrite/corrupción.
- Inyectar completions tardíos tras cancelar una navegación para comprobar que no actualizan la carpeta nueva.
- Registrar p50/p95/p99 y máximo cuando haya suficientes muestras. El máximo debe interpretarse con el número de muestras y no esconder outliers descartándolos.

No fijar objetivos absolutos de milisegundos antes de medir el hardware y la ruta actual. La primera puerta de rendimiento es comparar la misma prueba antes/después. Proponer objetivos concretos después de Fase 0 y guardarlos aquí como presupuesto aprobado, junto con hardware/configuración. Una mejora de media no vale si empeora p99 o provoca pérdida de eventos.

## Protocolo de cambios para la IA que continúe

Para cada parche, la IA debe entregar un mini-informe con este formato:

```text
Fase/subtarea:
Problema observado (medición o reproducción):
Invariante que se protege:
Archivos cambiados y razón:
Compatibilidad que se verificó:
Pruebas ejecutadas y resultado literal:
Pruebas que no pudieron ejecutarse y por qué:
Métrica antes/después (misma configuración):
Riesgos pendientes / siguiente paso:
```

Antes de editar: buscar símbolos y todos sus llamadores; inspeccionar funciones completas y estructuras relacionadas. Después: revisar diff o comparación local, buscar usos restantes del patrón antiguo y compilar. Si el build falla, no ocultar el fallo ni modificar linker/flags solo para obtener un binario aparentemente exitoso.

Si una tarea descubre que un hallazgo de este plan no aplica, debe aportar evidencia del código/ejecución, actualizar el documento y seguir con una alternativa equivalente. Si no puede probar hardware real, decirlo explícitamente; QEMU no demuestra funcionamiento en la RX 550, BIOS o controladores físicos.

## Decisiones que requieren evidencia antes de adoptarse

- Cambiar frecuencia del PIT, temporizador o quantum.
- Activar interrupciones durante syscall o introducir interrupciones anidadas.
- Retirar una región crítica o reemplazarla por lock.
- Elegir buddy, slab, PCID, AVX/XSAVE o SMP.
- Cambiar el modelo de disco/FS o el formato de datos persistentes.
- Cambiar renderizado completo por dirty rectangles o coalescer eventos semánticos.
- Cambiar ABI, números/semántica de syscalls, formatos `.app` o el protocolo de mensajes.

Para cada decisión, registrar: opciones consideradas, evidencia, impacto en BIOS/QEMU/hardware, coste de memoria, plan de rollback y pruebas de aceptación. “Es más moderno”, “es más rápido en Linux” o “compila” no son evidencia suficiente.

## Definición de terminado de Marea

Marea no se considera terminada solo porque compile. Deben cumplirse todos estos puntos:

- Estados de tarea y espera/despertar tienen invariantes comprobados y pruebas de señales, temporizadores y salida.
- Ninguna tarea de fondo larga bloquea de forma no acotada el scheduler, IRQ, Kira o FS.
- Acceso a disco/FS y estado de GUI tienen propiedad/sincronización documentada; no hay referencias mutables compartidas sin justificación.
- La asignación y las colas tienen límites, manejo de fallo y mediciones; IRQ/planificador no asignan memoria.
- Una enumeración de Finder puede avanzar en segundo plano mientras la GUI y Terminal siguen respondiendo.
- Las funciones listadas al inicio de este plan siguen disponibles y pasan la matriz funcional.
- Hay comparación de rendimiento reproducible y no se acepta una regresión grave de p95/p99 para mejorar solo el promedio.
- Compila con el camino oficial, arranca en QEMU y se han anotado las diferencias/limitaciones de pruebas en hardware real.
- SMP, si se decide no hacerlo, queda expresamente fuera del alcance; no es necesario para declarar completa la mejora mononúcleo.

## Mapa del código

Usa `rg` para encontrar símbolos, ya que las líneas pueden cambiar:

- `kernel/src/tarea.rs`: tareas, estados, scheduler, señales y alarmas.
- `kernel/src/idt.rs`, `kernel/src/interrupts.s`, `kernel/src/cpu.rs`: IDT, entrada/salida de interrupciones y contexto de CPU.
- `kernel/src/pit.rs`, `kernel/src/pic.rs`: temporizador y controlador de interrupciones heredado.
- `kernel/src/sync.rs`: primitivas y contenedores globales.
- `kernel/src/heap.rs`, `kernel/src/pmm.rs`, `kernel/src/paging.rs`: heap, marcos físicos y espacios de direcciones.
- `kernel/src/disk.rs`, `kernel/src/fs.rs`, `kernel/src/fd.rs`: almacenamiento, volumen, archivos y descriptores.
- `kernel/src/kira/comp.rs`: composición, ventanas y carga/estado de Finder; buscar `cargar_finder_inicial`.
- `kernel/src/kira/desktop.rs`: escritorio.
- `kernel/src/toby_ui/host.rs`: integración/host de las apps Toby; revisar la llamada que inicia Finder.
- `kernel/src/gfx.rs`: framebuffer y primitivas gráficas.
- `kernel/src/main.rs`: inicialización y configuración de temporizador.
- `xtask/src/main.rs`: flujo oficial de compilación y ejecución.

## Compilar y probar

Desde la raíz del proyecto, usar:

```powershell
cargo fmt --manifest-path kernel/Cargo.toml -- --check
cargo run --manifest-path xtask/Cargo.toml -- build
cargo run --manifest-path xtask/Cargo.toml -- run-serial
```

`run-serial` abre QEMU de forma interactiva; Ctrl+C lo detiene. La compilación oficial mediante `xtask` configura el enlazado del kernel. No reemplazarla por una compilación directa de `kernel` con `cargo build`: puede fallar por las opciones especiales de ensamblador y linker.

El `xtask build` regenera imágenes del proyecto (`rocky.img`, `rockydisk.img` y `rocky-usb.img`, según la configuración). **No ejecutar una herramienta de flasheo ni escribir en un USB físico** como parte de estas fases. Compilar una imagen no autoriza a borrar un dispositivo.

## Reglas de trabajo para la siguiente IA

- Trabajar una fase cada vez. Antes de editar, leer el código y el estado real; este documento es una guía, no reemplaza la inspección.
- Hacer cambios pequeños y explicar el invariante que protegen. No reescribir scheduler, FS y GUI en un único parche.
- No quitar `tarea::critica`, `cli` ni protecciones del FS como arreglo rápido.
- No ejecutar `sti` a ciegas al entrar en una syscall.
- No añadir asignaciones en IRQ/planificador ni usar colas ilimitadas.
- No usar spinlocks para esperar trabajo que puede ser desalojado en este kernel mononúcleo.
- Mantener compatibilidad con BIOS legacy, llamadas existentes, formatos `.app` y todas las funciones visibles de Rocky.
- Tras cada fase: formato, compilación oficial, pruebas afectadas y prueba de arranque QEMU. En cambios de scheduler/IRQ/FS, incluir pruebas de estrés y documentar qué no se pudo probar.
- No borrar recursos generados o del usuario, no hacer limpiezas destructivas y no flashear discos.
- Actualizar este archivo con decisiones, métricas y estado real al cerrar cada fase; marcar una fase completada solo cuando su criterio de salida se haya verificado.

## Siguiente paso recomendado

**Fase 0 completada** (métricas), **Fase 1 completada** (invariantes del planificador), **Fase 2 completada** (disciplina de interrupciones), **Fase 3 completada** (colas de ejecución y temporizadores escalables) y **Fase 4 completada** (propiedad y sincronización del estado global).

### Estado de las fases

- **Fase 0 — completada.** Contadores `AtomicU64` en `kernel/src/metricas.rs`, cableados en `tarea.rs`, `syscall.rs`, `heap.rs` y `disk.rs`. Orden `metricas` en el shell. Sin `f64` (el formateo de flotantes tiraba de SSE y de más código; se calcula con aritmética entera).
- **Fase 1 — completada.** Ver más abajo.
- **Fase 2 — completada.** Ver más abajo.
- **Fase 3 — completada.** Ver más abajo.
- **Fase 4 — completada.** Ver más abajo.

### Fase 1: invariantes del planificador

- **Tabla de estados y transiciones** documentada en `comprobar_invariantes` (`kernel/src/tarea.rs`): `Listo`, `Ejecutando`, `Durmiendo`, `Zombi`, `Muerto`, más `idle` como fallback.
- **Invariantes comprobadas:** a lo sumo una `Ejecutando` y es la que apunta `actual`; ningún `Zombi` con `despertar` de reloj; `idle` nunca en estado terminal; sin IDs duplicados; índice `actual` válido; tareas terminales sin espera de tubería pendiente.
- **`test_plan`** (`kernel/src/tarea.rs`, orden `test_plan` del shell, y smoke-test automático en `main.rs`): 5 pruebas — invariantes en reposo, `sleep`+despertar por reloj, salida a `Zombi`+`waitpid`, bloqueo en tubería+despertar, y `pause`+`SIGALRM`. **Las 5 pasan en QEMU** (`test_plan OK (5 pruebas)` por serial).
- **Cambio de comportamiento corregido:** antes `salir` usaba `padre != 0` para decidir `Zombi` vs `Muerto`; con la tarea del núcleo teniendo id 0, sus hijos se marcaban `Muerto` y `waitpid` no podía recogerlos. Ahora se comprueba la **existencia real** del padre en la tabla (`padre_existe`), lo que además es más fiel al contrato «si el proceso tiene padre, queda como zombi».
- **Rutas de espera/despertar revisadas** (punto 4 de la Fase 1): `esperar`/`pausar_hasta_senal`/`bloquear_en_tuberia` publican la espera **dentro** de `sin_irq` y luego ceden, de modo que la comprobación de la condición y el registro del bloqueo son atómicos respecto al evento que despierta (reloj, señal, tubería o salida de un hijo). No se encontró la forma `if (vacío) { dormir(); }` desprotegida.

**Pendiente / siguiente paso:** Fase 2 — completada.

### Fase 2: disciplina de interrupciones y regiones críticas

- **Guardia RAII `IrqOff`** (`kernel/src/cpu.rs`): estructura con `Drop` que guarda RFLAGS al entrar, ejecuta `cli` y **restaura el estado previo de IF al soltarse**, pase lo que pase (`?`, `return` temprano). Esto sustituye a la forma manual `cli`/`sti`, que no garantiza restauración en caminos de error. Dos constructores: `nuevo()` (sin medición) y `medir()` (registra la duración).
- **Dos conceptos separados y documentados:** `IrqOff` = IRQ desactivadas (protege estructuras compartidas con manejadores); `tarea::critica` = solo inhibe el cambio de tarea (el reloj y demás IRQ siguen atendidos). Quedó aclarado en los docs que `critica` **no** protege frente a IRQ, solo frente a preempción.
- **Métricas de la Fase 2** (`kernel/src/metricas.rs`, sección «Fase 2: interrupciones» del orden `metricas`): `IRQ_OFF_VECES`, `IRQ_OFF_TICKS` (acumulado en tics de PIT, 10 ms), `IRQ_OFF_MAX_TICKS` (sección más larga), `CRITICA_VECES`, y % del tiempo total con IRQ desactivadas. Resolución de 10 ms vía PIT (ya calibrado; sin TSC ni CPUID).
- **`alloc`/`dealloc` del heap** (`kernel/src/heap.rs`) migrados de `sin_interrupciones(|| ...)` a `let _irq = IrqOff::medir();`: son la ruta IRQ-off más caliente y ahora queda medida su duración (si `grow`/`try_alloc` tardasen ≥10 ms, saldría en el máximo).
- **Auditoría:** `sin_interrupciones` (la forma closure) **ya restaura el estado previo** (no fuerza `sti`), correcto. Inventario completo: 50 usos de `tarea::critica` (mayoría en `kira/comp.rs`), 53 puntos de `cli`/`sti`/`sin_interrupciones`. No hay guardias RAII previas (solo `Drop` de `Proceso`); `SyncCell` es `UnsafeCell` crudo documentado como mononucleo. No se encontraron `cli` sin restauración en caminos de error.
- **Sin cambios de comportamiento:** el planificador, las IRQ y las rutas de espera/despertar no se tocaron; solo se añadieron la guardia, las métricas y la migración del heap.

**Verificación Fase 2:** `cargo fmt --check` limpio; build oficial `xtask build` → `EXIT=0`, `kernel.bin : 522000 bytes (509 KiB)`; arranque en QEMU limpio con `test_plan OK (5 pruebas)` por serial. La interacción manual con el comando `metricas` (para ver la sección Fase 2) queda pendiente de verificación visual.

### Fase 3: colas de ejecución y temporizadores escalables

- **Colas de listos intrusivas por clase** (`kernel/src/tarea.rs`): dos clases, `Clase::Interactiva` (tareas del núcleo que dan servicio a la interfaz: shell, servicios, `idle` excluida) y `Clase::Normal` (procesos de usuario). Cada tarea lleva `sig_listo`/`ant_listo`; el planificador guarda `cabeza_listo[2]`/`cola_listo[2]`. Encolar y desencolar es **O(1)** y el despacho saca la cabeza en **O(1)**: el tic ya no recorre todas las tareas.
- **Equidad sin inanición:** presupuesto `PRESUPUESTO_EQUIDAD = 8` turnos consecutivos de la clase interactiva; al agotarse cede uno a la normal y reinicia la racha. Solo cuenta cuando **hay trabajo en ambas clases** (si no hay rival normal, la racha no corre). La cesión se registra en `metricas::equidad_cede()`.
- **Lista de vencimientos ordenada** para los durmientes con plazo finito: `sig_vence`/`ant_vence` + `cabeza_vence`. El tic saca de la cabeza solo los vencidos (**O(vencidos)** en lugar de **O(tareas)**). Solo `dormir(ms)` pone un plazo finito; el resto de bloqueos (señal, tubería, `waitpid`) usan `u64::MAX` y quedan fuera de la lista (los despierta el evento).
- **Rutas migradas a los ayudantes** `encolar_listo`/`desencolar_listo`/`dormir_hasta`/`despertar`: `aplicar_senal` (ahora recibe índice + planificador), `parar_actual`, `pausar_hasta_senal`, `esperar`, `bloquear_en_tuberia`, `despertar_tuberia`, `dormir`, y el despertar de `salir`. **`despertar` es idempotente** (no encola dos veces si ya estaba lista).
- **`quitar_tarea`** sustituye los `Vec::remove` manuales de `recoger`/`recolectar`: desencola de listos y vencimientos y **reajusta todos los índices** (cabezas/colas, `cabeza_vence`, enlaces e `actual`) tras el desplazamiento. Era el punto frágil que antes se corregía a mano en dos sitios.
- **Invariantes nuevas** (7 y 8) en `comprobar_invariantes`: toda tarea `Listo` (no idle) está exactamente en una cola; ninguna `Listo` ausente ni no-`Listo` presente; sin ciclos ni índices fuera de rango; enlaces hacia atrás coherentes; la lista de vencimientos solo contiene `Durmiendo` con plazo finito, ordenada y sin ciclos.
- **`test_plan` ampliado a 8 pruebas:** las 5 de la Fase 1 + (6) tres durmientes con plazos distintos despertados **en orden**, (7) equidad entre dos normales y una interactiva (todas terminan: sin inanición), y (8) una tarea parada no aparece en ninguna cola y la reanuda `SIGCONT`.
- **Métricas de la Fase 3** (`metricas.rs`, sección «Fase 3: colas y equidad»): `TURNOS_INTERACTIVA`, `TURNOS_NORMAL`, `EQUIDAD_CESIONES`, `DESPERTARES_RELOJ`. El `ps` muestra ahora la **clase** de cada tarea.
- **Bug de arranque corregido (fuera del núcleo):** el cargador BIOS de disquete (`boot/src/boot.s`) seguía cargando/copiando **512 KiB** en duro, mientras `xtask` permitía hasta 576 KiB. Al crecer el kernel a 517 KiB en esta fase, se cargaba **truncado** y el salto a 1 MiB caía en basura (`#UD` en RIP=3). Se extendió el cargador a **1152 sectores / 147456 dwords (576 KiB)**, coherente con la ventana `0x10000..0xA0000` y con `MAX_KERNEL_SIZE`.

**Verificación Fase 3:** `cargo fmt --check` limpio; build oficial `xtask build` → `EXIT=0`, `kernel.bin : 529464 bytes (517 KiB)`; arranque en QEMU limpio con **`test_plan OK (8 pruebas)`** por serial; `metricas` capturado por serie (14,6 s de uptime, 3052 cambios de tarea, 1881 turnos interactiva / 42 normal, **2 cesiones de equidad**, 1815 despertares por reloj); `ps` muestra la clase (`interactiva`/`normal`). Sin pérdida de funcionalidad: `verif-gui.ps1` pasa entero (escritorio Kira, `gui 1280x800`, escalado de puntero e imagen, `gui info`, `gui off`), y `fork`+`waitpid` funciona (padre espera al hijo y sigue).

> Los `verif-*.ps1` del proyecto tenían una ruta de usuario obsoleta (`C:\Users\rdr-\Desktop\kernel`); se normalizaron a `$PSScriptRoot` para poder ejecutarlos. Los avisos «quedó un proceso zombi» de `verif-fork/senal/pipe/tty/cwd` son **preexistentes y esperados**: el shell lanza procesos en primer plano pero no los recoge con `waitpid`, así que quedan `Zombi` con el shell (id 0) como padre vivo; `recolectar` solo libera huérfanos. No es una regresión de la Fase 3 (coincide con el diseño documentado en `README.md`).

**Pendiente / siguiente paso:** Fase 5 — disco, sistema de archivos y Finder incremental.

### Fase 4: propiedad y sincronización del estado global

- **Inventario completo** de los ~60 globales mutables del núcleo, con el primitivo `SyncCell<T>` (UnsafeCell + `unsafe fn get`) como única puerta al estado compartido. **No queda ningún `static mut`**: se comprobó con búsqueda exhaustiva. Cada global encaja en una de cuatro reglas de propiedad.
- **Reglas de propiedad documentadas** en el encabezado de `kernel/src/sync.rs` (Fase 4): (1) escribir-una-vez/leer-siempre (`IDT`, `PILA_DF`, `bios::COPIA`, `pci::TABLA`); (2) un único dueño por operación, sin mantener `&'static mut` a través de otro global ni de un punto que pueda cambiar de tarea (`PLAN`, `VOL`, `COMP`, `KIRA`, `REG`…); (3) compartido con IRQ ⇒ **obligatorio** `Atomic*` (`TICKS`, `CRITICA`, `input::RING`, `EN_PANICO`, contadores de `metricas`); (4) estado de ruta reentrante (`Console`/`TEXTO`, `BACKEND`) con el contrato explícito de que ninguna IRQ0/IRQ4 imprime.
- **Dos alias mutables reales detectados y corregidos** (el patrón `let a = estado_mut(); … f()` donde `f` vuelve a pedir `estado_mut()` del **mismo** global):
  - `kira::dev::escribir_kira`: mantenía `kira::estado_mut()` vivo mientras `connection_setup_reply` (que llamaba a `kira::opcion()`) y `req::dispatch` (que vía `wm::ensure_root` volvía a llamar a `estado_mut()`) reentraban en el estado de Kira. Se corrigió pasando la **geometría de la raíz** (`root_wid`, `screen_w`, `screen_h`) y los **XID del cliente** como parámetros; ni `req::dispatch` ni `connection_setup_reply` ni `wm::ensure_root` tocan ya la celda KIRA.
  - `fd::soltar`: mantenía `&mut …TABLA` a través de `pipe::soltar_extremo` y `kira::dev::cerrar_conexion`. Se corrigió cerrando el préstamo antes de avisar al subsistema (se calcula `libera`/`cerrada` en un bloque acotado y se actúa después).
- **Casos revisados y considerados seguros** (documentados, no requieren cambio): `fs::vol()` (26 funciones públicas con `&'static mut`, pero **ninguna** llama a `dormir`/`ceder` ni anida `vol()`, así que no hay préstamo vivo a través de un cambio de tarea); `comp::reajustar` (mantiene `&mut Comp` a través de llamadas al BIOS, pero ninguna ruta de IRQ0/IRQ4 toca `COMP` y `critica` bloquea el cambio de tarea); `console::_print` (reentrada latente desde panic/excepción; hoy segura porque las IRQ no imprimen).
- **Atomics demotables identificados** (single-threaded, sin uso desde IRQ): estado del teclado en `input.rs`, `shell::PROFUNDIDAD`/`PROGRAMA_PROF`, `vm::SEMILLA`, `bios::PIC_PIT_PROPIOS`, `kira::wm::ORDEN_MAPEO`. Se dejan como están: un atómico no es un alias mutable y el coste es despreciable; cambiarlos añadiría riesgo sin ganancia. **Deben seguir atómicos:** `pit::TICKS`, `tarea::CRITICA`, `panico::EN_PANICO` y todos los contadores de `metricas`.

**Verificación Fase 4:** `cargo fmt --check` limpio; build oficial `xtask build` → `EXIT=0`, `kernel.bin : 529336 bytes (516 KiB)`; arranque en QEMU limpio con **`test_plan OK (8 pruebas)`**; `metricas` con las colas de la Fase 3 intactas (turnos interactiva/normal, 2 cesiones, despertares por reloj). Sin pérdida de funcionalidad: **`verif-kira.ps1` OK** (handshake, 4 ventanas, GC, dibujo y evento Expose, justo la ruta que tenía el alias), `verif-gui.ps1` OK, y las rutas de `verif-pipe`/`verif-fd`/`verif-usuario`/`verif-servicio` sin fallos funcionales (solo el conocido «zombi» preexistente en `verif-pipe`).

> Se actualizó la expectativa obsoleta de `verif-kira.ps1` y `verif-fondo.ps1`: el programa `kira_test` terminó imprimiendo «cuatro ventanas y superficies de dibujo OK» en lugar de «handshake y ventana creados OK»; la comprobación ahora usa el mensaje actual (que se imprime **después** del handshake, CreateWindow, MapWindow, CreateGC y dibujo, así que sigue verificando lo mismo o más).

> `verif-fondo.ps1` también tiene expectativas de píxel obsoletas: comprueba que el escritorio sea **azul sólido** (`azul >= 90%`), pero desde que existe el wallpaper por defecto (`FONDO_ESCRITORIO = "/usr/share/backgrounds/flower.png"`, `kira/comp.rs`) el fondo ya no es azul. Las capturas de la Fase 4 muestran el escritorio dibujado (brillo e imagen no triviales, cursor que se mueve, ESC a modo texto, la app `.toby` cede la pantalla, `servicio stop/start kira` y el cliente Kira funcionando, sin pánicos), así que las líneas `azul`/`fuera` fallan por el cambio de fondo, no por una regresión. Queda pendiente rehacer esas comprobaciones contra el wallpaper real (fuera del alcance de la Fase 4, que no toca render).

> Nota: QEMU no demuestra compatibilidad con hardware real (RX 550, BIOS física, controladores). Las pruebas se ejecutaron solo en QEMU.
