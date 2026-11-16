# Verificacion de la particion FAT32 de logs del sistema.
#
# rocky deja en `rockydisk.img` una tabla de particiones real:
#   p1  0x83  LBA 2048    volumen Estrato (el sistema de ficheros del nucleo)
#   p2  0x0C  LBA 67584   logs en FAT32 (64 MiB), con COMANDOS.LOG
#
# La razon de existir es practica: Windows monta FAT32 sin mas, asi que los
# comandos que se ejecutan en la consola se leen desde el PC. Sin esto solo
# quedaria mirar la pantalla.
#
# La comprobacion se hace en dos tiempos:
#   1) QEMU: se arranca, se teclean ordenes y se mira la consola (deberia
#      decir que hay particion de logs y mostrar la orden `log`).
#   2) A mano: se abre `rockydisk.img` y se parsea la particion FAT32 tal cual
#      lo haria Windows: MBR -> BPB -> directorio raiz -> cadena en la FAT ->
#      contenido. Si en algun paso no cuadra, Windows no podria leerla.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }

$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$marca = "verif-log-unico-20261018"

# ---------------------------------------------------------------------------
# 1) Arranque y tecleo
# ---------------------------------------------------------------------------

function Ejecutar($log, $puerto) {
  if (Test-Path $log) { Remove-Item $log -Force }
  $opciones = @(
    "-m", "256",
    "-boot", "order=a",
    "-drive", "if=floppy,format=raw,file=$raiz\rocky.img",
    "-drive", "if=ide,index=0,media=disk,format=raw,cache=directsync,file=$raiz\rockydisk.img",
    "-serial", "file:$log",
    "-monitor", "tcp:127.0.0.1:$puerto,server,nowait",
    "-display", "none",
    "-no-reboot"
  )
  $p = Start-Process -FilePath $qemu -ArgumentList $opciones -PassThru `
        -RedirectStandardError "$tmp\qemu-log-err.txt" `
        -RedirectStandardOutput "$tmp\qemu-log-out.txt"

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

  # `sendkey` solo entiende nombres, no caracteres sueltos. Y hay una trampa:
  # QEMU nombra las teclas por su posicion en un teclado estadounidense, mientras
  # que rocky usa la distribucion espanola (lo dice `base_char` en `input.rs`),
  # asi que la raya (`-`) esta donde en EE. UU. esta la barra inclinada. Sin este
  # mapa, un argumento llega pegado al orden y la prueba se autocumple por error.
  function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 80 }
  function Teclas([string]$t) {
    foreach ($c in $t.ToCharArray()) {
      switch ($c) {
        ' ' { Tecla "spc" }
        '-' { Tecla "slash" }
        '_' { Tecla "shift-slash" }
        '/' { Tecla "shift-7" }
        '.' { Tecla "period" }
        ',' { Tecla "comma" }
        ':' { Tecla "shift-period" }
        ';' { Tecla "shift-comma" }
        "'" { Tecla "minus" }
        default { Tecla ([string]$c) }
      }
    }
  }
  function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 1500 }

  # ESC deja el escritorio de Kira y devuelve la consola de texto.
  Tecla "esc"; Start-Sleep -Seconds 2
  Orden "uname"
  Orden "gpu"
  Orden "log"
  Orden "echo $marca"
  Orden "log"
  Start-Sleep -Seconds 2

  # `directsync` arriba hace que cada escritura del huesped llegue al fichero en
  # el momento; aun asi se espera la salida limpia, porque matar a QEMU a
  # medias se come cualquier cosa pendiente en su cache.
  $escritor.WriteLine("quit")
  $limpio = $p.WaitForExit(20000)
  $cliente.Close()
  if (-not $limpio) {
    Write-Host "AVISO: QEMU no termino en 20 s; se fuerza la salida."
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $null = $p.WaitForExit(5000)
  }
  Start-Sleep -Seconds 1
}

$serie = "$tmp\rocky-log.log"
Ejecutar $serie 5613
$texto = Get-Content $serie -Raw

Write-Host "=== consola: ultimas 30 lineas ==="
($texto -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 30) | Out-String -Width 200 | Write-Host
Write-Host "=== fin consola ==="

# ---------------------------------------------------------------------------
# 2) Parseo de la particion FAT32 tal cual lo haria Windows
# ---------------------------------------------------------------------------

$ok = $true
function Comprobar([bool]$cond, [string]$msg) {
  if ($cond) { Write-Host "OK   $msg" } else { Write-Host "FALLO $msg"; $script:ok = $false }
}

$fs = [IO.File]::Open("$raiz\rockydisk.img", [IO.FileMode]::Open, [IO.FileAccess]::Read)

function Sectors($lba, $n) {
  $buf = New-Object byte[] ($n * 512)
  $script:fs.Seek([int64]$lba * 512, 'Begin') | Out-Null
  $script:fs.Read($buf, 0, $buf.Length) | Out-Null
  return $buf
}

