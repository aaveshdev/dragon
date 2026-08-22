use std::env;
use std::path::PathBuf;
use std::process::Command;


#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CrossTarget {
    AndroidArm64,   // arm64-v8a   — required, virtually all modern devices
    AndroidArmv7,   // armeabi-v7a — older 32-bit devices
    AndroidX86_64,  // x86_64      — emulator
    AndroidX86,     // x86         — legacy emulator
    IosArm64,           // physical device
    IosArm64Simulator,  // Apple Silicon simulator
    IosX86_64Simulator, // Intel simulator
}

impl CrossTarget {
    /// Parses a comma-separated `--target` spec, expanding the group
    /// aliases `android-all`, `ios-all`, and `all`.
    pub fn parse_list(spec: &str) -> Result<Vec<CrossTarget>, String> {
        let mut out = Vec::new();
        for tok in spec.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            match tok {
                "android-all" => out.extend_from_slice(&[
                    CrossTarget::AndroidArm64,
                    CrossTarget::AndroidArmv7,
                    CrossTarget::AndroidX86_64,
                    CrossTarget::AndroidX86,
                ]),
                "ios-all" => out.extend_from_slice(&[
                    CrossTarget::IosArm64,
                    CrossTarget::IosArm64Simulator,
                    CrossTarget::IosX86_64Simulator,
                ]),
                "all" => {
                    out.extend(Self::parse_list("android-all")?);
                    out.extend(Self::parse_list("ios-all")?);
                }
                "android-arm64" | "android64" => out.push(CrossTarget::AndroidArm64),
                "android-armv7" | "android-arm" => out.push(CrossTarget::AndroidArmv7),
                "android-x86_64" | "android-x64" => out.push(CrossTarget::AndroidX86_64),
                "android-x86" => out.push(CrossTarget::AndroidX86),
                "ios-arm64" | "ios-device" | "ios" => out.push(CrossTarget::IosArm64),
                "ios-arm64-sim" | "ios-sim-arm64" => out.push(CrossTarget::IosArm64Simulator),
                "ios-x86_64-sim" | "ios-sim-x86_64" => out.push(CrossTarget::IosX86_64Simulator),
                other => {
                    return Err(format!(
                        "Unknown --target '{}'. Valid values: android-arm64, android-armv7, \
                         android-x86_64, android-x86, android-all, ios-arm64, ios-arm64-sim, \
                         ios-x86_64-sim, ios-all, all",
                        other
                    ));
                }
            }
        }
        // De-duplicate while preserving order (e.g. "android-all,android-arm64").
        let mut seen = Vec::new();
        for t in out {
            if !seen.contains(&t) {
                seen.push(t);
            }
        }
        Ok(seen)
    }

    pub fn is_android(self) -> bool {
        matches!(
            self,
            CrossTarget::AndroidArm64
                | CrossTarget::AndroidArmv7
                | CrossTarget::AndroidX86_64
                | CrossTarget::AndroidX86
        )
    }

    pub fn is_ios(self) -> bool {
        !self.is_android()
    }

    /// Directory name Android Studio / Gradle expects under `jniLibs/`.
    pub fn android_abi_dir(self) -> &'static str {
        match self {
            CrossTarget::AndroidArm64 => "arm64-v8a",
            CrossTarget::AndroidArmv7 => "armeabi-v7a",
            CrossTarget::AndroidX86_64 => "x86_64",
            CrossTarget::AndroidX86 => "x86",
            _ => unreachable!("not an Android target"),
        }
    }

    /// NDK clang driver name, minus the `<api-level>` infix which is
    /// inserted by `ndk_clang_path`.
    fn android_triple_prefix(self) -> &'static str {
        match self {
            CrossTarget::AndroidArm64 => "aarch64-linux-android",
            CrossTarget::AndroidArmv7 => "armv7a-linux-androideabi",
            CrossTarget::AndroidX86_64 => "x86_64-linux-android",
            CrossTarget::AndroidX86 => "i686-linux-android",
            _ => unreachable!("not an Android target"),
        }
    }

    /// Apple `-target` triple, e.g. `arm64-apple-ios15.0` or
    /// `arm64-apple-ios15.0-simulator`.
    pub fn ios_clang_target(self, min_version: &str) -> String {
        match self {
            CrossTarget::IosArm64 => format!("arm64-apple-ios{}", min_version),
            CrossTarget::IosArm64Simulator => format!("arm64-apple-ios{}-simulator", min_version),
            CrossTarget::IosX86_64Simulator => {
                format!("x86_64-apple-ios{}-simulator", min_version)
            }
            _ => unreachable!("not an iOS target"),
        }
    }

    pub fn ios_arch(self) -> &'static str {
        match self {
            CrossTarget::IosArm64 | CrossTarget::IosArm64Simulator => "arm64",
            CrossTarget::IosX86_64Simulator => "x86_64",
            _ => unreachable!("not an iOS target"),
        }
    }

    pub fn ios_sdk(self) -> &'static str {
        match self {
            CrossTarget::IosArm64 => "iphoneos",
            CrossTarget::IosArm64Simulator | CrossTarget::IosX86_64Simulator => "iphonesimulator",
            _ => unreachable!("not an iOS target"),
        }
    }

    /// True for the two simulator slices, which Apple requires to be
    /// merged into one fat `.a` before packaging into an xcframework
    /// (an xcframework may only contain one library per platform variant).
    pub fn is_ios_simulator(self) -> bool {
        matches!(
            self,
            CrossTarget::IosArm64Simulator | CrossTarget::IosX86_64Simulator
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            CrossTarget::AndroidArm64 => "android-arm64",
            CrossTarget::AndroidArmv7 => "android-armv7",
            CrossTarget::AndroidX86_64 => "android-x86_64",
            CrossTarget::AndroidX86 => "android-x86",
            CrossTarget::IosArm64 => "ios-arm64",
            CrossTarget::IosArm64Simulator => "ios-arm64-sim",
            CrossTarget::IosX86_64Simulator => "ios-x86_64-sim",
        }
    }
}

