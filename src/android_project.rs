use std::collections::BTreeSet;
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

const VM_NATIVE_KT: &str = include_str!("../templates/android_vm/kotlin/DragonNative.kt");
const VM_PLATFORM_KT: &str = include_str!("../templates/android_vm/kotlin/DragonPlatform.kt");
const VM_PLUGIN_KT: &str = include_str!("../templates/android_vm/kotlin/DragonPlugin.kt");
const VM_HOST_KT: &str = include_str!("../templates/android_vm/kotlin/DragonHost.kt");
const VM_RENDERER_KT: &str = include_str!("../templates/android_vm/kotlin/DragonRenderer.kt");
const VM_MAIN_ACTIVITY: &str = include_str!("../templates/android_vm/MainActivity.kt.template");
const VM_MANIFEST: &str = include_str!("../templates/android_vm/AndroidManifest.xml.template");
const VM_APP_GRADLE: &str = include_str!("../templates/android_vm/app_build.gradle.kts.template");
const VM_MAIN_DGN: &str = include_str!("../templates/android_vm/main.dgn.template");
const VM_HOME_DGN: &str = include_str!("../templates/android_vm/home_page.dgn.template");
const ICON_FONT: &[u8] = include_bytes!("../framework/fonts/MaterialIcons-Regular.ttf");

const ANDROID_GITIGNORE: &str = "*.iml\n.gradle\n/local.properties\n/.idea\n.DS_Store\n/build\n/captures\n.externalNativeBuild\n.cxx\nlocal.properties\napp/build\napp/src/main/jniLibs/\n";

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

    write_file(&android.join(".gitignore"), ANDROID_GITIGNORE)?;

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
    runtime: String,
    abis: Vec<String>,
    permissions: Vec<serde_json::Value>,
    gradle: Vec<String>,
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
    let runtime = json.get("runtime").and_then(|v| v.as_str()).unwrap_or("native").to_string();
    let abis = json.get("abis").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec!["arm64-v8a".to_string(), "x86_64".to_string()]);
    let android_obj = json.get("android");
    let permissions = android_obj.and_then(|a| a.get("permissions")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let gradle = android_obj.and_then(|a| a.get("gradle")).and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    Ok(AndroidPackage { main, package, android_dir, runtime, abis, permissions, gradle })
}

pub fn is_android_project(pkg_path: &Path) -> bool {
    fs::read_to_string(pkg_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .map(|j| (j.get("platform").is_some() || j.get("platforms").is_some()) && crate::platforms::declared_in(&j).iter().any(|p| p == "android"))
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
    let android_dir = PathBuf::from(&pkg.android_dir);
    if pkg.runtime == "app" || pkg.runtime == "vm" {
        sync_app_project(&pkg, &android_dir)?;
        build_native_app_libs(&pkg, &android_dir)?;
    } else {
        compile_native_lib(dragon_exe, &pkg)?;
    }

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

fn snapshot_lib_mtimes(dir: &Path, out: &mut std::collections::HashMap<PathBuf, std::time::SystemTime>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            snapshot_lib_mtimes(&path, out);
        } else if path.extension().map(|e| e == "dgn").unwrap_or(false) {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    out.insert(path, modified);
                }
            }
        }
    }
}

pub fn watch_android(dragon_exe: &Path) -> Result<(), String> {
    let lib_dir = Path::new("lib");
    if !lib_dir.exists() {
        return Err("no 'lib' directory found -- are you inside a Dragon project?".into());
    }

    println!("Watch mode: rebuilding and relaunching on every change under lib/.");
    println!("Ctrl+C to stop.");
    println!();

    let mut last = std::collections::HashMap::new();
    snapshot_lib_mtimes(lib_dir, &mut last);

    match run_android(dragon_exe) {
        Ok(_) => {}
        Err(e) => eprintln!("Error: {}", e),
    }

    loop {
        std::thread::sleep(std::time::Duration::from_millis(600));
        let mut current = std::collections::HashMap::new();
        snapshot_lib_mtimes(lib_dir, &mut current);

        if current != last {
            println!();
            println!("Change detected -- rebuilding...");
            last = current;
            match run_android(dragon_exe) {
                Ok(_) => println!("Reinstalled and relaunched (full restart -- app state was reset)."),
                Err(e) => eprintln!("Error: {}", e),
            }
            println!();
            println!("Watching for more changes...");
        }
    }
}

