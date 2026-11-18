use super::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) struct FlattenCtx {
    pub packages: std::collections::BTreeMap<String, crate::packages::Package>,
    loaded: std::collections::HashMap<String, Loaded>,
    builtin_seen: HashSet<String>,
    counter: usize,
    loading: Vec<String>,
}

#[derive(Clone)]
struct Loaded {
    prefix: Option<String>,
    exports: HashSet<String>,
}

impl FlattenCtx {
    pub fn new(app_root: &Path) -> FlattenCtx {
        FlattenCtx {
            packages: crate::packages::package_roots(app_root),
            loaded: Default::default(),
            builtin_seen: Default::default(),
            counter: 0,
            loading: Vec::new(),
        }
    }
}

fn module_candidates(base_dir: &Path, module: &str) -> Vec<PathBuf> {
    let stem = Path::new(module).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    vec![
        base_dir.join(format!("{}.dgn", module)),
        base_dir.join(module).join(format!("{}.dgn", stem)),
        base_dir.join(module).join("index.dgn"),
        base_dir.join("dgn_modules").join(format!("{}.dgn", module)),
    ]
}

pub(crate) fn default_bind(module: &str) -> String {
    let m = module.strip_prefix("package:").unwrap_or(module);
    Path::new(m).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| m.to_string())
}

pub(crate) fn flatten_app(
    ast: &Stmt,
    base_dir: &Path,
    ctx: &mut FlattenCtx,
    label: &str,
    prefix: Option<String>,
    framework: bool,
) -> Result<Vec<Stmt>, String> {
    let stmts: Vec<Stmt> = match &ast.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s.clone(),
        _ => vec![ast.clone()],
    };
    ctx.counter += 1;
    let my_id = ctx.counter;
    let mut out: Vec<(bool, Stmt)> = Vec::new();
    let mut aliases: HashMap<String, super::modpass::AliasBinding> = HashMap::new();
    let mut star_names: HashMap<String, String> = HashMap::new();
    for s in &stmts {
        let (module, alias) = match &s.node {
            StmtNode::Import(m, a) => (m.clone(), a.clone()),
            _ => {
                out.push((true, s.clone()));
                continue;
            }
        };
        if is_builtin_module(&module) {
            let key = format!("{}|{}", module, alias.clone().unwrap_or_default());
            if ctx.builtin_seen.insert(key) {
                out.push((false, s.clone()));
            }
            continue;
        }
        let star = alias.as_deref() == Some("*");
        let fw_src = crate::ui_host::FRAMEWORK_SOURCES.iter().find(|(n, _)| *n == module).map(|(_, src)| *src);
        if fw_src.is_some() && !star {
            return Err(format!("{}: framework module \"{}\" must be imported with `as *`", label, module));
        }
        let (key, path): (String, Option<PathBuf>) = if fw_src.is_some() {
            (format!("framework:{}", module), None)
        } else if module.starts_with("package:") {
            let p = crate::packages::resolve_package_import(&module, &ctx.packages).map_err(|e| format!("{}: {}", label, e))?;
            (p.canonicalize().unwrap_or_else(|_| p.clone()).to_string_lossy().to_string(), Some(p))
        } else {
            let cands = module_candidates(base_dir, &module);
            let p = cands.iter().find(|p| p.is_file()).cloned().ok_or_else(|| {
                format!("{}: can't find module '{}' (looked for {})", label, module,
                    cands.iter().map(|c| c.display().to_string()).collect::<Vec<_>>().join(", "))
            })?;
            (p.canonicalize().unwrap_or_else(|_| p.clone()).to_string_lossy().to_string(), Some(p))
        };
        if ctx.loading.contains(&key) {
            return Err(format!("{}: cyclic import of '{}'", label, module));
        }
        let loaded: Loaded = if let Some(l) = ctx.loaded.get(&key).cloned() {
            if fw_src.is_none() {
                if star && l.prefix.is_some() {
                    return Err(format!("{}: module '{}' is imported with `as *` here but with a name elsewhere; use one style for a module in native builds", label, module));
                }
                if !star && l.prefix.is_none() {
                    return Err(format!("{}: module '{}' is imported with a name here but with `as *` elsewhere; use one style for a module in native builds", label, module));
                }
            }
            l
        } else {
            let is_fw = fw_src.is_some();
            let (mod_ast, mod_dir, mod_label): (Stmt, PathBuf, String) = if let Some(src) = fw_src {
                let tokens = lex(src).map_err(|e| format!("framework module {}: {}", module, e.with_source(src).format_pretty()))?;
                let a = Parser::new(tokens).parse().map_err(|e| format!("framework module {}: {}", module, e))?;
                (a, base_dir.to_path_buf(), format!("framework module {}", module))
            } else {
                let path = path.clone().unwrap();
                let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
                let tokens = lex(&src).map_err(|e| format!("{}: {}", path.display(), e.with_source(&src).format_pretty()))?;
                let a = Parser::new(tokens).parse().map_err(|e| format!("{}: {}", path.display(), e))?;
                (a, path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| base_dir.to_path_buf()), path.display().to_string())
            };
            let child_prefix = if is_fw || star { None } else { Some(format!("m{}__", ctx.counter + 1)) };
            ctx.loading.push(key.clone());
            let r = flatten_module(&mod_ast, &mod_dir, ctx, &mod_label, child_prefix.clone(), is_fw);
            ctx.loading.pop();
            let (inner, exports) = r?;
            let l = Loaded { prefix: child_prefix, exports };
            ctx.loaded.insert(key.clone(), l.clone());
            for st in inner { out.push((false, st)); }
            l
        };
        if star && fw_src.is_none() {
            for n in &loaded.exports {
                if let Some(prev) = star_names.insert(n.clone(), module.clone()) {
                    return Err(format!("{}: '{}' is exported by both \"{}\" and \"{}\"; import one of them with a name (import \"...\" as m) or rename it", label, n, prev, module));
                }
            }
        }
        if !star && fw_src.is_none() {
            let name = alias.clone().unwrap_or_else(|| default_bind(&module));
            aliases.insert(name, super::modpass::AliasBinding { prefix: loaded.prefix.clone().unwrap_or_default(), exports: loaded.exports.clone() });
        }
    }
    if !framework && !star_names.is_empty() {
        let own = out.iter().filter(|(o, _)| *o).map(|(_, s)| s.clone()).collect::<Vec<_>>();
        for n in super::modpass::top_level_names(&own) {
            if let Some(m) = star_names.get(&n) {
                return Err(format!("{}: '{}' is defined here and also exported by \"{}\" (imported with `as *`); rename one or import the module with a name", label, n, m));
            }
        }
    }
    finish_module(out, aliases, my_id, prefix, framework, label).map(|(v, _)| v)
}

