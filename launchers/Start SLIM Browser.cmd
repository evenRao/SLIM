@echo off
setlocal
set "SLIM_EXE=%~dp0slim-app.exe"
if not exist "%SLIM_EXE%" set "SLIM_EXE=%~dp0..\dist\SLIM-Windows\slim-app.exe"
if not exist "%SLIM_EXE%" (
    echo Install SLIM first using Install SLIM.cmd.
    pause
    exit /b 1
)
if exist "%~dp0my-inventory.db" (
    start "" "%SLIM_EXE%" --db "%~dp0my-inventory.db"
) else (
    start "" "%SLIM_EXE%"
)
