//! Catalog entitlement records for the selected installation.
//!
//! Behavioral evidence is in the Mac map: parse_QueryEntitlements at 0005f380,
//! get_filters at 00064210, and skip_entitlement at 00064c10. The reconstruction
//! has parser gaps. Native handoff is still unverified, so this module reports
//! catalog state and does not claim that play access is ready.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entitlement {
    pub pack_id: String,
    pub offer_id: String,
    pub group: String,
    pub tag: String,
    pub kind: String,
    pub version: String,
    /// Stable identity supplied by the catalog, independent of selection order.
    pub id: u64,
}

#[derive(Default, Debug)]
pub struct Filters {
    pub offers: BTreeSet<String>,
    pub groups: BTreeSet<String>,
    pub items: BTreeSet<String>,
}

impl Filters {
    /// Parse a single query, not an LSX transport envelope. Unknown extensions fail
    /// closed. No DTD, entity resolver, namespaces, or nested filters are accepted.
    pub fn parse(xml: &str) -> Result<Self, &'static str> {
        if xml.len() > 64 * 1024 || xml.contains("<!DOCTYPE") {
            return Err("Entitlement request exceeds limits or contains a DTD");
        }
        let doc = roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: 2048,
                ..Default::default()
            },
        )
        .map_err(|_| "Invalid entitlement request XML")?;
        let root = doc.root_element();
        if root.tag_name().name() != "QueryEntitlements" || root.namespaces().len() != 0 {
            return Err("Unsupported entitlement request");
        }
        let mut filters = Self::default();
        for attr in root.attributes() {
            if attr.namespace().is_some() {
                return Err("Unsupported request namespace");
            }
            let target = match attr.name() {
                "OfferId" | "ItemId" => Some(&mut filters.offers),
                "Group" => Some(&mut filters.groups),
                // These fields do not affect the original local catalog filtering.
                "UserId" | "includeChildGroups" | "includeExpiredTrialDLC" | "version" => None,
                _ => return Err("Unsupported entitlement request attribute"),
            };
            if attr.value().len() > 4096 {
                return Err("Entitlement filter exceeds limit");
            }
            if let Some(target) = target
                && !attr.value().is_empty()
            {
                target.insert(attr.value().to_owned());
            }
        }
        for child in root.children() {
            if child.is_text() && child.text().unwrap_or("").trim().is_empty() {
                continue;
            }
            if !child.is_element()
                || child.namespaces().len() != 0
                || child.attributes().len() != 0
                || child.children().any(|c| !c.is_text())
            {
                return Err("Unsupported entitlement filter structure");
            }
            let target = match child.tag_name().name() {
                "FilterOffers" => &mut filters.offers,
                "FilterGroups" => &mut filters.groups,
                "FilterItems" => &mut filters.items,
                _ => return Err("Unsupported entitlement filter"),
            };
            let value = child.text().unwrap_or("");
            if value.len() > 4096 {
                return Err("Entitlement filter exceeds limit");
            }
            if !value.is_empty() {
                target.insert(value.to_owned());
            }
        }
        Ok(filters)
    }

    pub fn matches(&self, entry: &Entitlement) -> bool {
        (self.offers.is_empty() && self.groups.is_empty() && self.items.is_empty())
            || self.offers.contains(&entry.offer_id)
            || self.groups.contains(&entry.group)
            || self.items.contains(&entry.tag)
    }
}

/// Local fixture catalog schema; not part of the release manifest contract and
/// never accepted as trusted release authorization merely because it parses.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: u32,
    pub game_id: String,
    pub entries: Vec<Entitlement>,
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Entitlement catalog exceeds limit");
        }
        let value =
            crate::strict_json::parse(bytes).map_err(|_| "Invalid entitlement catalog JSON")?;
        let catalog: Self =
            serde_json::from_value(value).map_err(|_| "Invalid entitlement catalog schema")?;
        if catalog.schema != 1 || catalog.game_id != "sims4" {
            return Err("Unsupported entitlement catalog");
        }
        response(&catalog.entries, &BTreeSet::new(), &Filters::default())?;
        Ok(catalog)
    }
}

/// Immutable query session bound to the same policy revision as its caller.
/// It performs no installation, authentication, or network communication.
pub struct Session {
    revision: u64,
    entries: Vec<Entitlement>,
    selected: BTreeSet<String>,
}