fn fill_abis(template: &str, abis: &[String]) -> String {
    let list = abis.iter().map(|a| format!("\"{}\"", a)).collect::<Vec<_>>().join(", ");
    template.replace("__DRAGON_ABIS__", &list)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("couldn't create {}: {}", parent.display(), e))?;
    }
    fs::write(path, bytes).map_err(|e| format!("couldn't write {}: {}", path.display(), e))
}

fn write_vm_runtime(android: &Path) -> Result<(), String> {
    let rt = android.join("app/src/main/java/org/dragonlang/runtime");
    write_file(&rt.join("DragonNative.kt"), VM_NATIVE_KT)?;
    write_file(&rt.join("DragonPlatform.kt"), VM_PLATFORM_KT)?;
    write_file(&rt.join("DragonPlugin.kt"), VM_PLUGIN_KT)?;
    write_file(&rt.join("DragonHost.kt"), VM_HOST_KT)?;
    write_file(&rt.join("DragonRenderer.kt"), VM_RENDERER_KT)?;
    write_bytes(&android.join("app/src/main/assets/dragon_fonts/MaterialIcons-Regular.ttf"), ICON_FONT)?;
    for stale in ["DragonNsfw.kt", "DragonAdBlock.kt", "DragonAudio.kt", "DragonAudioService.kt", "DragonWeb.kt"] {
        let _ = fs::remove_file(rt.join(stale));
    }
    let _ = fs::remove_dir_all(android.join("app/src/main/assets/dragon_web"));
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path, only_ext: Option<&str>) -> Result<(), String> {
    let entries = match fs::read_dir(src) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let out = dst.join(&name);
        if path.is_dir() {
            copy_tree(&path, &out, only_ext)?;
        } else {
            if let Some(ext) = only_ext {
                if path.extension().map(|e| e != ext).unwrap_or(true) {
                    continue;
                }
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&path, &out).map_err(|e| format!("couldn't copy {}: {}", path.display(), e))?;
        }
    }
    Ok(())
}

fn sync_app_project(pkg: &AndroidPackage, android: &Path) -> Result<(), String> {
    let main_path = Path::new(&pkg.main);
    if !main_path.exists() {
        return Err(format!("'{}' not found (from package.json's \"main\" field)", pkg.main));
    }
    write_vm_runtime(android)?;
    sync_packages(pkg, android, Path::new("."))?;
    sync_native_extras(android, Path::new("native"))?;
    let assets = android.join("app/src/main/assets");
    let _ = fs::remove_dir_all(assets.join("dragon"));
    copy_tree(Path::new("assets"), &assets, None)?;
    Ok(())
}

fn sync_native_extras(android: &Path, native: &Path) -> Result<(), String> {
    let record = android.join(".dragon_native_files");
    if let Ok(old) = fs::read_to_string(&record) {
        for line in old.lines().filter(|l| !l.trim().is_empty()) {
            let _ = fs::remove_file(android.join(line.trim()));
        }
    }
    let mut copied: Vec<String> = Vec::new();
    let rt_rel = "app/src/main/java/org/dragonlang/runtime";
    let assets_rel = "app/src/main/assets";
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, base, out); }
                else if let Ok(rel) = p.strip_prefix(base) { out.push((p.clone(), rel.to_path_buf())); }
            }
        }
    }
    let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
    let kt = native.join("kotlin");
    if kt.is_dir() {
        let mut v = Vec::new();
        walk(&kt, &kt, &mut v);
        for (src, rel) in v {
            if src.extension().map(|e| e == "kt").unwrap_or(false) {
                let name = rel.file_name().map(|n| n.to_owned()).unwrap_or_default();
                files.push((src, Path::new(rt_rel).join(name)));
            }
        }
    }
    let assets = native.join("assets");
    if assets.is_dir() {
        let mut v = Vec::new();
        walk(&assets, &assets, &mut v);
        for (src, rel) in v { files.push((src, Path::new(assets_rel).join(rel))); }
    }
    for (src, dst_rel) in files {
        let bytes = fs::read(&src).map_err(|e| format!("couldn't read {}: {}", src.display(), e))?;
        write_bytes(&android.join(&dst_rel), &bytes)?;
        copied.push(dst_rel.to_string_lossy().replace('\\', "/"));
    }
    if copied.is_empty() {
        let _ = fs::remove_file(&record);
    } else {
        write_file(&record, &(copied.join("\n") + "\n"))?;
        println!("  [android] native/ extras copied: {} files", copied.len());
    }
    Ok(())
}

