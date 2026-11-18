use crate::ui_host::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn load_host(dir: &PathBuf, media: &str) -> Result<UiHost, String> {
    let docs = dir.join(".dragon_docs");
    let _ = std::fs::create_dir_all(&docs);
    UiHost::new(dir, &docs.to_string_lossy(), "desktop", media)
}

pub fn run_dump(args: &[String]) -> i32 {
    let mut dir = PathBuf::from(".");
    let mut actions: Vec<(String, String)> = Vec::new();
    let mut pretty = false;
    let mut all = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tap" | "--event" | "--type" | "--tick" => {
                if i + 1 >= args.len() {
                    eprintln!("{} needs a value", args[i]);
                    return 2;
                }
                actions.push((args[i].clone(), args[i + 1].clone()));
                i += 2;
            }
            "--back" | "--platform" => {
                actions.push((args[i].clone(), String::new()));
                i += 1;
            }
            "--pretty" => {
                pretty = true;
                i += 1;
            }
            "--all" => {
                all = true;
                i += 1;
            }
            other => {
                dir = app_dir_from_arg(Some(other));
                i += 1;
            }
        }
    }

    let host = match load_host(&dir, &default_media_json()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: {}", e);
            return 1;
        }
    };

    let print = |text: &str| {
        if pretty {
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.to_string())),
                Err(_) => println!("{}", text),
            }
        } else {
            println!("{}", text);
        }
    };

    let mut frame_text = host.render();
    if all {
        print(&frame_text);
    }
    let mut clock: i64 = 1_000_000;
    host.tick(clock);

    for (action, value) in &actions {
        let frame: serde_json::Value = serde_json::from_str(&frame_text).unwrap_or(serde_json::Value::Null);
        match action.as_str() {
            "--tap" => match find_tap_id(&frame, value) {
                Some(id) => {
                    eprintln!("[action] event {}", id);
                    host.event(&id);
                }
                None => {
                    eprintln!("no tappable widget with text '{}'", value);
                    return 3;
                }
            },
            "--event" => {
                eprintln!("[action] event {}", value);
                host.event(value);
            }
            "--type" => {
                let (hint, text) = match value.split_once('=') {
                    Some((a, b)) => (a.to_string(), b.to_string()),
                    None => (value.clone(), String::new()),
                };
                match find_field_id(&frame, &hint) {
                    Some(id) => {
                        eprintln!("[action] event {}:{}", id, text);
                        host.event(&format!("{}:{}", id, text));
                    }
                    None => {
                        eprintln!("no text field with hint/label '{}'", hint);
                        return 3;
                    }
                }
            }
            "--tick" => {
                clock += value.parse::<i64>().unwrap_or(0);
                eprintln!("[action] tick {}", clock);
                host.tick(clock);
            }
            "--back" => {
                eprintln!("[action] back");
                host.back();
            }
            "--platform" => {
                println!("{}", host.platform_calls());
                continue;
            }
            _ => {}
        }
        frame_text = host.render();
        if all {
            print(&frame_text);
        }
    }
    if !all {
        print(&frame_text);
    }
    0
}

