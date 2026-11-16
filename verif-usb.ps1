# Verificacion de la pila USB de rocky (fases 0, 1 y 2 del plan USB/HID).
#
# Lo que se comprueba:
#
#   1) Arranque SIN controlador USB: la orden `usb` tiene que decir con claridad
#      que no hay ningun controlador de host, y la entrada del sistema (PS/2 y
#      puerto serie) tiene que seguir viva (la orden se ejecuta por teclado).
#   2) Arranque CON un xHCI emulado (`-device qemu-xhci`): el controlador tiene
#      que arrancar solo, en segundo plano, y la orden `usb` tiene que decir
#      que esta "inicializado y en marcha", enseñar la tabla de puertos raiz y
#      los contadores del bucle de sondeo.
#   3) Con un dispositivo USB conectado (`usb-storage`): algun puerto raiz tiene
#      que salir "conectado" con una velocidad definida.
#   4) Conectado en caliente (`device_add` por el monitor de QEMU): el anillo
#      de eventos tiene que entregar el evento de cambio de puerto (el
#      contador "cambios de puerto" sube).
#   5) `usb medir` y `usb preparar` tienen que **rechazar** el controlador: ya
#      lo tiene en uso el HCD, y el software de sondeo no puede escribir en
#      los registros de un controlador vivo.
#
#   6) Con un dispositivo USB de almacenamiento: se **enumera** (hueco,
#      direccion, descriptores y `SET_CONFIGURATION`) y la orden `usb` lo
#      lista con su clase, interfaces, endpoints y configuracion.
#
# Lo que NO se comprueba (aun): Transfer Rings de endpoints que no sean el 0
# y HID. Eso es de las fases siguientes.
#
# Toda la salida va por la consola y se refleja en el puerto serie.
#
# Nota sobre QEMU: un dispositivo USB **HID** (teclado o raton USB) le quita
# el foco de entrada al teclado PS/2, y entonces `sendkey` ya no llega al
# guest. Por eso aqui solo se usan dispositivos que no son HID (almacenamiento
# USB): el PS/2 tiene que seguir vivo para poder escribir las ordenes.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }
if (-not (Test-Path $qemu)) { throw "no se encuentra qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"

# Un disco pequeno para el dispositivo de almacenamiento USB.
$pen = "$tmp\pen-verif.img"
if (-not (Test-Path $pen)) { fsutil file createnew $pen 1048576 | Out-Null }

function Ejecutar($extra, $log, $puerto, $ordenes, $monitor = $null) {
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
    "-display", "none",
    "-no-reboot"
  )
  $p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
        -RedirectStandardError "$tmp\qemu-usb-$puerto-err.txt" `
        -RedirectStandardOutput "$tmp\qemu-usb-$puerto-out.txt"

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
  Start-Sleep -Seconds 14

  function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 80 }
  # `sendkey` no entiende un espacio literal: hay que darle el nombre de la
  # tecla ("spc"), o las ordenes con argumentos llegan pegadas ("usbmedir").
  function Teclas([string]$t) {
    foreach ($c in $t.ToCharArray()) {
      if ($c -eq ' ') { Tecla "spc" } else { Tecla ([string]$c) }
    }
  }
  function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 1500 }

  # ESC deja el escritorio de Kira y devuelve la consola de texto. Que la orden
  # llegue al interprete prueba ademas que el PS/2 sigue vivo.
  Tecla "esc"; Start-Sleep -Seconds 3

  # Un comando de monitor antes de escribir las ordenes: sirve para enchufar
  # un dispositivo en caliente, con el controlador ya en marcha.
  if ($null -ne $monitor) {
    $escritor.WriteLine($monitor)
    Start-Sleep -Seconds 4
  }

  foreach ($o in $ordenes) { Orden $o }
  Start-Sleep -Seconds 2

  $escritor.WriteLine("quit"); Start-Sleep -Seconds 1
  $cliente.Close()
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
  Start-Sleep -Seconds 1
  return (Get-Content $log -Raw)
}

# 1) Sin controlador USB.
$texto = Ejecutar @() "$tmp\rocky-usb.log" 5621 @("usb")

# 2) Con un xHCI emulado y sin dispositivos: el HCD arranca solo.
$textoXhci = Ejecutar @("-device", "qemu-xhci") "$tmp\rocky-usb-xhci.log" 5622 `
                   @("usb", "usb medir", "usb preparar")

# 3) Con un xHCI emulado y un dispositivo de almacenamiento USB conectado.
$textoDispositivo = Ejecutar @(
                     "-device", "qemu-xhci",
                     "-device", "usb-storage,drive=pen0",
                     "-drive", "if=none,id=pen0,file=$pen,format=raw"
                   ) "$tmp\rocky-usb-dispositivo.log" 5623 @("usb")

# 4) Con un xHCI emulado y un dispositivo enchufado en caliente.
$textoCaliente = Ejecutar @(
                    "-device", "qemu-xhci",
                    "-drive", "if=none,id=pen1,file=$pen,format=raw"
                  ) "$tmp\rocky-usb-caliente.log" 5624 @("usb") `
                  "device_add usb-storage,drive=pen1"

Write-Host "=== 1) QEMU sin USB: ultimas 25 lineas ==="
($texto -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 25) | Out-String -Width 200 | Write-Host
Write-Host "=== 2) QEMU con xHCI: ultimas 60 lineas ==="
($textoXhci -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 60) | Out-String -Width 200 | Write-Host
Write-Host "=== 3) QEMU con xHCI y dispositivo USB: ultimas 45 lineas ==="
($textoDispositivo -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 45) | Out-String -Width 200 | Write-Host
Write-Host "=== 4) QEMU con xHCI y dispositivo en caliente: ultimas 45 lineas ==="
($textoCaliente -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 45) | Out-String -Width 200 | Write-Host
Write-Host "=== fin ==="

$ok = $true
function Comprobar([bool]$cond, [string]$msg) {
  if ($cond) { Write-Host "OK   $msg" } else { Write-Host "FALLO $msg"; $script:ok = $false }
}

# --- Arranque sin controlador USB ---
Comprobar ($texto -match "rocky: USB: .*controlador") "el arranque resume el estado de USB"
Comprobar ($texto -match "(?m)^usb: inventario de controladores de host USB") "la orden usb responde"
Comprobar ($texto -match "(?m)^usb: estado: ") "la orden usb dice el estado"
Comprobar ($texto -match "PS/2 y el puerto serie") "sin USB, la entrada sigue siendo PS/2 y el puerto serie"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panics en el arranque sin USB"

# --- Arranque con xHCI emulado: el HCD arranca solo ---
Comprobar ($textoXhci -match "rocky: USB: \d+ controlador\(es\) de host \(xHCI\)") "el arranque detecta el xHCI"
Comprobar ($textoXhci -match "(?m)^usb: inventario de controladores de host USB \(solo lectura\)") "la orden usb lista el inventario"
Comprobar ($textoXhci -match "clase 0C subclase 03 prog_if 30  xHCI") "el xHCI se clasifica por clase (0C/0C03/30), no por fabricante"
Comprobar ($textoXhci -match "(?m)^    BAR0  base 0x[0-9A-F]+  memoria") "el inventario muestra el BAR de registros"
Comprobar ($textoXhci -match "(?m)^    registros en 0x[0-9A-F]+  \(cubierto por el mapa por identidad\)") "la ventana de registros la cubre el mapa del arranque"
Comprobar ($textoXhci -match "(?m)^usb: xHCI inicializado: \d+ puertos raiz, controlador en marcha\.") "el HCD arranca solo y en marcha"
Comprobar ($textoXhci -match "(?m)^usb: estado: HCD inicializado") "el estado dice HCD inicializado"
Comprobar ($textoXhci -match "(?m)^    HCD detectado \.+ si") "el hito 'HCD detectado' esta activo"
Comprobar ($textoXhci -match "(?m)^    HCD soportado por rocky \.+ si") "el hito 'HCD soportado por rocky' esta activo"
Comprobar ($textoXhci -match "(?m)^    HCD inicializado \.+ si") "el hito 'HCD inicializado' esta activo"
Comprobar ($textoXhci -match "(?m)^    fallo al iniciar \.+ no") "el hito 'fallo al iniciar' sigue a no"
Comprobar ($textoXhci -match "(?m)^usb: HCD xHCI: inicializado y en marcha") "el HCD publica su estado"
Comprobar ($textoXhci -match "(?m)^usb: puertos raiz \(xHCI\):") "la orden usb enseña la tabla de puertos raiz"
Comprobar ($textoXhci -match "(?m)^    1  USB 3\.x ") "el puerto 1 es un puerto USB 3.x"
Comprobar ($textoXhci -match "(?m)^    5  USB 2\.0 ") "el puerto 5 es un puerto USB 2.0"
Comprobar ($textoXhci -match "(?m)^    puerto conectado \.+ no") "sin dispositivos, el hito 'puerto conectado' sigue a no"
Comprobar ($textoXhci -match "(?m)^    dispositivo enumerado \.+ no") "el hito 'dispositivo enumerado' sigue a no"
Comprobar ($textoXhci -match "(?m)^    HID activo \.+ no") "el hito 'HID activo' sigue a no"
Comprobar ($textoXhci -match "la emulacion USB heredada no se toca") "no se toca la emulacion USB heredada"
Comprobar ($textoXhci -notmatch "(?i)panic|panico:") "sin panics con el xHCI"

# --- El sondeo no puede escribir en un controlador vivo ---
Comprobar ($textoXhci -match "no se pudo medir \(el controlador esta en uso\)") "usb medir rechaza el controlador en uso"
Comprobar ($textoXhci -match "(?m)^usb: controlador [0-9A-F]{2}:[0-9A-F]{2}\.\d: el controlador ya esta en uso") "usb preparar rechaza el controlador en uso"

# --- Con un dispositivo USB conectado ---
Comprobar ($textoDispositivo -match "(?m)^usb: xHCI inicializado: \d+ puertos raiz, controlador en marcha\.") "con dispositivo, el HCD tambien arranca en marcha"
Comprobar ($textoDispositivo -match "(?m)^    puerto conectado \.+ si") "el hito 'puerto conectado' se activa"
Comprobar ($textoDispositivo -match "(?m)^    1  USB 3\.x  conectado  (baja|completa|alta|SuperSpeed|SuperSpeed Plus)") "un puerto raiz informa de la conexion y de la velocidad"
Comprobar ($textoDispositivo -match "(?m)^usb: contadores: sondeos \d+, eventos \d+, cambios de puerto \d+, desconocidos 0, errores 0") "los contadores del sondeo son coherentes"
Comprobar ($textoDispositivo -match "(?m)^usb: dispositivos enumerados:") "la orden usb lista los dispositivos enumerados"
Comprobar ($textoDispositivo -match "(?m)^    puerto 1  hueco \d+  direccion \d+  [0-9A-F]{4}:[0-9A-F]{4}  clase 0  1 interfaces \(0 HID\)") "el dispositivo de almacenamiento se enumera (una interfaz, ninguna HID)"
Comprobar ($textoDispositivo -match "(?m)^    puerto 1  hueco \d+  direccion \d+  .*  configuracion 1") "el dispositivo queda configurado (configuracion 1)"
Comprobar ($textoDispositivo -match "(?m)^    dispositivo enumerado \.+ si") "el hito 'dispositivo enumerado' se activa"
Comprobar ($textoDispositivo -notmatch "(?i)panic|panico:") "sin panics con un dispositivo USB"

# --- Con un dispositivo enchufado en caliente: el anillo de eventos ---
Comprobar ($textoCaliente -match "(?m)^usb: contadores: sondeos \d+, eventos [1-9]\d*, cambios de puerto [1-9]\d*, desconocidos 0, errores 0") "el anillo de eventos entrega el cambio de puerto"
Comprobar ($textoCaliente -match "(?m)^    puerto conectado \.+ si") "el puerto detecta el dispositivo enchufado en caliente"
Comprobar ($textoCaliente -match "(?m)^usb: dispositivos enumerados:") "el dispositivo enchufado en caliente se enumera"
Comprobar ($textoCaliente -match "(?m)^    dispositivo enumerado \.+ si") "el hito 'dispositivo enumerado' se activa en caliente"
Comprobar ($textoCaliente -notmatch "(?i)panic|panico:") "sin panics con un dispositivo en caliente"

if ($ok) {
  Write-Host "RESULTADO: OK (HCD xHCI inicializado, puertos raiz con estado y eventos, dispositivos enumerados)"
} else {
  Write-Host "RESULTADO: FALLO"
}
