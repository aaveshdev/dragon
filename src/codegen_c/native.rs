#![allow(dead_code)]

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum NativeType {
    Int,
    Float,
    Str,
    Bool,
    StrList,
}

impl NativeType {
    fn c_type(self) -> &'static str {
        match self {
            NativeType::Int => "int64_t",
            NativeType::Float => "double",
            NativeType::Str => "const char*",
            NativeType::Bool => "int",
            NativeType::StrList => "DgnListStr*",
        }
    }
    fn from_annotation(s: &str) -> Option<NativeType> {
        match s.trim() {
            "int" | "i32" | "i64" | "long" => Some(NativeType::Int),
            "float" | "double" | "f64" | "f32" | "number" => Some(NativeType::Float),
            "str" | "string" => Some(NativeType::Str),
            "bool" => Some(NativeType::Bool),
            "strlist" => Some(NativeType::StrList),
            _ => None,
        }
    }
}

fn mangle_plain(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn parse_param(raw_name: &str, raw_type: &Option<String>) -> (String, Option<NativeType>) {
    let ty = raw_type.as_deref().and_then(NativeType::from_annotation);
    (raw_name.trim().to_string(), ty)
}

#[derive(Clone)]
pub(crate) struct LibFnSig {
    pub params: Vec<(String, NativeType)>,
    pub ret: NativeType,
}

fn collect_lib_fns(ast: &Stmt) -> Vec<(String, Vec<(String, Option<String>)>, Stmt)> {
    let mut out = Vec::new();
    match &ast.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { out.extend(collect_lib_fns(s)); }
        }
        StmtNode::FuncDef(name, params, _, body, _) => {
            out.push((name.clone(), params.clone(), (**body).clone()));
        }
        _ => {}
    }
    out
}

