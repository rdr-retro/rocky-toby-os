# Prueba de la Fase 9 (directorio de trabajo por proceso, el `fs/open.c` +
# `fs/namei.c` de Linux 0.01):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - `cwd`: lanza un proceso de anillo 3 que:
#       * normaliza con `chdir("/")` (syscall 80) y comprueba con `getcwd`
#         (syscall 79) que ve la raiz ("/");
#       * entra en `/home/escritorio` y comprueba que `getcwd` devuelve 16 bytes;
#       * abre `hola.rka` con ruta **relativa**: solo resuelve si el cwd es el
#         directorio correcto;
#       * comprueba que la misma relativa **no** ve `bienvenida` (que esta en la
#         raiz) y que la ruta **absoluta** `/bienvenida` si la ve;
#       * vuelve a la raiz y abre `bin/hola` con ruta relativa;
#       * termina.
#   - `pwd` comprueba que el `cd` del proceso **no** ha cambiado el del shell (el
#     directorio de trabajo es por proceso);
#   - `ps` comprueba que no queda ningun zombi.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-cwd.log"
if (Test-Path $log) { Remove-Item $log -Force }
$ppm = "$tmp\cwd.ppm"
if (Test-Path $ppm) { Remove-Item $ppm -Force }

$puerto = 5599
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

# Lanza el programa de prueba del directorio de trabajo.
Orden "cwd"
Start-Sleep -Seconds 3

# El `cd` del proceso no debe haber tocado el del shell.
Orden "pwd"
Start-Sleep -Seconds 1
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
if ($texto -match "tras chdir\(/\), getcwd ve la raiz") {
  Write-Host "[ok] `chdir('/')` + `getcwd` ven la raiz"
} else {
  Write-Host "[FALLO] getcwd no devolvio la raiz tras chdir('/')"
  $ok = $false
}
if ($texto -match "chdir entra en /home/escritorio") {
  Write-Host "[ok] `chdir('/home/escritorio')` funciono"
} else {
  Write-Host "[FALLO] chdir no entro en /home/escritorio"
  $ok = $false
}
if ($texto -match "getcwd ve /home/escritorio \(16 bytes\)") {
  Write-Host "[ok] `getcwd` devolvio la ruta nueva (16 bytes)"
} else {
  Write-Host "[FALLO] getcwd no devolvio la ruta del directorio nuevo"
  $ok = $false
}
if ($texto -match "open relativo encuentra hola.rka") {
  Write-Host "[ok] `open` relativo resolvio desde el directorio de trabajo"
} else {
  Write-Host "[FALLO] open relativo no encontro el fichero del cwd"
  $ok = $false
}
if ($texto -match "open relativo NO ve lo de la raiz") {
  Write-Host "[ok] la relativa no alcanzo lo que esta en la raiz (cwd de verdad)"
} else {
  Write-Host "[FALLO] la ruta relativa alcanzo la raiz (el cwd no se uso)"
  $ok = $false
}
if ($texto -match "open absoluto sigue viendo /bienvenida") {
  Write-Host "[ok] la ruta absoluta se sigue resolviendo desde la raiz"
} else {
  Write-Host "[FALLO] la ruta absoluta no funciono"
  $ok = $false
}
if ($texto -match "de vuelta a la raiz, open bin/hola") {
  Write-Host "[ok] tras volver a la raiz, la relativa encontro bin/hola"
} else {
  Write-Host "[FALLO] la relativa no funciono tras volver a la raiz"
  $ok = $false
}
if ($texto -match "cwd \(chdir \+ getcwd \+ rutas relativas\) OK") {
  Write-Host "[ok] el programa de directorio de trabajo llego al final"
} else {
  Write-Host "[FALLO] no se vio el mensaje de exito del programa"
  $ok = $false
}
if ($texto -match "la prueba de cwd fallo") {
  Write-Host "[FALLO] el programa se fue por la rama de error"
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
  Write-Host "RESULTADO: OK (cwd verificado)"
} else {
  Write-Host "RESULTADO: FALLO"
}
