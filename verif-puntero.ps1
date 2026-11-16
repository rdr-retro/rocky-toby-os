# Prueba de `gui puntero N`: que la escala del cursor del escritorio se vea de
# verdad y que sobreviva a que el escritorio se oculte y se vuelva a poner.
#
# Por que no basta `verif-gui.ps1`: esa prueba solo mira el mensaje de serie
# ("gui: puntero del escritorio a Nx"), que se imprimia aunque el cambio no se
# viera. El fallo era que la escala vivia en `Comp` y `ocultar` la ponia a 1:
# para teclear hay que salir de Kira con ESC (que oculta el escritorio) y `gui`
# a secas llama a `reajustar` -> `ocultar`, asi que la escala nueva se borraba
# justo antes de poder verla.
#
# Aqui se mide en la captura (HMP `screendump`, PPM) la extension del puntero
# desde su esquina (cursor::MARGEN_INICIAL = 24 px): a 1x el dibujo es 13x18,
# asi que a 4x tiene que pasar de 26x36.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }
$tmp = "$env:LOCALAPPDATA\Temp\opencode"
if (!(Test-Path $tmp)) { New-Item -ItemType Directory -Path $tmp | Out-Null }

$log = "$tmp\puntero.txt"
if (Test-Path $log) { Remove-Item $log -Force }
foreach ($n in @("puntero-01-uno", "puntero-02-cuatro", "puntero-03-sigue", "puntero-04-consulta")) {
  $f = "$tmp\$n.ppm"
  if (Test-Path $f) { Remove-Item $f -Force }
}

$puerto = 4460
$qemuArgs = @(
  "-m", "256",
  "-boot", "order=a",
  "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
  "-drive", "if=ide,index=0,media=disk,format=raw,file=$raiz\rockydisk.img",
  "-serial", "file:$log",
  "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
  "-display", "none",
  "-no-reboot"
)
$p = Start-Process -FilePath $qemu -ArgumentList $qemuArgs -PassThru -RedirectStandardError "$tmp\qemu-err-puntero.txt" -RedirectStandardOutput "$tmp\qemu-out-puntero.txt"

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
  Start-Sleep -Milliseconds 900
}

# --- Analisis de una captura PPM (P6) ---
# azul = % de pixeles del azul exacto de Kira (muestreo cada 8 px);
# ex/ey = cuanto se aleja del margen (24) la tinta mas lejana (blanco o negro)
#         dentro de la ventana donde puede estar el puntero;
# tinta = pixeles de tinta contados en esa ventana.
$margen = 24
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
      $q = $off + ($y * $w + $x) * 3
      $n++
      if ([math]::Abs($b[$q] - 34) -le 6 -and [math]::Abs($b[$q + 1] - 62) -le 6 -and [math]::Abs($b[$q + 2] - 168) -le 8) { $azul++ }
    }
  }

  # La esquina del puntero esta en (24,24); la ventana llega hasta 24 + 160 px,
  # de sobra para la escala 8 (8 * 18 = 144 px de alto).
  $ex = 0; $ey = 0; $tinta = 0
  $x1 = [math]::Min($w - 1, $margen + 160)
  $y1 = [math]::Min($h - 1, $margen + 160)
  for ($y = 0; $y -le $y1; $y++) {
    for ($x = 0; $x -le $x1; $x++) {
      $q = $off + ($y * $w + $x) * 3
      $r = $b[$q]; $g = $b[$q + 1]; $bl = $b[$q + 2]
      if (($r -ge 200 -and $g -ge 200 -and $bl -ge 200) -or ($r -le 40 -and $g -le 40 -and $bl -le 40)) {
        $tinta++
        if ($x - $margen -gt $ex) { $ex = $x - $margen }
        if ($y - $margen -gt $ey) { $ey = $y - $margen }
      }
    }
  }

  [pscustomobject]@{ azul = [int](100 * $azul / $n); ex = $ex; ey = $ey; tinta = $tinta }
}

# --- La prueba ---
# Al arrancar el escritorio ya esta puesto con el puntero a 1x.
Dump "puntero-01-uno"

# Para teclear hay que salir con ESC (que oculta el escritorio: es justo el
# punto donde antes se perdia la escala).
Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui puntero 4"
Start-Sleep -Seconds 2
Dump "puntero-02-cuatro"

# Se oculta otra vez y se recupera con `gui` a secas (el otro camino que pasaba
# por `reajustar`): la escala tiene que seguir siendo 4.
Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui"
Start-Sleep -Seconds 2
Dump "puntero-03-sigue"

# `gui puntero` sin numero es solo una consulta: dice la escala y no toca la
# pantalla, asi que en modo texto tiene que quedarse en modo texto (antes
# volvia a poner el escritorio, que para una consulta es una sorpresa).
Tecla "esc"
Start-Sleep -Seconds 2
Orden "gui puntero"
Start-Sleep -Seconds 2
Dump "puntero-04-consulta"

$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = if (Test-Path $log) { [string](Get-Content $log -Raw) } else { "" }
Write-Host "=== salida serie (ultimas lineas) ==="
Write-Host (($texto -split "`n" | Select-Object -Last 45) -join "`n")
Write-Host "=== fin ==="

# --- Veredicto ---
$ok = $true
function Comprobar([bool]$cond, [string]$txt) {
  # El prefijo es el que cuenta `verif-todo.ps1` (`FALLO` al principio de linea),
  # asi que no se adorna con corchetes.
  if ($cond) { Write-Host "OK    $txt" } else { Write-Host "FALLO $txt"; $script:ok = $false }
}

$c1 = Analizar "$tmp\puntero-01-uno.ppm"
$c2 = Analizar "$tmp\puntero-02-cuatro.ppm"
$c3 = Analizar "$tmp\puntero-03-sigue.ppm"
$c4 = Analizar "$tmp\puntero-04-consulta.ppm"
foreach ($c in @($c1, $c2, $c3, $c4)) {
  if ($null -ne $c) {
    Write-Host ("  captura: azul={0}%  extension del puntero={1}x{2} px  tinta={3}" -f $c.azul, $c.ex, $c.ey, $c.tinta)
  }
}

Comprobar ($null -ne $c1 -and $c1.azul -ge 90) "el escritorio de Kira esta puesto al arrancar"
Comprobar ($null -ne $c1 -and $c1.ex -gt 5 -and $c1.ex -lt 30) "el puntero a 1x mide como el dibujo (13x18)"
Comprobar ($null -ne $c2 -and $c2.azul -ge 90) "tras 'gui puntero 4' vuelve el escritorio (no se queda en texto)"
Comprobar ($null -ne $c1 -and $null -ne $c2 -and $c2.ex -ge (2 * $c1.ex) -and $c2.ey -ge (2 * $c1.ey)) "el puntero a 4x se ve al menos el doble de grande"
Comprobar ($null -ne $c1 -and $null -ne $c3 -and $c3.ex -ge (2 * $c1.ex)) "la escala sobrevive a ocultar el escritorio y recuperarlo con 'gui'"
Comprobar ([string]$texto -match "gui\s*:\s*puntero del escritorio a 4x") "la orden lo dice por serie"
Comprobar ($null -ne $c4 -and $c4.azul -lt 50) "'gui puntero' sin numero no vuelve a poner el escritorio (es solo una consulta)"
Comprobar ([regex]::Matches([string]$texto, "puntero del escritorio a 4x").Count -ge 2) "la consulta dice la escala que quedo puesta (4x)"

Write-Host ""
if ($ok) {
  Write-Host "RESULTADO: OK (escala del puntero verificada)"
} else {
  Write-Host "RESULTADO: FALLO"
  exit 1
}
