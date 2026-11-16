@echo off
rem ============================================================
rem  Toby AppKit
rem  Arranca el editor visual: panel lateral con todos los tipos
rem  de boton, lienzo en blanco donde se colocan libremente
rem  (arrastrando) y boton "Generar .toby".
rem
rem  Uso:
rem    arrancar.bat                      -> abre el editor
rem    arrancar.bat recompilar           -> recompila antes de arrancar
rem    arrancar.bat --png editor.png     -> sin ventana: vuelca la interfaz
rem    arrancar.bat --generar diseno.toby-> sin ventana: genera un .toby
rem    arrancar.bat --prueba             -> autoprueba de la logica
rem    arrancar.bat nuevo apps\reloj.app -> crea un paquete .app
rem    arrancar.bat validar apps\reloj.app -> valida el paquete
rem    arrancar.bat compilar apps\reloj.app -> compila su codigo Rust
rem    arrancar.bat --salida otro.toby   -> cambia el fichero que genera
rem ============================================================
setlocal
title Toby AppKit
chcp 65001 >nul

rem --- Localiza la carpeta del crate ---
rem Lo normal es que este .bat viva en la raiz del crate; si no,
rem se sube por el arbol buscando el Cargo.toml de AppKit.
set "DIR=%~dp0"
set "CRATE="
if exist "%DIR%Cargo.toml" set "CRATE=%DIR%"
set /A VUELTAS=0
:buscando
if defined CRATE goto :encontrado
set /A VUELTAS+=1
if %VUELTAS% GTR 12 goto :encontrado
for %%P in ("%DIR%..") do set "DIR=%%~fP\"
if exist "%DIR%appkit\Cargo.toml" set "CRATE=%DIR%appkit\"
if exist "%DIR%Cargo.toml" if not defined CRATE set "CRATE=%DIR%"
goto :buscando

:encontrado
if not defined CRATE (
    echo ERROR: no encuentro el crate de AppKit desde %~dp0
    pause
    exit /b 1
)

cd /d "%CRATE%"
set "EXE=target\release\toby-appkit.exe"
set "RECOMPILAR=0"
set "ARGUMENTOS="

rem --- Separa la opcion propia del .bat de lo que va al ejecutable ---
:bucle
if "%~1"=="" goto :fin_bucle
if /I "%~1"=="recompilar" (
    set "RECOMPILAR=1"
) else if /I "%~1"=="-b" (
    set "RECOMPILAR=1"
) else (
    set "ARGUMENTOS=%ARGUMENTOS% "%~1""
)
shift
goto :bucle
:fin_bucle

echo Carpeta   : %CRATE%

rem Si todavia no hay ejecutable, hay que compilar de todos modos.
if not exist "%EXE%" set "RECOMPILAR=1"

if "%RECOMPILAR%"=="1" (
    echo [1/2] Compilando en modo release...
    cargo build --release
    if errorlevel 1 (
        echo.
        echo ERROR: fallo la compilacion. Revisa los mensajes de arriba.
        pause
        exit /b 1
    )
) else (
    echo [1/2] Ejecutable ya compilado, se omite la compilacion.
)

echo [2/2] Arrancando: "%EXE%"%ARGUMENTOS%
echo.
"%EXE%"%ARGUMENTOS%
set "CODIGO=%ERRORLEVEL%"

echo.
if "%CODIGO%"=="0" (
    echo Programa terminado correctamente.
) else (
    echo Programa terminado con codigo %CODIGO%.
)
pause
exit /b %CODIGO%
