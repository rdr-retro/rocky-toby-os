# Verificacion del bus PCI, de la GPU AMD (fases 13 y 14) y del cursor por
# hardware (fase 14b).
#
# Los drivers viven ahora en `kernel/src/drivers/` (por clase y fabricante): el
# bus PCI solo enumera y describe; lo que se *hace* con cada dispositivo esta en
# su driver. Aqui se comprueba el de AMD, que es el unico que hay.
#
# Se arranca QEMU dos veces:
#   1) Sin GPU AMD (la VGA de Bochs, 1234:1111): el bus se enumera, `lspci`
#      lista, `gpu` avisa de que no hay GPU AMD (pero localiza la de Bochs) y
#      `cursor` dice que no hay ninguna con cursor por hardware.
#   2) Con una GPU AMD emulada (`-device ati-vga`, 1002:5046): `gpu` la
#      reconoce, saca sus BAR y ensena el bloque de pantalla; `cursor` deja el
#      bitmap en la VRAM (y lo relee para comprobarlo) y, como en esta GPU
#      antigua no cabe el bloque del cursor, se niega a programarlo.
#
# Toda la salida va por la consola y se refleja en el puerto serie.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"

function Ejecutar($extra, $log, $puerto) {
  if (Test-Path $log) { Remove-Item $log -Force }
  $opciones = @(
    "-m", "256",
    "-boot", "order=a",
    "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
    "-drive", "if=ide,index=0,media=disk,format=raw,file=$raiz\rockydisk.img"
  )
  if ($extra.Count -gt 0) { $opciones += $extra }
  $opciones += @(
    "-serial", "file:$log",
    "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
    "-display", "none",
    "-no-reboot"
  )
  $p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
        -RedirectStandardError "$tmp\qemu-pci-$puerto-err.txt" `
        -RedirectStandardOutput "$tmp\qemu-pci-$puerto-out.txt"

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
  Start-Sleep -Seconds 11

  function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 80 }
  function Teclas([string]$t) { foreach ($c in $t.ToCharArray()) { Tecla ([string]$c) } }
  function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 1500 }

  # ESC deja el escritorio de Kira y devuelve la consola de texto.
  Tecla "esc"; Start-Sleep -Seconds 2
  Orden "lspci"
  Orden "gpu"
  Orden "cursor"
  Start-Sleep -Seconds 2

  $escritor.WriteLine("quit"); Start-Sleep -Seconds 1
  $cliente.Close()
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
  Start-Sleep -Seconds 1
  return (Get-Content $log -Raw)
}

$texto = Ejecutar @() "$tmp\rocky-pci.log" 5611
$textoAmd = Ejecutar @("-device", "ati-vga") "$tmp\rocky-pci-amd.log" 5612

Write-Host "=== 1) QEMU normal: ultimas 30 lineas ==="
($texto -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 30) | Out-String -Width 200 | Write-Host
Write-Host "=== 2) QEMU con GPU AMD (ati-vga): ultimas 20 lineas ==="
($textoAmd -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 20) | Out-String -Width 200 | Write-Host
Write-Host "=== fin ==="

$ok = $true
function Comprobar([bool]$cond, [string]$msg) {
  if ($cond) { Write-Host "OK   $msg" } else { Write-Host "FALLO $msg"; $script:ok = $false }
}

# --- Arranque normal (sin GPU AMD) ---
Comprobar ($texto -match "rocky: PCI: \d+ dispositivos") "el arranque imprime el resumen del bus PCI"
Comprobar ($texto -match "(?m)^Bus PCI: \d+ funciones") "la orden lspci lista las funciones del bus"
Comprobar ($texto -match "8086:1237") "aparece el puente host i440FX (8086:1237)"
Comprobar ($texto -match "1234:1111") "aparece la VGA de QEMU/Bochs (1234:1111)"
Comprobar ($texto -match "\[0300\]") "la VGA se clasifica como display (clase 0300)"
Comprobar ($texto -match "gpu: no hay ninguna GPU AMD") "gpu avisa de que no hay GPU AMD en QEMU"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panics en el arranque normal"

# --- Arranque con GPU AMD emulada (ati-vga) ---
Comprobar ($textoAmd -match "rocky: PCI: \d+ dispositivos\.") "el arranque imprime el resumen del bus PCI"
Comprobar ($textoAmd -match "(?m)^rocky: GPU AMD 1002:5046 en 00:03.0") "el driver de AMD reconoce la GPU al arrancar"
Comprobar ($textoAmd -match "(?m)^GPU AMD 1002:5046") "la orden gpu identifica la GPU AMD (1002:5046)"
Comprobar ($textoAmd -match "(?m)^  comando 0x") "gpu muestra el registro de comando"
Comprobar ($textoAmd -match "(?m)^  BAR0\s+MMIO\s+base 0x[0-9A-F]+\s+tamano") "gpu mide el BAR0 (MMIO) con su tamano"
Comprobar ($textoAmd -match "(?m)^  BAR1\s+E/S") "gpu reconoce un BAR de E/S"
Comprobar ($textoAmd -match "(?m)^  resumen: BAR0=0x") "gpu imprime la linea-resumen de los BAR"
Comprobar ($textoAmd -notmatch "gpu: no hay ninguna GPU AMD") "con la ati-vga no dice que falte la GPU"
Comprobar ($textoAmd -match "rocky: GPU: registros en 0x") "el arranque abre la ventana de registros de la GPU"
Comprobar ($textoAmd -match "Registros del bloque de pantalla \(ventana 0x") "gpu ensena los registros del bloque de pantalla"
Comprobar ($textoAmd -match "el bloque DCE no cabe en esta ventana") "con una GPU anterior, el volcado avisa de que el bloque DCE no cabe"
Comprobar ($textoAmd -notmatch "(?i)panic|panico:") "sin panics con la GPU AMD"
Comprobar ($texto -notmatch "rocky: GPU: registros en") "sin GPU AMD no se abre ninguna ventana de registros"

# --- Cursor por hardware (fase 14b) ---
Comprobar ($texto -match "cursor: no hay ninguna GPU con cursor por hardware") "sin GPU AMD, cursor avisa de que no hay ninguna"
Comprobar ($textoAmd -match "cursor: bitmap de 64x64 px") "cursor prepara el bitmap del cursor (64x64)"
Comprobar ($textoAmd -match "cursor: VRAM escribe\/lee OK") "cursor escribe el bitmap en la VRAM y lo relee igual"
Comprobar ($textoAmd -match "cursor: el bloque del cursor no cabe en esta ventana") "con una GPU anterior, cursor se niega a programarlo"
Comprobar ($textoAmd -notmatch "cursor: la VRAM no devuelve lo escrito") "la VRAM devuelve lo que se le escribe"

if ($ok) {
  Write-Host "RESULTADO: OK (bus PCI y deteccion de la GPU verificados)"
} else {
  Write-Host "RESULTADO: FALLO"
}
