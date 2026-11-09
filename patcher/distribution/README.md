# Distribution

## What users run

| Platform | Command |
|---|---|
| Linux, macOS | `curl -fsSL https://<domain>/install.sh \| bash` |
| Windows (PowerShell) | `irm https://<domain>/install.ps1 \| iex` |
| Windows (installer) | `UniversalPatcherSetup-<version>.exe` |

All three run the same engine command, `patcher-engine install`. No administrator rights are
needed. The script reads `app/bootstrap/latest`, downloads that version's engine for the
platform, checks its SHA-256 against `app/bootstrap/<v>/manifest.json`, and runs it. The engine
carries the trusted root built into it, verifies the signed application release through TUF,
and unpacks it.

Installed layout (per user):

| | Linux | Windows |
|---|---|---|
| Data root | `~/.local/share/universal-patcher` | `%LOCALAPPDATA%\UniversalPatcher` |
| Releases | `versions/<v>/` | `versions\<v>\` |
| Launcher | `bin/universal-patcher`, linked from `~/.local/bin` | `bin\universal-patcher.exe`, added to the user PATH |
| Menu entry | `~/.local/share/applications/universal-patcher.desktop` | Start menu shortcut, Apps & features entry |

`universal-patcher launch` starts the current release. A downloaded update starts only at the
next launch, and only when game maintenance is not running. If the new release does not show
its interface within 60 seconds, the launcher returns to the previous release and never
activates the failed one again. `universal-patcher uninstall` keeps game choices unless given
`--remove-data`.

## Releasing

Build each platform on its own runner with the origin and the application root baked in:

```
PATCHER_DIST_ORIGIN=https://<domain>/ PATCHER_DIST_ROOT=<app root.json> flutter build linux --release
PATCHER_DIST_ORIGIN=https://<domain>/ PATCHER_DIST_ROOT=<app root.json> flutter build windows --release
```

Package every platform into one staging directory, then publish once:

```
patcher package-app <linux bundle>   --version X.Y.Z --os linux   --arch x64 --staging <dir>
patcher package-app <windows bundle> --version X.Y.Z --os windows --arch x64 --staging <dir>
patcher publish-app <dir> --root <app root.json> --key <online key> --repository <repo> \
    --origin https://<domain>/ --offer linux-x64 --notes notes.txt [--windows-setup <setup.exe>]
```

On appsynergy-server this runs as the `publish-app` Job (`../deploy/README.md`).

`publish-app` refuses a version that is not newer than the published one, an `--offer` for a
platform with no packaged build, and a Windows installer without a Windows build, before
anything changes. It copies objects and engines (staging may be read-only), signs the
application manifests, writes `install.sh` and `install.ps1` with the origin filled in, moves
`app/bootstrap/latest`, and finally writes the website's `site/data/catalog.json`: one download
per packaged platform, marked verified only when offered, and release notes accumulated across
releases. Offer a platform only after its checks in `AGENTS.md` pass.

Windows installer, compiled with Inno Setup 6 (`iscc`, or the pinned `amake/innosetup` image):

```
iscc /DAppVersion=X.Y.Z /DOrigin=https://<domain>/ /DEngineExe=<patcher-engine.exe> windows/universal-patcher.iss
```

## Verified

- Linux: `tools/e2e-install.sh` builds a release with a baked test origin and root, publishes
  it, serves it over HTTPS and HTTP/3 through the production front image, runs
  `curl ... | bash` in an empty home, checks the layout, desktop entry and icon, and confirms a
  tampered engine is not installed. An earlier run started the installed copy through its launcher on
  a virtual display, and it passed its startup check.
- `core/tests/distribution.rs`: install, launch, a broken update rolled back and never retried,
  a good update activated with old releases pruned, an older version left unpublished,
  tampered objects left unused, uninstall with and without data, archives that try to escape
  their release directory, sparse files in a build, publishing from read-only staging, the
  website catalog and publishes that leave it unchanged, and the health check.
- Windows, from Linux: the core and engine cross-compile and link for `x86_64-pc-windows-gnu`;
  `install.ps1` parses and, in PowerShell 7.5, downloads and checksum-verifies the engine from
  a repository served over HTTP, and does not install a tampered engine, an unsupported
  architecture, or an unpublished platform; the Inno Setup script compiles.

## Not verified

- Anything executed on Windows: the Flutter Windows build (MSVC, Windows only), the Start menu
  shortcut, PATH and Apps & features registration, launcher replacement while running, and the
  installer run. These need a Windows machine before Windows is offered.
- macOS: `install.sh` handles it, but the engine does not install until a macOS build exists.
- Code signing: Windows binaries and the installer are unsigned; SmartScreen will warn until
  they are signed. macOS needs signing and notarization.
- The bootstrap step trusts HTTPS, as Claude Code's installer does; everything after it is
  verified against the built-in TUF root.
