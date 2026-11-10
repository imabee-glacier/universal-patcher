//! Bounded LSX request/response correlation for the independent adapter.
//! No sockets, credentials, or native hooks are owned here. The adapter supplies
//! a connection identity and current binding. Unrelated messages pass unchanged.
use crate::entitlement::{Binding, BoundSession};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

struct Pending {
    response: String,
    expires: Instant,
}

pub struct Transport {
    session: BoundSession,
    pending: BTreeMap<(u64, String), Pending>,
}

impl Transport {
    pub fn new(session: BoundSession) -> Self {
        Self {
            session,
            pending: BTreeMap::new(),
        }
    }

    pub fn disconnect(&mut self, connection: u64) {
        self.pending.retain(|(id, _), _| *id != connection);
    }

    pub fn invalidate(&mut self) {
        self.session.invalidate();
        self.pending.clear();
    }

    /// None means preserve the original bytes. Errors never include message data.
    pub fn observe(
        &mut self,
        connection: u64,
        binding: &Binding,
        xml: &str,
        now: Instant,
    ) -> Result<Option<String>, &'static str> {
        self.pending.retain(|_, p| p.expires > now);
        if xml.len() > 1024 * 1024 {
            return Err("LSX message exceeds limit");
        }
        if !xml.contains("<LSX") {
            return Ok(None);
        }
        if xml.contains("<!DOCTYPE") {
            return Err("LSX DTD is unsupported");
        }
        let doc = roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: 8192,
                ..Default::default()
            },
        )
        .map_err(|_| "Invalid LSX message")?;
        let root = doc.root_element();
        if root.tag_name().name() != "LSX" || root.namespaces().len() != 0 {
            return Ok(None);
        }
        let envelopes: Vec<_> = root.children().filter(|n| n.is_element()).collect();
        if envelopes.len() != 1 {
            return Err("Ambiguous LSX envelope");
        }
        let envelope = envelopes[0];
        if !matches!(envelope.tag_name().name(), "Request" | "Response") {
            return Ok(None);
        }
        let payloads: Vec<_> = envelope.children().filter(|n| n.is_element()).collect();
        if payloads.len() != 1 {
            return Ok(None);
        }
        let payload = payloads[0];
        if payload.namespaces().len() != 0 {
            return Ok(None);
        }
        let id = match envelope.attribute("id") {
            Some(id) if !id.is_empty() && id.len() <= 128 => id,
            _ => return Err("Missing or invalid LSX correlation id"),
        };
        let key = (connection, id.to_owned());
        match (envelope.tag_name().name(), payload.tag_name().name()) {
            ("Request", "QueryEntitlements") => {
                // Retire any same-id state before validation; a rejected request
                // must never inherit a prior successful request's entitlements.
                self.pending.remove(&key);
                let response = self.session.query(binding, &xml[payload.range()])?;
                let retained: usize = self.pending.values().map(|p| p.response.len()).sum();
                if self.pending.len() >= 256
                    || retained.saturating_add(response.len()) > 8 * 1024 * 1024
                {
                    return Err("Too many outstanding entitlement requests");
                }
                self.pending.insert(
                    key,
                    Pending {
                        response,
                        expires: now + Duration::from_secs(60),
                    },
                );
                Ok(None)
            }
            ("Response", "QueryEntitlementsResponse") => {
                let Some(pending) = self.pending.remove(&key) else {
                    return Ok(None);
                };
                // Check binding again; the selection may change between messages.
                self.session.query(binding, "<QueryEntitlements/>")?;
                let range = payload.range();
                let mut changed = String::with_capacity(xml.len() + pending.response.len());
                changed.push_str(&xml[..range.start]);
                changed.push_str(&pending.response);
                changed.push_str(&xml[range.end..]);
                Ok(Some(changed))
            }
            ("Response", _) => {
                self.pending.remove(&key);
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}
