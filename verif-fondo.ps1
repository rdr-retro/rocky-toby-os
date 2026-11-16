# Verificacion del compositor de Kira: escritorio azul vacio + cursor.
#   - al arrancar, Kira toma el framebuffer y deja un **escritorio vacio**:
#     fondo azul (34,62,168) y el cursor del raton, SIN texto encima;
#   - el cursor sigue al raton (se mueve con QMP: input-send-event rel);
#   - **ESC sale de Kira**: vuelve el modo texto con la consola visible;
#   - `servicio` sigue listando `kira active` (ESC oculta el escritorio, no
#     para el servicio);
#   - una app `.toby` (orden `toby`) cede la pantalla; al salir de ella (ESC)
#     Kira recupera su escritorio azul y el cursor;
#   - `servicio stop kira` vuelve al modo texto (y `servicio` lo lista
#     `inactive`); `servicio start kira` recupera el escritorio azul;
#   - con Kira activa en segundo plano, el cliente `kira` (anillo 3) sigue
#     funcionando.
# Se usan capturas de pantalla (HMP `screendump`, PPM) y el monitor/QMP para
# mover el raton.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-fondo.log"
if (Test-Path $log) { Remove-Item $log -Force }
Get-ChildItem "$tmp\fondo-*.ppm" -ErrorAction SilentlyContinue | Remove-Item -Force

$puerto = 5608
$qmpPuerto = 5609
$opciones = @(
  "-m", "256",
  "-boot", "order=a",
  "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
  "-drive", "if=ide,index=0,media=disk,format=raw,file=$raiz\rockydisk.img",
  "-serial", "file:$log",
  "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
  "-qmp", "tcp:127.0.0.1:$qmpPuerto,server,nowait",
  "-display", "none",
  "-no-reboot"
)
$p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru -RedirectStandardError "$tmp\qemu-err.txt" -RedirectStandardOutput "$tmp\qemu-out.txt"

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
Start-Sleep -Seconds 8

function Nombre([char]$ch) {
  switch -CaseSensitive ([string]$ch) {
    "/" { "shift-7" }
    "." { "dot" }
    "-" { "slash" }
    " " { "spc" }
    "_" { "shift-slash" }
    default { [string]$ch }
  }
}
function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 70 }
function Teclas([string]$t) { foreach ($c in $t.ToCharArray()) { Tecla (Nombre $c) } }
function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 1200 }
function Dump([string]$nombre) {
  $escritor.WriteLine("screendump $tmp/$nombre.ppm")
  Start-Sleep -Milliseconds 800
}

# --- Analisis de una captura PPM (P6) ---
# azul  = % de pixeles del azul exacto de Kira (muestreo cada 8 px);
# ini   = pixeles blancos y negros en la caja de 60x60 donde arranca el
#         puntero (arriba a la izquierda, separado del borde);
# mov   = esos mismos en la caja a la que llega tras el movimiento del raton;
# brillo= pixeles claros (texto) en toda la captura (cada 4 px); el gris claro
#         de la VGA mide ~160, por eso el umbral es 140;
# fuera = esos mismos claros pero FUERA de las dos cajas del puntero: en el
#         escritorio vacio de Kira tiene que ser practicamente 0.
#
# La posicion por defecto es `cursor::MARGEN_INICIAL` del kernel (24 px del
# borde). El QMP del guion mueve el raton 50 px a la derecha y 20 arriba, que
# es el salto de la caja "mov".
$margen = 24
$pxIni = $margen
$pyIni = $margen
$pxMov = $margen + 50
$pyMov = $margen - 20

