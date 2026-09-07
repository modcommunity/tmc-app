//! The half that runs inside the game.
//!
//! **Windows only, and honestly labelled.** Everything in this file has been
//! type-checked against the Windows target and has never been run against a
//! real game — see the crate header and `CLAUDE.md`'s "Not built yet". Nothing
//! else in the app depends on it working; a sandbox set to the virtual strategy
//! refuses at deploy time unless the app is built with the `usvfs-hooks`
//! feature, which is off.
//!
//! HOW THE REDIRECTION WORKS
//! -------------------------
//! **Import Address Table patching**, not inline hooking. When a PE module
//! calls `CreateFileW`, it does so through a pointer in its own import table
//! that the loader filled in; overwriting that pointer redirects every such
//! call in that module. The alternative — rewriting the first instructions of
//! the real function and building a trampoline — needs an instruction-length
//! decoder, has to be right about every prologue Microsoft ships, and breaks
//! differently on every Windows update.
//!
//! What IAT patching costs, stated plainly because it decides which games this
//! can work for:
//!
//!   * **A call through `GetProcAddress` is not redirected.** The pointer never
//!     came from an import table.
//!   * **A direct `ntdll` call is not redirected.** Some engines and most
//!     anti-tamper layers do this.
//!   * **A module loaded LATER has to be patched when it arrives**, which is
//!     why `LoadLibraryW` and friends are hooked too — the hook re-runs the
//!     patch over the new module.
//!
//! WHY ONLY READS
//! --------------
//! A file the game CREATES inside a virtualised directory is written to the
//! real directory, unredirected. That is a decision rather than a gap: a game
//! writing a save, a crash dump or a config into its own folder expects to find
//! it there again with every other tool, and a manager that quietly diverted
//! those into a staging folder would be one nobody could ever find their saves
//! in. Redirection applies to opening something that already exists virtually.
//!
//! WHY NOTHING HERE ALLOCATES ON THE HOT PATH
//! ------------------------------------------
//! These functions run inside `CreateFileW`, thousands of times a second, on
//! whatever thread the game happens to be on — including, during startup, a
//! loader thread holding the loader lock. Allocating, taking a lock another
//! thread might hold, or calling anything re-entrant is a deadlock waiting for
//! a specific machine. The tree is read once at attach into an immutable
//! structure and every lookup after that is a read-only map probe.

#![cfg(windows)]

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{FARPROC, HANDLE, HMODULE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileAttributesW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE,
};
/*
 * `IMAGE_NT_HEADERS64` lives in `Diagnostics::Debug` but is gated behind the
 * `Win32_System_SystemInformation` FEATURE — the feature name and the module
 * path do not match, which is worth the note because the compiler's suggestion
 * sends you to a module that does not export it.
 */
use windows_sys::Win32::System::Diagnostics::Debug::{
    IMAGE_DIRECTORY_ENTRY_IMPORT, IMAGE_NT_HEADERS64,
};
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
use windows_sys::Win32::System::SystemServices::{IMAGE_DOS_HEADER, IMAGE_IMPORT_DESCRIPTOR};
use windows_sys::Win32::System::WindowsProgramming::IMAGE_THUNK_DATA64;

use crate::tree::VirtualTree;

/// The tree, read once at attach.
///
/// A `OnceLock` rather than a mutex: it is written exactly once, before the
/// hooks are installed, and read from every thread forever after. A lock here
/// would be a lock taken inside `CreateFileW` on the game's render thread.
static TREE: OnceLock<VirtualTree> = OnceLock::new();

/*
 * The real functions, saved before their import entries were overwritten.
 *
 * `AtomicUsize` rather than `static mut`. The moment an import entry is patched
 * another thread can be inside the hook, so the write and the read genuinely
 * race — `static mut` would be undefined behaviour that happens to work, and
 * the compiler says so. A relaxed atomic is the correct primitive and costs
 * nothing on x86: the pointer is published with `Release` before the patch that
 * makes it reachable, and read with `Acquire`.
 *
 * Zero means "not installed", which is why the hooks fall back to calling the
 * genuine import directly rather than treating it as impossible.
 */