fn collect_lib_globals(ast: &Stmt) -> Result<Vec<(String, NativeType, String)>, String> {
    let mut out = Vec::new();
    collect_lib_globals_inner(ast, &mut out)?;
    Ok(out)
}
fn collect_lib_globals_inner(ast: &Stmt, out: &mut Vec<(String, NativeType, String)>) -> Result<(), String> {
    match &ast.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_lib_globals_inner(s, out)?; }
        }
        StmtNode::ConstDecl(name, e) => {
            if let Some((ty, lit)) = expr_to_native_literal(e) {
                out.push((name.clone(), ty, lit));
            }
        }
        StmtNode::Expr(e) => {
            if let ExprNode::Assign(target, val) = &e.node {
                if let ExprNode::Var(name) = &target.node {
                    if let Some((ty, lit)) = expr_to_native_literal(val) {
                        out.push((name.clone(), ty, lit));
                    } else {
                        return Err(format!(
                            "--native --lib: top-level global '{}' must be initialized with a literal \
                             (number, string, or bool) — expressions aren't supported for module-level state.",
                            name
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn expr_to_native_literal(e: &Expr) -> Option<(NativeType, String)> {
    match &e.node {
        ExprNode::Number(n) => {
            if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                Some((NativeType::Int, format!("{}LL", *n as i64)))
            } else {
                Some((NativeType::Float, format!("{:.17}", n)))
            }
        }
        ExprNode::String(s) => Some((NativeType::Str, format!("\"{}\"", escape_c_string(s)))),
        ExprNode::Bool(b) => Some((NativeType::Bool, if *b { "1".into() } else { "0".into() })),

        ExprNode::List(items) if items.is_empty() => Some((NativeType::StrList, "NULL".to_string())),
        _ => None,
    }
}

fn builtin_call(name: &str, arg_types: &[NativeType]) -> Option<(NativeType, &'static str)> {
    match (name, arg_types) {
        ("sqrt", [NativeType::Int]) | ("sqrt", [NativeType::Float]) => Some((NativeType::Float, "sqrt")),
        ("abs", [NativeType::Int]) => Some((NativeType::Int, "llabs")),
        ("abs", [NativeType::Float]) => Some((NativeType::Float, "fabs")),
        ("floor", [NativeType::Int]) | ("floor", [NativeType::Float]) => Some((NativeType::Float, "floor")),
        ("ceil", [NativeType::Int]) | ("ceil", [NativeType::Float]) => Some((NativeType::Float, "ceil")),
        ("pow", [a, b]) if matches!(a, NativeType::Int | NativeType::Float) && matches!(b, NativeType::Int | NativeType::Float) => Some((NativeType::Float, "pow")),
        ("min", [NativeType::Int, NativeType::Int]) => Some((NativeType::Int, "__dgn_lib_min_i")),
        ("max", [NativeType::Int, NativeType::Int]) => Some((NativeType::Int, "__dgn_lib_max_i")),
        ("min", [NativeType::Float, NativeType::Float]) => Some((NativeType::Float, "__dgn_lib_min_f")),
        ("max", [NativeType::Float, NativeType::Float]) => Some((NativeType::Float, "__dgn_lib_max_f")),
        ("len", [NativeType::Str]) => Some((NativeType::Int, "(int64_t)strlen")),
        ("len", [NativeType::StrList]) => Some((NativeType::Int, "__dgn_list_len")),
        ("substr", [NativeType::Str, a, b]) if matches!(a, NativeType::Int) && matches!(b, NativeType::Int) => {
            Some((NativeType::Str, "__dgn_lib_substr"))
        }
        ("index_of", [NativeType::Str, NativeType::Str]) => Some((NativeType::Int, "__dgn_lib_index_of")),
        ("char_at", [NativeType::Str, NativeType::Int]) => Some((NativeType::Str, "__dgn_lib_char_at")),
        ("parse_int", [NativeType::Str]) => Some((NativeType::Int, "atoll")),
        ("parse_float", [NativeType::Str]) => Some((NativeType::Float, "atof")),
        ("file_write", [NativeType::Str, NativeType::Str]) => Some((NativeType::Bool, "__dgn_file_write")),
        ("file_append", [NativeType::Str, NativeType::Str]) => Some((NativeType::Bool, "__dgn_file_append")),
        ("file_read", [NativeType::Str]) => Some((NativeType::Str, "__dgn_file_read")),
        ("file_exists", [NativeType::Str]) => Some((NativeType::Bool, "__dgn_file_exists")),
        ("file_delete", [NativeType::Str]) => Some((NativeType::Bool, "__dgn_file_delete")),
        ("json_escape", [NativeType::Str]) => Some((NativeType::Str, "__dgn_json_escape")),
        _ => None,
    }
}

const BUILTIN_HELPERS: &str = "\
static int64_t __dgn_lib_min_i(int64_t a, int64_t b) { return a < b ? a : b; }\n\
static int64_t __dgn_lib_max_i(int64_t a, int64_t b) { return a > b ? a : b; }\n\
static double __dgn_lib_min_f(double a, double b) { return a < b ? a : b; }\n\
static double __dgn_lib_max_f(double a, double b) { return a > b ? a : b; }\n\
static char* __dgn_lib_substr(const char* s, int64_t start, int64_t len) {\n\
    int64_t slen = (int64_t)strlen(s);\n\
    if (start < 0) start = 0;\n\
    if (start > slen) start = slen;\n\
    if (len < 0) len = 0;\n\
    if (start + len > slen) len = slen - start;\n\
    char* out = (char*)malloc((size_t)len + 1);\n\
    memcpy(out, s + start, (size_t)len);\n\
    out[len] = '\\0';\n\
    return out;\n\
}\n\
static int64_t __dgn_lib_index_of(const char* s, const char* needle) {\n\
    const char* found = strstr(s, needle);\n\
    if (!found) return -1;\n\
    return (int64_t)(found - s);\n\
}\n\
static char* __dgn_lib_char_at(const char* s, int64_t idx) {\n\
    int64_t slen = (int64_t)strlen(s);\n\
    char* out = (char*)malloc(2);\n\
    if (idx < 0 || idx >= slen) { out[0] = '\\0'; return out; }\n\
    out[0] = s[idx];\n\
    out[1] = '\\0';\n\
    return out;\n\
}\n\
DGN_EXPORT void dgn_free_string(char* s) { free(s); }\n\
static char* __dgn_lib_strdup(const char* s) {\n\
    size_t n = strlen(s) + 1;\n\
    char* out = (char*)malloc(n);\n\
    memcpy(out, s, n);\n\
    return out;\n\
}\n\
\n\
typedef struct {\n\
    char** items;   /* each entry individually malloc'd, owned by the list */\n\
    int64_t len;\n\
    int64_t cap;\n\
} DgnListStr;\n\
\n\
static DgnListStr* __dgn_list_new(void) {\n\
    DgnListStr* l = (DgnListStr*)malloc(sizeof(DgnListStr));\n\
    l->items = NULL;\n\
    l->len = 0;\n\
    l->cap = 0;\n\
    return l;\n\
}\n\
static void __dgn_list_push(DgnListStr* l, char* item) {\n\
    if (l->len >= l->cap) {\n\
        int64_t new_cap = l->cap == 0 ? 4 : l->cap * 2;\n\
        l->items = (char**)realloc(l->items, sizeof(char*) * (size_t)new_cap);\n\
        l->cap = new_cap;\n\
    }\n\
    l->items[l->len] = item;\n\
    l->len++;\n\
}\n\
static const char* __dgn_list_get(DgnListStr* l, int64_t idx) {\n\
    if (!l || idx < 0 || idx >= l->len) return \"\";\n\
    return l->items[idx];\n\
}\n\
static int64_t __dgn_list_len(DgnListStr* l) {\n\
    return l ? l->len : 0;\n\
}\n\
static char* __dgn_list_pop(DgnListStr* l) {\n\
    if (!l || l->len == 0) {\n\
        char* out = (char*)malloc(1);\n\
        out[0] = '\\0';\n\
        return out;\n\
    }\n\
    l->len--;\n\
    return l->items[l->len];\n\
}\n\
static void __dgn_list_free(void* ptr) {\n\
    DgnListStr* l = (DgnListStr*)ptr;\n\
    if (!l) return;\n\
    for (int64_t i = 0; i < l->len; i++) {\n\
        free(l->items[i]);\n\
    }\n\
    free(l->items);\n\
    free(l);\n\
}\n\
static DgnListStr* __dgn_list_clone(DgnListStr* src) {\n\
    DgnListStr* out = __dgn_list_new();\n\
    if (!src) return out;\n\
    for (int64_t i = 0; i < src->len; i++) {\n\
        size_t n = strlen(src->items[i]) + 1;\n\
        char* copy = (char*)malloc(n);\n\
        memcpy(copy, src->items[i], n);\n\
        __dgn_list_push(out, copy);\n\
    }\n\
    return out;\n\
}\n\
static DgnListStr* __dgn_list_concat(DgnListStr* a, DgnListStr* b) {\n\
    DgnListStr* out = __dgn_list_new();\n\
    if (a) { for (int64_t i = 0; i < a->len; i++) { size_t n = strlen(a->items[i]) + 1; char* c = (char*)malloc(n); memcpy(c, a->items[i], n); __dgn_list_push(out, c); } }\n\
    if (b) { for (int64_t i = 0; i < b->len; i++) { size_t n = strlen(b->items[i]) + 1; char* c = (char*)malloc(n); memcpy(c, b->items[i], n); __dgn_list_push(out, c); } }\n\
    return out;\n\
}\n\
DGN_EXPORT void dgn_free_strlist(DgnListStr* l) { __dgn_list_free((void*)l); }\n\
DGN_EXPORT int64_t dgn_strlist_len(DgnListStr* l) { return __dgn_list_len(l); }\n\
DGN_EXPORT const char* dgn_strlist_get(DgnListStr* l, int64_t idx) { return __dgn_list_get(l, idx); }\n\
\n\
static int __dgn_file_write_mode(const char* path, const char* content, const char* mode) {\n\
    FILE* f = fopen(path, mode);\n\
    if (!f) return 0;\n\
    size_t len = strlen(content);\n\
    size_t written = fwrite(content, 1, len, f);\n\
    fclose(f);\n\
    return written == len;\n\
}\n\
static int __dgn_file_write(const char* path, const char* content) {\n\
    return __dgn_file_write_mode(path, content, \"wb\");\n\
}\n\
static int __dgn_file_append(const char* path, const char* content) {\n\
    return __dgn_file_write_mode(path, content, \"ab\");\n\
}\n\
static char* __dgn_file_read(const char* path) {\n\
    FILE* f = fopen(path, \"rb\");\n\
    if (!f) {\n\
        char* empty = (char*)malloc(1);\n\
        empty[0] = '\\0';\n\
        return empty;\n\
    }\n\
    fseek(f, 0, SEEK_END);\n\
    long size = ftell(f);\n\
    if (size < 0) {\n\
        fclose(f);\n\
        char* empty = (char*)malloc(1);\n\
        empty[0] = '\\0';\n\
        return empty;\n\
    }\n\
    fseek(f, 0, SEEK_SET);\n\
    char* buf = (char*)malloc((size_t)size + 1);\n\
    size_t read_n = fread(buf, 1, (size_t)size, f);\n\
    buf[read_n] = '\\0';\n\
    fclose(f);\n\
    return buf;\n\
}\n\
static int __dgn_file_exists(const char* path) {\n\
    FILE* f = fopen(path, \"rb\");\n\
    if (f) { fclose(f); return 1; }\n\
    return 0;\n\
}\n\
static int __dgn_file_delete(const char* path) {\n\
    return remove(path) == 0;\n\
}\n\
\n\
static char* __dgn_json_escape(const char* s) {\n\
    size_t len = strlen(s);\n\
    size_t out_len = 0;\n\
    for (size_t i = 0; i < len; i++) {\n\
        unsigned char c = (unsigned char)s[i];\n\
        if (c == '\"' || c == '\\\\') {\n\
            out_len += 2;\n\
        } else if (c == '\\b' || c == '\\f' || c == '\\n' || c == '\\r' || c == '\\t') {\n\
            out_len += 2;\n\
        } else if (c < 0x20) {\n\
            out_len += 6; /* \\u00XX */\n\
        } else {\n\
            out_len += 1;\n\
        }\n\
    }\n\
    char* out = (char*)malloc(out_len + 1);\n\
    size_t j = 0;\n\
    for (size_t i = 0; i < len; i++) {\n\
        unsigned char c = (unsigned char)s[i];\n\
        if (c == '\"') { out[j++] = '\\\\'; out[j++] = '\"'; }\n\
        else if (c == '\\\\') { out[j++] = '\\\\'; out[j++] = '\\\\'; }\n\
        else if (c == '\\b') { out[j++] = '\\\\'; out[j++] = 'b'; }\n\
        else if (c == '\\f') { out[j++] = '\\\\'; out[j++] = 'f'; }\n\
        else if (c == '\\n') { out[j++] = '\\\\'; out[j++] = 'n'; }\n\
        else if (c == '\\r') { out[j++] = '\\\\'; out[j++] = 'r'; }\n\
        else if (c == '\\t') { out[j++] = '\\\\'; out[j++] = 't'; }\n\
        else if (c < 0x20) {\n\
            static const char hex[] = \"0123456789abcdef\";\n\
            out[j++] = '\\\\'; out[j++] = 'u'; out[j++] = '0'; out[j++] = '0';\n\
            out[j++] = hex[(c >> 4) & 0xF];\n\
            out[j++] = hex[c & 0xF];\n\
        } else {\n\
            out[j++] = (char)c;\n\
        }\n\
    }\n\
    out[j] = '\\0';\n\
    return out;\n\
}\n\
";

fn is_freeable(t: NativeType) -> bool {
    matches!(t, NativeType::Str | NativeType::StrList)
}

fn is_number_literal(e: &Expr) -> bool {
    match &e.node {
        ExprNode::Number(_) => true,
        ExprNode::Unary(op, inner) if op == "-" => matches!(inner.node, ExprNode::Number(_)),
        _ => false,
    }
}

fn free_fn_for(t: NativeType) -> &'static str {
    match t {
        NativeType::StrList => "__dgn_list_free",
        _ => "free",
    }
}

struct TypeScope {
    vars: std::collections::HashMap<String, NativeType>,
}
impl TypeScope {
    fn get(&self, name: &str) -> Result<NativeType, String> {
        self.vars.get(name).copied().ok_or_else(|| {
            format!("--native --lib: reference to undeclared variable '{}'", name)
        })
    }
}

fn infer_expr_type(
    e: &Expr,
    scope: &mut TypeScope,
    sigs: &std::collections::HashMap<String, LibFnSig>,
    fn_name: &str,
) -> Result<NativeType, String> {
    match &e.node {
        ExprNode::Number(n) => Ok(if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 { NativeType::Int } else { NativeType::Float }),
        ExprNode::String(_) => Ok(NativeType::Str),
        ExprNode::Bool(_) => Ok(NativeType::Bool),
        ExprNode::Var(name) => scope.get(name),
        ExprNode::List(items) => {
            if items.is_empty() {
                return Ok(NativeType::StrList);
            }
            for item in items {
                let it = infer_expr_type(item, scope, sigs, fn_name)?;
                if it != NativeType::Str {
                    return Err(format!(
                        "--native --lib: in '{}', list literals only support str elements right now, \
                         found a {:?} element -- convert it to a string first",
                        fn_name, it
                    ));
                }
            }
            Ok(NativeType::StrList)
        }
        ExprNode::Index(list_expr, idx_expr) => {
            let lt = infer_expr_type(list_expr, scope, sigs, fn_name)?;
            let it = infer_expr_type(idx_expr, scope, sigs, fn_name)?;
            if lt != NativeType::StrList {
                return Err(format!(
                    "--native --lib: in '{}', indexing with [] is only supported on a strlist, not {:?}",
                    fn_name, lt
                ));
            }
            if it != NativeType::Int {
                return Err(format!("--native --lib: in '{}', a list index must be an int, not {:?}", fn_name, it));
            }
            Ok(NativeType::Str)
        }
        ExprNode::Binary(op, l, r) => {
            let lt = infer_expr_type(l, scope, sigs, fn_name)?;
            let rt = infer_expr_type(r, scope, sigs, fn_name)?;
            match op.as_str() {
                "+" if lt == NativeType::Str || rt == NativeType::Str => Ok(NativeType::Str),
                "+" if lt == NativeType::StrList && rt == NativeType::StrList => Ok(NativeType::StrList),
                "+" | "-" | "*" | "%" => {
                    if lt == NativeType::Str || rt == NativeType::Str {
                        return Err(format!(
                            "--native --lib: in '{}', operator '{}' mixes a string with a non-string operand",
                            fn_name, op
                        ));
                    }
                    if lt == NativeType::StrList || rt == NativeType::StrList {
                        return Err(format!(
                            "--native --lib: in '{}', operator '{}' isn't supported on a strlist \
                             (only '+' between two strlists, for concatenation, is)",
                            fn_name, op
                        ));
                    }
                    Ok(if lt == NativeType::Float || rt == NativeType::Float { NativeType::Float } else { NativeType::Int })
                }
                "/" => Ok(NativeType::Float),
                "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||" | "and" | "or" => Ok(NativeType::Bool),
                _ => Err(format!("--native --lib: in '{}', unsupported operator '{}'", fn_name, op)),
            }
        }
        ExprNode::Unary(op, inner) => {
            let it = infer_expr_type(inner, scope, sigs, fn_name)?;
            match op.as_str() {
                "-" if it == NativeType::Int || it == NativeType::Float => Ok(it),
                "!" | "not" => Ok(NativeType::Bool),
                _ => Err(format!("--native --lib: in '{}', unsupported unary operator '{}' on {:?}", fn_name, op, it)),
            }
        }
        ExprNode::Ternary(cond, t, e2) => {
            infer_expr_type(cond, scope, sigs, fn_name)?;
            let tt = infer_expr_type(t, scope, sigs, fn_name)?;
            let et = infer_expr_type(e2, scope, sigs, fn_name)?;
            if tt != et {
                return Err(format!(
                    "--native --lib: in '{}', the two branches of a ternary have different types ({:?} vs {:?})",
                    fn_name, tt, et
                ));
            }
            Ok(tt)
        }
        ExprNode::Call(callee, args) => {
            if let ExprNode::GetAttr(receiver, method) = &callee.node {
                let recv_name = match &receiver.node {
                    ExprNode::Var(n) => n.clone(),
                    _ => return Err(format!(
                        "--native --lib: in '{}', '.{}(...)' is only supported on a plain variable \
                         (e.g. `mylist.{}(...)`), not on an arbitrary expression",
                        fn_name, method, method
                    )),
                };
                let recv_type = infer_expr_type(receiver, scope, sigs, fn_name)?;
                if recv_type != NativeType::StrList {
                    return Err(format!(
                        "--native --lib: in '{}', '.{}()' is only supported on a strlist, but '{}' is {:?}",
                        fn_name, method, recv_name, recv_type
                    ));
                }
                return match method.as_str() {
                    "push" => {
                        if args.len() != 1 {
                            return Err(format!("--native --lib: in '{}', list.push(item) takes exactly 1 argument", fn_name));
                        }
                        let at = infer_expr_type(&args[0], scope, sigs, fn_name)?;
                        if at != NativeType::Str {
                            return Err(format!(
                                "--native --lib: in '{}', list.push(item) expects a str argument, got {:?}",
                                fn_name, at
                            ));
                        }
                        Ok(NativeType::Int)
                    }
                    "pop" => {
                        if !args.is_empty() {
                            return Err(format!("--native --lib: in '{}', list.pop() takes no arguments", fn_name));
                        }
                        Ok(NativeType::Str)
                    }
                    other => Err(format!(
                        "--native --lib: in '{}', unsupported list method '.{}()' -- only .push(item) \
                         and .pop() are supported (use len(list) for length and list[i] for indexing)",
                        fn_name, other
                    )),
                };
            }
            let name = match &callee.node {
                ExprNode::Var(n) => n,
                _ => return Err(format!("--native --lib: in '{}', only calling a plain function name is supported", fn_name)),
            };
            let arg_types: Result<Vec<NativeType>, String> =
                args.iter().map(|a| infer_expr_type(a, scope, sigs, fn_name)).collect();
            let arg_types = arg_types?;
            if let Some(sig) = sigs.get(name) {
                if sig.params.len() != arg_types.len() {
                    return Err(format!(
                        "--native --lib: in '{}', call to '{}' passes {} argument(s), expected {}",
                        fn_name, name, arg_types.len(), sig.params.len()
                    ));
                }
                for (i, ((_, pt), at)) in sig.params.iter().zip(arg_types.iter()).enumerate() {
                    let ok = pt == at || (*pt == NativeType::Float && *at == NativeType::Int && is_number_literal(&args[i]));
                    if !ok {
                        return Err(format!(
                            "--native --lib: in '{}', call to '{}' argument {} has type {:?}, expected {:?}",
                            fn_name, name, i + 1, at, pt
                        ));
                    }
                }
                Ok(sig.ret)
            } else if let Some((ret, _)) = builtin_call(name, &arg_types) {
                Ok(ret)
            } else {
                Err(format!(
                    "--native --lib: in '{}', call to unknown function '{}' (not a top-level function in this file, \
                     and not one of the supported builtins: sqrt, abs, floor, ceil, pow, min, max, len)",
                    fn_name, name
                ))
            }
        }
        ExprNode::Assign(target, val) => {
            let vname = match &target.node {
                ExprNode::Var(n) => n.clone(),
                _ => return Err(format!("--native --lib: in '{}', assignment target must be a plain variable", fn_name)),
            };
            let et = infer_expr_type(val, scope, sigs, fn_name)?;
            match scope.vars.get(&vname).copied() {
                Some(vt) if vt != et && !(vt == NativeType::Float && et == NativeType::Int && is_number_literal(val)) => Err(format!(
                    "--native --lib: in '{}', assigning a {:?} value to '{}' which is {:?}",
                    fn_name, et, vname, vt
                )),
                Some(vt) => Ok(vt),
                None => {
                    scope.vars.insert(vname, et);
                    Ok(et)
                }
            }
        }
        ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
            let it = infer_expr_type(inner, scope, sigs, fn_name)?;
            if (op == "++" || op == "--") && it == NativeType::Int {
                Ok(it)
            } else {
                Err(format!("--native --lib: in '{}', '{}' requires an int variable", fn_name, op))
            }
        }
        _ => Err(format!(
            "--native --lib: in '{}', this expression form isn't supported in native-lib mode \
             (only numbers, strings, bools, variables, arithmetic, comparisons, calls to other \
             top-level functions or the supported builtins, and simple assignment)",
            fn_name
        )),
    }
}

fn infer_return_type(
    body: &Stmt,
    scope: &mut TypeScope,
    sigs: &std::collections::HashMap<String, LibFnSig>,
    fn_name: &str,
) -> Result<Option<NativeType>, String> {
    let mut ret: Option<NativeType> = None;
    infer_return_type_stmt(body, scope, sigs, fn_name, &mut ret)?;
    Ok(ret)
}
fn infer_return_type_stmt(
    stmt: &Stmt,
    scope: &mut TypeScope,
    sigs: &std::collections::HashMap<String, LibFnSig>,
    fn_name: &str,
    ret: &mut Option<NativeType>,
) -> Result<(), String> {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { infer_return_type_stmt(s, scope, sigs, fn_name, ret)?; }
        }
        StmtNode::ConstDecl(name, e) => {
            let t = infer_expr_type(e, scope, sigs, fn_name)?;
            scope.vars.insert(name.clone(), t);
        }
        StmtNode::Expr(e) => { infer_expr_type(e, scope, sigs, fn_name)?; }
        StmtNode::Return(Some(e)) => {
            let t = infer_expr_type(e, scope, sigs, fn_name)?;
            match ret {
                None => *ret = Some(t),
                Some(existing) if *existing != t => {
                    return Err(format!(
                        "--native --lib: '{}' returns both {:?} and {:?} on different paths -- \
                         every return in a native-lib function must agree on one type",
                        fn_name, existing, t
                    ));
                }
                _ => {}
            }
        }
        StmtNode::Return(None) => {
            return Err(format!(
                "--native --lib: '{}' has a bare 'return' with no value -- every path in a native-lib \
                 function must return a value (there's no null/void value representable across the C ABI here)",
                fn_name
            ));
        }
        StmtNode::IfChain(branches) => {
            for (cond, b) in branches {
                if let Some(c) = cond { infer_expr_type(c, scope, sigs, fn_name)?; }
                infer_return_type_stmt(b, scope, sigs, fn_name, ret)?;
            }
        }
        StmtNode::While(cond, body) => {
            infer_expr_type(cond, scope, sigs, fn_name)?;
            infer_return_type_stmt(body, scope, sigs, fn_name, ret)?;
        }
        StmtNode::DoWhile(body, cond) => {
            infer_return_type_stmt(body, scope, sigs, fn_name, ret)?;
            infer_expr_type(cond, scope, sigs, fn_name)?;
        }
        StmtNode::CFor(init, cond, step, body) => {
            if let Some(e) = init { infer_expr_type(e, scope, sigs, fn_name)?; }
            if let Some(e) = cond { infer_expr_type(e, scope, sigs, fn_name)?; }
            if let Some(e) = step { infer_expr_type(e, scope, sigs, fn_name)?; }
            infer_return_type_stmt(body, scope, sigs, fn_name, ret)?;
        }
        StmtNode::For(var_name, value_var, iter_expr, body) => {
            if value_var.is_some() {
                return Err(format!(
                    "--native --lib: in '{}', 'for index, value in ...' isn't supported yet -- \
                     use 'for value in ...' and track the index yourself with a counter variable",
                    fn_name
                ));
            }
            let it = infer_expr_type(iter_expr, scope, sigs, fn_name)?;
            if it != NativeType::StrList {
                return Err(format!(
                    "--native --lib: in '{}', 'for x in ...' only supports iterating a strlist right now, not {:?}",
                    fn_name, it
                ));
            }
            scope.vars.insert(var_name.clone(), NativeType::Str);
            infer_return_type_stmt(body, scope, sigs, fn_name, ret)?;
        }
        StmtNode::Break | StmtNode::Continue => {}
        _ => {
            return Err(format!(
                "--native --lib: '{}' uses a statement form not supported in native-lib mode \
                 (supported: if/else, while, do-while, C-style for, return, variable declarations, \
                 assignment, break, continue) -- classes, try/catch, switch, closures, and each-loops \
                 over collections aren't representable as plain C here",
                fn_name
            ));
        }
    }
    Ok(())
}

fn assigned_names(stmt: &Stmt) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    assigned_names_stmt(stmt, &mut out);
    out
}

