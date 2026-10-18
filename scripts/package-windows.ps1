[CmdletBinding()]
param([string]$BinaryRoot)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if (-not $BinaryRoot) { $BinaryRoot = Join-Path $root 'target\release' }
$folder = Join-Path $root 'dist\SLIM-Windows'
New-Item -ItemType Directory -Path $folder -Force | Out-Null
foreach ($name in @('slim.exe','slim-app.exe')) {
    Copy-Item -LiteralPath (Join-Path $BinaryRoot $name) -Destination (Join-Path $folder $name) -Force
}
foreach ($name in @('install.ps1','Install SLIM.cmd','Start SLIM.cmd','README.md','LICENSE')) {
    Copy-Item -LiteralPath (Join-Path $root $name) -Destination (Join-Path $folder $name) -Force
}
Copy-Item -LiteralPath (Join-Path $root 'launchers\Start SLIM Browser.cmd') -Destination (Join-Path $folder 'Start SLIM Browser.cmd') -Force
Compress-Archive -LiteralPath $folder -DestinationPath (Join-Path $root 'dist\SLIM-Windows.zip') -Force
Get-FileHash -LiteralPath (Join-Path $root 'dist\SLIM-Windows.zip') -Algorithm SHA256
