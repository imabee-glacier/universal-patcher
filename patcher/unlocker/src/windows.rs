//! Narrow QByteArray import interception; response rewriting is explicit opt-in.
use std::{
    ffi::c_void,
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::*,
    System::{
        Diagnostics::Debug::ReadProcessMemory, LibraryLoader::*, Memory::*, ProcessStatus::*,
        SystemInformation::GetSystemDirectoryW, Threading::*,
    },
};

static SYSTEM_VERSION: OnceLock<usize> = OnceLock::new();
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static LSX: AtomicUsize = AtomicUsize::new(0);
static INVALID_ARRAYS: AtomicUsize = AtomicUsize::new(0);
static RUNTIME: OnceLock<Mutex<crate::runtime::Runtime>> = OnceLock::new();
static REPLACE: AtomicUsize = AtomicUsize::new(0);
static REWRITES: AtomicUsize = AtomicUsize::new(0);
thread_local! { static IN_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

// The imported functions are QByteArray accessors. Validate its Qt 5 x64
// header through checked process reads before inspecting any returned bytes.
// Never treat the returned pointer as an unbounded C string.
fn array_bytes(this: usize, returned: usize) -> Option<Vec<u8>> {
    let mut pointer = [0u8; 8];
    if !read(this, &mut pointer) {
        return None;
    }
    let header_address = usize::from_le_bytes(pointer);
    let mut header = [0u8; 24];
    if !read(header_address, &mut header) {
        return None;
    }
    let length = i32::from_le_bytes(header[4..8].try_into().ok()?);
    if !(5..=1024 * 1024).contains(&length) {
        return None;
    }
    let offset = isize::from_le_bytes(header[16..24].try_into().ok()?);
    if header_address.checked_add_signed(offset)? != returned {
        return None;
    }
    let mut bytes = vec![0; length as usize];
    if !read(returned, &mut bytes) {
        return None;
    }
    // Reject concurrent detach or resize; the caller must preserve the original.
    let mut after_pointer = [0u8; 8];
    let mut after_header = [0u8; 24];
    if !read(this, &mut after_pointer)
        || !read(header_address, &mut after_header)
        || pointer != after_pointer
        || header != after_header
    {
        return None;
    }
    Some(bytes)
}

fn symbol(name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    let module = *SYSTEM_VERSION.get_or_init(|| unsafe {
        let mut path = [0u16; 32768];
        let n = GetSystemDirectoryW(path.as_mut_ptr(), path.len() as u32) as usize;
        if n == 0 || n + 13 >= path.len() {
            return 0;
        }
        for (i, c) in "\\version.dll\0".encode_utf16().enumerate() {
            path[n + i] = c;
        }
        LoadLibraryExW(path.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) as usize
    });
    if module == 0 {
        return None;
    }
    unsafe { GetProcAddress(module as HMODULE, name.as_ptr()) }
}

macro_rules! forward {
    ($name:ident($($arg:ident : $ty:ty),* $(,)?) -> $ret:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $name($($arg:$ty),*) -> $ret {
            let Some(address) = symbol(concat!(stringify!($name), "\0").as_bytes()) else {
                unsafe { SetLastError(ERROR_PROC_NOT_FOUND); }
                return 0;
            };
            let f: unsafe extern "system" fn($($ty),*) -> $ret = unsafe { std::mem::transmute(address) };
            unsafe { f($($arg),*) }
        }
    };
}
forward!(GetFileVersionInfoA(path: *const u8, handle: u32, len: u32, data: *mut c_void) -> i32);
forward!(GetFileVersionInfoW(path: *const u16, handle: u32, len: u32, data: *mut c_void) -> i32);
forward!(
    GetFileVersionInfoExA(
        flags: u32,
        path: *const u8,
        handle: u32,
        len: u32,
        data: *mut c_void,
    ) -> i32
);
forward!(
    GetFileVersionInfoExW(
        flags: u32,
        path: *const u16,
        handle: u32,
        len: u32,
        data: *mut c_void,
    ) -> i32
);
forward!(GetFileVersionInfoSizeA(path: *const u8, handle: *mut u32) -> u32);
forward!(GetFileVersionInfoSizeW(path: *const u16, handle: *mut u32) -> u32);
forward!(GetFileVersionInfoSizeExA(flags: u32, path: *const u8, handle: *mut u32) -> u32);
forward!(GetFileVersionInfoSizeExW(flags: u32, path: *const u16, handle: *mut u32) -> u32);
forward!(
    VerQueryValueA(
        block: *const c_void,
        key: *const u8,
        result: *mut *mut c_void,
        len: *mut u32,
    ) -> i32
);
forward!(
    VerQueryValueW(
        block: *const c_void,
        key: *const u16,
        result: *mut *mut c_void,
        len: *mut u32,
    ) -> i32
);
forward!(VerLanguageNameA(lang: u32, buffer: *mut u8, len: u32) -> u32);
forward!(VerLanguageNameW(lang: u32, buffer: *mut u16, len: u32) -> u32);
forward!(
    VerFindFileA(
        flags: u32,
        file: *const u8,
        windows: *const u8,
        app: *const u8,
        current: *mut u8,
        current_len: *mut u32,
        dest: *mut u8,
        dest_len: *mut u32,
    ) -> u32
);
forward!(
    VerFindFileW(
        flags: u32,
        file: *const u16,
        windows: *const u16,
        app: *const u16,
        current: *mut u16,
        current_len: *mut u32,
        dest: *mut u16,
        dest_len: *mut u32,
    ) -> u32
);
forward!(
    VerInstallFileA(
        flags: u32,
        src: *const u8,
        dst: *const u8,
        src_dir: *const u8,
        dst_dir: *const u8,
        current: *const u8,
        temp: *mut u8,
        len: *mut u32,
    ) -> u32
);
forward!(
    VerInstallFileW(
        flags: u32,
        src: *const u16,
        dst: *const u16,
        src_dir: *const u16,
        dst_dir: *const u16,
        current: *const u16,
        temp: *mut u16,
        len: *mut u32,
    ) -> u32
);

fn read(address: usize, bytes: &mut [u8]) -> bool {
    let mut count = 0;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut count,
        ) != 0
            && count == bytes.len()
    }
}

