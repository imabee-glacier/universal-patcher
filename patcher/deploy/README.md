# Download service and website on appsynergy-server

One hostname serves the website, the one-line installers (`install.sh`, `install.ps1`), patcher
releases under `app/`, and game releases at the root. Caddy terminates TLS on the front's own
public /32 and serves static files only: no application server, nothing that accepts writes from
the internet, and no signing keys in the public Pod.

## What is here

| File | Purpose |
|---|---|
| `front/Caddyfile` | TLS 1.2 and 1.3 (forward-secret AEAD suites only, post-quantum X25519MLKEM768 key exchange), HTTP/2 and HTTP/3, Let's Encrypt only, HSTS, a strict Content Security Policy with Trusted Types for the website, GET/HEAD only. Release bytes are served uncompressed with strong ETags and Range; immutable paths are cached for a year, files a promotion replaces are revalidated. Anything not matched is the website or 404. |
| `front/Dockerfile` | Caddy 2.11.9 pinned by digest, with the website built in, running as uid 1000. |
| `Dockerfile` | Publisher CLI for the jobs (`patcher-cli`). |
| `k3s/namespace.yaml` | `universal-patcher`, Pod Security `restricted`. |
| `k3s/storage.yaml` | Local volumes: `patcher-data` (everything published and everything waiting to be; 300 GiB estimate) and `patcher-caddy` (certificate state). Layout below. |
| `k3s/site.yaml` | The deployment's two identifying values: the public address (Multus attachment on `br-pub`) and the domain. Placeholders until chosen. |
| `k3s/front.yaml` | The front Deployment. |
| `k3s/jobs.yaml` | `renew-metadata` (6-hourly, the only holder of signing keys), `check-expiry` (hourly, fails 12 h before any role expires), `check-health` (every 5 minutes, writes the status the website shows), `backup-repository` (daily rclone copy of everything published). |
| `k3s/publish-app.yaml` | On-demand Job that publishes the patcher release waiting in staging. |
| `../tools/deploy-preflight.sh` | Refuses to deploy while placeholders remain, DNS or CAA disagree, the firewall declaration is missing, or an image is not pinned. |
| `../tools/front-check.sh` | Checks a running front from outside: protocols, TLS versions, headers, caching, Range resume, refusals. |
| `../tools/site-browser-check.sh` | Loads the website in Chromium and fails on any CSP violation or script error. |

## Data layout

Everything lives on one volume so publishing moves files into place within one filesystem.
The front mounts only `repository/`, read-only; nothing under `staging/` is ever served.

```
/var/lib/appsynergy/volumes/patcher-data/_data/      PV patcher-data, uid 65532
  repository/                    https://play.imabee.com/
    objects/                     game release objects: every file of the game and its packs,
                                 content-addressed by BLAKE3 (objects/<hash>)
    metadata/  targets/          signed TUF metadata and release manifests for game releases
    app/                         patcher releases (same layout, plus bootstrap/ and setup/)
    site/data/                   catalog.json (written by publishing), health.json (health job)
    install.sh  install.ps1      one-line installers (written by publishing)
  staging/
    app/                         a packaged patcher release waiting for the publish-app Job
    game/                        a prepared game release waiting to be signed and promoted
/var/lib/appsynergy/volumes/patcher-caddy/_data/     PV patcher-caddy, uid 1000, certificate
```

A game release, base game and packs alike, reaches `repository/` only through the publisher:
objects are verified, the manifest is signed, and `metadata/timestamp.json` moves last. Files
copied into `repository/` by hand are not part of any release and no client will use them.
The game repository is empty: no game release has been published.

## First deployment

Values to settle first: the **domain** and a free **address** in `64.112.14.0/25` (the block OVH
routes to this host; never `.128` and above).

1. **Reserve the address** in the control plane before any DNS record exists
   (`POST /platform/public-ips/{id}/reserve-infrastructure`). An unreserved address with a live
   name is one a customer could claim and get a certificate for.
2. **Declare it** in appsynergy-rs `ops/netpolicy/exposure.toml` as a new `[[stack]]` with
   `names = ["<domain>"]` and exactly `80/tcp` (ACME HTTP-01 and redirect), `443/tcp` and
   `443/udp` (HTTP/3). Re-render with `python3 ops/netpolicy/render.py`, merge, and reload
   `appsynergy-netguard.service` on the host.
3. **DNS**: an `A` record for the domain to the address, no `AAAA` (the front has no IPv6
   address), and `CAA 0 issue "letsencrypt.org"`.
4. **Fill in** `k3s/site.yaml` (address in `rangeStart` and `rangeEnd`, `PATCHER_DOMAIN`,
   `PATCHER_ORIGIN`) and pin both images by the digests CI printed:
   `kustomize edit set image patcher-front=ghcr.io/imabee-glacier/patcher-front@sha256:<digest>`
   (same for `patcher-cli`, and in `publish-app.yaml`).