/// The adapter must supply the current run binding for every native request.
/// Matching a revision alone cannot distinguish installations or releases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub installation_identity: String,
    pub manifest_digest: String,
    pub policy_revision: u64,
}

pub struct BoundSession {
    binding: Binding,
    session: Session,
    active: bool,
}

impl BoundSession {
    pub fn bind(
        catalog: Catalog,
        selection: &crate::model::Selection,
        binding: Binding,
    ) -> Result<Self, &'static str> {
        if binding.installation_identity.is_empty() || binding.installation_identity.len() > 4096 {
            return Err("Invalid entitlement installation identity");
        }
        crate::model::valid_hash(&binding.manifest_digest)
            .map_err(|_| "Invalid entitlement manifest digest")?;
        let session = Session::bind(catalog, selection, binding.policy_revision)?;
        Ok(Self {
            binding,
            session,
            active: true,
        })
    }

    pub fn query(&self, current: &Binding, xml: &str) -> Result<String, &'static str> {
        if !self.active || current != &self.binding {
            return Err("Entitlement session is no longer active for this run");
        }
        self.session.query(current.policy_revision, xml)
    }

    /// Called at unapply or run supersession before accepting further requests.
    /// Reactivation requires a newly validated session, not toggling this flag.
    pub fn invalidate(&mut self) {
        self.active = false;
    }
}

impl Session {
    pub fn bind(
        catalog: Catalog,
        selection: &crate::model::Selection,
        revision: u64,
    ) -> Result<Self, &'static str> {
        if catalog.schema != 1 || catalog.game_id != "sims4" {
            return Err("Unsupported entitlement catalog");
        }
        response(&catalog.entries, &BTreeSet::new(), &Filters::default())?;
        let selected: BTreeSet<String> = selection
            .dlc
            .iter()
            .filter(|(_, enabled)| **enabled)
            .map(|(id, _)| id.clone())
            .collect();
        for pack in &selected {
            if !catalog.entries.iter().any(|e| &e.pack_id == pack) {
                return Err("Selected pack has no entitlement mapping");
            }
        }
        Ok(Self {
            revision,
            entries: catalog.entries,
            selected,
        })
    }

    pub fn query(&self, current_revision: u64, xml: &str) -> Result<String, &'static str> {
        if current_revision != self.revision {
            return Err("Entitlement policy revision changed");
        }
        response(&self.entries, &self.selected, &Filters::parse(xml)?)
    }
}

/// The caller supplies explicitly enabled pack ids from the bound desired policy.
/// A broad game query cannot re-enable a disabled or unanswered pack.
/// Catalog entries are data, never proof that an EA account owns a product.
pub fn response(
    catalog: &[Entitlement],
    selected: &BTreeSet<String>,
    filters: &Filters,
) -> Result<String, &'static str> {
    if catalog.len() > 4096 {
        return Err("Entitlement catalog exceeds limit");
    }
    let mut ids = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut field_bytes = 0usize;
    for entry in catalog {
        if !identities.insert((&entry.offer_id, &entry.group, &entry.tag)) {
            return Err("Duplicate entitlement mapping");
        }
        if !ids.insert(entry.id) {
            return Err("Duplicate entitlement identity");
        }
        for value in [
            &entry.pack_id,
            &entry.offer_id,
            &entry.group,
            &entry.tag,
            &entry.kind,
            &entry.version,
        ] {
            field_bytes = field_bytes.saturating_add(value.len());
            if field_bytes > 1024 * 1024 {
                return Err("Entitlement catalog field data exceeds limit");
            }
            if value.is_empty()
                || value.len() > 4096
                || value.chars().any(|c| {
                    !matches!(c, '\u{9}' | '\u{a}' | '\u{d}' | '\u{20}'..='\u{d7ff}'
                    | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
                })
            {
                return Err("Invalid entitlement catalog field");
            }
        }
    }
    let mut xml = String::from("<QueryEntitlementsResponse>");
    for entry in catalog
        .iter()
        .filter(|e| selected.contains(&e.pack_id) && filters.matches(e))
    {
        xml.push_str("<Entitlement");
        let id = entry.id.to_string();
        for (key, value) in [
            ("ItemId", entry.offer_id.as_str()),
            ("Type", entry.kind.as_str()),
            ("EntitlementId", id.as_str()),
            ("EntitlementTag", entry.tag.as_str()),
            ("Group", entry.group.as_str()),
            ("ResourceId", ""),
            ("UseCount", "0"),
            ("Expiration", "0000-00-00T00:00:00"),
            ("GrantDate", "0000-00-00T00:00:00"),
            ("LastModifiedDate", "0000-00-00T00:00:00"),
            ("Version", entry.version.as_str()),
        ] {
            xml.push(' ');
            xml.push_str(key);
            xml.push_str("=\"");
            for c in value.chars() {
                match c {
                    '&' => xml.push_str("&amp;"),
                    '<' => xml.push_str("&lt;"),
                    '>' => xml.push_str("&gt;"),
                    '"' => xml.push_str("&quot;"),
                    '\t' => xml.push_str("&#9;"),
                    '\n' => xml.push_str("&#10;"),
                    '\r' => xml.push_str("&#13;"),
                    _ => xml.push(c),
                }
            }
            xml.push('"');
        }
        xml.push_str("/>");
    }
    xml.push_str("</QueryEntitlementsResponse>");
    Ok(xml)
}

