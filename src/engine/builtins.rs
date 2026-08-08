use super::*;

#[cfg(not(any(target_arch = "wasm32")))]
fn get_ffi_type_info(name: &str) -> Option<(Type, usize, usize)> {
    match name {
        "u8" | "byte"   => Some((Type::u8(), 1, 1)),
        "i8" | "char"   => Some((Type::i8(), 1, 1)),
        "u16"           => Some((Type::u16(), 2, 2)),
        "i16" | "short" => Some((Type::i16(), 2, 2)),
        "u32"           => Some((Type::u32(), 4, 4)),
        "i32" | "int"   => Some((Type::i32(), 4, 4)),
        "u64"           => Some((Type::u64(), 8, 8)),
        "i64" | "long"  => Some((Type::i64(), 8, 8)),
        "f32" | "float" => Some((Type::f32(), 4, 4)),
        "f64" | "double"=> Some((Type::f64(), 8, 8)),
        "pointer" | "string" => Some((Type::pointer(), 8, 8)), 
        _ => None, 
    }
}

fn make_promise<F>(f: F) -> DgnValue
where
    F: FnOnce() -> Result<DgnValue, DgnError> + Send + 'static,
{
    let blocking_handle = tokio::task::spawn_blocking(f);
    
    let async_handle = tokio::spawn(async move {
        blocking_handle.await.unwrap_or_else(|_| Err(DgnError::new("Background thread panicked", 0)))
    });
    
    DgnValue::Promise(Arc::new(AsyncMutex::new(PromiseState::Pending(
        Arc::new(AsyncMutex::new(Some(async_handle)))
    ))))
}


pub fn get_builtin_module(name: &str) -> Option<DgnValue> {
  {
     let cache = BUILTIN_MODULE_CACHE.lock().unwrap();
        if let Some(v) = cache.get(name) {
            return Some(v.clone());
        }
    }
    // Not cached yet — build it
    let val = build_builtin_module(name)?;
    BUILTIN_MODULE_CACHE.lock().unwrap().insert(name.to_owned(), val.clone());
    Some(val)
}

