@echo off
rem ============================================================
rem  Toby UI - interprete de tobyui lang
rem  Arranca el anfitrion de Windows que abre la ventana y pinta
rem  un documento .toby con el motor real del lenguaje.
rem
rem  Este lanzador vive en kernel\src\toby_ui (el motor), asi que
rem  primero localiza la carpeta del crate 'interprete de tobyui lang'
rem  subiendo por el arbol de directorios.
rem
rem  Uso:
rem    arrancar.bat                    -> documentos\ejemplo.toby (por defecto)
rem    arrancar.bat otro.toby          -> un documento concreto
rem    arrancar.bat --png salida.png   -> sin ventana, vuelca un PNG
rem    arrancar.bat recompilar         -> recompila antes de arrancar
rem ============================================================
setlocal
title Toby UI - interprete de tobyui lang

rem --- Localiza la carpeta del crate subiendo desde la del .bat ---
set "DIR=%~dp0"
set "CRATE="
set /A VUELTAS=0
:buscando
set /A VUELTAS+=1
if %VUELTAS% GTR 12 goto :encontrado
if defined CRATE goto :encontrado
for %%P in ("%DIR%\..") do set "DIR=%%~fP"
if exist "%DIR%\interprete de tobyui lang\Cargo.toml" set "CRATE=%DIR%\interprete de tobyui lang"
goto :buscando

:encontrado
if not defined CRATE (
    echo ERROR: no encuentro la carpeta 'interprete de tobyui lang' en la
    echo        ruta de arriba de %~dp0
    pause
    exit /b 1
)

cd /d "%CRATE%"
set "EXE=target\release\interprete-tobyui.exe"
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
