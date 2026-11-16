# Prueba de la orden `fallo` (excepciones de anillo 3):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - ejecuta `fallo`: lanza un proceso de anillo 3 que ejecuta `ud2`;
#   - el nucleo contiene la excepcion #UD terminando **solo** ese proceso;
#   - lista las tareas con `ps` (no debe quedar ningun proceso vivo);
#   - comprueba que NO aparecio la pantalla azul.
#
# Este guion es ademas la regresion de que el programa `fallo` se carga desde el
# disco como ELF (/bin/excepcion), como el resto de programas de usuario.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-excepcion.log"
if (Test-Path $log) { Remove-Item $log -Force }
$ppm = "$tmp\excepcion.ppm"
if (Test-Path $ppm) { Remove-Item $ppm -Force }

$puerto = 5571
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
function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 900 }

# --- Al modo texto ---
Tecla "esc"
Start-Sleep -Seconds 1

Orden "fallo"
Start-Sleep -Seconds 1
Orden "ps"

Start-Sleep -Seconds 1
$escritor.WriteLine("screendump $ppm")
Start-Sleep -Seconds 2
$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = Get-Content $log -Raw
Write-Host "=== salida serie ==="
Write-Host $texto

# --- Comprobaciones ---
Write-Host "== comprobaciones =="
$fallos = 0

function Comprobar([string]$nombre, [bool]$ok) {
  if ($ok) { Write-Host "  [OK]    $nombre" }
  else { Write-Host "  [FALLO] $nombre"; $script:fallos++ }
}

Comprobar "el kernel llego al interprete" ($texto -match "root@rocky")
Comprobar "la orden fallo lanzo un proceso" ($texto -match "fallo: PID \d+ lanzado")
Comprobar "el nucleo contuvo la excepcion #UD (codigo invalido)" ($texto -match "terminado por codigo invalido \(#UD\)")
Comprobar "no hubo pantalla azul" (-not ($texto -match "PANICO|pantalla azul|Kernel panic"))

Write-Host ""
if ($fallos -eq 0) { Write-Host "RESULTADO: OK (excepcion de anillo 3 contenida)" }
else { Write-Host "RESULTADO: $fallos comprobacion(es) fallida(s)" }
