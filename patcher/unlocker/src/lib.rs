//! Independent native adapter with opt-in experimental response replacement.
//! Not an enabled unlock capability. No original unlocker implementation is used.
#[path = "../../core/src/entitlement_pe.rs"]
pub mod pe;
#[cfg(all(windows, target_arch = "x86_64"))]
mod runtime;
#[cfg(all(windows, target_arch = "x86_64"))]
mod windows;
