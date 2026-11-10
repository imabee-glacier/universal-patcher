//! Explicit opt-in native test session. Not a release authorization mechanism.
use patcher_core::{
    entitlement::{Binding, BoundSession, Catalog, Filters},
    entitlement_transport::Transport,
    model::Selection,
};
use serde::Deserialize;
use std::{path::PathBuf, time::Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: u32,
    installation_identity: String,
    manifest_digest: String,
    policy_revision: u64,
    catalog: Catalog,
    selection: Selection,
}

pub struct Runtime {
    path: PathBuf,
    bytes: Vec<u8>,
    binding: Binding,
    transport: Transport,
}

impl Runtime {
    pub fn load(path: PathBuf) -> Result<Self, &'static str> {
        let bytes = Self::read_config(&path)?;
        let value = patcher_core::strict_json::parse(&bytes).map_err(|_| "invalid-config-json")?;
        let config: Config = serde_json::from_value(value).map_err(|_| "invalid-config-schema")?;
        if config.schema != 1 {
            return Err("unsupported-config-schema");
        }
        let binding = Binding {
            installation_identity: config.installation_identity,
            manifest_digest: config.manifest_digest,
            policy_revision: config.policy_revision,
        };
        let session = BoundSession::bind(config.catalog, &config.selection, binding.clone())?;
        Ok(Self {
            path,
            bytes,
            binding,
            transport: Transport::new(session),
        })
    }

    fn read_config(path: &std::path::Path) -> Result<Vec<u8>, &'static str> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|_| "config-unavailable")?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "config-read")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("config-limit");
        }
        Ok(bytes)
    }

    pub fn observe(&mut self, xml: &str) -> Result<Option<String>, &'static str> {
        // Removal or changes immediately retire this session. Never silently
        // hot-load a new selection while requests from the previous one exist.
        if Self::read_config(&self.path).as_deref() != Ok(self.bytes.as_slice()) {
            self.transport.invalidate();
            return Err("config-changed");
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
                // Scope this experimental session to the observed Sims group.
                // Broad or mixed-game queries need verified routing evidence.
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
        self.transport
            .observe(0, &self.binding, xml, Instant::now())
    }
}