unsafe extern "system" fn observe(this: *const c_void) -> *const u8 {
    let address = ORIGINAL.load(Ordering::Acquire);
    if address == 0 {
        return ptr::null();
    }
    let original: unsafe extern "system" fn(*const c_void) -> *const u8 =
        unsafe { std::mem::transmute(address) };
    let result = unsafe { original(this) };
    if IN_CALLBACK.with(|flag| flag.replace(true)) {
        return result;
    }
    struct Exit;
    impl Drop for Exit {
        fn drop(&mut self) {
            IN_CALLBACK.with(|f| f.set(false));
        }
    }
    let _exit = Exit;
    CALLS.fetch_add(1, Ordering::Relaxed);
    let mut prefix = [0u8; 5];
    if read(result as usize, &mut prefix) && prefix == *b"<LSX>" {
        if let Some(bytes) = array_bytes(this as usize, result as usize) {
            LSX.fetch_add(1, Ordering::Relaxed);
            let replacement = std::panic::catch_unwind(|| {
                let xml = std::str::from_utf8(&bytes).ok()?;
                let mut runtime = RUNTIME.get()?.try_lock().ok()?;
                match runtime.observe(xml) {
                    Ok(Some(text)) => {
                        note_lsx(xml, "rewrite");
                        Some(text)
                    }
                    Ok(None) => {
                        note_lsx(xml, "unchanged");
                        None
                    }
                    Err(error) => {
                        note_lsx(xml, error);
                        if error == "selection-changed" && xml.contains("QueryEntitlementsResponse")
                        {
                            remember_unchanged(xml);
                        }
                        None
                    }
                }
            })
            .ok()
            .flatten();
            if let Some(replacement) = replacement {
                let replace = REPLACE.load(Ordering::Acquire);
                if replace != 0
                    && replacement.len() <= 8 * 1024 * 1024
                    && array_bytes(this as usize, result as usize).as_deref()
                        == Some(bytes.as_slice())
                {
                    // Qt owns allocation and detaches shared storage. Never
                    // return a Rust temporary or free memory owned by Qt.
                    let f: unsafe extern "system" fn(
                        *mut c_void,
                        i32,
                        i32,
                        *const u8,
                        i32,
                    ) -> *mut c_void = unsafe { std::mem::transmute(replace) };
                    unsafe {
                        f(
                            this.cast_mut(),
                            0,
                            bytes.len() as i32,
                            replacement.as_ptr(),
                            replacement.len() as i32,
                        );
                    }
                    // Read the pointer Qt returns after it copies into its own storage.
                    let reread = unsafe { original(this) };
                    if let Some(stored) = array_bytes(this as usize, reread as usize)
                        && stored == replacement.as_bytes()
                    {
                        REWRITES.fetch_add(1, Ordering::Relaxed);
                        if let Some(runtime) = RUNTIME.get()
                            && let Ok(runtime) = runtime.try_lock()
                            && let Ok(text) = std::str::from_utf8(&stored)
                            && runtime.note_rewrite(text).is_ok()
                        {
                            remember_rewrite(text);
                        }
                    }
                    return reread;
                }
            }
        } else {
            INVALID_ARRAYS.fetch_add(1, Ordering::Relaxed);
        }
    }
    result
}

