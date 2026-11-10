//! Synthetic protocol fixtures, not proof of native game handoff.
use patcher_core::{
    entitlement::{Catalog, Entitlement, Filters, Session},
    model::Selection,
};
use std::collections::BTreeMap;

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
