//! Loads the installation's saved policy. The Windows hook and the tests call this.
use patcher_core::entitlement::Replacement;
use std::path::PathBuf;

pub struct Runtime(Replacement);

impl Runtime {
    pub fn load(path: PathBuf) -> Result<Self, &'static str> {
        Ok(Self(Replacement::open(&path)?))
    }

    pub fn any_enabled(&self) -> bool {
        self.0.any_enabled()
    }

    pub fn observe(&mut self, xml: &str) -> Result<Option<String>, &'static str> {
        self.0.observe(xml)
    }

    pub fn note_rewrite(&self, read_back: &str) -> Result<(), &'static str> {
        patcher_core::entitlement::record_verified_rewrite(
            self.0.directory(),
            read_back,
            &host_path(),
        )
    }
}

fn host_path() -> String {
    let mut name = [0u16; 32768];
    let len = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    String::from_utf16_lossy(&name[..len as usize])
}
