# Builds the release binary and the Windows installer:
#   target\installer\TinyTodo-Setup-<version>.exe
# Needs Inno Setup 6 (https://jrsoftware.org/isdl.php).
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE) { throw 'cargo build failed' }

    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
    $iscc = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { throw 'Inno Setup 6 not found (https://jrsoftware.org/isdl.php)' }

    & $iscc /Q "/DAppVersion=$version" installer\tinytodo.iss
    if ($LASTEXITCODE) { throw 'ISCC failed' }
    Get-Item "target\installer\TinyTodo-Setup-$version.exe"
}
finally {
    Pop-Location
}
