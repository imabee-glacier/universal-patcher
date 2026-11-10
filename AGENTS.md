# AGENTS.md

Operating manual for `universal-patcher`. Follow it on every task. This is the only product document; do not add a second spec.

## Scope

One patcher maintains The Sims 4: game updates, repair, selected DLC, unlock, and language. Implement Sims 4 only. Keep game identifiers and adapters separate from the transfer engine so another game can be added later. Build the program only when the user orders a build.

Rust owns discovery, policy, planning, verification, transfer, installation, and recovery. Flutter draws: one `lib/` for Linux, Mac, and Windows, with `linux/`, `macos/`, and `windows/` runner shells. OS differences belong in Rust adapters. Dart holds no plan, hash, URL, or retry logic.

A K3s node will host the website, patcher installers, application updates, and game/DLC releases. This document specifies requirements; implementation must be checked separately. Previous projects supply evidence of behaviour; their architecture and performance claims are not accepted without testing.

## Implementation boundary

Implement the publisher, Rust engine, and Flutter interface against the contracts below. Start with full objects and raw/zstd decoding; delta lists may be empty. Add a delta codec only after reconstruction and resource tests pass. Do not delay reliable full-file updates for delta optimization.

Platform adapters advertise discovery, file maintenance, language, unlock, launch, and online capabilities independently. An unverified capability returns Unsupported with a reason; it must never simulate success. This is an implementation boundary, not permission to call an incomplete product finished.

Before enabling a platform for users, supply fixtures proving its markers, install roots, version evidence, pack/language ownership, shared dependencies, required attributes, launch command, and unlock postconditions. Classify every source entry and validate the resulting selected-content install. Native Mac support requires native Mac evidence; Windows support requires Windows evidence. Online Connect requires a demonstrated acquisition and handoff; automatic renewal additionally requires a demonstrated renewal mechanism. Keep unavailable controls labelled with their reason. A demonstrated provider flow is required. A copied browser session or an invented endpoint does not qualify.

Build against local test fixtures while these checks are pending. Publication requires the relevant platform checks and the acceptance tests below. Hosting configuration supplies the domain, storage, trusted root, publishing credentials, and backup destination; none belongs in game logic.

## Source evidence

Inspected on 2026-11-09: a full-installation source archive with 106 packs (80,656,791,399 bytes). Python `zipfile` central-directory inspection found 6,450 files and 688 directories; all entries use ZIP stored mode. Declared file payload totals 80,655,310,813 bytes.

The 106 root pack directories are `EP01–EP20`, `GP01–GP12`, `FP01`, `SP01–SP18`, `SP20–SP26`, `SP28–SP74`, and `SP81`. Matching their individual paths yields 2,674 files and 52,641,033,849 bytes. `Delta/<pack-id>/` covers the same ids: 2,701 files and 6,917,886,673 bytes, including package, world, configuration, and ZIP files. These are source game paths, not evidence of transport deltas. Neither directory presence nor the archive title proves pack completeness.

The archive also contains `Data/`, Windows binaries under `Game/`, third-party DLLs, installers, and extras. No duplicate paths, case-fold collisions, parent traversal, backslash paths, absolute paths, or Unix symlink entries were found in this directory inspection. This does not verify payload integrity, authenticity, game version, dependencies, or native Mac compatibility.

Compression spot-check with Zstandard CLI 1.5.7: whole files selected as the median by size then path among root-pack files up to 16 MiB in each category (non-language package, language package, world). ZIP CRC checks and decompression round trips passed for these samples only:

| Archive-relative path under `The Sims 4/` | Original bytes | zstd -1 bytes | zstd -3 bytes |
|---|---:|---:|---:|
| `GP08/magalog.package` | 1,355,929 | 1,355,972 | 1,355,972 |
| `SP23/Strings_JPN_JP.package` | 7,404 | 6,881 | 6,884 |
| `EP04/Worlds/Areas/EP04_Coastal_01.world` | 6,073,996 | 3,884,052 | 3,744,914 |

These samples justify allowing raw and compressed objects, not a corpus-wide savings or speed claim. Full payload verification, cross-platform validation, transport benchmarks, and comparisons with another verified game version remain outstanding.

## Policy and user flow