fn assigned_names_stmt(stmt: &Stmt, out: &mut std::collections::HashSet<String>) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { assigned_names_stmt(s, out); }
        }
        StmtNode::IfChain(branches) => {
            for (cond, body) in branches {
                if let Some(c) = cond { assigned_names_expr(c, out); }
                assigned_names_stmt(body, out);
            }
        }
        StmtNode::While(cond, body) => {
            assigned_names_expr(cond, out);
            assigned_names_stmt(body, out);
        }
        StmtNode::DoWhile(body, cond) => {
            assigned_names_stmt(body, out);
            assigned_names_expr(cond, out);
        }
        StmtNode::Expr(e) => assigned_names_expr(e, out),
        StmtNode::Return(Some(e)) => assigned_names_expr(e, out),
        StmtNode::For(_, _, iter, body) => {
            assigned_names_expr(iter, out);
            assigned_names_stmt(body, out);
        }
        _ => {}
    }
}

fn assigned_names_expr(expr: &Expr, out: &mut std::collections::HashSet<String>) {
    match &expr.node {
        ExprNode::Assign(target, val) => {
            if let ExprNode::Var(name) = &target.node {
                out.insert(name.clone());
            }
            assigned_names_expr(val, out);
        }
        ExprNode::Binary(_, l, r) => { assigned_names_expr(l, out); assigned_names_expr(r, out); }
        ExprNode::Unary(_, e) => assigned_names_expr(e, out),
        ExprNode::Ternary(c, t, e) => { assigned_names_expr(c, out); assigned_names_expr(t, out); assigned_names_expr(e, out); }
        ExprNode::Call(_, args) => { for a in args { assigned_names_expr(a, out); } }
        ExprNode::PostOp(_, e) | ExprNode::PreOp(_, e) => assigned_names_expr(e, out),
        _ => {}
    }
}