# --- MBR ---
$mbr = Sectors 0 1
Comprobar ($mbr[510] -eq 0x55 -and $mbr[511] -eq 0xAA) "el MBR tiene su firma 55AA"
$p1tipo = $mbr[450]; $p1lba = [BitConverter]::ToUInt32($mbr, 454); $p1n = [BitConverter]::ToUInt32($mbr, 458)
$p2tipo = $mbr[466]; $p2lba = [BitConverter]::ToUInt32($mbr, 470); $p2n = [BitConverter]::ToUInt32($mbr, 474)
Comprobar ($p1tipo -eq 0x83 -and $p1lba -eq 2048 -and $p1n -eq 65536) "p1: volumen Estrato (0x83) en LBA 2048, 65536 sectores"
Comprobar ($p2tipo -eq 0x0C) "p2: la particion de logs es tipo 0x0C (FAT32 LBA)"
Comprobar ($p2lba -eq 67584 -and $p2n -eq 131072) "p2: LBA 67584, 131072 sectores (64 MiB)"

# --- BPB de la particion 2 ---
$bpb = Sectors $p2lba 1
$bps = [BitConverter]::ToUInt16($bpb, 11)
$spc = $bpb[13]
$reservados = [BitConverter]::ToUInt16($bpb, 14)
$nfats = $bpb[16]
$fatsz16 = [BitConverter]::ToUInt16($bpb, 22)
$fatsz = [BitConverter]::ToUInt32($bpb, 36)
$raizClus = [BitConverter]::ToUInt32($bpb, 44)
$oculto = [BitConverter]::ToUInt32($bpb, 28)
$tipo = [Text.Encoding]::ASCII.GetString($bpb, 82, 8)
$etiqueta = [Text.Encoding]::ASCII.GetString($bpb, 71, 11)
Comprobar ($bpb[510] -eq 0x55 -and $bpb[511] -eq 0xAA) "el sector de arranque de la particion tiene firma"
Comprobar ($bps -eq 512) "512 bytes por sector"
Comprobar ($fatsz16 -eq 0 -and $fatsz -gt 0) "usa la FAT de 32 bits (FATSz16=0 y FATSz32>0)"
Comprobar ($tipo -eq "FAT32   ") "el tipo del sistema es 'FAT32'"
Comprobar ($oculto -eq $p2lba) "los sectores ocultos indican el LBA real ($p2lba)"
Comprobar ($etiqueta.Trim() -eq "ROCKYLOG") "la etiqueta del volumen es ROCKYLOG"
Comprobar ($spc -eq 1) "un sector por cluster"
Comprobar ($raizClus -eq 2) "el cluster raiz es el 2"
Comprobar ($nfats -eq 2) "hay dos copias de la FAT"
$datos = $reservados + $nfats * $fatsz
$clusters = ($p2n - $datos) / $spc
Comprobar ($clusters -gt 65525) "mas de 65525 clusters ($clusters): Windows lo reconoce como FAT32"

# --- Entrada FSInfo ---
$fsinfo = Sectors ($p2lba + 1) 1
$lead = [BitConverter]::ToUInt32($fsinfo, 0)
$trail = [BitConverter]::ToUInt32($fsinfo, 508)
# La firma de cierre (0xAA550000) no cabe en un entero de 32 bits con signo y
# PowerShell se come el literal 0xAA550000, asi que se comparan los bytes.
Comprobar ($lead -eq 0x41615252 -and $fsinfo[508] -eq 0 -and $fsinfo[509] -eq 0 -and $fsinfo[510] -eq 0x55 -and $fsinfo[511] -eq 0xAA) "el FSInfo lleva sus marcas"

# --- Directorio raiz ---
$raizSect = $datos + ($raizClus - 2) * $spc
$dir = Sectors ($p2lba + $raizSect) 1
$e0 = [Text.Encoding]::ASCII.GetString($dir, 0, 11)
Comprobar (($dir[11] -band 0x08) -ne 0 -and $e0.Trim() -eq "ROCKYLOG") "el directorio raiz trae la etiqueta del volumen"

$nombre = [Text.Encoding]::ASCII.GetString($dir, 32, 8) + "." + [Text.Encoding]::ASCII.GetString($dir, 40, 3)
$attr = $dir[43]
$alto = [BitConverter]::ToUInt16($dir, 52)
$bajo = [BitConverter]::ToUInt16($dir, 58)
$tam = [BitConverter]::ToUInt32($dir, 60)
$cl0 = (($alto -shl 16) -bor $bajo)
Comprobar ($nombre -eq "COMANDOS.LOG") "el directorio raiz contiene COMANDOS.LOG"
Comprobar (($attr -band 0x10) -eq 0 -and ($attr -band 0x08) -eq 0) "COMANDOS.LOG es un fichero normal"
Comprobar ($cl0 -ge 3) "su primer cluster es $cl0"
Comprobar ($tam -gt 0 -and $tam -le 262144) "su tamano es $tam bytes (dentro de la reserva de 262144)"