/// Builds a built-in module map from scratch.
/// Called only ONCE per module name — result is cached by get_builtin_module.
fn build_builtin_module(name: &str) -> Option<DgnValue> {
    match name {
        "math" => {
            let mut map = HashMap::new();
            map.insert("sin".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(n) = a.get(0).and_then(|v| v.as_number()) { n.sin() } else { 0.0 }))));
            map.insert("cos".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(n) = a.get(0).and_then(|v| v.as_number()) { n.cos() } else { 0.0 }))));
            map.insert("sqrt".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(n) = a.get(0).and_then(|v| v.as_number()) { n.sqrt() } else { 0.0 }))));
            map.insert("abs".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(n) = a.get(0).and_then(|v| v.as_number()) { n.abs() } else { 0.0 }))));
            map.insert("floor".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(n) = a.get(0).and_then(|v| v.as_number()) { n.floor() } else { 0.0 }))));
            map.insert("ceil".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(n) = a.get(0).and_then(|v| v.as_number()) { n.ceil() } else { 0.0 }))));
            map.insert("round".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(n) = a.get(0).and_then(|v| v.as_number()) { n.round() } else { 0.0 }))));
            map.insert("pow".to_string(), native_fn(|args| {
                let base = args.get(0).and_then(|v| v.as_number()).unwrap_or(0.0);
                let exp = args.get(1).and_then(|v| v.as_number()).unwrap_or(1.0);
                Ok(DgnValue::Number(base.powf(exp)))
            }));
            map.insert("log".to_string(), native_fn(|args| {
                let n = args.get(0).and_then(|v| v.as_number()).unwrap_or(0.0);
                Ok(DgnValue::Number(n.ln()))
            }));
            map.insert("min".to_string(), native_fn(|args| {
                let mut min = std::f64::INFINITY;
                for arg in args { if let Some(n) = arg.as_number() { if n < min { min = n; } } }
                Ok(DgnValue::Number(min))
            }));
            map.insert("max".to_string(), native_fn(|args| {
                let mut max = std::f64::NEG_INFINITY;
                for arg in args { if let Some(n) = arg.as_number() { if n > max { max = n; } } }
                Ok(DgnValue::Number(max))
            }));
            map.insert("range".to_string(), native_fn(|args| {
                let (start, stop, step) = match args.len() {
                    1 => (0.0, args[0].as_number().unwrap_or(0.0), 1.0),
                    2 => (args[0].as_number().unwrap_or(0.0), args[1].as_number().unwrap_or(0.0), 1.0),
                    3 => (args[0].as_number().unwrap_or(0.0), args[1].as_number().unwrap_or(0.0), args[2].as_number().unwrap_or(1.0)),
                    _ => return Err(DgnError::new("range() expects 1 to 3 arguments", 0)),
                };
                let mut nums = Vec::new();
                let mut curr = start;
                if step > 0.0 {
                    while curr < stop { nums.push(DgnValue::Number(curr)); curr += step; }
                } else if step < 0.0 {
                    while curr > stop { nums.push(DgnValue::Number(curr)); curr += step; }
                }
                Ok(DgnValue::List(Arc::new(Mutex::new(nums))))
            }));
            map.insert("random".to_string(), native_fn(|_| Ok(DgnValue::Number(rand::random::<f64>()))));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "json" => {
            let mut map = HashMap::new();
            fn dgn_to_json(v: &DgnValue) -> serde_json::Value {
                match v {
                    DgnValue::Null => serde_json::Value::Null,
                    DgnValue::Bool(b) => serde_json::Value::Bool(*b),
                    DgnValue::Number(n) => serde_json::Number::from_f64(*n).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
                    DgnValue::String(s) => serde_json::Value::String(s.clone()),
                    DgnValue::List(l) => serde_json::Value::Array(l.lock().unwrap().iter().map(dgn_to_json).collect()),
                    DgnValue::Int(n) => serde_json::Value::Number((*n).into()), 
                    DgnValue::Map(m) | DgnValue::Instance(m) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in m.lock().unwrap().iter() { map.insert(k.clone(), dgn_to_json(v)); }
                        serde_json::Value::Object(map)
                    },
                    _ => serde_json::Value::String(val_to_string(v)),
                }
            }

            map.insert("encode".to_string(), native_fn(|args| {
                let v = args.get(0).unwrap_or(&DgnValue::Null);
                Ok(DgnValue::String(dgn_to_json(v).to_string()))
            }));

            map.insert("decode".to_string(), native_fn(|args| {
                let json_str = args.get(0).map(val_to_string).unwrap_or_default();
                fn json_to_dgn(v: serde_json::Value) -> DgnValue {
                    match v {
                        serde_json::Value::Null => DgnValue::Null,
                        serde_json::Value::Bool(b) => DgnValue::Bool(b),
                        serde_json::Value::Number(n) => DgnValue::Number(n.as_f64().unwrap_or(0.0)),
                        serde_json::Value::String(s) => DgnValue::String(s),
                        serde_json::Value::Array(arr) => DgnValue::List(Arc::new(Mutex::new(arr.into_iter().map(json_to_dgn).collect()))),
                        serde_json::Value::Object(m) => {
                            let mut dgn_map = HashMap::new();
                            for (k, v) in m { dgn_map.insert(k, json_to_dgn(v)); }
                            DgnValue::Map(Arc::new(Mutex::new(dgn_map)))
                        }
                    }
                }
                match serde_json::from_str(&json_str) {
                    Ok(val) => Ok(json_to_dgn(val)),
                    Err(_) => Ok(DgnValue::Null)
                }
            }));
            
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "base64" => {
            let mut map = HashMap::new();
            map.insert("encode".to_string(), native_fn(|args| {
                use base64::{Engine as _, engine::general_purpose};
                let s = args.get(0).map(val_to_string).unwrap_or_default();
                Ok(DgnValue::String(general_purpose::STANDARD.encode(s)))
            }));
            map.insert("decode".to_string(), native_fn(|args| {
                use base64::{Engine as _, engine::general_purpose};
                let s = args.get(0).map(val_to_string).unwrap_or_default();
                match general_purpose::STANDARD.decode(s) {
                    Ok(bytes) => Ok(DgnValue::String(String::from_utf8_lossy(&bytes).to_string())),
                    Err(_) => Ok(DgnValue::Null)
                }
            }));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "file" => {
            let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
            {
                map.insert("read_all".to_string(), native_fn(|a| {
                    let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("File path required",0)) };
                    match fs::read_to_string(p) { Ok(c) => Ok(DgnValue::String(c)), Err(_) => Ok(DgnValue::Null) }
                }));
                map.insert("read_bytes".to_string(), native_fn(|a| {
                    let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("File path required",0)) };
                    match fs::read(p) { Ok(c) => Ok(DgnValue::ByteArray(Arc::new(Mutex::new(c)))), Err(_) => Ok(DgnValue::Null) }
                }));
                map.insert("write".to_string(), native_fn(|a| {
                    let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("Path required",0)) };
                    let c = if let Some(DgnValue::String(s)) = a.get(1) { s } else { "" };
                    Ok(DgnValue::Bool(fs::write(p, c).is_ok()))
                }));

                map.insert("write_bytes".to_string(), native_fn(|args| {
                    let p = if let Some(DgnValue::String(s)) = args.get(0) { s.clone() }
                            else { return Err(DgnError::new("Path required", 0)) };
                    let data = if let Some(DgnValue::ByteArray(b)) = args.get(1) {
                        b.lock().unwrap().clone()
                    } else { return Err(DgnError::new("ByteArray required", 0)) };
                    Ok(DgnValue::Bool(std::fs::write(p, data).is_ok()))
                }));


                map.insert("write_bytes_slice".to_string(), native_fn(|args| {
                    let p = if let Some(DgnValue::String(s)) = args.get(0) { s.clone() }
                            else { return Err(DgnError::new("Path required", 0)) };
                    let data = if let Some(DgnValue::ByteArray(b)) = args.get(1) {
                        b.lock().unwrap().clone()
                    } else { return Err(DgnError::new("ByteArray required", 0)) };
                    let start = args.get(2).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    let end = args.get(3).and_then(|v| v.as_int()).unwrap_or(data.len() as i64) as usize;
                    let end = end.min(data.len());
                    let slice = &data[start..end];
                    Ok(DgnValue::Bool(std::fs::write(p, slice).is_ok()))
                }));

                map.insert("exists".to_string(), native_fn(|args| {
                    let path = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(Path::new(&path).exists()))
                }));
                map.insert("delete".to_string(), native_fn(|args| {
                    let path = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(fs::remove_file(path).is_ok()))
                }));

                map.insert("append".to_string(), native_fn(|args| {
                    let p = if let Some(DgnValue::String(s)) = args.get(0) { s } else { return Err(DgnError::new("Path required", 0)) };
                    let c = if let Some(DgnValue::String(s)) = args.get(1) { s } else { "" };
                    let res = std::fs::OpenOptions::new().create(true).append(true).open(p)
                        .and_then(|mut f| std::io::Write::write_all(&mut f, c.as_bytes()));
                    Ok(DgnValue::Bool(res.is_ok()))
                }));
                
                map.insert("mkdir".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::create_dir_all(p).is_ok()))
                }));
                
                map.insert("rmdir".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::remove_dir_all(p).is_ok()))
                }));
                
                map.insert("rename".to_string(), native_fn(|args| {
                    let from = args.get(0).map(val_to_string).unwrap_or_default();
                    let to = args.get(1).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::rename(from, to).is_ok()))
                }));
                
                map.insert("size".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Number(std::fs::metadata(p).map(|m| m.len() as f64).unwrap_or(-1.0)))
                }));
                
                map.insert("is_dir".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)))
                }));
            }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },

        "pdf" => {
                let mut map = HashMap::new();
            
                // pdf.create() → doc handle (a Map)
                map.insert("create".to_string(), native_fn(|_args| {
                    let mut doc = HashMap::new();
                    doc.insert("__type".to_string(), DgnValue::String("pdf_doc".to_string()));
                    doc.insert("pages".to_string(), DgnValue::List(Arc::new(Mutex::new(Vec::new()))));
                    doc.insert("fonts".to_string(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
                    doc.insert("images".to_string(), DgnValue::List(Arc::new(Mutex::new(Vec::new()))));
                    Ok(DgnValue::Map(Arc::new(Mutex::new(doc))))
                }));

                map.insert("draw_image".to_string(), native_fn(|args| {
                        let page      = args.get(0).ok_or(DgnError::new("Page required", 0))?;
                        let img_bytes = args.get(1).cloned().unwrap_or(DgnValue::Null);
                        let x  = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0);
                        let y  = args.get(3).and_then(|v| v.as_number()).unwrap_or(0.0);
                        let iw = args.get(4).and_then(|v| v.as_number()).unwrap_or(50.0);
                        let ih = args.get(5).and_then(|v| v.as_number()).unwrap_or(50.0);

                        let mut op = HashMap::new();
                        op.insert("type".to_string(),  DgnValue::String("image".to_string()));
                        op.insert("x".to_string(),     DgnValue::Number(x));
                        op.insert("y".to_string(),     DgnValue::Number(y));
                        op.insert("w".to_string(),     DgnValue::Number(iw));
                        op.insert("h".to_string(),     DgnValue::Number(ih));
                        op.insert("data".to_string(),  img_bytes);

                        if let DgnValue::Map(pm) = page {
                            let pm = pm.lock().unwrap();
                            if let Some(DgnValue::List(ops)) = pm.get("ops") {
                                ops.lock().unwrap().push(DgnValue::Map(Arc::new(Mutex::new(op))));
                            }
                        }
                        Ok(DgnValue::Null)
                    }));
            
                // pdf.add_page(doc, width_pt, height_pt) → page handle (a Map)
                map.insert("add_page".to_string(), native_fn(|args| {
                    let doc = args.get(0).ok_or(DgnError::new("Doc handle required", 0))?;
                    let width = args.get(1).and_then(|v| v.as_number()).unwrap_or(612.0);
                    let height = args.get(2).and_then(|v| v.as_number()).unwrap_or(792.0);
            
                    let mut page = HashMap::new();
                    page.insert("width".to_string(), DgnValue::Number(width));
                    page.insert("height".to_string(), DgnValue::Number(height));
                    page.insert("ops".to_string(), DgnValue::List(Arc::new(Mutex::new(Vec::new()))));
                    page.insert("current_font".to_string(), DgnValue::Null);
                    page.insert("current_size".to_string(), DgnValue::Number(12.0));
            
                    let page_val = DgnValue::Map(Arc::new(Mutex::new(page)));
            
                    if let DgnValue::Map(dm) = doc {
                        let dm = dm.lock().unwrap();
                        if let Some(DgnValue::List(pages)) = dm.get("pages") {
                            pages.lock().unwrap().push(page_val.clone());
                        }
                    }
            
                    Ok(page_val)
                }));
            
                // pdf.draw_text(page, text, x, y, options?) → null
                // options is optional map: { size: 12, r: 0, g: 0, b: 0, bold: false }
                map.insert("draw_text".to_string(), native_fn(|args| {
                        let page = args.get(0).ok_or(DgnError::new("Page handle required", 0))?;
                        let text = args.get(1).map(val_to_string).unwrap_or_default();
                        let x = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0);
                        let y = args.get(3).and_then(|v| v.as_number()).unwrap_or(0.0);

                        let size = args.get(4).and_then(|v| {
                            if let DgnValue::Map(m) = v {
                                m.lock().unwrap().get("size").and_then(|v| v.as_number())
                            } else { v.as_number() }
                        }).unwrap_or(12.0);

                        let (r, g, b) = if let Some(DgnValue::Map(opts)) = args.get(4) {
                            let opts = opts.lock().unwrap();
                            (
                                opts.get("r").and_then(|v| v.as_number()).unwrap_or(0.0),
                                opts.get("g").and_then(|v| v.as_number()).unwrap_or(0.0),
                                opts.get("b").and_then(|v| v.as_number()).unwrap_or(0.0),
                            )
                        } else { (0.0, 0.0, 0.0) };

                        let bold = if let Some(DgnValue::Map(opts)) = args.get(4) {
                            let opts = opts.lock().unwrap();
                            opts.get("bold").map(|v| is_truthy(v)).unwrap_or(false)
                            
                        } else { false };

                        let italic = if let Some(DgnValue::Map(opts)) = args.get(4) {
                            let opts = opts.lock().unwrap();
                            opts.get("italic").map(|v| if let DgnValue::Bool(b) = v { *b } else { false }).unwrap_or(false)
                        } else { false };

                        let font_name = if let Some(DgnValue::Map(opts)) = args.get(4) {
                            let opts = opts.lock().unwrap();
                            opts.get("font").map(val_to_string)
                        } else { None };

                        let url_str = if let Some(DgnValue::Map(opts)) = args.get(4) {
                            let opts = opts.lock().unwrap();
                            if let Some(v) = opts.get("url") {
                                if matches!(v, DgnValue::Null) { None } else { Some(val_to_string(v)) }
                            } else { None }
                        } else { None };

                        let mut op = HashMap::new();
                        op.insert("type".to_string(), DgnValue::String("text".to_string()));
                        op.insert("text".to_string(), DgnValue::String(text));
                        op.insert("x".to_string(), DgnValue::Number(x));
                        op.insert("y".to_string(), DgnValue::Number(y));
                        op.insert("size".to_string(), DgnValue::Number(size));
                        op.insert("r".to_string(), DgnValue::Number(r));
                        op.insert("g".to_string(), DgnValue::Number(g));
                        op.insert("b".to_string(), DgnValue::Number(b));
                        op.insert("bold".to_string(), DgnValue::Bool(bold));
                        op.insert("italic".to_string(), DgnValue::Bool(italic));
                        if let Some(fname) = font_name {
                            op.insert("font_name".to_string(), DgnValue::String(fname));
                        }

                        if let Some(u) = url_str {
                            op.insert("url".to_string(), DgnValue::String(u));
                        }

                        if let DgnValue::Map(pm) = page {
                            let pm = pm.lock().unwrap();
                            if let Some(DgnValue::List(ops)) = pm.get("ops") {
                                ops.lock().unwrap().push(DgnValue::Map(Arc::new(Mutex::new(op))));
                            }
                        }
                        Ok(DgnValue::Null)
                    }));
            
                // pdf.draw_line(page, x1, y1, x2, y2, width?, r?, g?, b?)
                map.insert("draw_line".to_string(), native_fn(|args| {
                    let page = args.get(0).ok_or(DgnError::new("Page required", 0))?;
                    let x1 = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let y1 = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let x2 = args.get(3).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let y2 = args.get(4).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let w  = args.get(5).and_then(|v| v.as_number()).unwrap_or(1.0);
                    let r  = args.get(6).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let g  = args.get(7).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let b  = args.get(8).and_then(|v| v.as_number()).unwrap_or(0.0);
            
                    let mut op = HashMap::new();
                    op.insert("type".to_string(), DgnValue::String("line".to_string()));
                    op.insert("x1".to_string(), DgnValue::Number(x1));
                    op.insert("y1".to_string(), DgnValue::Number(y1));
                    op.insert("x2".to_string(), DgnValue::Number(x2));
                    op.insert("y2".to_string(), DgnValue::Number(y2));
                    op.insert("w".to_string(), DgnValue::Number(w));
                    op.insert("r".to_string(), DgnValue::Number(r));
                    op.insert("g".to_string(), DgnValue::Number(g));
                    op.insert("b".to_string(), DgnValue::Number(b));
            
                    if let DgnValue::Map(pm) = page {
                        let pm = pm.lock().unwrap();
                        if let Some(DgnValue::List(ops)) = pm.get("ops") {
                            ops.lock().unwrap().push(DgnValue::Map(Arc::new(Mutex::new(op))));
                        }
                    }
                    Ok(DgnValue::Null)
                }));
            
                // pdf.draw_rect(page, x, y, w, h, fill_r?, fill_g?, fill_b?, filled?)
                map.insert("draw_rect".to_string(), native_fn(|args| {
                    let page = args.get(0).ok_or(DgnError::new("Page required", 0))?;
                    let x = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let y = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let w = args.get(3).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let h = args.get(4).and_then(|v| v.as_number()).unwrap_or(0.0);
                    let r = args.get(5).and_then(|v| v.as_number()).unwrap_or(1.0);
                    let g = args.get(6).and_then(|v| v.as_number()).unwrap_or(1.0);
                    let b = args.get(7).and_then(|v| v.as_number()).unwrap_or(1.0);
                    let filled = args.get(8).map(|v| is_truthy(v)).unwrap_or(true);
            
                    let mut op = HashMap::new();
                    op.insert("type".to_string(), DgnValue::String("rect".to_string()));
                    op.insert("x".to_string(), DgnValue::Number(x));
                    op.insert("y".to_string(), DgnValue::Number(y));
                    op.insert("w".to_string(), DgnValue::Number(w));
                    op.insert("h".to_string(), DgnValue::Number(h));
                    op.insert("r".to_string(), DgnValue::Number(r));
                    op.insert("g".to_string(), DgnValue::Number(g));
                    op.insert("b".to_string(), DgnValue::Number(b));
                    op.insert("filled".to_string(), DgnValue::Bool(filled));
            
                    if let DgnValue::Map(pm) = page {
                        let pm = pm.lock().unwrap();
                        if let Some(DgnValue::List(ops)) = pm.get("ops") {
                            ops.lock().unwrap().push(DgnValue::Map(Arc::new(Mutex::new(op))));
                        }
                    }
                    Ok(DgnValue::Null)
                }));

                  map.insert("register_font".to_string(), native_fn(|args| {
                    let doc = args.get(0).ok_or(DgnError::new("Doc required", 0))?;
                    let font_handle = args.get(1).ok_or(DgnError::new("Font handle required", 0))?;
                    let key = args.get(2).map(val_to_string).unwrap_or("regular".to_string());

                    if let (DgnValue::Map(dm), DgnValue::Map(fh)) = (doc, font_handle) {
                        let dm = dm.lock().unwrap();
                        let fh_lock = fh.lock().unwrap();

                        let font_data = if let Some(DgnValue::ByteArray(b)) = fh_lock.get("__font_data") {
                            b.lock().unwrap().clone()
                        } else {
                            return Err(DgnError::new("Invalid font handle", 0));
                        };

                        let family = fh_lock.get("family").map(val_to_string).unwrap_or("Unknown".to_string());
                        let upem = fh_lock.get("units_per_em").and_then(|v| v.as_number()).unwrap_or(1000.0);
                        let asc = fh_lock.get("ascender").and_then(|v| v.as_number()).unwrap_or(800.0);
                        let desc = fh_lock.get("descender").and_then(|v| v.as_number()).unwrap_or(-200.0);

                        if let Some(DgnValue::Map(fonts)) = dm.get("fonts") {
                            let mut fonts = fonts.lock().unwrap();
                            let mut info = HashMap::new();
                            info.insert("data".to_string(), DgnValue::ByteArray(Arc::new(Mutex::new(font_data))));
                            info.insert("family".to_string(), DgnValue::String(family));
                            info.insert("units_per_em".to_string(), DgnValue::Number(upem));
                            info.insert("ascender".to_string(), DgnValue::Number(asc));
                            info.insert("descender".to_string(), DgnValue::Number(desc));
                            fonts.insert(key, DgnValue::Map(Arc::new(Mutex::new(info))));
                        }
                    }
                    Ok(DgnValue::Null)
                }));
            
                // pdf.save(doc) → ByteArray containing valid PDF
                // This is the PDF generator — converts stored ops into raw PDF binary
                map.insert("save".to_string(), native_fn(|args| {
                    let doc = args.get(0).ok_or(DgnError::new("Doc handle required", 0))?;
            
                    if let DgnValue::Map(dm) = doc {
                        let dm = dm.lock().unwrap();
                        let pages = if let Some(DgnValue::List(p)) = dm.get("pages") {
                            p.lock().unwrap().clone()
                        } else { Vec::new() };
            
                        // Build raw PDF
                        let mut pdf = Vec::<u8>::new();
                        let mut offsets: Vec<usize> = Vec::new();
            
                        // Header
                        pdf.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
            
                        // We'll build objects: catalog, pages, each page, each content stream
                        let num_pages = pages.len();

            // --- Extract registered fonts ---
            let registered_fonts = if let Some(DgnValue::Map(fonts)) = dm.get("fonts") {
                fonts.lock().unwrap().clone()
            } else { HashMap::new() };

            // Helper: WinAnsiEncoding code 128-159 → Unicode mapping
            fn winansi_to_unicode(code: u32) -> u32 {
                match code {
                    128 => 0x20AC, 130 => 0x201A, 131 => 0x0192, 132 => 0x201E,
                    133 => 0x2026, 134 => 0x2020, 135 => 0x2021, 136 => 0x02C6,
                    137 => 0x2030, 138 => 0x0160, 139 => 0x2039, 140 => 0x0152,
                    142 => 0x017D, 145 => 0x2018, 146 => 0x2019, 147 => 0x201C,
                    148 => 0x201D, 149 => 0x2022, 150 => 0x2013, 151 => 0x2014,
                    152 => 0x02DC, 153 => 0x2122, 154 => 0x0161, 155 => 0x203A,
                    156 => 0x0153, 158 => 0x017E, 159 => 0x0178,
                    _ => code, // 32-127 and 160-255 map directly
                }
            }

            fn compute_widths(font_data: &[u8], upem: f64) -> Vec<i64> {
                let scale = 1000.0 / upem;
                let mut widths = Vec::with_capacity(224);
                if let Ok(face) = ttf_parser::Face::parse(font_data, 0) {
                    for code in 32u32..=255u32 {
                        let unicode = winansi_to_unicode(code);
                        let w = if let Some(ch) = char::from_u32(unicode) {
                            if let Some(gid) = face.glyph_index(ch) {
                                (face.glyph_hor_advance(gid).unwrap_or(0) as f64 * scale).round() as i64
                            } else { 0 }
                        } else { 0 };
                        widths.push(w);
                    }
                } else {
                    for _ in 32..=255u32 { widths.push(500); }
                }
                widths
            }

            let get_font_data = |key: &str| -> Option<(Vec<u8>, String, f64, f64, f64)> {
                registered_fonts.get(key).and_then(|v| {
                    if let DgnValue::Map(m) = v {
                        let m = m.lock().unwrap();
                        let data = if let Some(DgnValue::ByteArray(b)) = m.get("data") { b.lock().unwrap().clone() } else { return None };
                        let family = m.get("family").map(val_to_string).unwrap_or("CustomFont".to_string());
                        let upem = m.get("units_per_em").and_then(|v| v.as_number()).unwrap_or(1000.0);
                        let asc = m.get("ascender").and_then(|v| v.as_number()).unwrap_or(800.0);
                        let desc = m.get("descender").and_then(|v| v.as_number()).unwrap_or(-200.0);
                        Some((data, family, upem, asc, desc))
                    } else { None }
                })
            };

            let reg_data = get_font_data("regular");
            let bold_data = get_font_data("bold");
            let italic_data = get_font_data("italic");
            let bold_italic_data = get_font_data("bold_italic");

            let reg_obj_count: usize = if reg_data.is_some() { 3 } else { 1 };
            let bold_obj_count: usize = if bold_data.is_some() { 3 } else { 1 };
            let italic_obj_count: usize = if italic_data.is_some() { 3 } else { 1 };
            let bold_italic_obj_count: usize = if bold_italic_data.is_some() { 3 } else { 1 };

            let font_obj_regular: usize = 3;
            let font_obj_bold: usize = font_obj_regular + reg_obj_count;
            let font_obj_italic: usize = font_obj_bold + bold_obj_count;
            let font_obj_bold_italic: usize = font_obj_italic + italic_obj_count;
            let first_page_obj: usize = font_obj_bold_italic + bold_italic_obj_count;
            let first_stream_obj = first_page_obj + num_pages;

            // Object 1: Catalog
            offsets.push(pdf.len());
            pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

            // Object 2: Pages
            offsets.push(pdf.len());
            let kids: String = (0..num_pages)
                .map(|i| format!("{} 0 R", first_page_obj + i))
                .collect::<Vec<_>>()
                .join(" ");
            pdf.extend_from_slice(
                format!("2 0 obj\n<< /Type /Pages /Kids [ {} ] /Count {} >>\nendobj\n",
                    kids, num_pages
                ).as_bytes()
            );

            // Macro to quickly write font objects
            macro_rules! write_font {
                ($data_opt:expr, $font_obj:expr, $fallback:expr) => {
                    if let Some((ref data, ref family, upem, asc, desc)) = $data_opt {
                        let pdf_name = family.replace(' ', "-");
                        let scale = 1000.0 / upem;
let widths = compute_widths(data, upem);
                        let widths_str: String = widths.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" ");
                        let (bbox, cap_h, stem_v) = if let Ok(face) = ttf_parser::Face::parse(data, 0) {
                            let bb = face.global_bounding_box();
                            let ch = face.capital_height().unwrap_or(face.ascender());
                            let sv = if face.weight().to_number() >= 700 { 120 } else { 80 };
                            (format!("[{} {} {} {}]",
                                (bb.x_min as f64 * scale).round() as i64,
                                (bb.y_min as f64 * scale).round() as i64,
                                (bb.x_max as f64 * scale).round() as i64,
                                (bb.y_max as f64 * scale).round() as i64),
                            (ch as f64 * scale).round() as i64, sv)
                        } else { ("[-200 -300 1200 1000]".to_string(), 700, 80) };
                        let fd_obj = $font_obj + 1;
                        let ff_obj = $font_obj + 2;

                        offsets.push(pdf.len());
                        pdf.extend_from_slice(format!(
                            "{} 0 obj\n<< /Type /Font /Subtype /TrueType /BaseFont /{} /FirstChar 32 /LastChar 255 /Widths [{}] /FontDescriptor {} 0 R /Encoding /WinAnsiEncoding >>\nendobj\n",
                            $font_obj, pdf_name, widths_str, fd_obj
                        ).as_bytes());

                        offsets.push(pdf.len());
                        pdf.extend_from_slice(format!(
                            "{} 0 obj\n<< /Type /FontDescriptor /FontName /{} /Flags 32 /FontBBox {} /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV {} /FontFile2 {} 0 R >>\nendobj\n",
                            fd_obj, pdf_name, bbox, (asc * scale).round() as i64, (desc * scale).round() as i64, cap_h, stem_v, ff_obj
                        ).as_bytes());

                        offsets.push(pdf.len());
                        pdf.extend_from_slice(format!(
                            "{} 0 obj\n<< /Length {} /Length1 {} >>\nstream\n", ff_obj, data.len(), data.len()
                        ).as_bytes());
                        pdf.extend_from_slice(data);
                        pdf.extend_from_slice(b"\nendstream\nendobj\n");
                    } else {
                        offsets.push(pdf.len());
                        pdf.extend_from_slice(format!(
                            "{} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>\nendobj\n",
                            $font_obj, $fallback
                        ).as_bytes());
                    }
                }
            }

            write_font!(reg_data, font_obj_regular, "Helvetica");
            write_font!(bold_data, font_obj_bold, "Helvetica-Bold");
            write_font!(italic_data, font_obj_italic, "Helvetica-Oblique");
            write_font!(bold_italic_data, font_obj_bold_italic, "Helvetica-BoldOblique");
                        
            
                        // Build page objects and content streams
                        let mut stream_data: Vec<Vec<u8>> = Vec::new();
                          let mut image_xobjects: Vec<Vec<u8>> = Vec::new();
                        for (i, page_val) in pages.iter().enumerate() {
                            if let DgnValue::Map(pm) = page_val {
                                let pm = pm.lock().unwrap();
                                let w = pm.get("width").and_then(|v| v.as_number()).unwrap_or(612.0);
                                let h = pm.get("height").and_then(|v| v.as_number()).unwrap_or(792.0);
                                let ops = if let Some(DgnValue::List(o)) = pm.get("ops") {
                                    o.lock().unwrap().clone()
                                } else { Vec::new() };
                                let mut annots_str = String::new();
                                // Build content stream
                                let mut stream = Vec::<u8>::new();
            
                                for op_val in &ops {
                                    if let DgnValue::Map(op_map) = op_val {
                                        let op = op_map.lock().unwrap();
                                        let op_type = op.get("type").map(val_to_string).unwrap_or_default();
            
                                        match op_type.as_str() {
                                           "text" => {
                                                    let raw_text = op.get("text").map(val_to_string).unwrap_or_default();
                                                    let x    = op.get("x").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                    let y    = op.get("y").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                    let sz   = op.get("size").and_then(|v| v.as_number()).unwrap_or(12.0);

                                                    if let Some(mut target_url) = op.get("url").map(val_to_string) {
                                                        if !target_url.is_empty() && target_url != "null" {
                                                            // Force absolute protocol
                                                            if !target_url.starts_with("http") && !target_url.starts_with("mailto:") {
                                                                target_url = format!("https://{}", target_url);
                                                            }
                                                            
                                                            let est_width = raw_text.len() as f64 * sz * 0.5;
                                                            
                                                            annots_str.push_str(&format!(
                                                                "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /A << /S /URI /URI ({}) >> >> ",
                                                                x, y - (sz * 0.2), x + est_width, y + sz, target_url
                                                            ));
                                                        }
                                                    }
                                                    
                                                    let r    = op.get("r").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                    let g    = op.get("g").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                    let b    = op.get("b").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                    let bold = op.get("bold")
                                                        .map(|v| if let DgnValue::Bool(bv) = v { *bv } else { false })
                                                        .unwrap_or(false);

                                                    let italic = op.get("italic")
                                                    .map(|v| if let DgnValue::Bool(bv) = v { *bv } else { false })
                                                    .unwrap_or(false);

                                                    let font_ref = match (bold, italic) {
                                                        (true, true) => "F4",   // Bold Italic
                                                        (true, false) => "F2",  // Bold
                                                        (false, true) => "F3",  // Italic
                                                        (false, false) => "F1", // Regular
                                                    };
                                                    let box_size = sz * 0.75;

                                                    let mut cur_x = x;
                                                    let mut escaped_buf = String::new();
                                                    let mut char_count = 0.0;

                                                    for ch in raw_text.chars() {
                                                        match ch {
                                                            '\u{2610}' | '\u{2611}' | '\u{2612}' => {
                                                                // Flush buffered normal text first
                                                                if !escaped_buf.is_empty() {
                                                                    stream.extend_from_slice(
                                                                        format!("BT\n{} {} {} rg\n/{} {} Tf\n{} {} Td\n({}) Tj\nET\n",
                                                                            r, g, b, font_ref, sz, cur_x, y, escaped_buf
                                                                        ).as_bytes()
                                                                    );
                                                                    // Approximate advance for flushed text based on visual characters
                                                                    cur_x += char_count as f64 * sz * 0.5;
                                                                    escaped_buf.clear();
                                                                    char_count = 0.0;
                                                                }
                                                                // Draw unfilled square for checkbox
                                                                stream.extend_from_slice(
                                                                    format!("{} {} {} RG\n0.5 w\n{} {} {} {} re\nS\n",
                                                                        r, g, b, cur_x, y, box_size, box_size
                                                                    ).as_bytes()
                                                                );
                                                                cur_x += box_size + 2.0;
                                                            }
                                                            _ => {
                                                                let code = ch as u32;
                                                                if code >= 32 && code <= 126 {
                                                                    // ASCII: Handle standard PDF text escaping directly
                                                                    if ch == '\\' || ch == '(' || ch == ')' {
                                                                        escaped_buf.push('\\');
                                                                    }
                                                                    escaped_buf.push(ch);
                                                                    char_count += 1.0;
                                                                } else {
                                                                    // Map common Unicode chars to WinAnsiEncoding octal equivalents
                                                                    let (replacement, width) = match ch {
                                                                        '\u{201C}' => ("\\223", 1.0), // smart double quote (left)
                                                                        '\u{201D}' => ("\\224", 1.0), // smart double quote (right)
                                                                        '\u{2018}' => ("\\221", 1.0), // smart single quote (left)
                                                                        '\u{2019}' => ("\\222", 1.0), // smart single quote (right)
                                                                        '\u{2013}' => ("\\226", 1.0), // en-dash
                                                                        '\u{2014}' => ("\\227", 1.0), // em-dash
                                                                        '\u{2022}' => ("\\225", 1.0), // bullet (WinAnsi 0x95)
                                                                        '\u{2026}' => ("\\205", 1.0), // ellipsis
                                                                        '\u{2122}' => ("\\231", 2.0), // trademark
                                                                        '\u{00A9}' => ("\\251", 1.0), // copyright
                                                                        '\u{00AE}' => ("\\256", 1.0), // registered
                                                                        '\u{00B0}' => ("\\260", 1.0), // degree sign
                                                                        '\u{00D7}' => ("\\327", 1.0), // multiplication sign
                                                                        '\u{00A0}' => (" ", 1.0),     // non-breaking space
                                                                        '\u{2265}' => (">=", 2.0),    // greater than or equal
                                                                        '\u{2264}' => ("<=", 2.0),    // less than or equal
                                                                        _ if code > 126 && code <= 255 => {
                                                                            // Standard Latin-1 supplement mapping to octal
                                                                            escaped_buf.push_str(&format!("\\{:03o}", code));
                                                                            char_count += 1.0;
                                                                            continue;
                                                                        },
                                                                        _ => ("?", 1.0), // unknown -> placeholder
                                                                    };
                                                                    escaped_buf.push_str(replacement);
                                                                    char_count += width;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    
                                                    // Flush any remaining normal text
                                                    if !escaped_buf.is_empty() {
                                                        stream.extend_from_slice(
                                                            format!("BT\n{} {} {} rg\n/{} {} Tf\n{} {} Td\n({}) Tj\nET\n",
                                                                r, g, b, font_ref, sz, cur_x, y, escaped_buf
                                                            ).as_bytes()
                                                        );
                                                    }
                                                }
                                            "line" => {
                                                let x1 = op.get("x1").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let y1 = op.get("y1").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let x2 = op.get("x2").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let y2 = op.get("y2").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let lw = op.get("w").and_then(|v| v.as_number()).unwrap_or(1.0);
                                                let r = op.get("r").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let g = op.get("g").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let b = op.get("b").and_then(|v| v.as_number()).unwrap_or(0.0);
            
                                                stream.extend_from_slice(
                                                    format!("{} {} {} RG\n{} w\n{} {} m\n{} {} l\nS\n",
                                                        r, g, b, lw, x1, y1, x2, y2
                                                    ).as_bytes()
                                                );
                                            }
                                            "image" => {
                                                let ix = op.get("x").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let iy = op.get("y").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let iw = op.get("w").and_then(|v| v.as_number()).unwrap_or(50.0);
                                                let ih = op.get("h").and_then(|v| v.as_number()).unwrap_or(50.0);

                                                if let Some(DgnValue::ByteArray(ba)) = op.get("data") {
                                                    let jpeg_bytes = ba.lock().unwrap().clone();
                                                    if !jpeg_bytes.is_empty() {
                                                        // Index for this image XObject (tracked via stream length as unique key)
                                                        // We embed the image inline using a Do operator referencing /Im0, /Im1 etc.
                                                        // The actual XObject is written after content streams — use page-local index.
                                                        // Simple approach: store count in stream via a sentinel we can match later.
                                                        // Instead, we write the XObject immediately and track the index via a separate vec.
                                                        // Since we're inside the page loop, use image_xobjects defined before this loop.
                                                        let img_idx = image_xobjects.len();
                                                        image_xobjects.push(jpeg_bytes);
                                                        stream.extend_from_slice(
                                                            format!("q\n{} 0 0 {} {} {} cm\n/Im{} Do\nQ\n",
                                                                iw, ih, ix, iy, img_idx
                                                            ).as_bytes()
                                                        );
                                                    }
                                                }
                                            }
                                            "rect" => {
                                                let x = op.get("x").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let y = op.get("y").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let rw = op.get("w").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let rh = op.get("h").and_then(|v| v.as_number()).unwrap_or(0.0);
                                                let r = op.get("r").and_then(|v| v.as_number()).unwrap_or(1.0);
                                                let g = op.get("g").and_then(|v| v.as_number()).unwrap_or(1.0);
                                                let b = op.get("b").and_then(|v| v.as_number()).unwrap_or(1.0);
                                                let filled = op.get("filled")
                                                    .map(|v| if let DgnValue::Bool(b) = v { *b } else { true })
                                                    .unwrap_or(true);
            
                                                if filled {
                                                    stream.extend_from_slice(
                                                        format!("{} {} {} rg\n{} {} {} {} re\nf\n",
                                                            r, g, b, x, y, rw, rh
                                                        ).as_bytes()
                                                    );
                                                } else {
                                                    stream.extend_from_slice(
                                                        format!("{} {} {} RG\n{} {} {} {} re\nS\n",
                                                            r, g, b, x, y, rw, rh
                                                        ).as_bytes()
                                                    );
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
            
                                stream_data.push(stream);
            
                                offsets.push(pdf.len());
                            
                               let xobj_str: String = image_xobjects.iter().enumerate()
                                    .map(|(idx, _)| format!("/Im{} {} 0 R", idx, first_stream_obj + num_pages + idx))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                
                                // NEW: Includes /F3 and /F4 so italics actually load
                                let resources = if xobj_str.is_empty() {
                                    format!("/Font << /F1 {} 0 R /F2 {} 0 R /F3 {} 0 R /F4 {} 0 R >>", font_obj_regular, font_obj_bold, font_obj_italic, font_obj_bold_italic)
                                } else {
                                    format!("/Font << /F1 {} 0 R /F2 {} 0 R /F3 {} 0 R /F4 {} 0 R >> /XObject << {} >>",
                                        font_obj_regular, font_obj_bold, font_obj_italic, font_obj_bold_italic, xobj_str)
                                };

                                let annots_entry = if annots_str.is_empty() { 
                                    String::new() 
                                } else { 
                                    format!("/Annots [ {} ]", annots_str) 
                                };
                                
                                // NEW: Added the `{}` placeholder for annots_entry so URLs are actually clickable!
                                pdf.extend_from_slice(
                                    format!(
                                        "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {} 0 R /Resources << {} >> {} >>\nendobj\n",
                                        first_page_obj + i, w, h, first_stream_obj + i, resources, annots_entry
                                    ).as_bytes()
                                );
                            }
                        }
            
                        // Write content stream objects
                        for (i, stream) in stream_data.iter().enumerate() {
                            offsets.push(pdf.len());
                            pdf.extend_from_slice(
                                format!("{} 0 obj\n<< /Length {} >>\nstream\n",
                                    first_stream_obj + i, stream.len()
                                ).as_bytes()
                            );
                            pdf.extend_from_slice(stream);
                            pdf.extend_from_slice(b"\nendstream\nendobj\n");
                        }

                         // Write image XObject objects (after all content streams)
                        for (img_idx, raw_bytes) in image_xobjects.iter().enumerate() {
                            let bytes = raw_bytes.as_slice();
 
                            // Detect format: PNG starts with 89 50 4E 47, JPEG with FF D8 FF
                            let is_png = bytes.len() >= 4 
                                && bytes[0] == 0x89 && bytes[1] == 0x50 
                                && bytes[2] == 0x4E && bytes[3] == 0x47;
                            let is_jpeg = bytes.len() >= 3 
                                && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF;
 
                            if is_jpeg {
                                // JPEG: embed directly with DCTDecode (existing behavior)
                                let (img_w, img_h) = {
                                    let mut w = 1u32; let mut h = 1u32;
                                    let mut pos = 0usize;
                                    while pos + 3 < bytes.len() {
                                        if bytes[pos] == 0xFF {
                                            let marker = bytes[pos + 1];
                                            if marker == 0xC0 || marker == 0xC2 {
                                                if pos + 8 < bytes.len() {
                                                    h = ((bytes[pos+5] as u32) << 8) | bytes[pos+6] as u32;
                                                    w = ((bytes[pos+7] as u32) << 8) | bytes[pos+8] as u32;
                                                }
                                                break;
                                            }
                                            // Handle markers without length fields
                                            if marker == 0xD9 { break; } // EOI
                                            if marker == 0xD8 || marker == 0x00 || marker == 0x01 
                                                || (marker >= 0xD0 && marker <= 0xD7) {
                                                pos += 2;
                                                continue;
                                            }
                                            let seg_len = if pos + 3 < bytes.len() {
                                                ((bytes[pos+2] as usize) << 8) | bytes[pos+3] as usize
                                            } else { break };
                                            pos += 2 + seg_len;
                                        } else { pos += 1; }
                                    }
                                    (w, h)
                                };
 
                                offsets.push(pdf.len());
                                pdf.extend_from_slice(
                                    format!(
                                        "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                                        /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                                        first_stream_obj + num_pages + img_idx,
                                        img_w, img_h, raw_bytes.len()
                                    ).as_bytes()
                                );
                                pdf.extend_from_slice(raw_bytes);
                                pdf.extend_from_slice(b"\nendstream\nendobj\n");
 
                            } else {
                                // PNG or any other format: decode to RGB, compress with flate
                                match image::load_from_memory(raw_bytes) {
                                    Ok(img) => {
                                        let (img_w, img_h) = img.dimensions();
                                        let rgba = img.to_rgba8();
                                        
                                        // Check if image has meaningful alpha
                                        let has_alpha = rgba.pixels().any(|p| p[3] < 250);
                                        
                                        // Extract RGB pixels
                                        let mut rgb_data = Vec::with_capacity((img_w * img_h * 3) as usize);
                                        let mut alpha_data = if has_alpha {
                                            Vec::with_capacity((img_w * img_h) as usize)
                                        } else {
                                            Vec::new()
                                        };
                                        
                                        for pixel in rgba.pixels() {
                                            let a = pixel[3] as f32 / 255.0;
                                            // Pre-multiply with white background for transparency
                                            rgb_data.push(((pixel[0] as f32 * a + 255.0 * (1.0 - a)) as u8));
                                            rgb_data.push(((pixel[1] as f32 * a + 255.0 * (1.0 - a)) as u8));
                                            rgb_data.push(((pixel[2] as f32 * a + 255.0 * (1.0 - a)) as u8));
                                            if has_alpha {
                                                alpha_data.push(pixel[3]);
                                            }
                                        }
                                        
                                        // Compress RGB data with flate
                                        use flate2::write::ZlibEncoder;
                                        use flate2::Compression;
                                        use std::io::Write;
                                        
                                        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                                        encoder.write_all(&rgb_data).unwrap_or(());
                                        let compressed = encoder.finish().unwrap_or_default();
 
                                        offsets.push(pdf.len());
                                        pdf.extend_from_slice(
                                            format!(
                                                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                                                /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
                                                first_stream_obj + num_pages + img_idx,
                                                img_w, img_h, compressed.len()
                                            ).as_bytes()
                                        );
                                        pdf.extend_from_slice(&compressed);
                                        pdf.extend_from_slice(b"\nendstream\nendobj\n");
                                    },
                                    Err(_e) => {
                                        // Failed to decode — write a 1x1 placeholder
                                        offsets.push(pdf.len());
                                        let placeholder = vec![255u8, 255, 255]; // white pixel
                                        pdf.extend_from_slice(
                                            format!(
                                                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width 1 /Height 1 \
                                                /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n",
                                                first_stream_obj + num_pages + img_idx
                                            ).as_bytes()
                                        );
                                        pdf.extend_from_slice(&placeholder);
                                        pdf.extend_from_slice(b"\nendstream\nendobj\n");
                                    }
                                }
                            }
                        }
            
                        // Cross-reference table
                        let xref_offset = pdf.len();
                        let total_objects = offsets.len() + 1; // +1 for object 0
                        pdf.extend_from_slice(
                            format!("xref\n0 {}\n", total_objects).as_bytes()
                        );
                        pdf.extend_from_slice(b"0000000000 65535 f \n");
                        for off in &offsets {
                            pdf.extend_from_slice(
                                format!("{:010} 00000 n \n", off).as_bytes()
                            );
                        }
            
                        // Trailer
                        pdf.extend_from_slice(
                            format!(
                                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                                total_objects, xref_offset
                            ).as_bytes()
                        );
            
                        Ok(DgnValue::ByteArray(Arc::new(Mutex::new(pdf))))
                    } else {
                        Err(DgnError::new("Doc handle required", 0))
                    }
                }));
            
                Some(DgnValue::Map(Arc::new(Mutex::new(map))))
            },

        "font" => {
            let mut map = HashMap::new();
        
            // font.load(path) → font handle (ByteArray containing font data + metadata)
            // We store font data as a map: { "__font_data": ByteArray, "family": String, ... }
            map.insert("load".to_string(), native_fn(|args| {
                let path = args.get(0).map(val_to_string)
                    .ok_or(DgnError::new("Font file path required", 0))?;
        
                let data = std::fs::read(&path)
                    .map_err(|e| DgnError::new(&format!("Failed to read font: {}", e), 0))?;
        
                let face = ttf_parser::Face::parse(&data, 0)
                    .map_err(|e| DgnError::new(&format!("Invalid font file: {}", e), 0))?;

                // Extract everything from face BEFORE moving data
                let family = face.names()
                    .into_iter()
                    .find(|n| n.name_id == ttf_parser::name_id::FAMILY)
                    .and_then(|n| n.to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                let units_per_em = face.units_per_em() as f64;
                let ascender = face.ascender() as f64;
                let descender = face.descender() as f64;
                let line_gap = face.line_gap() as f64;
                drop(face); // release borrow

                let mut m = HashMap::new();
                m.insert("__font_data".to_string(),
                    DgnValue::ByteArray(Arc::new(Mutex::new(data))));
                m.insert("family".to_string(), DgnValue::String(family));
                m.insert("units_per_em".to_string(), DgnValue::Number(units_per_em));
                m.insert("ascender".to_string(), DgnValue::Number(ascender));
                m.insert("descender".to_string(), DgnValue::Number(descender));
                m.insert("line_gap".to_string(), DgnValue::Number(line_gap));
        
                Ok(DgnValue::Map(Arc::new(Mutex::new(m))))
            }));


            // font.load_bytes(family_name, ByteArray) → font handle
                map.insert("load_bytes".to_string(), native_fn(|args| {
                    let family_name = args.get(0).map(val_to_string)
                        .ok_or(DgnError::new("Font family name required", 0))?;
                    let font_data = match args.get(1) {
                        Some(DgnValue::ByteArray(b)) => b.lock().unwrap().clone(),
                        _ => return Err(DgnError::new("ByteArray required as second argument", 0)),
                    };

                    let face = ttf_parser::Face::parse(&font_data, 0)
                        .map_err(|e| DgnError::new(&format!("Invalid font data: {}", e), 0))?;

                    // Extract metrics before face is dropped
                    let family = face.names()
                        .into_iter()
                        .find(|n| n.name_id == ttf_parser::name_id::FAMILY)
                        .and_then(|n| n.to_string())
                        .unwrap_or_else(|| family_name.clone());
                    let units_per_em = face.units_per_em() as f64;
                    let ascender = face.ascender() as f64;
                    let descender = face.descender() as f64;
                    let line_gap = face.line_gap() as f64;
                    drop(face);

                    let mut m = HashMap::new();
                    m.insert("__font_data".to_string(),
                        DgnValue::ByteArray(Arc::new(Mutex::new(font_data))));
                    m.insert("family".to_string(), DgnValue::String(family));
                    m.insert("units_per_em".to_string(), DgnValue::Number(units_per_em));
                    m.insert("ascender".to_string(), DgnValue::Number(ascender));
                    m.insert("descender".to_string(), DgnValue::Number(descender));
                    m.insert("line_gap".to_string(), DgnValue::Number(line_gap));

                    Ok(DgnValue::Map(Arc::new(Mutex::new(m))))
                }));
        
            // font.measure(font_handle, text, size_pt) → width in points
            map.insert("measure".to_string(), native_fn(|args| {
                let font_handle = args.get(0).ok_or(DgnError::new("Font handle required", 0))?;
                let text = args.get(1).map(val_to_string).unwrap_or_default();
                let size_pt = args.get(2).and_then(|v| v.as_number()).unwrap_or(12.0);
        
                if let DgnValue::Map(m) = font_handle {
                    let m = m.lock().unwrap();
                    let data = if let Some(DgnValue::ByteArray(b)) = m.get("__font_data") {
                        b.lock().unwrap().clone()
                    } else {
                        return Err(DgnError::new("Invalid font handle", 0));
                    };
                    let upem = if let Some(DgnValue::Number(n)) = m.get("units_per_em") {
                        *n
                    } else { 1000.0 };
        
                    let face = ttf_parser::Face::parse(&data, 0)
                        .map_err(|e| DgnError::new(&format!("Font parse error: {}", e), 0))?;
        
                    let scale = size_pt / upem;
                    let mut total_width: f64 = 0.0;
        
                    for ch in text.chars() {
                        if let Some(glyph_id) = face.glyph_index(ch) {
                            let advance = face.glyph_hor_advance(glyph_id).unwrap_or(0);
                            total_width += advance as f64 * scale;
                        } else {
                            // Fallback: use average width estimate
                            total_width += upem * 0.5 * scale;
                        }
                    }
        
                    Ok(DgnValue::Number(total_width))
                } else {
                    Err(DgnError::new("First argument must be a font handle from font.load()", 0))
                }
            }));
        
            // font.line_height(font_handle, size_pt) → line height in points
            map.insert("line_height".to_string(), native_fn(|args| {
                let font_handle = args.get(0).ok_or(DgnError::new("Font handle required", 0))?;
                let size_pt = args.get(1).and_then(|v| v.as_number()).unwrap_or(12.0);
        
                if let DgnValue::Map(m) = font_handle {
                    let m = m.lock().unwrap();
                    let upem = if let Some(DgnValue::Number(n)) = m.get("units_per_em") { *n } else { 1000.0 };
                    let asc = if let Some(DgnValue::Number(n)) = m.get("ascender") { *n } else { 800.0 };
                    let desc = if let Some(DgnValue::Number(n)) = m.get("descender") { *n } else { -200.0 };
                    let gap = if let Some(DgnValue::Number(n)) = m.get("line_gap") { *n } else { 0.0 };
        
                    let scale = size_pt / upem;
                    let height = (asc - desc + gap) * scale;
                    Ok(DgnValue::Number(height))
                } else {
                    Err(DgnError::new("Font handle required", 0))
                }
            }));
        
            // font.char_width(font_handle, char, size_pt) → width of single character
            map.insert("char_width".to_string(), native_fn(|args| {
                let font_handle = args.get(0).ok_or(DgnError::new("Font handle required", 0))?;
                let ch_str = args.get(1).map(val_to_string).unwrap_or_default();
                let size_pt = args.get(2).and_then(|v| v.as_number()).unwrap_or(12.0);
                let ch = ch_str.chars().next().unwrap_or(' ');
        
                if let DgnValue::Map(m) = font_handle {
                    let m = m.lock().unwrap();
                    let data = if let Some(DgnValue::ByteArray(b)) = m.get("__font_data") {
                        b.lock().unwrap().clone()
                    } else {
                        return Err(DgnError::new("Invalid font handle", 0));
                    };
                    let upem = if let Some(DgnValue::Number(n)) = m.get("units_per_em") { *n } else { 1000.0 };
        
                    let face = ttf_parser::Face::parse(&data, 0)
                        .map_err(|e| DgnError::new(&format!("Font error: {}", e), 0))?;
        
                    let scale = size_pt / upem;
                    let width = if let Some(gid) = face.glyph_index(ch) {
                        face.glyph_hor_advance(gid).unwrap_or(0) as f64 * scale
                    } else {
                        upem * 0.5 * scale
                    };
                    Ok(DgnValue::Number(width))
                } else {
                    Err(DgnError::new("Font handle required", 0))
                }
            }));
        
            // font.find(family_name) → font handle or null (searches system fonts)
            map.insert("find".to_string(), native_fn(|args| {
                  let family = args.get(0).map(val_to_string).unwrap_or_default();
                    // NEW: optional second arg — true means bold variant
                    let want_bold = args.get(1).map(|v| is_truthy(v)).unwrap_or(false);
                    let want_italic = args.get(2).map(|v| is_truthy(v)).unwrap_or(false);
                    let mut db = fontdb::Database::new();
                    db.load_system_fonts();

                    let want_condensed = family.to_lowercase().contains("narrow") 
                        || family.to_lowercase().contains("condensed");

                    // Strip "Condensed"/"Narrow" suffix for the family lookup if needed
                    let lookup_family = family
                        .replace("Condensed", "").replace("Narrow", "")
                        .trim().to_string();
                    let lookup_family = if want_condensed && lookup_family.len() > 0 { 
                        lookup_family 
                    } else { 
                        family.clone() 
                    };

                    let query = fontdb::Query {
                        families: &[fontdb::Family::Name(&lookup_family)],
                        weight: if want_bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
                        stretch: if want_condensed { fontdb::Stretch::Condensed } else { fontdb::Stretch::Normal },

                        style: if want_italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
                        ..fontdb::Query::default()
                    };
        
                if let Some(id) = db.query(&query) {
                    if let Some(face_info) = db.face(id) {
                        // Get the font file path
                        if let fontdb::Source::File(path) = &face_info.source {
                            let data = std::fs::read(path)
                                .map_err(|e| DgnError::new(&format!("Failed to read font: {}", e), 0))?;
                            let face = ttf_parser::Face::parse(&data, face_info.index)
                                .map_err(|e| DgnError::new(&format!("Font parse error: {}", e), 0))?;

                            // Extract before moving data
                            let upem = face.units_per_em() as f64;
                            let asc = face.ascender() as f64;
                            let desc = face.descender() as f64;
                            let lgap = face.line_gap() as f64;
                            let font_path = path.to_string_lossy().to_string();
                            drop(face);

                            let mut m = HashMap::new();
                            m.insert("__font_data".to_string(),
                                DgnValue::ByteArray(Arc::new(Mutex::new(data))));
                            m.insert("family".to_string(), DgnValue::String(family));
                            m.insert("units_per_em".to_string(), DgnValue::Number(upem));
                            m.insert("ascender".to_string(), DgnValue::Number(asc));
                            m.insert("descender".to_string(), DgnValue::Number(desc));
                            m.insert("line_gap".to_string(), DgnValue::Number(lgap));
                            m.insert("path".to_string(), DgnValue::String(font_path));
        
                            return Ok(DgnValue::Map(Arc::new(Mutex::new(m))));
                        }
                    }
                }
                Ok(DgnValue::Null)
            }));
        
            // font.list() → list of available system font family names
            map.insert("list".to_string(), native_fn(|_args| {
                let mut db = fontdb::Database::new();
                db.load_system_fonts();
        
                let mut families: Vec<String> = db.faces()
                    .map(|f| {
                        f.families.first()
                            .map(|(name, _)| name.clone())
                            .unwrap_or_default()
                    })
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                families.sort();
        
                let list: Vec<DgnValue> = families.into_iter()
                    .map(DgnValue::String)
                    .collect();
                Ok(DgnValue::List(Arc::new(Mutex::new(list))))
            }));
        
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "xml" => {
            let mut map = HashMap::new();
        
            // Helper: convert roxmltree node into a DgnValue tree (recursive)
            fn xml_node_to_dgn(node: &roxmltree::Node) -> DgnValue {
                if node.is_text() {
                    return DgnValue::String(node.text().unwrap_or("").to_string());
                }
                if !node.is_element() {
                    return DgnValue::Null;
                }
        
                let mut m = HashMap::new();
        
                // Tag name — includes namespace prefix if present
                let tag = if let Some(prefix) = node.tag_name().namespace() {
                    // Find the prefix used in the document for this namespace
                    let pfx = node.lookup_prefix(prefix).unwrap_or("");
                    if pfx.is_empty() {
                        node.tag_name().name().to_string()
                    } else {
                        format!("{}:{}", pfx, node.tag_name().name())
                    }
                } else {
                    node.tag_name().name().to_string()
                };
                m.insert("tag".to_string(), DgnValue::String(tag));
        
                // Local name (without prefix)
                m.insert("local".to_string(), DgnValue::String(
                    node.tag_name().name().to_string()
                ));
        
                // Namespace URI
                if let Some(ns) = node.tag_name().namespace() {
                    m.insert("ns".to_string(), DgnValue::String(ns.to_string()));
                }
        
                // Attributes as a map
                let mut attrs = HashMap::new();
                for attr in node.attributes() {
                    let key = if let Some(ns) = attr.namespace() {
                        // Try to find a prefix for this attribute namespace
                        let pfx = node.lookup_prefix(ns).unwrap_or("");
                        if pfx.is_empty() {
                            attr.name().to_string()
                        } else {
                            format!("{}:{}", pfx, attr.name())
                        }
                    } else {
                        attr.name().to_string()
                    };
                    attrs.insert(key, DgnValue::String(attr.value().to_string()));
                }
                m.insert("attrs".to_string(), DgnValue::Map(Arc::new(Mutex::new(attrs))));
        
                // Direct text content (concatenated text of immediate text children)
                let text: String = node.children()
                    .filter(|c| c.is_text())
                    .filter_map(|c| c.text())
                    .collect();
                m.insert("text".to_string(), DgnValue::String(text));
        
                // All text content recursively
                fn collect_text(n: &roxmltree::Node) -> String {
                    let mut s = String::new();
                    for child in n.children() {
                        if child.is_text() {
                            s.push_str(child.text().unwrap_or(""));
                        } else if child.is_element() {
                            s.push_str(&collect_text(&child));
                        }
                    }
                    s
                }
                m.insert("inner_text".to_string(), DgnValue::String(collect_text(&node)));
        
                // Child elements (skip text nodes at this level — they're in "text")
                let children: Vec<DgnValue> = node.children()
                    .filter(|c| c.is_element())
                    .map(|c| xml_node_to_dgn(&c))
                    .collect();
                m.insert("children".to_string(), DgnValue::List(Arc::new(Mutex::new(children))));
        
                DgnValue::Map(Arc::new(Mutex::new(m)))
            }
        
            // xml.parse(string) → tree of maps
            map.insert("parse".to_string(), native_fn(|args| {
                let xml_str = args.get(0).map(val_to_string).unwrap_or_default();
                match roxmltree::Document::parse(&xml_str) {
                    Ok(doc) => {
                        let root = doc.root_element();
                        Ok(xml_node_to_dgn(&root))
                    },
                    Err(e) => Err(DgnError::new(&format!("XML parse error: {}", e), 0))
                }
            }));
        
            // xml.find(node_map, tag_name) → first matching child element or null
            map.insert("find".to_string(), native_fn(|args| {
                let node = args.get(0).unwrap_or(&DgnValue::Null).clone();
                let tag = args.get(1).map(val_to_string).unwrap_or_default();
        
                if let DgnValue::Map(m) = &node {
                    let m = m.lock().unwrap();
                    if let Some(DgnValue::List(children)) = m.get("children") {
                        let children = children.lock().unwrap();
                        for child in children.iter() {
                            if let DgnValue::Map(cm) = child {
                                let cm = cm.lock().unwrap();
                                if let Some(DgnValue::String(t)) = cm.get("tag") {
                                    if *t == tag {
                                        return Ok(child.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(DgnValue::Null)
            }));
        
            // xml.find_all(node_map, tag_name) → list of all descendant elements matching tag
            map.insert("find_all".to_string(), native_fn(|args| {
                let node = args.get(0).unwrap_or(&DgnValue::Null).clone();
                let tag = args.get(1).map(val_to_string).unwrap_or_default();
        
                fn find_recursive(node: &DgnValue, tag: &str, results: &mut Vec<DgnValue>) {
                    if let DgnValue::Map(m) = node {
                        let m = m.lock().unwrap();
                        // Check this node's tag
                        if let Some(DgnValue::String(t)) = m.get("tag") {
                            if *t == tag {
                                results.push(node.clone());
                            }
                        }
                        // Recurse into children
                        if let Some(DgnValue::List(children)) = m.get("children") {
                            let children = children.lock().unwrap();
                            for child in children.iter() {
                                find_recursive(child, tag, results);
                            }
                        }
                    }
                }
        
                let mut results = Vec::new();
                // Search children (not the root itself for find_all)
                if let DgnValue::Map(m) = &node {
                    let m = m.lock().unwrap();
                    if let Some(DgnValue::List(children)) = m.get("children") {
                        let children = children.lock().unwrap();
                        for child in children.iter() {
                            find_recursive(child, &tag, &mut results);
                        }
                    }
                }
        
                Ok(DgnValue::List(Arc::new(Mutex::new(results))))
            }));
        
            // xml.attr(node_map, attr_name) → attribute value or null
            map.insert("attr".to_string(), native_fn(|args| {
                let node = args.get(0).unwrap_or(&DgnValue::Null).clone();
                let attr_name = args.get(1).map(val_to_string).unwrap_or_default();
        
                if let DgnValue::Map(m) = &node {
                    let m = m.lock().unwrap();
                    if let Some(DgnValue::Map(attrs)) = m.get("attrs") {
                        let attrs = attrs.lock().unwrap();
                        if let Some(val) = attrs.get(&attr_name) {
                            return Ok(val.clone());
                        }
                    }
                }
                Ok(DgnValue::Null)
            }));
        
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
 
        "uuid" => {
            let mut map = HashMap::new();
            map.insert("v4".to_string(), native_fn(|_| {
                let u1: u32 = rand::random();
                let u2: u32 = rand::random();
                let u3: u32 = rand::random();
                let u4: u32 = rand::random();
                let uuid = format!("{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                    u1,
                    (u2 >> 16) as u16,
                    (u2 & 0xFFF) as u16,
                    ((u3 >> 16) & 0x3FFF | 0x8000) as u16,
                    (((u3 & 0xFFFF) as u64) << 32) | (u4 as u64)
                );
                Ok(DgnValue::String(uuid))
            }));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "os" => {
            let mut map = HashMap::new();
            #[cfg(not(any(target_arch = "wasm32")))]
            {
                map.insert("name".to_string(), native_fn(|_| Ok(DgnValue::String(std::env::consts::OS.to_string()))));
                map.insert("arch".to_string(), native_fn(|_| Ok(DgnValue::String(std::env::consts::ARCH.to_string()))));
                map.insert("family".to_string(), native_fn(|_| Ok(DgnValue::String(std::env::consts::FAMILY.to_string()))));
                map.insert("system".to_string(), native_fn(|args| {
                    let cmd = if let Some(DgnValue::String(s)) = args.get(0) { s } else { "" };
                    if cfg!(target_os = "windows") {
                        Command::new("cmd").args(["/C", cmd]).status().map(|s| DgnValue::Number(s.code().unwrap_or(0) as f64)).map_err(|e| DgnError::new(&e.to_string(),0))
                    } else {
                        Command::new("sh").args(["-c", cmd]).status().map(|s| DgnValue::Number(s.code().unwrap_or(0) as f64)).map_err(|e| DgnError::new(&e.to_string(),0))
                    }
                }));
                map.insert("setenv".to_string(), native_fn(|args| {
                    let k = args.get(0).map(val_to_string).unwrap_or_default();
                    let v = args.get(1).map(val_to_string).unwrap_or_default();
                    std::env::set_var(k, v);
                    Ok(DgnValue::Null)
                }));
                
                map.insert("cwd".to_string(), native_fn(|_| {
                    match std::env::current_dir() {
                        Ok(path) => Ok(DgnValue::String(path.to_string_lossy().into_owned())),
                        Err(_) => Ok(DgnValue::Null)
                    }
                }));
                
                map.insert("chdir".to_string(), native_fn(|args| {
                    let path = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::env::set_current_dir(path).is_ok()))
                }));
                    map.insert("listdir".to_string(), native_fn(|args| {
                        let path = args.get(0).map(val_to_string).unwrap_or_default();
                    let mut files = Vec::new();
                    if let Ok(entries) = fs::read_dir(path) {
                        for entry in entries {
                            if let Ok(e) = entry { if let Ok(name) = e.file_name().into_string() { files.push(DgnValue::String(name)); } }
                        }
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(files))))
                }));
            }
            #[cfg(target_arch = "wasm32")]
            { map.insert("name".to_string(), native_fn(|_| Ok(DgnValue::String("web-wasm".into())))); }
            map.insert("getenv".to_string(), native_fn(|args| {
                if let Some(DgnValue::String(k)) = args.get(0) { Ok(DgnValue::String(env::var(k).unwrap_or_default())) } else { Ok(DgnValue::Null) }
            }));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "path" => {
            let mut map = HashMap::new();
            
            // Joins multiple path segments using the OS-specific separator
            map.insert("join".to_string(), native_fn(|args| {
                let mut path = std::path::PathBuf::new();
                for arg in args {
                    path.push(val_to_string(&arg));
                }
                Ok(DgnValue::String(path.to_string_lossy().into_owned()))
            }));

            // Gets the final component of a path (e.g., "file.txt" from "/dir/file.txt")
            map.insert("basename".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                let name = std::path::Path::new(&p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(DgnValue::String(name))
            }));

            // Gets the directory portion of a path (e.g., "/dir" from "/dir/file.txt")
            map.insert("dirname".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                let dir = std::path::Path::new(&p)
                    .parent()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(DgnValue::String(dir))
            }));

            // Gets the extension of a file path (e.g., ".txt" from "file.txt")
            map.insert("extname".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                let ext = std::path::Path::new(&p)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                Ok(DgnValue::String(ext))
            }));

            // Checks if a path is absolute
            map.insert("is_absolute".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                Ok(DgnValue::Bool(std::path::Path::new(&p).is_absolute()))
            }));

            // Resolves a path to its absolute, normalized format (resolving symlinks and '..')
            #[cfg(not(target_arch = "wasm32"))]
            map.insert("resolve".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                match std::fs::canonicalize(&p) {
                    Ok(canon) => Ok(DgnValue::String(canon.to_string_lossy().into_owned())),
                    Err(_) => Ok(DgnValue::Null) 
                }
            }));

            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "time" =>{
             let mut map = HashMap::new();
            map.insert("now".to_string(), native_fn(|_| { Ok(DgnValue::Number(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64())) }));
            map.insert("unix".to_string(), native_fn(|_| {
                    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
                    Ok(DgnValue::Number(t))
                }));
            map.insert("date".to_string(), native_fn(|args| {
                let now = std::time::SystemTime::now();
                let elapsed = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                
                let days = elapsed / 86400;
                let year = 1970 + (days / 365); 
                
                let mut map1 = HashMap::new();
                map1.insert("timestamp".to_string(), DgnValue::Number(elapsed as f64));
                map1.insert("approx_year".to_string(), DgnValue::Number(year as f64));
                
                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
            }));

            map.insert("date_now".to_string(), native_fn(|_| {
                let now: DateTime<Local> = Local::now();
                let mut map1 = HashMap::new();
                map1.insert("year".to_string(), DgnValue::Number(now.year() as f64));
                map1.insert("month".to_string(), DgnValue::Number(now.month() as f64));
                map1.insert("day".to_string(), DgnValue::Number(now.day() as f64));
                map1.insert("hour".to_string(), DgnValue::Number(now.hour() as f64));
                map1.insert("minute".to_string(), DgnValue::Number(now.minute() as f64));
                map1.insert("second".to_string(), DgnValue::Number(now.second() as f64));
                map1.insert("str".to_string(), DgnValue::String(now.format("%Y-%m-%d %H:%M:%S").to_string()));
                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
            }));

            map.insert("sleep_ms".to_string(), native_fn(|args| {
               let ms = args.get(0).and_then(|v| v.as_number()).map(|n| n as u64).unwrap_or(0);
                thread::sleep(Duration::from_millis(ms));
                Ok(DgnValue::Null)
            }));
            
              Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "net" =>{
            let mut map = HashMap::new();
             #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        {
              map.insert("http_get".to_string(), native_fn(|args| {
                    let url = args.get(0).map(val_to_string).ok_or(DgnError::new("URL required", 0))?;
                    
                    Ok(make_promise(move || {
                        match ureq::get(&url).call() {
                            Ok(resp) => {
                                let status = resp.status();
                                let text = resp.into_string().unwrap_or_default();
                                
                                let mut map1 = HashMap::new();
                                map1.insert("status".to_string(), DgnValue::Number(status as f64));
                                map1.insert("body".to_string(), DgnValue::String(text));
                                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                            },
                             Err(ureq::Error::Status(code, resp)) => {
                                let text = resp.into_string().unwrap_or_default();
                                let mut map1 = HashMap::new();
                                map1.insert("status".to_string(), DgnValue::Number(code as f64));
                                map1.insert("body".to_string(), DgnValue::String(text));
                                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                            },
                            Err(e) => Err(DgnError::new(&format!("HTTP Request failed: {}", e), 0))
                        }
                    }))
                }));

                map.insert("http_post".to_string(), native_fn(|args| {
                    let url = args.get(0).map(val_to_string).ok_or(DgnError::new("URL required", 0))?;
                    let body = args.get(1).map(val_to_string).unwrap_or_default();
                    
                    // Extract headers safely before passing them to the background thread
                    let mut headers_vec = Vec::new();
                    if let Some(DgnValue::Map(headers)) = args.get(2) {
                        for (k, v) in headers.lock().unwrap().iter() {
                            headers_vec.push((k.clone(), val_to_string(v)));
                        }
                    }

                    Ok(make_promise(move || {
                        let mut req = ureq::post(&url);
                        for (k, v) in headers_vec {
                            req = req.set(&k, &v);
                        }

                        match req.send_string(&body) {
                            Ok(resp) => {
                                let status = resp.status();
                                let text = resp.into_string().unwrap_or_default();
                                let mut map1 = HashMap::new();
                                map1.insert("status".to_string(), DgnValue::Number(status as f64));
                                map1.insert("body".to_string(), DgnValue::String(text));
                                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                            },
                             Err(ureq::Error::Status(code, resp)) => {
                                let text = resp.into_string().unwrap_or_default();
                                let mut map1 = HashMap::new();
                                map1.insert("status".to_string(), DgnValue::Number(code as f64));
                                map1.insert("body".to_string(), DgnValue::String(text));
                                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                            },
                            Err(e) => Err(DgnError::new(&format!("HTTP Request failed: {}", e), 0))
                        }
                    }))
                }));

       
                map.insert("socket_connect".to_string(), native_fn(|args| {
                    let addr = args.get(0).map(val_to_string).ok_or(DgnError::new("Address required", 0))?;
                    
                    Ok(make_promise(move || {
                        match TcpStream::connect(&addr) {
                            Ok(stream) => Ok(DgnValue::Socket(Arc::new(Mutex::new(stream)))),
                            Err(e) => Err(DgnError::new(&format!("Connection failed: {}", e), 0))
                        }
                    }))
                }));

                map.insert("socket_send".to_string(), native_fn(|args| {
                    if let Some(DgnValue::Socket(s)) = args.get(0) {
                        let msg = args.get(1).map(val_to_string).unwrap_or_default();
                        let socket_clone = s.clone();
                        
                        Ok(make_promise(move || {
                            let mut stream = socket_clone.lock().unwrap();
                            match stream.write_all(msg.as_bytes()) {
                                Ok(_) => Ok(DgnValue::Bool(true)),
                                Err(_) => Ok(DgnValue::Bool(false))
                            }
                        }))
                    } else {
                        Err(DgnError::new("Socket required", 0))
                    }
                }));

                map.insert("socket_read".to_string(), native_fn(|args| {
                    if let Some(DgnValue::Socket(s)) = args.get(0) {
                        let socket_clone = s.clone();
                        
                        Ok(make_promise(move || {
                            let mut stream = socket_clone.lock().unwrap();
                            let mut reader = BufReader::new(&*stream);
                            let mut buffer = String::new();
                            match reader.read_line(&mut buffer) {
                                Ok(_) => Ok(DgnValue::String(buffer)),
                                Err(_) => Ok(DgnValue::Null)
                            }
                        }))
                    } else {
                        Err(DgnError::new("Socket required", 0))
                    }
                }));
                
                map.insert("socket_close".to_string(), native_fn(|args| {
                    if let Some(DgnValue::Socket(s)) = args.get(0) {
                        let _ = s.lock().unwrap().shutdown(Shutdown::Both);
                        Ok(DgnValue::Bool(true))
                    } else { Ok(DgnValue::Bool(false)) }
                }));

                map.insert("socket_server".to_string(), native_fn(|args| {
                    let port = args.get(0).and_then(|v| v.as_number()).unwrap_or(8080.0) as u16;
                    let handler = args.get(1).cloned().ok_or(DgnError::new("Handler function required", 0))?;
                    
                    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
                        .map_err(|e| DgnError::new(&e.to_string(), 0))?;
                        
                    thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap(); 
                        for stream in listener.incoming() {
                            if let Ok(s) = stream {
                                let client_socket = DgnValue::Socket(Arc::new(Mutex::new(s)));
                                let handler_clone = handler.clone();
                                
                                rt.spawn(async move {
                                    let _ = call_dgn_func(handler_clone, vec![client_socket]).await; 
                                });
                            }
                        }
                    });

                    Ok(DgnValue::Bool(true))
                }));

                    map.insert("start_http_server".to_string(), native_fn(|args| {
                    let port = args.get(0).and_then(|v| v.as_number()).map(|n| n as u16).unwrap_or(8080);
                    let handler = args.get(1).cloned().ok_or(DgnError::new("Handler required", 0))?;
                    thread::spawn(move || {
                        let server = match Server::http(format!("0.0.0.0:{}", port)) {
                            Ok(s) => s,
                            Err(e) => { println!("Failed to start server: {}", e); return; }
                        };
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        for mut request in server.incoming_requests() {
                            let method = request.method().to_string();
                            let url = request.url().to_string();
                            let mut body_bytes: Vec<u8> = Vec::new();
                            std::io::Read::read_to_end(&mut request.as_reader(), &mut body_bytes).unwrap_or(0);
                            let content = String::from_utf8_lossy(&body_bytes).into_owned();
                            
                            let headers_map = Arc::new(Mutex::new(HashMap::new()));
                            for h in request.headers() {
                                headers_map.lock().unwrap().insert(h.field.to_string(), DgnValue::String(h.value.to_string()));
                            }

                            let query_map = Arc::new(Mutex::new(HashMap::new()));
                            if let Some(pos) = url.find('?') {
                                for pair in url[pos+1..].split('&') {
                                    let mut parts = pair.split('=');
                                    if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                                        query_map.lock().unwrap().insert(k.to_string(), DgnValue::String(v.to_string()));
                                    }
                                }
                            }

                            let remote_addr = request.remote_addr()
                                .map(|a| a.to_string())
                                .unwrap_or_default();
                            let req_obj = DgnValue::Map(Arc::new(Mutex::new(HashMap::from([
                                ("method".to_string(), DgnValue::String(method)),
                                ("path".to_string(), DgnValue::String(url)),
                                ("body".to_string(), DgnValue::String(content)),
                                ("body_bytes".to_string(), DgnValue::ByteArray(Arc::new(Mutex::new(body_bytes)))),
                                ("headers".to_string(), DgnValue::Map(headers_map)),
                                ("query".to_string(), DgnValue::Map(query_map)),
                                ("remote_addr".to_string(), DgnValue::String(remote_addr)),
                            ]))));
                            
                            let req_handle = Arc::new(Mutex::new(Some(request)));
                            let res_map = Arc::new(Mutex::new(HashMap::new()));
                            
                            let rh = req_handle.clone();
                            res_map.lock().unwrap().insert("send".into(), native_method(rh, move |rh, args| {
                                let body = val_to_string(args.get(0).unwrap_or(&DgnValue::String("".into())));
                                let status = args.get(1).and_then(|v| v.as_number()).map(|n| n as u16).unwrap_or(200);
                                let headers_arg = args.get(2).cloned();

                                if let Some(req) = rh.lock().unwrap().take() {
                                    let mut response = Response::from_string(body).with_status_code(status);
                                    
                                    if let Some(DgnValue::Map(hm)) = headers_arg {
                                        for (k, v) in hm.lock().unwrap().iter() {
                                            if let Ok(h) = Header::from_bytes(k.as_bytes(), val_to_string(v).as_bytes()) {
                                                response.add_header(h);
                                            }
                                        }
                                    }

                                    let _ = req.respond(response);
                                }
                                Ok(DgnValue::Null)
                            }));

                            let rh2 = req_handle.clone();
                            res_map.lock().unwrap().insert("send_bytes".into(), native_method(rh2, move |rh2, args| {
                                let status = args.get(1).and_then(|v| v.as_number()).map(|n| n as u16).unwrap_or(200);
                                let headers_arg = args.get(2).cloned();
                                let raw_bytes = match args.get(0) {
                                    Some(DgnValue::ByteArray(ba)) => ba.lock().unwrap().clone(),
                                    Some(other) => val_to_string(other).into_bytes(),
                                    None => vec![],
                                };
                                if let Some(req) = rh2.lock().unwrap().take() {
                                    let cursor = std::io::Cursor::new(raw_bytes);
                                    let mut response = Response::new(
                                        tiny_http::StatusCode(status),
                                        vec![],
                                        cursor,
                                        None,
                                        None,
                                    );
                                    if let Some(DgnValue::Map(hm)) = headers_arg {
                                        for (k, v) in hm.lock().unwrap().iter() {
                                            if let Ok(h) = Header::from_bytes(k.as_bytes(), val_to_string(v).as_bytes()) {
                                                response.add_header(h);
                                            }
                                        }
                                    }
                                    let _ = req.respond(response);
                                }
                                Ok(DgnValue::Null)
                            }));

                            let handler_clone = handler.clone();
                            rt.block_on(async move {
                                let _ = call_dgn_func(handler_clone, vec![req_obj, DgnValue::Map(res_map)]).await;
                            });
                        }
                    });
                    Ok(DgnValue::Bool(true))
                }));
             }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },

        "zip" => {
            let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
            {
                map.insert("list".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    
                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut files = Vec::new();
                    for i in 0..archive.len() {
                        if let Ok(z_file) = archive.by_index(i) {
                            files.push(DgnValue::String(z_file.name().to_string()));
                        }
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(files))))
                }));

                // Checks if a specific file exists inside the ZIP/JAR
                // Checks if a specific file exists inside the ZIP/JAR
                map.insert("exists".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    // FIX: Evaluate and drop the temporary borrow immediately
                    let file_exists = archive.by_name(&internal_path).is_ok();
                    
                    Ok(DgnValue::Bool(file_exists))
                }));
                // Returns metadata about a file inside the archive (size, compressed size, type)
                map.insert("info".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    
                    let mut info_map = HashMap::new();
                    info_map.insert("size".to_string(), DgnValue::Number(z_file.size() as f64));
                    info_map.insert("compressed_size".to_string(), DgnValue::Number(z_file.compressed_size() as f64));
                    info_map.insert("is_dir".to_string(), DgnValue::Bool(z_file.is_dir()));
                    info_map.insert("is_file".to_string(), DgnValue::Bool(z_file.is_file()));

                    Ok(DgnValue::Map(Arc::new(Mutex::new(info_map))))
                }));

                // Extracts a single file from the ZIP/JAR to the host disk
                map.insert("extract".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;
                    let dest_path = args.get(2).map(val_to_string).ok_or(DgnError::new("Destination path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut out_file = std::fs::File::create(&dest_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    
                    std::io::copy(&mut z_file, &mut out_file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    Ok(DgnValue::Bool(true))
                }));
                // Reads a file inside the ZIP/JAR as a String (Great for MANIFEST.MF)
                map.insert("read_string".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut buffer = String::new();
                    std::io::Read::read_to_string(&mut z_file, &mut buffer).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    Ok(DgnValue::String(buffer))
                }));

                // Reads a file inside the ZIP/JAR as raw bytes (Great for .class files)
                map.insert("read_bytes".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut buffer = Vec::new();
                    std::io::Read::read_to_end(&mut z_file, &mut buffer).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    Ok(DgnValue::ByteArray(Arc::new(Mutex::new(buffer))))
                }));
            }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },

        "crypto" => {
            let mut map = HashMap::new();

            // Helper to extract bytes from a Dragon String or ByteArray
            fn get_bytes(val: &DgnValue) -> Option<Vec<u8>> {
                match val {
                    DgnValue::String(s) => Some(s.as_bytes().to_vec()),
                    DgnValue::ByteArray(b) => Some(b.lock().unwrap().clone()),
                    _ => None,
                }
            }

            // 1. Generate Random Bytes
            map.insert("random_bytes".to_string(), native_fn(|args| {
                let len = args.get(0).and_then(|v| v.as_int()).unwrap_or(32) as usize;
                let mut bytes = vec![0u8; len];
                OsRng.fill_bytes(&mut bytes);
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(bytes))))
            }));

            // 2. AES-256-GCM Encryption
            map.insert("aes_encrypt".to_string(), native_fn(move |args| {
                let data = get_bytes(args.get(0).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Data required (String or ByteArray)", 0))?;
                let key_bytes = get_bytes(args.get(1).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Key required (String or ByteArray)", 0))?;

                if key_bytes.len() != 32 { return Err(DgnError::new("AES-256 key must be exactly 32 bytes", 0)); }

                let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
                let cipher = Aes256Gcm::new(key);

                let mut nonce_bytes = [0u8; 12];
                OsRng.fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);

                match cipher.encrypt(nonce, data.as_ref()) {
                    Ok(ciphertext) => {
                        let mut out = Vec::with_capacity(12 + ciphertext.len());
                        out.extend_from_slice(&nonce_bytes);
                        out.extend_from_slice(&ciphertext);
                        Ok(DgnValue::ByteArray(Arc::new(Mutex::new(out))))
                    },
                    Err(_) => Err(DgnError::new("AES Encryption failed", 0))
                }
            }));

            // 3. AES-256-GCM Decryption
            map.insert("aes_decrypt".to_string(), native_fn(move |args| {
                let data = get_bytes(args.get(0).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Encrypted data required (ByteArray)", 0))?;
                let key_bytes = get_bytes(args.get(1).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Key required", 0))?;

                if key_bytes.len() != 32 { return Err(DgnError::new("AES-256 key must be exactly 32 bytes", 0)); }
                if data.len() < 12 + 16 { return Err(DgnError::new("Invalid ciphertext length", 0)); }

                let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
                let cipher = Aes256Gcm::new(key);
                let nonce = Nonce::from_slice(&data[..12]);
                let ciphertext = &data[12..];

                match cipher.decrypt(nonce, ciphertext) {
                    Ok(plaintext) => Ok(DgnValue::ByteArray(Arc::new(Mutex::new(plaintext)))),
                    Err(_) => Err(DgnError::new("AES Decryption failed", 0))
                }
            }));

            // 4. SHA-256 Hashing
            map.insert("sha256".to_string(), native_fn(|args| {
                use sha2::{Sha256, Digest};
                let data = get_bytes(args.get(0).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Data required", 0))?;
                let mut hasher = Sha256::new();
                hasher.update(&data);
                let hex_str = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
                Ok(DgnValue::String(hex_str))
            }));

            // 5. SHA-512 Hashing
            map.insert("sha512".to_string(), native_fn(|args| {
                use sha2::{Sha512, Digest};
                let data = get_bytes(args.get(0).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Data required", 0))?;
                let mut hasher = Sha512::new();
                hasher.update(&data);
                let hex_str = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
                Ok(DgnValue::String(hex_str))
            }));

            // 6. MD5 Hashing (Legacy, but often required for older APIs)
            map.insert("md5".to_string(), native_fn(|args| {
                use md5::{Md5, Digest};
                let data = get_bytes(args.get(0).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Data required", 0))?;
                let mut hasher = Md5::new();
                hasher.update(&data);
                let hex_str = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();
                Ok(DgnValue::String(hex_str))
            }));

            // 7. HMAC-SHA256 (Used for API authentication / Webhooks)
            map.insert("hmac_sha256".to_string(), native_fn(|args| {
                use hmac::{Hmac, Mac};
                use sha2::Sha256;
                type HmacSha256 = Hmac<Sha256>;

                let data = get_bytes(args.get(0).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Data required", 0))?;
                let key = get_bytes(args.get(1).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Key required", 0))?;

                let mut mac = <HmacSha256 as Mac>::new_from_slice(&key).map_err(|_| DgnError::new("Invalid HMAC key", 0))?;
                mac.update(&data);
                let hex_str = mac.finalize().into_bytes().iter().map(|b| format!("{:02x}", b)).collect::<String>();
                Ok(DgnValue::String(hex_str))
            }));

            // 8. PBKDF2 Password Key Derivation
            map.insert("pbkdf2".to_string(), native_fn(|args| {
                use pbkdf2::pbkdf2_hmac;
                use sha2::Sha256;

                let password = get_bytes(args.get(0).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Password required", 0))?;
                let salt = get_bytes(args.get(1).unwrap_or(&DgnValue::Null))
                    .ok_or_else(|| DgnError::new("Salt required", 0))?;
                
                // Optional args for iterations and key length
                let iterations = args.get(2).and_then(|v| v.as_int()).unwrap_or(100_000) as u32;
                let key_len = args.get(3).and_then(|v| v.as_int()).unwrap_or(32) as usize;

                let mut derived_key = vec![0u8; key_len];
                pbkdf2_hmac::<Sha256>(&password, &salt, iterations, &mut derived_key);

                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(derived_key))))
            }));

            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        
        "ffi" => {
            let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
    {
            map.insert("loadlib".to_string(), native_fn(|args| {
                let path = if let Some(DgnValue::String(s)) = args.get(0) { s } else { return Err(DgnError::new("Path req", 0)) };
                unsafe {
                    let lib = Library::new(path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    
                    let mut map1 = HashMap::new();
                    map1.insert("path".to_string(), DgnValue::String(path.clone()));
                    map1.insert("ptr".to_string(), DgnValue::NativeLib(Arc::new(Mutex::new(lib))));
                    
                    Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                }
            }));
            
            map.insert("def".to_string(), native_fn(|args| {
                if args.len() < 4 { return Err(DgnError::new("Usage: ffi_def(lib, name, ret_type, [arg_types])", 0)); }
                
                let lib_map = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid Lib Object",0)); };
                let lib_path = val_to_string(lib_map.get("path").unwrap());
                
                let func_name = val_to_string(&args[1]);
                let ret_type = val_to_string(&args[2]);
                
                let arg_types_list = if let DgnValue::List(l) = &args[3] { l.lock().unwrap().clone() } else { vec![] };
                let arg_types: Vec<String> = arg_types_list.iter().map(val_to_string).collect();

                // Store in the global registry
                let key = format!("{}::{}", lib_path, func_name);
                FFI_SIGNATURES.lock().unwrap().insert(key, (ret_type, arg_types));
                
                Ok(DgnValue::Null)
            }));
            
           map.insert("loadlib_call".to_string(), native_fn(|args| {
                if args.len() < 2 { return Err(DgnError::new("loadlib_call(lib, symbol, args_json)", 0)); }
                
                let lib_val = &args[0];
                let symbol_name = val_to_string(&args[1]);
                let input_arg = val_to_string(args.get(2).unwrap_or(&DgnValue::String("".to_string())));

                if let DgnValue::NativeLib(lib) = lib_val {
                    let l = lib.lock().unwrap();
                    unsafe {
                        type DragonFunc = unsafe extern "C" fn(*const c_char) -> *mut c_char;
                        let func: Symbol<DragonFunc> = l.get(symbol_name.as_bytes()).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                        
                        let c_input = CString::new(input_arg).unwrap();
                        let c_result = func(c_input.as_ptr());
                        
                        if c_result.is_null() {
                            return Ok(DgnValue::Null);
                        }
                        
                        let res_str = CStr::from_ptr(c_result).to_string_lossy().into_owned();
                        Ok(DgnValue::String(res_str))
                    }
                } else {
                    Err(DgnError::new("First arg must be NativeLib", 0))
                }
            }));
             
              map.insert("symbol_addr".to_string(), native_fn(|args| {
                // 1. Get Library
                let lib_map = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid Lib",0)); };
                let lib_arc = if let DgnValue::NativeLib(l) = lib_map.get("ptr").unwrap() { l } else { return Err(DgnError::new("Invalid Lib Ptr",0)); };
                
                // 2. Get Symbol Name
                let func_name = val_to_string(&args[1]);
                
                // 3. Return Address
                let l = lib_arc.lock().unwrap();
                unsafe {
                    let symbol: Symbol<*const std::ffi::c_void> = l.get(func_name.as_bytes())
                        .map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    // FIX: Return as true Int!
                    Ok(DgnValue::Int(*symbol as usize as i64))
                }
            }));

            map.insert("call".to_string(), native_fn(|args| {
                    if args.len() < 2 { return Err(DgnError::new("Usage: ffi_call(lib, name, ...args)", 0)); }

                    let lib_map = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid Lib",0)); };
                    let lib_path = val_to_string(lib_map.get("path").unwrap());
                    let lib_arc = if let DgnValue::NativeLib(l) = lib_map.get("ptr").unwrap() { l } else { return Err(DgnError::new("Invalid Lib Ptr",0)); };
                    
                    let func_name = val_to_string(&args[1]);
                    let call_args_values = &args[2..];

                    let key = format!("{}::{}", lib_path, func_name);
                    let sig_guard = FFI_SIGNATURES.lock().unwrap();
                    let (ret_type_str, arg_types_str) = sig_guard.get(&key)
                        .ok_or(DgnError::new(&format!("No signature for '{}'. Call ffi_def() first.", func_name), 0))?;

                    // 1. Prepare Types
                    let mut arg_types: Vec<Type> = Vec::new();
                    for t in arg_types_str {
                        match t.as_str() {
                            "int" | "i32" => arg_types.push(Type::i32()),
                            "long" | "i64" => arg_types.push(Type::i64()),
                            "float" | "f32" => arg_types.push(Type::f32()),
                            "double" | "f64" => arg_types.push(Type::f64()),
                            "pointer" | "void*" => arg_types.push(Type::pointer()),
                            "string" | "char*" => arg_types.push(Type::pointer()),
                            "bool" => arg_types.push(Type::u8()),
                            "void" => {}, 
                            name => {
                                // NEW: Check if it is a registered C-Struct
                                if let Some(def) = FFI_STRUCTS.lock().unwrap().get(name) {
                                    // We clone the pre-calculated LibFFI Type::structure
                                    arg_types.push(def.libffi_type.clone());
                                } else {
                                    return Err(DgnError::new(&format!("Unknown FFI type: {}", name), 0));
                                }
                            }
                        }
                    }

                    let ret_type = match ret_type_str.as_str() {
                        "void" => Type::void(),
                        "int" | "i32" => Type::i32(),
                        "long" | "i64" => Type::i64(),
                        "float" | "f32" => Type::f32(),
                        "double" | "f64" => Type::f64(),
                        "pointer" | "void*" | "string" | "str" => Type::pointer(),
                        "bool" => Type::u8(),
                        _ => return Err(DgnError::new("Unknown return type", 0))
                    };

                    // FIX: Use into_iter() to satisfy Cif::new requirements
                    let cif = Cif::new(arg_types.into_iter(), ret_type.clone());
                    
                    let l = lib_arc.lock().unwrap();
                    let code_ptr: CodePtr = unsafe {
                        let symbol: Symbol<*mut c_void> = l.get(func_name.as_bytes())
                            .map_err(|e| DgnError::new(&e.to_string(), 0))?;
                        CodePtr::from_ptr(*symbol)
                    };

                    // 2. Prepare Values safely
                    // We use an enum to keep the converted data alive during the call
                    enum FfiValue {
                        I32(i32), I64(i64),
                        F32(f32), F64(f64),
                        Ptr(usize), U8(u8),
                        Struct(Vec<u8>),
                    }

                    let mut values = Vec::new();
                    let mut keep_alive_strs: Vec<CString> = Vec::new();

                    for (i, val) in call_args_values.iter().enumerate() {
                        let t_str = arg_types_str.get(i).map(|s| s.as_str()).unwrap_or("int");
                        match t_str {
                            "int" | "i32" => values.push(FfiValue::I32(val.as_int().unwrap_or(0) as i32)),
                            "long" | "i64" => values.push(FfiValue::I64(val.as_int().unwrap_or(0) as i64)),
                            "float" | "f32" => values.push(FfiValue::F32(val.as_number().unwrap_or(0.0) as f32)),
                            "double" | "f64" => values.push(FfiValue::F64(val.as_number().unwrap_or(0.0) as f64)),
                            "pointer" | "void*" => values.push(FfiValue::Ptr(val.as_int().unwrap_or(0) as usize)),
                            "string" | "char*" => {
                                let s = val_to_string(val);
                                let c_str = CString::new(s).unwrap_or_default();
                                let ptr = c_str.as_ptr() as usize;
                                keep_alive_strs.push(c_str); 
                                values.push(FfiValue::Ptr(ptr));
                            },
                            "bool" => values.push(FfiValue::U8(if is_truthy(val) { 1 } else { 0 })),
                        name => {
                                if FFI_STRUCTS.lock().unwrap().contains_key(name) {
                                    if let DgnValue::Map(m) = val {
                                        let m = m.lock().unwrap();
                                        if let Some(DgnValue::ByteArray(b)) = m.get("__bytes__") {
                                            // COPY the bytes into our FfiValue holder
                                            let data = b.lock().unwrap().clone();
                                            values.push(FfiValue::Struct(data));
                                        } else {
                                            // Fallback: empty struct
                                            values.push(FfiValue::Struct(vec![])); 
                                        }
                                    } else {
                                        values.push(FfiValue::I32(0)); // Error fallback
                                    }
                                } else {
                                    values.push(FfiValue::I32(0));
                                }
                            }
                        }
                    }

                    // 3. Create LibFFI Arguments
                    // We keep a separate Vec of struct byte Vecs alive so their
                    // pointers remain valid for the duration of the call.
                    let mut libffi_args = Vec::new();
                    let mut struct_keep_alive: Vec<Vec<u8>> = Vec::new();
                    for v in &values {
                        match v {
                            FfiValue::I32(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::I64(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::F32(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::F64(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::Ptr(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::U8(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::Struct(data) => {
                                if data.is_empty() {
                                    static EMPTY: u8 = 0;
                                    libffi_args.push(Arg::new(&EMPTY));
                                } else {
                                    // Clone into keep_alive so the Vec (and its
                                    // heap buffer) outlives the cif.call() below.
                                    struct_keep_alive.push(data.clone());
                                    let buf = struct_keep_alive.last().unwrap();
                                    // Cast the slice pointer to *const usize so
                                    // Arg::new stores a pointer-to-struct-memory,
                                    // which is what libffi's struct ABI expects.
                                    let ptr: *const usize = buf.as_ptr() as *const usize;
                                    // SAFETY: buf is kept alive in struct_keep_alive
                                    // until after cif.call() completes.
                                    libffi_args.push(unsafe { Arg::new(&*ptr) });
                                }
                            }
                        }
                    }

                    // 4. Execute Call
                    unsafe {
                        match ret_type_str.as_str() {
                            "void" => {
                                cif.call::<()>(code_ptr, &libffi_args);
                                Ok(DgnValue::Null)
                            },
                            "int" | "i32" => {
                                let res = cif.call::<i32>(code_ptr, &libffi_args);
                                Ok(DgnValue::Int(res as i64))
                            },
                            "long" | "i64" => {
                                let res = cif.call::<i64>(code_ptr, &libffi_args);
                                Ok(DgnValue::Int(res))
                            },
                            "float" | "f32" => {
                                let res = cif.call::<f32>(code_ptr, &libffi_args);
                                Ok(DgnValue::Number(res as f64))
                            },
                            "double" | "f64" => {
                                let res = cif.call::<f64>(code_ptr, &libffi_args);
                                Ok(DgnValue::Number(res))
                            },
                            "pointer" | "void*" => {
                                let res = cif.call::<usize>(code_ptr, &libffi_args);
                                Ok(DgnValue::Int(res as i64))
                            },
                            "bool" => {
                                let res = cif.call::<u8>(code_ptr, &libffi_args);
                                Ok(DgnValue::Bool(res != 0))
                            },
                            "string" | "str" => {
                                // Retrieve the result as a raw C string pointer
                                let res_ptr = cif.call::<*mut std::os::raw::c_char>(code_ptr, &libffi_args);
                                
                                if res_ptr.is_null() {
                                    Ok(DgnValue::Null)
                                } else {
                                    // Safely read the C string from memory and convert it to a Rust String
                                    let c_str = unsafe { std::ffi::CStr::from_ptr(res_ptr) };
                                    Ok(DgnValue::String(c_str.to_string_lossy().into_owned()))
                                }
                            },
                            
                            _ => Ok(DgnValue::Null)
                        }
                    }
                }));
            }
             Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "struct" => {
               let mut map = HashMap::new();

           #[cfg(not(target_arch = "wasm32"))]
        {    
                map.insert("def".to_string(), native_fn(|args| {
                    let name = val_to_string(args.get(0).unwrap());
                    let fields_list = if let DgnValue::List(l) = args.get(1).unwrap() { l.lock().unwrap().clone() } else { return Err(DgnError::new("Fields must be a list", 0)); };

                    let mut offset = 0;
                    let mut max_align = 1;
                    let mut struct_fields = Vec::new();
                    let mut field_types = Vec::new();

                    for field in fields_list {
                        if let DgnValue::List(def) = field {
                            let def = def.lock().unwrap();
                            let type_name = val_to_string(&def[0]);
                            let field_name = val_to_string(&def[1]);

                            let (ffi_type, size, align) = get_ffi_type_info(&type_name)
                                .ok_or(DgnError::new(&format!("Unknown type: {}", type_name), 0))?;

                            // 1. Add Padding for Alignment
                            let padding = (align - (offset % align)) % align;
                            offset += padding;

                            // 2. Store Field
                            struct_fields.push((field_name, offset, ffi_type.clone()));
                            field_types.push(ffi_type);

                            // 3. Advance Offset
                            offset += size;
                            if align > max_align { max_align = align; }
                        }
                    }

                    // 4. Final Padding (Structure size must be multiple of largest alignment)
                    let final_padding = (max_align - (offset % max_align)) % max_align;
                    let total_size = offset + final_padding;

                    // 5. Create LibFFI Structure Type
                    let struct_type = Type::structure(field_types);

                    let def = FfiStructDef {
                        size: total_size,
                        fields: struct_fields,
                        libffi_type: struct_type,
                    };

                    FFI_STRUCTS.lock().unwrap().insert(name, def);
                    Ok(DgnValue::Number(total_size as f64))
                }));


                map.insert("create".to_string(), native_fn(|args| {
                    let name = val_to_string(args.get(0).unwrap());
                    let structs = FFI_STRUCTS.lock().unwrap();
                    let def = structs.get(&name).ok_or(DgnError::new("Struct not defined", 0))?;
                    // Allocate zeroed memory
                    let bytes = vec![0u8; def.size];
                    
                    // Return a Map acting as the Struct Object
                    // It contains the Raw Bytes + Metadata
                    let mut map1 = HashMap::new();
                    map1.insert("__struct_id__".to_string(), DgnValue::String(name.clone()));
                    map1.insert("__bytes__".to_string(), DgnValue::ByteArray(Arc::new(Mutex::new(bytes))));
                    
                    Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                }));

               map.insert("set".to_string(), native_fn(|args| {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // 1. SUPPORT BOTH MAP AND INSTANCE
                        let mut obj = match &args[0] {
                            DgnValue::Map(m) => m.lock().unwrap(),
                            DgnValue::Instance(m) => m.lock().unwrap(),
                            _ => return Err(DgnError::new("struct.set: expected struct instance or map", 0))
                        };

                        let struct_id_val = obj.get("__struct_id__").ok_or(DgnError::new("Not a struct (missing __struct_id__)", 0))?;
                        let struct_name = val_to_string(struct_id_val);
                        
                        let buffer_val = obj.get("__bytes__").ok_or(DgnError::new("Not a struct (missing __bytes__)", 0))?;
                        let buffer = if let DgnValue::ByteArray(b) = buffer_val { b.clone() } else { return Err(DgnError::new("Invalid bytes", 0)); };

                        let field_name = if let DgnValue::String(s) = &args[1] { s.clone() } else { val_to_string(&args[1]) };
                        let val = args[2].clone();
                        let structs = FFI_STRUCTS.lock().unwrap();
                        let def = structs.get(&struct_name).ok_or(DgnError::new("Struct not defined", 0))?;
                        
                        let (_, offset, _) = def.fields.iter().find(|(n, _, _)| n == &field_name)
                            .ok_or(DgnError::new("Field not found", 0))?;

                        let mut b = buffer.lock().unwrap();
                        let offset = *offset;
                        
                        // 2. CONVERT INT TO FLOAT FOR C-BUFFER
                        if let Some(n) = val.as_number() {
                            let remaining = b.len().saturating_sub(offset);
                            let bytes = (n as f32).to_ne_bytes();
                            let write_len = std::cmp::min(4, remaining);
                            for i in 0..write_len {
                                b[offset + i] = bytes[i];
                            }
                        }

                        // 3. CRITICAL: SYNC DICTIONARY FOR ffi.call PACKER
                        // We force Number(1.0) instead of Int(1) so the FFI doesn't get confused
                        let sync_val = if let Some(n) = val.as_number() { DgnValue::Number(n) } else { val };
                        obj.insert(field_name, sync_val);

                        Ok(DgnValue::Null)
                    }));
                    
                    match result {
                        Ok(v) => v,
                        Err(e) => Err(DgnError::new("Dragon: Memory access violation in struct.set", 0))
                    }
                }));

            // [INSERT THIS AFTER struct_set block (~Line 1485)]
            map.insert("get".to_string(), native_fn(|args| {
                let obj = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid struct", 0)); };
                let struct_name = val_to_string(obj.get("__struct_id__").unwrap());
                let buffer = if let DgnValue::ByteArray(b) = obj.get("__bytes__").unwrap() { b.clone() } else { return Err(DgnError::new("Invalid bytes", 0)); };
                
                let field_name = val_to_string(&args[1]);

                let structs = FFI_STRUCTS.lock().unwrap();
                let def = structs.get(&struct_name).ok_or(DgnError::new("Struct def not found", 0))?;
                
                // Find field offset and type
                let (_, offset, type_info) = def.fields.iter().find(|(n, _, _)| n == &field_name)
                    .ok_or(DgnError::new("Field not found", 0))?;

                let b = buffer.lock().unwrap();
                let offset = *offset;
                // Read based on type size (Simplified for basics, expand logic as needed)
                // We look at the 'size' of the field type to decide how to read
                // Note: You might want to match on 'type_info' specifically for cleaner code
                if offset + 4 <= b.len() {
                    let bytes: [u8; 4] = b[offset..offset+4].try_into().unwrap();
                    let f_val = f32::from_ne_bytes(bytes); // Assume float for colors
                    let i_val = u32::from_ne_bytes(bytes);
                    
                    // Heuristic: If field name suggests float, return float, else int
                    // In a perfect world, you'd check type_info. 
                    // For Allegro colors (r,g,b,a are floats), we return float.
                    return Ok(DgnValue::Number(f_val as f64));
                }

                Ok(DgnValue::Null)
            }));

        }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
         "mem" => {
               let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
             {
              map.insert("alloc".to_string(), native_fn(|args| {
                    // FIX: Safely accept both Ints and Numbers for allocation size!
                    let size = args.get(0).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    let vec = vec![0u8; size];
                    Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec))))
                }));

                // Add this inside the "mem" map insertions
                map.insert("read_ptr".to_string(), native_fn(|args| {
                    let ptr = args.get(0).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    let offset = args.get(1).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    
                    if ptr == 0 { return Ok(DgnValue::Null); }
                    unsafe {
                        let target_addr = (ptr + offset) as *const usize;
                        let val = *target_addr;
                        // Return pointers as true Integers to prevent 53-bit float corruption!
                        Ok(DgnValue::Int(val as i64)) 
                    }
                }));

map.insert("read_float".to_string(), native_fn(|args| {
                    let ptr = args.get(0).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    let offset = args.get(1).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    
                    if ptr == 0 { return Ok(DgnValue::Null); }
                    unsafe {
                        let target_addr = (ptr + offset) as *const f32;
                        let val = *target_addr;
                        Ok(DgnValue::Number(val as f64))
                    }
                }));

                map.insert("read_mem_int".to_string(), native_fn(|args| {
                    let ptr = args.get(0).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    let offset = args.get(1).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    
                    if ptr == 0 { return Ok(DgnValue::Null); }
                    unsafe {
                        let target_addr = (ptr + offset) as *const u32;
                        let val = *target_addr;
                        Ok(DgnValue::Int(val as i64))
                    }
                }));
                
                map.insert("read_string".to_string(), native_fn(|args| {
                    let ptr = args.get(0).and_then(|v| v.as_int()).unwrap_or(0) as usize;
                    if ptr == 0 { return Ok(DgnValue::Null); }
                    unsafe {
                        let c_str = std::ffi::CStr::from_ptr(ptr as *const std::os::raw::c_char);
                        Ok(DgnValue::String(c_str.to_string_lossy().into_owned()))
                    }
                }));

                map.insert("addr".to_string(), native_fn(|args| {
                    match args.get(0) {
                        Some(DgnValue::ByteArray(b)) => {
                            let ptr = b.lock().unwrap().as_ptr() as usize;
                            // FIX: Return as true Int to prevent float precision loss!
                            Ok(DgnValue::Int(ptr as i64)) 
                        },
                        Some(DgnValue::String(s)) => {
                            let ptr = s.as_ptr() as usize;
                            Ok(DgnValue::Int(ptr as i64))
                        }
                        _ => Ok(DgnValue::Null)
                    }
                }));

                map.insert("read_int".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                    
                    let b = buffer.lock().unwrap();
                    if offset + 4 <= b.len() {
                        let bytes: [u8; 4] = b[offset..offset+4].try_into().unwrap();
                        let val = u32::from_ne_bytes(bytes); 
                        Ok(DgnValue::Number(val as f64))
                    } else { Ok(DgnValue::Null) }
                }));

                map.insert("write_int".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                    let val = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0) as u32;

                    let mut b = buffer.lock().unwrap();
                    if offset + 4 <= b.len() {
                        let bytes = val.to_ne_bytes();
                        for i in 0..4 { b[offset + i] = bytes[i]; }
                    }
                    Ok(DgnValue::Null)
                }));

                map.insert("read_byte".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                    
                    let b = buffer.lock().unwrap();
                    if offset < b.len() { Ok(DgnValue::Number(b[offset] as f64)) } else { Ok(DgnValue::Null) }
                }));

                map.insert("write_byte".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                    let val = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0) as u8;

                    let mut b = buffer.lock().unwrap();
                    if offset < b.len() { b[offset] = val; }
                    Ok(DgnValue::Null)
                }));


              
             }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
         },
         
         "thread" => {
             let mut map = HashMap::new();
             #[cfg(not(target_arch = "wasm32"))]
             {
                    map.insert("spawn".to_string(), native_fn(|args| {
                        let func = args.get(0).cloned().ok_or(DgnError::new("Func required",0))?;
                        let t_args = if let Some(DgnValue::List(l)) = args.get(1) { l.lock().unwrap().clone() } else { vec![] };
                        let handle = thread::spawn(move || { 
                            // Create a runtime for the new thread
                            let rt = tokio::runtime::Runtime::new().unwrap(); 
                            let _ = rt.block_on(call_dgn_func(func, t_args)); 
                        });
                        Ok(DgnValue::Thread(Arc::new(handle)))
                    }));
            }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
         }
        // To keep things truly modular, you can move memory, ffi, and struct globals here as well!
        // You just transfer the logic from `populate_builtins` verbatim under an "ffi" and "mem" match block.
        _ => None,
    }
}

