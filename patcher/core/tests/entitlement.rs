//! Synthetic protocol fixtures, not proof of native game handoff.
use patcher_core::{
    entitlement::{
        Catalog, Entitlement, Filters, Replacement, Session, place_component, record_hook,
        record_verified_rewrite, shipped_catalog, unlock_report,
    },
    model::Selection,
    storage::{Installation, Policy},
};
use std::{collections::BTreeMap, fs};

#[test]
fn pe_import_discovery_is_bounded_and_does_not_hook_qstring() {
    use patcher_core::entitlement_pe::byte_array_imports;
    let mut b = vec![0u8; 4096];
    b[..2].copy_from_slice(b"MZ");
    b[0x3c..0x40].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    b[208..212].copy_from_slice(&4096u32.to_le_bytes());
    b[260..264].copy_from_slice(&16u32.to_le_bytes());
    b[272..276].copy_from_slice(&512u32.to_le_bytes());
    b[276..280].copy_from_slice(&40u32.to_le_bytes());
    b[512..516].copy_from_slice(&768u32.to_le_bytes());
    b[524..528].copy_from_slice(&640u32.to_le_bytes());
    b[528..532].copy_from_slice(&896u32.to_le_bytes());
    b[640..652].copy_from_slice(b"Qt5Core.dll\0");
    for (i, symbol) in [
        "?constData@QByteArray@@QEBAPEBDXZ",
        "?unicode@QString@@QEBAPEBVQChar@@XZ",
    ]
    .iter()
    .enumerate()
    {
        let rva = 1024 + i * 128;
        b[768 + i * 8..776 + i * 8].copy_from_slice(&(rva as u64).to_le_bytes());
        b[rva + 2..rva + 2 + symbol.len()].copy_from_slice(symbol.as_bytes());
    }
    let slots = byte_array_imports(&b).unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].offset, 896);
    for len in 0..b.len() {
        assert!(byte_array_imports(&b[..len]).is_err());
    }
    b[512..516].copy_from_slice(&4095u32.to_le_bytes());
    assert!(byte_array_imports(&b).is_err());
}

#[test]
fn transport_correlates_connections_expires_and_preserves_envelopes() {
    use patcher_core::{
        entitlement::{Binding, BoundSession},
        entitlement_transport::Transport,
    };
    use std::time::{Duration, Instant};
    let binding = Binding {
        installation_identity: "fixture".into(),
        manifest_digest: "a".repeat(64),
        policy_revision: 1,
    };
    let selected = Selection {
        dlc: BTreeMap::from([("EP01".into(), true)]),
        ..Default::default()
    };
    let mut transport =
        Transport::new(BoundSession::bind(catalog(), &selected, binding.clone()).unwrap());
    let now = Instant::now();
    let request = "<LSX><Request id='1' recipient='fixture'><QueryEntitlements/></Request></LSX>";
    let response =
        "<LSX><Response id='1' sender='fixture'><QueryEntitlementsResponse/></Response></LSX>";
    assert!(
        transport
            .observe(1, &binding, request, now)
            .unwrap()
            .is_none()
    );
    assert!(
        transport
            .observe(2, &binding, response, now)
            .unwrap()
            .is_none()
    );
    let changed = transport
        .observe(1, &binding, response, now)
        .unwrap()
        .unwrap();
    assert!(changed.starts_with("<LSX><Response id='1' sender='fixture'>"));
    assert!(changed.contains("ItemId=\"offer-1\""));
    assert!(!changed.contains("offer-2"));
    assert!(
        transport
            .observe(1, &binding, response, now)
            .unwrap()
            .is_none()
    );
    transport.observe(1, &binding, request, now).unwrap();
    assert!(
        transport
            .observe(1, &binding, response, now + Duration::from_secs(61))
            .unwrap()
            .is_none()
    );
    transport.observe(1, &binding, request, now).unwrap();
    transport.disconnect(1);
    assert!(
        transport
            .observe(1, &binding, response, now)
            .unwrap()
            .is_none()
    );
    transport.observe(1, &binding, request, now).unwrap();
    let mut changed_binding = binding.clone();
    changed_binding.policy_revision += 1;
    assert!(
        transport
            .observe(1, &changed_binding, response, now)
            .is_err()
    );
    transport.invalidate();
    assert!(transport.observe(1, &binding, request, now).is_err());
}

