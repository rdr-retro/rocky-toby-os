# Verificacion de "Toby UI con PNG": el lenguaje .toby compone una aplicacion
# cuyos botones son PNG (kit AQUA) ensamblados en 3 slices.
#
#   - el kernel arranca con Kira (escritorio AQUA);
#   - ESC vuelve al modo texto;
#   - la orden `toby` monta /usr/share/tobyui/demo.toby a pantalla completa;
#   - en la captura aparecen los botones de COLOR del kit AQUA (azul, verde,
#     rojo, naranja, amarillo, magenta, blanco), que solo pueden venir de las
#     piezas PNG ensambladas (el tema ya no pinta un relleno plano);
#   - con el foco en el primer boton, dos flechas a la derecha lo dejan en
#     "Salir" e Intro ejecuta window.close: la aplicacion se cierra. Es la
#     prueba de que las flechas (que antes se descartaban) llegan a la cola
#     neutral y mueven el foco; si no llegaran, Intro guardaria y la
#     aplicacion seguiria en pantalla;
#   - al cerrar, Kira recupera su pantalla; una segunda vuelta comprueba que
#     ESC tambien sale de la aplicacion.
#
# Se usan capturas de pantalla (HMP `screendump`, PPM).

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-toby.log"
if (Test-Path $log) { Remove-Item $log -Force }
Get-ChildItem "$tmp\toby-*.ppm" -ErrorAction SilentlyContinue | Remove-Item -Force

$puerto = 5610
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
$p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru -RedirectStandardError "$tmp\qemu-toby-err.txt" -RedirectStandardOutput "$tmp\qemu-toby-out.txt"

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
Start-Sleep -Seconds 4

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
function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 80 }
function Teclas([string]$t) { foreach ($c in $t.ToCharArray()) { Tecla (Nombre $c) } }
function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 1200 }
function Dump([string]$nombre) {
  $escritor.WriteLine("screendump $tmp/$nombre.ppm")
  Start-Sleep -Milliseconds 900
}

# --- Analisis de una captura PPM (P6) ---
# Cuenta pixeles (absolutos) del area de la ventana (x<600, y<240) cercanos al
# color dominante de cada pieza del kit AQUA, mas el azul del escritorio de
# Kira. Los botones son pequenos, asi que se cuentan pixeles y no porcentajes.
function Analizar([string]$ruta) {
  if (!(Test-Path $ruta)) { return $null }
  $b = [System.IO.File]::ReadAllBytes($ruta)
  $hdr = [System.Text.Encoding]::ASCII.GetString($b, 0, 64)
  $m = [regex]::Match($hdr, '^P6\s+(?:#[^\r\n]*\s+)?(\d+)\s+(\d+)\s+(\d+)\s')
  if (!$m.Success) { return $null }
  $w = [int]$m.Groups[1].Value
  $h = [int]$m.Groups[2].Value
  $off = $m.Index + $m.Length

  $centros = @(
    @(96, 144, 192),     # boton azul (cuerpo/cap del kit AQUA)
    @(64, 192, 80),      # boton verde
    @(192, 48, 48),      # boton rojo
    @(192, 128, 48),     # boton naranja
    @(184, 168, 16),     # boton amarillo
    @(192, 56, 176),     # boton magenta
    @(200, 200, 200)     # boton blanco/gris
  )
  $cuenta = New-Object int[] $centros.Count
  # Escritorio de Kira: fondo AQUA con degradado (azul/cian). El color solido
  # de antes ya no vale porque el escritorio lleva papel pintado: se cuenta la
  # familia de tonos (rojo bajo, verde y azul altos) muestreada en pantalla.
  $escritorio = 0
  $totalEscritorio = 0
  for ($y = 0; $y -lt $h; $y += 4) {
    for ($x = 0; $x -lt $w; $x += 4) {
      $p = $off + ($y * $w + $x) * 3
      $r = $b[$p]; $g = $b[$p + 1]; $bl = $b[$p + 2]
      $totalEscritorio++
      if ($r -le 90 -and $g -ge 90 -and $bl -ge 140) { $escritorio++ }
    }
  }
  # Botones: solo el area de la ventana.
  $xmax = [math]::Min(600, $w)
  $ymax = [math]::Min(240, $h)
  for ($y = 0; $y -lt $ymax; $y++) {
    for ($x = 0; $x -lt $xmax; $x++) {
      $p = $off + ($y * $w + $x) * 3
      $r = $b[$p]; $g = $b[$p + 1]; $bl = $b[$p + 2]
      for ($k = 0; $k -lt $centros.Count; $k++) {
        $c = $centros[$k]
        if ([math]::Abs($r - $c[0]) -le 45 -and [math]::Abs($g - $c[1]) -le 45 -and [math]::Abs($bl - $c[2]) -le 45) { $cuenta[$k]++ }
      }
    }
  }
  [pscustomobject]@{
    escritorio = [int](100 * $escritorio / $totalEscritorio)
    azul    = $cuenta[0]
    verde   = $cuenta[1]
    rojo    = $cuenta[2]
    naranja = $cuenta[3]
    amarillo= $cuenta[4]
    magenta = $cuenta[5]
    blanco  = $cuenta[6]
  }
}