struct NativeTranspiler<'a> {
    out: String,
    indent: usize,
    tmp_ctr: usize,
    scope: TypeScope,
    sigs: &'a std::collections::HashMap<String, LibFnSig>,
    fn_name: &'a str,
    ret_type: NativeType,
    owned_temps: std::collections::HashMap<String, NativeType>,
    freeable_vars_with_flag: std::collections::HashMap<String, NativeType>,
    global_freeable_names: std::collections::HashSet<String>,
}

impl<'a> NativeTranspiler<'a> {
    fn line(&mut self, s: &str) {
        let pad = "    ".repeat(self.indent);
        self.out.push_str(&pad);
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn fresh(&mut self) -> String {
        self.tmp_ctr += 1;
        format!("_t{}", self.tmp_ctr)
    }

    fn default_return_literal(&self) -> &'static str {
        match self.ret_type {
            NativeType::Str => "\"\"",
            NativeType::StrList => "NULL",
            NativeType::Float => "0.0",
            NativeType::Int | NativeType::Bool => "0",
        }
    }

    fn emit_owned_locals_cleanup(&mut self, exclude: Option<&str>) {
        let ind = "    ".repeat(self.indent);
        let mut names: Vec<(String, NativeType)> = self.freeable_vars_with_flag.iter().map(|(k, v)| (k.clone(), *v)).collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (n, ty) in names {
            if Some(n.as_str()) == exclude || self.global_freeable_names.contains(&n) {
                continue;
            }
            writeln!(self.out, "{}if ({}_owned) {{ {}((void*){}); }}", ind, n, free_fn_for(ty), n).unwrap();
        }
    }

    fn emit_body(&mut self, body: &Stmt) -> Result<(), String> {
        let mut prescan_scope = TypeScope { vars: self.scope.vars.clone() };
        let mut dummy_ret = None;
        let _ = infer_return_type_stmt(body, &mut prescan_scope, self.sigs, self.fn_name, &mut dummy_ret);
        let mut hoisted_locals: Vec<(String, NativeType)> = prescan_scope.vars.iter()
            .filter(|(name, ty)| is_freeable(**ty) && !self.scope.vars.contains_key(*name))
            .map(|(name, ty)| (name.clone(), *ty))
            .collect();
        hoisted_locals.sort_by(|a, b| a.0.cmp(&b.0));

        let reassigned = assigned_names(body);
        let param_names: Vec<String> = self.scope.vars.keys().cloned().collect();
        for name in param_names {
            let n = format!("_{}", mangle_plain(&name));
            let param_ty = self.scope.vars.get(&name).copied();
            if let Some(ty) = param_ty {
                if is_freeable(ty) && reassigned.contains(&name) && !self.global_freeable_names.contains(&n) {
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}int {}_owned = 0;", ind, n).unwrap();
                    self.freeable_vars_with_flag.insert(n, ty);
                }
            }
        }

        for (name, ty) in hoisted_locals {
            let n = format!("_{}", mangle_plain(&name));
            let ind = "    ".repeat(self.indent);
            let init = match ty {
                NativeType::StrList => "NULL".to_string(),
                _ => "\"\"".to_string(),
            };
            writeln!(self.out, "{}{} {} = {};", ind, ty.c_type(), n, init).unwrap();
            writeln!(self.out, "{}int {}_owned = 0;", ind, n).unwrap();
            self.scope.vars.insert(name, ty);
            self.freeable_vars_with_flag.insert(n, ty);
        }