const SHIPPED_CATALOG: &[u8] = include_bytes!("entitlements.json");
const PROOF_NAME: &str = "unlock-postcondition.json";
const HOOK_NAME: &str = "unlock-hook.json";

/// Copies the shipped `version.dll` into `destination_dir`. The bytes must already
/// be a component binary. A locked destination is left in place and reported.
pub fn place_component(bundled: &Path, destination_dir: &Path) -> Result<(), &'static str> {
    let bytes = fs::read(bundled).map_err(|_| "component-missing")?;
    if bytes.len() < 64 || bytes.get(..2) != Some(b"MZ") {
        return Err("component-invalid");
    }
    let dest = destination_dir.join("version.dll");
    if fs::read(&dest).ok().as_deref() == Some(bytes.as_slice()) {
        return Ok(());
    }
    fs::copy(bundled, &dest).map_err(|_| "component-replace")?;
    Ok(())
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn ea_desktop_dir() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("ProgramFiles") {
        roots.push(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(dir));
    }
    for root in roots {
        let base = root.join("Electronic Arts").join("EA Desktop");
        let direct = base.join("EA Desktop");
        if direct.join("EADesktop.exe").is_file() {
            return Some(direct);
        }
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let nested = entry.path().join("EA Desktop");
            if nested.join("EADesktop.exe").is_file() {
                return Some(nested);
            }
        }
    }
    None
}

/// Installs the component that ships beside this program into the EA Desktop
/// directory, so EA Desktop loads it on its next start.
#[cfg(all(windows, target_arch = "x86_64"))]
fn install_log(line: &str) {
    let Some(root) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let path = PathBuf::from(root)
        .join("UniversalPatcher")
        .join("component-install.log");
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    use std::io::Write;
    let _ = writeln!(file, "{line}");
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub fn install_windows_component() -> Result<PathBuf, &'static str> {
    let bundled = std::env::current_exe()
        .map_err(|_| "component-missing")?
        .parent()
        .ok_or("component-missing")?
        .join("version.dll");
    install_log(&format!("bundled {}", bundled.display()));
    let destination = match ea_desktop_dir() {
        Some(dir) => dir,
        None => {
            install_log("ea-desktop-missing");
            return Err("ea-desktop-missing");
        }
    };
    install_log(&format!("destination {}", destination.display()));
    let bytes = fs::read(&bundled).map_err(|_| "component-missing")?;
    if bytes.len() < 64 || bytes.get(..2) != Some(b"MZ") {
        install_log("component-invalid");
        return Err("component-invalid");
    }
    let installed = destination.join("version.dll");
    if fs::read(&installed).ok().as_deref() == Some(bytes.as_slice()) {
        install_log(&format!("already-installed {}", installed.display()));
        return Ok(installed);
    }
    let temporary = destination.join("version.dll.new");
    if let Err(error) = fs::write(&temporary, &bytes) {
        install_log(&format!("write {error}"));
        return Err("component-write");
    }
    if installed.exists()
        && let Err(error) = fs::remove_file(&installed)
    {
        install_log(&format!("remove {error}"));
        let _ = fs::remove_file(&temporary);
        return Err("component-replace");
    }
    if let Err(error) = fs::rename(&temporary, &installed) {
        install_log(&format!("rename {error}"));
        let _ = fs::remove_file(&temporary);
        return Err("component-replace");
    }
    install_log(&format!("installed {}", installed.display()));
    Ok(installed)
}