# --- La prueba ---
# El arranque en TCG tarda un tiempo variable (decodifica fuentes y PNG antes
# del primer fotograma de Kira), asi que se reintenta hasta ver el escritorio.
function CapturaEscritorio([string]$nombre) {
  for ($i = 0; $i -lt 10; $i++) {
    Dump $nombre
    $a = Analizar "$tmp/$nombre.ppm"
    if ($null -ne $a -and $a.escritorio -ge 60) { break }
    Start-Sleep -Seconds 2
  }
}

CapturaEscritorio "toby-00-inicio"

# ESC deja el escritorio de Kira y vuelve al modo texto.
Tecla "esc"
Start-Sleep -Seconds 2

# Monta la aplicacion .toby (botones = PNG AQUA ensamblados). El primer
# fotograma decodifica las 21 piezas PNG, asi que se espera de sobra.
Orden "toby"
Start-Sleep -Seconds 15
Dump "toby-01-app"

# Ronda A: navegacion con flechas. El foco empieza en "Guardar"; dos flechas a
# la derecha lo dejan en "Salir", e Intro ejecuta window.close. Si las flechas
# no llegaran, Intro guardaria y los botones seguirian en pantalla.
Tecla "tab"
Start-Sleep -Milliseconds 400
Tecla "right"
Start-Sleep -Milliseconds 400
Tecla "right"
Start-Sleep -Milliseconds 400
Tecla "ret"
Start-Sleep -Seconds 2
CapturaEscritorio "toby-02-cerrada"

# Ronda B: volver a abrir y salir con ESC, que debe devolver tambien a Kira.
# Tras cerrar con Intro, Kira vuelve con sus ventanas de inicio, asi que el
# interprete solo recibe teclas despues de salir del escritorio con ESC.
Tecla "esc"
Start-Sleep -Seconds 2
Orden "toby"
Start-Sleep -Seconds 15
Tecla "esc"
Start-Sleep -Seconds 2
CapturaEscritorio "toby-03-texto"

$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
Write-Host "=== salida serie (ultimas lineas) ==="
Write-Host (($texto -split "`n" | Select-Object -Last 30) -join "`n")
Write-Host "=== fin ==="

# --- Veredicto ---
$ok = $true
function Comprobar([bool]$cond, [string]$txt) {
  if ($cond) { Write-Host "[ok] $txt" } else { Write-Host "[FALLO] $txt"; $script:ok = $false }
}

$i0 = Analizar "$tmp\toby-00-inicio.ppm"
$i1 = Analizar "$tmp\toby-01-app.ppm"
$i2 = Analizar "$tmp\toby-02-cerrada.ppm"
$i3 = Analizar "$tmp\toby-03-texto.ppm"

foreach ($i in @($i0, $i1, $i2, $i3)) {
  if ($null -ne $i) {
    Write-Host ("  captura: escritorio={0}%  azul={1} verde={2} rojo={3} naranja={4} amarillo={5} magenta={6} blanco={7}" -f `
      $i.escritorio, $i.azul, $i.verde, $i.rojo, $i.naranja, $i.amarillo, $i.magenta, $i.blanco)
  }
}

Comprobar ($null -ne $i0 -and $i0.escritorio -ge 60) "al arrancar esta el escritorio de Kira"
Comprobar ($texto -match "toby:\s*/usr/share/tobyui/demo\.toby") "la orden toby monta el documento .toby del disco"
Comprobar ($null -ne $i1 -and $i1.escritorio -le 5) "la aplicacion .toby tapa el escritorio de Kira"
# Los botones son PNG ensamblados: cada variante aporta su color real.
Comprobar ($null -ne $i1 -and $i1.azul -ge 60) "hay botones azules del kit AQUA"
Comprobar ($null -ne $i1 -and $i1.rojo -ge 60) "hay botones rojos del kit AQUA"
Comprobar ($null -ne $i1 -and $i1.naranja -ge 60) "hay botones naranjas del kit AQUA"
Comprobar ($null -ne $i1 -and $i1.verde -ge 60) "hay botones verdes del kit AQUA"
Comprobar ($null -ne $i1 -and $i1.amarillo -ge 60) "hay botones amarillos del kit AQUA"
Comprobar ($null -ne $i1 -and $i1.magenta -ge 60) "hay botones magentas del kit AQUA"
Comprobar ($null -ne $i1 -and $i1.blanco -ge 30) "hay boton blanco/gris del kit AQUA"

$botones2 = 0
if ($null -ne $i2) { $botones2 = $i2.azul + $i2.verde + $i2.rojo + $i2.naranja + $i2.amarillo + $i2.magenta }
Comprobar ($null -ne $i2 -and $i2.escritorio -ge 60) "Tab y flechas mueven el foco: Intro cierra la aplicacion"
Comprobar ($botones2 -le 400) "tras cerrar con flechas ya no quedan botones de la aplicacion"

$botones3 = 0
if ($null -ne $i3) { $botones3 = $i3.azul + $i3.verde + $i3.rojo + $i3.naranja + $i3.amarillo + $i3.magenta }
Comprobar ($null -ne $i3 -and $i3.escritorio -ge 60) "ESC sale de la aplicacion y Kira recupera su pantalla"
Comprobar ($botones3 -le 400) "tras ESC ya no quedan botones de la aplicacion"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panics en la serie"

if ($ok) {
  Write-Host "RESULTADO: OK (Toby UI con PNG verificado)"
} else {
  Write-Host "RESULTADO: FALLO"
}