fn flatten_module(
    ast: &Stmt,
    base_dir: &Path,
    ctx: &mut FlattenCtx,
    label: &str,
    prefix: Option<String>,
    framework: bool,
) -> Result<(Vec<Stmt>, HashSet<String>), String> {
    let stmts_clone = ast.clone();
    let own_names: Vec<String> = match &stmts_clone.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => super::modpass::top_level_names(s),
        _ => super::modpass::top_level_names(&[stmts_clone.clone()]),
    };
    let exports: HashSet<String> = own_names.iter().filter(|n| !n.starts_with('_')).cloned().collect();
    let stmts = flatten_app(ast, base_dir, ctx, label, prefix, framework)?;
    Ok((stmts, exports))
}

fn finish_module(
    out: Vec<(bool, Stmt)>,
    aliases: HashMap<String, super::modpass::AliasBinding>,
    my_id: usize,
    prefix: Option<String>,
    framework: bool,
    label: &str,
) -> Result<(Vec<Stmt>, ()), String> {
    let mut own: Vec<Stmt> = out.iter().filter(|(o, _)| *o).map(|(_, s)| s.clone()).collect();
    if !framework {
        let names = super::modpass::top_level_names(&own);
        let mut vars: HashMap<String, String> = HashMap::new();
        for n in names {
            if let Some(p) = &prefix {
                vars.insert(n.clone(), format!("{}{}", p, n));
            } else if n.starts_with('_') {
                vars.insert(n.clone(), format!("__p{}_{}", my_id, n));
            }
        }
        super::modpass::rewrite_file(&mut own, &vars, &aliases).map_err(|e| format!("{}: {}", label, e))?;
    }
    let mut it = own.into_iter();
    let merged: Vec<Stmt> = out.into_iter().map(|(o, s)| if o { it.next().unwrap() } else { s }).collect();
    Ok((merged, ()))
}