Policy schema 1 stores outside the game directory: `game_id`, `install` (identity, roots, variant), `revision`, `desired` (DLC decisions, deferred ids, language), `applied` (policy revision and release), and `preferences` (channel, future packs, schedule, network, bandwidth). Trusted metadata versions stay in the TUF client store. Keep desired selections separate from the last successfully applied selections. Version and durably replace policy records, retaining a last valid copy. Keep the disposable hash cache and recovery journal separately; corrupt state must not reset choices to defaults.

On first launch, detect the installation and ask the user to confirm selections proposed from validated installed DLC. Save enabled and disabled decisions; an absent decision means unanswered. Later launches maintain those selections without repeating setup. New packs appear together until answered or deferred. Deferral leaves a notice in Settings without prompting on every launch. An optional “include future packs automatically” preference authorizes their downloads in advance. Explain the additional storage and bandwidth use when enabling it.

Settings permits selection changes. Disabled DLC is neither downloaded nor enabled by the unlock component. If disabling requires removal of existing files, ask once for that removal; otherwise retain the files. Never modify saves, mods, or unrelated files.

Every launch checks for compatible releases and repairs detected drift. Optional maintenance at login and while idle uses the same Rust engine through the OS scheduler. Respect pause, metered-network preferences, and bandwidth limits. Defer writes while the game or its installer is running. Retry temporary network and file-lock failures within a bounded budget; preserve progress for the next run when exhausted. Notify for required choices and persistent failures, not each retry. Reboot recovery occurs on the next launch or enabled scheduled run.

## Interface

One persistent main screen carries setup, maintenance, and Ready. Keep the game identity, status, and primary action in stable positions. Use one dominant action for the current state; expose Pause/Resume beside active progress. DLC selection, Settings, and Details open within the same window and return to the same context. Avoid separate update, repair, entitlement, and account screens.

Use a warm neutral background, charcoal text, teal primary actions, system sans-serif type, an 8-point spacing scale, and one outline icon family. Reserve warning and error colours for actual conditions. The header holds game identity and Settings; the centre holds status and progress; the bottom holds the primary action and DLC summary. Details expands below progress. Use a single-column layout at a minimum 720 × 560 logical pixels, scrolling at larger text sizes. DLC selection opens a searchable, scrollable sheet with installed/selected states and download size. Share these design tokens across Mac, Linux, and the website; adjust colours if contrast checks fail.

Make interaction satisfying through immediate feedback, stable layouts, and short transitions that explain state changes. Keep real progress visible during maintenance; never animate fictitious progress or delay completion for an effect. Ready settles into a quiet state with Play as the primary action. Respect reduced motion, text scaling, keyboard navigation, visible focus, and readable contrast; status must remain understandable without colour.

Review the rendered screen in setup, downloading, paused, error, offline, and Ready states on both platforms. Exercise long pack names, translated text, large text, and the minimum supported window size. Controls must remain reachable without clipping or covering progress. Visual approval requires inspecting the running interface, not just this document or screenshots of its ideal state.

## User controls and online access

The main screen names the detected game and location, current action, reason for that action, download size/progress, and whether play is available. Use plain states: Checking, Downloading, Applying, Waiting for game to close, Paused, Needs attention, and Ready. Show the next automatic action. Keep game readiness separate from online sign-in status; an expired online session must not imply damaged DLC or block offline maintenance.

Provide Pause/Resume, DLC and language selections, Verify/Repair, update timing, bandwidth limits, and online Connect/Disconnect. Explain that pausing during application can leave play unavailable until completion; Cancel does not roll back committed files. Settings holds advanced controls and a manual location override. Details explains changed files and failures without requiring the user to interpret logs. Automatic discovery remains the default. Do not expose codec, hash, token, or retry configuration in normal use.

Treat online access as a separate credential lifecycle owned by Rust. The target experience is Connect once, maintain the session when supported, and ask to sign in again only when required. Use a supported provider sign-in flow in the system browser where available. Sign-in stays in that browser. Pasting session material or editing configuration is not the normal path. Provider consent, MFA, and revoked sessions may require user action; do not promise unattended authentication around them.

