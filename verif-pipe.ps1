# Prueba de la Fase 6 (tuberias anonimas, `pipe` de Linux 0.01):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - `pipe`: lanza un proceso de anillo 3 que crea una tuberia con `pipe` (syscall 22),
#     se bifurca con fork, el hijo escribe por el extremo de escritura y lo cierra,
#     y el padre se BLOQUEA en read hasta que llegan los datos (y el cierre del ultimo
#     escritor, que es el fin de tuberia: la segunda lectura da 0).
#   - `free` antes y despues comprueba que no se filtra ninguna tuberia;
#   - `ps` comprueba que no queda ningun zombi.
#   - termina.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-pipe.log"
if (Test-Path $log) { Remove-Item $log -Force }
$ppm = "$tmp\pipe.ppm"
if (Test-Path $ppm) { Remove-Item $ppm -Force }

$puerto = 5588
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

Orden "free"
Orden "pipe"
Start-Sleep -Seconds 2
Orden "free"
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
if ($texto -match "tuberia anonima \(pipe \+ fork\) OK") {
  Write-Host "[ok] el programa de tuberias termino con exito"
} else {
  Write-Host "[FALLO] no se vio el mensaje de exito del programa"
  $ok = $false
}
if ($texto -match "hijo: hola desde la tuberia") {
  Write-Host "[ok] los datos cruzaron la tuberia (eco del hijo en la consola)"
} else {
  Write-Host "[FALLO] no llego el mensaje del hijo por la tuberia"
  $ok = $false
}
if ($texto -match "segunda lectura = 0 \(fin de tuberia\)") {
  Write-Host "[ok] el cierre del ultimo escritor produjo el fin de tuberia (read = 0)"
} else {
  Write-Host "[FALLO] no se vio el fin de tuberia"
  $ok = $false
}
if ($texto -match "abiertas:\s+[1-9]") {
  Write-Host "[FALLO] alguna tuberia quedo abierta (fuga de recursos)"
  $ok = $false
} else {
  Write-Host "[ok] no quedo ninguna tuberia abierta tras terminar el proceso"
}
if ($texto -match "la tuberia fallo") {
  Write-Host "[FALLO] el programa de anillo 3 reporto un fallo interno"
  $ok = $false
}
if ($texto -match "zombi") {
  Write-Host "[FALLO] quedo un proceso zombi"
  $ok = $false
} else {
  Write-Host "[ok] no quedo ningun zombi"
}

if ($ok) { Write-Host "RESULTADO: OK (tuberia anonima verificada)" }
else { Write-Host "RESULTADO: FALLO" }