        let definitely_returns = self.emit_stmt(body)?;
        if !definitely_returns {
            self.emit_owned_locals_cleanup(None);
            if self.ret_type == NativeType::Str {
                let ind = "    ".repeat(self.indent);
                let tmp = self.fresh();
                writeln!(self.out, "{}char* {} = (char*)malloc(1);", ind, tmp).unwrap();
                writeln!(self.out, "{}{}[0] = '\\0';", ind, tmp).unwrap();
                self.line(&format!("return {};", tmp));
            } else {
                self.line(&format!("return {};", self.default_return_literal()));
            }
        }
        Ok(())
    }

    fn emit_stmt(&mut self, stmt: &Stmt) -> Result<bool, String> {
        match &stmt.node {
            StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
                let mut returns = false;
                for s in stmts {
                    if returns { break; }
                    returns = self.emit_stmt(s)?;
                }
                Ok(returns)
            }
            StmtNode::Return(Some(e)) => {
                let t = infer_expr_type(e, &mut self.scope, self.sigs, self.fn_name)?;
                if t != self.ret_type {
                    return Err(format!(
                        "--native --lib: internal error: '{}' return type mismatch ({:?} vs {:?}) \
                         -- this should have been caught during signature inference",
                        self.fn_name, t, self.ret_type
                    ));
                }
                let v = self.emit_expr(e)?;
                let v = if !is_freeable(t) {
                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}{} {} = {};", ind, t.c_type(), tmp, v).unwrap();
                    tmp
                } else {
                    v
                };
                let exclude = if is_freeable(t) { Some(v.as_str()) } else { None };
                self.emit_owned_locals_cleanup(exclude);
                if is_freeable(t) && self.freeable_vars_with_flag.contains_key(&v) && !self.global_freeable_names.contains(&v) {
                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}{} {};", ind, t.c_type(), tmp).unwrap();
                    writeln!(self.out, "{}if ({}_owned) {{", ind, v).unwrap();
                    writeln!(self.out, "{}    {} = {};", ind, tmp, v).unwrap();
                    writeln!(self.out, "{}}} else {{", ind).unwrap();
                    if t == NativeType::StrList {
                        writeln!(self.out, "{}    {} = __dgn_list_clone({});", ind, tmp, v).unwrap();
                    } else {
                        writeln!(self.out, "{}    size_t {}_len = strlen({}) + 1;", ind, tmp, v).unwrap();
                        writeln!(self.out, "{}    char* {}_copy = (char*)malloc({}_len);", ind, tmp, tmp).unwrap();
                        writeln!(self.out, "{}    memcpy({}_copy, {}, {}_len);", ind, tmp, v, tmp).unwrap();
                        writeln!(self.out, "{}    {} = {}_copy;", ind, tmp, tmp).unwrap();
                    }
                    writeln!(self.out, "{}}}", ind).unwrap();
                    self.line(&format!("return {};", tmp));
                } else if is_freeable(t) && !self.owned_temps.contains_key(&v) {
                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    if t == NativeType::StrList {
                        writeln!(self.out, "{}{} {} = __dgn_list_clone({});", ind, t.c_type(), tmp, v).unwrap();
                    } else {
                        writeln!(self.out, "{}size_t {}_len = strlen({}) + 1;", ind, tmp, v).unwrap();
                        writeln!(self.out, "{}char* {} = (char*)malloc({}_len);", ind, tmp, tmp).unwrap();
                        writeln!(self.out, "{}memcpy({}, {}, {}_len);", ind, tmp, v, tmp).unwrap();
                    }
                    self.line(&format!("return {};", tmp));
                } else {
                    self.line(&format!("return {};", v));
                }
                Ok(true)
            }
            StmtNode::Return(None) => unreachable!("caught during infer_return_type_stmt"),
            StmtNode::Expr(e) => {
                let v = self.emit_expr(e)?;
                self.line(&format!("(void){};", v));
                if let Some(ty) = self.owned_temps.remove(&v) {
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}{}((void*){});", ind, free_fn_for(ty), v).unwrap();
                }
                Ok(false)
            }
            StmtNode::ConstDecl(name, e) => {
                let t = infer_expr_type(e, &mut self.scope, self.sigs, self.fn_name)?;
                let v = self.emit_expr(e)?;
                self.scope.vars.insert(name.clone(), t);
                self.line(&format!("{} _{} = {};", t.c_type(), mangle_plain(name), v));
                Ok(false)
            }
            StmtNode::IfChain(branches) => {
                let mut first = true;
                let mut all_return = true;
                let mut has_else = false;
                for (cond_opt, body) in branches {
                    if let Some(cond) = cond_opt {
                        let cv = self.emit_expr(cond)?;
                        if first {
                            self.line(&format!("if ({}) {{", cv));
                            first = false;
                        } else {
                            self.line(&format!("}} else if ({}) {{", cv));
                        }
                    } else {
                        self.line("} else {");
                        has_else = true;
                    }
                    self.indent += 1;
                    let branch_returns = self.emit_stmt(body)?;
                    all_return = all_return && branch_returns;
                    self.indent -= 1;
                }
                if !first { self.line("}"); }
                Ok(has_else && all_return)
            }
            StmtNode::While(cond, body) => {
                let cv = self.emit_expr(cond)?;
                self.line(&format!("while ({}) {{", cv));
                self.indent += 1;
                self.emit_stmt(body)?;
                self.indent -= 1;
                self.line("}");
                Ok(false)
            }
            StmtNode::DoWhile(body, cond) => {
                self.line("do {");
                self.indent += 1;
                self.emit_stmt(body)?;
                self.indent -= 1;
                let cv = self.emit_expr(cond)?;
                self.line(&format!("}} while ({});", cv));
                Ok(false)
            }
            StmtNode::CFor(init, cond, step, body) => {
                if let Some(e) = init {
                    self.emit_stmt(&Stmt { node: StmtNode::Expr(e.clone()), line: 0 })?;
                }
                let cond_s = match cond { Some(e) => self.emit_expr(e)?, None => "1".to_string() };
                let step_s = match step { Some(e) => self.emit_inline_step(e)?, None => String::new() };
                self.line(&format!("for (; {}; {}) {{", cond_s, step_s));
                self.indent += 1;
                self.emit_stmt(body)?;
                self.indent -= 1;
                self.line("}");
                Ok(false)
            }
            StmtNode::For(var_name, value_var, iter_expr, body) => {
                if value_var.is_some() {
                    return Err(format!(
                        "--native --lib: in '{}', 'for index, value in ...' isn't supported yet -- \
                         use 'for value in ...' and track the index yourself with a counter variable",
                        self.fn_name
                    ));
                }
                let it = infer_expr_type(iter_expr, &mut self.scope, self.sigs, self.fn_name)?;
                if it != NativeType::StrList {
                    return Err(format!(
                        "--native --lib: in '{}', 'for x in ...' only supports iterating a strlist right now, not {:?}",
                        self.fn_name, it
                    ));
                }
                let lv = self.emit_expr(iter_expr)?;
                let lv_is_owned_temp = self.owned_temps.contains_key(&lv);
                let ind = "    ".repeat(self.indent);
                let len_var = self.fresh();
                let idx_var = self.fresh();
                writeln!(self.out, "{}int64_t {} = __dgn_list_len({});", ind, len_var, lv).unwrap();
                writeln!(self.out, "{}int64_t {} = 0;", ind, idx_var).unwrap();
                writeln!(self.out, "{}while ({} < {}) {{", ind, idx_var, len_var).unwrap();
                self.indent += 1;
                let ind2 = "    ".repeat(self.indent);
                let vn = format!("_{}", mangle_plain(var_name));
                if self.freeable_vars_with_flag.contains_key(&vn) {
                    writeln!(self.out, "{}if ({}_owned) {{ free((void*){}); }}", ind2, vn, vn).unwrap();
                }
                let already_declared = self.scope.vars.contains_key(var_name);
                if !already_declared {
                    self.scope.vars.insert(var_name.clone(), NativeType::Str);
                    writeln!(self.out, "{}const char* {} = __dgn_list_get({}, {});", ind2, vn, lv, idx_var).unwrap();
                } else {
                    writeln!(self.out, "{}{} = __dgn_list_get({}, {});", ind2, vn, lv, idx_var).unwrap();
                }
                if !self.freeable_vars_with_flag.contains_key(&vn) {
                    writeln!(self.out, "{}int {}_owned = 0;", ind2, vn).unwrap();
                    self.freeable_vars_with_flag.insert(vn.clone(), NativeType::Str);
                } else {
                    writeln!(self.out, "{}{}_owned = 0;", ind2, vn).unwrap();
                }
                self.emit_stmt(body)?;
                writeln!(self.out, "{}{} = {} + 1;", ind2, idx_var, idx_var).unwrap();
                self.indent -= 1;
                self.line("}");
                if lv_is_owned_temp {
                    self.owned_temps.remove(&lv);
                    self.line(&format!("__dgn_list_free((void*){});", lv));
                }
                Ok(false)
            }
            StmtNode::Break => { self.line("break;"); Ok(false) }
            StmtNode::Continue => { self.line("continue;"); Ok(false) }
            _ => Err(format!(
                "--native --lib: '{}' uses a statement form not supported in native-lib mode",
                self.fn_name
            )),
        }
    }

    fn emit_inline_step(&mut self, e: &Expr) -> Result<String, String> {
        match &e.node {
            ExprNode::Assign(target, val) => {
                let name = match &target.node {
                    ExprNode::Var(n) => n.clone(),
                    _ => return Err(format!("--native --lib: in '{}', a for-loop step's assignment target must be a plain variable", self.fn_name)),
                };
                if !self.scope.vars.contains_key(&name) {
                    return Err(format!(
                        "--native --lib: in '{}', for-loop step assigns to '{}', which isn't declared yet -- \
                         declare it before the loop (a step clause can't introduce a new variable)",
                        self.fn_name, name
                    ));
                }
                let vt = self.scope.vars[&name];
                if vt == NativeType::Str {
                    return Err(format!(
                        "--native --lib: in '{}', a for-loop step can't build a string (that needs a helper \
                         statement that can't run inline in the loop header) -- move it into the loop body instead",
                        self.fn_name
                    ));
                }
                let vv = self.emit_inline_step(val)?;
                Ok(format!("_{} = {}", mangle_plain(&name), vv))
            }
            ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
                let iname = match &inner.node {
                    ExprNode::Var(n) => n.clone(),
                    _ => return Err(format!("--native --lib: in '{}', '{}' in a for-loop step needs a plain variable", self.fn_name, op)),
                };
                if self.scope.vars.get(&iname) != Some(&NativeType::Int) {
                    return Err(format!("--native --lib: in '{}', '{}' in a for-loop step requires an already-declared int variable", self.fn_name, op));
                }
                let n = format!("_{}", mangle_plain(&iname));
                match op.as_str() {
                    "++" => Ok(format!("{}++", n)),
                    "--" => Ok(format!("{}--", n)),
                    _ => Err(format!("--native --lib: in '{}', unsupported operator '{}' in for-loop step", self.fn_name, op)),
                }
            }
            ExprNode::Number(n) => Ok(if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                format!("{}LL", *n as i64)
            } else {
                format!("{:.17}", n)
            }),
            ExprNode::Bool(b) => Ok(if *b { "1".into() } else { "0".into() }),
            ExprNode::Var(name) => {
                if !self.scope.vars.contains_key(name) {
                    return Err(format!("--native --lib: in '{}', reference to undeclared variable '{}' in for-loop step", self.fn_name, name));
                }
                Ok(format!("_{}", mangle_plain(name)))
            }
            ExprNode::Binary(op, l, r) => {
                let mut scope_peek = TypeScope { vars: self.scope.vars.clone() };
                let lt = infer_expr_type(l, &mut scope_peek, self.sigs, self.fn_name)?;
                let rt = infer_expr_type(r, &mut scope_peek, self.sigs, self.fn_name)?;
                if lt == NativeType::Str || rt == NativeType::Str {
                    return Err(format!("--native --lib: in '{}', a for-loop step can't involve strings inline", self.fn_name));
                }
                let lv = self.emit_inline_step(l)?;
                let rv = self.emit_inline_step(r)?;
                Ok(format!("({} {} {})", lv, op, rv))
            }
            ExprNode::Unary(op, inner) => {
                let v = self.emit_inline_step(inner)?;
                match op.as_str() {
                    "-" => Ok(format!("(-{})", v)),
                    "!" | "not" => Ok(format!("(!{})", v)),
                    _ => Err(format!("--native --lib: in '{}', unsupported unary operator '{}' in for-loop step", self.fn_name, op)),
                }
            }
            _ => Err(format!(
                "--native --lib: in '{}', this expression form isn't supported in a for-loop step \
                 (supported: number/bool literals, variables, arithmetic, ++/--, simple assignment)",
                self.fn_name
            )),
        }
    }

    fn emit_expr(&mut self, expr: &Expr) -> Result<String, String> {
        match &expr.node {
            ExprNode::Number(n) => Ok(if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                format!("{}LL", *n as i64)
            } else {
                format!("{:.17}", n)
            }),
            ExprNode::String(s) => Ok(format!("\"{}\"", escape_c_string(s))),
            ExprNode::Bool(b) => Ok(if *b { "1".into() } else { "0".into() }),
            ExprNode::Var(name) => Ok(format!("_{}", mangle_plain(name))),
            ExprNode::List(items) => {

                let ind = "    ".repeat(self.indent);
                let tmp = self.fresh();
                writeln!(self.out, "{}DgnListStr* {} = __dgn_list_new();", ind, tmp).unwrap();
                for item in items {
                    let it = infer_expr_type(item, &mut self.scope, self.sigs, self.fn_name)?;
                    let iv = self.emit_expr(item)?;
                    if self.owned_temps.remove(&iv).is_some() {

                        writeln!(self.out, "{}__dgn_list_push({}, (char*){});", ind, tmp, iv).unwrap();
                    } else {
                        let cs = self.to_cstr_expr(it, &iv)?;
                        if self.owned_temps.remove(&cs).is_some() {
                            writeln!(self.out, "{}__dgn_list_push({}, (char*){});", ind, tmp, cs).unwrap();
                        } else {
                            let copy = self.fresh();
                            writeln!(self.out, "{}size_t {}_len = strlen({}) + 1;", ind, copy, cs).unwrap();
                            writeln!(self.out, "{}char* {} = (char*)malloc({}_len);", ind, copy, copy).unwrap();
                            writeln!(self.out, "{}memcpy({}, {}, {}_len);", ind, copy, cs, copy).unwrap();
                            writeln!(self.out, "{}__dgn_list_push({}, {});", ind, tmp, copy).unwrap();
                        }
                    }
                }
                self.owned_temps.insert(tmp.clone(), NativeType::StrList);
                Ok(tmp)
            }
            ExprNode::Index(list_expr, idx_expr) => {

                let lv = self.emit_expr(list_expr)?;
                let iv = self.emit_expr(idx_expr)?;
                Ok(format!("__dgn_list_get({}, {})", lv, iv))
            }
            ExprNode::Binary(op, l, r) => {
                let lt = infer_expr_type(l, &mut self.scope, self.sigs, self.fn_name)?;
                let rt = infer_expr_type(r, &mut self.scope, self.sigs, self.fn_name)?;
                let lv = self.emit_expr(l)?;
                let rv = self.emit_expr(r)?;
                if op == "+" && (lt == NativeType::Str || rt == NativeType::Str) {

                    let ls = self.to_cstr_expr(lt, &lv)?;
                    let rs = self.to_cstr_expr(rt, &rv)?;
                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}size_t {}_len = strlen({}) + strlen({}) + 1;", ind, tmp, ls, rs).unwrap();
                    writeln!(self.out, "{}char* {} = (char*)malloc({}_len);", ind, tmp, tmp).unwrap();
                    writeln!(self.out, "{}snprintf({}, {}_len, \"%s%s\", {}, {});", ind, tmp, tmp, ls, rs).unwrap();

                    if self.owned_temps.remove(&ls).is_some() {
                        writeln!(self.out, "{}free((void*){});", ind, ls).unwrap();
                    }
                    if self.owned_temps.remove(&rs).is_some() {
                        writeln!(self.out, "{}free((void*){});", ind, rs).unwrap();
                    }
                    self.owned_temps.insert(tmp.clone(), NativeType::Str);
                    Ok(tmp)
                } else if op == "+" && lt == NativeType::StrList && rt == NativeType::StrList {

                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}DgnListStr* {} = __dgn_list_concat({}, {});", ind, tmp, lv, rv).unwrap();
                    if self.owned_temps.remove(&lv).is_some() {
                        writeln!(self.out, "{}__dgn_list_free((void*){});", ind, lv).unwrap();
                    }
                    if self.owned_temps.remove(&rv).is_some() {
                        writeln!(self.out, "{}__dgn_list_free((void*){});", ind, rv).unwrap();
                    }
                    self.owned_temps.insert(tmp.clone(), NativeType::StrList);
                    Ok(tmp)
                } else if (op == "==" || op == "!=") && lt == NativeType::Str && rt == NativeType::Str {

                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    let c_op = if op == "==" { "==" } else { "!=" };
                    writeln!(self.out, "{}int {} = (strcmp({}, {}) {} 0);", ind, tmp, lv, rv, c_op).unwrap();
                    if self.owned_temps.remove(&lv).is_some() {
                        writeln!(self.out, "{}free((void*){});", ind, lv).unwrap();
                    }
                    if self.owned_temps.remove(&rv).is_some() {
                        writeln!(self.out, "{}free((void*){});", ind, rv).unwrap();
                    }
                    Ok(tmp)
                } else {
                    Ok(format!("({} {} {})", lv, op, rv))
                }
            }
            ExprNode::Unary(op, inner) => {
                let v = self.emit_expr(inner)?;
                match op.as_str() {
                    "!" | "not" => Ok(format!("(!{})", v)),
                    "-" => Ok(format!("(-{})", v)),
                    _ => Err(format!("--native --lib: in '{}', unsupported unary operator '{}'", self.fn_name, op)),
                }
            }
            ExprNode::Assign(target, val) => {
                let name = match &target.node {
                    ExprNode::Var(n) => n.clone(),
                    _ => return Err(format!("--native --lib: in '{}', assignment target must be a plain variable", self.fn_name)),
                };
                let already_declared = self.scope.vars.contains_key(&name);

                let mut scope_peek = TypeScope { vars: self.scope.vars.clone() };
                let et = infer_expr_type(val, &mut scope_peek, self.sigs, self.fn_name)?;
                let vv = self.emit_expr(val)?;
                let n = format!("_{}", mangle_plain(&name));
                let ind = "    ".repeat(self.indent);

                let rhs_owner_var: Option<String> =
                    if vv != n && self.freeable_vars_with_flag.contains_key(&vv) && !self.global_freeable_names.contains(&vv) {
                        Some(vv.clone())
                    } else {
                        None
                    };
                let rhs_is_fresh_alloc = self.owned_temps.contains_key(&vv);

                let target_is_global = self.global_freeable_names.contains(&n);
                let source_is_global = self.global_freeable_names.contains(&vv);
                let (vv, rhs_is_fresh_alloc) = if is_freeable(et) && (target_is_global || source_is_global) && !rhs_is_fresh_alloc && rhs_owner_var.is_none() {
                    let copy = self.fresh();
                    if et == NativeType::StrList {
                        writeln!(self.out, "{}DgnListStr* {} = __dgn_list_clone({});", ind, copy, vv).unwrap();
                    } else {
                        writeln!(self.out, "{}size_t {}_len = strlen({}) + 1;", ind, copy, vv).unwrap();
                        writeln!(self.out, "{}char* {} = (char*)malloc({}_len);", ind, copy, copy).unwrap();
                        writeln!(self.out, "{}memcpy({}, {}, {}_len);", ind, copy, vv, copy).unwrap();
                    }
                    (copy, true)
                } else {
                    (vv, rhs_is_fresh_alloc)
                };
                if is_freeable(et) && already_declared && self.freeable_vars_with_flag.contains_key(&n) {

                    writeln!(self.out, "{}if ({}_owned) {{ {}((void*){}); }}", ind, n, free_fn_for(et), n).unwrap();
                }
                self.owned_temps.remove(&vv);
                if already_declared {
                    self.line(&format!("{} = {};", n, vv));
                } else {

                    self.scope.vars.insert(name, et);
                    self.line(&format!("{} {} = {};", et.c_type(), n, vv));
                }
                if is_freeable(et) {
                    let owned_expr = if rhs_is_fresh_alloc {
                        "1".to_string()
                    } else if let Some(ref rv) = rhs_owner_var {
                        format!("{}_owned", rv)
                    } else {
                        "0".to_string()
                    };
                    if !self.freeable_vars_with_flag.contains_key(&n) {
                        let ind2 = "    ".repeat(self.indent);
                        writeln!(self.out, "{}int {}_owned = {};", ind2, n, owned_expr).unwrap();
                        self.freeable_vars_with_flag.insert(n.clone(), et);
                    } else {
                        self.line(&format!("{}_owned = {};", n, owned_expr));
                    }
                    if let Some(rv) = rhs_owner_var {

                        self.line(&format!("{}_owned = 0;", rv));
                    }
                }
                Ok(n)
            }
            ExprNode::PostOp(op, inner) => {
                let v = self.emit_expr(inner)?;
                match op.as_str() {
                    "++" => Ok(format!("({}++)", v)),
                    "--" => Ok(format!("({}--)", v)),
                    _ => Err(format!("--native --lib: in '{}', unsupported operator '{}'", self.fn_name, op)),
                }
            }
            ExprNode::PreOp(op, inner) => {
                let v = self.emit_expr(inner)?;
                match op.as_str() {
                    "++" => Ok(format!("(++{})", v)),
                    "--" => Ok(format!("(--{})", v)),
                    _ => Err(format!("--native --lib: in '{}', unsupported operator '{}'", self.fn_name, op)),
                }
            }
            ExprNode::Ternary(cond, t, e2) => {
                let cv = self.emit_expr(cond)?;
                let tv = self.emit_expr(t)?;
                let ev = self.emit_expr(e2)?;
                Ok(format!("({} ? {} : {})", cv, tv, ev))
            }
            ExprNode::Call(callee, args) => {
                if let ExprNode::GetAttr(receiver, method) = &callee.node {
                    let recv_name = match &receiver.node {
                        ExprNode::Var(n) => n.clone(),
                        _ => return Err(format!(
                            "--native --lib: in '{}', '.{}(...)' is only supported on a plain variable",
                            self.fn_name, method
                        )),
                    };
                    let recv_c = format!("_{}", mangle_plain(&recv_name));
                    let ind = "    ".repeat(self.indent);
                    return match method.as_str() {
                        "push" => {
                            let item_val = self.emit_expr(&args[0])?;
                            writeln!(self.out, "{}if ({} == NULL) {{ {} = __dgn_list_new(); }}", ind, recv_c, recv_c).unwrap();
                            let dup = self.fresh();
                            writeln!(self.out, "{}char* {} = __dgn_lib_strdup({});", ind, dup, item_val).unwrap();
                            if let Some(ty) = self.owned_temps.remove(&item_val) {
                                writeln!(self.out, "{}{}((void*){});", ind, free_fn_for(ty), item_val).unwrap();
                            }
                            writeln!(self.out, "{}__dgn_list_push({}, {});", ind, recv_c, dup).unwrap();
                            Ok("0LL".to_string())
                        }
                        "pop" => {
                            let tmp = self.fresh();
                            writeln!(self.out, "{}char* {} = __dgn_list_pop({});", ind, tmp, recv_c).unwrap();
                            self.owned_temps.insert(tmp.clone(), NativeType::Str);
                            Ok(tmp)
                        }
                        other => Err(format!("--native --lib: in '{}', unsupported list method '.{}()'", self.fn_name, other)),
                    };
                }
                let name = match &callee.node {
                    ExprNode::Var(n) => n.clone(),
                    _ => return Err(format!("--native --lib: in '{}', only calling a plain function name is supported", self.fn_name)),
                };
                let arg_types: Result<Vec<NativeType>, String> = args.iter()
                    .map(|a| infer_expr_type(a, &mut self.scope, self.sigs, self.fn_name))
                    .collect();
                let arg_types = arg_types?;
                let arg_vals: Result<Vec<String>, String> = args.iter().map(|a| self.emit_expr(a)).collect();
                let arg_vals = arg_vals?;
                let (call_text, ret_ty) = if let Some(sig) = self.sigs.get(&name) {
                    (format!("{}({})", name, arg_vals.join(", ")), sig.ret)
                } else if let Some((ret, c_fn)) = builtin_call(&name, &arg_types) {
                    (format!("{}({})", c_fn, arg_vals.join(", ")), ret)
                } else {
                    return Err(format!("--native --lib: in '{}', call to unknown function '{}'", self.fn_name, name));
                };

                let owned_args: Vec<(String, NativeType)> = arg_vals.iter()
                    .filter_map(|a| self.owned_temps.get(a).map(|ty| (a.clone(), *ty)))
                    .collect();
                if is_freeable(ret_ty) {
                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}{} {} = {};", ind, ret_ty.c_type(), tmp, call_text).unwrap();
                    for (a, ty) in &owned_args {
                        self.owned_temps.remove(a);
                        writeln!(self.out, "{}{}((void*){});", ind, free_fn_for(*ty), a).unwrap();
                    }
                    self.owned_temps.insert(tmp.clone(), ret_ty);
                    Ok(tmp)
                } else if !owned_args.is_empty() {
                    let tmp = self.fresh();
                    let ind = "    ".repeat(self.indent);
                    writeln!(self.out, "{}{} {} = {};", ind, ret_ty.c_type(), tmp, call_text).unwrap();
                    for (a, ty) in &owned_args {
                        self.owned_temps.remove(a);
                        writeln!(self.out, "{}{}((void*){});", ind, free_fn_for(*ty), a).unwrap();
                    }
                    Ok(tmp)
                } else {
                    Ok(call_text)
                }
            }
            _ => Err(format!(
                "--native --lib: '{}' uses an expression form not supported in native-lib mode",
                self.fn_name
            )),
        }
    }

    fn to_cstr_expr(&mut self, t: NativeType, val: &str) -> Result<String, String> {
        match t {
            NativeType::Str => Ok(val.to_string()),
            NativeType::Int => {
                let tmp = self.fresh();
                let ind = "    ".repeat(self.indent);
                writeln!(self.out, "{}char* {} = (char*)malloc(32);", ind, tmp).unwrap();
                writeln!(self.out, "{}snprintf({}, 32, \"%lld\", (long long)({}));", ind, tmp, val).unwrap();
                self.owned_temps.insert(tmp.clone(), NativeType::Str);
                Ok(tmp)
            }
            NativeType::Float => {
                let tmp = self.fresh();
                let ind = "    ".repeat(self.indent);
                writeln!(self.out, "{}char* {} = (char*)malloc(64);", ind, tmp).unwrap();
                writeln!(self.out, "{}snprintf({}, 64, \"%g\", (double)({}));", ind, tmp, val).unwrap();
                self.owned_temps.insert(tmp.clone(), NativeType::Str);
                Ok(tmp)
            }
            NativeType::Bool => Ok(format!("(({}) ? \"true\" : \"false\")", val)),
            NativeType::StrList => Err(format!(
                "--native --lib: in '{}', a strlist can't be used directly in string concatenation \
                 -- index into it (list[i]) or pass it to a function that formats it explicitly",
                self.fn_name
            )),
        }
    }
}