#[test]
fn sessions_cannot_cross_installations_releases_or_unapply() {
    use patcher_core::entitlement::{Binding, BoundSession};
    let binding = Binding {
        installation_identity: "fixture-install".into(),
        manifest_digest: "a".repeat(64),
        policy_revision: 1,
    };
    let mut session =
        BoundSession::bind(catalog(), &Selection::default(), binding.clone()).unwrap();
    assert!(session.query(&binding, "<QueryEntitlements/>").is_ok());
    let mut changed = binding.clone();
    changed.installation_identity = "another-install".into();
    assert!(session.query(&changed, "<QueryEntitlements/>").is_err());
    changed = binding.clone();
    changed.manifest_digest = "b".repeat(64);
    assert!(session.query(&changed, "<QueryEntitlements/>").is_err());
    session.invalidate();
    assert!(session.query(&binding, "<QueryEntitlements/>").is_err());
    let replacement =
        BoundSession::bind(catalog(), &Selection::default(), binding.clone()).unwrap();
    assert!(replacement.query(&binding, "<QueryEntitlements/>").is_ok());
}

fn catalog() -> Catalog {
    Catalog {
        schema: 1,
        game_id: "sims4".into(),
        entries: (1..=2)
            .map(|i| Entitlement {
                pack_id: format!("EP{i:02}"),
                offer_id: format!("offer-{i}"),
                group: "group".into(),
                tag: format!("tag-{i}"),
                kind: "DEFAULT".into(),
                version: "0".into(),
                id: i,
            })
            .collect(),
    }
}
fn session() -> Session {
    Session::bind(
        catalog(),
        &Selection {
            dlc: BTreeMap::from([("EP01".into(), true), ("EP02".into(), false)]),
            ..Default::default()
        },
        7,
    )
    .unwrap()
}

#[test]
fn policy_selection_is_enforced_for_all_filter_forms() {
    for query in [
        "<QueryEntitlements/>",
        "<QueryEntitlements Group='group'/>",
        "<QueryEntitlements><FilterGroups>group</FilterGroups></QueryEntitlements>",
    ] {
        let response = session().query(7, query).unwrap();
        let doc = roxmltree::Document::parse(&response).unwrap();
        let entries: Vec<_> = doc
            .root_element()
            .children()
            .filter(|n| n.is_element())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].attribute("ItemId"), Some("offer-1"));
    }
    let response = session()
        .query(7, "<QueryEntitlements OfferId='offer-2'/>")
        .unwrap();
    assert!(!response.contains("<Entitlement "));
}

#[test]
fn attribute_item_id_and_child_items_have_different_meanings() {
    let e = &catalog().entries[0];
    assert!(
        Filters::parse("<QueryEntitlements ItemId='offer-1'/>")
            .unwrap()
            .matches(e)
    );
    assert!(
        !Filters::parse("<QueryEntitlements ItemId='tag-1'/>")
            .unwrap()
            .matches(e)
    );
    assert!(
        Filters::parse("<QueryEntitlements><FilterItems>tag-1</FilterItems></QueryEntitlements>")
            .unwrap()
            .matches(e)
    );
}

#[test]
fn policy_change_invalidates_old_session() {
    assert!(session().query(8, "<QueryEntitlements/>").is_err());
    assert!(session().query(6, "<QueryEntitlements/>").is_err());
    let empty = Session::bind(catalog(), &Selection::default(), 8).unwrap();
    assert!(
        !empty
            .query(8, "<QueryEntitlements/>")
            .unwrap()
            .contains("<Entitlement ")
    );
}

#[test]
fn unknown_selected_pack_is_not_silently_ignored() {
    let selection = Selection {
        dlc: BTreeMap::from([("EP99".into(), true)]),
        ..Default::default()
    };
    assert!(Session::bind(catalog(), &selection, 1).is_err());
}

#[test]
fn malformed_and_unrecognized_queries_fail_closed() {
    for query in [
        "",
        "<QueryEntitlements",
        "<Other/>",
        "<QueryEntitlements Surprise='yes'/>",
        "<QueryEntitlements Group='a' Group='b'/>",
        "<QueryEntitlements><Unknown/></QueryEntitlements>",
        "<QueryEntitlements><FilterItems><Nested/></FilterItems></QueryEntitlements>",
        "<QueryEntitlements><FilterItems attr='x'>a</FilterItems></QueryEntitlements>",
        "<QueryEntitlements xmlns='urn:other'/>",
        "<QueryEntitlements>unexpected</QueryEntitlements>",
        "<!DOCTYPE QueryEntitlements><QueryEntitlements/>",
        "<!DOCTYPE q [<!ENTITY a SYSTEM 'file:///etc/passwd'>]><QueryEntitlements ItemId='&a;'/>",
        "<QueryEntitlements/><QueryEntitlements/>",
    ] {
        assert!(Filters::parse(query).is_err(), "accepted {query}");
    }
}