pub fn populate_builtins(env: Arc<Mutex<Env>>) {
    let mut e = env.lock().unwrap();
    let gc_env = env.clone();

    e.set("show", native_fn(|args| { 
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&s.into()); 
        #[cfg(not(target_arch = "wasm32"))]
        print!("{}", s); 
        Ok(DgnValue::Null) 
    }));

    e.set("showln", native_fn(|args| { 
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&s.into());
        #[cfg(not(target_arch = "wasm32"))]
        println!("{}", s); 
        Ok(DgnValue::Null) 
    }));

  e.set("readln", native_fn(|args| {
    use std::io::{self, Write};

    if let Some(prompt) = args.get(0) {
        print!("{}", val_to_string(prompt));
        let _ = io::stdout().flush(); 
    }

    let mut b = String::new();
    match io::stdin().read_line(&mut b) {
        Ok(_) => Ok(DgnValue::String(b.trim_end_matches(|c| c == '\r' || c == '\n').to_string())),
        Err(e) => Err(DgnError::new(&e.to_string(), 0)),
    }
}));
    e.set("exit", native_fn(|args| { let c = args.get(0).and_then(|v| v.as_number()).map(|n| n as i32).unwrap_or(0); std::process::exit(c); }));
    e.set("gc", native_fn(move |_| {
        let count = gc_collect(gc_env.clone());
        Ok(DgnValue::Number(count as f64))
    }));


    e.set("sleep", native_fn(|args| { thread::sleep(Duration::from_secs_f64(args.get(0).and_then(|a| a.as_number()).unwrap_or(0.0))); Ok(DgnValue::Null) }));
    e.set("hashmap", native_fn(|_| {
        let val = DgnValue::Map(Arc::new(Mutex::new(HashMap::new())));
        gc_register(&val);
        Ok(val)
    }));
    e.set("list", native_fn(|args| {
        let val = if let Some(DgnValue::List(l)) = args.get(0) { 
            DgnValue::List(l.clone()) 
        } else { 
            DgnValue::List(Arc::new(Mutex::new(Vec::new()))) 
        };
        gc_register(&val);
        Ok(val)
    }));
     e.set("regex", native_fn(|args| { let p = if let Some(DgnValue::String(s)) = args.get(0) { s } else { "" }; match Regex::new(p) { Ok(r) => Ok(DgnValue::Regex(Arc::new(r))), Err(e) => Err(DgnError::new(&e.to_string(), 0)) } }));
    

            e.set("bytes", native_fn(|args| {
            if let Some(DgnValue::List(l)) = args.get(0) {
                let list = l.lock().unwrap();
                let mut vec = Vec::with_capacity(list.len());
                for val in list.iter() {
                    // Support both int and number inside the list
                    if let Some(n) = val.as_int() {
                        vec.push(n as u8);
                    } else {
                        vec.push(0);
                    }
                }
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec))))
            } else if let Some(DgnValue::String(s)) = args.get(0) {
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(s.as_bytes().to_vec()))))
            } else if let Some(n) = args.get(0).and_then(|v| v.as_int()) { // FIX: Use as_int here
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec![0; n as usize]))))
            } else {
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(Vec::new()))))
            }
        }));


        e.set("format", native_fn(|args| {
        if args.is_empty() { return Ok(DgnValue::String(String::new())); }
        let fmt_str = val_to_string(&args[0]);
        let mut out = String::new();
        let mut arg_idx = 1;
        let mut chars = fmt_str.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '{' {
                // Handle escaped {{
                if let Some(&'{') = chars.peek() {
                    chars.next(); out.push('{'); continue;
                }
                
                let mut spec = String::new();
                let mut closed = false;
                for sc in chars.by_ref() {
                    if sc == '}' { closed = true; break; }
                    spec.push(sc);
                }
                
                if !closed { return Err(DgnError::new("Unclosed '{' in format string", 0)); }
                
                // Get the argument, default to null if missing
                let arg = args.get(arg_idx).unwrap_or(&DgnValue::Null);
                arg_idx += 1;
                
                if spec.is_empty() {
                    out.push_str(&val_to_string(arg));
                } else if spec.starts_with(':') {
                    let rule = &spec[1..]; 
                    
                    if rule.ends_with('f') && rule.starts_with('.') { 
                        // Float Precision: {:.2f}
                        if let Ok(prec) = rule[1..rule.len()-1].parse::<usize>() {
                            if let Some(n) = arg.as_number() {
                                out.push_str(&format!("{:.*}", prec, n));
                            } else { out.push_str(&val_to_string(arg)); }
                        } else { out.push_str(&val_to_string(arg)); }
                    } else if rule.starts_with('>') { 
                        // Right Align: {:>10}
                        if let Ok(width) = rule[1..].parse::<usize>() {
                            let s = val_to_string(arg);
                            if s.len() < width { out.push_str(&" ".repeat(width - s.len())); }
                            out.push_str(&s);
                        }
                    } else if rule.starts_with('<') { 
                        // Left Align: {:<10}
                        if let Ok(width) = rule[1..].parse::<usize>() {
                            let s = val_to_string(arg);
                            out.push_str(&s);
                            if s.len() < width { out.push_str(&" ".repeat(width - s.len())); }
                        }
                    } else if rule.starts_with('0') { 
                        // Zero Padding: {:05}
                        if let Ok(width) = rule.parse::<usize>() {
                            let s = val_to_string(arg);
                            if s.len() < width { out.push_str(&"0".repeat(width - s.len())); }
                            out.push_str(&s);
                        }
                    } else { 
                        // Unknown format, fallback to standard string
                        out.push_str(&val_to_string(arg)); 
                    }
                } else { out.push_str(&val_to_string(arg)); }
                
            } else if c == '}' {
                // Handle escaped }}
                if let Some(&'}') = chars.peek() {
                    chars.next(); out.push('}');
                } else { return Err(DgnError::new("Unmatched '}' in format string", 0)); }
            } else { 
                out.push(c); 
            }
        }
        Ok(DgnValue::String(out))
    }));

     e.set("typeof", native_fn(|args| {
        let val = args.get(0).unwrap_or(&DgnValue::Null);
        let type_name = match val {
            DgnValue::Null => "null",
            DgnValue::Bool(_) => "bool",
            DgnValue::Number(_) => "number",
            DgnValue::Int(_) => "int",
            DgnValue::String(_) => "string",
            DgnValue::List(_) => "list",
            DgnValue::Map(_) => "map",
            DgnValue::ByteArray(_) => "bytes",
            DgnValue::Func(_) => "function",
            DgnValue::Class(_) => "class", 
            DgnValue::Instance(_) => "instance",
            #[cfg(not(any(target_arch = "wasm32")))]
            DgnValue::NativeLib(_) => "nativelib",
            #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
            DgnValue::Socket(_) => "socket",
            DgnValue::Regex(_) => "regex",
            DgnValue::Thread(_) => "thread",
            DgnValue::Promise(_) => "promise",
            DgnValue::NativeMethod(_) => "function",
        };
        Ok(DgnValue::String(type_name.to_string()))
    }));


     e.set("int", native_fn(|args| {
        if let Some(n) = args.get(0).and_then(|a| a.as_number()) {
            return Ok(DgnValue::Int(n.trunc() as i64)); // Now explicitly returns Int!
        }
        if let Some(DgnValue::String(s)) = args.get(0) {
            if let Ok(n) = s.parse::<f64>() { return Ok(DgnValue::Int(n.trunc() as i64)); }
        }
        if let Some(DgnValue::Bool(b)) = args.get(0) {
            return Ok(DgnValue::Int(if *b { 1 } else { 0 }));
        }
        Ok(DgnValue::Null)
    }));

        e.set("float", native_fn(|args| {
            if let Some(val) = args.get(0) {
                if let Some(n) = val.as_number() {
                    return Ok(DgnValue::Number(n));
                }
                if let DgnValue::String(s) = val {
                    return Ok(DgnValue::Number(s.parse().unwrap_or(0.0)));
                }
            }
            Ok(DgnValue::Null)
        }));

    e.set("chr", native_fn(|args| {
        if let Some(n) = args.get(0).and_then(|a| a.as_number()) {
            if let Some(c) = std::char::from_u32(n as u32) {
                return Ok(DgnValue::String(c.to_string()));
            }
        }
        Ok(DgnValue::Null)
    }));

     e.set("str", native_fn(|args| {
        let v = args.get(0).unwrap_or(&DgnValue::Null);
        Ok(DgnValue::String(val_to_string(v)))
    }));

     e.set("len", native_fn(|args| {
        match args.get(0) {
            Some(DgnValue::String(s)) => Ok(DgnValue::Number(s.len() as f64)),
            Some(DgnValue::List(l)) => Ok(DgnValue::Number(l.lock().unwrap().len() as f64)),
            Some(DgnValue::Map(m)) => Ok(DgnValue::Number(m.lock().unwrap().len() as f64)),
            Some(DgnValue::ByteArray(b)) => Ok(DgnValue::Number(b.lock().unwrap().len() as f64)),
            _ => Ok(DgnValue::Number(0.0))
        }
    }));



    e.set("ord", native_fn(|args| {
        if let Some(DgnValue::String(s)) = args.get(0) {
            if let Some(c) = s.chars().next() {
                return Ok(DgnValue::Number(c as u32 as f64));
            }
        }
        Ok(DgnValue::Null)
    }));
    

    e.set("parse_int", native_fn(|args| {
        let s = args.get(0).map(val_to_string).unwrap_or_default();
        match s.parse::<i64>() {
            Ok(n) => Ok(DgnValue::Number(n as f64)),
            Err(_) => Ok(DgnValue::Null)
        }
    }));

    e.set("parse_float", native_fn(|args| {
        let s = args.get(0).map(val_to_string).unwrap_or_default();
        match s.parse::<f64>() {
            Ok(n) => Ok(DgnValue::Number(n)),
            Err(_) => Ok(DgnValue::Null)
        }
    }));


#[cfg(target_arch = "wasm32")]
    {
        e.set("alert", native_fn(|args| {
            let msg = args.get(0).map(val_to_string).unwrap_or_default();
            web_sys::window().unwrap().alert_with_message(&msg).unwrap();
            Ok(DgnValue::Null)
        }));
        
    }

    e.define_const("PI".to_string(), DgnValue::Number(std::f64::consts::PI)).ok();
    e.define_const("E".to_string(), DgnValue::Number(std::f64::consts::E)).ok();

}