/// Locates the Android NDK's prebuilt LLVM toolchain and returns the path
/// to the per-ABI `clang` wrapper (e.g. `aarch64-linux-android24-clang`).
///
/// Resolution order: `$ANDROID_NDK_HOME`, then `$ANDROID_NDK_ROOT`,
/// then `$ANDROID_HOME/ndk/<latest>`.
pub fn ndk_clang_path(target: CrossTarget, api_level: u32) -> Result<PathBuf, String> {
    let ndk_root = env::var("ANDROID_NDK_HOME")
        .or_else(|_| env::var("ANDROID_NDK_ROOT"))
        .ok()
        .map(PathBuf::from)
        .or_else(|| find_latest_ndk_under_sdk())
        .ok_or_else(|| {
            "Android NDK not found. Set ANDROID_NDK_HOME to your NDK install \
             (e.g. $ANDROID_HOME/ndk/26.1.10909125), or install it via \
             `sdkmanager --install \"ndk;26.1.10909125\"`."
                .to_string()
        })?;

    let host_tag = if cfg!(target_os = "linux") {
        "linux-x86_64"
    } else if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else {
        return Err("Unsupported host OS for Android NDK cross-compilation.".into());
    };

    let bin_dir = ndk_root
        .join("toolchains")
        .join("llvm")
        .join("prebuilt")
        .join(host_tag)
        .join("bin");

    let exe_suffix = if cfg!(target_os = "windows") { ".cmd" } else { "" };
    let clang_name = format!(
        "{}{}-clang{}",
        target.android_triple_prefix(),
        api_level,
        exe_suffix
    );
    let clang_path = bin_dir.join(&clang_name);

    if !clang_path.exists() {
        return Err(format!(
            "NDK clang not found at {}. Check that ANDROID_NDK_HOME ({}) points at a valid \
             NDK install and that api-level {} is supported by it (try a lower level with \
             --android-api).",
            clang_path.display(),
            ndk_root.display(),
            api_level
        ));
    }

    Ok(clang_path)
}

fn find_latest_ndk_under_sdk() -> Option<PathBuf> {
    let sdk_root = env::var("ANDROID_HOME").or_else(|_| env::var("ANDROID_SDK_ROOT")).ok()?;
    let ndk_dir = PathBuf::from(sdk_root).join("ndk");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&ndk_dir).ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    versions.pop()
}

/// Resolves the Apple `clang` driver via `xcrun` and the SDK sysroot path
/// for the given iOS slice. Building for iOS is only possible from macOS —
/// this is an Apple SDK/toolchain restriction, not something a `--target`
/// flag can work around.
pub fn ios_clang_and_sysroot(target: CrossTarget) -> Result<(PathBuf, PathBuf), String> {
    if !cfg!(target_os = "macos") {
        return Err(format!(
            "Building the '{}' slice requires macOS with Xcode command line tools installed \
             (Apple only ships the iOS SDK for macOS). Run this build on a Mac, or build the \
             Android slices here and the iOS slices separately on a Mac CI runner.",
            target.label()
        ));
    }

    let sdk = target.ios_sdk();

    let clang_out = Command::new("xcrun")
        .args(["--sdk", sdk, "--find", "clang"])
        .output()
        .map_err(|e| format!("Failed to run xcrun: {}", e))?;
    if !clang_out.status.success() {
        return Err(format!(
            "xcrun could not find clang for SDK '{}'. Run `xcode-select --install` and make \
             sure Xcode (not just the CLT) is selected via `xcode-select -s`.",
            sdk
        ));
    }
    let clang_path = PathBuf::from(String::from_utf8_lossy(&clang_out.stdout).trim());

    let sdk_out = Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .map_err(|e| format!("Failed to run xcrun: {}", e))?;
    if !sdk_out.status.success() {
        return Err(format!("xcrun could not locate the '{}' SDK path.", sdk));
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sdk_out.stdout).trim());

    Ok((clang_path, sysroot))
}
