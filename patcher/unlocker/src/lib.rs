//! Windows entitlement component. It rewrites Qt-owned responses from the saved
//! policy. It does not claim EA account ownership or that the game accepted play.
#[path = "../../core/src/entitlement_pe.rs"]
pub mod pe;
#[cfg(all(windows, target_arch = "x86_64"))]
mod runtime;
#[cfg(all(windows, target_arch = "x86_64"))]
mod windows;