fn sync_packages(pkg: &AndroidPackage, android: &Path, app_dir: &Path) -> Result<(), String> {
    let record = android.join(".dragon_plugin_files");
    if let Ok(old) = fs::read_to_string(&record) {
        for line in old.lines().filter(|l| !l.trim().is_empty()) {
            let _ = fs::remove_file(android.join(line.trim()));
        }
    }
    let used = crate::packages::used_packages(app_dir);
    let java_rel = Path::new("app/src/main/java");
    let assets_rel = Path::new("app/src/main/assets");
    let mut copied: Vec<String> = Vec::new();
    let mut plugins: Vec<String> = Vec::new();
    let mut perms: Vec<serde_json::Value> = Vec::new();
    let mut xml: Vec<String> = Vec::new();
    let mut deps: Vec<String> = Vec::new();
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, base, out); }
                else if let Ok(rel) = p.strip_prefix(base) { out.push((p.clone(), rel.to_path_buf())); }
            }
        }
    }
    for p in &used {
        let native = p.root.join("native/android");
        let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (sub, dest) in [("kotlin", java_rel), ("assets", assets_rel)] {
            let base = native.join(sub);
            let mut v = Vec::new();
            walk(&base, &base, &mut v);
            for (src, rel) in v {
                if sub == "kotlin" && src.extension().map(|e| e != "kt").unwrap_or(true) { continue; }
                files.push((src, dest.join(rel)));
            }
        }
        for (src, dst_rel) in files {
            let bytes = fs::read(&src).map_err(|e| format!("package {}: couldn't read {}: {}", p.name, src.display(), e))?;
            write_bytes(&android.join(&dst_rel), &bytes)?;
            copied.push(dst_rel.to_string_lossy().replace('\\', "/"));
        }
        if let Some(c) = p.android_str("plugin") { plugins.push(c); }
        if let Some(a) = p.manifest.get("android").and_then(|a| a.get("permissions")).and_then(|v| v.as_array()) { perms.extend(a.iter().cloned()); }
        if let Some(m) = p.android_str("manifest") {
            let f = p.root.join(&m);
            let text = fs::read_to_string(&f).map_err(|e| format!("package {}: couldn't read manifest snippet {}: {}", p.name, f.display(), e))?;
            xml.push(text);
        }
        deps.extend(p.android_list("gradle"));
    }
    perms.extend(pkg.permissions.iter().cloned());
    deps.extend(pkg.gradle.iter().cloned());
    write_plugin_registry(android, &plugins)?;
    rewrite_manifest_and_gradle(android, &perms, &xml, &deps)?;
    if copied.is_empty() {
        let _ = fs::remove_file(&record);
    } else {
        write_file(&record, &(copied.join("\n") + "\n"))?;
    }
    if !used.is_empty() {
        println!("  [android] packages: {}", used.iter().map(|p| format!("{} {}", p.name, p.version())).collect::<Vec<_>>().join(", "));
    }
    Ok(())
}

