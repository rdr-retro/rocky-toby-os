# Ejecuta todos los verificadores de la raiz (en procesos aparte) y resume.
#
# Cada `verif-*.ps1` arranca QEMU por su cuenta y pinta lineas que empiezan por
# `OK   ` o `FALLO`. Aqui solo se cuentan las fallidas: cualquier `FALLO` hace
# que el resumen acabe mal. Los verificadores se ejecutan en orden alfabetico y
# el ultimo es `verif-log`, para que la imagen de disco que queda en el arbol
# sea la del sesion recien registrada.

$raiz = $PSScriptRoot
# `verif-todo.ps1` tambien cumple el patron `verif-*.ps1`: si se dejara entrar,
# se lanzaria a si mismo y creceria sin parar. `verif-log.ps1` se reserva para
# el final, para que la imagen de disco que queda en el arbol sea la del
# sesion recien registrada.
$lista = @(Get-ChildItem $raiz -Filter "verif-*.ps1" |
           Where-Object { $_.Name -notin @('verif-todo.ps1', 'verif-log.ps1') } |
           Sort-Object Name)
$scripts = $lista + @(Get-ChildItem $raiz -Filter "verif-log.ps1")

$malos = @()
$inicio = Get-Date

foreach ($s in $scripts) {
  $t0 = Get-Date
  $sal = (& powershell -NoProfile -ExecutionPolicy Bypass -File $s.FullName 2>&1 |
          Out-String -Width 250)
  $fallos = ([regex]::Matches($sal, '(?m)^\s*FALLO')).Count
  $seg = [math]::Round(((Get-Date) - $t0).TotalSeconds)
  if ($fallos -eq 0) {
    Write-Host ("OK    {0,-20} ({1} s)" -f $s.Name, $seg)
  } else {
    Write-Host ("FALLO {0,-20} ({1} s) -> {2} comprobacion(es)" -f $s.Name, $seg, $fallos)
    $sal -split "`r?`n" | Where-Object { $_ -match '^\s*FALLO' } |
      ForEach-Object { Write-Host ("      " + $_.Trim()) }
    $malos += $s.Name
  }
}

$tot = [math]::Round(((Get-Date) - $inicio).TotalSeconds)
Write-Host ""
if ($malos.Count -eq 0) {
  Write-Host "RESULTADO: OK (todos los verificadores, $tot s)"
} else {
  Write-Host ("RESULTADO: FALLO -> " + ($malos -join ", ") + " ($tot s)")
}