/// Pack offer and tag records shipped with the patcher. These are game catalog
/// identifiers. They are not an EA account and they do not authorize play.
pub fn shipped_catalog() -> Result<Catalog, &'static str> {
    Catalog::parse(SHIPPED_CATALOG)
}

/// Writes the shipped catalog beside the saved policy. An existing catalog is replaced
/// only by this explicit install, so a caller can supply a fixture catalog first.
pub fn install_catalog(dir: &Path) -> Result<(), &'static str> {
    let catalog = shipped_catalog()?;
    let bytes = serde_json::to_vec(&catalog).map_err(|_| "catalog-encode")?;
    crate::storage::atomic_write(&dir.join("entitlements.json"), &bytes)
        .map_err(|_| "catalog-write")
}

fn binding_from(policy: &crate::storage::Policy) -> Result<Binding, &'static str> {
    if policy.schema != 1 || policy.game_id != "sims4" {
        return Err("unsupported-policy");
    }
    let manifest_digest = match &policy.applied {
        Some(applied) if crate::model::valid_hash(&applied.manifest_hash).is_ok() => {
            applied.manifest_hash.clone()
        }
        _ => blake3::hash(policy.install.identity.as_bytes())
            .to_hex()
            .to_string(),
    };
    Ok(Binding {
        installation_identity: policy.install.identity.clone(),
        manifest_digest,
        policy_revision: policy.revision,
    })
}

fn read_limited(path: &Path) -> Result<Vec<u8>, &'static str> {
    let bytes = fs::read(path).map_err(|_| "state-unavailable")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("state-limit");
    }
    Ok(bytes)
}

/// The session the Windows component loads from the installation's saved policy.
/// The same functions serve the in-process hook and the tests.
pub struct Replacement {
    dir: PathBuf,
    policy_bytes: Vec<u8>,
    catalog_bytes: Vec<u8>,
    binding: Binding,
    transport: crate::entitlement_transport::Transport,
    enabled: bool,
    /// Original entitlement responses already rewritten. An identical one is left alone.
    seen_responses: BTreeSet<String>,
}