# --- Sigue la cadena en la FAT y lee el contenido ---
$contenido = New-Object Text.StringBuilder
$offset = 0
$c = $cl0
$seguridad = 0
while ($c -ge 2 -and $c -lt 0x0FFFFFF8 -and $offset -lt $tam -and $seguridad -lt 2000) {
  $seguridad++
  $sec = $datos + ($c - 2) * $spc
  $bloque = Sectors ($p2lba + $sec) $spc
  $tomar = [Math]::Min(512 * $spc, $tam - $offset)
  [void]$contenido.Append([Text.Encoding]::UTF8.GetString($bloque, 0, $tomar))
  $offset += $tomar
  # Lee la entrada de la FAT de este cluster.
  $fatSec = $reservados + [int]([Math]::Floor($c * 4 / 512))
  $fatOff = ($c * 4) % 512
  $fatB = Sectors ($p2lba + $fatSec) 1
  $c = [BitConverter]::ToUInt32($fatB, $fatOff) -band 0x0FFFFFFF
}
Comprobar ($offset -eq $tam) "se pudo recorrer la cadena entera hasta los $tam bytes"
# Si el log aun no llena la reserva, el proximo cluster es el siguiente de la
# cadena; si la llena exacta, la cadena cierra con fin de cadena. Ambas cosas
# estan bien (lo malo seria 0, un cluster roto o un numero fuera de la gama).
Comprobar (($c -ge 3 -and $c -le 515) -or $c -eq 0x0FFFFFFF) "la cadena no se corta: el proximo cluster es $c"

$textoLog = $contenido.ToString()
Comprobar ($textoLog.Length -gt 0 -and $textoLog.Length -le $tam) "el contenido leido tiene $($textoLog.Length) caracteres"
Comprobar ($textoLog -match "--- arranque de rocky ---") "el log arranca con la marca de arranque"
Comprobar ($textoLog -match "uname") "se registro la orden uname"
Comprobar ($textoLog -match "gpu") "se registro la orden gpu"
Comprobar ($textoLog -match "log") "se registro la orden log"
Comprobar ($textoLog -match [regex]::Escape($marca)) "se registro la orden con la marca unica"
Comprobar ($textoLog -match "`r`n") "las lineas acaban en CRLF (Windows no pide mas)"

# --- Segunda copia de la FAT: es lo que usa Windows si lee la primera ---
$fat1 = Sectors ($p2lba + $reservados) $fatsz
$fat2 = Sectors ($p2lba + $reservados + $fatsz) $fatsz
$iguales = $true
for ($i = 0; $i -lt [Math]::Min($fat1.Length, $fat2.Length); $i += 4096) {
  if ($fat1[$i] -ne $fat2[$i]) { $iguales = $false; break }
}
Comprobar $iguales "las dos copias de la FAT coinciden"
Comprobar ([BitConverter]::ToUInt32($fat1, 0) -eq 0x0FFFFFF8) "la entrada 0 de la FAT lleva la marca de medios"
# El cluster 2 es el directorio raiz: sin fin de cadena Windows no ve su contenido.
Comprobar (([BitConverter]::ToUInt32($fat1, 8) -band 0x0FFFFFFF) -eq 0x0FFFFFFF) "el cluster 2 (raiz) marca fin de cadena"
Comprobar (([BitConverter]::ToUInt32($fat1, 12) -band 0x0FFFFFFF) -eq 4) "el cluster 3 apunta al 4 (cadena del log)"
# La reserva va del 3 al 514: 512 clusters, el ultimo con fin de cadena.
Comprobar (([BitConverter]::ToUInt32($fat1, 514 * 4) -band 0x0FFFFFFF) -eq 0x0FFFFFFF) "el ultimo cluster reservado (514) es fin de cadena"

# --- Copias de seguridad del sector de arranque ---
$bootCop = Sectors ($p2lba + 6) 1
$igualBoot = $true
for ($i = 0; $i -lt 512; $i++) { if ($bootCop[$i] -ne $bpb[$i]) { $igualBoot = $false; break } }
Comprobar $igualBoot "la copia de seguridad del sector de arranque coincide"

$fs.Close()

# ---------------------------------------------------------------------------
# 3) Consola
# ---------------------------------------------------------------------------

Write-Host ""
Comprobar ($texto -match "rocky: logs en COMANDOS.LOG") "el arranque anuncia la particion de logs"
Comprobar ($texto -match "(?m)^COMANDOS.LOG") "la orden log ensena el uso del fichero"
Comprobar ($texto -match [regex]::Escape($marca)) "la consola recibe la orden que se registro en el log"
Comprobar ($texto -notmatch "(?i)panic|panico:") "sin panics"

if ($ok) {
  Write-Host "RESULTADO: OK (particion FAT32 de logs verificada)"
} else {
  Write-Host "RESULTADO: FALLO"
}
