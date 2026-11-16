# Prueba de la enumeracion xHCI (fase 3): Enable Slot, Address Device y
# transferencias de control por el endpoint 0.
#
# Arranca rocky en QEMU con un controlador xHCI emulado y un teclado y un raton
# USB (los dos son HID) y comprueba que el driver:
#
#   1) emite comandos de verdad y recibe sus eventos de fin;
#   2) enumera los dos dispositivos: los direcciona, lee sus descriptores y los
#      configura;
#   3) reconoce la interfaz HID y su endpoint de interrupcion de entrada.
#
# Lo que NO se comprueba (aun): las transferencias de interrupcion. Esta fase
# enumera; no lee todavia el teclado ni el raton.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }
if (-not (Test-Path $qemu)) { throw "no se encuentra qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\rocky-usb-cmd.log"
if (Test-Path $log) { Remove-Item $log -Force }

$opciones = @(
  "-m", "256",
  "-boot", "order=a",
  "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
  "-drive", "if=ide,index=0,media=disk,format=raw,file=$raiz\rockydisk.img",
  "-device", "qemu-xhci,id=xhci",
  "-device", "usb-kbd,bus=xhci.0",
  "-device", "usb-mouse,bus=xhci.0",
  "-serial", "file:$log",
  "-display", "none",
  "-no-reboot"
)
$p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
      -RedirectStandardError "$tmp\qemu-usb-cmd-err.txt" `
      -RedirectStandardOutput "$tmp\qemu-usb-cmd-out.txt"
Start-Sleep -Seconds 22
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1

$texto = Get-Content $log -Raw
Write-Host "=== lineas con 'usb:' ==="
Write-Host (($texto -split "`r?`n" | Where-Object { $_ -match "usb:" }) -join "`n")
Write-Host "=== fin ==="

$ok = $true
function Comprobar([bool]$cond, [string]$msg) {
  if ($cond) { Write-Host "OK   $msg" } else { Write-Host "FALLO $msg"; $script:ok = $false }
}
Comprobar ($texto -match "usb: xHCI inicializado: \d+ puertos raiz") "el HCD arranca con sus puertos raiz"

# --- Enumeracion ---
# Los dos dispositivos de QEMU van a los dos primeros puertos USB 2.0 libres,
# que en un qemu-xhci son el 5 y el 6.
Comprobar ($texto -match "(?m)^usb: dispositivo en el puerto 5: 0627:0001 clase 0, 1 interfaces \(1 HID\)") "se enumera el teclado del puerto 5 (clase 0, interfaz HID)"
Comprobar ($texto -match "(?m)^usb: dispositivo en el puerto 6: 0627:0001 clase 0, 1 interfaces \(1 HID\)") "se enumera el raton del puerto 6 (clase 0, interfaz HID)"
Comprobar ($texto -match "(?m)^usb: dispositivo en el puerto 5: .* 1 endpoints \(1 de interrupcion\), .*, configuracion 1\.") "el teclado tiene un endpoint de interrupcion de entrada y configuracion 1"
Comprobar ($texto -match "(?m)^usb: dispositivo en el puerto 6: .* 1 endpoints \(1 de interrupcion\), .*, configuracion 1\.") "el raton tiene un endpoint de interrupcion de entrada y configuracion 1"
Comprobar ($texto -match "(?m)^usb: dispositivo en el puerto 5: .*, velocidad (baja|completa|alta|super) \(USB") "la velocidad del teclado se lee del puerto"
Comprobar ($texto -notmatch "no se pudo enumerar") "ningun puerto falla al enumerar"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panics"
if ($ok) { Write-Host "RESULTADO: OK" } else { Write-Host "RESULTADO: FALLO" }