impl Replacement {
    pub fn open(dir: &Path) -> Result<Self, &'static str> {
        let policy_bytes = read_limited(&dir.join("policy.json"))?;
        let policy = crate::storage::Policy::load(dir).map_err(|_| "policy-unavailable")?;
        let catalog_bytes = read_limited(&dir.join("entitlements.json"))?;
        let catalog = Catalog::parse(&catalog_bytes)?;
        let binding = binding_from(&policy)?;
        let enabled = policy.desired.dlc.values().any(|on| *on);
        let session = BoundSession::bind(catalog, &policy.desired, binding.clone())?;
        Ok(Self {
            dir: dir.to_path_buf(),
            policy_bytes,
            catalog_bytes,
            binding,
            transport: crate::entitlement_transport::Transport::new(session),
            enabled,
            seen_responses: BTreeSet::new(),
        })
    }

    pub fn any_enabled(&self) -> bool {
        self.enabled
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    fn unchanged(&self) -> bool {
        read_limited(&self.dir.join("policy.json")).ok().as_deref()
            == Some(self.policy_bytes.as_slice())
            && read_limited(&self.dir.join("entitlements.json"))
                .ok()
                .as_deref()
                == Some(self.catalog_bytes.as_slice())
    }

    /// Rewrites one correlated entitlement response, or returns None when the
    /// original bytes must be left as they are. A changed policy file retires
    /// the session.
    pub fn observe(&mut self, xml: &str) -> Result<Option<String>, &'static str> {
        if !self.unchanged() {
            self.transport.invalidate();
            return Err("selection-changed");
        }
        let doc = roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: 8192,
                ..Default::default()
            },
        )
        .map_err(|_| "invalid-message")?;
        for node in doc.descendants().filter(|n| n.is_element()) {
            if node.tag_name().name() == "ChallengeResponse" {
                self.transport.disconnect(0);
                return Ok(None);
            }
            if node.tag_name().name() == "QueryEntitlements" {
                let filters = Filters::parse(&xml[node.range()])?;
                if filters.groups.len() != 1
                    || !filters.groups.contains("THESIMS4PC")
                    || !filters.offers.is_empty()
                    || !filters.items.is_empty()
                {
                    self.transport.disconnect(0);
                    return Ok(None);
                }
            }
        }
        let correlated = self
            .transport
            .observe(0, &self.binding, xml, Instant::now())?;
        // Identity is the original message. A second copy is left unchanged even
        // when correlation already consumed its request and the fill path would
        // otherwise build another replacement.
        let identity = blake3::hash(xml.as_bytes()).to_hex().to_string();
        if self.seen_responses.contains(&identity) {
            return Ok(None);
        }
        let rewritten = if let Some(rewritten) = correlated {
            rewritten
        } else if let Some(rewritten) = self.fill_sims_response(xml)? {
            rewritten
        } else {
            return Ok(None);
        };
        self.seen_responses.insert(identity);
        Ok(Some(rewritten))
    }

    /// A single `Response` / `QueryEntitlementsResponse` envelope whose entitlements
    /// are all in group `THESIMS4PC`. Anything else is left unchanged.
    fn fill_sims_response(&self, xml: &str) -> Result<Option<String>, &'static str> {
        let doc = roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: 8192,
                ..Default::default()
            },
        )
        .map_err(|_| "invalid-message")?;
        let root = doc.root_element();
        if root.tag_name().name() != "LSX" || root.namespaces().len() != 0 {
            return Ok(None);
        }
        let envelopes: Vec<_> = root.children().filter(|node| node.is_element()).collect();
        if envelopes.len() != 1 {
            return Ok(None);
        }
        let envelope = envelopes[0];
        if envelope.tag_name().name() != "Response" || envelope.namespaces().len() != 0 {
            return Ok(None);
        }
        let payloads: Vec<_> = envelope
            .children()
            .filter(|node| node.is_element())
            .collect();
        if payloads.len() != 1 {
            return Ok(None);
        }
        let payload = payloads[0];
        if payload.tag_name().name() != "QueryEntitlementsResponse"
            || payload.namespaces().len() != 0
        {
            return Ok(None);
        }
        let mut saw_group = false;
        for node in payload.descendants().filter(|node| node.is_element()) {
            if node.tag_name().name() != "Entitlement" {
                continue;
            }
            if node.attribute("Group") != Some("THESIMS4PC") {
                return Ok(None);
            }
            saw_group = true;
        }
        if !saw_group {
            return Ok(None);
        }
        let range = payload.range();
        let expected = self.expected_response()?;
        let mut changed = String::with_capacity(xml.len() + expected.len());
        changed.push_str(&xml[..range.start]);
        changed.push_str(&expected);
        changed.push_str(&xml[range.end..]);
        Ok(Some(changed))
    }

    pub fn expected_response(&self) -> Result<String, &'static str> {
        self.transport
            .response_for(&self.binding, "<QueryEntitlements Group='THESIMS4PC'/>")
    }
}

/// Records installed packs as enabled and mapped packs that are not installed as
/// disabled, once, when the confirmed installation still has no decisions.
/// Unmapped packs stay unanswered, so selecting one later fails closed.
pub fn record_installed_selection(
    dir: &Path,
    installed: &BTreeSet<String>,
) -> Result<bool, &'static str> {
    let mut policy = crate::storage::Policy::load(dir).map_err(|_| "policy-unavailable")?;
    if !policy.desired.dlc.is_empty() || !policy.desired.deferred.is_empty() {
        return Ok(false);
    }
    install_catalog(dir)?;
    let catalog = shipped_catalog()?;
    let mut ids = BTreeSet::new();
    for entry in &catalog.entries {
        ids.insert(entry.pack_id.clone());
    }
    for id in ids {
        policy
            .desired
            .dlc
            .insert(id.clone(), installed.contains(&id));
    }
    policy.revision = policy.revision.saturating_add(1);
    policy.save(dir).map_err(|_| "policy-save")?;
    Ok(true)
}

