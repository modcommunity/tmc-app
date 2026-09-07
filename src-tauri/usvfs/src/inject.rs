//! Starting a game with the hook DLL already inside it.
//!
//! **Windows only, and never run against a real game** — see the crate header.
//!
//! THE SEQUENCE, AND WHY IT IS THIS ONE
//! ------------------------------------
//! ```text
//!   CreateProcessW(CREATE_SUSPENDED)   the game exists but has run no code
//!   VirtualAllocEx                     room in it for one string
//!   WriteProcessMemory                 the DLL's path
//!   CreateRemoteThread(LoadLibraryW)   the loader maps our DLL
//!   WaitForSingleObject                until it has
//!   ResumeThread                       the game's own entry point runs
//! ```
//!
//! Suspended-then-inject rather than injecting into a running process, because
//! the hooks have to be in place before the game opens its first file — and a
//! game opens files during static initialisation, long before anything a
//! debugger could attach to.
//!
//! `LoadLibraryW` as the remote thread's start address is the standard trick
//! and it works because `kernel32.dll` is mapped at the same address in every
//! process on a given boot, so our `LoadLibraryW` pointer is also the child's.
//! That has been true since NT and stops being true only under a session-wide
//! ASLR mode nobody ships.
//!
//! WHAT THIS IS NOT
//! ----------------
//! It is not a way to run code in a process the user did not start. It launches
//! a program, from a path the launch rule resolved through the plugin jail,
//! with a DLL this app shipped. Nothing about it accepts a process id, and
//! nothing about it can attach to something already running — which is the
//! shape that would make it an injection tool rather than a launcher.
//!
//! ANTI-CHEAT
//! ----------
//! Every kernel-level anti-cheat treats this as an attack, correctly. A game
//! declaring `antiCheat: "kernel"` in its `sandbox.json` is refused the virtual
//! strategy before it reaches here — see `deploy::engine` — and the picker
//! warns for the linking strategies too.

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CreateRemoteThread, ResumeThread, TerminateProcess, WaitForSingleObject,
    CREATE_SUSPENDED, PROCESS_INFORMATION, STARTUPINFOW,
};

/// How long to wait for the remote `LoadLibrary` before giving up, in ms.
///
/// A DLL that has not loaded in ten seconds is one that is not going to. The
/// child is terminated rather than resumed, because resuming it would start the
/// game UNMODDED while the app reported that it had launched a sandbox — which
/// is worse than not launching at all.
const LOAD_TIMEOUT_MS: u32 = 10_000;

/// A launched, injected process.
pub struct Injected {
    pub process_id: u32,
}

/// What one launch needs.
pub struct LaunchSpec<'a> {
    /// The executable, absolute. Resolved through the plugin jail by the
    /// caller — nothing here accepts a path from anywhere else.
    pub program: &'a Path,
    /// Argv, already split. Joined and quoted here because `CreateProcessW`
    /// takes one string; there is no shell anywhere in this path.
    pub args: &'a [String],
    pub working_dir: Option<&'a Path>,
    /// `KEY=VALUE` pairs to add to the child's environment.
    pub env: &'a [(String, String)],
    /// The hook DLL to inject, absolute.
    pub dll: &'a Path,
}

/// Launch the program with the DLL loaded before its entry point runs.
pub fn launch(spec: &LaunchSpec<'_>) -> std::io::Result<Injected> {
    if !spec.dll.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "the virtual filesystem component is not installed",
        ));
    }

    let mut command = quote_command(spec.program, spec.args);
    let mut environment = build_environment(spec.env);

    let working = spec
        .working_dir
        .map(|dir| wide(dir.as_os_str()))
        .unwrap_or_default();

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;

    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            // `CREATE_UNICODE_ENVIRONMENT` because the block below is wide.
            CREATE_SUSPENDED | 0x0000_0400,
            environment.as_mut_ptr() as *mut std::ffi::c_void,
            if working.is_empty() {
                std::ptr::null()
            } else {
                working.as_ptr()
            },
            &startup,
            &mut info,
        )
    };

    if created == 0 {
        return Err(std::io::Error::last_os_error());
    }

    /*
     * From here on every failure TERMINATES the child. A suspended process left
     * behind is invisible in every task list a user reads and holds the game's
     * files open forever; and resuming it would start the game unmodded while
     * the app said it had launched a sandbox.
     */
    if let Err(err) = inject(info.hProcess, spec.dll) {
        unsafe {
            TerminateProcess(info.hProcess, 1);
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
        }

        return Err(err);
    }

    unsafe {
        ResumeThread(info.hThread);

        CloseHandle(info.hThread);
        CloseHandle(info.hProcess);
    }

    Ok(Injected {
        process_id: info.dwProcessId,
    })
}

