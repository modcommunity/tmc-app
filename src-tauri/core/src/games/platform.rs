//! Which build of a game this machine can actually run.
//!
//! ARCHITECTURE IS PART OF THE ANSWER, and that is the whole reason this is not
//! `std::env::consts::OS`. "Windows" does not identify a file somebody is about
//! to execute: an x64 archive handed to an ARM machine either refuses to start
//! or runs under emulation at a cost nobody chose, and the person it happens to
//! has no way to see why the game is slow.
//!
//! The values mirror `AppBuildPlatform` on the website, and the mapping is made
//! HERE rather than by the server, because the server cannot see this machine.
//! It is also why an unknown target answers `None` instead of guessing at the
//! closest thing: no build is a clear "nothing to install for you", and the
//! wrong build is a download that lands and then will not start.

use serde::{Deserialize, Serialize};

/// One target, as the API names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BuildPlatform {
    WindowsX64,
    WindowsArm64,
    LinuxX64,
    LinuxArm64,
    /// One universal binary covering Intel and Apple Silicon — the platform's
    /// own tooling produces the combined artifact, so there is no pair here.
    MacosUniversal,
    AndroidArm64,
    IosArm64,
}

impl BuildPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowsX64 => "WINDOWS_X64",
            Self::WindowsArm64 => "WINDOWS_ARM64",
            Self::LinuxX64 => "LINUX_X64",
            Self::LinuxArm64 => "LINUX_ARM64",
            Self::MacosUniversal => "MACOS_UNIVERSAL",
            Self::AndroidArm64 => "ANDROID_ARM64",
            Self::IosArm64 => "IOS_ARM64",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "WINDOWS_X64" => Self::WindowsX64,
            "WINDOWS_ARM64" => Self::WindowsArm64,
            "LINUX_X64" => Self::LinuxX64,
            "LINUX_ARM64" => Self::LinuxArm64,
            "MACOS_UNIVERSAL" => Self::MacosUniversal,
            "ANDROID_ARM64" => Self::AndroidArm64,
            "IOS_ARM64" => Self::IosArm64,
            _ => return None,
        })
    }

    /// Whether this app can put a build of its own on this platform's disk and
    /// start it.
    ///
    /// FALSE ON MOBILE, and that is a statement about the operating system
    /// rather than about work left to do. Android and iOS both refuse to
    /// execute code an app wrote into its own container; an `.apk` is installed
    /// by the OS package installer and an `.ipa` by the store. The enum carries
    /// those two targets because the SITE may publish them and the app should
    /// be able to say "there is a build, get it from the store" — which is a
    /// true and useful thing to say, and is not an install.
    pub fn installable(self) -> bool {
        !matches!(self, Self::AndroidArm64 | Self::IosArm64)
    }
}

/// This machine's target, or `None` for one nothing is published for.
///
/// Compile-time, from the target triple this binary was built for — not runtime
/// detection. The binary that is asking IS the evidence: a 64-bit x86 build of
/// the app running under Rosetta or under Windows-on-ARM emulation should ask
/// for the same architecture it is itself, because that is the architecture
/// whose emulation is already working on this machine.
pub const fn current() -> Option<BuildPlatform> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some(BuildPlatform::WindowsX64)
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        Some(BuildPlatform::WindowsArm64)
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some(BuildPlatform::LinuxX64)
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Some(BuildPlatform::LinuxArm64)
    }
    // Both Macs, one artifact — see `MacosUniversal`.
    #[cfg(target_os = "macos")]
    {
        Some(BuildPlatform::MacosUniversal)
    }
    #[cfg(target_os = "android")]
    {
        Some(BuildPlatform::AndroidArm64)
    }
    #[cfg(target_os = "ios")]
    {
        Some(BuildPlatform::IosArm64)
    }
    #[cfg(not(any(
        all(
            target_os = "windows",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        target_os = "macos",
        target_os = "android",
        target_os = "ios",
    )))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{current, BuildPlatform};

    #[test]
    fn every_target_round_trips_under_its_wire_name() {
        // The same check `QueryProtocol` carries, and for the same reason: the
        // variant names look identical to the wire names right up until a
        // serde rule splits one of them somewhere unexpected.
        for (variant, wire) in [
            (BuildPlatform::WindowsX64, "WINDOWS_X64"),
            (BuildPlatform::WindowsArm64, "WINDOWS_ARM64"),
            (BuildPlatform::LinuxX64, "LINUX_X64"),
            (BuildPlatform::LinuxArm64, "LINUX_ARM64"),
            (BuildPlatform::MacosUniversal, "MACOS_UNIVERSAL"),
            (BuildPlatform::AndroidArm64, "ANDROID_ARM64"),
            (BuildPlatform::IosArm64, "IOS_ARM64"),
        ] {
            assert_eq!(variant.as_str(), wire);
            assert_eq!(BuildPlatform::parse(wire), Some(variant));
            assert_eq!(
                serde_json::to_string(&variant).expect("serialise"),
                format!("\"{wire}\"")
            );
        }
    }

    #[test]
    fn an_unknown_target_is_not_guessed_at() {
        assert_eq!(BuildPlatform::parse("WINDOWS_X86"), None);
        assert_eq!(BuildPlatform::parse("linux_x64"), None);
    }

    #[test]
    fn the_test_runner_is_a_platform_we_publish_for() {
        // Tests run on the three desktop targets in CI. If this ever fails on a
        // machine somebody cares about, `current()` needs an arm rather than
        // the test needing a relaxation.
        let here = current().expect("this machine has a build target");

        assert!(here.installable());
    }
}