pub(crate) const IOS_HEADER: &str = r#"#ifndef DRAGON_APP_H
#define DRAGON_APP_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
const char* dgn_app_init(const char* docs_dir, const char* media_json);
const char* dgn_app_render(void);
int         dgn_app_event(const char* event);
int         dgn_app_is_dirty(void);
int64_t     dgn_app_tick(int64_t now_ms);
const char* dgn_app_platform_calls(void);
int         dgn_app_platform_result(const char* id, const char* json);
int         dgn_app_back(void);
int         dgn_app_set_media(const char* media_json);
int         dgn_app_lifecycle(const char* state);
const char* dgn_app_last_error(void);
#ifdef __cplusplus
}
#endif
#endif
"#;

const GLUE: &str = r#"
#ifdef DGN_APP_WINDOWS
#  define DGN_APP_API __declspec(dllexport)
#else
#  define DGN_APP_API
#endif
#ifdef DGN_APP_JNI
#  include <jni.h>
#endif

static char* _dgn_app_ret = NULL;
static char* _dgn_app_err = NULL;
static int   _dgn_app_booted = 0;

static const char* dgn_app_keep(DgnVal v) {
    const char* s = (v.tag == DGN_STR && v.s) ? v.s : "";
    free(_dgn_app_ret);
    _dgn_app_ret = strdup(s);
    return _dgn_app_ret;
}
static void dgn_app_note_error(DgnVal thrown) {
    char* s = dgn_val_to_cstr(thrown);
    free(_dgn_app_err);
    _dgn_app_err = strdup(s);
    fprintf(stderr, "Dragon error: %s\n", s);
    free(s);
}

typedef struct { int op; const char* s1; const char* s2; int64_t n; } DgnAppCall;

static __attribute__((noinline)) DgnVal dgn_app_dispatch(DgnAppCall* c) {
    DgnVal a[3];
    switch (c->op) {
        case 1:
            a[0] = dgn_str(c->s1); a[1] = dgn_str("android"); a[2] = dgn_str(c->s2);
            return dgn_call_val(_v_dragon_host_init, a, 3);
        case 2: return dgn_call_val(_v_dragon_host_render, a, 0);
        case 3: a[0] = dgn_str(c->s1); return dgn_call_val(_v_dragon_host_event, a, 1);
        case 4: return dgn_call_val(_v_dragon_host_is_dirty, a, 0);
        case 5: a[0] = dgn_int(c->n); return dgn_call_val(_v_dragon_host_tick, a, 1);
        case 6: return dgn_call_val(_v_dragon_host_platform_calls, a, 0);
        case 7: a[0] = dgn_str(c->s1); a[1] = dgn_str(c->s2); return dgn_call_val(_v_dragon_host_platform_result, a, 2);
        case 8: return dgn_call_val(_v_dragon_host_back, a, 0);
        case 9: a[0] = dgn_str(c->s1); return dgn_call_val(_v_dragon_host_set_media, a, 1);
        case 10: a[0] = dgn_str(c->s1); return dgn_call_val(_v_dragon_host_lifecycle, a, 1);
        case 11: return dgn_call_val(_v_dragon_host_last_error, a, 0);
        case 12:
            if (_v_main.tag == DGN_FUNC) return dgn_call_val(_v_main, a, 0);
            return dgn_null();
    }
    return dgn_null();
}

static __attribute__((noinline)) DgnVal dgn_app_run(DgnAppCall* c) {
    int stack_marker = 0;
    DgnVal result = dgn_null();
    _dgn_stack_bottom = (void*)&stack_marker;
    DgnTryFrame tf;
    dgn_try_push(&tf);
    if (setjmp(tf.buf) == 0) {
        result = dgn_app_dispatch(c);
        dgn_try_pop(&tf);
    } else {
        dgn_app_note_error(tf.thrown);
        result = dgn_null();
    }
    return result;
}