function Analizar([string]$ruta) {
  if (!(Test-Path $ruta)) { return $null }
  $b = [System.IO.File]::ReadAllBytes($ruta)
  $hdr = [System.Text.Encoding]::ASCII.GetString($b, 0, 64)
  $m = [regex]::Match($hdr, '^P6\s+(?:#[^\r\n]*\s+)?(\d+)\s+(\d+)\s+(\d+)\s')
  if (!$m.Success) { return $null }
  $w = [int]$m.Groups[1].Value
  $h = [int]$m.Groups[2].Value
  $off = $m.Index + $m.Length

  $azul = 0; $n = 0
  for ($y = 0; $y -lt $h; $y += 8) {
    for ($x = 0; $x -lt $w; $x += 8) {
      $p = $off + ($y * $w + $x) * 3
      $r = $b[$p]; $g = $b[$p + 1]; $bl = $b[$p + 2]
      $n++
      if ([math]::Abs($r - 34) -le 6 -and [math]::Abs($g - 62) -le 6 -and [math]::Abs($bl - 168) -le 8) { $azul++ }
    }
  }
  # Caja de 60x60 centrada en (qx, qy), recortada a los limites de la captura:
  # la de arranque queda tan cerca de la esquina que el recorte manda.
  function Caja($qx, $qy) {
    $blanco = 0; $negro = 0
    $x0 = [math]::Max(0, $qx - 30); $x1 = [math]::Min($w - 1, $qx + 30)
    $y0 = [math]::Max(0, $qy - 30); $y1 = [math]::Min($h - 1, $qy + 30)
    for ($y = $y0; $y -le $y1; $y++) {
      for ($x = $x0; $x -le $x1; $x++) {
        $p = $off + ($y * $w + $x) * 3
        $r = $b[$p]; $g = $b[$p + 1]; $bl = $b[$p + 2]
        if ($r -ge 200 -and $g -ge 200 -and $bl -ge 200) { $blanco++ }
        if ($r -le 40 -and $g -le 40 -and $bl -le 40) { $negro++ }
      }
    }
    ,@($blanco, $negro)
  }
  $brillo = 0; $fuera = 0
  for ($y = 0; $y -lt $h; $y += 4) {
    for ($x = 0; $x -lt $w; $x += 4) {
      $p = $off + ($y * $w + $x) * 3
      $r = $b[$p]; $g = $b[$p + 1]; $bl = $b[$p + 2]
      if ($r -ge 140 -and $g -ge 140 -and $bl -ge 140) {
        $brillo++
        $enIni = [math]::Abs($x - $pxIni) -le 30 -and [math]::Abs($y - $pyIni) -le 30
        $enMov = [math]::Abs($x - $pxMov) -le 30 -and [math]::Abs($y - $pyMov) -le 30
        if (-not $enIni -and -not $enMov) { $fuera++ }
      }
    }
  }
  $c1 = Caja $pxIni $pyIni                # donde arranca el puntero
  $c2 = Caja $pxMov $pyMov                # donde queda tras el movimiento
  [pscustomobject]@{
    azul         = [int](100 * $azul / $n)
    iniBlanco    = $c1[0]; iniNegro = $c1[1]
    movBlanco    = $c2[0]; movNegro = $c2[1]
    brillo       = $brillo
    fuera        = $fuera
  }
}

# --- La prueba ---
Dump "fondo-01-inicio"

# Mueve el raton con QMP (input-send-event rel): 50 px a la derecha y 20 px
# arriba, que es lo que debe arrastrar el cursor de Kira.
$qmp = $null
for ($i = 0; $i -lt 10; $i++) {
  Start-Sleep -Milliseconds 300
  try { $qmp = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $qmpPuerto); break } catch {}
}
if ($null -ne $qmp) {
  $qf = $qmp.GetStream()
  $qb = New-Object byte[] 4096
  [void]$qf.Read($qb, 0, 4096)
  $qw = New-Object System.IO.StreamWriter($qf); $qw.AutoFlush = $true
  $qw.WriteLine('{"execute":"qmp_capabilities"}')
  Start-Sleep -Milliseconds 300
  [void]$qf.Read($qb, 0, 4096)
  $qw.WriteLine('{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"x","value":50}},{"type":"rel","data":{"axis":"y","value":-20}}]}}')
  Start-Sleep -Milliseconds 500
  $qmp.Close()
}
Start-Sleep -Milliseconds 500
Dump "fondo-02-movido"

# ESC: sale de Kira y vuelve el modo texto (el servicio sigue activo).
Tecla "esc"
Start-Sleep -Seconds 2
Dump "fondo-03-texto"

Orden "servicio"
Start-Sleep -Milliseconds 500

# Desde el texto se lanza una app `.toby`. Al salir de ella (ESC), Kira
# recupera su pantalla.
Orden "toby"
Start-Sleep -Seconds 6
Dump "fondo-04-toby"

Tecla "esc"
Start-Sleep -Seconds 3
Dump "fondo-05-recuperado"

# Otra vez al texto y se para/arranca el servicio de verdad.
Tecla "esc"
Start-Sleep -Seconds 2
Dump "fondo-06-oculto"

