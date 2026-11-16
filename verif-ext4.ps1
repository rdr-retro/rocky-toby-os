# Verificacion del sistema de ficheros ext4 de rocky.
#
# Arranca QEMU sin ventana, inyecta ordenes por el monitor (sendkey) y lee la
# salida por el puerto serie. Al terminar, deja el disco listo para pasarle
# e2fsck.
#
#   powershell -ExecutionPolicy Bypass -File verif-ext4.ps1

$ErrorActionPreference = "Stop"
$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "C:\Users\rdr-ryzen\AppData\Local\Temp\opencode"
$log = "$tmp\serial-ext4.log"
if (Test-Path $log) { Remove-Item $log -Force }

$puerto = 5555
$args = @(
  "-m", "256",
  "-boot", "order=a",
  "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
  "-drive", "if=ide,index=0,media=disk,format=raw,file=$raiz\rockydisk.img",
  "-serial", "file:$log",
  "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
  "-display", "none",
  "-no-reboot"
)
$p = Start-Process -FilePath $qemu -ArgumentList $args -PassThru

Start-Sleep -Seconds 8

$cliente = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $puerto)
$flujo = $cliente.GetStream()
$escritor = New-Object System.IO.StreamWriter($flujo)
$escritor.AutoFlush = $true
Start-Sleep -Milliseconds 400

function Nombre([char]$ch) {
  switch -CaseSensitive ([string]$ch) {
    "/" { "shift-7" }
    "." { "dot" }
    "-" { "minus" }
    " " { "spc" }
    "_" { "shift+minus" }
    default { [string]$ch }
  }
}

function Tecla([string]$k) {
  $escritor.WriteLine("sendkey $k")
  Start-Sleep -Milliseconds 80
}

function Teclas([string]$texto) {
  foreach ($ch in $texto.ToCharArray()) { Tecla (Nombre $ch) }
}

function Enter { Tecla "ret"; Start-Sleep -Milliseconds 700 }

# Escapa del escritorio grafico al interprete en modo texto.
Tecla "esc"
Start-Sleep -Seconds 1

Teclas "ls"; Enter
Teclas "cat /bienvenida"; Enter
Teclas "df"; Enter
Teclas "ls /home/escritorio"; Enter
Teclas "mkdir /prueba"; Enter
Teclas "write /prueba/hola"; Enter
Teclas "hola desde ext4"; Enter
Teclas "."; Enter
Teclas "cat /prueba/hola"; Enter
Teclas "tree /"; Enter

Start-Sleep -Seconds 1
$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

Write-Host "=== salida serie ==="
Get-Content $log -Raw
