# Verificacion de la fase 18: la orden `gui` pone el escritorio de Kira a la
# resolucion pedida (VBE y compositor Kira; no mide ni afirma aceleracion GPU).
#   - `gui` sin argumentos (re)pon el escritorio y dice la resolucion activa;
#   - `gui 1280x800` cambia el modo y lo confirma por serie;
#   - `gui 9999x9999` (modo inexistente) cae en la cascada y lo dice;
#   - `gui info` lista los modos VESA ademas del estado;
#   - tras `gui 1280x800`, la captura de pantalla mide 1280x800 (el modo VBE
#     de verdad esta puesto, no solo el mensaje);
#   - `gui off` vuelve al modo texto.
# Se usa la serie COM1 (canal que leen las pruebas) y HMP `screendump`.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-gui.log"
if (Test-Path $log) { Remove-Item $log -Force }
Get-ChildItem "$tmp\gui-*.ppm" -ErrorAction SilentlyContinue | Remove-Item -Force

$puerto = 5614
$opciones = @(
  "-m", "256",
  "-boot", "order=a",
  "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
  "-drive", "if=ide,index=0,media=disk,format=raw,file=$raiz\rockydisk.img",
  "-serial", "file:$log",
  "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
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

# Teclado es_ES en el kernel; QEMU `sendkey` usa posiciones US.
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
function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Seconds 2 }

# (ancho, alto) de la captura PPM (P6), leyendo la cabecera.
function Dimensiones([string]$ruta) {
  if (!(Test-Path $ruta)) { return $null }
  $b = [System.IO.File]::ReadAllBytes($ruta)
  $hdr = [System.Text.Encoding]::ASCII.GetString($b, 0, 64)
  $m = [regex]::Match($hdr, '^P6\s+(?:#[^\r\n]*\s+)?(\d+)\s+(\d+)\s+(\d+)\s')
  if (!$m.Success) { return $null }
  ,@([int]$m.Groups[1].Value, [int]$m.Groups[2].Value)
}

# --- La prueba ---
Write-Host ("[{0:HH:mm:ss}] t0: arranque" -f (Get-Date))
# Kira arranca tomando el framebuffer; ESC devuelve la consola de texto para
# poder teclear. Comprobamos que el modo por defecto es 1024x768.
Tecla "esc"
Start-Sleep -Seconds 2

Orden "gui"
Write-Host ("[{0:HH:mm:ss}] gui (sin args) tecleado" -f (Get-Date))
Start-Sleep -Seconds 2

# Kira esta en graficos (sin consola): se mira la resolucion activa por HMP sin
# tocar el teclado, y se captura la pantalla.
$escritor.WriteLine("info vnc")
Start-Sleep -Milliseconds 500
$escritor.WriteLine("screendump $tmp/gui-01-default.ppm")
Start-Sleep -Milliseconds 800

# Vuelve el modo texto para poder seguir tecleando.
Tecla "esc"
Start-Sleep -Seconds 2

# Cambio de resolucion: 1280x800. Tras `gui`, Kira vuelve a graficos.
Orden "gui 1280x800"
Write-Host ("[{0:HH:mm:ss}] gui 1280x800 tecleado" -f (Get-Date))
Start-Sleep -Seconds 2

$escritor.WriteLine("info vnc")
Start-Sleep -Milliseconds 500
$escritor.WriteLine("screendump $tmp/gui-02-1280.ppm")
Start-Sleep -Milliseconds 800

# Prueba del escalador de Kira (cursor + imagen). Con el escritorio aun activo
# (tras la captura de 1280x800), se escala el cursor y se pinta una imagen
# reescalada; ambos mensajes confirman por serie que el muestreo se ejecuto.
Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui puntero 2"
Write-Host ("[{0:HH:mm:ss}] gui puntero 2 tecleado" -f (Get-Date))
Start-Sleep -Seconds 1

Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui prueba"
Write-Host ("[{0:HH:mm:ss}] gui prueba tecleado" -f (Get-Date))
Start-Sleep -Seconds 1
$escritor.WriteLine("screendump $tmp\gui-03-prueba.ppm")
Start-Sleep -Milliseconds 800

# Al texto de nuevo y se prueba un modo que existe como numero pero no como
# modo VBE (1000x1000): tiene que caer en la cascada (1024x768) y avisar.
Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui 1000x1000"
Write-Host ("[{0:HH:mm:ss}] gui 1000x1000 tecleado" -f (Get-Date))
Start-Sleep -Seconds 2

# Y la lista de modos.
Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui info"
Write-Host ("[{0:HH:mm:ss}] gui info tecleado" -f (Get-Date))
Start-Sleep -Seconds 2

# Apagar el escritorio (vuelve al modo texto definitivo).
Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui off"
Write-Host ("[{0:HH:mm:ss}] gui off tecleado" -f (Get-Date))
Start-Sleep -Seconds 2

$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
Write-Host "=== salida serie (ultimas lineas) ==="
Write-Host (($texto -split "`n" | Select-Object -Last 40) -join "`n")
Write-Host "=== fin ==="

# --- Veredicto ---
$ok = $true
function Comprobar([bool]$cond, [string]$txt) {
  if ($cond) { Write-Host "[ok] $txt" } else { Write-Host "[FALLO] $txt"; $script:ok = $false }
}

$d1 = Dimensiones "$tmp\gui-01-default.ppm"
$d2 = Dimensiones "$tmp\gui-02-1280.ppm"

if ($d1) { Write-Host ("  captura 01 (default): {0}x{1}" -f $d1[0], $d1[1]) }
if ($d2) { Write-Host ("  captura 02 (gui 1280x800): {0}x{1}" -f $d2[0], $d2[1]) }

Comprobar ($texto -match "gui\s*:\s*escritorio de Kira activo\s*\(1024x768\)") "gui (sin args) arranca el escritorio a 1024x768"
Comprobar ($texto -match "gui\s*:\s*escritorio de Kira a\s*1280x800") "gui 1280x800 confirma el modo por serie"
Comprobar ($d2 -ne $null -and $d2[0] -eq 1280 -and $d2[1] -eq 800) "la captura tras `gui 1280x800` mide 1280x800"
Comprobar ($texto -match "gui\s*:\s*cursor Flecha, escala 2x") "gui puntero 2 escala el cursor del escritorio"
Comprobar ($texto -match "gui\s*:\s*imagen de prueba .* pintada en") "gui prueba pinta una imagen reescalada (escalador de Kira)"
Comprobar ($texto -match "no tiene 1000x1000") "gui 1000x1000 (inexistente) cae en la cascada y lo avisa"
Comprobar ($texto -match "modos VESA|resolucion pedida") "gui info lista los modos y la resolucion pedida"
Comprobar ($texto -match "gui\s*:\s*escritorio apagado") "gui off vuelve al modo texto"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panicos en la serie"

if ($ok) {
  Write-Host "RESULTADO: OK (orden gui verificada)"
} else {
  Write-Host "RESULTADO: FALLO"
}
