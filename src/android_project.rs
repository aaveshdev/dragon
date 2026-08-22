use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const KOTLIN_BRIDGE: &str = include_str!("../templates/android/kotlin/DragonBridge.kt.template");
const KOTLIN_RENDERER: &str = include_str!("../templates/android/kotlin/DragonRenderer.kt.template");
const KOTLIN_THEME: &str = include_str!("../templates/android/kotlin/DragonTheme.kt.template");
const KOTLIN_MAIN_ACTIVITY: &str = include_str!("../templates/android/kotlin/MainActivity.kt.template");
const UI_DGN: &str = include_str!("../templates/android/ui.dgn.template");
const MAIN_DGN: &str = include_str!("../templates/android/main.dgn.template");
const ANDROID_MANIFEST: &str = include_str!("../templates/android/AndroidManifest.xml.template");
const ROOT_BUILD_GRADLE: &str = include_str!("../templates/android/build.gradle.kts.template");
const SETTINGS_GRADLE: &str = include_str!("../templates/android/settings.gradle.kts.template");
const GRADLE_PROPERTIES: &str = include_str!("../templates/android/gradle.properties.template");
const APP_BUILD_GRADLE: &str = include_str!("../templates/android/app_build.gradle.kts.template");
const STRINGS_XML: &str = include_str!("../templates/android/res/values/strings.xml.template");
const THEMES_XML: &str = include_str!("../templates/android/res/values/themes.xml.template");
const IC_LAUNCHER_BG: &str = include_str!("../templates/android/res/drawable/ic_launcher_background.xml");
const IC_LAUNCHER_FG: &str = include_str!("../templates/android/res/drawable/ic_launcher_foreground.xml");
const IC_LAUNCHER_ADAPTIVE: &str = include_str!("../templates/android/res/mipmap-anydpi-v26/ic_launcher.xml");

fn fill(template: &str, app_name: &str, package: &str) -> String {
    template
        .replace("__DRAGON_PACKAGE__", package)
        .replace("__DRAGON_APP_NAME__", app_name)
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("couldn't create {}: {}", parent.display(), e))?;
    }
    fs::write(path, content).map_err(|e| format!("couldn't write {}: {}", path.display(), e))
}

pub fn create_project(name: &str, package: &str) -> Result<(), String> {
    if !package.contains('.') {
        return Err(format!(
            "'{}' doesn't look like a package name (expected reverse-DNS style, e.g. com.example.{})",
            package, name
        ));
    }
    let root = PathBuf::from(name);
    if root.exists() {
        return Err(format!("'{}' already exists", root.display()));
    }

    let pkg_json = format!(
        "{{\n  \"name\": \"{}\",\n  \"package\": \"{}\",\n  \"platform\": \"android\",\n  \"main\": \"lib/main.dgn\",\n  \"androidDir\": \"android\"\n}}\n",
        name, package
    );
    write_file(&root.join("package.json"), &pkg_json)?;

    write_file(&root.join("lib/ui.dgn"), UI_DGN)?;
    write_file(&root.join("lib/main.dgn"), &fill(MAIN_DGN, name, package))?;

    let android = root.join("android");
    write_file(&android.join("build.gradle.kts"), ROOT_BUILD_GRADLE)?;
    write_file(&android.join("settings.gradle.kts"), &fill(SETTINGS_GRADLE, name, package))?;
    write_file(&android.join("gradle.properties"), GRADLE_PROPERTIES)?;

    let app = android.join("app");
    write_file(&app.join("build.gradle.kts"), &fill(APP_BUILD_GRADLE, name, package))?;
    write_file(&app.join("src/main/AndroidManifest.xml"), ANDROID_MANIFEST)?;
    write_file(&app.join("src/main/res/values/strings.xml"), &fill(STRINGS_XML, name, package))?;
    write_file(&app.join("src/main/res/values/themes.xml"), THEMES_XML)?;
    write_file(&app.join("src/main/res/drawable/ic_launcher_background.xml"), IC_LAUNCHER_BG)?;
    write_file(&app.join("src/main/res/drawable/ic_launcher_foreground.xml"), IC_LAUNCHER_FG)?;
    write_file(&app.join("src/main/res/mipmap-anydpi-v26/ic_launcher.xml"), IC_LAUNCHER_ADAPTIVE)?;

    let pkg_path = package.replace('.', "/");
    let java_dir = app.join("src/main/java").join(&pkg_path);
    write_file(&java_dir.join("DragonBridge.kt"), &fill(KOTLIN_BRIDGE, name, package))?;
    write_file(&java_dir.join("DragonRenderer.kt"), &fill(KOTLIN_RENDERER, name, package))?;
    write_file(&java_dir.join("DragonTheme.kt"), &fill(KOTLIN_THEME, name, package))?;
    write_file(&java_dir.join("MainActivity.kt"), &fill(KOTLIN_MAIN_ACTIVITY, name, package))?;

    for abi in ["arm64-v8a", "armeabi-v7a", "x86_64", "x86"] {
        fs::create_dir_all(app.join("src/main/jniLibs").join(abi))
            .map_err(|e| format!("couldn't create jniLibs/{}: {}", abi, e))?;
    }

    println!("Created {}/", root.display());
    println!("  lib/main.dgn       <- edit this to build your app");
    println!("  lib/ui.dgn         <- the widget library main.dgn imports");
    println!("  android/           <- generated project; shouldn't need editing day to day");
    println!();

    if system_gradle_available() {
        println!("Generating Gradle wrapper...");
        let wrapper_status = Command::new("gradle").arg("wrapper").current_dir(&android).status();
        match wrapper_status {
            Ok(s) if s.success() && gradlew_path(&android).exists() => {
                println!("  done -- this project now runs entirely from the CLI:");
                println!("Next: cd {} && dragon run", name);
            }
            _ => {
                println!("  wrapper generation didn't complete -- `dragon build`/`dragon run` will retry.");
                println!("Next: cd {} && dragon run", name);
            }
        }
    } else {
        println!("No system `gradle` found on PATH, so no wrapper could be generated yet.");
        println!("Install Gradle (e.g. `sdk install gradle` via sdkman, or `brew install gradle`)");
        println!("-- no Android Studio needed -- and `dragon build`/`dragon run` will generate the");
        println!("wrapper automatically on first use. (Opening {}/android/ in Android Studio once", name);
        println!("also works, if you'd rather do it that way.)");
        println!();
        println!("Next: cd {} && dragon run", name);
    }
    Ok(())
}


