//! Catalog entitlement records for the selected installation.
//!
//! Behavioral evidence is in the Mac map: parse_QueryEntitlements at 0005f380,
//! get_filters at 00064210, and skip_entitlement at 00064c10. The reconstruction
//! has parser gaps. Native handoff is still unverified, so this module reports
//! catalog state and does not claim that play access is ready.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
            if let Some(target) = target {
                if !attr.value().is_empty() {
                    target.insert(attr.value().to_owned());
                }
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
