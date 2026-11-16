# Prueba del subsistema de emergencia (la pantalla azul de rocky).
#
# Cada caso arranca su propio QEMU, porque el panico detiene la maquina y no se
# puede seguir tecleando. Se mira la salida por el puerto serie (que la pantalla
# de emergencia escribe ademas del video) y, en el caso 'fb', la captura.
#
#   panico kernel  panic! de Rust -> pantalla azul y parada
#   panico ud      excepcion #UD (instruccion invalida)
#   panico gp      excepcion #GP (direccion no canonica)
#   panico pf      excepcion #PF y comprobacion de CR2
#   panico df      doble fallo (#DF) sobre la pila de emergencia (IST)
#   panico fb      pantalla azul sobre el framebuffer VESA (se mira la captura)
#   fallo          #UD de un proceso de anillo 3: solo muere el proceso
#
# Un #DF mal atendido se convierte en triple falta y QEMU sale (no-reboot): ese
# caso se marca como FALLO si la maquina se reinicia en vez de informar.

$raiz = $PSScriptRoot
$qemu = "C:\Users\rdr-ryzen\AppData\Local\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }
$tmp = "$env:LOCALAPPDATA\Temp\opencode"

# Canal del monitor del QEMU en curso (lo fija Caso).
$script:escritor = $null

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
function Tecla([string]$k) { $script:escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 70 }
function Teclas([string]$t) { foreach ($c in $t.ToCharArray()) { Tecla (Nombre $c) } }
function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 900 }

