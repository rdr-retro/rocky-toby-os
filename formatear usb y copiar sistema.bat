@echo off
title formatear usb y copiar sistema
color 0A
cd /d "%~dp0"

set "BUILDLOG=%TEMP%\rocky-build.log"

echo ============================================================
echo   FORMATEAR USB Y COPIAR SISTEMA  (rocky)
echo ============================================================
echo.

echo [1/2] Reconstruyendo el sistema y generando rocky-usb.img...
echo        Esto puede tardar un momento (log: %BUILDLOG%)
echo.

cargo run --manifest-path xtask/Cargo.toml -- build > "%BUILDLOG%" 2>&1
if errorlevel 1 (
    echo.
    echo ERROR: ha fallado la reconstruccion del sistema.
    echo        Revisa el log: %BUILDLOG%
    echo.
    pause
    exit /b 1
)

echo OK: imagen generada correctamente.
echo.

echo [2/2] Formateando y copiando al pendrive...
echo.
echo  AVISO: se BORRA POR COMPLETO el pendrive.
echo         Incluye las fotos del DCIM.
echo  Se usara el disco 1 (el pendrive). El SSD (disco 0) NO se toca.
echo.
pause

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0rocky-usb.ps1" -Disco 1 -Si

if errorlevel 2 goto fallo
if errorlevel 1 goto fallo

echo.
echo ============================================================
echo   TODO BIEN: el pendrive ya tiene rocky y coincide con la
echo   imagen. Ya puedes arrancar de el en el PC.
echo ============================================================
goto fin

:fallo
echo.
echo ============================================================
echo   ALGO HA FALLADO. Revisa los mensajes de arriba.
echo ============================================================

:fin
echo.
pause