fn write_plugin_registry(android: &Path, plugins: &[String]) -> Result<(), String> {
    let list = if plugins.is_empty() {
        "listOf()".to_string()
    } else {
        format!("listOf(\n{}\n    )", plugins.iter().map(|c| format!("        {}(ctx)", c)).collect::<Vec<_>>().join(",\n"))
    };
    let kt = format!(
        "// Generated by `dragon` from the packages this app imports. Do not edit.\npackage org.dragonlang.runtime\n\nobject DragonPlugins {{\n    fun create(@Suppress(\"UNUSED_PARAMETER\") ctx: PluginContext): List<DragonPlugin> = {}\n}}\n",
        list);
    write_file(&android.join("app/src/main/java/org/dragonlang/runtime/DragonPlugins.kt"), &kt)
}

fn perm_line(v: &serde_json::Value) -> Option<(String, String)> {
    match v {
        serde_json::Value::String(s) => Some((s.clone(), format!("<uses-permission android:name=\"{}\" />", s))),
        serde_json::Value::Object(o) => {
            let name = o.get("name")?.as_str()?.to_string();
            let max = o.get("maxSdkVersion").and_then(|m| m.as_i64())
                .map(|m| format!(" android:maxSdkVersion=\"{}\"", m)).unwrap_or_default();
            Some((name.clone(), format!("<uses-permission android:name=\"{}\"{} />", name, max)))
        }
        _ => None,
    }
}

fn replace_block(text: &str, begin: &str, end: &str, body: &str, anchor: impl Fn(&str) -> Option<usize>) -> String {
    let mut t = text.to_string();
    if !(t.contains(begin) && t.contains(end)) {
        if let Some(at) = anchor(&t) {
            t.insert_str(at, &format!("{}\n{}\n", begin, end));
        } else {
            return t;
        }
    }
    let b = t.find(begin).unwrap();
    let b_line_end = t[b..].find('\n').map(|n| b + n + 1).unwrap_or(t.len());
    let e = t.find(end).unwrap();
    let e_line_start = t[..e].rfind('\n').map(|n| n + 1).unwrap_or(0);
    let indent: String = t[e_line_start..e].chars().take_while(|c| c.is_whitespace()).collect();
    let mut out = String::new();
    out.push_str(&t[..b_line_end]);
    for l in body.lines() { out.push_str(&format!("{}{}\n", indent, l)); }
    out.push_str(&t[e_line_start..]);
    out
}