fn find_field_id(frame: &serde_json::Value, hint: &str) -> Option<String> {
    fn walk(node: &serde_json::Value, hint: &str) -> Option<String> {
        match node {
            serde_json::Value::Object(map) => {
                if map.get("type").and_then(|v| v.as_str()) == Some("textField") {
                    let matches = ["hint", "label"]
                        .iter()
                        .any(|k| map.get(*k).and_then(|v| v.as_str()) == Some(hint));
                    if matches {
                        return map.get("onChanged").and_then(|v| v.as_str()).map(|s| s.to_string());
                    }
                }
                for (_, v) in map {
                    if let Some(x) = walk(v, hint) {
                        return Some(x);
                    }
                }
                None
            }
            serde_json::Value::Array(items) => items.iter().find_map(|v| walk(v, hint)),
            _ => None,
        }
    }
    walk(frame, hint)
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn run_preview(args: &[String]) -> i32 {
    use tiny_http::{Header, Method, Response, Server};

    let mut dir = PathBuf::from(".");
    let mut port: u16 = 8080;
    let mut open_browser = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(8080);
                i += 2;
            }
            "--open" => {
                open_browser = true;
                i += 1;
            }
            other => {
                dir = app_dir_from_arg(Some(other));
                i += 1;
            }
        }
    }

    let mut media = default_media_json();
    let mut host = match load_host(&dir, &media) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: {}", e);
            return 1;
        }
    };
    let mut version: u64 = 1;
    let mut seen_mtime = newest_mtime(&dir.join("lib"));
    let mut last_check = Instant::now();

    let server = match Server::http(format!("127.0.0.1:{}", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not listen on port {}: {}", port, e);
            return 1;
        }
    };
    let url = format!("http://127.0.0.1:{}/", port);
    println!("Dragon preview running at {}  (edit any .dgn under lib/ to hot restart, Ctrl+C to stop)", url);
    if open_browser {
        let _ = open_url(&url);
    }

    let json_header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let html_header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
    let text_header = Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..]).unwrap();
    let no_cache = Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap();

    for mut request in server.incoming_requests() {
        if last_check.elapsed() > Duration::from_millis(300) {
            last_check = Instant::now();
            let m = newest_mtime(&dir.join("lib"));
            if m != seen_mtime {
                seen_mtime = m;
                match load_host(&dir, &media) {
                    Ok(h) => {
                        host = h;
                        version += 1;
                        println!("[preview] reloaded");
                    }
                    Err(e) => eprintln!("[preview] reload failed: {}", e),
                }
            }
        }

        let url_path = request.url().to_string();
        let path = url_path.split('?').next().unwrap_or("/").to_string();
        let query = url_path.split_once('?').map(|x| x.1.to_string()).unwrap_or_default();
        let mut body = String::new();
        if request.method() == &Method::Post {
            use std::io::Read;
            let _ = request.as_reader().read_to_string(&mut body);
        }

        if request.method() == &Method::Get && path == "/icons.ttf" {
            static ICON_FONT: &[u8] = include_bytes!("../framework/fonts/MaterialIcons-Regular.ttf");
            let h = Header::from_bytes(&b"Content-Type"[..], &b"font/ttf"[..]).unwrap();
            let _ = request.respond(Response::from_data(ICON_FONT).with_header(h));
            continue;
        }
        if request.method() == &Method::Get && path.starts_with("/asset/") {
            let rel = path.trim_start_matches("/asset/");
            let rel = rel.replace("%20", " ");
            if rel.contains("..") {
                let _ = request.respond(Response::from_string("forbidden").with_status_code(403));
                continue;
            }
            let candidates = [dir.join(&rel), dir.join("assets").join(&rel)];
            let found = candidates.iter().find_map(|c| std::fs::read(c).ok().map(|b| (c.clone(), b)));
            match found {
                Some((c, bytes)) => {
                    let ext = c.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    let mime = match ext.as_str() {
                        "png" => "image/png",
                        "jpg" | "jpeg" => "image/jpeg",
                        "gif" => "image/gif",
                        "webp" => "image/webp",
                        "svg" => "image/svg+xml",
                        "json" => "application/json",
                        "ttf" => "font/ttf",
                        _ => "application/octet-stream",
                    };
                    let h = Header::from_bytes(&b"Content-Type"[..], mime.as_bytes()).unwrap();
                    let _ = request.respond(Response::from_data(bytes).with_header(h));
                }
                None => {
                    let _ = request.respond(Response::from_string("not found").with_status_code(404));
                }
            }
            continue;
        }

        if request.method() == &Method::Post && path == "/api/http" {
            let payload = body.clone();
            let json_h = json_header.clone();
            std::thread::spawn(move || {
                let out = proxy_http(&payload);
                let _ = request.respond(Response::from_string(out).with_header(json_h));
            });
            continue;
        }

        let (text, header) = match (request.method(), path.as_str()) {
            (&Method::Get, "/") => (
                std::env::var("DRAGON_PREVIEW_HTML")
                    .ok()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .unwrap_or_else(|| include_str!("../framework/preview/index.html").to_string()),
                html_header.clone(),
            ),
            (&Method::Get, "/api/frame") => (host.render(), json_header.clone()),
            (&Method::Get, "/api/version") => (version.to_string(), text_header.clone()),
            (&Method::Post, "/api/event") => (host.event(&body).to_string(), text_header.clone()),
            (&Method::Post, "/api/tick") => {
                let next = host.tick(body.trim().parse::<i64>().unwrap_or_else(|_| now_ms()));
                (format!("{},{}", next, host.is_dirty()), text_header.clone())
            }
            (&Method::Get, "/api/platform") => (host.platform_calls(), json_header.clone()),
            (&Method::Post, "/api/platform_result") => {
                let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).unwrap_or("").to_string();
                (host.platform_result(&id, &body).to_string(), text_header.clone())
            }
            (&Method::Post, "/api/back") => (host.back().to_string(), text_header.clone()),
            (&Method::Post, "/api/lifecycle") => (host.lifecycle(body.trim()).to_string(), text_header.clone()),
            (&Method::Post, "/api/media") => {
                media = body.clone();
                (host.set_media(&body).to_string(), text_header.clone())
            }
            _ => {
                let _ = request.respond(Response::from_string("not found").with_status_code(404));
                continue;
            }
        };
        let _ = request.respond(Response::from_string(text).with_header(header).with_header(no_cache.clone()));
    }
    0
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
#[derive(Debug)]
struct AcceptAnyCert(std::sync::Arc<rustls::crypto::CryptoProvider>);

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn insecure_tls_config() -> std::sync::Arc<rustls::ClientConfig> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(AcceptAnyCert(provider)))
        .with_no_client_auth();
    std::sync::Arc::new(config)
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn proxy_http(payload: &str) -> String {
    let spec: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"status": 0, "error": e.to_string()}).to_string(),
    };
    let method = spec["method"].as_str().unwrap_or("GET").to_uppercase();
    let url = spec["url"].as_str().unwrap_or("");
    let timeout = spec["timeout"].as_u64().unwrap_or(30000);
    let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_millis(timeout));
    if spec["insecure"].as_bool().unwrap_or(false) {
        builder = builder.tls_config(insecure_tls_config());
    }
    let agent = builder.build();
    let mut req = agent.request(&method, url);
    if let Some(h) = spec["headers"].as_object() {
        for (k, v) in h {
            if let Some(v) = v.as_str() {
                req = req.set(k, v);
            }
        }
    }
    let result = if let Some(parts) = spec["bodyParts"].as_array() {
        use base64::Engine;
        let mut bytes: Vec<u8> = Vec::new();
        for p in parts {
            if let Some(t) = p["text"].as_str() {
                bytes.extend_from_slice(t.as_bytes());
            } else if let Some(b) = p["base64"].as_str() {
                if let Ok(d) = base64::engine::general_purpose::STANDARD.decode(b) {
                    bytes.extend_from_slice(&d);
                }
            }
        }
        req.send_bytes(&bytes)
    } else {
        match spec["body"].as_str() {
            Some(b) if method != "GET" => req.send_string(b),
            _ => req.call(),
        }
    };
    let resp = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return serde_json::json!({"status": 0, "error": e.to_string()}).to_string(),
    };
    let status = resp.status();
    let mut headers = serde_json::Map::new();
    for name in resp.headers_names() {
        if let Some(v) = resp.header(&name) {
            headers.insert(name, serde_json::Value::String(v.to_string()));
        }
    }
    let text = resp.into_string().unwrap_or_default();
    serde_json::json!({"status": status, "body": text, "headers": headers}).to_string()
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub fn run_preview(_args: &[String]) -> i32 {
    eprintln!("dragon preview is not available on this platform");
    1
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";
    std::process::Command::new(cmd).arg(url).spawn().map(|_| ())
}