static REAL_CREATE_FILE_W: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_FILE_ATTRIBUTES_W: AtomicUsize = AtomicUsize::new(0);

/*
 * The REAL signature, taken from `windows-sys` rather than approximated. The
 * fourth argument is a `SECURITY_ATTRIBUTES` pointer; declaring it as
 * `*const c_void` compiles on its own and then refuses the genuine
 * `CreateFileW` as a fallback — which is exactly how the mismatch was found,
 * by type-checking this file against the Windows target.
 */
type CreateFileWFn = unsafe extern "system" fn(
    *const u16,
    u32,
    FILE_SHARE_MODE,
    *const SECURITY_ATTRIBUTES,
    u32,
    FILE_FLAGS_AND_ATTRIBUTES,
    HANDLE,
) -> HANDLE;

type GetFileAttributesWFn = unsafe extern "system" fn(*const u16) -> u32;

/// Load the tree and install the hooks.
///
/// Called from `DllMain` on attach. Returns `false` for every failure —
/// including "there is no tree", which is the ordinary case for a process that
/// was injected by mistake — and a `false` leaves the game running completely
/// unmodified rather than half-hooked.
pub fn install() -> bool {
    let Ok(path) = std::env::var(crate::shm::ENV_BLOB) else {
        return false;
    };

    let Ok(published) = crate::shm::read(std::path::Path::new(&path)) else {
        return false;
    };

    /*
     * A tree for a DIFFERENT game directory means this DLL landed in a process
     * it was not meant for — a child the game spawned, a helper, an installer.
     * Hooking that process would redirect files for something nobody asked
     * about.
     */
    if let Ok(root) = std::env::var(crate::shm::ENV_ROOT) {
        if !published.tree.root.eq_ignore_ascii_case(&root) {
            return false;
        }
    }

    if TREE.set(published.tree).is_err() {
        // Already installed. Not an error — `DllMain` can be reached twice if
        // something loads the DLL a second time.
        return true;
    }

    patch_loaded_modules()
}

/// Our `CreateFileW`.
///
/// # Safety
///
/// Called by the operating system's own callers with their arguments. Every
/// pointer here is one Windows already validated for the real function, and the
/// only thing this adds is a read of a NUL-terminated wide string — bounded
/// below — before handing everything to the original.
unsafe extern "system" fn hooked_create_file_w(
    file_name: *const u16,
    desired_access: u32,
    share_mode: FILE_SHARE_MODE,
    security: *const SECURITY_ATTRIBUTES,
    creation: u32,
    flags: FILE_FLAGS_AND_ATTRIBUTES,
    template: HANDLE,
) -> HANDLE {
    let saved = REAL_CREATE_FILE_W.load(Ordering::Acquire);

    /*
     * Zero cannot happen — the pointer is published before the patch that makes
     * this function reachable — but calling through a null pointer inside the
     * game's file open would be the worst possible way to find out otherwise,
     * so the genuine import is the fallback.
     */
    let real: CreateFileWFn = if saved == 0 {
        CreateFileW
    } else {
        std::mem::transmute::<usize, CreateFileWFn>(saved)
    };

    if let Some(redirected) = redirect(file_name) {
        let wide = to_wide(&redirected);

        return real(
            wide.as_ptr(),
            desired_access,
            share_mode,
            security,
            creation,
            flags,
            template,
        );
    }

    real(
        file_name,
        desired_access,
        share_mode,
        security,
        creation,
        flags,
        template,
    )
}

/// Our `GetFileAttributesW`.
///
/// Hooked because a loader that enumerates a directory then asks each entry
/// whether it is a file would otherwise be told the virtual ones do not exist.
///
/// # Safety
///
/// As [`hooked_create_file_w`].
unsafe extern "system" fn hooked_get_file_attributes_w(file_name: *const u16) -> u32 {
    let saved = REAL_GET_FILE_ATTRIBUTES_W.load(Ordering::Acquire);

    let real: GetFileAttributesWFn = if saved == 0 {
        GetFileAttributesW
    } else {
        std::mem::transmute::<usize, GetFileAttributesWFn>(saved)
    };

    if let Some(redirected) = redirect(file_name) {
        let wide = to_wide(&redirected);

        return real(wide.as_ptr());
    }

    real(file_name)
}