fn rewrite_manifest_and_gradle(android: &Path, perms: &[serde_json::Value], app_xml: &[String], deps: &[String]) -> Result<(), String> {
    let manifest = android.join("app/src/main/AndroidManifest.xml");
    if let Ok(m) = fs::read_to_string(&manifest) {
        let mut m2 = m.clone();
        while let Some(i) = m2.find("android:name=\"org.dragonlang.runtime.DragonAudioService\"") {
            let start = m2[..i].rfind("<service").unwrap_or(i);
            let line_start = m2[..start].rfind('\n').map(|n| n + 1).unwrap_or(0);
            let end = m2[i..].find("</service>").map(|n| i + n + "</service>".len()).unwrap_or(m2.len());
            let end = m2[end..].find('\n').map(|n| end + n + 1).unwrap_or(m2.len());
            m2.replace_range(line_start..end, "");
        }
        let legacy_perms = ["READ_MEDIA_AUDIO", "FOREGROUND_SERVICE", "WAKE_LOCK", "READ_EXTERNAL_STORAGE"];
        m2 = m2.lines().filter(|l| !(l.contains("<uses-permission") && legacy_perms.iter().any(|p| l.contains(p)))).collect::<Vec<_>>().join("\n") + "\n";

        let outside_perms = {
            let mut o = m2.clone();
            if let (Some(b), Some(e)) = (o.find("<!-- dragon:permissions:begin -->"), o.find("<!-- dragon:permissions:end -->")) {
                if e > b { o.replace_range(b..e, ""); }
            }
            o
        };
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut lines: Vec<String> = Vec::new();
        for v in perms {
            if let Some((name, line)) = perm_line(v) {
                if outside_perms.contains(&format!("android:name=\"{}\"", name)) { continue; }
                if seen.insert(name) { lines.push(line); }
            }
        }
        let m3 = replace_block(&m2, "<!-- dragon:permissions:begin -->", "<!-- dragon:permissions:end -->", &lines.join("\n"),
            |t| t.find("<application").map(|i| t[..i].rfind('\n').map(|n| n + 1).unwrap_or(0)));
        let body = app_xml.iter().map(|x| x.trim_end().to_string()).collect::<Vec<_>>().join("\n");
        let m4 = replace_block(&m3, "<!-- dragon:application:begin -->", "<!-- dragon:application:end -->", &body,
            |t| t.rfind("</application>").map(|i| t[..i].rfind('\n').map(|n| n + 1).unwrap_or(0)));
        if m4 != m { write_file(&manifest, &m4)?; }
    }
    let gradle = android.join("app/build.gradle.kts");
    if let Ok(g) = fs::read_to_string(&gradle) {
        let g2 = g.lines().filter(|l| !(l.contains("androidx.media3") || l.contains("androidx.webkit:webkit"))
            || (l.trim_start().starts_with("//"))).collect::<Vec<_>>().join("\n") + "\n";
        let mut seen: BTreeSet<&String> = BTreeSet::new();
        let body = deps.iter().filter(|d| seen.insert(*d)).map(|d| d.as_str()).collect::<Vec<_>>().join("\n");
        let g3 = replace_block(&g2, "// dragon:dependencies:begin", "// dragon:dependencies:end", &body,
            |t| t.find("activity-compose").map(|i| t[i..].find('\n').map(|n| i + n + 1).unwrap_or(t.len())));
        if g3 != g { write_file(&gradle, &g3)?; }
    }
    Ok(())
}

struct AbiInfo {
    abi: &'static str,
    triple: &'static str,
    target: crate::codegen_c::CrossTarget,
}

fn abi_info(abi: &str) -> Result<AbiInfo, String> {
    use crate::codegen_c::CrossTarget as T;
    match abi {
        "arm64-v8a" => Ok(AbiInfo { abi: "arm64-v8a", triple: "aarch64-linux-android", target: T::AndroidArm64 }),
        "armeabi-v7a" => Ok(AbiInfo { abi: "armeabi-v7a", triple: "armv7-linux-androideabi", target: T::AndroidArmv7 }),
        "x86_64" => Ok(AbiInfo { abi: "x86_64", triple: "x86_64-linux-android", target: T::AndroidX86_64 }),
        "x86" => Ok(AbiInfo { abi: "x86", triple: "i686-linux-android", target: T::AndroidX86 }),
        other => Err(format!("unknown ABI '{}' in package.json (use arm64-v8a, armeabi-v7a, x86_64, x86)", other)),
    }
}

fn build_native_app_libs(pkg: &AndroidPackage, android: &Path) -> Result<(), String> {
    let api: u32 = std::env::var("DRAGON_ANDROID_API").ok().and_then(|v| v.parse().ok()).unwrap_or(24);
    let mut targets = Vec::new();
    for abi in &pkg.abis {
        targets.push(abi_info(abi)?.target);
    }
    let opts = crate::codegen_c::CrossBuildOptions {
        targets,
        android_api: api,
        ios_min_version: String::new(),
        make_xcframework: false,
        jni_package: None,
        jni_class: None,
    };
    println!("Compiling {} to native code for {}...", pkg.main, pkg.abis.join(", "));
    let out = android.join("app/src/main/dragon_lib");
    crate::codegen_c::compile_app(Path::new(&pkg.main), &out, false, &opts, false)
}

