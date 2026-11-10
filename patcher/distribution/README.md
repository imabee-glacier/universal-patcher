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

Updates to the patcher itself are downloaded by `universal-patcher update`, or by a scheduled
run when background maintenance has been enabled (it is off by default). The desktop app
checks only when asked: Settings, Check for patcher update.

`universal-patcher launch` starts the current release. A downloaded update starts only at the
next launch, and only when game maintenance is not running. If the new release does not show
its interface within 60 seconds, the launcher returns to the previous release and never
activates the failed one again. `universal-patcher uninstall` keeps game choices unless given
`--remove-data`.

## Releasing

GitHub builds; our server publishes; users download only from `https://play.imabee.com/`.

1. Add `distribution/release-notes/X.Y.Z.txt` (paragraphs separated by blank lines). To offer
   a platform's download on the website, add it to `distribution/offer` and record what was
   run on that platform under "Checked" below. Merge to main.
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
website's `site/data/catalog.json`: one download per packaged platform with `offered: true`
only for platforms listed in `distribution/offer`, and release notes accumulated across
releases. Catalogs published before 0.1.6 call that field `verified`; the website reads both.
`offered` is a release decision, not a test result: what was actually checked is listed below.

`distribution/app-root.json` is the public half of the release keys (`patcher init-root`). The
root key that signs it is kept offline; the online key exists only in the cluster's
`release-signing` Secret.

## Offered

`distribution/offer` lists `linux-x64` and `windows-x64`. macOS is not offered: no macOS build
exists. On every platform the app reads the game folder and does not change game files; it
cannot download, update or unlock the game or its packs (see the 0.1.5 and 0.1.6 notes).

## Checked

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
- Windows, by hand on one Windows 11 machine, as recorded in the commit that added
  `windows-x64` to `distribution/offer` (15c6c4f): install with the one-line command and with
  the setup program, launch and the startup check, automatic game discovery, two patcher
  updates, and uninstall with and without data. No automated test runs on Windows, and no other
  Windows version or machine has been recorded.

## Not checked

- Windows beyond the manual run above: other Windows versions, launcher replacement while it
  is running, and PATH and Apps & features registration on a second machine.
- macOS: `install.sh` handles it, but the engine does not install until a macOS build exists.
- Code signing: Windows binaries and the installer are unsigned; SmartScreen warns until they
  are signed, and the website says so on the Windows download. macOS needs signing and
  notarization.
- The bootstrap step trusts HTTPS, as Claude Code's installer does; everything after it is
  checked against the built-in TUF root.
- The health check (`patcher health`) probes only the download service: `/healthz`, both install
  scripts, `app/bootstrap/latest`, and the expiry of the patcher's signed metadata. Its
  `operational` result says nothing about the app or the game.
