# Prueba de la Fase 8 (TTY / terminal, el `kernel/tty_io.c` de Linux 0.01):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - `tty`: lanza un proceso de anillo 3 que:
#       * lee el `termios` de la consola con `ioctl(0, TCGETS)` (syscall 16) y
#         comprueba que viene en modo canonico (`ICANON`) y con eco (`ECHO`);
#       * estrena sesion/grupo con `setsid` (syscall 112): devuelve su PID;
#       * apaga `ECHO|ICANON` con `TCSETS`, relee con `TCGETS` y comprueba que se
#         ha aplicado, y luego restaura los ajustes;
#       * fija y relee el grupo en primer plano (`TIOCSPGRP`/`TIOCGPGRP`);
#       * por ultimo hace `read(0, ...)`: la disciplina canonica del terminal
#         espera a un salto de linea y hace el eco; el programa reescribe la linea.
#   - el guion teclea una linea ("hola tty") mientras el proceso espera en `read`;
#   - `ps` comprueba que no queda ningun zombi.
#   - termina.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-tty.log"
if (Test-Path $log) { Remove-Item $log -Force }
$ppm = "$tmp\tty.ppm"
if (Test-Path $ppm) { Remove-Item $ppm -Force }

$puerto = 5598
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

# Lanza el programa de prueba del terminal (imprime sus comprobaciones por serie
# y se queda esperando en `read`).
Orden "tty"
Start-Sleep -Seconds 3

# Teclea una linea para que el proceso la lea por la consola.
Teclas "hola tty"
Tecla "ret"
Start-Sleep -Seconds 2

Orden "ps"
Orden "ps"

Start-Sleep -Seconds 1
$escritor.WriteLine("screendump $ppm")
Start-Sleep -Seconds 2
$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
Write-Host "=== salida serie ==="
Write-Host $texto
Write-Host "=== captura ==="
if (Test-Path $ppm) { Write-Host "ppm: $((Get-Item $ppm).Length) bytes" } else { Write-Host "sin captura" }

# --- Veredicto ---
$ok = $true
if ($texto -match "ICANON=true ECHO=true ISIG=true") {
  Write-Host "[ok] el shell ve la consola en modo canonico con eco e ISIG"
} else {
  Write-Host "[FALLO] el shell no vio el termios por defecto del terminal"
  $ok = $false
}
if ($texto -match "TCGETS ok") {
  Write-Host "[ok] `ioctl(0, TCGETS)` entrego el termios del terminal"
} else {
  Write-Host "[FALLO] el proceso no pudo leer el termios con TCGETS"
  $ok = $false
}
if ($texto -match "setsid creo sesion y grupo nuevos") {
  Write-Host "[ok] `setsid` estreno sesion/grupo (devolvio el PID)"
} else {
  Write-Host "[FALLO] `setsid` no devolvio el PID del proceso"
  $ok = $false
}
if ($texto -match "modo crudo \(ECHO/ICANON off\)") {
  Write-Host "[ok] `TCSETS` apago ECHO/ICANON y `TCGETS` lo confirmo"
} else {
  Write-Host "[FALLO] los cambios de termios con TCSETS no se aplicaron"
  $ok = $false
}
if ($texto -match "TIOCSPGRP / TIOCGPGRP ok") {
  Write-Host "[ok] el grupo en primer plano se fijo y se releyo"
} else {
  Write-Host "[FALLO] TIOCSPGRP/TIOCGPGRP no funcionaron"
  $ok = $false
}
if ($texto -match "hola tty") {
  Write-Host "[ok] la consola leyo (con eco) la linea tecleada"
} else {
  Write-Host "[FALLO] no se vio la linea tecleada devuelta por el terminal"
  $ok = $false
}
if ($texto -match "tty \(termios \+ ioctl \+ setsid\) OK") {
  Write-Host "[ok] el programa de terminal llego al final"
} else {
  Write-Host "[FALLO] no se vio el mensaje de exito del programa"
  $ok = $false
}
if ($texto -match "la prueba de tty fallo") {
  Write-Host "[FALLO] el programa de terminal se fue por la rama de error"
  $ok = $false
} else {
  Write-Host "[ok] el programa no se fue por la rama de error"
}
if ($texto -match "zombi") {
  Write-Host "[FALLO] quedo algun proceso zombi"
  $ok = $false
} else {
  Write-Host "[ok] no quedo ningun zombi"
}

if ($ok) {
  Write-Host "RESULTADO: OK (tty verificado)"
} else {
  Write-Host "RESULTADO: FALLO"
}