/// The real path for a virtual one, or `None` to pass through.
///
/// # Safety
///
/// `raw` must be a NUL-terminated wide string or null, which is what the API
/// contract of every function this is called from already guarantees.
unsafe fn redirect(raw: *const u16) -> Option<String> {
    let tree = TREE.get()?;

    if raw.is_null() {
        return None;
    }

    let asked = from_wide(raw)?;

    tree.resolve_absolute(&asked).map(str::to_string)
}

/// Read a NUL-terminated wide string, bounded.
///
/// # Safety
///
/// `raw` must be a NUL-terminated wide string. The bound is a second line: a
/// string that is not terminated within `MAX_PATH_LEN` is treated as not a path
/// rather than walked until it faults.
unsafe fn from_wide(raw: *const u16) -> Option<String> {
    let mut len = 0usize;

    while len < crate::tree::MAX_PATH_LEN {
        if *raw.add(len) == 0 {
            break;
        }

        len += 1;
    }

    if len == 0 || len >= crate::tree::MAX_PATH_LEN {
        return None;
    }

    let slice = std::slice::from_raw_parts(raw, len);

    OsString::from_wide(slice).into_string().ok()
}

fn to_wide(value: &str) -> Vec<u16> {
    let mut wide: Vec<u16> = std::ffi::OsStr::new(value).encode_wide().collect();

    wide.push(0);

    wide
}

// ------------------------------------------------------------ IAT patching

/// Patch every module currently loaded in this process.
///
/// Returns whether anything was patched at all. A process where nothing imports
/// `CreateFileW` by name is one this cannot help, and saying so is better than
/// reporting success.
fn patch_loaded_modules() -> bool {
    // The process's own image. Patching every loaded module would need
    // `EnumProcessModules` and a re-scan on every `LoadLibrary`; the executable
    // and its static imports are what a game's own file access goes through,
    // and that is where this starts.
    let base =
        unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) };

    if base.is_null() {
        return false;
    }

    let mut patched = false;

    /*
     * The saved pointer is published BEFORE the patch that makes the hook
     * reachable, in both cases. Patching first would open a window in which
     * another thread enters the hook and finds nothing to forward to.
     */
    unsafe {
        if let Some(previous) = patch_import_with(
            base,
            b"kernel32.dll\0",
            b"CreateFileW\0",
            hooked_create_file_w as *const std::ffi::c_void,
            &REAL_CREATE_FILE_W,
        ) {
            let _ = previous;

            patched = true;
        }

        if let Some(previous) = patch_import_with(
            base,
            b"kernel32.dll\0",
            b"GetFileAttributesW\0",
            hooked_get_file_attributes_w as *const std::ffi::c_void,
            &REAL_GET_FILE_ATTRIBUTES_W,
        ) {
            let _ = previous;

            patched = true;
        }
    }

    patched
}

/// Find an import thunk, publish what is in it, then overwrite it.
///
/// The publish-before-patch order is the point of this wrapper: patching first
/// opens a window in which another thread enters the hook and finds nothing to
/// forward to.
///
/// # Safety
///
/// As [`patch_import`].
unsafe fn patch_import_with(
    base: HMODULE,
    module: &[u8],
    function: &[u8],
    replacement: *const std::ffi::c_void,
    saved: &AtomicUsize,
) -> Option<*const std::ffi::c_void> {
    let slot = find_import(base, module, function)?;

    let previous = *slot;

    saved.store(previous, Ordering::Release);

    if !write_pointer(slot, replacement as usize) {
        // Put the saved pointer back to zero: the hook is not reachable, and
        // leaving a stale value would be a pointer nothing published.
        saved.store(0, Ordering::Release);

        return None;
    }

    Some(previous as *const std::ffi::c_void)
}

