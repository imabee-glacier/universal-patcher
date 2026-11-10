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
    ($name:ident($($arg:ident : $ty:ty),*) -> $ret:ty) => {
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
forward!(GetFileVersionInfoA(path:*const u8, handle:u32, len:u32, data:*mut c_void)->i32);
forward!(GetFileVersionInfoW(path:*const u16, handle:u32, len:u32, data:*mut c_void)->i32);
forward!(GetFileVersionInfoExA(flags:u32,path:*const u8,handle:u32,len:u32,data:*mut c_void)->i32);
forward!(GetFileVersionInfoExW(flags:u32,path:*const u16,handle:u32,len:u32,data:*mut c_void)->i32);
forward!(GetFileVersionInfoSizeA(path:*const u8,handle:*mut u32)->u32);
forward!(GetFileVersionInfoSizeW(path:*const u16,handle:*mut u32)->u32);
forward!(GetFileVersionInfoSizeExA(flags:u32,path:*const u8,handle:*mut u32)->u32);
forward!(GetFileVersionInfoSizeExW(flags:u32,path:*const u16,handle:*mut u32)->u32);
forward!(VerQueryValueA(block:*const c_void,key:*const u8,result:*mut *mut c_void,len:*mut u32)->i32);
forward!(VerQueryValueW(block:*const c_void,key:*const u16,result:*mut *mut c_void,len:*mut u32)->i32);
forward!(VerLanguageNameA(lang:u32,buffer:*mut u8,len:u32)->u32);
forward!(VerLanguageNameW(lang:u32,buffer:*mut u16,len:u32)->u32);
forward!(VerFindFileA(flags:u32,file:*const u8,windows:*const u8,app:*const u8,current:*mut u8,current_len:*mut u32,dest:*mut u8,dest_len:*mut u32)->u32);
forward!(VerFindFileW(flags:u32,file:*const u16,windows:*const u16,app:*const u16,current:*mut u16,current_len:*mut u32,dest:*mut u16,dest_len:*mut u32)->u32);
forward!(VerInstallFileA(flags:u32,src:*const u8,dst:*const u8,src_dir:*const u8,dst_dir:*const u8,current:*const u8,temp:*mut u8,len:*mut u32)->u32);
forward!(VerInstallFileW(flags:u32,src:*const u16,dst:*const u16,src_dir:*const u16,dst_dir:*const u16,current:*const u16,temp:*mut u16,len:*mut u32)->u32);

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
                runtime.observe(xml).ok().flatten()
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
                    REWRITES.fetch_add(1, Ordering::Relaxed);
                    return unsafe { original(this) };
                }
            }
        } else {
            INVALID_ARRAYS.fetch_add(1, Ordering::Relaxed);
        }
    }
    result
}