DGN_APP_API const char* dgn_app_init(const char* docs_dir, const char* media_json) {
    char marker;
    if (!_dgn_app_booted) {
        _dgn_app_booted = 1;
        dgn_app_main(0, NULL);
        _dgn_stack_bottom = (void*)&marker;
    }
    free(_dgn_app_err); _dgn_app_err = NULL;
    DgnAppCall m = {12, NULL, NULL, 0};
    dgn_app_run(&m);
    DgnAppCall c = {1, docs_dir ? docs_dir : "", media_json ? media_json : "", 0};
    dgn_app_run(&c);
    return _dgn_app_err ? _dgn_app_err : "";
}
DGN_APP_API const char* dgn_app_render(void)  { DgnAppCall c = {2, NULL, NULL, 0}; return dgn_app_keep(dgn_app_run(&c)); }
DGN_APP_API int dgn_app_event(const char* e)  { DgnAppCall c = {3, e, NULL, 0}; DgnVal r = dgn_app_run(&c); return r.tag == DGN_BOOL && r.i; }
DGN_APP_API int dgn_app_is_dirty(void)        { DgnAppCall c = {4, NULL, NULL, 0}; DgnVal r = dgn_app_run(&c); return r.tag == DGN_BOOL && r.i; }
DGN_APP_API int64_t dgn_app_tick(int64_t now) { DgnAppCall c = {5, NULL, NULL, now}; DgnVal r = dgn_app_run(&c); return r.tag == DGN_INT ? r.i : -1; }
DGN_APP_API const char* dgn_app_platform_calls(void) { DgnAppCall c = {6, NULL, NULL, 0}; return dgn_app_keep(dgn_app_run(&c)); }
DGN_APP_API int dgn_app_platform_result(const char* id, const char* json) { DgnAppCall c = {7, id, json, 0}; DgnVal r = dgn_app_run(&c); return r.tag == DGN_BOOL && r.i; }
DGN_APP_API int dgn_app_back(void)            { DgnAppCall c = {8, NULL, NULL, 0}; DgnVal r = dgn_app_run(&c); return r.tag == DGN_BOOL && r.i; }
DGN_APP_API int dgn_app_set_media(const char* m) { DgnAppCall c = {9, m, NULL, 0}; DgnVal r = dgn_app_run(&c); return r.tag == DGN_BOOL && r.i; }
DGN_APP_API int dgn_app_lifecycle(const char* s) { DgnAppCall c = {10, s, NULL, 0}; DgnVal r = dgn_app_run(&c); return r.tag == DGN_BOOL && r.i; }
DGN_APP_API const char* dgn_app_last_error(void) { DgnAppCall c = {11, NULL, NULL, 0}; return dgn_app_keep(dgn_app_run(&c)); }

