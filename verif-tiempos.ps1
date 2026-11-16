# Comprobacion en el PC de escritorio (no en el kernel) de la aritmetica de
# `Timing::cvt_rb` (kernel/src/drivers/gpu/atom.rs). Se recalcula aqui lo mismo
# que hace el kernel para ver que modos como 320x240 acaban con frecuencia de
# linea (>= 31,5 kHz) y reloj (>= 25,175 MHz) suficientes para un CRT.
$HZ_MINIMA = 31500
$RELOJ_MINIMO = 25175

$modos = @(
  @(320, 200), @(320, 240), @(400, 300), @(640, 480), @(800, 600),
  @(1024, 768), @(1280, 720), @(1280, 800), @(1600, 900), @(1920, 1080),
  @(1920, 1200), @(2560, 1440)
)

$ok = $true
foreach ($m in $modos) {
  $w = $m[0]; $h = $m[1]; $hz = 60
  # Igual que en el kernel: enteros, redondeando hacia arriba.
  $v_total = [math]::Max([math]::Ceiling($HZ_MINIMA / $hz), $h + 31)
  $h_total = [math]::Max([math]::Ceiling(($RELOJ_MINIMO * 1000) / ($v_total * $hz)), $w + 160)
  $clock_khz = [math]::Floor(($h_total * $v_total * $hz) / 1000)
  $linea = $hz * $v_total            # Hz de linea
  $refresco = $clock_khz * 1000 / ($h_total * $v_total)
  $h_freq_real = $clock_khz * 1000 / $h_total

  $bien = $h_freq_real -ge ($HZ_MINIMA - 500) -and $clock_khz -ge $RELOJ_MINIMO -and $h_total -ge ($w + 160) -and $v_total -ge ($h + 31)
  if (!$bien) { $ok = $false }
  Write-Host ("{0,5}x{1,-5} h_total={2,5} v_total={3,5} reloj={4,6} kHz  linea={5,6:N1} kHz  refresco={6,5:N2} Hz  {7}" -f `
    $w, $h, $h_total, $v_total, $clock_khz, ($h_freq_real / 1000), $refresco, $(if ($bien) { "ok" } else { "MAL" }))
}

Write-Host ""
if ($ok) {
  Write-Host "RESULTADO: OK (modos CVT reducido dentro del rango de un CRT)"
} else {
  Write-Host "FALLO algun modo se queda corto (ver la tabla)"
  exit 1
}