Orden "servicio stop kira"
Start-Sleep -Seconds 2
Dump "fondo-07-stop"

Orden "servicio"
Start-Sleep -Milliseconds 500

Orden "servicio start kira"
Start-Sleep -Seconds 3
Dump "fondo-08-start"

# Con Kira activa, el cliente de anillo 3 sigue funcionando.
Tecla "esc"
Start-Sleep -Seconds 2
Orden "kira"
Start-Sleep -Seconds 6

$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
Write-Host "=== salida serie (ultimas lineas) ==="
Write-Host (($texto -split "`n" | Select-Object -Last 45) -join "`n")
Write-Host "=== fin ==="

# --- Veredicto ---
$ok = $true
function Comprobar([bool]$cond, [string]$txt) {
  if ($cond) { Write-Host "[ok] $txt" } else { Write-Host "[FALLO] $txt"; $script:ok = $false }
}

$i1 = Analizar "$tmp\fondo-01-inicio.ppm"
$i2 = Analizar "$tmp\fondo-02-movido.ppm"
$i3 = Analizar "$tmp\fondo-03-texto.ppm"
$i4 = Analizar "$tmp\fondo-04-toby.ppm"
$i5 = Analizar "$tmp\fondo-05-recuperado.ppm"
$i6 = Analizar "$tmp\fondo-06-oculto.ppm"
$i7 = Analizar "$tmp\fondo-07-stop.ppm"
$i8 = Analizar "$tmp\fondo-08-start.ppm"

foreach ($i in @($i1, $i2, $i3, $i4, $i5, $i6, $i7, $i8)) {
  if ($null -ne $i) {
    Write-Host ("  captura: azul={0}%  ini(b/n)={1}/{2}  mov(b/n)={3}/{4}  brillo={5} fuera={6}" -f `
      $i.azul, $i.iniBlanco, $i.iniNegro, $i.movBlanco, $i.movNegro, $i.brillo, $i.fuera)
  }
}

Comprobar ($null -ne $i1 -and $i1.azul -ge 90) "al arrancar la pantalla es azul de Kira"
Comprobar ($null -ne $i1 -and $i1.iniBlanco -ge 10 -and $i1.iniNegro -ge 10) "el puntero arranca arriba a la izquierda, separado del borde"
Comprobar ($null -ne $i1 -and $i1.fuera -le 5) "el escritorio esta vacio (ni una letra encima)"
Comprobar ($null -ne $i2 -and $i2.movBlanco + $i2.movNegro -ge 20 -and ($i2.iniBlanco + $i2.iniNegro) -lt ($i1.iniBlanco + $i1.iniNegro)) "el cursor se mueve con el raton"
Comprobar ($null -ne $i3 -and $i3.azul -le 5) "ESC sale de Kira y vuelve al modo texto"
Comprobar ($null -ne $i3 -and $i3.brillo -ge 50) "el modo texto muestra la consola (lo impreso durante el arranque)"
Comprobar ($texto -match "kira\s+active") "ESC no para el servicio: se lista kira active"
Comprobar ($null -ne $i4 -and $i4.azul -le 5) "la app .toby cede la pantalla (sin azul de Kira)"
Comprobar ($null -ne $i5 -and $i5.azul -ge 90) "al salir de la app .toby, Kira recupera su escritorio azul"
Comprobar ($null -ne $i5 -and $i5.iniBlanco + $i5.iniNegro -ge 20) "el cursor vuelve a aparecer al recuperar"
Comprobar ($null -ne $i6 -and $i6.azul -le 5) "otro ESC vuelve a ocultar el escritorio"
Comprobar ($null -ne $i7 -and $i7.azul -le 5) "servicio stop kira vuelve al modo texto"
Comprobar ($texto -match "kira\s+inactive") "servicio stop lo lista como kira inactive"
Comprobar ($null -ne $i8 -and $i8.azul -ge 90) "servicio start kira recupera el escritorio azul"
Comprobar ($null -ne $i8 -and $i8.iniBlanco + $i8.iniNegro -ge 20) "al recuperar vuelve el cursor"
Comprobar ($texto -match "kira_test: evento 12 recibido OK" -and $texto -match "kira_test: cuatro ventanas y superficies de dibujo OK") "el cliente kira sigue funcionando"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panics en la serie"

if ($ok) {
  Write-Host "RESULTADO: OK (compositor Kira verificado)"
} else {
  Write-Host "RESULTADO: FALLO"
}
