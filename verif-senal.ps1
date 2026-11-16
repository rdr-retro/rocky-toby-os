# Prueba de la Fase 7 (senales, el `kernel/signal.c` de Linux 0.01):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - `senal`: lanza un proceso de anillo 3 que:
#       * instala un manejador de SIGUSR1 con `sigaction` (syscall 13);
#       * se envia SIGUSR1 a si mismo con `kill` (syscall 62): la senal se
#         entrega AL VOLVER de la llamada;
#       * el manejador imprime y vuelve con `sigreturn` (syscall 15);
#       * sigue el programa principal y, al final, se envia SIGTERM, cuya
#         accion por defecto es terminar el proceso.
#   - `ps` comprueba que no queda ningun zombi.
#   - termina.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-senal.log"
if (Test-Path $log) { Remove-Item $log -Force }
$ppm = "$tmp\senal.ppm"
if (Test-Path $ppm) { Remove-Item $ppm -Force }

$puerto = 5591
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

Orden "senal"
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
if ($texto -match "manejador SIGUSR1 en ejecucion") {
  Write-Host "[ok] el manejador de SIGUSR1 corrio en anillo 3"
} else {
  Write-Host "[FALLO] no se ejecuto el manejador de SIGUSR1"
  $ok = $false
}
if ($texto -match "de vuelta del manejador") {
  Write-Host "[ok] sigreturn volvio al programa principal"
} else {
  Write-Host "[FALLO] el proceso no volvio del manejador"
  $ok = $false
}
if ($texto -match "senales \(sigaction \+ kill \+ sigreturn\) OK") {
  Write-Host "[ok] el programa de senales llego al final de su parte capturable"
} else {
  Write-Host "[FALLO] no se vio el mensaje de exito del programa"
  $ok = $false
}
if ($texto -match "terminado por la senal 15 \(SIGTERM\)") {
  Write-Host "[ok] SIGTERM (accion por defecto) termino el proceso"
} else {
  Write-Host "[FALLO] no se vio la muerte del proceso por SIGTERM"
  $ok = $false
}
if ($texto -match "no deberia verse") {
  Write-Host "[FALLO] el proceso siguio tras SIGTERM (la accion por defecto no actuo)"
  $ok = $false
} else {
  Write-Host "[ok] el proceso no continuo tras SIGTERM"
}
if ($texto -match "la prueba de senales fallo") {
  Write-Host "[FALLO] el programa de anillo 3 reporto un fallo interno"
  $ok = $false
}
if ($texto -match "zombi") {
  Write-Host "[FALLO] quedo un proceso zombi"
  $ok = $false
} else {
  Write-Host "[ok] no quedo ningun zombi"
}

if ($ok) { Write-Host "RESULTADO: OK (senales verificadas)" }
else { Write-Host "RESULTADO: FALLO" }