pub fn create_project_vm(name: &str, package: &str, platforms: &[String]) -> Result<(), String> {
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
    let abis = vec!["arm64-v8a".to_string(), "x86_64".to_string()];
    let with_android = platforms.iter().any(|p| p == "android");
    let pkg_json = crate::platforms::package_json(name, package, platforms);
    write_file(&root.join("package.json"), &pkg_json)?;
    write_file(&root.join("lib/main.dgn"), &fill(VM_MAIN_DGN, name, package))?;
    write_file(&root.join("lib/pages/home.dgn"), &fill(VM_HOME_DGN, name, package))?;
    fs::create_dir_all(root.join("assets")).map_err(|e| e.to_string())?;
    write_file(&root.join("assets/.gitkeep"), "")?;

    let android = root.join("android");
    if with_android {
        write_file(&android.join("build.gradle.kts"), ROOT_BUILD_GRADLE)?;
        write_file(&android.join("settings.gradle.kts"), &fill(SETTINGS_GRADLE, name, package))?;
        write_file(&android.join("gradle.properties"), GRADLE_PROPERTIES)?;
        let app = android.join("app");
        write_file(&app.join("build.gradle.kts"), &fill_abis(&fill(VM_APP_GRADLE, name, package), &abis))?;
        write_file(&app.join("src/main/AndroidManifest.xml"), VM_MANIFEST)?;
        write_file(&app.join("src/main/res/values/strings.xml"), &fill(STRINGS_XML, name, package))?;
        write_file(&app.join("src/main/res/values/themes.xml"), THEMES_XML)?;
        write_file(&app.join("src/main/res/drawable/ic_launcher_background.xml"), IC_LAUNCHER_BG)?;
        write_file(&app.join("src/main/res/drawable/ic_launcher_foreground.xml"), IC_LAUNCHER_FG)?;
        write_file(&app.join("src/main/res/mipmap-anydpi-v26/ic_launcher.xml"), IC_LAUNCHER_ADAPTIVE)?;
        let pkg_path = package.replace('.', "/");
        write_file(&app.join("src/main/java").join(&pkg_path).join("MainActivity.kt"), &fill(VM_MAIN_ACTIVITY, name, package))?;
        write_vm_runtime(&android)?;
        write_plugin_registry(&android, &[])?;
        rewrite_manifest_and_gradle(&android, &[], &[], &[])?;
        write_file(&android.join(".gitignore"), &format!("{}app/src/main/assets/dragon/\napp/src/main/assets/dragon_web/\n", ANDROID_GITIGNORE))?;
    }
    write_file(&root.join(".gitignore"), ".dragon_docs/\n")?;

    println!("Created {}/", root.display());
    println!("  lib/main.dgn   <- your app starts here; screens live in lib/pages/ (import \"pages/home\" as *)");
    println!("  assets/        <- images, fonts, data files bundled into the app");
    for p in platforms {
        match p.as_str() {
            "android" => println!("  android/       <- generated Gradle project; you rarely need to touch it"),
            "ios" => println!("  ios/           <- generated Xcode project inputs (build on a Mac)"),
            "windows" => println!("  windows/       <- generated WPF desktop project (build with the .NET 8 SDK)"),
            _ => {}
        }
    }
    println!();
    println!("Platforms: {}", platforms.join(", "));
    println!("Next:");
    println!("  cd {}", name);
    println!("  dragon preview                      # live preview in your browser, no device needed");
    println!("  dragon run [--platform <p>]         # build and launch");
    println!("  dragon build [--platform <p|all>]   # build release artifacts");

    if with_android && system_gradle_available() {
        println!();
        println!("Generating Gradle wrapper...");
        let _ = Command::new("gradle").arg("wrapper").current_dir(&android).status();
    }
    if platforms.iter().any(|p| p == "ios" || p == "windows") {
        let prev = std::env::current_dir().map_err(|e| e.to_string())?;
        std::env::set_current_dir(&root).map_err(|e| e.to_string())?;
        let mut res = Ok(());
        if platforms.iter().any(|p| p == "ios") { res = res.and_then(|_| crate::ios_project::ios_init()); }
        if platforms.iter().any(|p| p == "windows") { res = res.and_then(|_| crate::windows_project::windows_init()); }
        let _ = std::env::set_current_dir(prev);
        res?;
    }
    Ok(())
}
