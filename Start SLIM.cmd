@echo off
setlocal
set "SLIM_EXE=%~dp0slim.exe"
if not exist "%SLIM_EXE%" set "SLIM_EXE=%LOCALAPPDATA%\Programs\SLIM\slim.cmd"
if not exist "%SLIM_EXE%" set "SLIM_EXE=%~dp0dist\SLIM-Windows\slim.exe"
if not exist "%SLIM_EXE%" (
    call "%~dp0Install SLIM.cmd"
    exit /b
)
if exist "%~dp0my-inventory.db" (
    call "%SLIM_EXE%" --db "%~dp0my-inventory.db" tui
) else (
    call "%SLIM_EXE%" tui
)
if errorlevel 1 pause
