# Verificacion de los endpoints de interrupcion y del HID boot (fase 4).
#
# Lo que se comprueba:
#
#   1) Con `qemu-xhci` + `usb-kbd` + `usb-mouse`: el HCD enumera los dos
#      dispositivos, valida la subclase y el protocolo boot, pide el descriptor
#      HID y el de informe, manda `SET_PROTOCOL(Boot)` y `SET_IDLE(0)` y
#      programa un endpoint de interrupcion por dispositivo.
#   2) El teclado USB **escribe de verdad** en la consola: las ordenes de este
#      guion se teclean con `sendkey`, y con un teclado USB delante QEMU manda
#      esas teclas al HID, no al PS/2. Que la orden `usb` se ejecute prueba el
#      camino entero: informe -> usos HID -> juego 1 -> distribucion española.
#   3) La distribucion sigue siendo la española: la tecla de la derecha de la P
#      es un acento muerto, y con la A detras sale una a con acento (0xA0 en
#      CP437), que es el mismo byte que produce el PS/2.
#   4) El raton USB entrega movimiento: `mouse_move` del monitor hace que su
#      contador de informes suba.
#   5) Conexion y desconexion en caliente: `device_add`/`device_del` de un
#      teclado USB; al quitarlo se olvidan sus endpoints y el sistema sigue
#      vivo. Ademas, si se va con Shift pulsado, el kernel lo suelta solo: la
#      orden siguiente (tecleada por el PS/2, que nunca vio ese Shift) tiene que
#      salir en minusculas.
#   6) Un hub USB 2.0 (`usb-hub`) con un teclado y un raton colgando de dos de
#      sus puertos: el hub se configura y alimenta, sus puertos se sondean de
#      uno en uno, y los hijos se enumeran con su route string y su TT. Los
#      informes del teclado y del raton tienen que seguir llegando.
#
#   7) Conexion y desconexion en caliente dentro de un puerto de hub: se quita
#      el teclado de un puerto, se enchufa otro en otro puerto y se comprueba
#      que el kernel libera el hueco viejo y enumera el nuevo.
#
# Lo que NO se comprueba: interpretar el descriptor de informe (fase 6), mas de
# un endpoint de interrupcion por dispositivo, hubs USB 3.x, ni hardware fisico.
#
# La salida de la consola se refleja en el puerto serie y se lee de ahi.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }
if (-not (Test-Path $qemu)) { throw "no se encuentra qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"

# Las funciones de tecleo viven en el ambito de Ejecutar; el guion se ejecuta
# como bloque en ese mismo ambito, asi que las ve por alcance dinamico.
function Ejecutar($extra, $log, $puerto, $guion) {
  if (Test-Path $log) { Remove-Item $log -Force }
  $opciones = @(
    "-m", "256",
    "-boot", "order=a",
    "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
    "-drive", "if=ide,index=0,media=disk,format=raw,file=$raiz\rockydisk.img"
  )
  if ($extra.Count -gt 0) { $opciones += $extra }
  $opciones += @(
    "-serial", "file:$log",
    "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
    "-qmp", "tcp:127.0.0.1:$($puerto + 1000),server,nowait",
    "-display", "none",
    "-no-reboot"
  )
  $p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
        -RedirectStandardError "$tmp\qemu-hid-$puerto-err.txt" `
        -RedirectStandardOutput "$tmp\qemu-hid-$puerto-out.txt"

  $cliente = $null
  for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Milliseconds 500
    try { $cliente = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $puerto); break }
    catch { if ($p.HasExited) { throw "QEMU termino enseguida (ExitCode=$($p.ExitCode))" } }
  }
  if ($null -eq $cliente) { throw "no se pudo conectar al monitor en el puerto $puerto" }
  $flujo = $cliente.GetStream()
  $escritor = New-Object System.IO.StreamWriter($flujo)
  $escritor.AutoFlush = $true
  # Esperar al prompt en vez de un tiempo fijo: con un hub y varios hijos el
  # arranque puede tardar mas y las teclas se perderian antes de la consola.
  $limite = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $limite) {
    Start-Sleep -Milliseconds 250
    if (Test-Path $log) {
      $texto = Get-Content $log -Raw -ErrorAction SilentlyContinue
      if ($texto -and $texto -match 'root@rocky:/') { break }
    }
  }
  Start-Sleep -Milliseconds 300

  function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 90 }
  function Teclas([string]$t) {
    foreach ($c in $t.ToCharArray()) {
      if ($c -eq ' ') { Tecla "spc" } else { Tecla ([string]$c) }
    }
  }
  function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 2000 }
  function Monitor([string]$m, [int]$espera = 4) {
    $escritor.WriteLine($m); Start-Sleep -Seconds $espera
  }
  # Una tecla que se queda abajo: el monitor solo tiene `sendkey`, que siempre
  # manda el key-up al cabo de un tiempo. Por QMP se manda solo la bajada, para
  # que nadie la suelte y se pueda probar que la suelta el kernel.
  function Qmp([string]$cuerpo) {
    $c = New-Object System.Net.Sockets.TcpClient("127.0.0.1", ($puerto + 1000))
    $flujoQmp = $c.GetStream()
    $flujoQmp.ReadTimeout = 3000
    $lector = New-Object System.IO.StreamReader($flujoQmp)
    $emisor = New-Object System.IO.StreamWriter($flujoQmp)
    $emisor.AutoFlush = $true
    $null = $lector.ReadLine()   # saludo {"QMP": ...}
    $emisor.WriteLine('{"execute":"qmp_capabilities"}')
    $null = $lector.ReadLine()   # {"return": {}}
    $emisor.WriteLine($cuerpo)
    $null = $lector.ReadLine()
    $c.Close()
  }
  function TeclaAbajo([string]$k) {
    Qmp ('{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"' + $k + '"}}}]}}')
  }

  & $guion

  Start-Sleep -Seconds 2
  $escritor.WriteLine("quit"); Start-Sleep -Seconds 1
  $cliente.Close()
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
  Start-Sleep -Seconds 1
  return (Get-Content $log -Raw)
}

# --- 1) Teclado y raton USB: todo por el camino nuevo ---
$textoTodo = Ejecutar @(
                 "-device", "qemu-xhci",
                 "-device", "usb-kbd",
                 "-device", "usb-mouse"
               ) "$tmp\rocky-hid-todo.log" 5631 {
  # ESC sale del escritorio y vuelve a la consola. Lo recibe el teclado USB.
  Tecla "esc"; Start-Sleep -Seconds 3
  # El raton entrega un informe de movimiento antes de preguntar.
  Monitor "mouse_move 20 10" 3
  # La tecla del acento agudo (en un teclado de EE. UU., la comilla simple) es
  # acento muerto en la distribucion española: con la A detras sale "á".
  Tecla "apostrophe"; Tecla "a"
  # Se borra esa letra (un solo caracter) y se teclea la orden de verdad.
  Tecla "backspace"
  Orden "usb"
}

# --- 2) Conexion y desconexion en caliente ---
$textoCaliente = Ejecutar @("-device", "qemu-xhci") "$tmp\rocky-hid-caliente.log" 5632 {
  # Antes de enchufar nada, el PS/2 sigue siendo el teclado.
  Tecla "esc"; Start-Sleep -Seconds 3
  Monitor "device_add usb-kbd,id=kbd1" 5
  # Ahora QEMU manda las teclas al teclado USB recien conectado.
  Orden "usb"
  # Shift abajo en el teclado USB y desconexion sin soltarlo. El key-up no se
  # manda nunca: el unico que puede soltar el modificador es el kernel.
  TeclaAbajo "shift"
  Start-Sleep -Milliseconds 400
  Monitor "device_del kbd1" 5
  # Sin teclado USB, el PS/2 vuelve a recibir las teclas. Si Shift se hubiera
  # quedado pegado, esto seria "USB" y la orden no se ejecutaria.
  Orden "usb"
}

# --- 3) Un hub USB 2.0 con teclado y raton colgando ---
$textoHub = Ejecutar @(
                 "-device", "qemu-xhci",
                 "-device", "usb-hub,id=hub0",
                 "-device", "usb-kbd,port=1.1",
                 "-device", "usb-mouse,port=1.2"
               ) "$tmp\rocky-hid-hub.log" 5633 {
  Tecla "esc"; Start-Sleep -Seconds 3
  Monitor "mouse_move 20 10" 3
  Orden "usb"
}

Write-Host "=== 1) HID teclado + raton: ultimas 70 lineas ==="
($textoTodo -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 70) | Out-String -Width 200 | Write-Host
Write-Host "=== 2) HID en caliente: ultimas 70 lineas ==="
($textoCaliente -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 70) | Out-String -Width 200 | Write-Host
# --- 4) Conexion y desconexion en caliente dentro de un hub ---
$textoHubCaliente = Ejecutar @(
                 "-device", "qemu-xhci",
                 "-device", "usb-hub,id=hub1",
                 "-device", "usb-kbd,id=hkbd1,port=1.1"
               ) "$tmp\rocky-hid-hub-caliente.log" 5634 {
  Tecla "esc"; Start-Sleep -Seconds 3
  # Quitar el teclado del puerto 1 del hub: el kernel tiene que devolver su
  # hueco y dejar de contar su endpoint.
  Monitor "device_del hkbd1" 6
  # Enchufar otro teclado en el puerto 2: el sondeo del hub lo tiene que ver,
  # reiniciar el puerto y enumerarlo.
  Monitor "device_add usb-kbd,id=hkbd2,port=1.2" 7
  # La orden la teclea el teclado nuevo: prueba que el hijo en caliente funciona.
  Orden "usb"
  # Y ahora se quita tambien el segundo; el PS/2 vuelve a mandar.
  Monitor "device_del hkbd2" 6
  Orden "usb"
}

Write-Host "=== 3) HID detras de un hub: ultimas 90 lineas ==="
($textoHub -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 90) | Out-String -Width 200 | Write-Host
Write-Host "=== 4) HID en caliente dentro de un hub: ultimas 90 lineas ==="
($textoHubCaliente -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 90) | Out-String -Width 200 | Write-Host
Write-Host "=== fin ==="

$ok = $true
function Comprobar([bool]$cond, [string]$msg) {
  if ($cond) { Write-Host "OK   $msg" } else { Write-Host "FALLO $msg"; $script:ok = $false }
}

# --- 1) teclado y raton ---
Comprobar ($textoTodo -match "rocky: USB: \d+ controlador\(es\) de host \(xHCI\)") "el arranque detecta el xHCI"
Comprobar ($textoTodo -match "(?m)^usb: xHCI inicializado: \d+ puertos raiz, controlador en marcha\.") "el HCD arranca solo"
Comprobar ($textoTodo -match "(?m)^usb: dispositivo en el puerto \d+: 0627:0001 clase 0, 1 interfaces \(1 HID\), 1 endpoints \(1 de interrupcion\), velocidad (alta|completa) \(USB [12]\.[01]\), configuracion 1\.") "el teclado USB se enumera como HID con un endpoint de interrupcion"
Comprobar ($textoTodo -match "(?m)^usb: HID en el puerto \d+: teclado boot, interfaz 0, endpoint 1 IN, informe de \d+ bytes, descriptor de \d+ bytes\.") "el teclado se pone en protocolo boot y se programa su endpoint"
Comprobar ($textoTodo -match "(?m)^usb: HID en el puerto \d+: raton boot, interfaz 0, endpoint 1 IN, informe de \d+ bytes, descriptor de \d+ bytes\.") "el raton se pone en protocolo boot y se programa su endpoint"
Comprobar ($textoTodo -match "(?m)^    HID activo \.+ si \(2 endpoints, [1-9]\d* informes, 0 errores\)") "hay dos endpoints HID activos y llegan informes"
Comprobar ($textoTodo -match "(?m)^    puerto \d+  hueco \d+  direccion \d+  0627:0001  clase 0  1 interfaces \(1 HID\)  1 endpoints  hasta \d+ mA  configuracion 1  1 HID activos  [1-9]\d* informes") "el dispositivo enseña su HID activo y sus informes"
Comprobar ($textoTodo -match "(?m)^usb: contadores: .*informes HID [1-9]\d*, errores HID 0, descartes HID 0") "los contadores HID cuadran y no se ha descartado nada"
Comprobar ($textoTodo -match "(?m)^usb: estado: HCD inicializado") "la orden usb se ha ejecutado (la tecleo el teclado USB)"
Comprobar ($textoTodo -notmatch "(?i)panic|panico:") "sin panics con teclado y raton USB"

# El acento muerto de la distribucion española, por el camino del HID.
$bytes = [System.IO.File]::ReadAllBytes("$tmp\rocky-hid-todo.log")
Comprobar ($bytes -contains 0xA0) "el acento muerto y la A producen la a con acento (0xA0 de CP437)"

# --- 2) en caliente ---
Comprobar ($textoCaliente -match "(?m)^usb: HID en el puerto \d+: teclado boot") "el teclado USB enchufado en caliente acaba en protocolo boot"
Comprobar ($textoCaliente -match "usb: dispositivo del puerto \d+ \(hueco \d+\) desconectado\.") "al quitarlo, el puerto se da por desconectado"
Comprobar (([regex]::Matches($textoCaliente, "(?m)^usb: estado: HCD inicializado")).Count -ge 2) "las dos ordenes usb se ejecutan (USB y despues PS/2)"
Comprobar ($textoCaliente -match "(?m)^    HID activo \.+ no \(0 endpoints, \d+ informes, 0 errores\)") "tras la desconexion no queda ningun HID activo"
Comprobar ($textoCaliente -notmatch "Orden desconocida") "al desconectar con Shift pulsado no queda el modificador pegado"
Comprobar ($textoCaliente -notmatch "(?i)panic|panico:") "sin panics con conexion y desconexion en caliente"

# --- 3) hub USB 2.0 con teclado y raton ---
Comprobar ($textoHub -match "(?m)^usb: hub \d+: descriptor de \d+ puertos, \d+ mA, alimentacion en \d+ ms\.") "el hub se reconoce y se lee su descriptor"
Comprobar ($textoHub -match "(?m)^usb: hub \d+: \d+ puertos alimentados; se sondearan en las rondas siguientes\.") "el hub alimenta sus puertos"
Comprobar (([regex]::Matches($textoHub, "(?m)^usb: hub \d+ puerto \d+: dispositivo 0627:0001,")).Count -ge 2) "los dos hijos del hub se enumeran"
Comprobar ($textoHub -match "(?m)^usb: HID en el puerto \d+: teclado boot") "el teclado colgado del hub acaba en protocolo boot"
Comprobar ($textoHub -match "(?m)^usb: HID en el puerto \d+: raton boot") "el raton colgado del hub acaba en protocolo boot"
Comprobar ($textoHub -match "(?m)^    HID activo \.+ si \(2 endpoints, [1-9]\d* informes, 0 errores\)") "los dos HID del hub entregan informes"
Comprobar (([regex]::Matches($textoHub, "(?m)^    puerto \d+  hueco \d+  direccion \d+  0409:55AA.*clase 9")).Count -ge 1) "la lista incluye el hub"
Comprobar (([regex]::Matches($textoHub, "(?m)^    puerto \d+  hueco \d+  direccion \d+  0627:0001")).Count -ge 2) "la lista incluye los dos hijos del hub"
Comprobar ($textoHub -match "(?m)^    dispositivo enumerado \.+ si") "el hito 'dispositivo enumerado' se activa con el hub"
Comprobar ($textoHub -notmatch "(?i)panic|panico:") "sin panics con un hub USB 2.0"

# --- 4) en caliente dentro del hub ---
Comprobar ($textoHubCaliente -match "(?m)^usb: hub \d+ puerto 1: dispositivo 0627:0001") "el hub enumera el teclado de arranque"
Comprobar ($textoHubCaliente -match "usb: dispositivo del puerto \d+ \(hueco \d+\) desconectado\.") "al quitarlo del hub se libera el hijo"
Comprobar ($textoHubCaliente -match "(?m)^usb: hub \d+ puerto 2: dispositivo 0627:0001") "el teclado enchufado en caliente se enumera en el otro puerto"
Comprobar (([regex]::Matches($textoHubCaliente, "(?m)^usb: estado: HCD inicializado")).Count -ge 2) "las dos ordenes usb se ejecutan (teclado nuevo y PS/2)"
Comprobar ($textoHubCaliente -match "(?m)^    HID activo \.+ no \(0 endpoints, \d+ informes, 0 errores\)") "tras quitar los dos teclados no queda ningun HID activo"
Comprobar ($textoHubCaliente -notmatch "(?i)panic|panico:") "sin panics con conexion en caliente dentro de un hub"

if ($ok) {
  Write-Host "RESULTADO: OK (HID boot de teclado y raton, informes, acentos, conexion en caliente y hub USB 2.0)"
} else {
  Write-Host "RESULTADO: FALLO"
  exit 1
}
