# Universal Patcher installer for Windows.
#   irm @ORIGIN@install.ps1 | iex
# Downloads the engine for this platform, checks its SHA-256 against the published bootstrap
# manifest, and runs its "install" command. The engine verifies the application release with
# its built-in trusted root and installs for the current user. No administrator rights needed.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# Windows PowerShell 5.1 can default to TLS versions the download service does not accept.
if ($PSVersionTable.PSVersion.Major -lt 6) {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
}

$Origin = '@ORIGIN@'
$Bootstrap = "${Origin}app/bootstrap"

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'Universal Patcher requires 64-bit Windows.'
}
# The machine's own architecture: an emulated x64 PowerShell on ARM reports AMD64 in
# PROCESSOR_ARCHITECTURE, so ask the OS, and fall back to the variables when that is unavailable.
$machine = try { [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() } catch { '' }
if (-not $machine) { $machine = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE } }
$arch = switch ($machine) {
    { $_ -in 'X64', 'AMD64' } { 'x64' }
    { $_ -in 'Arm64', 'ARM64' } { throw 'The Sims 4 does not run on Windows on ARM, so Universal Patcher is not offered there.' }
    default { throw "The Sims 4 does not run on this processor ($machine), so Universal Patcher is not offered there." }
}
$platform = "windows-$arch"

$version = (Invoke-RestMethod -UseBasicParsing "$Bootstrap/latest").ToString().Trim()
if ($version -notmatch '^\d+\.\d+\.\d+$') {
    throw 'The download service returned an unexpected version; try again later.'
}
$manifest = Invoke-RestMethod -UseBasicParsing "$Bootstrap/$version/manifest.json"
$entry = $manifest.platforms.$platform
if (-not $entry -or $entry.sha256 -notmatch '^[a-f0-9]{64}$') {
    throw "Universal Patcher $version is not available for $platform yet."
}

$work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $engine = Join-Path $work 'patcher-engine.exe'
    Invoke-WebRequest -UseBasicParsing "$Bootstrap/$version/$platform/patcher-engine.exe" -OutFile $engine
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $engine).Hash.ToLowerInvariant()
    if ($actual -ne $entry.sha256) {
        throw 'Checksum verification failed; nothing was installed.'
    }
    Write-Host "Installing Universal Patcher $version..."
    & $engine install --origin $Origin
    if ($LASTEXITCODE -ne 0) {
        throw "Installation failed with exit code $LASTEXITCODE."
    }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