5. `tools/deploy-preflight.sh <appsynergy-rs checkout>` must pass.
6. Create the patcher release keys on a workstation, never on the server:
   `patcher init-root <dir>` writes `root-key.pem` (signs only `root.json`; move it to offline
   storage), `online-key.pem` (the only key the cluster receives) and `root.json` (public; also
   baked into every build as `PATCHER_DIST_ROOT`). Game release keys are added the same way when
   a game release exists; until then the jobs skip the empty game repository.
7. On the host, create the volumes owned by the uid that writes them, the host route, and the
   Secret (the online key and public root only; nothing is echoed or tracked):

   ```bash
   D=/var/lib/appsynergy/volumes/patcher-data/_data
   install -d -o 65532 -g 65532 -m 0755 "$D" "$D/repository" "$D/repository/objects" \
       "$D/repository/metadata" "$D/repository/targets" "$D/repository/app" \
       "$D/repository/site" "$D/repository/site/data" "$D/staging" "$D/staging/app" "$D/staging/game"
   install -d -o 1000 -g 1000 -m 0700 /var/lib/appsynergy/volumes/patcher-caddy/_data
   ip route replace <address>/32 dev br-pub
   kubectl -n universal-patcher create secret generic release-signing \
       --from-file=app-root.json=root.json --from-file=app-online.pem=online-key.pem
   kubectl -n universal-patcher create secret generic release-backup \
       --from-file=rclone.conf --from-literal=destination=<remote:path>
   ```

   Make the route persistent the way `ops/network/30-br-pub.network` does for the other /32s.
8. `kubectl apply -k deploy/k3s`, then publish the first release (below).
9. Verify from off the host:

   ```bash
   kubectl -n universal-patcher exec deploy/front -- ip route get 1.1.1.1   # src <address>
   tools/front-check.sh https://<domain>/
   tools/site-browser-check.sh https://<domain>/
   ```

   and from the appsynergy-rs checkout, `ops/netpolicy/verify.py` (only the declared ports
   answer) and `ops/netpolicy/dnscheck.py`.

## Publishing a patcher release

Build and package each platform (`../distribution/README.md`), then upload staging and run the
Job:

```bash
rsync -a --delete staging/ root@144.217.66.212:/var/lib/appsynergy/volumes/patcher-data/_data/staging/app/
ssh root@144.217.66.212 chown -R 65532:65532 /var/lib/appsynergy/volumes/patcher-data/_data/staging/app
kubectl create -f deploy/k3s/publish-app.yaml
kubectl -n universal-patcher wait --for=condition=complete job -l app=publish-app --timeout=15m
```

`staging/` holds `packages/` (every `patcher package-app` output), `notes.txt`, `offer` (one
verified platform per line, for example `linux-x64`) and optionally the Windows installer.
`publish-app` refuses a version that is not newer, an offered platform with no build, and a
Windows installer without a Windows build, before it changes anything. The website's
`data/catalog.json` is written last.

## Verified locally

- `tools/e2e-install.sh`: a release build is published and served by the front image (read-only
  root, all capabilities dropped but NET_BIND_SERVICE) with a local test CA. `curl ... | bash`
  installs it over HTTPS into an empty home, a tampered engine is refused, the health check
  reports operational, every check in `front-check.sh` passes (HTTP/2, HTTP/3, TLS 1.3 and 1.2
  accepted, TLS 1.1 and non-forward-secret suites refused, X25519MLKEM768 negotiated, HSTS, CSP
  with Trusted Types, 206/200/416/304 behaviour, raw immutable objects, refusals), and Chromium
  loads the site with no CSP violation, shows the Linux command and release notes from the
  published catalog, offers nothing for an unverified platform, and does not scroll sideways at
  720 x 560.
- The Caddyfile adapts to a single Let's Encrypt issuer.
- `kubectl kustomize` renders; kubeconform (Kubernetes 1.33, the host's k3s v1.33.13; strict) accepts every resource
  except the Multus attachment, which has no published schema.
- The publish Job's argument assembly was run under the CLI image's shell.

## Not verified

- Nothing has been applied to appsynergy-server. The domain and address are not chosen.
- Let's Encrypt issuance and renewal against the real domain, HTTP/3 through the host's
  firewall, and the netguard rules for the new address.
- Public throughput, resume over the internet, disk use and request load.
- Backup restoration and key recovery; both must be exercised before relying on the backups.
- Kubernetes NetworkPolicy is not enforced on this kernel (see appsynergy-rs `ops/DEPLOY.md`),
  so in-cluster isolation rests on the host firewall and on the front serving files only.
- HSTS omits `includeSubDomains` and `preload` until it is known that every subdomain of the
  chosen domain serves HTTPS.
- Encrypted Client Hello is not enabled; it needs DNS `HTTPS` records managed by Caddy.
- Game releases: the repository root stays empty until a game release passes the platform
  checks in `AGENTS.md`. No game content is published.
