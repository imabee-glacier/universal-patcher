param(
  [Parameter(Mandatory = $true)][string]$Engine,
  [Parameter(Mandatory = $true)][string]$OutputDirectory
)
# Observe an already-built patcher.exe driving a synthetic signed release through
# interruption, recovery, repair and a no-op on the real local filesystem. It never
# builds or publishes. This is the Windows counterpart of tools/observe-engine.py for
# hosts without Python; the checks are the same.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8

$engine = (Resolve-Path -LiteralPath $Engine).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Evidence directory must be new' }
$out = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$fixture = Join-Path $out 'fixture'
$config = Join-Path $fixture 'config.json'
$game = Join-Path $fixture 'game'
$commands = New-Object System.Collections.Generic.List[object]

function Require($condition, $message) { if (-not $condition) { throw $message } }

function Sha([string]$path) {
  (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Invoke-Engine([string]$name, [string[]]$arguments, [int]$expected = 0) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $stdout = Join-Path $out "$name.stdout.jsonl"
  $stderr = Join-Path $out "$name.stderr.txt"
  $p = Start-Process -FilePath $engine -ArgumentList $arguments -NoNewWindow -Wait -PassThru `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
  $sw.Stop()
  $commands.Add([ordered]@{ name = $name; argv = @($engine) + $arguments; exit_code = $p.ExitCode; elapsed_seconds = [math]::Round($sw.Elapsed.TotalSeconds, 3) })
  Require ($p.ExitCode -eq $expected) "${name}: unexpected exit $($p.ExitCode): $(Get-Content -Raw -LiteralPath $stderr)"
  return [pscustomobject]@{ stdout = (Get-Content -Raw -LiteralPath $stdout); stderr = (Get-Content -Raw -LiteralPath $stderr) }
}

function Get-Policy { (Get-Content -Raw -LiteralPath (Join-Path $fixture 'state\policy.json') | ConvertFrom-Json).payload }

function Get-Records([string]$stdout) {
  if ([string]::IsNullOrWhiteSpace($stdout)) { return @() }
  $stdout -split "`r?`n" | Where-Object { $_.Trim() -ne '' } | ForEach-Object { $_ | ConvertFrom-Json }
}

function Get-Report($result) {
  $records = @(Get-Records $result.stdout)
  Require (@($records | Where-Object { $_.phase -eq 'ready' }).Count -gt 0) 'No Ready progress'
  return $records[-1]
}

# 1. Create the signed synthetic fixture.
Invoke-Engine 'create' @('fixture', $fixture) | Out-Null
$originalPolicy = Get-Policy
$unrelated = Join-Path $game 'unmanaged.txt'
[IO.File]::WriteAllText($unrelated, "unrelated fixture data must survive`n")

# 2. Interrupt immediately after the first atomic rename.
$interrupted = Invoke-Engine 'interrupted' @('run', '--config', $config, '--interrupt-after-rename', '1') 1
Require ($interrupted.stderr -match 'Injected interruption') 'Run did not reach the injected boundary'
Require ($null -eq (Get-Policy).applied) 'Interrupted run claimed applied policy'
Require (-not ($interrupted.stdout -match '"phase":"ready"')) 'Interrupted run claimed Ready'
$committed = Join-Path $game 'Data\base.package'
$baseInput = Join-Path $fixture 'input\base'
Require ([IO.File]::ReadAllBytes($committed).Length -eq (Get-Item -LiteralPath $baseInput).Length) 'Interrupted commit size differs'
Require ((Sha $committed) -eq (Sha $baseInput)) 'Interrupted commit bytes differ'
$committedMtime = (Get-Item -LiteralPath $committed).LastWriteTimeUtc.Ticks

# 3. Resume: recovery must finish EP01 without rewriting the committed base file.
$recovered = Get-Report (Invoke-Engine 'recovered' @('run', '--config', $config))
Require (@($recovered.replaced) -join ',' -eq 'EP01/content.package') 'Recovery repeated the committed replacement'
Require ((Get-Item -LiteralPath $committed).LastWriteTimeUtc.Ticks -eq $committedMtime) 'Recovery rewrote the committed file'
$expected = @{ 'Data/base.package' = 'base'; 'EP01/content.package' = 'EP01' }
$comparisons = New-Object System.Collections.Generic.List[object]
foreach ($dest in $expected.Keys) {
  $actual = Join-Path $game ($dest -replace '/', '\')
  $src = Join-Path $fixture ('input\' + $expected[$dest])
  Require ((Sha $actual) -eq (Sha $src)) "Recovery bytes differ: $dest"
  $comparisons.Add([ordered]@{ path = $dest; bytes = (Get-Item -LiteralPath $actual).Length; actual_sha256 = (Sha $actual); source_sha256 = (Sha $src) })
}
Require (-not (Test-Path -LiteralPath (Join-Path $game 'SP01\content.package'))) 'Disabled DLC was installed'

# 4. Same-size same-mtime corruption is only caught by an explicit full verify.
$damaged = Join-Path $game 'EP01\content.package'
$info = Get-Item -LiteralPath $damaged
$size = $info.Length
$mtime = $info.LastWriteTimeUtc
[IO.File]::WriteAllBytes($damaged, (New-Object byte[] $size))
$d = Get-Item -LiteralPath $damaged; $d.LastWriteTimeUtc = $mtime
$repaired = Get-Report (Invoke-Engine 'repaired' @('run', '--config', $config, '--verify'))
Require (@($repaired.replaced) -join ',' -eq 'EP01/content.package') 'Repair changed unexpected files'
Require ((Sha $damaged) -eq (Sha (Join-Path $fixture 'input\EP01'))) 'Repair bytes differ'

# 5. A clean installation is a no-op: no downloads, no writes, nothing touched.
$mtimes = @{}; foreach ($dest in $expected.Keys) { $mtimes[$dest] = (Get-Item -LiteralPath (Join-Path $game ($dest -replace '/', '\'))).LastWriteTimeUtc.Ticks }
$noopResult = Invoke-Engine 'noop' @('run', '--config', $config)
$noop = Get-Report $noopResult
Require ((@($noop.replaced).Count -eq 0) -and (@($noop.deleted).Count -eq 0)) 'No-op mutated files'
Require (@(Get-Records $noopResult.stdout | Where-Object { $_.phase -eq 'downloading' }).Count -eq 0) 'No-op downloaded objects'
foreach ($dest in $expected.Keys) { Require ((Get-Item -LiteralPath (Join-Path $game ($dest -replace '/', '\'))).LastWriteTimeUtc.Ticks -eq $mtimes[$dest]) "No-op changed mtime: $dest" }
Require ([IO.File]::ReadAllText($unrelated) -eq "unrelated fixture data must survive`n") 'Unrelated file changed'
$finalPolicy = Get-Policy
Require (($finalPolicy.desired | ConvertTo-Json -Depth 8) -eq ($originalPolicy.desired | ConvertTo-Json -Depth 8)) 'Choices changed'
Require ($finalPolicy.applied.revision -eq $finalPolicy.revision) 'Applied revision differs'

$evidence = [ordered]@{
  schema                  = 1
  observed_at             = [DateTime]::UtcNow.ToString('o')
  platform                = [Environment]::OSVersion.VersionString
  engine_sha256           = (Sha $engine)
  scope                   = 'Synthetic signed CLI fixture on the real local filesystem; no real game, platform unlock or online handoff established'
  commands                = $commands
  recovery                = $recovered
  repair                  = $repaired
  noop                    = $noop
  byte_comparisons        = $comparisons
  disabled_pack_absent    = $true
  choices_preserved       = $true
  unrelated_file_preserved = $true
  applied                 = $finalPolicy.applied
}
$json = $evidence | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText((Join-Path $out 'evidence.json'), $json + "`n")
$json