Store credentials in the OS credential store, separately from game policy, objects, and manifests. Never send them to our K3s service or place them in logs, URLs, crash reports, or support exports. Renew automatically only through a verified supported mechanism, with one refresh operation in flight. Distinguish unavailable network/service from expired or revoked authorization; bound retries and preserve offline use. Disconnect cancels pending authentication and clears local credentials; late responses must not restore the session; revoke remotely only when the provider supports it. An unavailable credential store must not silently fall back to plaintext.

The local reconstruction `ast-map/ast_mac.json` names `authenticate`, `_authenticate`, `get_auth_code`, and `get_auth` in the online libraries, and `load_token` in the companion libraries. These symbols are investigation targets until a renewal protocol is shown. Before implementing online automation, establish the credential issuer, intended audience, acquisition, expiry, renewal behaviour, storage, and the game's handoff on each platform. Validate with a user-authorized account through the provider's own sign-in. This program uses its own credentials and its own hosts. When renewal is unavailable and sign-in and handoff work, offer explicit sign-in. When handoff is unverified, mark online access Unsupported and leave sign-in explicit.

## Discovery and compatibility

Validate a saved installation first. Otherwise probe Steam libraries under native, Flatpak, and Snap roots on Linux, Steam roots on Mac, and Steam libraries and EA app installations on Windows. Mac candidates also include bundles with id `com.ea.mac.thesims4` in `/Applications` and `~/Applications` and their associated packs directories. Verify markers, paths, and platform identity before accepting a candidate. Test probe fixtures before claiming a layout is supported.

Use a picker only when probes miss; show the locations checked. Multiple valid installations require a choice. Confirm a replacement when the saved installation is gone. Save confirmed paths so normal launches require neither picker nor game selector.

Linux Windows-format, native Windows, and native Mac installations are distinct variants. Validate base versions, DLC dependencies, language mapping, and unlock behaviour on each. This ZIP establishes no Mac compatibility. Share objects only where verified bytes match. An external game update invalidates the old inventory; an unknown or newer base version must not be overwritten as though it were ordinary file corruption.

## Release data

Read archives without executing their contents. Classify every entry as included, excluded, or unresolved. Included files require destination, platform, DLC/language ownership, and shared dependencies. Resolve `Delta/` and base-game dependencies before declaring DLC payloads complete. Exclude installers that are not part of the game, separate entitlement tools, service credentials, and unrelated extras. Preparation results are data, not another product spec.

Stream files into staging objects without extracting a second game tree. Check ZIP CRCs, compute BLAKE3, enforce declared sizes, and reject unsafe paths, links, and destination collisions. Hashes identify bytes; record source provenance and compatibility evidence separately. Unresolved entries block release promotion.

Use UTF-8 JSON schema version 1 for manifests. The publisher and Rust client share schema definitions and valid/invalid fixtures. Reject unsupported schema versions, duplicate keys, invalid references, conflicting destinations, and unknown required capabilities before writing files. TUF authenticates the exact manifest bytes. BLAKE3 values are lowercase 64-character hex; sizes are nonnegative byte counts. Destination paths use `/`, are relative to a named adapter root, and cannot contain empty, `.` or `..` segments.

| Record | Required fields |
|---|---|
| Release | `schema`, `game_id`, `variant`, `release_id`, `game_version`, `required_capabilities`, `compatible_bases`, `groups`, `files`, `deletions` |
| Group | `id`, `display_name`, `kind` (base/DLC/language/unlock), `requires` (group ids), `file_ids` |
| File | `id`, `root_id`, `path`, `target_hash`, `target_size`, `attributes`, `full`, `deltas` |
| Object | `hash` of served bytes, `encoded_size`, `codec` (raw/zstd), `decoded_hash`, `decoded_size` |
| Delta | `base_hash`, `base_size`, `format`, `object` |
| Deletion | `root_id`, `path`, `expected_hash` |

`compatible_bases` contains named fingerprints, each listing critical `root_id`, `path`, `hash`, and `size` tuples. The adapter validates their structure and platform identity. New verified fingerprints travel with the feed, so an ordinary game update need not require a new patcher. Include the target fingerprint for repair; version labels alone never authorize writes. `attributes` contains only adapter-supported values. Optional delta lists start empty. Each shared file has one target and remains required while any selected group references it. Reject dependency cycles, missing dependencies, and case/Unicode collisions on the destination filesystem. Deletions cannot overlap required targets.

