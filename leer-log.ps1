# Saca COMANDOS.LOG de la particion FAT32 de logs del disco de rocky.
#
# Windows monta FAT32 sin instalar nada (disco duro virtual, pendrive...), pero
# a veces no asigna letra a un disco que mete el usuario a mano y hay que abrir
# el volumen desde el Administrador de discos. Este guion no monta nada: parsea
# la particion 0x0C del MBR y escribe el texto en `comandos.txt`, que se abre
# con cualquier editor.
#
#   powershell -File .\leer-log.ps1            -> disco por defecto
#   powershell -File .\leer-log.ps1 -Img ruta  -> otra imagen

param(
  [string]$Img = "C:\Users\rdr-\Desktop\kernel\rockydisk.img",
  [string]$Sal = "C:\Users\rdr-\Desktop\kernel\comandos.txt",
  [int]$Ultimas = 0
)

if (-not (Test-Path $Img)) { Write-Error "no existe $Img"; exit 1 }

# Se abre con FileShare.ReadWrite: QEMU puede tener la imagen abierta (la
# ventana del usuario o un verificador que aun no ha salido), y asi se lee el
# volumen de logs aunque lo este escribiendo en ese momento, en vez de fallar
# con "el fichero esta en uso".
$fs = [System.IO.FileStream]::new($Img, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)

function Sect([int64]$lba, [int]$n) {
  $b = New-Object byte[] ($n * 512)
  $script:fs.Seek($lba * 512, 'Begin') | Out-Null
  $script:fs.Read($b, 0, $b.Length) | Out-Null
  $b
}

# --- MBR: localiza la particion de logs (tipo 0x0B o 0x0C) ---
$mbr = Sect 0 1
if ($mbr[510] -ne 0x55 -or $mbr[511] -ne 0xAA) { $fs.Close(); Write-Error "el disco no tiene MBR"; exit 1 }

$base = -1
for ($i = 0; $i -lt 4; $i++) {
  $o = 446 + $i * 16
  $tipo = $mbr[$o + 4]
  if ($tipo -eq 0x0B -or $tipo -eq 0x0C) { $base = [BitConverter]::ToUInt32($mbr, $o + 8); break }
}
if ($base -lt 0) { $fs.Close(); Write-Error "no hay particion FAT32 de logs en el MBR"; exit 1 }

# --- BPB ---
$bpb = Sect $base 1
$spc = $bpb[13]
$reserv = [BitConverter]::ToUInt16($bpb, 14)
$nfats = $bpb[16]
$fsz = [BitConverter]::ToUInt32($bpb, 36)
$raiz = [BitConverter]::ToUInt32($bpb, 44)
$datos = $reserv + $nfats * $fsz

# --- Directorio raiz: busca COMANDOS.LOG ---
$dir = Sect ($base + $datos + (($raiz - 2) * $spc)) 1
$encontrado = $false
$cl = 0; $tam = 0
for ($e = 0; $e -lt 512; $e += 32) {
  if ($dir[$e] -eq 0) { break }
  $attr = $dir[$e + 11]
  if (($attr -band 0x08) -ne 0 -or ($attr -band 0x10) -ne 0) { continue }  # ni etiqueta de volumen ni directorio
  $nom = [Text.Encoding]::ASCII.GetString($dir, $e, 8).Trim() + "." +
         [Text.Encoding]::ASCII.GetString($dir, $e + 8, 3).Trim()
  if ($nom -eq "COMANDOS.LOG") {
    $cl = [BitConverter]::ToUInt16($dir, $e + 26)
    $tam = [BitConverter]::ToUInt32($dir, $e + 28)
    $encontrado = $true
    break
  }
}
if (-not $encontrado) { $fs.Close(); Write-Error "COMANDOS.LOG no esta en la particion"; exit 1 }
if ($tam -eq 0) { $fs.Close(); Write-Host "COMANDOS.LOG esta vacio."; exit 0 }

# --- Recorre la cadena en la FAT y junta los bytes ---
$bytes = New-Object System.Collections.Generic.List[byte]
$leido = 0
while ($cl -ge 2 -and $cl -lt 0x0FFFFFF8 -and $leido -lt $tam) {
  $sec = $datos + (($cl - 2) * $spc)
  $b = Sect ($base + $sec) $spc
  $n = [Math]::Min($b.Length, $tam - $leido)
  for ($k = 0; $k -lt $n; $k++) { $bytes.Add($b[$k]) }
  $leido += $n
  $fatOff = $cl * 4
  $fb = Sect ($base + $reserv + [int]($fatOff / 512)) 1
  $cl = [BitConverter]::ToUInt32($fb, $fatOff % 512) -band 0x0FFFFFFF
}
$fs.Close()

$texto = [Text.Encoding]::UTF8.GetString($bytes.ToArray())
[IO.File]::WriteAllText($Sal, $texto, [Text.UTF8Encoding]::new($false))

$lineas = ($texto -split "`r?`n") | Where-Object { $_ -ne "" }
Write-Host ("{0}: {1} bytes, {2} lineas." -f $Img, $tam, $lineas.Count)
Write-Host ("Texto guardado en {0}" -f $Sal)
Write-Host ""
if ($Ultimas -gt 0) { $lineas | Select-Object -Last $Ultimas } else { $lineas }