# Arranca QEMU, espera al escritorio, sale a texto (ESC), ejecuta el guion y
# devuelve el texto del puerto serie. Deja la captura en $captura (si se pide).
function Caso([string]$nombre, [int]$puerto, [string]$captura, [scriptblock]$guion) {
  $log = "$tmp\serial-$nombre.log"
  if (Test-Path $log) { Remove-Item $log -Force }
  if ($captura -and (Test-Path $captura)) { Remove-Item $captura -Force }
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
  $p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
      -RedirectStandardError "$tmp\qemu-$nombre-err.txt" `
      -RedirectStandardOutput "$tmp\qemu-$nombre-out.txt"
  $cliente = $null
  for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    try { $cliente = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $puerto); break }
    catch { if ($p.HasExited) { break } }
  }
  if ($cliente) {
    $flujo = $cliente.GetStream()
    $script:escritor = New-Object System.IO.StreamWriter($flujo)
    $script:escritor.AutoFlush = $true
    Start-Sleep -Seconds 8
    Tecla "esc"
    Start-Sleep -Seconds 1
    & $guion
    if ($captura) {
      Start-Sleep -Seconds 1
      $script:escritor.WriteLine("screendump $captura")
      Start-Sleep -Seconds 2
    }
    $cliente.Close()
  }
  Start-Sleep -Seconds 1
  # Si la maquina no se ha parado sola (caso 'fallo'), se cierra a mano.
  $reiniciada = $p.HasExited
  if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
  Start-Sleep -Milliseconds 500
  $texto = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
  return [pscustomobject]@{ Texto = $texto; Reiniciada = $reiniciada }
}

# Analiza una captura P6 de QEMU: porcentaje de fondo azul y de texto claro.
# (Ojo: el color claro tiene el azul como canal dominante, asi que el fondo se
# distingue por ser un azul saturado, no por "ser mas azul".)
function AnalizarCaptura([string]$ppm) {
  if (!(Test-Path $ppm)) { return [pscustomobject]@{ Azul = -1.0; Claro = -1.0 } }
  $b = [System.IO.File]::ReadAllBytes($ppm)
  $nl = 0; $i = 0
  while ($i -lt $b.Length -and $nl -lt 3) {
    if ($b[$i] -eq 10) { $nl++ }
    $i++
  }
  $off = $i
  $npix = [int](($b.Length - $off) / 3)
  if ($npix -le 0) { return [pscustomobject]@{ Azul = -1.0; Claro = -1.0 } }
  $azul = 0; $claro = 0; $muestras = 0
  for ($k = 0; $k -lt $npix; $k += 53) {
    $r = $b[$off + $k * 3]; $g = $b[$off + $k * 3 + 1]; $bl = $b[$off + $k * 3 + 2]
    $muestras++
    # Azul de fondo: canales rojo y verde bajos (el texto claro tiene ambos altos).
    if ($bl -gt 80 -and $r -lt 100 -and $g -lt 100) { $azul++ }
    elseif ($r -gt 200 -and $g -gt 200 -and $bl -gt 200) { $claro++ }
  }
  return [pscustomobject]@{
    Azul  = (100.0 * $azul / [Math]::Max(1, $muestras))
    Claro = (100.0 * $claro / [Math]::Max(1, $muestras))
  }
}

$fallos = 0
function Comprobar([string]$nombre, [bool]$ok, [string]$detalle) {
  if ($ok) { Write-Host "  [OK]    $nombre" }
  else { Write-Host "  [FALLO] $nombre  ($detalle)"; $script:fallos++ }
}

# --- caso 1: panic! de Rust -------------------------------------------------
Write-Host "== 1) panico kernel (panic! de Rust) =="
$r = Caso "kernel" 5580 $null { Orden "panico kernel"; Start-Sleep -Seconds 2 }
Comprobar "titulo ROCKY KERNEL PANIC" ($r.Texto -match "ROCKY KERNEL PANIC") "sin titulo"
Comprobar "motivo del panic!"          ($r.Texto -match "prueba de panico controlada") "sin motivo"
Comprobar "vector no aplicable"        ($r.Texto -match "no aplicable \(panic! de Rust\)") "sin nota de vector"
Comprobar "CR3 capturado"              ($r.Texto -match "CR3     : [0-9A-F]{16}") "sin CR3"

# --- caso 2: #UD ------------------------------------------------------------
Write-Host "== 2) panico ud (excepcion #UD) =="
$r = Caso "ud" 5581 $null { Orden "panico ud"; Start-Sleep -Seconds 2 }
Comprobar "titulo ROCKY KERNEL PANIC" ($r.Texto -match "ROCKY KERNEL PANIC") "sin titulo"
Comprobar "causa #UD"                  ($r.Texto -match "codigo invalido \(#UD\)") "sin causa #UD"
Comprobar "vector 0x0006"              ($r.Texto -match "Vector  : 0x0006") "sin vector 6"

# --- caso 3: #GP ------------------------------------------------------------
Write-Host "== 3) panico gp (excepcion #GP) =="
$r = Caso "gp" 5582 $null { Orden "panico gp"; Start-Sleep -Seconds 2 }
Comprobar "titulo ROCKY KERNEL PANIC" ($r.Texto -match "ROCKY KERNEL PANIC") "sin titulo"
Comprobar "causa #GP"                  ($r.Texto -match "fallo general de proteccion \(#GP\)") "sin causa #GP"
Comprobar "vector 0x000D"              ($r.Texto -match "Vector  : 0x000D") "sin vector 13"

# --- caso 4: #PF con CR2 ----------------------------------------------------
Write-Host "== 4) panico pf (excepcion #PF, se comprueba CR2) =="
$r = Caso "pf" 5583 $null { Orden "panico pf"; Start-Sleep -Seconds 2 }
Comprobar "titulo ROCKY KERNEL PANIC" ($r.Texto -match "ROCKY KERNEL PANIC") "sin titulo"
Comprobar "causa #PF"                  ($r.Texto -match "fallo de pagina \(#PF\)") "sin causa #PF"
Comprobar "vector 0x000E"              ($r.Texto -match "Vector  : 0x000E") "sin vector 14"
Comprobar "CR2 = direccion probada"    ($r.Texto -match "CR2     : 0xFFFFC00000000000") "CR2 distinto"
Comprobar "explicacion de CR2"         ($r.Texto -match "direccion del #PF") "sin etiqueta CR2"

# --- caso 5: #DF sobre la pila de emergencia --------------------------------
Write-Host "== 5) panico df (doble fallo) =="
$r = Caso "df" 5584 $null { Orden "panico df"; Start-Sleep -Seconds 3 }
Comprobar "titulo ROCKY KERNEL PANIC" ($r.Texto -match "ROCKY KERNEL PANIC") "sin titulo"
Comprobar "causa #DF"                  ($r.Texto -match "doble fallo \(#DF\)") "sin causa #DF"
Comprobar "no se reinicio"             (-not $r.Reiniciada) "la maquina se reinicio (triple falta)"

# --- caso 6: pantalla azul sobre framebuffer VESA ---------------------------
Write-Host "== 6) panico fb (framebuffer VESA) =="
$captura = "$tmp\panico-fb.ppm"
$r = Caso "fb" 5585 $captura { Orden "panico fb"; Start-Sleep -Seconds 3 }
Comprobar "titulo ROCKY KERNEL PANIC" ($r.Texto -match "ROCKY KERNEL PANIC") "sin titulo"
$cap = AnalizarCaptura $captura
Comprobar "fondo azul en la captura"  ($cap.Azul -gt 60.0) ("azul=$([Math]::Round($cap.Azul,1))%")
Comprobar "texto claro en la captura" ($cap.Claro -gt 0.5) ("claro=$([Math]::Round($cap.Claro,2))%")
Write-Host "  captura: $(if (Test-Path $captura) { (Get-Item $captura).Length } else { 0 }) bytes"

# --- caso 7: excepcion de usuario (no tumba el sistema) ---------------------
Write-Host "== 7) fallo (excepcion #UD de un proceso de anillo 3) =="
$r = Caso "fallo" 5586 $null {
  Orden "fallo"
  Start-Sleep -Seconds 2
  Orden "echo el sistema sigue vivo"
  Start-Sleep -Seconds 1
}
Comprobar "el proceso se contuvo"      ($r.Texto -match "terminado por codigo invalido \(#UD\)") "sin mensaje de contencion"
Comprobar "el sistema sigue vivo"      ($r.Texto -match "el sistema sigue vivo") "no respondio despues"
Comprobar "no hubo pantalla azul"      (-not ($r.Texto -match "ROCKY KERNEL PANIC")) "aparecio la pantalla azul"

Write-Host ""
if ($fallos -eq 0) {
  Write-Host "RESULTADO: OK (subsistema de emergencia verificado)"
} else {
  Write-Host "RESULTADO: $fallos comprobacion(es) fallida(s)"
}
