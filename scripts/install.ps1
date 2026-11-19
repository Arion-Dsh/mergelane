$ErrorActionPreference = 'Stop'

if ([Console]::IsInputRedirected) {
    throw 'Run the MergeLane installer in a terminal.'
}

$mergelanePreviousInstallDir = $env:MERGELANE_INSTALL_DIR
$mergelaneInstallRoot = if ($mergelanePreviousInstallDir) {
    $mergelanePreviousInstallDir
} else {
    Join-Path $HOME '.mergelane'
}
$mergelaneInstaller = Join-Path ([IO.Path]::GetTempPath()) "mergelane-$([guid]::NewGuid()).ps1"

try {
    Invoke-WebRequest -UseBasicParsing `
        -Uri 'https://github.com/Arion-Dsh/mergelane/releases/latest/download/mergelane-installer.ps1' `
        -OutFile $mergelaneInstaller
    $env:MERGELANE_INSTALL_DIR = $mergelaneInstallRoot
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $mergelaneInstaller
    if ($LASTEXITCODE -ne 0) {
        throw "MergeLane binary installation failed with exit code $LASTEXITCODE."
    }

    & (Join-Path $mergelaneInstallRoot 'bin/mergelane.exe') setup
    if ($LASTEXITCODE -ne 0) {
        throw "MergeLane setup failed with exit code $LASTEXITCODE."
    }
} finally {
    $env:MERGELANE_INSTALL_DIR = $mergelanePreviousInstallDir
    Remove-Item -LiteralPath $mergelaneInstaller -ErrorAction SilentlyContinue
}