fn install_probe() -> Result<usize, &'static str> {
    let mut modules = vec![std::ptr::null_mut(); 1024];
    let mut needed = 0u32;
    if unsafe {
        K32EnumProcessModules(
            GetCurrentProcess(),
            modules.as_mut_ptr(),
            (modules.len() * std::mem::size_of::<HMODULE>()) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err("module-list");
    }
    let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());
    let mut installed = 0usize;
    for module in &modules[..count] {
        if module.is_null() {
            continue;
        }
        match install_module(*module) {
            Ok(n) => installed += n,
            Err("no-byte-array-imports") => {}
            Err(error) => return Err(error),
        }
    }
    // A DLL that imports QByteArray may load after this probe. The caller keeps
    // trying. An empty result is not a failure.
    Ok(installed)
}

fn install_module(module: HMODULE) -> Result<usize, &'static str> {
    let mut info: MODULEINFO = unsafe { std::mem::zeroed() };
    if unsafe {
        K32GetModuleInformation(
            GetCurrentProcess(),
            module,
            &mut info,
            std::mem::size_of::<MODULEINFO>() as u32,
        )
    } == 0
    {
        return Ok(0);
    }
    if info.SizeOfImage > 256 * 1024 * 1024 {
        return Err("image-limit");
    }
    let mut image = vec![0u8; info.SizeOfImage as usize];
    if !read(module as usize, &mut image) {
        return Ok(0);
    }
    let slots = match crate::pe::byte_array_imports(&image) {
        Ok(slots) => slots,
        Err(_) => return Ok(0),
    };
    if slots.is_empty() {
        return Ok(0);
    }
    let observe_address = observe as *const () as usize;
    let addresses: Vec<_> = slots
        .iter()
        .map(|slot| usize::from_le_bytes(image[slot.offset..slot.offset + 8].try_into().unwrap()))
        .collect();
    let original = addresses[0];
    if original == 0
        || addresses
            .iter()
            .any(|address| *address != original && *address != observe_address)
    {
        return Err("import-address-mismatch");
    }
    if original != observe_address {
        match ORIGINAL.compare_exchange(0, original, Ordering::Release, Ordering::Acquire) {
            Ok(_) => {}
            Err(existing) if existing == original || existing == observe_address => {}
            Err(_) => return Err("import-address-mismatch"),
        }
    }
    let mut installed = 0;
    for slot in slots {
        let address = (module as usize + slot.offset) as *mut AtomicUsize;
        let current = unsafe { &*address }.load(Ordering::SeqCst);
        if current == observe_address {
            continue;
        }
        if current != original {
            return Err("import-changed");
        }
        let mut protection = 0;
        if unsafe { VirtualProtect(address.cast(), 8, PAGE_READWRITE, &mut protection) } == 0 {
            return Err("import-protection");
        }
        let result = unsafe { &*address }.compare_exchange(
            original,
            observe_address,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        let mut unused = 0;
        if unsafe { VirtualProtect(address.cast(), 8, protection, &mut unused) } == 0 {
            return Err("restore-protection");
        }
        if result.is_err() {
            return Err("import-changed");
        }
        installed += 1;
    }
    Ok(installed)
}

fn saved_policy_dir() -> Option<std::path::PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")?;
    let dir = std::path::PathBuf::from(root)
        .join("UniversalPatcher")
        .join("game");
    dir.join("policy.json").is_file().then_some(dir)
}

