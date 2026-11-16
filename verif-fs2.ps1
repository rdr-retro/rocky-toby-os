# Prueba de ejecucion: visor PNG (usa las librerias inflate+png) y escritura
# desde el kernel. Deja el disco para pasarle e2fsck despues.
#
# OJO con la distribucion es_ES: '/' se teclea con Shift+7.

$ErrorActionPreference = "Stop"
$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "C:\Users\rdr-ryzen\AppData\Local\Temp\opencode"
$log = "$tmp\serial-fs2.log"
if (Test-Path $log) { Remove-Item $log -Force }

$puerto = 5557
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
$err = "$tmp\qemu-err.txt"
$p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru -RedirectStandardError $err -RedirectStandardOutput "$tmp\qemu-out.txt"

# Espera a que el monitor TCP este escuchando (con reintentos).
$cliente = $null
for ($i = 0; $i -lt 30; $i++) {
  Start-Sleep -Milliseconds 500
  try {
    $cliente = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $puerto)
    break
  } catch {
    if ($p.HasExited) { throw "QEMU termino enseguida (ExitCode=$($p.ExitCode))" }
  }
}
if ($null -eq $cliente) { throw "no se pudo conectar al monitor en el puerto $puerto" }
$flujo = $cliente.GetStream()
$escritor = New-Object System.IO.StreamWriter($flujo)
$escritor.AutoFlush = $true
# Espera a que el invitado termine de arrancar (hasta el escritorio).
Start-Sleep -Seconds 7

function Nombre([char]$ch) {
  switch -CaseSensitive ([string]$ch) {
    "/" { "shift-7" }
    "." { "dot" }
    "-" { "slash" }
    " " { "spc" }
    "_" { "shift+slash" }
    default { [string]$ch }
  }
}
function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 80 }
function Teclas([string]$t) { foreach ($c in $t.ToCharArray()) { Tecla (Nombre $c) } }
function Enter { Tecla "ret"; Start-Sleep -Milliseconds 700 }

Tecla "esc"          # sale del escritorio al interprete en modo texto
Start-Sleep -Seconds 1

# --- Visor PNG: ejercita png + inflate (2,4 MiB) ---
Teclas "imagen /home/isma.png"; Enter
Start-Sleep -Seconds 6
Tecla "esc"          # cierra el visor y vuelve al texto
Start-Sleep -Seconds 1

# --- Escritura desde el kernel ---
Teclas "mkdir /prueba"; Enter
Teclas "write /prueba/hola"; Enter
Teclas "hola desde ext4"; Enter
Teclas "."; Enter
Teclas "cat /prueba/hola"; Enter
Teclas "ls /"; Enter
Teclas "df"; Enter

Start-Sleep -Seconds 1
$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

Write-Host "=== salida serie ==="
Get-Content $log -Raw