pub(crate) fn flatten_imports_for_lib(
    ast: &Stmt,
    base_dir: &std::path::PathBuf,
    visited: &mut std::collections::HashSet<std::path::PathBuf>,
) -> Result<Stmt, String> {
    let stmts: Vec<Stmt> = match &ast.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s.clone(),
        _ => vec![ast.clone()],
    };

    let mut merged: Vec<Stmt> = Vec::new();
    for s in &stmts {
        if let StmtNode::Import(module, alias) = &s.node {
            if alias.is_some() {
                return Err(format!(
                    "--native --lib: 'import \"{}\" as ...' isn't supported -- --native --lib has no \
                     qualified-call syntax (no module.function()), so an aliased import's functions \
                     wouldn't be reachable under that alias anyway. Use a bare 'import \"{}\";' instead; \
                     its functions/globals merge directly into this file's own namespace.",
                    module, module
                ));
            }
            if super::is_builtin_module(module) {
                return Err(format!(
                    "--native --lib: can't import built-in module '{}' -- built-in modules (math, json, \
                     etc.) are implemented in the VM/interpreter, not as portable C, so they have no \
                     --native equivalent. Only local .dgn file imports are supported here.",
                    module
                ));
            }

            let candidate1 = base_dir.join(format!("{}.dgn", module));
            let candidate2 = base_dir.join("dgn_modules").join(format!("{}.dgn", module));
            let module_path = if candidate1.exists() { candidate1.clone() } else { candidate2.clone() };
            let canon = module_path.canonicalize().unwrap_or_else(|_| module_path.clone());
            if visited.contains(&canon) {
                continue;
            }

            let module_ast = load_user_module(module, base_dir).ok_or_else(|| format!(
                "--native --lib: couldn't find module '{}' -- looked for {} and {} relative to the \
                 entry file's directory ({})",
                module, candidate1.display(), candidate2.display(), base_dir.display()
            ))?;
            visited.insert(canon);

            let module_base_dir = module_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| base_dir.clone());
            let flattened_module = flatten_imports_for_lib(&module_ast, &module_base_dir, visited)?;
            let module_stmts: Vec<Stmt> = match flattened_module.node {
                StmtNode::Block(s) | StmtNode::Multi(s) => s,
                other => vec![Stmt { node: other, line: flattened_module.line }],
            };
            merged.extend(module_stmts);
        } else {
            merged.push(s.clone());
        }
    }

    Ok(Stmt { node: StmtNode::Block(merged), line: ast.line })
}

