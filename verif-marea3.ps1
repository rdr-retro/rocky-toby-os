# Verificacion de la Fase 3 (Marea): colas de listos, vencimientos y equidad.
#
# Arranca Rocky en QEMU sin pantalla, captura la salida por serie, sale del
# escritorio grafico con ESC y teclea ordenes en el interprete de comandos.
# Al final imprime el log de serie completo.
#
# Uso:  powershell -NoProfile -ExecutionPolicy Bypass -File verif-marea3.ps1
param(
    # Ordenes a teclear en el shell, en orden (sin el 'ret').
    [string[]]$Ordenes = @("test_plan", "metricas"),
    [int]$SegundosBoot = 10
)

$raiz = "C:\Users\rdr-ryzen\Desktop\kernel"
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
if (!(Test-Path $tmp)) { New-Item -ItemType Directory -Path $tmp | Out-Null }
$log = "$tmp\serial-marea3.log"
if (Test-Path $log) { Remove-Item $log -Force }

$puerto = 5633
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
$p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
    -RedirectStandardError "$tmp\marea3-err.txt" -RedirectStandardOutput "$tmp\marea3-out.txt"

# Conecta al monitor de QEMU (se reintenta hasta que el puerto escuche).
$cliente = $null
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    try { $cliente = New-Object System.Net.Sockets.TcpClient("127.0.0.1", $puerto); break }
    catch { if ($p.HasExited) { throw "QEMU termino (ExitCode=$($p.ExitCode))" } }
}
if ($null -eq $cliente) { throw "no se pudo conectar al monitor de QEMU" }
$f = $cliente.GetStream()
$w = New-Object System.IO.StreamWriter($f)
$w.AutoFlush = $true

Start-Sleep -Seconds $SegundosBoot

# ESC sale del escritorio grafico (Kira) al interprete de comandos.
$w.WriteLine("sendkey esc")
Start-Sleep -Seconds 2

function Tecla([string]$k) {
    $w.WriteLine("sendkey $k")
    # Teclado lento: el driver de PS/2 sondea a 1 ms y no debe perder teclas.
    Start-Sleep -Milliseconds 300
}

# Traduce un caracter a la tecla de `sendkey` (con shift cuando haga falta).
function TeclaChar([string]$c) {
    switch ($c) {
        "_" { Tecla "shift-minus" }
        "-" { Tecla "minus" }
        "/" { Tecla "slash" }
        "." { Tecla "dot" }
        " " { Tecla "spc" }
        default { Tecla $c }
    }
}

function Orden([string]$t) {
    foreach ($c in $t.ToCharArray()) { TeclaChar ([string]$c) }
    Start-Sleep -Milliseconds 800
    Tecla "ret"
    Start-Sleep -Milliseconds 3500
}

foreach ($o in $Ordenes) { Orden $o }

$w.WriteLine("quit")
Start-Sleep -Milliseconds 500
$cliente.Close()
if (!$p.HasExited) { Stop-Process -Id $p.Id -Force }

Write-Host "=== SERIE ($log) ==="
if (Test-Path $log) { Get-Content $log -Raw }