#[test]
fn bounds_and_entities() {
    let query = format!("<QueryEntitlements ItemId='{}'/>", "x".repeat(4097));
    assert!(Filters::parse(&query).is_err());
    assert!(Filters::parse(&" ".repeat(65537)).is_err());
    let f = Filters::parse("<QueryEntitlements OfferId='a&amp;b'><FilterItems>x&lt;y</FilterItems></QueryEntitlements>").unwrap();
    assert!(f.offers.contains("a&b"));
    assert!(f.items.contains("x<y"));
}

#[test]
fn catalog_roundtrip_and_invalid_schemas() {
    let bytes = serde_json::to_vec(&catalog()).unwrap();
    assert_eq!(Catalog::parse(&bytes).unwrap().entries.len(), 2);
    for bytes in [
        br#"{"schema":1,"schema":1,"game_id":"sims4","entries":[]}"#.as_slice(),
        br#"{"schema":2,"game_id":"sims4","entries":[]}"#,
        br#"{"schema":1,"game_id":"other","entries":[]}"#,
        br#"{"schema":1,"game_id":"sims4","entries":[],"extra":true}"#,
    ] {
        assert!(Catalog::parse(bytes).is_err());
    }
    let mut c = catalog();
    c.entries[1].id = c.entries[0].id;
    assert!(Catalog::parse(&serde_json::to_vec(&c).unwrap()).is_err());
}

#[test]
fn conflicting_pack_ownership_and_large_catalogs_are_rejected() {
    let mut c = catalog();
    let mut conflicting = c.entries[0].clone();
    conflicting.id = 3;
    conflicting.pack_id = "EP03".into();
    c.entries.push(conflicting);
    assert!(Session::bind(c, &Selection::default(), 1).is_err());
    let mut c = catalog();
    c.entries = (0..300)
        .map(|id| {
            let mut entry = catalog().entries.remove(0);
            entry.id = id;
            entry.offer_id = format!("offer-{id}");
            entry.tag = "x".repeat(4096);
            entry
        })
        .collect();
    assert!(Session::bind(c, &Selection::default(), 1).is_err());
}

fn saved_policy(dir: &std::path::Path, enabled: &[(&str, bool)]) {
    Policy {
        schema: 1,
        game_id: "sims4".into(),
        install: Installation {
            identity: "fixture-install".into(),
            variant: "windows-native".into(),
            roots: BTreeMap::from([("game".into(), dir.join("game"))]),
        },
        revision: 3,
        desired: Selection {
            dlc: enabled.iter().map(|(id, on)| ((*id).into(), *on)).collect(),
            ..Default::default()
        },
        applied: None,
        preferences: Default::default(),
        suspended: None,
    }
    .save(dir)
    .unwrap();
}

fn two_pack_catalog() -> Catalog {
    Catalog {
        schema: 1,
        game_id: "sims4".into(),
        entries: vec![
            Entitlement {
                pack_id: "EP01".into(),
                offer_id: "selected-offer".into(),
                group: "THESIMS4PC".into(),
                tag: "selected-tag".into(),
                kind: "DEFAULT".into(),
                version: "0".into(),
                id: 1,
            },
            Entitlement {
                pack_id: "EP02".into(),
                offer_id: "deselected-offer".into(),
                group: "THESIMS4PC".into(),
                tag: "deselected-tag".into(),
                kind: "DEFAULT".into(),
                version: "0".into(),
                id: 2,
            },
        ],
    }
}