fn install_probe() -> Result<usize, &'static str> {
    let module = unsafe { GetModuleHandleW(ptr::null()) };
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
        return Err("module-info");
    }
    if info.SizeOfImage > 256 * 1024 * 1024 {
        return Err("image-limit");
    }
    let mut image = vec![0u8; info.SizeOfImage as usize];
    if !read(module as usize, &mut image) {
        return Err("image-read");
    }
    let slots = crate::pe::byte_array_imports(&image)?;
    if slots.is_empty() {
        return Err("no-byte-array-imports");
    }
    let addresses: Vec<_> = slots
        .iter()
        .map(|s| usize::from_le_bytes(image[s.offset..s.offset + 8].try_into().unwrap()))
        .collect();
    if addresses.iter().any(|a| *a == 0 || *a != addresses[0]) {
        return Err("import-address-mismatch");
    }
    ORIGINAL.store(addresses[0], Ordering::Release);
    let mut installed = 0;
    for slot in slots {
        let address = (module as usize + slot.offset) as *mut AtomicUsize;
        let mut protection = 0;
        if unsafe { VirtualProtect(address.cast(), 8, PAGE_READWRITE, &mut protection) } == 0 {
            return Err("import-protection");
        }
        let result = unsafe { &*address }.compare_exchange(
            addresses[0],
            observe as *const () as usize,
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

unsafe extern "system" fn worker(_: *mut c_void) -> u32 {
    let mut name = [0u16; 32768];
    let len = unsafe { GetModuleFileNameW(ptr::null_mut(), name.as_mut_ptr(), name.len() as u32) };
    let host = String::from_utf16_lossy(&name[..len as usize]);
    if !host.to_ascii_lowercase().ends_with("\\eadesktop.exe") {
        return 0;
    }
    // Explicit probe opt-in; a DLL load alone does not authorize interception.
    if std::env::var("UNIVERSAL_PATCHER_PROBE").as_deref() != Ok("1") {
        return 0;
    }
    if let Some(path) = std::env::var_os("UNIVERSAL_PATCHER_TEST_CONFIG") {
        if let Ok(runtime) = crate::runtime::Runtime::load(path.into()) {
            let qt: Vec<u16> = "Qt5Core.dll\0".encode_utf16().collect();
            let module = unsafe { GetModuleHandleW(qt.as_ptr()) };
            if let Some(replace) = unsafe {
                GetProcAddress(
                    module,
                    c"?replace@QByteArray@@QEAAAEAV1@HHPEBDH@Z".as_ptr().cast(),
                )
            } {
                REPLACE.store(replace as usize, Ordering::Release);
                let _ = RUNTIME.set(Mutex::new(runtime));
            }
        }
    }
    let result = install_probe();
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        let path = std::path::PathBuf::from(dir).join("UniversalPatcherEntitlementTests");
        if std::fs::create_dir_all(&path).is_ok() {
            // Observe for the lifetime of the diagnostic session rather than
            // silently freezing counters before a slow game launch completes.
            // The bounded six-hour window prevents indefinite diagnostic IO.
            let mut previous = String::new();
            for _ in 0..4320 {
                let summary = format!(
                    "probe={result:?}\ncalls={}\nlsx={}\ninvalid_arrays={}\nrewrites={}\n",
                    CALLS.load(Ordering::Relaxed),
                    LSX.load(Ordering::Relaxed),
                    INVALID_ARRAYS.load(Ordering::Relaxed),
                    REWRITES.load(Ordering::Relaxed)
                );
                if summary != previous {
                    if std::fs::write(path.join("native-probe.txt"), &summary).is_err() {
                        break;
                    }
                    previous = summary;
                }
                unsafe {
                    Sleep(5000);
                }
            }
        }
    }
    0
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
        let config = std::env::temp_dir().join(format!("patcher-native-test-{}.json", unsafe {
            GetCurrentProcessId()
        }));
        let json = serde_json::json!({"schema":1,"installation_identity":"native-fixture",
            "manifest_digest":"a".repeat(64),"policy_revision":1,
            "catalog":{"schema":1,"game_id":"sims4","entries":[{
                "pack_id":"EP01","offer_id":"fixture-offer","group":"THESIMS4PC",
                "tag":"fixture-tag","kind":"DEFAULT","version":"0","id":1}]},
            "selection":{"dlc":{"EP01":true},"deferred":[],"language":null}});
        std::fs::write(&config, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(
            RUNTIME
                .set(Mutex::new(
                    crate::runtime::Runtime::load(config.clone()).unwrap()
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
            rewritten.contains("fixture-offer"),
            "response was not rewritten"
        );
        assert!(rewritten.contains("fixture-tag"));
        assert_eq!(
            run(response),
            response,
            "duplicate response must pass unchanged"
        );
        assert_eq!(REWRITES.load(Ordering::Relaxed), 1);
        assert_eq!(run(request), request);
        std::fs::remove_file(config).unwrap();
        assert_eq!(
            run(response),
            response,
            "unapply invalidates the pending request"
        );
        println!(
            "Observed real Qt allocation, Rust replacement, Qt destruction, duplicate suppression and config-removal invalidation"
        );
    }
}