struct AndroidPackage {
    main: String,
    package: String,
    android_dir: String,
}

fn read_android_package(pkg_path: &Path) -> Result<AndroidPackage, String> {
    let data = fs::read_to_string(pkg_path)
        .map_err(|_| "'package.json' not found -- are you inside a Dragon project?".to_string())?;
    let json: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| format!("'package.json' contains invalid JSON: {}", e))?;
    let main = json.get("main").and_then(|v| v.as_str())
        .ok_or("'package.json' must contain a \"main\" field (e.g. \"lib/main.dgn\")")?
        .to_string();
    let package = json.get("package").and_then(|v| v.as_str())
        .ok_or("'package.json' must contain a \"package\" field for an Android project (e.g. \"com.example.myapp\")")?
        .to_string();
    let android_dir = json.get("androidDir").and_then(|v| v.as_str()).unwrap_or("android").to_string();
    Ok(AndroidPackage { main, package, android_dir })
}

pub fn is_android_project(pkg_path: &Path) -> bool {
    fs::read_to_string(pkg_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|j| j.get("platform").and_then(|v| v.as_str()).map(|s| s == "android"))
        .unwrap_or(false)
}


fn compile_native_lib(dragon_exe: &Path, pkg: &AndroidPackage) -> Result<PathBuf, String> {
    let main_dgn = PathBuf::from(&pkg.main);
    if !main_dgn.exists() {
        return Err(format!("'{}' not found (from package.json's \"main\" field)", pkg.main));
    }
    let jni_libs_dir = PathBuf::from(&pkg.android_dir).join("app/src/main/jniLibs");
    let out_stem = jni_libs_dir.parent().unwrap().join("applib"); 

    println!("Compiling {} for Android...", pkg.main);
    let status = Command::new(dragon_exe)
        .args([
            "-c", &pkg.main,
            "-o", out_stem.to_str().unwrap(),
            "--native", "--lib",
            "--target", "android-all",
            "--jni-package", &pkg.package,
            "--jni-class", "DragonBridge",
        ])
        .status()
        .map_err(|e| format!("failed to launch dragon compiler: {}", e))?;
    if !status.success() {
        return Err("native compilation failed (see output above)".into());
    }
    Ok(jni_libs_dir)
}

fn gradlew_path(android_dir: &Path) -> PathBuf {
    if cfg!(target_os = "windows") { android_dir.join("gradlew.bat") } else { android_dir.join("gradlew") }
}

fn system_gradle_available() -> bool {
    Command::new("gradle").arg("-v").output().map(|o| o.status.success()).unwrap_or(false)
}


