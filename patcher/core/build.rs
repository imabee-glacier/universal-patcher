//! Bakes the distribution origin and trusted application root into release builds.
//! Both come from hosting configuration, never from source:
//!   PATCHER_DIST_ORIGIN   e.g. https://downloads.example.org/
//!   PATCHER_DIST_ROOT     path to the application repository's root.json
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=PATCHER_DIST_ORIGIN");
    println!("cargo:rerun-if-env-changed=PATCHER_DIST_ROOT");
    println!("cargo:rerun-if-env-changed=PATCHER_APP_VERSION");
    // The release version a build reports; the release workflow sets it from the tag.
    let version = env::var("PATCHER_APP_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| env::var("CARGO_PKG_VERSION").unwrap());
    println!("cargo:rustc-env=PATCHER_APP_VERSION={version}");
    let origin = env::var("PATCHER_DIST_ORIGIN").unwrap_or_default();
    let root = match env::var("PATCHER_DIST_ROOT") {
        Ok(path) if !path.is_empty() => {
            println!("cargo:rerun-if-changed={path}");
            fs::read(&path).unwrap_or_else(|e| panic!("Cannot read PATCHER_DIST_ROOT {path}: {e}"))
        }
        _ => Vec::new(),
    };
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out.join("dist_root.json"), root).unwrap();
    fs::write(out.join("dist_origin.txt"), origin).unwrap();
}