/// Arms unlock from the confirmed installation: installed mapped packs are enabled
/// and mapped packs that are not installed are disabled, when no decision exists yet.
pub fn arm_saved_installation(dir: &Path) -> Result<bool, &'static str> {
    let policy = crate::storage::Policy::load(dir).map_err(|_| "policy-unavailable")?;
    let Some(root) = policy.install.roots.get("game").cloned() else {
        return Ok(false);
    };
    let mut installed = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if crate::local::is_pack_id(&name) && entry.path().is_dir() {
                installed.insert(name);
            }
        }
    }
    record_installed_selection(dir, &installed)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookRecord {
    schema: u32,
    host: String,
    result: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnlockProof {
    schema: u32,
    installation_identity: String,
    manifest_digest: String,
    policy_revision: u64,
    rewrites: u64,
    response_blake3: String,
    host: String,
}

fn ea_host(host: &str) -> bool {
    host.to_ascii_lowercase().ends_with("\\eadesktop.exe")
}

/// Records whether the component loaded inside EA Desktop installed its hook.
pub fn record_hook(dir: &Path, host: &str, result: &str) -> Result<(), &'static str> {
    let record = HookRecord {
        schema: 1,
        host: host.to_owned(),
        result: result.to_owned(),
    };
    let bytes = crate::storage::checked_bytes(&record).map_err(|_| "hook-encode")?;
    crate::storage::atomic_write(&dir.join(HOOK_NAME), &bytes).map_err(|_| "hook-write")
}

pub fn clear_rewrite_proof(dir: &Path) {
    let _ = fs::remove_file(dir.join(PROOF_NAME));
}

fn offer_present(xml: &str, offer: &str) -> bool {
    xml.contains(&format!("ItemId=\"{offer}\""))
}

/// Called only after Qt-owned storage was read back and matched the replacement.
/// A mismatch is not recorded as success.
pub fn record_verified_rewrite(
    dir: &Path,
    read_back: &str,
    host: &str,
) -> Result<(), &'static str> {
    if !ea_host(host) {
        return Err("not-ea-desktop");
    }
    let replacement = Replacement::open(dir)?;
    let expected = replacement.expected_response()?;
    if !read_back.contains(&expected) {
        return Err("read-back-mismatch");
    }
    let policy = crate::storage::Policy::load(dir).map_err(|_| "policy-unavailable")?;
    let catalog = Catalog::parse(&read_limited(&dir.join("entitlements.json"))?)?;
    for entry in &catalog.entries {
        let enabled = policy.desired.dlc.get(&entry.pack_id) == Some(&true);
        let present = offer_present(read_back, &entry.offer_id);
        if enabled != present {
            return Err(if enabled {
                "selected-missing"
            } else {
                "disabled-present"
            });
        }
    }
    let binding = binding_from(&policy)?;
    let response_blake3 = blake3::hash(expected.as_bytes()).to_hex().to_string();
    let previous: Option<UnlockProof> = crate::storage::read_checked(&dir.join(PROOF_NAME)).ok();
    let rewrites = previous
        .filter(|proof| {
            proof.schema == 1
                && proof.host == host
                && proof.installation_identity == binding.installation_identity
                && proof.manifest_digest == binding.manifest_digest
                && proof.policy_revision == binding.policy_revision
                && proof.response_blake3 == response_blake3
        })
        .map(|proof| proof.rewrites.saturating_add(1))
        .unwrap_or(1);
    let proof = UnlockProof {
        schema: 1,
        installation_identity: binding.installation_identity,
        manifest_digest: binding.manifest_digest,
        policy_revision: binding.policy_revision,
        rewrites,
        response_blake3,
        host: host.to_owned(),
    };
    let bytes = crate::storage::checked_bytes(&proof).map_err(|_| "proof-encode")?;
    crate::storage::atomic_write(&dir.join(PROOF_NAME), &bytes).map_err(|_| "proof-write")
}

