# Prueba de la Fase 11 (el sistema de ficheros visto desde el anillo 3: el
# `fs/link.c`, `fs/namei.c` y `fs/stat.c` de Linux 0.01):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - `fs3`: lanza un proceso de anillo 3 que, **sin ayuda del nucleo**, hace
#       * `mkdir` (crea y rechaza repetir);
#       * `open` + `write` + `read` sobre un fichero creado por el propio
#         proceso, y comprueba el contenido;
#       * `stat`: inodo, modo 100644, un enlace y el tamano escrito;
#       * `link`: un segundo nombre para el **mismo** inodo (nlink = 2);
#       * `rename` **entre directorios**: el registro se mueve con sus datos;
#       * `getdents64` sobre un descriptor de directorio, con el buffer
#         pequeno devolviendo error (no un falso "fin") y el cookie de la
#         posicion respetado entre llamadas;
#       * `unlink`/`rmdir`: cada uno rechaza el tipo que no le toca;
#       * `access` y `fstat`;
#       * `sync`, y deja el volumen limpio (borra todo lo que creo);
#   - `ln` + `stat` comprueban el enlace duro tambien desde el interprete;
#   - `ps` comprueba que no queda ningun zombi.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-fs3.log"
if (Test-Path $log) { Remove-Item $log -Force }
$ppm = "$tmp\fs3.ppm"
if (Test-Path $ppm) { Remove-Item $ppm -Force }

$puerto = 5601
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

# Lanza el programa de prueba del filesystem en anillo 3.
Orden "fs3"
Start-Sleep -Seconds 6

# El enlace duro tambien desde el interprete: dos nombres, un mismo inodo.
Orden "ln /bin/mensaje /bin/mensaje-duro"
Orden "stat /bin/mensaje-duro"
Orden "rm /bin/mensaje-duro"

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
$comprobaciones = @(
  @{ m = "mkdir crea el directorio y rechaza repetirlo"; t = "mkdir crea y rechaza repetir" },
  @{ m = "open, write y read devuelven el contenido";     t = "open+write+read del fichero" },
  @{ m = "stat da size, mode, nlink e ino correctos";     t = "stat con los metadatos" },
  @{ m = "link anade un segundo nombre al mismo inodo";  t = "link (enlace duro)" },
  @{ m = "rename mueve el fichero entre directorios";     t = "rename entre directorios" },
  @{ m = "getdents64 lista el directorio y respeta el buffer"; t = "getdents64 sobre un descriptor" },
  @{ m = "unlink y rmdir distinguen fichero de directorio"; t = "unlink/rmdir por tipo" },
  @{ m = "access y fstat responden lo esperado";          t = "access y fstat" }
)
foreach ($c in $comprobaciones) {
  if ($texto -match [regex]::Escape($c.m)) {
    Write-Host "[ok] $($c.t)"
  } else {
    Write-Host "[FALLO] no se vio: $($c.m)"
    $ok = $false
  }
}

if ($texto -match "rocky: el filesystem de anillo 3 funciona") {
  Write-Host "[ok] el programa de filesystem llego al final"
} else {
  Write-Host "[FALLO] no se vio el mensaje de exito del programa"
  $ok = $false
}
if ($texto -match "la prueba de filesystem fallo") {
  Write-Host "[FALLO] el programa se fue por la rama de error"
  $ok = $false
} else {
  Write-Host "[ok] el programa no se fue por la rama de error"
}
if ($texto -match "enlaces: 2") {
  Write-Host "[ok] el enlace duro del interprete dejo dos nombres en el mismo inodo"
} else {
  Write-Host "[FALLO] stat no encontro dos enlaces tras el ln"
  $ok = $false
}
if ($texto -match "modo:    100644") {
  Write-Host "[ok] stat muestra el modo POSIX del inodo"
} else {
  Write-Host "[FALLO] stat no muestra el modo 100644"
  $ok = $false
}
if ($texto -match "zombi") {
  Write-Host "[FALLO] quedo algun proceso zombi"
  $ok = $false
} else {
  Write-Host "[ok] no quedo ningun zombi"
}

if ($ok) {
  Write-Host "RESULTADO: OK (filesystem en anillo 3 verificado)"
} else {
  Write-Host "RESULTADO: FALLO"
}