fn resolve_gradle(android_dir: &Path) -> Result<PathBuf, String> {
    let gradlew = gradlew_path(android_dir);
    if gradlew.exists() {
        return Ok(gradlew);
    }
    if system_gradle_available() {
        println!("No Gradle wrapper yet -- generating one with your system `gradle`...");
        let wrapper_status = Command::new("gradle")
            .arg("wrapper")
            .current_dir(android_dir)
            .status()
            .map_err(|e| format!("found `gradle` on PATH but failed to run it: {}", e))?;
        if wrapper_status.success() && gradlew.exists() {
            return Ok(gradlew);
        }

        return Ok(PathBuf::from("gradle"));
    }
    Err(format!(
        "no Gradle available: no wrapper at {}, and no working `gradle` found on PATH. Install \
         Gradle (e.g. `sdk install gradle` via sdkman, or `brew install gradle`) and re-run -- \
         no Android Studio needed. Once Gradle is installed, `dragon build`/`dragon run` will \
         automatically generate this project's own wrapper on the next attempt.",
        gradlew.display()
    ))
}

pub fn build_android(dragon_exe: &Path, release: bool) -> Result<PathBuf, String> {
    let pkg = read_android_package(Path::new("package.json"))?;
    compile_native_lib(dragon_exe, &pkg)?;

    let android_dir = PathBuf::from(&pkg.android_dir);
    let gradle_cmd = resolve_gradle(&android_dir)?;
    let task = if release { "assembleRelease" } else { "assembleDebug" };

    let display_cmd = if gradle_cmd.is_absolute() || gradle_cmd.components().count() > 1 {
        format!("./{}", gradle_cmd.file_name().unwrap().to_string_lossy())
    } else {
        gradle_cmd.to_string_lossy().to_string()
    };
    println!("Running {} {} ...", display_cmd, task);
    let status = Command::new(&gradle_cmd)
        .arg(task)
        .current_dir(&android_dir)
        .status()
        .map_err(|e| format!("failed to launch Gradle ({}): {}", gradle_cmd.display(), e))?;
    if !status.success() {
        return Err("Gradle build failed (see output above)".into());
    }

    let variant_dir = if release { "release" } else { "debug" };
    let apk_dir = android_dir.join("app/build/outputs/apk").join(variant_dir);
    let apk_path = apk_dir.join(format!("app-{}.apk", variant_dir));
    if apk_path.exists() {
        println!("Built: {}", apk_path.display());
        Ok(apk_path)
    } else {
        println!("Gradle reported success but the expected APK path wasn't found.");
        println!("Check under: {}", apk_dir.display());
        Ok(apk_dir)
    }
}

pub fn run_android(dragon_exe: &Path) -> Result<(), String> {
    let pkg = read_android_package(Path::new("package.json"))?;
    let apk_path = build_android(dragon_exe, false)?;
    if !apk_path.extension().map(|e| e == "apk").unwrap_or(false) {
        return Err(format!("build succeeded but no single APK was found under {} -- install it manually", apk_path.display()));
    }

    let devices_out = Command::new("adb")
        .arg("devices")
        .output()
        .map_err(|e| format!("failed to launch adb (is it on your PATH?): {}", e))?;
    let devices_text = String::from_utf8_lossy(&devices_out.stdout);
    // `adb devices` output: a header line, then "<serial>\tdevice" per
    // attached device/emulator (also "\toffline"/"\tunauthorized" for a
    // device that isn't ready -- only "\tdevice" is usable).
    let device_serial = devices_text
        .lines()
        .skip(1)
        .find(|l| l.ends_with("\tdevice"))
        .and_then(|l| l.split('\t').next())
        .map(|s| s.to_string());

    let serial = match device_serial {
        Some(s) => s,
        None => {
            return Err(
                "no connected device or running emulator found (`adb devices` returned none ready). \
                 Start an emulator (Android Studio's Device Manager, or `emulator -avd <name>`) or \
                 connect a device with USB debugging enabled, then try again.".into()
            );
        }
    };
    println!("Installing on {}...", serial);

    let install_status = Command::new("adb")
        .args(["-s", &serial, "install", "-r"])
        .arg(&apk_path)
        .status()
        .map_err(|e| format!("failed to launch adb install: {}", e))?;
    if !install_status.success() {
        return Err("adb install failed (see output above)".into());
    }

    let component = format!("{}/{}.MainActivity", pkg.package, pkg.package);
    println!("Launching {}...", component);
    let start_status = Command::new("adb")
        .args(["-s", &serial, "shell", "am", "start", "-n", &component])
        .status()
        .map_err(|e| format!("failed to launch adb shell am start: {}", e))?;
    if !start_status.success() {
        return Err("adb shell am start failed (see output above) -- the app installed but didn't launch".into());
    }

    println!("Running on {}.", serial);
    Ok(())
}