Application manifests use `schema`, `app_version`, `os`, `arch`, `required_capabilities`, `artifact` (an Object), and `state_schema_range`; the OS adapter defines activation. Serve objects at `objects/<hash>` under the configured origin. Fix encoder settings; never replace bytes at an existing object URL. The initial publisher tries zstd level 3 and retains it only when smaller than raw; this is a starting policy, not an optimum claim. Verify every encoded object by decoding it. Future deltas must reconstruct the exact target, beat its encoded full object, and meet resource limits. Retain full fallbacks; no delta chains.

Use [The Update Framework (TUF)](https://theupdateframework.github.io/specification/latest/) for signed metadata, key rotation, expiry, and rollback protection. Keep root keys offline and online signing keys scoped. Content rollback requires a new authorized release; clients must not accept older metadata.

## Publishing and hosting

Serve immutable objects and release metadata from a K3s download service with read-only persistent storage. Keep the website service separate. Do not put DLCs in container images or proxy downloads through website code. No application API server is required. A CDN is optional.

Use HTTPS ingress, automatic certificate renewal, pinned container images, health probes, resource limits, and declarative deployment configuration. Inspect the actual cluster before configuring ingress; [K3s networking defaults](https://docs.k3s.io/networking/networking-services) are not evidence of its installed configuration. Publishing and retention jobs receive scoped write access; public services receive no signing keys.

Cache immutable artifacts long-term; revalidate mutable metadata within its freshness limits. Preserve encoded bytes, validators, and Range responses through ingress/CDN. Measure public-endpoint throughput, resume behaviour, disk use, and request load before sizing capacity. A single node has no node redundancy. Keep off-node backups of objects, metadata, and deployment configuration; test restoration and key recovery.

After a source and channel are configured, intake runs validation, classification, hashing, encoding/deltas, reconstruction tests, staging, signing, and public-endpoint checks against staged artifacts. Repeated inputs reuse verified outputs. Serialize promotion: upload and verify objects and release metadata, then publish the trusted snapshot and timestamp. Failed checks leave the current release intact. Generate website release data from the promoted catalog.

Application and content releases have separate versions and explicit compatibility constraints. Once a build is ordered, build and test each advertised OS/architecture on suitable runners, including platform signing/notarization where required. K3s hosting does not provide Mac build capability. Promote only when the configured channel's checks pass. Source acquisition requires a configured permitted source; the supplied ZIP cannot provide future updates.

Renew metadata before expiry using scoped keys. Retain supported releases and their objects for a declared resume window. Retention jobs delete only unreferenced objects after that window. If a pinned release is retired or withdrawn, reconcile actual client files against an authorized compatible replacement before continuing. Preserve user choices.

Monitor public downloads, metadata/certificate expiry, disk capacity, backups, and publishing failures. Bound automatic retries and alert on persistent failures. Record release inputs, check results, and promotion outcomes. Client telemetry is not required.

## Website

Provide a responsive layout, keyboard navigation, readable contrast, and a primary patcher download action. Offer and accept only platforms where The Sims 4 runs: Windows and Linux on x64, and macOS on Intel and Apple silicon. The website, install scripts, and engine accept only those platforms. Suggest the platform when detectable; ask a Mac's processor rather than guess it. Show supported platforms, installation steps, pack catalog, and release notes from the published catalog. Service status comes from health checks with a timestamp, not release metadata.

Normal use requires no account, manual DLC extraction, or a separate entitlement tool. The patcher accesses release endpoints directly and does not depend on website JavaScript. Check download links, layout, keyboard use, and load time before release; record the results.

## Engine contract and defaults

Rust exposes commands for Discover, ConfirmInstall, SetPolicy, Start, Pause, Resume, Cancel, Verify, Launch, Connect, and Disconnect. Commands carry an id; policy changes also carry the expected policy revision. Reject stale revisions. Rust returns accepted/rejected plus a structured error and publishes snapshots containing run id, state, phase, progress, active policy revision, readiness, online status, and available actions. Dart renders those actions; it does not infer permission to launch or retry.

The game adapter supplies probe/validate, version fingerprint, root mapping, busy check, language/unlock planning and postcondition checks, and launch. The OS adapter supplies locking, durable file operations, scheduling, and credential storage. Adapters return typed results, including Unsupported; neither adapter owns the transfer loop.

Store versioned JSON policy with a checksum and last valid copy under the OS application-data directory. Use a disposable cache database. The journal is append-only, with sequenced checksummed records for run binding, operation intent, commit, supersession, and completion. Fsync records before dependent mutations. Ignore a torn final record; interior corruption requires reconciliation. Operation ids are unique within a run. Schema migrations preserve the last recoverable state and fail closed on an unknown newer schema.

Default choices below are initial operating limits, not measurements of optimal performance:

| Setting | Initial value |
|---|---|
| Channel and consent | Stable; future packs and background maintenance off until enabled |
| Transfer/hash workers | One object, one apply worker; two hash workers |
| Transient retries | Three retries after the first attempt; 2/8/30-second backoff with jitter; respect longer server retry delays |
| Network timeout | 15-second connection timeout; 60 seconds without received bytes; no whole-file duration limit |
| Scheduled maintenance | When enabled, check at login and every 6 hours; defer while busy or on a disallowed network |
| Verification | Critical files each run; other cached hashes expire after 30 days and must be refreshed before reuse |
| Disk margin | Planned peak workspace plus the greater of 512 MiB or 5% of that workspace, per filesystem |
| Release retention | Keep current supported releases; retain superseded release objects for at least 90 days after retirement |
| Network preference | Unmetered allowed; metered requires opt-in; unknown allows foreground transfers but defers unattended transfers until permitted |

Pause preserves the run and suppresses automatic resume until the user resumes. Cancel stops the current run at a durable boundary and suppresses its scheduled restart until explicit Start; neither action rolls back committed files. Ordinary interruption resumes by reconciliation. State flow is checking → planned → applying → verifying → ready; a no-op plan skips applying. Pause, waiting, interruption, and failure preserve the run binding. Changes of installation, policy, or authorized release re-enter planning through a journaled transition. An invalid signature stops the run without automatic retry.

## Maintenance and recovery

A run binds the installation identity, verified manifest digest, desired-policy revision, and resolved file set. Persist that binding before mutation. States are checking, planned, applying, verifying, ready, or interrupted/blocked. Ready requires the applied policy and release to match the run, no pending operations, and successful final checks. A clean-release marker alone is insufficient.

1. **Establish ownership.** Hold an OS lock keyed to the installation identity across foreground and scheduled runs. An old lock file is not proof of a live owner. This excludes our writers only: check for the game and external installers before mutation and again before each commit. Never kill them. If a process or unexpected file change is detected, stop writes and invalidate the affected plan. Process checks cannot guarantee exclusion of an uncooperative writer; validate adapter coordination and document this limit.
2. **Plan.** Verify metadata, pin a compatible release, and resolve selected DLC, language, shared files, and unlock dependencies. Distinguish a recognized installation with damaged managed files from an unknown base version. Use adapter-defined version evidence; if it is inconclusive, do not force the feed over it. Check permissions and free space on every affected filesystem for downloads, outputs, codec scratch, journal, and a margin. Recheck before allocation. Resolve operations relative to validated directory handles; reject escaping links and recheck installation identity before committing.
3. **Inventory.** Cache installation identity, path, file identity where available, size, high-resolution mtime, hash, and last full verification. Metadata matches are a shortcut, not integrity proof. Hash adapter-defined compatibility-critical files every run. Rehash other files through a resumable verification sweep with a configured maximum age; explicit Verify checks all selected files immediately. Discard invalid cache records. For uncached files, an impossible target/base size takes the full object without hashing; otherwise hash with bounded workers. Check file identity and metadata before and after reads; a changing file is not a valid hash result.
4. **Transfer.** A matching target needs no transfer. An exact verified base may use a direct delta; other damaged or missing managed files use the full object. Begin with one object and one file application at a time. Adopt parallelism, ranges, chunking, or bundling only after measuring elapsed time, bytes, CPU, memory, disk, requests, and cost. Enforce the disk budget in every mode. Resume encoded bytes with Range and a stable validator; validate `206` offsets and length, restart on `200`, and check local size/hash on `416`. Verify the complete object before decoding. Stream decoding, cap zstd window memory at 64 MiB, and enforce the manifest output size; do not buffer a whole game file in memory. Follow [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html#section-14). Bound retries; repeated failure preserves progress and reports one next action.
5. **Prepare.** Revalidate the delta base and destination against the plan. Decode a full object, or reconstruct a delta from the validated base, into a unique temporary file in the destination directory. Enforce output limits, required attributes, and target BLAKE3. A delta reconstruction failure permits one full-object fallback; repeated integrity failure stops the run. Unexpected external changes require replanning, not a fallback that overwrites them.
6. **Commit.** Durably journal operation intent, expected prior state, target hash, and temporary path. Fsync the verified output, atomically rename it over the destination, fsync the directory, then durably record completion. Check every write, rename, and synchronization result. Apply deletions only when the path is no longer required and its current content matches the authorized old content; preserve unexplained changes. Journal deletions and synchronize their directories. Update the cache after commit and reclaim consumed downloads. Do not batch durability barriers until crash tests establish the ordering.
7. **Verify and finish.** Reconcile the entire required file set, critical hashes, deletions, and applied policy; hash changed or uncertain files again. Journal language and unlock operations too. Each adapter operation must be repeatable and expose a postcondition that recovery can inspect. Record the clean release and applied-policy revision durably only after all postconditions hold. Release the lock last.

File replacement and directory durability are separate operations; see Linux [rename](https://man7.org/linux/man-pages/man2/rename.2.html) and [fsync](https://man7.org/linux/man-pages/man2/fsync.2.html). Verify equivalent durability on each supported OS/filesystem. Unsupported guarantees or synchronization failures prevent a successful completion report.

On restart, verify journal integrity, installation identity, and release authorization. Reconcile pending operations with disk: a verified target completes the operation, a verified temporary output may be committed, and a missing or invalid output is rebuilt. Repeating this process must converge without repeated downloads of completed files or duplicate side effects. Never infer completion solely from the journal. A corrupt cache is rebuilt; a corrupt journal requires a full managed-file inventory against a trusted manifest before further writes. Recover policy from its valid copy; if both copies fail, ask for choices rather than guessing.

Selection changes during a run are saved as a new desired revision. At a durable boundary, supersede the old plan through a journaled transition and resolve from actual files; never change its file set in place or complete an obsolete selection silently. Feed changes do not alter a pinned run unless it is withdrawn or unavailable under retention policy. A replacement must account for every managed file left by the interrupted run.

Per-file commits can leave a mixed release after cancellation or power loss. Mark it incomplete and disable launch within the patcher until reconciled. The patcher cannot prevent an external launcher from starting it. Recovery completes forward; it may require network access, and rollback copies require additional disk. Offline, keep a previously clean installation usable and report update availability as unknown; never label an interrupted installation Ready. Refresh expired authorization before resuming mutations. Cancel and pause retain committed progress and resumable downloads; partial objects still require full verification before use. They obey the restart controls above.

Unlock maintenance uses the mapped behaviour and platform tests. Verify component version, installed files, and effective DLC configuration after updates and external game changes. Report enabled, disabled, unsupported, or failed. Missing permissions or repeated replacement by another process must not trigger an endless reinstall loop.

## Distribution and patcher updates

Install with one command and no administrator rights, the same form Claude Code uses: `curl -fsSL https://<domain>/install.sh | bash` on Linux and Mac, and `irm https://<domain>/install.ps1 | iex` in Windows PowerShell. Windows also gets a per-user graphical installer that produces the same layout. The script only detects OS/architecture, downloads the engine for that platform, checks its SHA-256 against the published bootstrap manifest, and runs `install`. The engine carries the trusted root, verifies the application release through TUF, and does all installation work. The publisher generates both scripts with the configured origin; never pipe an unpinned third-party script.

Install per user under the OS application-data directory: `versions/<app_version>/` holds each unpacked release, a `current` pointer names the active one, and a stable launcher outside `versions/` starts it. Add a menu entry (Linux `.desktop`, Windows Start menu, Mac `~/Applications`) and the command on the user's path. Uninstall removes the program, launcher, and menu entries and asks before removing policy, cache, and journal.


Rust exposes phase, bytes done, bytes total, bytes per second, outcome, and error as one coherent snapshot. Distinguish transfer bytes from verification/application work. Dart reads on a frame tick into one notifier; only dependent widgets rebuild. Errors provide one sentence and one next action. Details opens a credential-free local log.

Stage signed patcher updates into a new `versions/` directory and activate on restart: the launcher moves the `current` pointer, starts the new version, and moves it back if the startup check does not succeed, without retrying that version. Keep the prior version until a startup check succeeds; recover if activation fails. Activate only when no game mutation is in progress. State migrations must preserve recovery of policy and journals; the previous patcher must still read its retained state if activation fails. Update the patcher before the game only when manifest/client compatibility requires it. Include self-update resources in the disk and transfer budgets.

## 1. The order is the scope

Do the task as written, in the order written.

- Gathering what a later build needs is gathering. It is not the build.
- Mapping a target produces the map. It does not produce a merged map, a unified runtime, or a replacement program.
- A question is a question. It does not create a deadline, a backlog, or a tracking item. A date exists only after the user sets one.
- If the order is ambiguous, ask one precise question and wait.

## 2. Claims require evidence

A number, a "done", or a verdict appears only after it has been checked against the thing it describes.

- Count by matching each item to the source (address, line, or module name). A node count from a parser is not a function count.
- "Full" means every item is listed. Name every gap.
- Decompiler output, parser output, and generated code are reconstructions. The file itself says that, and it names the known defects. Chat is not the record.
- If a check fails, report the failure and the missing pieces. Withdraw the earlier claim in the same message.
- A speed or size claim for this program cites a measurement on the files it moves.

## 3. Source of truth first

- For map completeness, the source binary and the map are the check.
- For what the program is and how it runs, this file is the check.
- For what to do next, the user's latest order is the check.
- Return only what that decision needs.
- Before changing or deleting anything, name the exact target. A destructive or hard-to-reverse action waits for an explicit yes. A delegated brief names that same target.

## 4. Stop means stop

"Stop", "Done", and "Delete it" halt the work, including helpers and background jobs.

- Stop first. Then say what was stopped and what was left untouched.
- A barrier the user has named ends that line of work.
- If a stop fails, say so. Do not use output produced after the stop.

## 5. Clean room

These rules cover the Sims 4 work and every reimplementation in this repo.

- Use observed behaviour and AST maps as evidence, not as architectural authority. Choose methods independently under this document. The new program is written from this document. Reconstructions stay in `ast-map/` as evidence.
- This program has its own feed, its own keys, and its own hosting. Original service credentials stay out of the program, the repo, and the logs.
- Game, DLC, and update bytes come from a configured permitted source. Other distribution hosts stay out of download and test work.
- Cite a third-party project under its own name.

## 6. Done means checked

- Files exist at the path you named, and their contents match the order.
- Maps contain every module and function from the source binary, each count matched, every gap named.
- The build you report is a build you ran.
- A changed UI flow was exercised.
- Before production promotion, test first-run setup on an existing game, no-op launch, selected-content update, repair, external game update, policy change, and self-update on every advertised platform.
- Inject interruption before and after every journal/rename/fsync boundary, disk and synchronization failures, corrupt policy/cache/journal, same-size same-mtime corruption, game/installer races, selection changes, shared-DLC removal, withdrawn releases, offline/expired metadata, corrupt objects, and failed application activation. Run recovery repeatedly: it must converge, preserve choices and unrelated files, and never falsely report Ready. Process-kill tests alone do not establish power-loss durability.
- Exercise automatic discovery and every user control, including pause during application, deferred maintenance, online expiry, network loss during renewal, revoked authorization, failed credential storage, and disconnect during sign-in. No credential may appear in diagnostic output.
- Verify public HTTPS downloads and Range resume, metadata publication, website download links, and backup restoration. Record observed completion time, resource use, and user interventions; do not call automation or hosting production-ready from configuration alone.

## 7. Delegation

A worker brief includes the goal, the allowed work, the work that is out of scope, the exact target, and the clean-room rules. Read the result before you show it. If it breaks the brief, fix it first.

## 8. How to write

Plain language, short, the useful fact first, then the evidence. No flattery, no padding, no closing pitch.

## 9. Layout

`ast-map/` is the Sims 4 map work: AST maps, decompiled Python, and Ghidra output. `AST_MAP.md` records how those maps were produced and what was verified. Counts live in the map files.

Program code, when a build is ordered, goes in its own directory. Rust is the core. Flutter is the UI, one `lib/` for Linux, Mac, and Windows.
