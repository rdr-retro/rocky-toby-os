# Prueba de execve: reemplazo de la imagen del proceso.
#   - arranca, sale del escritorio a modo texto (ESC);
#   - ejecuta `exec`: se lanza un proceso en anillo 3 que llama a
#     execve("/bin/hola", {"uno","dos"}, NULL); el nucleo lee el ejecutable del
#     disco (Estrato) y reemplaza su imagen; el programa nuevo imprime y termina;
#   - lista las tareas con `ps` (no debe quedar ningun proceso vivo);
#   - comprueba que NO aparecio la pantalla azul.
#
# Ademas de probar execve, este guion es la **regresion** de un fallo real: el
# nucleo escribe el bloque de argumentos en la pila del usuario desde el anillo
# 0. Antes, `montar_pila` daba por supuesta mapeada la pagina superior de la pila
# y, si no lo estaba, el #PF de anillo 0 se tomaba por un fallo del nucleo y
# acababa en la pantalla azul (memcpy sobre 0x...3FFFFFEC, justo bajo 1 GiB).
# Ahora `tarea::ejecutar` trae esas paginas a memoria antes de escribir, y el
# manejador de #PF tambien sabe traerlas bajo demanda.

$raiz = $PSScriptRoot
$qemu = "$env:LOCALAPPDATA\Programs\qemu\qemu-system-x86_64.exe"
if (!(Test-Path $qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe" }
$tmp = "$env:LOCALAPPDATA\Temp\opencode"
$log = "$tmp\serial-exec.log"
$ppm = "$tmp\exec.ppm"
if (Test-Path $log) { Remove-Item $log -Force }
if (Test-Path $ppm) { Remove-Item $ppm -Force }
$puerto = 5587

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

function Nombre([char]$ch) {
  switch -CaseSensitive ([string]$ch) {
    "/" { "shift-7" }; "." { "dot" }; "-" { "slash" }; " " { "spc" }; "_" { "shift-slash" }
    default { [string]$ch }
  }
}
function Tecla([string]$k) { $escritor.WriteLine("sendkey $k"); Start-Sleep -Milliseconds 70 }
function Teclas([string]$t) { foreach ($c in $t.ToCharArray()) { Tecla (Nombre $c) } }
function Orden([string]$t) { Teclas $t; Tecla "ret"; Start-Sleep -Milliseconds 900 }

Start-Sleep -Seconds 8
Tecla "esc"
Start-Sleep -Seconds 1
Orden "cat /bin/hola"
Orden "exec"
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

$fallos = 0
function Comprobar([string]$nombre, [bool]$ok, [string]$detalle) {
  if ($ok) { Write-Host "  [OK]    $nombre" }
  else { Write-Host "  [FALLO] $nombre  ($detalle)"; $script:fallos++ }
}

Write-Host "== comprobaciones =="
Comprobar "el kernel llego al interprete"        ($texto -match "root@rocky")                     "sin prompt"
Comprobar "exec lanza un proceso de anillo 3"    ($texto -match "exec: proceso en anillo 3")      "no se lanzo"
Comprobar "el programa nuevo corre (argc=2)"     ($texto -match "execve ha cargado un programa")  "no corrio el programa nuevo"
Comprobar "la tarea exec termina"                ($texto -match "terminado\s+exec")               "no consta terminada"
# La ultima tabla de `ps` no debe tener ninguna fila `exec` (los servicios
# de tiempo y Kira siguen vivos, asi que no se cuenta el numero de tareas).
$psFinal = ($texto -split "(?ms)PID\s+ESTADO\s+NOMBRE")[-1]
Comprobar "no queda ningun proceso vivo"         (-not ($psFinal -match "\bexec\b"))              "quedo algun proceso"
Comprobar "no hubo pantalla azul"                (-not ($texto -match "ROCKY KERNEL PANIC"))      "aparecio la pantalla azul"

Write-Host ""
if ($fallos -eq 0) {
  Write-Host "RESULTADO: OK (execve verificado)"
  exit 0
} else {
  Write-Host "RESULTADO: $fallos comprobacion(es) fallida(s)"
  exit 1
}