pub(crate) fn transpile_native_lib(ast: &Stmt) -> Result<String, String> {
    let globals = collect_lib_globals(ast)?;
    let fns = collect_lib_fns(ast);

    if fns.is_empty() {
        return Err("--native --lib: no top-level functions found to export.".into());
    }

    let mut sigs: std::collections::HashMap<String, LibFnSig> = std::collections::HashMap::new();
    let mut bodies: std::collections::HashMap<String, Stmt> = std::collections::HashMap::new();
    for (name, params, body) in &fns {
        if sigs.contains_key(name) {
            return Err(format!("--native --lib: duplicate top-level function name '{}'", name));
        }
        let mut resolved_params = Vec::new();
        for (raw_name, raw_type) in params {
            if raw_name.starts_with("...") {
                return Err(format!(
                    "--native --lib: rest parameters ('{}' in function '{}') aren't supported -- \
                     a C function needs a fixed parameter list",
                    raw_name, name
                ));
            }
            let (pname, pty) = parse_param(raw_name, raw_type);
            let pty = pty.ok_or_else(|| format!(
                "--native --lib: parameter '{}' of function '{}' has no type annotation -- \
                 every exported function's parameters need an explicit ': type' (int, float, str, or bool), \
                 since guessing wrong here would silently corrupt the caller's ABI",
                pname, name
            ))?;
            resolved_params.push((pname, pty));
        }
        sigs.insert(name.clone(), LibFnSig { params: resolved_params, ret: NativeType::Int });
        bodies.insert(name.clone(), body.clone());
    }

    let mut resolved_ret: std::collections::HashMap<String, Option<NativeType>> =
        sigs.keys().map(|k| (k.clone(), None)).collect();
    let mut progressed = true;
    let mut last_errors: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    while progressed {
        progressed = false;
        let names: Vec<String> = sigs.keys().cloned().collect();
        for name in &names {

            let mut scope = TypeScope { vars: std::collections::HashMap::new() };
            for (pname, pty) in &sigs[name].params {
                scope.vars.insert(pname.clone(), *pty);
            }
            for (gname, gty, _) in &globals {
                scope.vars.entry(gname.clone()).or_insert(*gty);
            }
            match infer_return_type(&bodies[name], &mut scope, &sigs, name) {
                Ok(Some(t)) => {
                    if resolved_ret[name] != Some(t) {
                        resolved_ret.insert(name.clone(), Some(t));
                        sigs.get_mut(name).unwrap().ret = t;
                        progressed = true;
                    }
                    last_errors.remove(name);
                }
                Ok(None) => {
                    return Err(format!("--native --lib: function '{}' has no 'return' statement -- every exported function must return a value", name));
                }
                Err(e) => {
                    last_errors.insert(name.clone(), e);
                }
            }
        }
    }
    let unresolved: Vec<&String> = resolved_ret.iter().filter(|(_, v)| v.is_none()).map(|(k, _)| k).collect();
    if let Some(name) = unresolved.first() {
        let msg = last_errors.get(*name).cloned().unwrap_or_else(|| {
            format!("--native --lib: could not resolve a return type for '{}' (likely a call cycle with an unresolvable function)", name)
        });
        return Err(msg);
    }

    let mut out = String::new();
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <string.h>\n");
    out.push_str("#include <stdlib.h>\n");
    out.push_str("#include <stdio.h>\n");
    out.push_str("#include <math.h>\n\n");
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    out.push_str("#ifdef _WIN32\n");
    out.push_str("#  define DGN_EXPORT __declspec(dllexport)\n");
    out.push_str("#else\n");
    out.push_str("#  define DGN_EXPORT __attribute__((visibility(\"default\")))\n");
    out.push_str("#endif\n\n");
    out.push_str(BUILTIN_HELPERS);
    out.push('\n');

    for (name, ty, init) in &globals {
        out.push_str(&format!("static {} _g_{} = {};\n", ty.c_type(), mangle_plain(name), init));
        if is_freeable(*ty) {

            out.push_str(&format!("static int _g_{}_owned = 0;\n", mangle_plain(name)));
        }
    }
    if !globals.is_empty() { out.push('\n'); }

    let mut sorted_fn_names: Vec<&String> = sigs.keys().collect();
    sorted_fn_names.sort();
    for name in &sorted_fn_names {
        let sig = &sigs[*name];
        let params_str = if sig.params.is_empty() {
            "void".to_string()
        } else {
            sig.params.iter().map(|(p, t)| format!("{} _{}", t.c_type(), mangle_plain(p))).collect::<Vec<_>>().join(", ")
        };
        out.push_str(&format!("DGN_EXPORT {} {}({});\n", sig.ret.c_type(), name, params_str));
    }
    out.push('\n');

    for name in &sorted_fn_names {
        let sig = &sigs[*name];
        let body = &bodies[*name];
        let params_str = if sig.params.is_empty() {
            "void".to_string()
        } else {
            sig.params.iter().map(|(p, t)| format!("{} _{}", t.c_type(), mangle_plain(p))).collect::<Vec<_>>().join(", ")
        };

        let mut scope = TypeScope { vars: std::collections::HashMap::new() };
        let mut param_names_in_fn = std::collections::HashSet::new();
        for (pname, pty) in &sig.params {
            scope.vars.insert(pname.clone(), *pty);
            param_names_in_fn.insert(pname.clone());
        }
        let shadowed_globals: Vec<&(String, NativeType, String)> =
            globals.iter().filter(|(gname, _, _)| !param_names_in_fn.contains(gname)).collect();
        for (gname, gty, _) in &shadowed_globals {
            scope.vars.insert(gname.clone(), *gty);
        }

        out.push_str(&format!("DGN_EXPORT {} {}({}) {{\n", sig.ret.c_type(), name, params_str));
        let mut global_freeable_names_in_fn: std::collections::HashMap<String, NativeType> = std::collections::HashMap::new();
        for (gname, gty, _) in &shadowed_globals {
            out.push_str(&format!(
                "    {}* const __{}_ptr = &_g_{};\n#define _{} (*__{}_ptr)\n",
                gty.c_type(), mangle_plain(gname), mangle_plain(gname), mangle_plain(gname), mangle_plain(gname)
            ));
            if is_freeable(*gty) {
                let mn = mangle_plain(gname);
                out.push_str(&format!(
                    "    int* const __{}_owned_ptr = &_g_{}_owned;\n#define _{}_owned (*__{}_owned_ptr)\n",
                    mn, mn, mn, mn
                ));
                global_freeable_names_in_fn.insert(format!("_{}", mn), *gty);
            }
        }

        let mut nt = NativeTranspiler {
            out: String::new(),
            indent: 1,
            tmp_ctr: 0,
            scope,
            sigs: &sigs,
            fn_name: name,
            ret_type: sig.ret,
            owned_temps: std::collections::HashMap::new(),
            freeable_vars_with_flag: global_freeable_names_in_fn.clone(),
            global_freeable_names: global_freeable_names_in_fn.keys().cloned().collect(),
        };
        nt.emit_body(body)?;
        out.push_str(&nt.out);

        for (gname, _, _) in &shadowed_globals {
            out.push_str(&format!("#undef _{}\n", mangle_plain(gname)));
        }
        out.push_str("}\n\n");
    }

    out.push_str("#ifdef __cplusplus\n}\n#endif\n");

    Ok(out)
}