/// Get the DLL into the (suspended) child.
fn inject(process: HANDLE, dll: &Path) -> std::io::Result<()> {
    let path = wide(dll.as_os_str());
    let bytes = path.len() * std::mem::size_of::<u16>();

    let remote = unsafe {
        VirtualAllocEx(
            process,
            std::ptr::null(),
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };

    if remote.is_null() {
        return Err(std::io::Error::last_os_error());
    }

    let written = unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory(
            process,
            remote,
            path.as_ptr() as *const std::ffi::c_void,
            bytes,
            std::ptr::null_mut(),
        )
    };

    if written == 0 {
        let err = std::io::Error::last_os_error();

        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };

        return Err(err);
    }

    /*
     * OUR `LoadLibraryW`, used as the child's. `kernel32.dll` is mapped at the
     * same address in every process on a given boot, which is what makes this
     * standard trick work at all.
     */
    let kernel = unsafe { GetModuleHandleW(wide(OsStr::new("kernel32.dll")).as_ptr()) };

    if kernel.is_null() {
        return Err(std::io::Error::other("kernel32 is not loaded"));
    }

    let loader = unsafe { GetProcAddress(kernel, c"LoadLibraryW".as_ptr() as *const u8) };

    let Some(loader) = loader else {
        return Err(std::io::Error::other("LoadLibraryW was not found"));
    };

    let thread = unsafe {
        CreateRemoteThread(
            process,
            std::ptr::null(),
            0,
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
            >(loader)),
            remote,
            0,
            std::ptr::null_mut(),
        )
    };

    if thread.is_null() {
        let err = std::io::Error::last_os_error();

        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };

        return Err(err);
    }

    let waited = unsafe { WaitForSingleObject(thread, LOAD_TIMEOUT_MS) };

    unsafe {
        CloseHandle(thread);
        VirtualFreeEx(process, remote, 0, MEM_RELEASE);
    }

    if waited != WAIT_OBJECT_0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "the virtual filesystem component did not load",
        ));
    }

    Ok(())
}

/// One command line from a program and its arguments.
///
/// `CreateProcessW` takes a single string and splits it by its own rules, so
/// every argument is quoted here. There is no shell involved at any point — the
/// quoting is about `CreateProcessW`'s parser and nothing else.
fn quote_command(program: &Path, args: &[String]) -> Vec<u16> {
    let mut line = String::new();

    line.push('"');
    line.push_str(&program.to_string_lossy());
    line.push('"');

    for arg in args {
        line.push(' ');
        line.push_str(&quote_argument(arg));
    }

    wide(OsStr::new(&line))
}

/// One argument, by `CommandLineToArgvW`'s rules.
///
/// The awkward rule: a backslash is literal EXCEPT immediately before a quote,
/// where it escapes. So a run of backslashes at the end of an argument has to
/// be doubled, or the closing quote it precedes is escaped and the argument
/// swallows everything after it.
fn quote_argument(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"', '\\']) {
        return arg.to_string();
    }

    let mut out = String::with_capacity(arg.len() + 2);

    out.push('"');

    let mut backslashes = 0usize;

    for ch in arg.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                out.push('\\');
            }
            '"' => {
                // Double the run, then escape the quote itself.
                for _ in 0..backslashes {
                    out.push('\\');
                }

                backslashes = 0;
                out.push('\\');
                out.push('"');
            }
            other => {
                backslashes = 0;
                out.push(other);
            }
        }
    }

    // The closing quote is preceded by the same run, so double it too.
    for _ in 0..backslashes {
        out.push('\\');
    }

    out.push('"');

    out
}

/// The child's environment: ours, plus the additions.
///
/// A wide, NUL-separated, double-NUL-terminated block, which is what
/// `CREATE_UNICODE_ENVIRONMENT` expects.
fn build_environment(extra: &[(String, String)]) -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();

    let overridden: Vec<String> = extra
        .iter()
        .map(|(key, _)| key.to_ascii_uppercase())
        .collect();

    for (key, value) in std::env::vars() {
        // An addition REPLACES rather than duplicates: a block with two entries
        // for one name has undefined behaviour depending on who reads it.
        if overridden.contains(&key.to_ascii_uppercase()) {
            continue;
        }

        block.extend(wide(OsStr::new(&format!("{key}={value}"))));
    }

    for (key, value) in extra {
        block.extend(wide(OsStr::new(&format!("{key}={value}"))));
    }

    // The terminating empty entry.
    block.push(0);

    block
}

/// A NUL-terminated wide string.
fn wide(value: &OsStr) -> Vec<u16> {
    let mut out: Vec<u16> = value.encode_wide().collect();

    out.push(0);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CommandLineToArgvW`'s backslash rule is the one thing here that is
    /// worth a test on its own: a run of backslashes at the end of an argument
    /// escapes the closing quote, and the argument then swallows everything
    /// after it.
    #[test]
    fn arguments_survive_command_line_to_argv() {
        assert_eq!(quote_argument("plain"), "plain");
        assert_eq!(quote_argument("with space"), "\"with space\"");
        assert_eq!(quote_argument(""), "\"\"");

        // A trailing backslash is doubled so the closing quote is not escaped.
        assert_eq!(quote_argument("C:\\path\\"), "\"C:\\path\\\\\"");

        // An embedded quote is escaped.
        assert_eq!(quote_argument("say \"hi\""), "\"say \\\"hi\\\"\"");

        // Backslashes before a quote are doubled AND the quote escaped.
        assert_eq!(quote_argument("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn an_environment_addition_replaces_rather_than_duplicates() {
        std::env::set_var("TMC_TEST_VFS_VAR", "original");

        let block =
            build_environment(&[("TMC_TEST_VFS_VAR".to_string(), "replacement".to_string())]);

        let text = String::from_utf16_lossy(&block);

        assert!(text.contains("TMC_TEST_VFS_VAR=replacement"));
        assert!(!text.contains("TMC_TEST_VFS_VAR=original"));

        std::env::remove_var("TMC_TEST_VFS_VAR");
    }

    #[test]
    fn a_command_line_quotes_the_program_and_every_argument() {
        let line = quote_command(
            Path::new("C:\\Games\\My Game\\game.exe"),
            &["--profile".into(), "Kitchen sink".into()],
        );

        let text = String::from_utf16_lossy(&line);

        assert!(text.starts_with("\"C:\\Games\\My Game\\game.exe\""));
        assert!(text.contains("\"Kitchen sink\""));
        assert!(text.contains("--profile"));
    }
}
