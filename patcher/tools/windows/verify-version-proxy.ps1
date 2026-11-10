param([Parameter(Mandatory=$true)][string]$Proxy, [Parameter(Mandatory=$true)][string]$Target)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class VersionProxyCheck {
  [DllImport("kernel32", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern IntPtr LoadLibraryW(string path);
  [DllImport("kernel32", CharSet=CharSet.Ansi, ExactSpelling=true)]
  static extern IntPtr GetProcAddress(IntPtr module, string name);
  [UnmanagedFunctionPointer(CallingConvention.Winapi, CharSet=CharSet.Unicode)]
  delegate uint Size(string path, out uint handle);
  [UnmanagedFunctionPointer(CallingConvention.Winapi, CharSet=CharSet.Unicode)]
  delegate int Info(string path, uint handle, uint size, IntPtr buffer);
  [UnmanagedFunctionPointer(CallingConvention.Winapi, CharSet=CharSet.Unicode)]
  delegate int Query(IntPtr buffer, string key, out IntPtr value, out uint size);
  static T Proc<T>(IntPtr module, string name) {
    var p = GetProcAddress(module, name);
    if (p == IntPtr.Zero) throw new Exception("Missing export: " + name);
    return (T)(object)Marshal.GetDelegateForFunctionPointer(p, typeof(T));
  }
  static byte[] Read(IntPtr module, string target) {
    uint handle;
    uint size = Proc<Size>(module, "GetFileVersionInfoSizeW")(target, out handle);
    if (size == 0 || size > 16777216) throw new Exception("Invalid version resource size");
    IntPtr buffer = Marshal.AllocHGlobal((int)size);
    try {
      for (int i=0; i<size; ++i) Marshal.WriteByte(buffer, i, 0);
      if (Proc<Info>(module, "GetFileVersionInfoW")(target, 0, size, buffer) == 0)
        throw new Exception("Version resource read failed");
      IntPtr value; uint length;
      if (Proc<Query>(module, "VerQueryValueW")(buffer, "\\", out value, out length) == 0 || length != 52)
        throw new Exception("Fixed version info missing");
      byte[] result = new byte[length]; Marshal.Copy(value, result, 0, (int)length);
      return result;
    } finally { Marshal.FreeHGlobal(buffer); }
  }
  public static string Verify(string proxy, string target) {
    var system = LoadLibraryW(System.IO.Path.Combine(Environment.SystemDirectory, "version.dll"));
    var replacement = LoadLibraryW(System.IO.Path.GetFullPath(proxy));
    if (system == IntPtr.Zero || replacement == IntPtr.Zero || system == replacement)
      throw new Exception("Could not load distinct proxy and system DLLs");
    string[] exports = {"GetFileVersionInfoA", "GetFileVersionInfoW", "GetFileVersionInfoSizeA",
      "GetFileVersionInfoSizeW", "VerQueryValueA", "VerQueryValueW"};
    foreach (var name in exports) if (GetProcAddress(replacement, name) == IntPtr.Zero)
      throw new Exception("Missing EA app import: " + name);
    var expected = Read(system, target); var actual = Read(replacement, target);
    if (Convert.ToBase64String(expected) != Convert.ToBase64String(actual))
      throw new Exception("Forwarded version data differs from system DLL");
    // Keep modules loaded until process exit; unloading a live probe is unsupported.
    return "version forwarding matches system DLL; all six EA app imports resolve";
  }
}
'@
[VersionProxyCheck]::Verify($Proxy, $Target)