#[test]
fn replacement_uses_saved_policy_and_leaves_a_changed_selection_alone() {
    let dir = tempfile::tempdir().unwrap();
    saved_policy(dir.path(), &[("EP01", true), ("EP02", false)]);
    fs::write(
        dir.path().join("entitlements.json"),
        serde_json::to_vec(&two_pack_catalog()).unwrap(),
    )
    .unwrap();
    let mut replacement = Replacement::open(dir.path()).unwrap();
    let request = "<LSX><Request id='7'><QueryEntitlements Group='THESIMS4PC'/></Request></LSX>";
    let response = "<LSX><Response id='7'><QueryEntitlementsResponse/></Response></LSX>";
    assert!(replacement.observe(request).unwrap().is_none());
    let rewritten = replacement.observe(response).unwrap().unwrap();
    assert!(rewritten.contains("ItemId=\"selected-offer\""));
    assert!(rewritten.contains("EntitlementTag=\"selected-tag\""));
    assert!(!rewritten.contains("deselected-offer"));
    assert!(!rewritten.contains("deselected-tag"));
    let sims = "<LSX><Response id='9'><QueryEntitlementsResponse><Entitlement Group=\"THESIMS4PC\"/></QueryEntitlementsResponse></Response></LSX>";
    let rows: [&[&str]; 3] = [
        &[
            "<LSX><Request id='9'><QueryEntitlements Group='THESIMS4PC'/></Request></LSX>",
            sims,
            sims,
        ],
        &[sims, sims],
        &[
            "<LSX><Request id='7'><QueryEntitlements Group='THESIMS4PC'/></Request></LSX>",
            "<LSX><Response id='7'><QueryEntitlementsResponse/></Response></LSX>",
            "<LSX><Response id='7'><QueryEntitlementsResponse/></Response></LSX>",
        ],
    ];
    for messages in rows {
        let mut session = Replacement::open(dir.path()).unwrap();
        let results: Vec<Option<String>> = messages
            .iter()
            .map(|xml| session.observe(xml).unwrap())
            .collect();
        let rewrites: Vec<&str> = results.iter().filter_map(|item| item.as_deref()).collect();
        assert_eq!(
            rewrites.len(),
            1,
            "a second identical response was rewritten"
        );
        assert!(results.last().unwrap().is_none());
        assert!(rewrites[0].contains("ItemId=\"selected-offer\""));
        assert!(!rewrites[0].contains("deselected-offer"));
    }
    assert!(
        replacement
            .observe("<LSX><Response id='1'><GetProfileResponse/></Response></LSX>")
            .unwrap()
            .is_none()
    );
    let path = dir.path().join("policy.json");
    let saved = fs::read(&path).unwrap();
    let mut changed = saved.clone();
    changed[0] ^= 0xff;
    fs::write(&path, &changed).unwrap();
    assert!(
        replacement.observe(response).is_err(),
        "a changed selection invalidates the session"
    );
    assert_eq!(unlock_report(true, Some(dir.path())).0, "disabled");
    fs::write(&path, &saved).unwrap();
    let host = r"C:\Program Files\Electronic Arts\EA Desktop\EA Desktop\EADesktop.exe";
    assert!(record_verified_rewrite(dir.path(), &rewritten, r"C:\not-ea\helper.exe").is_err());
    record_verified_rewrite(dir.path(), &rewritten, host).unwrap();
    assert_eq!(unlock_report(true, Some(dir.path())).0, "disabled");
    record_hook(dir.path(), r"C:\Windows\System32\notepad.exe", "installed").unwrap();
    assert_eq!(unlock_report(true, Some(dir.path())).0, "failed");
    record_hook(dir.path(), host, "no-byte-array-imports").unwrap();
    assert_eq!(unlock_report(true, Some(dir.path())).0, "failed");
    record_hook(dir.path(), host, "installed").unwrap();
    let (state, detail) = unlock_report(true, Some(dir.path()));
    assert_eq!(state, "enabled");
    assert!(detail.contains("does not show EA account ownership"));
    assert_eq!(unlock_report(false, Some(dir.path())).0, "unsupported");
}

#[test]
fn component_is_copied_into_the_destination_directory() {
    let dir = tempfile::tempdir().unwrap();
    let bundled = dir.path().join("version.dll");
    let dest = dir.path().join("EA Desktop");
    fs::create_dir(&dest).unwrap();
    let mut bytes = b"MZ".to_vec();
    bytes.extend(std::iter::repeat_n(0, 62));
    fs::write(&bundled, bytes).unwrap();
    place_component(&bundled, &dest).unwrap();
    assert_eq!(
        fs::read(dest.join("version.dll")).unwrap(),
        fs::read(&bundled).unwrap()
    );
    place_component(&bundled, &dest).unwrap();
}

#[test]
fn unmapped_selected_pack_fails_closed_and_is_not_enabled() {
    let dir = tempfile::tempdir().unwrap();
    saved_policy(dir.path(), &[("EP99", true), ("EP01", false)]);
    fs::write(
        dir.path().join("entitlements.json"),
        serde_json::to_vec(&two_pack_catalog()).unwrap(),
    )
    .unwrap();
    match Replacement::open(dir.path()) {
        Err("Selected pack has no entitlement mapping") => {}
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("unmapped pack was accepted"),
    }
    assert_eq!(unlock_report(true, Some(dir.path())).0, "failed");
    let shipped = shipped_catalog().unwrap();
    assert!(shipped.entries.iter().any(|entry| entry.pack_id == "EP01"));
    assert!(
        shipped
            .entries
            .iter()
            .all(|entry| entry.tag != "ONLINE_ACCESS")
    );
    assert!(
        shipped
            .entries
            .iter()
            .all(|entry| entry.group == "THESIMS4PC")
    );
}