#ifdef DGN_APP_JNI
static jstring dgn_jstr(JNIEnv* env, const char* s) { return (*env)->NewStringUTF(env, s ? s : ""); }
JNIEXPORT jstring JNICALL Java_org_dragonlang_runtime_DragonNative_init(JNIEnv* env, jobject self, jstring appDir, jstring docsDir, jstring media) {
    (void)self; (void)appDir;
    const char* d = (*env)->GetStringUTFChars(env, docsDir, 0);
    const char* m = (*env)->GetStringUTFChars(env, media, 0);
    const char* r = dgn_app_init(d, m);
    jstring out = dgn_jstr(env, r);
    (*env)->ReleaseStringUTFChars(env, docsDir, d);
    (*env)->ReleaseStringUTFChars(env, media, m);
    return out;
}
JNIEXPORT jstring JNICALL Java_org_dragonlang_runtime_DragonNative_render(JNIEnv* env, jobject self) { (void)self; return dgn_jstr(env, dgn_app_render()); }
JNIEXPORT jboolean JNICALL Java_org_dragonlang_runtime_DragonNative_event(JNIEnv* env, jobject self, jstring e) {
    (void)self; const char* s = (*env)->GetStringUTFChars(env, e, 0); int r = dgn_app_event(s); (*env)->ReleaseStringUTFChars(env, e, s); return r ? JNI_TRUE : JNI_FALSE;
}
JNIEXPORT jboolean JNICALL Java_org_dragonlang_runtime_DragonNative_isDirty(JNIEnv* env, jobject self) { (void)env; (void)self; return dgn_app_is_dirty() ? JNI_TRUE : JNI_FALSE; }
JNIEXPORT jlong JNICALL Java_org_dragonlang_runtime_DragonNative_tick(JNIEnv* env, jobject self, jlong now) { (void)env; (void)self; return (jlong)dgn_app_tick((int64_t)now); }
JNIEXPORT jstring JNICALL Java_org_dragonlang_runtime_DragonNative_platformCalls(JNIEnv* env, jobject self) { (void)self; return dgn_jstr(env, dgn_app_platform_calls()); }
JNIEXPORT jboolean JNICALL Java_org_dragonlang_runtime_DragonNative_platformResult(JNIEnv* env, jobject self, jstring id, jstring json) {
    (void)self; const char* a = (*env)->GetStringUTFChars(env, id, 0); const char* b = (*env)->GetStringUTFChars(env, json, 0);
    int r = dgn_app_platform_result(a, b); (*env)->ReleaseStringUTFChars(env, id, a); (*env)->ReleaseStringUTFChars(env, json, b); return r ? JNI_TRUE : JNI_FALSE;
}
JNIEXPORT jboolean JNICALL Java_org_dragonlang_runtime_DragonNative_back(JNIEnv* env, jobject self) { (void)env; (void)self; return dgn_app_back() ? JNI_TRUE : JNI_FALSE; }
JNIEXPORT jboolean JNICALL Java_org_dragonlang_runtime_DragonNative_setMedia(JNIEnv* env, jobject self, jstring m) {
    (void)self; const char* s = (*env)->GetStringUTFChars(env, m, 0); int r = dgn_app_set_media(s); (*env)->ReleaseStringUTFChars(env, m, s); return r ? JNI_TRUE : JNI_FALSE;
}
JNIEXPORT jboolean JNICALL Java_org_dragonlang_runtime_DragonNative_lifecycle(JNIEnv* env, jobject self, jstring m) {
    (void)self; const char* s = (*env)->GetStringUTFChars(env, m, 0); int r = dgn_app_lifecycle(s); (*env)->ReleaseStringUTFChars(env, m, s); return r ? JNI_TRUE : JNI_FALSE;
}
JNIEXPORT jstring JNICALL Java_org_dragonlang_runtime_DragonNative_lastError(JNIEnv* env, jobject self) { (void)self; return dgn_jstr(env, dgn_app_last_error()); }
#endif

#ifdef DGN_APP_TEST_MAIN
int main(int argc, char** argv) {
    (void)argc; (void)argv;
    const char* media = "{\"width\":360,\"height\":720,\"pixelRatio\":2,\"top\":24,\"bottom\":0,\"left\":0,\"right\":0,\"keyboard\":0,\"brightness\":\"light\",\"textScale\":1,\"locale\":\"en_US\"}";
    const char* err = dgn_app_init("/tmp/dragon_docs", media);
    if (err && *err) printf("INIT ERROR: %s\n", err);
    static char line[1 << 24];
    while (fgets(line, sizeof line, stdin)) {
        size_t n = strlen(line); while (n && (line[n-1] == '\n' || line[n-1] == '\r')) line[--n] = 0;
        if (!strcmp(line, "render")) printf("%s\n", dgn_app_render());
        else if (!strncmp(line, "event ", 6)) printf("event -> %d\n", dgn_app_event(line + 6));
        else if (!strncmp(line, "tick ", 5)) printf("tick -> %lld\n", (long long)dgn_app_tick(atoll(line + 5)));
        else if (!strcmp(line, "calls")) printf("%s\n", dgn_app_platform_calls());
        else if (!strncmp(line, "result ", 7)) { char* sp = strchr(line + 7, ' '); if (sp) { *sp = 0; printf("result -> %d\n", dgn_app_platform_result(line + 7, sp + 1)); } }
        else if (!strcmp(line, "back")) printf("back -> %d\n", dgn_app_back());
        else if (!strcmp(line, "dirty")) printf("dirty -> %d\n", dgn_app_is_dirty());
        fflush(stdout);
    }
    return 0;
}
#endif
"#;

pub(crate) fn flatten_for_executable(ast: &Stmt, entry: &Path) -> Result<Option<Stmt>, String> {
    let stmts: Vec<Stmt> = match &ast.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s.clone(),
        _ => vec![ast.clone()],
    };
    let needs = stmts.iter().any(|s| match &s.node {
        StmtNode::Import(m, a) => !is_builtin_module(m) && (a.as_deref() == Some("*") || m.starts_with("package:")),
        _ => false,
    });
    if !needs { return Ok(None); }
    let base_dir = entry.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let mut ctx = FlattenCtx::new(&crate::packages::app_root_of(entry));
    let flat = flatten_app(ast, &base_dir, &mut ctx, &entry.display().to_string(), None, false)?;
    Ok(Some(Stmt { node: StmtNode::Block(flat), line: ast.line }))
}