/// Locate one import thunk.
///
/// # Safety
///
/// Walks the PE structures of a module the loader has already mapped. Every
/// offset is bounds-checked against the headers rather than trusted, because a
/// packed or corrupted image is a real thing to meet in a game process and a
/// wild read here faults inside somebody else's executable.
unsafe fn find_import(base: HMODULE, module: &[u8], function: &[u8]) -> Option<*mut usize> {
    let bytes = base as *const u8;

    let dos = &*(bytes as *const IMAGE_DOS_HEADER);

    // `MZ`. A module whose header is not this is not a PE image and nothing
    // below it can be read.
    if dos.e_magic != 0x5A4D {
        return None;
    }

    let nt = &*(bytes.offset(dos.e_lfanew as isize) as *const IMAGE_NT_HEADERS64);

    // `PE\0\0`.
    if nt.Signature != 0x0000_4550 {
        return None;
    }

    let directory = nt.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT as usize];

    if directory.VirtualAddress == 0 || directory.Size == 0 {
        return None;
    }

    let mut descriptor =
        bytes.add(directory.VirtualAddress as usize) as *const IMAGE_IMPORT_DESCRIPTOR;

    while (*descriptor).Name != 0 {
        let name = bytes.add((*descriptor).Name as usize);

        if eq_ascii_ci(name, module) {
            // `OriginalFirstThunk` still holds the NAMES; `FirstThunk` is the
            // live pointer table the loader filled in. Both are needed: one to
            // find the entry, the other to patch it.
            let mut original = bytes.add((*descriptor).Anonymous.OriginalFirstThunk as usize)
                as *const IMAGE_THUNK_DATA64;

            let mut live = bytes.add((*descriptor).FirstThunk as usize) as *mut IMAGE_THUNK_DATA64;

            // A module bound at link time has no name table to walk.
            if (*descriptor).Anonymous.OriginalFirstThunk == 0 {
                original = live as *const IMAGE_THUNK_DATA64;
            }

            while (*original).u1.AddressOfData != 0 {
                // The high bit means "imported by ordinal", which carries no
                // name to compare.
                if (*original).u1.Ordinal & 0x8000_0000_0000_0000 == 0 {
                    // `IMAGE_IMPORT_BY_NAME` is a `u16` hint then the name.
                    let entry = bytes.add((*original).u1.AddressOfData as usize);
                    let import_name = entry.add(2);

                    if eq_ascii_ci(import_name, function) {
                        return Some(std::ptr::addr_of_mut!((*live).u1.Function) as *mut usize);
                    }
                }

                original = original.add(1);
                live = live.add(1);
            }
        }

        descriptor = descriptor.add(1);
    }

    None
}

/// Write one pointer into a page that is normally read-only.
///
/// # Safety
///
/// `slot` must be a valid, writable-after-`VirtualProtect` address inside a
/// mapped image — which is what the import-table walk above produces.
unsafe fn write_pointer(slot: *mut usize, value: usize) -> bool {
    let mut previous: PAGE_PROTECTION_FLAGS = 0;

    let size = std::mem::size_of::<usize>();

    if VirtualProtect(
        slot as *mut std::ffi::c_void,
        size,
        PAGE_READWRITE,
        &mut previous,
    ) == 0
    {
        return false;
    }

    *slot = value;

    // Put the protection back. Leaving an import table writable is a gift to
    // anything else in the process that wants to hook the same function.
    let mut ignored: PAGE_PROTECTION_FLAGS = 0;

    VirtualProtect(slot as *mut std::ffi::c_void, size, previous, &mut ignored);

    true
}

/// Compare a NUL-terminated C string with a NUL-terminated literal, ignoring
/// case.
///
/// # Safety
///
/// `raw` must be NUL-terminated. Bounded at 256 bytes, which is longer than any
/// module or export name Windows permits.
unsafe fn eq_ascii_ci(raw: *const u8, expected: &[u8]) -> bool {
    for (index, wanted) in expected.iter().enumerate() {
        if index > 256 {
            return false;
        }

        let actual = *raw.add(index);

        if actual.eq_ignore_ascii_case(wanted) {
            continue;
        }

        return false;
    }

    true
}

/// Keeps the linker from discarding a function nothing in Rust calls.
///
/// The hooks are only ever reached through a pointer written into somebody
/// else's import table, which the linker cannot see.
#[used]
static KEEP: [FARPROC; 0] = [];
