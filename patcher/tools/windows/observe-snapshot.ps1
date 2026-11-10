param(
  [Parameter(Mandatory = $true)][string]$CoreLibrary,
  [Parameter(Mandatory = $true)][string]$OutputDirectory
)
# Read-only observation of the production Rust bridge: it loads the real cdylib, lets
# discovery run, and records the snapshot the interface would draw. It issues no command
# and starts no game.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$core = (Resolve-Path -LiteralPath $CoreLibrary).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Evidence directory must be new' }
$dir = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PatcherSnapshotOnly {
  [DllImport("kernel32", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern IntPtr LoadLibraryW(string path);
  [DllImport("kernel32", CharSet=CharSet.Ansi, ExactSpelling=true)]
  static extern IntPtr GetProcAddress(IntPtr module, string name);
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate IntPtr Snapshot();
  [UnmanagedFunctionPointer(CallingConvention.Cdecl)] delegate void Free(IntPtr value);
  static Snapshot snapshot; static Free free; static IntPtr module;
  public static void Load(string path) {
    module = LoadLibraryW(path);
    if (module == IntPtr.Zero) throw new Exception("Core load failed: " + Marshal.GetLastWin32Error());
    snapshot = (Snapshot)Marshal.GetDelegateForFunctionPointer(GetProcAddress(module, "patcher_snapshot"), typeof(Snapshot));
    free = (Free)Marshal.GetDelegateForFunctionPointer(GetProcAddress(module, "patcher_free"), typeof(Free));
  }
  public static string Read() {
    var p = snapshot();
    if (p == IntPtr.Zero) throw new Exception("Null snapshot");
    try {
      int n = 0; while (Marshal.ReadByte(p, n) != 0) { if (++n > 16777216) throw new Exception("Oversized"); }
      var b = new byte[n]; Marshal.Copy(p, b, 0, n); return Encoding.UTF8.GetString(b);
    } finally { free(p); }
  }
}
'@
[PatcherSnapshotOnly]::Load($core)
$deadline = [DateTime]::UtcNow.AddSeconds(30)
$last = $null
do {
  $raw = [PatcherSnapshotOnly]::Read()
  [IO.File]::AppendAllText((Join-Path $dir 'snapshots.jsonl'), $raw + "`n")
  $s = $raw | ConvertFrom-Json
  $last = $s
  if ($s.play -ne 'hidden' -or $s.state -in @('ready', 'failed', 'needs_attention', 'blocked')) { break }
  Start-Sleep -Milliseconds 300
} while ([DateTime]::UtcNow -lt $deadline)
$evidence = [ordered]@{
  schema        = 1
  observed_at   = [DateTime]::UtcNow.ToString('o')
  platform      = [Environment]::OSVersion.VersionString
  core_sha256   = (Get-FileHash -LiteralPath $core -Algorithm SHA256).Hash.ToLowerInvariant()
  scope         = 'Read-only production bridge discovery on a native installation; no command issued, no game started'
  final         = $last
}
$json = $evidence | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText((Join-Path $dir 'evidence.json'), $json + "`n")
$json
