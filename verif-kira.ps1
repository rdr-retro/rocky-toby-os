# Prueba de la Fase 12 (servidor Kira Window Server):
#   - arranca, sale del escritorio a modo texto (ESC);
#   - `kira`: lanza un proceso de anillo 3 que:
#       * abre /dev/kira;
#       * realiza el handshake Kira (byte-order LSB, version 11);
#       * recibe resource-id-base, resource-id-mask y root window;
#       * crea una ventana (CreateWindow);
#       * la mapea (MapWindow);
#       * recibe el evento Expose.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-kira.log"
if (Test-Path $log) { Remove-Item $log -Force }

$puerto = 5604
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

# Lanza el programa de prueba del servidor Kira.
Orden "kira"
Start-Sleep -Seconds 4

Start-Sleep -Seconds 1
$escritor.WriteLine("quit")
Start-Sleep -Seconds 1
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = if (Test-Path $log) { Get-Content $log -Raw } else { "" }
Write-Host "=== salida serie ==="
Write-Host $texto
Write-Host "=== fin ==="

# --- Veredicto ---
$ok = $true
# La salida del proceso de anillo 3 se entrelaza en la serie con el mensaje del
# shell (ambos escriben a la consola), asi que el primer patron se busca como
# regex tolerante a texto intercalado entre "anillo 3" y "lanzado".
$comprobaciones = @(
  @{ m = "kira_test: proceso en anillo 3[\s\S]{0,200}?lanzado \(PID"; t = "kira_test lanzado desde el shell"; re = $true },
  @{ m = "kira_test: evento 12 recibido OK"; t = "evento Expose recibido" },
  @{ m = "kira_test: cuatro ventanas y superficies de dibujo OK"; t = "handshake + CreateWindow + MapWindow + GC + dibujo completos" }
)
foreach ($c in $comprobaciones) {
  $encontrado = if ($c.re) { $texto -match $c.m } else { $texto -match [regex]::Escape($c.m) }
  if ($encontrado) {
    Write-Host "[ok] $($c.t)"
  } else {
    Write-Host "[FALLO] no se vio: $($c.m)"
    $ok = $false
  }
}

if ($texto -match "kira_test: no se pudo abrir") {
  Write-Host "[FALLO] no se pudo abrir /dev/kira"
  $ok = $false
}
if ($texto -match "kira_test: no hubo respuesta") {
  Write-Host "[FALLO] el servidor no respondio al handshake"
  $ok = $false
}
if ($texto -match "kira_test: setup rechazado") {
  Write-Host "[FALLO] el setup fue rechazado"
  $ok = $false
}
if ($texto -match "kira_test: sin evento Expose") {
  Write-Host "[FALLO] no llego el evento Expose"
  $ok = $false
}

if ($ok) {
  Write-Host "RESULTADO: OK (servidor Kira verificado)"
} else {
  Write-Host "RESULTADO: FALLO"
}