pub(crate) fn transpile_app(flat: &Stmt, base_dir: &PathBuf) -> Result<String, String> {
    APP_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let c = transpile(flat, false, base_dir);
    let marker = "int main(int argc, char** argv) {";
    if !c.contains(marker) {
        return Err("internal error: generated C has no main() to turn into the app boot function".into());
    }
    let mut c = c.replacen(marker, "static int dgn_app_main(int argc, char** argv) {", 1);
    c.push_str(GLUE);
    Ok(c)
}

fn windows_compiler() -> Result<(String, Vec<String>), String> {
    if let Ok(v) = std::env::var("DRAGON_WINDOWS_CC") {
        let mut parts = v.split_whitespace().map(|s| s.to_string());
        if let Some(p) = parts.next() { return Ok((p, parts.collect())); }
    }
    if which_exists("x86_64-w64-mingw32-gcc") { return Ok(("x86_64-w64-mingw32-gcc".into(), vec![])); }
    if which_exists("zig") {
        return Ok(("zig".into(), vec!["cc".into(), "-target".into(), "x86_64-windows-gnu".into()]));
    }
    if Command::new("python3").args(["-m", "ziglang", "version"]).output().map(|o| o.status.success()).unwrap_or(false) {
        return Ok(("python3".into(), vec!["-m".into(), "ziglang".into(), "cc".into(), "-target".into(), "x86_64-windows-gnu".into()]));
    }
    if cfg!(target_os = "windows") {
        for c in ["gcc", "clang"] { if which_exists(c) { return Ok((c.into(), vec![])); } }
    }
    Err("no Windows C compiler found. Install one of: Zig (https://ziglang.org, works on every OS), MSYS2/MinGW-w64 (gcc), or LLVM clang on Windows; or set DRAGON_WINDOWS_CC.".into())
}