fn bind_qt_replace(module: HMODULE) -> bool {
    let Some(replace) = (unsafe {
        GetProcAddress(
            module,
            c"?replace@QByteArray@@QEAAAEAV1@HHPEBDH@Z".as_ptr().cast(),
        )
    }) else {
        return false;
    };
    REPLACE.store(replace as usize, Ordering::Release);
    true
}

unsafe extern "system" fn worker(_: *mut c_void) -> u32 {
    let mut name = [0u16; 32768];
    let len = unsafe { GetModuleFileNameW(ptr::null_mut(), name.as_mut_ptr(), name.len() as u32) };
    let host = String::from_utf16_lossy(&name[..len as usize]);
    if !host.to_ascii_lowercase().ends_with("\\eadesktop.exe") {
        return 0;
    }
    let Some(dir) = saved_policy_dir() else {
        return 0;
    };
    let _ = patcher_core::entitlement::arm_saved_installation(&dir);
    let Ok(runtime) = crate::runtime::Runtime::load(dir.clone()) else {
        patcher_core::entitlement::clear_rewrite_proof(&dir);
        let _ = patcher_core::entitlement::record_hook(&dir, &host, "policy-unavailable");
        return 0;
    };
    if !runtime.any_enabled() {
        return 0;
    }
    let qt: Vec<u16> = "Qt5Core.dll\0".encode_utf16().collect();
    // EA Desktop loads this component before Qt. Wait until Qt5Core is mapped.
    let mut module = unsafe { GetModuleHandleW(qt.as_ptr()) };
    for _ in 0..120 {
        if !module.is_null() && bind_qt_replace(module) {
            break;
        }
        unsafe { Sleep(500) };
        module = unsafe { GetModuleHandleW(qt.as_ptr()) };
    }
    if REPLACE.load(Ordering::Acquire) == 0 || RUNTIME.set(Mutex::new(runtime)).is_err() {
        patcher_core::entitlement::clear_rewrite_proof(&dir);
        let _ = patcher_core::entitlement::record_hook(&dir, &host_name(), "qt-unavailable");
        return 0;
    }
    // Entitlement code may live in a DLL loaded after EA Desktop starts. Keep
    // installing newly loaded import tables, and record whether any call arrives.
    let mut reported = false;
    for _ in 0..180 {
        match install_probe() {
            Ok(0) if !reported && ORIGINAL.load(Ordering::Acquire) == 0 => {}
            Ok(_) => {
                if !reported {
                    let _ = patcher_core::entitlement::record_hook(&dir, &host_name(), "installed");
                    reported = true;
                }
            }
            Err(error) => {
                patcher_core::entitlement::clear_rewrite_proof(&dir);
                let _ = patcher_core::entitlement::record_hook(&dir, &host_name(), error);
                return 0;
            }
        }
        let _ = std::fs::write(
            dir.join("hook-activity.txt"),
            format!(
                "calls={}\nlsx={}\nrewrites={}\n",
                CALLS.load(Ordering::Relaxed),
                LSX.load(Ordering::Relaxed),
                REWRITES.load(Ordering::Relaxed)
            ),
        );
        unsafe { Sleep(1000) };
    }
    if !reported {
        patcher_core::entitlement::clear_rewrite_proof(&dir);
        let _ = patcher_core::entitlement::record_hook(&dir, &host_name(), "no-byte-array-imports");
    }
    0
}

fn note_lsx(xml: &str, outcome: &str) {
    let Some(dir) = saved_policy_dir() else {
        return;
    };
    let path = dir.join("hook-lsx.txt");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().count() >= 80 {
        return;
    }
    let tags: Vec<&str> = xml
        .split('<')
        .skip(1)
        .filter_map(|part| part.split([' ', '>', '/']).next())
        .filter(|name| !name.is_empty() && !name.starts_with(['!', '?', '/']))
        .take(4)
        .collect();
    let marker = if xml.contains("QueryEntitlements") {
        " query"
    } else if xml.contains("THESIMS4PC") {
        " sims"
    } else {
        ""
    };
    let _ = std::fs::write(
        path,
        format!("{existing}{outcome} {}{marker}\n", tags.join("/")),
    );
}

fn host_name() -> String {
    let mut name = [0u16; 32768];
    let len = unsafe { GetModuleFileNameW(ptr::null_mut(), name.as_mut_ptr(), name.len() as u32) };
    String::from_utf16_lossy(&name[..len as usize])
}

