param(
  [Parameter(Mandatory=$true)][string]$TestExecutable,
  [Parameter(Mandatory=$true)][string]$QtCore,
  [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$exe = (Resolve-Path -LiteralPath $TestExecutable).Path
$qt = (Resolve-Path -LiteralPath $QtCore).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Evidence directory must be new' }
$dir = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$oldQt = $env:PATCHER_TEST_QT
try {
  $env:PATCHER_TEST_QT = $qt
  $test = 'windows::native_tests::real_qt_request_response_and_invalidation'
  $started = [DateTime]::UtcNow
  $clock = [Diagnostics.Stopwatch]::StartNew()
  $lines = @(& $exe --ignored --exact $test --nocapture 2>&1 | ForEach-Object { "$_" })
  $code = $LASTEXITCODE
  $clock.Stop()
  $text = $lines -join "`n"
  [IO.File]::WriteAllText((Join-Path $dir 'native-test.txt'), $text + "`n")
  $passed = $code -eq 0 -and $text.Contains('1 passed; 0 failed') -and
    $text.Contains('Observed real Qt allocation, Rust replacement, Qt destruction, duplicate suppression and selection invalidation')
  $evidence = [ordered]@{
    schema = 1
    observed_at = $started.ToString('o')
    platform = [Environment]::OSVersion.VersionString
    test = $test
    executable_sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    qt_sha256 = (Get-FileHash -LiteralPath $qt -Algorithm SHA256).Hash.ToLowerInvariant()
    qt_version = (Get-Item -LiteralPath $qt).VersionInfo.FileVersion
    exit_code = $code
    elapsed_seconds = $clock.Elapsed.TotalSeconds
    passed = $passed
    scope = 'Real Qt allocation and mutation with synthetic protocol input; no in-game activation or account handoff established'
    log = 'native-test.txt'
  }
  $json = $evidence | ConvertTo-Json -Depth 4
  [IO.File]::WriteAllText((Join-Path $dir 'evidence.json'), $json + "`n")
  $json
  if (-not $passed) { throw 'Native replacement test failed; inspect native-test.txt' }
} finally {
  $env:PATCHER_TEST_QT = $oldQt
}
