use super::*;

pub fn check_for_bundle(exe_path: &Path) -> Option<String> {
    let mut file = match File::open(exe_path) {
        Ok(f) => f,
        Err(_) => return None,
    };

    let len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return None,
    };

    if len < 24 { return None; }

    if file.seek(SeekFrom::End(-24)).is_err() { return None; }
    let mut footer = [0u8; 24];
    if file.read_exact(&mut footer).is_err() { return None; }

    if &footer[8..24] != BUNDLE_SIGNATURE { return None; }


    let script_len = u64::from_le_bytes(footer[0..8].try_into().unwrap());
    if script_len > len - 24 { return None; }


    if file.seek(SeekFrom::End(-(24 + script_len as i64))).is_err() { return None; }
    let mut script_buf = vec![0u8; script_len as usize];
    if file.read_exact(&mut script_buf).is_err() { return None; }


    let decrypted = decrypt_payload(&script_buf)?;
    String::from_utf8(decrypted).ok()
}

pub fn gather_sources(path: &Path, sources: &mut HashMap<String, String>, is_root: bool) -> Result<(), String> {
    let code = fs::read_to_string(path)
        .map_err(|e| format!("Could not read source file '{}': {}", path.display(), e))?;

    let key = if is_root { "__entry__".to_string() } else { path.file_name().unwrap().to_string_lossy().to_string() };
    if sources.contains_key(&key) { return Ok(()); }
    sources.insert(key, code.clone());

    let re = Regex::new(r#"import[ \t]+["']([^"']+)["']"#).unwrap();
    for cap in re.captures_iter(&code) {
        let import_ref = &cap[1];
        
        // // Skip built-ins so the bundler doesn't try to look for them on disk
        // let builtins = ["math", "json", "base64", "file", "os", "path", "time", "net", "ffi", "struct", "mem", "thread", "uuid"];
        // if builtins.contains(&import_ref.as_str()) { continue; }

        let current_dir = path.parent().unwrap_or(Path::new("."));
        let mut search_dirs = vec![current_dir.to_path_buf(), current_dir.join("dgn_modules")];
        if let Ok(dp) = std::env::var("DRAGON_PATH") {
            for p in std::env::split_paths(&dp) { search_dirs.push(p); }
        }

        let mut target_files = Vec::new();
        let path_obj = Path::new(import_ref);
        if path_obj.extension().is_some() {
            target_files.push(path_obj.to_path_buf());
        } else {
            target_files.push(PathBuf::from(format!("{}.dgn", import_ref)));
            if let Some(name) = path_obj.file_name() {
                let n = name.to_string_lossy();
                target_files.push(PathBuf::from(import_ref).join(format!("{}.dgn", n)));
                target_files.push(PathBuf::from(import_ref).join("index.dgn"));
            }
        }

        let mut found_path = None;
        for dir in &search_dirs {
            for target in &target_files {
                let candidate = dir.join(target);
                if candidate.exists() && candidate.is_file() {
                    found_path = Some(candidate);
                    break;
                }
            }
            if found_path.is_some() { break; }
        }

        if let Some(import_path) = found_path {
            gather_sources(&import_path, sources, false)?;
        } else {
            println!("Warning: Bundler skipped missing import '{}'", import_ref);
        }
    }
    Ok(())
}

