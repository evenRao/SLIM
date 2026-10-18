[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\SLIM'),
    [string]$ShortcutDirectory,
    [switch]$NoLaunch,
    [switch]$NoShortcuts
)
$ErrorActionPreference = 'Stop'
$source = $PSScriptRoot
$binaryRoot = $source
if (-not (Test-Path -LiteralPath (Join-Path $binaryRoot 'slim-app.exe'))) {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    $cargoPath = if ($cargo) { $cargo.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
    if (-not (Test-Path -LiteralPath $cargoPath)) {
        throw 'This source checkout needs Rust 1.89 or newer and the Visual C++ build tools. Use the prebuilt Windows release archive to install without development tools.'
    }
    & $cargoPath build --manifest-path (Join-Path $source 'Cargo.toml') --release --locked --package slim-cli --bins
    if ($LASTEXITCODE -ne 0) { throw 'SLIM could not be built.' }
    $binaryRoot = Join-Path $source 'target\release'
}
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
# Shortcuts target immutable releases so a running older session cannot block an update.
$tuiBuildHash = (Get-FileHash -LiteralPath (Join-Path $binaryRoot 'slim.exe') -Algorithm SHA256).Hash.Substring(0, 12)
$browserBuildHash = (Get-FileHash -LiteralPath (Join-Path $binaryRoot 'slim-app.exe') -Algorithm SHA256).Hash.Substring(0, 12)
$buildHash = "$tuiBuildHash-$browserBuildHash".ToLowerInvariant()
$releaseDir = Join-Path $InstallDir "releases\$buildHash"
New-Item -ItemType Directory -Path $releaseDir -Force | Out-Null
foreach ($name in @('slim-app.exe', 'slim.exe')) {
    $sourceBinary = Join-Path $binaryRoot $name
    $releaseBinary = Join-Path $releaseDir $name
    $sourceHash = (Get-FileHash -LiteralPath $sourceBinary -Algorithm SHA256).Hash
    if (-not (Test-Path -LiteralPath $releaseBinary) -or (Get-FileHash -LiteralPath $releaseBinary -Algorithm SHA256).Hash -ne $sourceHash) {
        Copy-Item -LiteralPath $sourceBinary -Destination $releaseBinary -Force
    }
}
$resolvedInstallDir = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
$activeExecutables = @(Get-Process -Name 'slim','slim-app' -ErrorAction SilentlyContinue | ForEach-Object { try { $_.Path } catch {} })
foreach ($oldRelease in Get-ChildItem -LiteralPath (Join-Path $InstallDir 'releases') -Directory) {
    if ($oldRelease.Name -eq $buildHash -or $oldRelease.Name -notmatch '^[a-f0-9]{12}-[a-f0-9]{12}$') { continue }
    $oldPath = [IO.Path]::GetFullPath($oldRelease.FullName)
    if (-not $oldPath.StartsWith($resolvedInstallDir + '\releases\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Release cleanup escaped the installation folder' }
    if ($oldRelease.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
    if (@(Get-ChildItem -LiteralPath $oldPath -Force | Where-Object { $_.PSIsContainer -or $_.Name -notin @('slim.exe','slim-app.exe') }).Count) { continue }
    if (@($activeExecutables | Where-Object { $_ -and $_.StartsWith($oldPath + '\', [StringComparison]::OrdinalIgnoreCase) }).Count) { continue }
    Remove-Item -LiteralPath $oldPath -Recurse -Force
}
foreach ($legacyName in @('slim.exe','slim-app.exe')) {
    $legacyPath = Join-Path $resolvedInstallDir $legacyName
    if ((Test-Path -LiteralPath $legacyPath) -and $legacyPath -notin $activeExecutables) { Remove-Item -LiteralPath $legacyPath -Force }
}
$entryScript = "@echo off`r`n`"%~dp0releases\$buildHash\slim.exe`" %*`r`n"
Set-Content -LiteralPath (Join-Path $InstallDir 'slim.cmd') -Value $entryScript -Encoding ascii
if (-not $NoShortcuts) {
    $shortcutShell = New-Object -ComObject WScript.Shell
    $shortcutPaths = @(
        (Join-Path ([Environment]::GetFolderPath('Desktop')) 'SLIM.lnk'),
        (Join-Path ([Environment]::GetFolderPath('Programs')) 'SLIM.lnk')
    )
    if ($ShortcutDirectory) {
        New-Item -ItemType Directory -Path $ShortcutDirectory -Force | Out-Null
        $shortcutPaths = @(Join-Path $ShortcutDirectory 'SLIM.lnk')
    }
    foreach ($shortcutPath in $shortcutPaths) {
        $shortcut = $shortcutShell.CreateShortcut($shortcutPath)
        $shortcut.TargetPath = Join-Path $releaseDir 'slim.exe'
        $shortcut.Arguments = 'tui'
        $shortcut.WorkingDirectory = $InstallDir
        $shortcut.Description = 'SLIM Ratatui inventory workspace'
        $shortcut.Save()
        $browserPath = Join-Path (Split-Path $shortcutPath -Parent) 'SLIM Browser.lnk'
        $browserShortcut = $shortcutShell.CreateShortcut($browserPath)
        $browserShortcut.TargetPath = Join-Path $releaseDir 'slim-app.exe'
        $browserShortcut.WorkingDirectory = $InstallDir
        $browserShortcut.Description = 'SLIM browser inventory workspace'
        $browserShortcut.Save()
    }
}
Write-Host "SLIM installed at $InstallDir. Inventory data is stored separately and is never removed by this installer."
if (-not $NoLaunch) { Start-Process -FilePath (Join-Path $releaseDir 'slim.exe') -ArgumentList 'tui' -WorkingDirectory $InstallDir -WindowStyle Normal }
