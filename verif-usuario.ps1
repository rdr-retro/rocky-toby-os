# Prueba de la frontera usuario/nucleo (anillo 3 + int 0x80):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - ejecuta `usuario` (programa de ejemplo en anillo 3, que imprime por la
#     llamada al sistema write y termina con exit);
#   - lista las tareas con `ps`;
#   - vuelve a ejecutar `usuario` y termina.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-usuario.log"
if (Test-Path $log) { Remove-Item $log -Force }
$ppm = "$tmp\usuario.ppm"
if (Test-Path $ppm) { Remove-Item $ppm -Force }

$puerto = 5560
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

Orden "usuario"
Start-Sleep -Seconds 1
Orden "ps"
Orden "usuario"
Orden "ps"

Start-Sleep -Seconds 1
$escritor.WriteLine("screendump $ppm")
Start-Sleep -Seconds 2
$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

Write-Host "=== salida serie ==="
Get-Content $log -Raw
Write-Host "=== captura ==="
if (Test-Path $ppm) { Write-Host "ppm: $((Get-Item $ppm).Length) bytes" } else { Write-Host "sin captura" }