/// `enabled`, `disabled`, `unsupported`, or `failed`. Enabled requires a recorded
/// rewrite whose bytes match the saved selection. It does not mean the game
/// accepted that response or that an EA account owns the packs.
pub fn unlock_report(windows_x64: bool, dir: Option<&Path>) -> (&'static str, String) {
    if !windows_x64 {
        return (
            "unsupported",
            "Unlock is not available on this system.".into(),
        );
    }
    let Some(dir) = dir else {
        return (
            "disabled",
            "Unlock waits for a confirmed installation.".into(),
        );
    };
    let opened = Replacement::open(dir);
    let replacement = match opened {
        Ok(value) => value,
        Err("Selected pack has no entitlement mapping") => {
            return (
                "failed",
                "A selected pack has no entitlement mapping.".into(),
            );
        }
        Err(_) => {
            return (
                "disabled",
                "Unlock waits for a confirmed installation.".into(),
            );
        }
    };
    let expected = match replacement.expected_response() {
        Ok(value) => value,
        Err(_) => {
            return (
                "failed",
                "The entitlement response did not match the saved selection.".into(),
            );
        }
    };
    let hook: HookRecord = match crate::storage::read_checked(&dir.join(HOOK_NAME)) {
        Ok(value) => value,
        Err(_) => {
            return (
                "disabled",
                "EA Desktop has not loaded the entitlement component.".into(),
            );
        }
    };
    if !ea_host(&hook.host) {
        return (
            "failed",
            "The entitlement component did not run inside EA Desktop.".into(),
        );
    }
    if hook.result != "installed" {
        return (
            "failed",
            format!("The entitlement hook did not install ({}).", hook.result),
        );
    }
    let proof: UnlockProof = match crate::storage::read_checked(&dir.join(PROOF_NAME)) {
        Ok(value) => value,
        Err(_) => {
            return (
                "disabled",
                "Unlock has not rewritten an entitlement response for this selection.".into(),
            );
        }
    };
    if proof.schema != 1
        || proof.host != hook.host
        || proof.installation_identity != replacement.binding.installation_identity
        || proof.manifest_digest != replacement.binding.manifest_digest
        || proof.policy_revision != replacement.binding.policy_revision
    {
        return (
            "disabled",
            "The selection changed. The previous entitlement response is no longer used.".into(),
        );
    }
    if proof.rewrites == 0 {
        return (
            "failed",
            "The entitlement response was not rewritten.".into(),
        );
    }
    if proof.response_blake3 != blake3::hash(expected.as_bytes()).to_hex().as_str() {
        return (
            "failed",
            "The entitlement response did not match the saved selection.".into(),
        );
    }
    (
        "enabled",
        "Entitlement responses match the saved selection. This does not show EA account ownership or that the game accepted them.".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Entitlement {
        Entitlement {
            pack_id: "EP01".into(),
            offer_id: "fixture-offer".into(),
            group: "fixture-group".into(),
            tag: "fixture-tag".into(),
            kind: "DEFAULT".into(),
            version: "0".into(),
            id: 1,
        }
    }
    #[test]
    fn filters_are_a_union() {
        let e = entry();
        let mut f = Filters::default();
        assert!(f.matches(&e));
        f.offers.insert("other".into());
        assert!(!f.matches(&e));
        f.groups.insert(e.group.clone());
        assert!(f.matches(&e));
        f.groups.clear();
        f.items.insert(e.tag.clone());
        assert!(f.matches(&e));
    }
    #[test]
    fn broad_queries_do_not_enable_unselected_packs() {
        assert_eq!(
            response(&[entry()], &BTreeSet::new(), &Filters::default()).unwrap(),
            "<QueryEntitlementsResponse></QueryEntitlementsResponse>"
        );
    }
    #[test]
    fn escapes_xml_and_preserves_identity() {
        let mut e = entry();
        e.tag = "a<&\"\n".into();
        let xml = response(&[e], &BTreeSet::from(["EP01".into()]), &Filters::default()).unwrap();
        assert!(xml.contains("EntitlementTag=\"a&lt;&amp;&quot;&#10;\""));
        assert!(xml.contains("EntitlementId=\"1\""));
    }
    #[test]
    fn rejects_invalid_catalog_even_if_unselected() {
        assert!(response(&[entry(), entry()], &BTreeSet::new(), &Filters::default()).is_err());
        let mut e = entry();
        e.tag.push('\0');
        assert!(response(&[e], &BTreeSet::new(), &Filters::default()).is_err());
    }
}