pub fn compile_app(
    entry: &Path,
    out_path: &Path,
    keep_c: bool,
    cross: &CrossBuildOptions,
    test_main: bool,
) -> Result<(), String> {
    let src = std::fs::read_to_string(entry).map_err(|e| format!("Error reading {}: {}", entry.display(), e))?;
    let tokens = lex(&src).map_err(|e| format!("Syntax Error: {}", e.with_source(&src).format_pretty()))?;
    let ast = Parser::new(tokens).parse().map_err(|e| format!("Parse Error: {}", e))?;
    let base_dir = entry.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));

    let mut ctx = FlattenCtx::new(&crate::packages::app_root_of(entry));
    let stmts = flatten_app(&ast, &base_dir, &mut ctx, &entry.display().to_string(), None, false)?;
    let flat = Stmt { node: StmtNode::Block(stmts), line: ast.line };

    let report = crate::engine::lint_ast(&flat);
    if !report.errors.is_empty() {
        report.print();
        return Err("lint errors (see above)".into());
    }

    println!("  [native] Transpiling app to C...");
    let c_src = transpile_app(&flat, &base_dir)?;
    let stem = out_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let c_path = std::env::temp_dir().join(format!("dragon_app_{}.c", stem));
    std::fs::write(&c_path, &c_src).map_err(|e| format!("Failed to write C source: {}", e))?;
    println!("  [native] C source -> {}", c_path.display());

    let result = (|| -> Result<(), String> {
        if cross.targets.is_empty() {
            let cc = if which_exists("gcc") { "gcc" } else if which_exists("clang") { "clang" } else { "cc" };
            let mut args: Vec<String> = vec![c_path.to_string_lossy().to_string()];
            args.extend(["-O2", "-fno-inline-functions", "-o"].iter().map(|s| s.to_string()));
            args.push(out_path.to_string_lossy().to_string());
            args.extend(["-lm", "-lpthread", "-w"].iter().map(|s| s.to_string()));
            if test_main {
                args.push("-DDGN_APP_TEST_MAIN".into());
            } else {
                args.extend(["-shared", "-fPIC"].iter().map(|s| s.to_string()));
            }
            let st = Command::new(cc).args(&args).status().map_err(|e| format!("failed to run {}: {}", cc, e))?;
            if !st.success() {
                return Err("C compiler exited with non-zero status".into());
            }
            println!("Success! Native output: {}", out_path.display());
            return Ok(());
        }
        let out_dir = out_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        let mut errors = Vec::new();
        for t in cross.targets.iter().copied().filter(|t| t.is_android()) {
            let clang = match ndk_clang_path(t, cross.android_api) {
                Ok(c) => c,
                Err(e) => { errors.push(format!("[{}] {}", t.label(), e)); continue; }
            };
            let abi_dir = out_dir.join("jniLibs").join(t.android_abi_dir());
            std::fs::create_dir_all(&abi_dir).map_err(|e| format!("could not create {}: {}", abi_dir.display(), e))?;
            let so_name = format!("lib{}.so", stem);
            let so_path = abi_dir.join(&so_name);
            let soname = format!("-Wl,-soname,{}", so_name);
            println!("  [native] [{}] compiling with {}", t.label(), clang.display());
            let st = Command::new(&clang)
                .arg(&c_path)
                .args(["-shared", "-fPIC", "-O2", "-fno-inline-functions", "-DDGN_APP_JNI", "-w", "-o"])
                .arg(&so_path)
                .args(["-lm", "-llog", soname.as_str(), "-Wl,-z,max-page-size=16384"])
                .status();
            match st {
                Ok(s) if s.success() => println!("  [native] [{}] -> {}", t.label(), so_path.display()),
                Ok(s) => errors.push(format!("[{}] clang exited with {}", t.label(), s)),
                Err(e) => errors.push(format!("[{}] failed to launch {}: {}", t.label(), clang.display(), e)),
            }
        }
        for t in cross.targets.iter().copied().filter(|t| t.is_windows()) {
            let dll = out_dir.join(format!("{}.dll", stem));
            match windows_compiler() {
                Ok((prog, pre)) => {
                    println!("  [native] [{}] compiling with {}", t.label(), prog);
                    let st = Command::new(&prog).args(&pre).arg(&c_path)
                        .args(["-shared", "-O2", "-fno-inline-functions", "-DDGN_APP_WINDOWS", "-w", "-o"])
                        .arg(&dll)
                        .args(["-lws2_32"])
                        .status();
                    match st {
                        Ok(s) if s.success() => {
                            for ext in ["pdb", "lib", "exp"] { let _ = std::fs::remove_file(out_dir.join(format!("{}.{}", stem, ext))); }
                            let _ = std::fs::remove_file(out_dir.join(format!("dragon_app_{}.lib", stem)));
                            println!("  [native] [{}] -> {}", t.label(), dll.display())
                        }
                        Ok(s) => errors.push(format!("[{}] C compiler exited with {}", t.label(), s)),
                        Err(e) => errors.push(format!("[{}] failed to launch {}: {}", t.label(), prog, e)),
                    }
                }
                Err(e) => errors.push(format!("[{}] {}", t.label(), e)),
            }
        }
        let ios_targets: Vec<CrossTarget> = cross.targets.iter().copied().filter(|t| t.is_ios()).collect();
        if !ios_targets.is_empty() {
            let ios_dir = out_dir.join("ios");
            let c_str = c_path.to_string_lossy().to_string();
            let mut slices: Vec<(CrossTarget, PathBuf)> = Vec::new();
            for t in ios_targets {
                match super::build_ios_slice(&c_str, &ios_dir, &stem, t, &cross.ios_min_version, &["-DDGN_APP_IOS", "-w"]) {
                    Ok(a) => slices.push((t, a)),
                    Err(e) => errors.push(format!("[{}] {}", t.label(), e)),
                }
            }
            if cross.make_xcframework && !slices.is_empty() {
                match super::package_xcframework(&out_dir, &stem, &slices, Some(IOS_HEADER)) {
                    Ok(p) => println!("  [native] xcframework -> {}", p.display()),
                    Err(e) => errors.push(format!("[xcframework] {}", e)),
                }
            } else if !slices.is_empty() {
                let inc = ios_dir.join("include");
                let _ = std::fs::create_dir_all(&inc);
                let _ = std::fs::write(inc.join(format!("{}.h", stem)), IOS_HEADER);
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("\n  - ")) }
    })();
    if !keep_c { let _ = std::fs::remove_file(&c_path); }
    result
}
