param(
  [Parameter(Mandatory=$true)][string]$CoreLibrary,
  [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$core = (Resolve-Path -LiteralPath $CoreLibrary).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Evidence directory must be new' }
$dir = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
# Call the production bridge. Discovery, launch choice and process tracking stay in Rust.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PatcherObservation {
  [DllImport("kernel32", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern IntPtr LoadLibraryW(string path);
  [DllImport("kernel32", CharSet=CharSet.Ansi, ExactSpelling=true)]
  static extern IntPtr GetProcAddress(IntPtr module, string name);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate IntPtr Snapshot();
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate IntPtr Command(IntPtr input);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate void Free(IntPtr value);
  static Snapshot snapshot;
  static Command command;
  static Free free;
  static IntPtr module;
  static T Proc<T>(string name) {
    var p = GetProcAddress(module, name);
    if (p == IntPtr.Zero) throw new Exception("Missing bridge export: " + name);
    return (T)(object)Marshal.GetDelegateForFunctionPointer(p, typeof(T));
  }
  public static void Load(string path) {
    module = LoadLibraryW(path);
    if (module == IntPtr.Zero) throw new Exception("Core load failed: " + Marshal.GetLastWin32Error());
    snapshot = Proc<Snapshot>("patcher_snapshot");
    command = Proc<Command>("patcher_command");
    free = Proc<Free>("patcher_free");
  }
  static string Take(IntPtr p) {
    if (p == IntPtr.Zero) throw new Exception("Null bridge response");
    try {
      int n = 0;
      while (Marshal.ReadByte(p, n) != 0) {
        if (++n > 16777216) throw new Exception("Oversized bridge response");
      }
      var bytes = new byte[n];
      Marshal.Copy(p, bytes, 0, n);
      return Encoding.UTF8.GetString(bytes);
    } finally { free(p); }
  }
  public static string Read() { return Take(snapshot()); }
  public static string Submit(string json) {
    var bytes = Encoding.UTF8.GetBytes(json + "\0");
    var p = Marshal.AllocHGlobal(bytes.Length);
    try {
      Marshal.Copy(bytes, 0, p, bytes.Length);
      return Take(command(p));
    } finally { Marshal.FreeHGlobal(p); }
  }
}
'@
$started = [DateTime]::UtcNow
$events = New-Object System.Collections.Generic.List[object]
$lastRevision = $null
$result = [ordered]@{
  schema = 1
  observed_at = $started.ToString('o')
  core_sha256 = (Get-FileHash -LiteralPath $core -Algorithm SHA256).Hash.ToLowerInvariant()
  scope = 'Production Rust discovery and Launch command on a native installation; does not establish DLC entitlement activation'
  passed = $false
  error = $null
}
function Read-Snapshot {
  $s = [PatcherObservation]::Read() | ConvertFrom-Json
  if ($s.revision -ne $script:lastRevision) {
    $script:lastRevision = $s.revision
    $event = [ordered]@{
      time = [DateTime]::UtcNow.ToString('o')
      revision = $s.revision
      state = $s.state
      phase = $s.phase
      ready = $s.ready
      play = $s.play
      play_reason = $s.play_reason
      path = $s.path
      error = $s.error
    }
    $events.Add($event)
    [IO.File]::AppendAllText((Join-Path $dir 'snapshots.jsonl'), (($event | ConvertTo-Json -Compress) + "`n"))
  }
  return $s
}
try {
  [PatcherObservation]::Load($core)
  $deadline = [DateTime]::UtcNow.AddSeconds(90)
  do {
    $s = Read-Snapshot
    if ($s.play -eq 'playing') { throw 'The game was already running before the Launch command' }
    if ($s.play -eq 'available') { break }
    if ($s.state -in @('failed','error','needs_attention')) {
      throw 'Discovery needs attention; inspect the snapshot record'
    }
    Start-Sleep -Milliseconds 250
  } while ([DateTime]::UtcNow -lt $deadline)
  if ($s.play -ne 'available') { throw 'Rust did not offer Play within 90 seconds' }
  $reply = [PatcherObservation]::Submit('{"id":"native-launch-observation","command":"Launch"}') | ConvertFrom-Json
  $result.command = $reply
  if (-not $reply.accepted) { throw 'Rust rejected Launch' }
  $deadline = [DateTime]::UtcNow.AddSeconds(90)
  $playingSince = $null
  do {
    $s = Read-Snapshot
    if ($s.play -eq 'playing') {
      if ($null -eq $playingSince) { $playingSince = [DateTime]::UtcNow }
      if (([DateTime]::UtcNow - $playingSince).TotalSeconds -ge 10) {
        $result.passed = $true
        break
      }
    } elseif ($null -ne $playingSince) {
      throw 'The game exited before ten seconds of observed play'
    } elseif ($s.play -notin @('starting','playing')) {
      throw 'Rust returned from Starting without observing a running game'
    }
    Start-Sleep -Milliseconds 250
  } while ([DateTime]::UtcNow -lt $deadline)
  if (-not $result.passed) { throw 'Rust did not observe stable play within 90 seconds' }
} catch {
  $result.error = $_.Exception.Message
} finally {
  $result.finished_at = [DateTime]::UtcNow.ToString('o')
  $result.snapshots = 'snapshots.jsonl'
  $json = $result | ConvertTo-Json -Depth 5
  [IO.File]::WriteAllText((Join-Path $dir 'evidence.json'), $json + "`n")
  $json
}
if (-not $result.passed) { exit 1 }