fn remember_rewrite(read_back: &str) {
    let Some(dir) = saved_policy_dir() else {
        return;
    };
    let _ = std::fs::write(dir.join("hook-readback.xml"), read_back.as_bytes());
    publish_hook_evidence(&dir, read_back, false, "");
}

fn remember_unchanged(original: &str) {
    let Some(dir) = saved_policy_dir() else {
        return;
    };
    let _ = std::fs::write(dir.join("hook-unchanged.xml"), original.as_bytes());
    let previous = std::fs::read_to_string(dir.join("hook-readback.xml")).unwrap_or_default();
    publish_hook_evidence(&dir, &previous, true, original);
}

fn message_id(xml: &str) -> String {
    xml.split("id=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .or_else(|| {
            xml.split("id='")
                .nth(1)
                .and_then(|rest| rest.split('\'').next())
        })
        .unwrap_or("")
        .to_string()
}

fn publish_hook_evidence(
    dir: &std::path::Path,
    read_back: &str,
    invalidation_unchanged: bool,
    unchanged_xml: &str,
) {
    let Ok(policy) = patcher_core::storage::Policy::load(dir) else {
        return;
    };
    let Ok(bytes) = std::fs::read(dir.join("entitlements.json")) else {
        return;
    };
    let Ok(catalog) = patcher_core::entitlement::Catalog::parse(&bytes) else {
        return;
    };
    let selected = catalog
        .entries
        .iter()
        .find(|entry| policy.desired.dlc.get(&entry.pack_id) == Some(&true));
    let deselected = catalog
        .entries
        .iter()
        .find(|entry| policy.desired.dlc.get(&entry.pack_id) == Some(&false));
    let Some(selected) = selected else {
        return;
    };
    let Some(deselected) = deselected else {
        return;
    };
    let observed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let response_id = message_id(read_back);
    let unchanged_response_id = message_id(unchanged_xml);
    let evidence = serde_json::json!({
        "schema": 1,
        "observed_at": observed_at,
        "response_id": response_id,
        "unchanged_response_id": unchanged_response_id,
        "unchanged_is_entitlement": unchanged_xml.contains("QueryEntitlementsResponse"),
        "os": std::env::var("OS").unwrap_or_else(|_| std::env::consts::OS.into()),
        "user": std::env::var("USERNAME").unwrap_or_default(),
        "host": host_name(),
        "hook": "installed",
        "selected_offer": selected.offer_id,
        "selected_tag": selected.tag,
        "selected_present": read_back.contains(&format!("ItemId=\"{}\"", selected.offer_id)),
        "deselected_offer": deselected.offer_id,
        "deselected_tag": deselected.tag,
        "deselected_absent": !read_back.contains(&format!("ItemId=\"{}\"", deselected.offer_id)),
        "rewrites": REWRITES.load(Ordering::Relaxed),
        "invalidation_unchanged": invalidation_unchanged,
        "read_back": read_back,
    });
    let _ = std::fs::write(
        dir.join("unlock-evidence.json"),
        serde_json::to_vec_pretty(&evidence).unwrap_or_default(),
    );
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: HMODULE, reason: u32, _: *mut c_void) -> i32 {
    if reason == 1 {
        unsafe {
            DisableThreadLibraryCalls(module);
            let thread = CreateThread(
                ptr::null(),
                0,
                Some(worker),
                ptr::null(),
                0,
                ptr::null_mut(),
            );
            if !thread.is_null() {
                CloseHandle(thread);
            }
        }
    }
    1
}

#[cfg(test)]
mod native_tests {
    use super::*;

