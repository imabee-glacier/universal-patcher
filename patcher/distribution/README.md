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

GitHub builds; our server publishes; users download only from `https://play.imabee.com/`.

1. Add `distribution/release-notes/X.Y.Z.txt` (paragraphs separated by blank lines) and, if a
   platform has passed its checks in `AGENTS.md`, add it to `distribution/offer`. Merge to main.
2. Tag the merge commit `app-vX.Y.Z` and push the tag. `.github/workflows/release.yml` builds
   Linux on Ubuntu and Windows on a Windows runner, both with the origin and
   `distribution/app-root.json` baked in, packages them, compiles the Windows installer with the
   pinned Inno Setup image, checks every engine carries the origin and the root keys, and
   attaches `patcher-X.Y.Z-staging.tar.gz` (and its SHA-256) to a release in this private
   repository. No signing key is used on GitHub.
3. From a workstation with SSH to the host: `tools/release-to-site.sh X.Y.Z`. It downloads the
   archive, checks its checksum, origin, root keys and version, uploads it to the host's
   `staging/app`, runs the `publish-app` Job with the online key held in the cluster, runs the
   health check, and checks the live site.

`publish-app` stops before any change when the version is not newer than the published one, an
offered platform has no packaged build, or a Windows installer has no Windows build. It copies objects and engines, signs the application manifests, writes `install.sh` and
`install.ps1` with the origin filled in, moves `app/bootstrap/latest`, and finally writes the
website's `site/data/catalog.json`: one download per packaged platform, marked verified only
when offered, and release notes accumulated across releases.

`distribution/app-root.json` is the public half of the release keys (`patcher init-root`). The
root key that signs it is kept offline; the online key exists only in the cluster's
`release-signing` Secret.

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
