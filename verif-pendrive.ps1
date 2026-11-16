# Verificacion del arranque por pendrive (`rocky-usb.img`).
#
# La imagen se arranca *como disco duro* (no como disquete): QEMU/SeaBIOS lee
# el sector de arranque del LBA 0 y tira de la cadena de siempre. Es la unica
# prueba automatica del reparto en dos tramos:
#
#   LBA 0      primer tramo: BPB de disco duro + carga del segundo tramo
#   LBA 1 y 2  segundo tramo (`bootusb2.bin`): carga el kernel y lo cuenta
#   LBA 3 ..   kernel
#
# Se mira el fichero de la imagen (estructura y tabla de particiones) y la
# salida de serie de un arranque completo. Si el primer tramo se equivoca de
# direccion (le paso: cargaba el segundo tramo en 0x80000 y saltaba a 0x8000),
# por serie no llega nada y aqui se ve el fallo.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }
$tmp = "$env:LOCALAPPDATA\Temp\opencode"
if (!(Test-Path $tmp)) { New-Item -ItemType Directory -Path $tmp | Out-Null }
$log = "$tmp\pendrive.log"
if (Test-Path $log) { Remove-Item $log -Force }

$imagen = "$raiz\rocky-usb.img"
$ok = $true
function Comprobar([bool]$cond, [string]$txt) {
  # El prefijo es el que cuenta `verif-todo.ps1` (`FALLO` al principio de
  # linea), asi que no se adorna con corchetes.
  if ($cond) { Write-Host "OK    $txt" } else { Write-Host "FALLO $txt"; $script:ok = $false }
}

# ---------------------------------------------------------------------------
# 1) La imagen, sin arrancar
# ---------------------------------------------------------------------------

if (!(Test-Path $imagen)) { throw "no existe $imagen (hay que compilar)" }
$f = [IO.File]::OpenRead($imagen)
$cab = New-Object byte[] 1024
$leidos = $f.Read($cab, 0, 1024)
$f.Close()
Comprobar ($leidos -eq 1024) "la imagen tiene al menos 1024 bytes"

Comprobar ($cab[0] -eq 0xEB -and $cab[1] -eq 0x52 -and $cab[2] -eq 0x90) `
  "el LBA 0 salta por encima del BPB (EB 52 90)"
Comprobar (([Text.Encoding]::ASCII.GetString($cab, 3, 8)) -eq "NTFS    ") `
  "el BPB es de disco fijo (estilo NTFS): la BIOS lo toma por USB-HDD"
Comprobar ($cab[510] -eq 0x55 -and $cab[511] -eq 0xAA) "el MBR acaba con su firma (55 AA)"
Comprobar ($cab[512] -eq 0xFA -and $cab[513] -eq 0x31 -and $cab[514] -eq 0xC0) `
  "en el LBA 1 empieza el segundo tramo (cli; xor ax,ax)"

$p1tipo = $cab[446 + 4]
$p1lba = [BitConverter]::ToUInt32($cab, 446 + 8)
$p1n = [BitConverter]::ToUInt32($cab, 446 + 12)
Comprobar ($p1tipo -eq 0xEF -and $p1lba -eq 2048 -and $p1n -eq 131072) `
  "p1: ESP (0xEF) en el LBA 2048, 131072 sectores"
$p2tipo = $cab[462 + 4]
$p2lba = [BitConverter]::ToUInt32($cab, 462 + 8)
Comprobar ($p2tipo -eq 0x0C -and $p2lba -eq 198656) `
  "p2: logs FAT32 (0x0C) en el LBA 198656, detras del volumen Estrato"

# ---------------------------------------------------------------------------
# 2) Arranque de verdad (QEMU, la imagen como disco C)
# ---------------------------------------------------------------------------

$puerto = 5621
$opciones = @(
  "-m", "256",
  "-boot", "order=c",
  "-drive", "if=ide,index=0,media=disk,format=raw,cache=directsync,file=$imagen",
  "-serial", "file:$log",
  "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
  "-display", "none",
  "-no-reboot"
)
$p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
      -RedirectStandardError "$tmp\qemu-pendrive-err.txt" `
      -RedirectStandardOutput "$tmp\qemu-pendrive-out.txt"

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

# Se espera a que el kernel acabe de arrancar (`[4/4]`) o a que se agote el
# tiempo: la prueba no depende de acertar con el numero de segundos.
$texto = ""
for ($i = 0; $i -lt 45; $i++) {
  Start-Sleep -Seconds 1
  $texto = if (Test-Path $log) { [string](Get-Content $log -Raw) } else { "" }
  if ($texto -match "\[4/4\]") { break }
  if ($p.HasExited) { break }
}
Start-Sleep -Seconds 2
$texto = if (Test-Path $log) { [string](Get-Content $log -Raw) } else { "" }

# Salida limpia (nada de matar a QEMU a medias con el fichero de la imagen
# abierto en escritura).
$escritor.WriteLine("quit")
$limpio = $p.WaitForExit(20000)
$cliente.Close()
if (-not $limpio) {
  Write-Host "AVISO: QEMU no termino en 20 s; se fuerza la salida."
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 1

Write-Host "=== salida serie ==="
Write-Host ((($texto -split "`r?`n") | Where-Object { $_ -ne "" }) -join "`n")
Write-Host "=== fin ==="

# ---------------------------------------------------------------------------
# 3) Veredicto
# ---------------------------------------------------------------------------

$esperados = [int][Math]::Ceiling((Get-Item "$raiz\build\kernel.bin").Length / 512)
$hex = "{0:X4}" -f $esperados

Comprobar ($texto -match "rocky: arranque: unidad 0x80") `
  "el segundo tramo arranca y saluda con la unidad de disco (0x80)"
Comprobar ($texto -match ("kernel de 0x" + [regex]::Escape($hex) + " sectores")) `
  "dice los sectores del kernel (0x$hex = $esperados)"
Comprobar ($texto -match "rocky: kernel cargado, entrando en modo largo\.") `
  "carga el kernel entero y entra en modo largo"
Comprobar ($texto -match "rocky: cargando kernel \(x86_64, modo largo\)") `
  "el kernel toma el relevo"
Comprobar ($texto -match "\[4/4\] temporizador e interrupciones") `
  "el kernel termina su arranque"
Comprobar ($texto -match "logs en COMANDOS.LOG") `
  "encuentra la particion de logs dentro del pendrive"
Comprobar ($texto -notmatch "no arranca \(error|no lee el pendrive") `
  "no aparece ningun aviso del cargador"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panicos en la serie"

Write-Host ""
if ($ok) {
  Write-Host "RESULTADO: OK (arranque del pendrive verificado)"
} else {
  Write-Host "RESULTADO: FALLO"
  exit 1
}