    #[test]
    #[ignore = "requires the installed EA Qt5Core.dll; set PATCHER_TEST_QT"]
    fn real_qt_request_response_and_invalidation() {
        let path = std::env::var("PATCHER_TEST_QT").expect("PATCHER_TEST_QT");
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let qt = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        };
        assert!(!qt.is_null(), "Qt load failed: {}", unsafe {
            GetLastError()
        });
        unsafe fn proc(qt: HMODULE, name: &std::ffi::CStr) -> usize {
            unsafe { GetProcAddress(qt, name.as_ptr().cast()) }.expect("Qt export") as usize
        }
        let ctor: unsafe extern "system" fn(*mut usize, *const u8, i32) =
            unsafe { std::mem::transmute(proc(qt, c"??0QByteArray@@QEAA@PEBDH@Z")) };
        let dtor: unsafe extern "system" fn(*mut usize) =
            unsafe { std::mem::transmute(proc(qt, c"??1QByteArray@@QEAA@XZ")) };
        ORIGINAL.store(
            unsafe { proc(qt, c"?constData@QByteArray@@QEBAPEBDXZ") },
            Ordering::Release,
        );
        REPLACE.store(
            unsafe { proc(qt, c"?replace@QByteArray@@QEAAAEAV1@HHPEBDH@Z") },
            Ordering::Release,
        );
        let dir = std::env::temp_dir().join(format!("patcher-native-policy-{}", unsafe {
            GetCurrentProcessId()
        }));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let policy = patcher_core::storage::Policy {
            schema: 1,
            game_id: "sims4".into(),
            install: patcher_core::storage::Installation {
                identity: "native-fixture".into(),
                variant: "windows-native".into(),
                roots: Default::default(),
            },
            revision: 1,
            desired: patcher_core::model::Selection {
                dlc: [("EP01".into(), true), ("EP02".into(), false)]
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
            applied: None,
            preferences: Default::default(),
            suspended: None,
        };
        policy.save(&dir).unwrap();
        let catalog = patcher_core::entitlement::Catalog {
            schema: 1,
            game_id: "sims4".into(),
            entries: vec![
                patcher_core::entitlement::Entitlement {
                    pack_id: "EP01".into(),
                    offer_id: "selected-offer".into(),
                    group: "THESIMS4PC".into(),
                    tag: "selected-tag".into(),
                    kind: "DEFAULT".into(),
                    version: "0".into(),
                    id: 1,
                },
                patcher_core::entitlement::Entitlement {
                    pack_id: "EP02".into(),
                    offer_id: "deselected-offer".into(),
                    group: "THESIMS4PC".into(),
                    tag: "deselected-tag".into(),
                    kind: "DEFAULT".into(),
                    version: "0".into(),
                    id: 2,
                },
            ],
        };
        std::fs::write(
            dir.join("entitlements.json"),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
        assert!(
            RUNTIME
                .set(Mutex::new(
                    crate::runtime::Runtime::load(dir.clone()).unwrap()
                ))
                .is_ok()
        );
        let run = |input: &str| {
            let mut object = 0usize;
            unsafe {
                ctor(&mut object, input.as_ptr(), input.len() as i32);
            }
            let this = (&mut object as *mut usize).cast();
            let output = unsafe { observe(this) };
            let bytes =
                array_bytes(this as usize, output as usize).expect("valid Qt array after callback");
            unsafe {
                dtor(&mut object);
            }
            String::from_utf8(bytes).unwrap()
        };
        let request =
            "<LSX><Request id='7'><QueryEntitlements Group='THESIMS4PC'/></Request></LSX>";
        let response = "<LSX><Response id='7'><QueryEntitlementsResponse/></Response></LSX>";
        assert_eq!(run(request), request);
        let rewritten = run(response);
        assert!(
            rewritten.contains("ItemId=\"selected-offer\""),
            "response was not rewritten"
        );
        assert!(rewritten.contains("selected-tag"));
        assert!(
            !rewritten.contains("deselected-offer"),
            "disabled pack was enabled"
        );
        assert!(
            !rewritten.contains("deselected-tag"),
            "disabled pack was enabled"
        );
        assert_eq!(
            run(response),
            response,
            "duplicate response must pass unchanged"
        );
        assert_eq!(REWRITES.load(Ordering::Relaxed), 1);
        let (status, _) = patcher_core::entitlement::unlock_report(true, Some(&dir));
        assert_ne!(
            status, "enabled",
            "a Qt test process is not EA Desktop and must not report unlock enabled"
        );
        assert_eq!(run(request), request);
        let policy_path = dir.join("policy.json");
        let saved = std::fs::read(&policy_path).unwrap();
        let mut changed = saved.clone();
        changed[0] ^= 0xff;
        std::fs::write(&policy_path, &changed).unwrap();
        assert_eq!(
            run(response),
            response,
            "a changed selection leaves the next response unchanged"
        );
        std::fs::write(&policy_path, &saved).unwrap();
        println!(
            "Observed real Qt allocation, Rust replacement, Qt destruction, duplicate suppression and selection invalidation"
        );
    }
}