pub(crate) fn generate_native_lib_header(ast: &Stmt, lib_name: &str) -> Result<String, String> {
    let globals = collect_lib_globals(ast)?;
    let fns = collect_lib_fns(ast);
    let mut sigs: std::collections::HashMap<String, LibFnSig> = std::collections::HashMap::new();
    let mut bodies: std::collections::HashMap<String, Stmt> = std::collections::HashMap::new();
    for (name, params, _body) in &fns {
        let mut resolved_params = Vec::new();
        for (raw_name, raw_type) in params {
            let (pname, pty) = parse_param(raw_name, raw_type);
            let pty = pty.ok_or_else(|| format!("--native --lib: parameter '{}' of function '{}' has no type annotation", pname, name))?;
            resolved_params.push((pname, pty));
        }
        sigs.insert(name.clone(), LibFnSig { params: resolved_params, ret: NativeType::Int });
    }
    for (name, _params, body) in &fns {
        bodies.insert(name.clone(), body.clone());
    }
    let mut resolved_ret: std::collections::HashMap<String, Option<NativeType>> = sigs.keys().map(|k| (k.clone(), None)).collect();
    let mut progressed = true;
    while progressed {
        progressed = false;
        let names: Vec<String> = sigs.keys().cloned().collect();
        for name in &names {
            let mut scope = TypeScope { vars: std::collections::HashMap::new() };
            for (pname, pty) in &sigs[name].params { scope.vars.insert(pname.clone(), *pty); }
            for (gname, gty, _) in &globals { scope.vars.entry(gname.clone()).or_insert(*gty); }
            if let Ok(Some(t)) = infer_return_type(&bodies[name], &mut scope, &sigs, name) {
                if resolved_ret[name] != Some(t) {
                    resolved_ret.insert(name.clone(), Some(t));
                    sigs.get_mut(name).unwrap().ret = t;
                    progressed = true;
                }
            }
        }
    }

    let guard = format!("DGN_{}_H", mangle_plain(lib_name).to_uppercase());
    let mut out = String::new();
    out.push_str(&format!("#ifndef {}\n#define {}\n\n", guard, guard));
    out.push_str("#include <stdint.h>\n\n");
    out.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    out.push_str("/* Free a string returned by any function below. Required -- this library\n");
    out.push_str("   may be linked against a different CRT than your program, so you must not\n");
    out.push_str("   call your own free() on a pointer it returned. */\n");
    out.push_str("void dgn_free_string(char* s);\n\n");

    let uses_strlist = sigs.values().any(|s| s.ret == NativeType::StrList || s.params.iter().any(|(_, t)| *t == NativeType::StrList));
    if uses_strlist {
        out.push_str("/* Opaque -- use dgn_strlist_len/dgn_strlist_get to read one, never access\n");
        out.push_str("   fields directly (its layout is a private implementation detail). */\n");
        out.push_str("typedef struct DgnListStr DgnListStr;\n\n");
        out.push_str("/* Free a strlist returned by any function below (same free()-vs-different-CRT\n");
        out.push_str("   reasoning as dgn_free_string -- never call your own free() on one). */\n");
        out.push_str("void dgn_free_strlist(DgnListStr* l);\n");
        out.push_str("int64_t dgn_strlist_len(DgnListStr* l);\n");
        out.push_str("/* Returns a BORROWED pointer into the list -- do not free it separately;\n");
        out.push_str("   it's only valid as long as the list itself hasn't been freed. */\n");
        out.push_str("const char* dgn_strlist_get(DgnListStr* l, int64_t idx);\n\n");
    }

    let mut sorted: Vec<&String> = sigs.keys().collect();
    sorted.sort();
    for name in &sorted {
        let sig = &sigs[*name];
        let params_str = if sig.params.is_empty() {
            "void".to_string()
        } else {
            sig.params.iter().map(|(p, t)| format!("{} {}", t.c_type(), mangle_plain(p))).collect::<Vec<_>>().join(", ")
        };
        out.push_str(&format!("{} {}({});\n", sig.ret.c_type(), name, params_str));
    }

    out.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n");
    out.push_str(&format!("#endif /* {} */\n", guard));
    Ok(out)
}

fn jni_type(t: NativeType) -> Option<&'static str> {
    match t {
        NativeType::Int => Some("jlong"),
        NativeType::Float => Some("jdouble"),
        NativeType::Str => Some("jstring"),
        NativeType::Bool => Some("jboolean"),
        NativeType::StrList => None,
    }
}

fn jni_mangle_component(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '_' => out.push_str("_1"),
            ';' => out.push_str("_2"),
            '[' => out.push_str("_3"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            c => out.push_str(&format!("_0{:04x}", c as u32)),
        }
    }
    out
}

fn jni_symbol_name(package: &str, class: &str, method: &str) -> String {
    let pkg_mangled: Vec<String> = package.split('.').map(jni_mangle_component).collect();
    format!(
        "Java_{}_{}_{}",
        pkg_mangled.join("_"),
        jni_mangle_component(class),
        jni_mangle_component(method)
    )
}

pub(crate) fn generate_jni_wrappers(ast: &Stmt, package: &str, class: &str) -> Result<String, String> {
    let globals = collect_lib_globals(ast)?;
    let fns = collect_lib_fns(ast);
    let mut sigs: std::collections::HashMap<String, LibFnSig> = std::collections::HashMap::new();
    let mut bodies: std::collections::HashMap<String, Stmt> = std::collections::HashMap::new();
    for (name, params, _body) in &fns {
        let mut resolved_params = Vec::new();
        for (raw_name, raw_type) in params {
            let (pname, pty) = parse_param(raw_name, raw_type);
            let pty = pty.ok_or_else(|| format!("--jni: parameter '{}' of function '{}' has no type annotation", pname, name))?;
            resolved_params.push((pname, pty));
        }
        sigs.insert(name.clone(), LibFnSig { params: resolved_params, ret: NativeType::Int });
    }
    for (name, _params, body) in &fns {
        bodies.insert(name.clone(), body.clone());
    }
    let mut resolved_ret: std::collections::HashMap<String, Option<NativeType>> =
        sigs.keys().map(|k| (k.clone(), None)).collect();
    let mut progressed = true;
    while progressed {
        progressed = false;
        let names: Vec<String> = sigs.keys().cloned().collect();
        for name in &names {
            let mut scope = TypeScope { vars: std::collections::HashMap::new() };
            for (pname, pty) in &sigs[name].params { scope.vars.insert(pname.clone(), *pty); }
            for (gname, gty, _) in &globals { scope.vars.entry(gname.clone()).or_insert(*gty); }
            if let Ok(Some(t)) = infer_return_type(&bodies[name], &mut scope, &sigs, name) {
                if resolved_ret[name] != Some(t) {
                    resolved_ret.insert(name.clone(), Some(t));
                    sigs.get_mut(name).unwrap().ret = t;
                    progressed = true;
                }
            }
        }
    }
    if let Some(name) = resolved_ret.iter().find(|(_, v)| v.is_none()).map(|(k, _)| k) {
        return Err(format!("--jni: could not resolve a return type for '{}'", name));
    }

    let mut out = String::new();
    let mut sorted: Vec<&String> = sigs.keys().collect();
    sorted.sort();
    for name in &sorted {
        let sig = &sigs[*name];

        let all_types_jni_ok = sig.params.iter().all(|(_, t)| jni_type(*t).is_some()) && jni_type(sig.ret).is_some();
        if !all_types_jni_ok {
            continue;
        }

        let jni_name = jni_symbol_name(package, class, name);

        let jni_params: Vec<String> = sig.params.iter()
            .map(|(p, t)| format!("{} j_{}", jni_type(*t).unwrap(), mangle_plain(p)))
            .collect();
        let params_str = if jni_params.is_empty() {
            "JNIEnv* env, jobject thiz".to_string()
        } else {
            format!("JNIEnv* env, jobject thiz, {}", jni_params.join(", "))
        };

        out.push_str(&format!("JNIEXPORT {} JNICALL {}({}) {{\n", jni_type(sig.ret).unwrap(), jni_name, params_str));

        let mut call_args = Vec::new();
        let mut string_locals: Vec<(String, String)> = Vec::new();
        for (p, t) in &sig.params {
            let mp = mangle_plain(p);
            match t {
                NativeType::Str => {
                    out.push_str(&format!(
                        "    const char* _{mp} = (*env)->GetStringUTFChars(env, j_{mp}, NULL);\n",
                        mp = mp
                    ));
                    call_args.push(format!("_{}", mp));
                    string_locals.push((mp.clone(), format!("j_{}", mp)));
                }
                NativeType::Bool => call_args.push(format!("(int)j_{}", mp)),
                _ => call_args.push(format!("j_{}", mp)),
            }
        }

        let call = format!("{}({})", name, call_args.join(", "));
        let release_strings = |out: &mut String| {
            for (mp, jname) in &string_locals {
                out.push_str(&format!("    (*env)->ReleaseStringUTFChars(env, {}, _{});\n", jname, mp));
            }
        };

        match sig.ret {
            NativeType::Str => {
                out.push_str(&format!("    const char* _ret = {};\n", call));
                out.push_str("    jstring j_ret = (*env)->NewStringUTF(env, _ret);\n");
                out.push_str("    dgn_free_string((char*)_ret);\n");
                release_strings(&mut out);
                out.push_str("    return j_ret;\n");
            }
            NativeType::Bool => {
                out.push_str(&format!("    int _ret = {};\n", call));
                release_strings(&mut out);
                out.push_str("    return (jboolean)_ret;\n");
            }
            _ => {
                out.push_str(&format!("    {} _ret = {};\n", sig.ret.c_type(), call));
                release_strings(&mut out);
                out.push_str(&format!("    return ({})_ret;\n", jni_type(sig.ret).unwrap()));
            }
        }
        out.push_str("}\n\n");
    }
    Ok(out)
}
