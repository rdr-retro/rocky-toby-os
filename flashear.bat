@echo off
setlocal EnableDelayedExpansion
title rocky-usb - flashear pendrive

set "BASE=%~dp0"
set "EXE=%BASE%flashusb\target\release\flashusb.exe"
set "DISCO=%~1"
set "IMAGEN=%~2"
if "%IMAGEN%"=="" set "IMAGEN=%BASE%rocky-usb.img"

echo ============================================================
echo   rocky-usb: flashear pendrive con flashusb
echo ============================================================
echo.

if not exist "%EXE%" (
    echo  ERROR: falta flashusb.exe
    echo    %EXE%
    echo.
    echo  Compilalo primero:
    echo    cd /d "%BASE%flashusb" ^&^& cargo build --release
    echo.
    pause
    exit /b 1
)
if not exist "%IMAGEN%" (
    echo  ERROR: falta la imagen
    echo    %IMAGEN%
    echo.
    echo  Genera la imagen con:
    echo    cargo run --manifest-path "%BASE%xtask\Cargo.toml" -- build
    echo.
    pause
    exit /b 1
)

echo   Imagen: %IMAGEN%
echo.

powershell -NoProfile -Command "exit ([int](-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)))" >nul 2>&1
if %errorlevel% equ 0 goto :ya_admin

echo  Se necesitan permisos de administrador.
echo  Acepta el cuadro UAC que va a aparecer...
echo.
powershell -NoProfile -Command "try { $p='%~f0'; $a=@(); if('%DISCO%' -ne ''){$a+='%DISCO%'}; if('%~2' -ne ''){$a+='%~2'}; if($a.Count){Start-Process $p -ArgumentList $a -Verb RunAs -ErrorAction Stop}else{Start-Process $p -Verb RunAs -ErrorAction Stop}; Write-Host '  Ventana elevada lanzada correctamente.' } catch { Write-Host ('  Error: ' + $_.Exception.Message); exit 1 }"
if !errorlevel! neq 0 (
    echo.
    echo  No se pudo elevar. Ejecuta el .bat manualmente como administrador:
    echo    boton derecho en %~nx0 ^> Ejecutar como administrador
)
echo.
pause
exit /b

:ya_admin
echo  Ejecutando como administrador.
echo.

if not "%DISCO%"=="" goto :disco_ok

echo  Detectando pendrive / tarjeta extraible...
set "DISKINFO=%TEMP%\flashear-disk.txt"
powershell -NoProfile -Command "Get-CimInstance Win32_DiskDrive | Where-Object { $_.Index -ne 0 -and ($_.InterfaceType -eq 'USB' -or $_.MediaType -like '*Removable*') } | Sort-Object Index | Select-Object -First 1 | ForEach-Object { $_.Index } | Out-File -Encoding ascii '%DISKINFO%' -ErrorAction SilentlyContinue"
set "DISCO="
if exist "%DISKINFO%" set /p DISCO=<"%DISKINFO%"
del "%DISKINFO%" >nul 2>&1

echo.
echo  Discos conectados:
powershell -NoProfile -Command "Get-CimInstance Win32_DiskDrive | Sort-Object Index | ForEach-Object { Write-Host ('    #{0}  {1}  ({2} GB)  {3}' -f $_.Index, $_.Model, [math]::Round($_.Size/1GB,1), $_.InterfaceType) }"

if "%DISCO%"=="" (
    echo.
    echo  No veo ningun pendrive extraible USB.
    echo  Conectalo y vuelve a intentarlo, o indicarlo a mano:
    echo    %~nx0 2
    echo.
    pause
    exit /b 1
)
if "%DISCO%"=="0" (
    echo.
    echo  El disco 0 es el del sistema; no lo toco.
    echo  Indica el disco correcto: %~nx0 N
    echo.
    pause
    exit /b 1
)

:disco_ok
echo.
echo   Disco destino: PhysicalDrive%DISCO%
echo.
echo  ********************************************
echo   ATENCION: se BORRA POR COMPLETO ese disco.
echo   Incluye fotos, documentos, todo.
echo   El SSD del sistema NO se toca (disco 0).
echo  ********************************************
echo.
echo  Escribe BORRAR en mayusculas para confirmar:
echo.

"%EXE%" --image "%IMAGEN%" --disk %DISCO%

echo.
if !errorlevel! neq 0 (
    echo  ALGO HA FALLADO. Revisa los mensajes de arriba.
) else (
    echo  OK: imagen escrita correctamente en el pendrive.
)
echo.
pause
