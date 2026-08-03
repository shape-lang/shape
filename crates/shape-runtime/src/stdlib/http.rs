//! Native `http` module for making HTTP requests.
//!
//! Exports: http.get, http.delete (Stage C); http.post_text,
//! http.post_bytes, http.put_text, http.put_bytes (Stage D).
//!
//! Uses reqwest under the hood. `get` / `delete` / `post_text` /
//! `post_bytes` / `put_text` / `put_bytes` are async; `post_json` /
//! `put_json` are sync bodies that block on a shared runtime.
//!
//! **Permission gating (#252, owner ruling 2026-08-02 §R-G3).** Every
//! function requires `NetConnect`, checked by `check_http_permission`
//! immediately above its `send()` against the host parsed from that call's
//! concrete URL — so `ScopeConstraints::allowed_hosts` narrows per request,
//! not per run. Until #252 this header claimed a gate that did not exist:
//! the async ABI had no `ModuleContext` (it borrows the VM and cannot cross
//! an await), so all six async verbs ran unconditionally, and the two sync
//! JSON verbs simply never called a check despite having a context in hand.
//! Async bodies now receive an owned `Arc<PermissionContext>` instead.
//!
//! Stage C HashMap-marshal P1(b) migration (2026-05-07):
//! - Outer response shape `{status, headers, body, ok}` returns via
//!   `TypedReturn::OkObjectPairs` (Cluster #4 β shape, mirrors
//!   `arrow.metadata` precedent at `arrow_module.rs:127`).
//! - Inner `headers` field carries `HashMap<string, string>` payload via
//!   `ConcreteReturn::HashMapStringString` (insertion-order preserved).
//! - Options arg parsing uses `JsonValue`
//!   FromSlot impl from Step 1 P1(b) infrastructure
//!   (`crates/shape-runtime/src/marshal.rs`, Stage C commit `36519f6`).
//!
//! Stage D N4 partial sign-off (2026-05-07; supervisor relay):
//! - `http.post`/`http.put` legacy shape (single fn with `body: any`)
//!   replaced by typed overloads via Shape API split
//!   (`stdlib-src/core/http.shape`):
//!     - `post_text(url, body: string, options)` — sets
//!       `Content-Type: text/plain; charset=utf-8`
//!     - `post_bytes(url, body: Array<int>, options)` — sets
//!       `Content-Type: application/octet-stream`
//!     - `put_text(url, body: string, options)` — same content-type as
//!       post_text
//!     - `put_bytes(url, body: Array<int>, options)` — same as post_bytes
//! - Body types map directly to existing `FromSlot` impls
//!   (`Arc<String>` at `marshal.rs:129`, `Vec<u8>` at `marshal.rs:330`)
//!   per supervisor's "mechanical typed marshal" framing.
//! - `http.post_json(url, body: object, options)` and `http.put_json`
//!   remain DEFERRED pending architectural sub-decision **N7 —
//!   HeapValue→JSON serializer for HTTP / object-output marshal
//!   contexts.** The `body: object` shape requires walking the
//!   polymorphic `JsonValue` tree and producing
//!   a JSON string; per-variant serialization choices for Decimal,
//!   DataTable, Content, Temporal, TableView each represent a
//!   user-visible behavioral commitment that needs supervisor sign-off
//!   (architectural-adjacent helper, refused as bundled with Step 2 per
//!   the "no bundling architectural decisions" watchlist refusal).
//!   Surfaced via team-lead's relay batch.
//!
//! Tests deleted along with the legacy ValueWord-based fixtures, mirroring
//! the csv_module migration (commit `9f6b1d3`). New typed-marshal test
//! harness arrives with the shape-vm cleanup workstream.

use crate::json_value::JsonValue;
use crate::marshal::register_typed_async_fn_2_full;
use crate::marshal::register_typed_async_fn_3_full;
use crate::marshal::register_typed_fn_3_full;
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Extract the authority (`host` or `host:port`) from a request URL.
///
/// Deliberately small — `shape-runtime` does not depend on the `url` crate,
/// and the only consumer is the scope check below, which needs the host and
/// nothing else. Returns `None` when no authority can be identified, which
/// the caller turns into a refusal (fail closed: an unparseable URL must not
/// slip past the host scope check).
///
/// Known limitation, shared with `check_net_permission`'s own `split(':')`
/// and the `io.tcp_*` sites that feed it: a bracketed IPv6 literal
/// (`http://[::1]:8080/`) does not scope-match correctly. The base
/// `NetConnect` check is unaffected.
fn request_authority(url: &str) -> Option<&str> {
    let after_scheme = match url.find("://") {
        // A non-empty scheme is required: `://host` is not a URL, and
        // accepting it would let a malformed input pick up a host.
        Some(i) if i > 0 => &url[i + 3..],
        // No scheme: reqwest rejects it, but refuse before we get there.
        _ => return None,
    };
    let authority = after_scheme
        .find(['/', '?', '#'])
        .map_or(after_scheme, |i| &after_scheme[..i]);
    // Strip any `user:pass@` userinfo prefix.
    let authority = authority
        .rfind('@')
        .map_or(authority, |i| &authority[i + 1..]);
    if authority.is_empty() {
        return None;
    }
    Some(authority)
}

/// Gate one outbound request against the run's permission envelope.
///
/// Called immediately above the `send()` in every `http.*` body, against the
/// host parsed from that call's concrete URL argument — the same shape the
/// sync half uses (`stdlib/file.rs`, `stdlib_io/network_ops.rs`). The async
/// bodies reach it through the owned `Arc<PermissionContext>` the VM hands
/// them at spawn (#252 owner ruling 2026-08-02, §R-G3); before that ruling
/// they received no permission context at all and every request ran
/// unconditionally.
fn check_http_permission(
    perms: &crate::module_exports::PermissionContext,
    url: &str,
    func: &str,
) -> Result<(), String> {
    let authority = request_authority(url).ok_or_else(|| {
        format!("{func}: cannot determine a host from URL '{url}'; refusing the request")
    })?;
    crate::module_exports::check_net_permission(
        perms,
        shape_abi_v1::Permission::NetConnect,
        authority,
    )
    .map_err(|e| format!("{func}: {e}"))
}

/// Build the schemaful HttpResponse pair-list returned by every http.*
/// function. Schema: `{status: int, headers: HashMap<string, string>,
/// body: string, ok: bool}`. Insertion order preserved per `ObjectPairs`
/// contract (`crates/shape-runtime/src/typed_module_exports.rs:117`).
fn build_response_pairs(
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
) -> Vec<(String, ConcreteReturn)> {
    vec![
        ("status".to_string(), ConcreteReturn::I64(status as i64)),
        (
            "headers".to_string(),
            ConcreteReturn::HashMapStringString(headers),
        ),
        ("body".to_string(), ConcreteReturn::String(body)),
        (
            "ok".to_string(),
            ConcreteReturn::Bool((200..300).contains(&status)),
        ),
    ]
}

/// Extract optional headers from the `options` object.
///
/// WF-2E (2026-07-05): `options` is walked into a `JsonValue` tree at the
/// marshal boundary (typed, kind-directed — no pointer reinterpretation).
/// A `"headers"` field whose value is an object contributes one HTTP
/// header per key; scalar values render to their string form. A non-object
/// or absent `headers` field yields no headers.
fn extract_headers(options: &JsonValue) -> Vec<(String, String)> {
    let JsonValue::Object(fields) = options else {
        return Vec::new();
    };
    for (k, v) in fields.iter() {
        if k == "headers" {
            if let JsonValue::Object(hdrs) = v {
                let mut out = Vec::with_capacity(hdrs.len());
                for (hk, hv) in hdrs.iter() {
                    let val = match hv {
                        JsonValue::String(s) => s.clone(),
                        JsonValue::Int(i) => i.to_string(),
                        JsonValue::Number(n) => n.to_string(),
                        JsonValue::Bool(b) => b.to_string(),
                        // null / nested containers are not valid header
                        // values — skip.
                        _ => continue,
                    };
                    out.push((hk.clone(), val));
                }
                return out;
            }
            return Vec::new();
        }
    }
    Vec::new()
}

/// Extract optional `timeout` (milliseconds) from the `options` object.
///
/// Accepts both `int` and `number` timeout values (both are ordinary scalar
/// leaves in the `JsonValue` tree — the pre-WF-2E `HeapValue::BigInt`
/// int-only restriction is gone). Non-positive / non-numeric values yield
/// no timeout.
fn extract_timeout(options: &JsonValue) -> Option<std::time::Duration> {
    let JsonValue::Object(fields) = options else {
        return None;
    };
    for (k, v) in fields.iter() {
        if k == "timeout" {
            let ms: i64 = match v {
                JsonValue::Int(i) => *i,
                JsonValue::Number(n) => *n as i64,
                _ => return None,
            };
            if ms > 0 {
                return Some(std::time::Duration::from_millis(ms as u64));
            }
            return None;
        }
    }
    None
}

/// Create the `http` module with async HTTP request functions.
pub fn create_http_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::http");
    module.description = "HTTP client for making web requests".to_string();

    let url_param = ModuleParam {
        name: "url".to_string(),
        type_name: "string".to_string(),
        required: true,
        description: "URL to request".to_string(),
        ..Default::default()
    };

    let options_param = ModuleParam {
        name: "options".to_string(),
        type_name: "HashMap<string, any>".to_string(),
        required: false,
        description: "Request options: { headers?: HashMap, timeout?: int }".to_string(),
        default_snippet: Some("{}".to_string()),
        ..Default::default()
    };

    let response_ty =
        ConcreteType::Result(Box::new(ConcreteType::Named("HttpResponse".to_string())));

    // http.get(url: string, options?: HashMap) -> Result<HttpResponse>
    register_typed_async_fn_2_full::<_, _, Arc<String>, JsonValue>(
        &mut module,
        "get",
        "Perform an HTTP GET request",
        [url_param.clone(), options_param.clone()],
        response_ty.clone(),
        |url: Arc<String>,
         options: JsonValue,
         perms: Arc<crate::module_exports::PermissionContext>| async move {
            check_http_permission(&perms, url.as_str(), "http.get()")?;
            let mut builder = reqwest::Client::new().get(url.as_str());

            for (k, v) in extract_headers(&options) {
                builder = builder.header(&k, &v);
            }
            if let Some(timeout) = extract_timeout(&options) {
                builder = builder.timeout(timeout);
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.get() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body = resp
                .text()
                .await
                .map_err(|e| format!("http.get() body read failed: {}", e))?;

            Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                status, headers, body,
            )))
        },
    );

    // http.delete(url: string, options?: HashMap) -> Result<HttpResponse>
    register_typed_async_fn_2_full::<_, _, Arc<String>, JsonValue>(
        &mut module,
        "delete",
        "Perform an HTTP DELETE request",
        [url_param, options_param],
        response_ty,
        |url: Arc<String>,
         options: JsonValue,
         perms: Arc<crate::module_exports::PermissionContext>| async move {
            check_http_permission(&perms, url.as_str(), "http.delete()")?;
            let mut builder = reqwest::Client::new().delete(url.as_str());

            for (k, v) in extract_headers(&options) {
                builder = builder.header(&k, &v);
            }
            if let Some(timeout) = extract_timeout(&options) {
                builder = builder.timeout(timeout);
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.delete() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body = resp
                .text()
                .await
                .map_err(|e| format!("http.delete() body read failed: {}", e))?;

            Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                status, headers, body,
            )))
        },
    );

    // Stage D N4 partial sign-off: 4 typed overloads via Shape API
    // split. Each body type is a fixed-arity register_typed_async_fn_3
    // with one specific body type per overload, per supervisor's
    // "mechanical typed marshal" framing. Reuses build_response_pairs +
    // extract_headers + extract_timeout from the get/delete path.

    let url_param_3 = ModuleParam {
        name: "url".to_string(),
        type_name: "string".to_string(),
        required: true,
        description: "URL to request".to_string(),
        ..Default::default()
    };
    let options_param_3 = ModuleParam {
        name: "options".to_string(),
        type_name: "HashMap<string, any>".to_string(),
        required: false,
        description: "Request options: { headers?: HashMap, timeout?: int }".to_string(),
        default_snippet: Some("{}".to_string()),
        ..Default::default()
    };
    let body_text_param = ModuleParam {
        name: "body".to_string(),
        type_name: "string".to_string(),
        required: true,
        description: "Request body as a string (sent verbatim)".to_string(),
        ..Default::default()
    };
    let body_bytes_param = ModuleParam {
        name: "body".to_string(),
        type_name: "Array<int>".to_string(),
        required: true,
        description: "Request body as a byte array".to_string(),
        ..Default::default()
    };
    let response_ty_3 =
        ConcreteType::Result(Box::new(ConcreteType::Named("HttpResponse".to_string())));

    // http.post_text(url: string, body: string, options?: HashMap) -> Result<HttpResponse>
    register_typed_async_fn_3_full::<_, _, Arc<String>, Arc<String>, JsonValue>(
        &mut module,
        "post_text",
        "Perform an HTTP POST request with a text body",
        [
            url_param_3.clone(),
            body_text_param.clone(),
            options_param_3.clone(),
        ],
        response_ty_3.clone(),
        |url: Arc<String>,
         body: Arc<String>,
         options: JsonValue,
         perms: Arc<crate::module_exports::PermissionContext>| async move {
            check_http_permission(&perms, url.as_str(), "http.post_text()")?;
            let mut builder = reqwest::Client::new()
                .post(url.as_str())
                .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(body.as_str().to_string());

            for (k, v) in extract_headers(&options) {
                builder = builder.header(&k, &v);
            }
            if let Some(timeout) = extract_timeout(&options) {
                builder = builder.timeout(timeout);
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.post_text() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body_out = resp
                .text()
                .await
                .map_err(|e| format!("http.post_text() body read failed: {}", e))?;

            Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                status, headers, body_out,
            )))
        },
    );

    // http.post_bytes(url: string, body: Array<int>, options?: HashMap) -> Result<HttpResponse>
    //
    // WF-2E (2026-07-05): `Array<int>` is a `TypedArray<i64>` (8-byte
    // elements), so the body reads `Vec<i64>` and narrows each element to a
    // byte. Reading it as `Vec<u8>` (a `TypedArray<u8>` reader) mis-strided
    // the i64 buffer at 1 byte and corrupted the body (`[72,105]` → `[72,0]`).
    register_typed_async_fn_3_full::<_, _, Arc<String>, Vec<i64>, JsonValue>(
        &mut module,
        "post_bytes",
        "Perform an HTTP POST request with a binary body",
        [
            url_param_3.clone(),
            body_bytes_param.clone(),
            options_param_3.clone(),
        ],
        response_ty_3.clone(),
        |url: Arc<String>,
         body: Vec<i64>,
         options: JsonValue,
         perms: Arc<crate::module_exports::PermissionContext>| async move {
            check_http_permission(&perms, url.as_str(), "http.post_bytes()")?;
            let body: Vec<u8> = body.into_iter().map(|b| b as u8).collect();
            let mut builder = reqwest::Client::new()
                .post(url.as_str())
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .body(body);

            for (k, v) in extract_headers(&options) {
                builder = builder.header(&k, &v);
            }
            if let Some(timeout) = extract_timeout(&options) {
                builder = builder.timeout(timeout);
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.post_bytes() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body_out = resp
                .text()
                .await
                .map_err(|e| format!("http.post_bytes() body read failed: {}", e))?;

            Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                status, headers, body_out,
            )))
        },
    );

    // http.put_text(url: string, body: string, options?: HashMap) -> Result<HttpResponse>
    register_typed_async_fn_3_full::<_, _, Arc<String>, Arc<String>, JsonValue>(
        &mut module,
        "put_text",
        "Perform an HTTP PUT request with a text body",
        [
            url_param_3.clone(),
            body_text_param,
            options_param_3.clone(),
        ],
        response_ty_3.clone(),
        |url: Arc<String>,
         body: Arc<String>,
         options: JsonValue,
         perms: Arc<crate::module_exports::PermissionContext>| async move {
            check_http_permission(&perms, url.as_str(), "http.put_text()")?;
            let mut builder = reqwest::Client::new()
                .put(url.as_str())
                .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(body.as_str().to_string());

            for (k, v) in extract_headers(&options) {
                builder = builder.header(&k, &v);
            }
            if let Some(timeout) = extract_timeout(&options) {
                builder = builder.timeout(timeout);
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.put_text() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body_out = resp
                .text()
                .await
                .map_err(|e| format!("http.put_text() body read failed: {}", e))?;

            Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                status, headers, body_out,
            )))
        },
    );

    // http.put_bytes(url: string, body: Array<int>, options?: HashMap) -> Result<HttpResponse>
    //
    // WF-2E (2026-07-05): see post_bytes — `Array<int>` is `TypedArray<i64>`;
    // read `Vec<i64>` and narrow to bytes rather than mis-striding as `Vec<u8>`.
    register_typed_async_fn_3_full::<_, _, Arc<String>, Vec<i64>, JsonValue>(
        &mut module,
        "put_bytes",
        "Perform an HTTP PUT request with a binary body",
        [url_param_3, body_bytes_param, options_param_3],
        response_ty_3,
        |url: Arc<String>,
         body: Vec<i64>,
         options: JsonValue,
         perms: Arc<crate::module_exports::PermissionContext>| async move {
            check_http_permission(&perms, url.as_str(), "http.put_bytes()")?;
            let body: Vec<u8> = body.into_iter().map(|b| b as u8).collect();
            let mut builder = reqwest::Client::new()
                .put(url.as_str())
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .body(body);

            for (k, v) in extract_headers(&options) {
                builder = builder.header(&k, &v);
            }
            if let Some(timeout) = extract_timeout(&options) {
                builder = builder.timeout(timeout);
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.put_bytes() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body_out = resp
                .text()
                .await
                .map_err(|e| format!("http.put_bytes() body read failed: {}", e))?;

            Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                status, headers, body_out,
            )))
        },
    );

    // W71 strict-flip correction (2026-07-01): post_json / put_json take
    // `body: object`, which is a `Ptr(HeapKind::TypedObject)` carrier for
    // object literals. Options remain `HashMap<string, any>` and continue
    // through the direct HashMap marshal path. The body serializer walks
    // the TypedObject schema + per-field NativeKind table; it never casts
    // object bits to HashMap or resurrects a HeapValue wrapper.
    //
    // Body algorithm: build `JsonValue::Object` by walking each HashMap
    // pair via `heap_to_json_value(&v)?` (C2) → `json_value_to_serde_json`
    // (C3) → `serde_json::to_string(&serde_json_v)?` → reqwest body with
    // `Content-Type: application/json`. Insertion order preserved per
    // ObjectPairs contract.

    let url_param_post_json = ModuleParam {
        name: "url".to_string(),
        type_name: "string".to_string(),
        required: true,
        description: "URL to request".to_string(),
        ..Default::default()
    };
    let body_object_param_post = ModuleParam {
        name: "body".to_string(),
        type_name: "object".to_string(),
        required: true,
        description: "Request body as an object (sent as JSON)".to_string(),
        ..Default::default()
    };
    let options_param_post_json = ModuleParam {
        name: "options".to_string(),
        type_name: "HashMap<string, any>".to_string(),
        required: false,
        description: "Request options: { headers?: HashMap, timeout?: int }".to_string(),
        default_snippet: Some("{}".to_string()),
        ..Default::default()
    };
    let response_ty_post_json =
        ConcreteType::Result(Box::new(ConcreteType::Named("HttpResponse".to_string())));

    // http.post_json(url: string, body: object, options?: HashMap) -> Result<HttpResponse>
    register_typed_fn_3_full::<_, Arc<String>, shape_value::heap_value::TypedObjectPtr, JsonValue>(
        &mut module,
        "post_json",
        "Perform an HTTP POST request with a JSON body",
        [
            url_param_post_json.clone(),
            body_object_param_post,
            options_param_post_json.clone(),
        ],
        response_ty_post_json.clone(),
        |url: Arc<String>,
         body: shape_value::heap_value::TypedObjectPtr,
         options: JsonValue,
         ctx| {
            check_http_permission(&ctx.permissions, url.as_str(), "http.post_json()")?;
            let json_value = crate::json_value::typed_object_ptr_to_json_value_with_registry(
                &body,
                ctx.schemas,
            )?;
            let serde_json_v = crate::json_value::json_value_to_serde_json(&json_value);
            let body_str = serde_json::to_string(&serde_json_v)
                .map_err(|e| format!("http.post_json() body serialization failed: {}", e))?;

            crate::sync_bridge::block_on_shared(async move {
                let mut builder = reqwest::Client::new()
                    .post(url.as_str())
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body_str);

                for (k, v) in extract_headers(&options) {
                    builder = builder.header(&k, &v);
                }
                if let Some(timeout) = extract_timeout(&options) {
                    builder = builder.timeout(timeout);
                }

                let resp = builder
                    .send()
                    .await
                    .map_err(|e| format!("http.post_json() failed: {}", e))?;

                let status = resp.status().as_u16();
                let headers: Vec<(String, String)> = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                let body_out = resp
                    .text()
                    .await
                    .map_err(|e| format!("http.post_json() body read failed: {}", e))?;

                Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                    status, headers, body_out,
                )))
            })
            .map_err(|e| e.to_string())?
        },
    );

    // http.put_json(url: string, body: object, options?: HashMap) -> Result<HttpResponse>
    let body_object_param_put = ModuleParam {
        name: "body".to_string(),
        type_name: "object".to_string(),
        required: true,
        description: "Request body as an object (sent as JSON)".to_string(),
        ..Default::default()
    };
    register_typed_fn_3_full::<_, Arc<String>, shape_value::heap_value::TypedObjectPtr, JsonValue>(
        &mut module,
        "put_json",
        "Perform an HTTP PUT request with a JSON body",
        [
            url_param_post_json,
            body_object_param_put,
            options_param_post_json,
        ],
        response_ty_post_json,
        |url: Arc<String>,
         body: shape_value::heap_value::TypedObjectPtr,
         options: JsonValue,
         ctx| {
            check_http_permission(&ctx.permissions, url.as_str(), "http.put_json()")?;
            let json_value = crate::json_value::typed_object_ptr_to_json_value_with_registry(
                &body,
                ctx.schemas,
            )?;
            let serde_json_v = crate::json_value::json_value_to_serde_json(&json_value);
            let body_str = serde_json::to_string(&serde_json_v)
                .map_err(|e| format!("http.put_json() body serialization failed: {}", e))?;

            crate::sync_bridge::block_on_shared(async move {
                let mut builder = reqwest::Client::new()
                    .put(url.as_str())
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body_str);

                for (k, v) in extract_headers(&options) {
                    builder = builder.header(&k, &v);
                }
                if let Some(timeout) = extract_timeout(&options) {
                    builder = builder.timeout(timeout);
                }

                let resp = builder
                    .send()
                    .await
                    .map_err(|e| format!("http.put_json() failed: {}", e))?;

                let status = resp.status().as_u16();
                let headers: Vec<(String, String)> = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                let body_out = resp
                    .text()
                    .await
                    .map_err(|e| format!("http.put_json() body read failed: {}", e))?;

                Ok(TypedReturn::OkObjectPairs(build_response_pairs(
                    status, headers, body_out,
                )))
            })
            .map_err(|e| e.to_string())?
        },
    );

    module
}

// ───────────────────── #252 permission-gate tests ─────────────────────
//
// Until the #252 owner ruling (2026-08-02, §R-G3) every `http.*` call ran
// unconditionally: the async ABI carried no permission context, so there was
// nothing for a body to check against. These tests pin the gate at the
// dispatch entry point that the VM actually calls — `TypedModuleAsyncFunction
// ::invoke` — rather than at the helper, so a future ABI change that drops
// the envelope again fails here.
//
// Every refusal test points at a port bound by the test itself and never
// accepted from. The assertion is therefore two-sided: the call must fail
// with the permission error AND the listener must have seen no connection at
// all. Without the second half a passing test could not distinguish "refused
// before dialling" from "dialled and the connection happened to fail".
#[cfg(test)]
mod permission_gate_tests {
    use super::*;
    use crate::module_exports::PermissionContext;
    use crate::typed_module_exports::TypedReturn;
    use shape_abi_v1::{Permission, PermissionSet, ScopeConstraints};
    use shape_value::KindedSlot;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Everything granted except `NetConnect` — the envelope an operator gets
    /// from `[permissions] net.connect = false`.
    fn without_net_connect() -> Arc<PermissionContext> {
        let mut set = PermissionSet::full();
        set.remove(&Permission::NetConnect);
        Arc::new(PermissionContext::new(Some(set), None))
    }

    /// `NetConnect` granted, optionally narrowed to `allowed_hosts`.
    fn with_net_connect(allowed_hosts: Option<Vec<String>>) -> Arc<PermissionContext> {
        let mut set = PermissionSet::pure();
        set.insert(Permission::NetConnect);
        let scope = allowed_hosts.map(|allowed_hosts| {
            set.insert(Permission::NetScoped);
            ScopeConstraints {
                allowed_hosts,
                ..Default::default()
            }
        });
        Arc::new(PermissionContext::new(Some(set), scope))
    }

    /// Invoke an async `http.*` export exactly as the VM's `TypedAsync`
    /// dispatch arm does (`vm_impl/modules.rs`).
    fn call_async(
        name: &str,
        args: Vec<KindedSlot>,
        perms: Arc<PermissionContext>,
    ) -> Result<TypedReturn, String> {
        // The VM installs this at run start; these tests drive the dispatch
        // entry point directly, so they install it themselves. Guarded by a
        // `Once` because `initialize_shared_runtime` checks-then-sets a
        // `OnceCell` and errors with "race condition" when two test threads
        // interleave there.
        static RUNTIME: std::sync::Once = std::sync::Once::new();
        RUNTIME.call_once(|| {
            crate::sync_bridge::initialize_shared_runtime().expect("shared tokio runtime");
        });
        let module = create_http_module();
        let entry = module
            .typed_exports()
            .get_async(name)
            .unwrap_or_else(|| panic!("http.{name} is not a registered async export"))
            .clone();
        crate::sync_bridge::block_on_shared((entry.invoke)(args, perms))
            .expect("no ambient tokio runtime for the http test")
    }

    /// A bound-but-never-accepted port. Any connection attempt queues on it,
    /// so `assert_never_dialled` can tell whether the request left the gate.
    fn dead_port() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("set_nonblocking");
        let url = format!(
            "http://127.0.0.1:{}/probe",
            listener.local_addr().unwrap().port()
        );
        (listener, url)
    }

    /// Assert no client ever reached the listener — i.e. the refusal happened
    /// before any socket work, not after a failed connect.
    fn assert_never_dialled(listener: &TcpListener) {
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok((peer, _)) => panic!(
                "a connection reached the listener from {:?} — the request was NOT refused \
                 before dialling",
                peer.peer_addr()
            ),
            Err(e) => panic!("unexpected accept error: {e}"),
        }
    }

    /// One-shot HTTP/1.1 server returning `200 OK` with body `hi`.
    fn one_shot_server() -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
                );
                let _ = stream.flush();
            }
        });
        (format!("http://127.0.0.1:{port}/"), handle)
    }

    fn status_of(ret: &TypedReturn) -> i64 {
        let TypedReturn::OkObjectPairs(pairs) = ret else {
            panic!("expected OkObjectPairs, got {ret:?}");
        };
        for (k, v) in pairs {
            if k == "status" {
                if let ConcreteReturn::I64(s) = v {
                    return *s;
                }
            }
        }
        panic!("no int `status` field in {pairs:?}");
    }

    // ── refusal: the permission is absent ──────────────────────────────

    #[test]
    fn http_get_refuses_without_net_connect() {
        let (listener, url) = dead_port();
        let err = call_async(
            "get",
            vec![KindedSlot::from_string(&url), KindedSlot::none()],
            without_net_connect(),
        )
        .expect_err("http.get must be refused without NetConnect");

        assert!(
            err.contains("Permission denied") && err.contains("net.connect"),
            "expected a NetConnect permission refusal, got: {err}"
        );
        assert_never_dialled(&listener);
    }

    #[test]
    fn http_post_text_refuses_without_net_connect() {
        let (listener, url) = dead_port();
        let err = call_async(
            "post_text",
            vec![
                KindedSlot::from_string(&url),
                KindedSlot::from_string("payload"),
                KindedSlot::none(),
            ],
            without_net_connect(),
        )
        .expect_err("http.post_text must be refused without NetConnect");

        assert!(
            err.contains("Permission denied") && err.contains("net.connect"),
            "expected a NetConnect permission refusal, got: {err}"
        );
        assert_never_dialled(&listener);
    }

    #[test]
    fn http_delete_refuses_without_net_connect() {
        let (listener, url) = dead_port();
        let err = call_async(
            "delete",
            vec![KindedSlot::from_string(&url), KindedSlot::none()],
            without_net_connect(),
        )
        .expect_err("http.delete must be refused without NetConnect");

        assert!(err.contains("Permission denied"), "got: {err}");
        assert_never_dialled(&listener);
    }

    // ── refusal: the host is outside ScopeConstraints ──────────────────

    #[test]
    fn http_get_refuses_host_outside_scope_constraints() {
        let (listener, url) = dead_port();
        let err = call_async(
            "get",
            vec![KindedSlot::from_string(&url), KindedSlot::none()],
            with_net_connect(Some(vec!["api.example.com".to_string()])),
        )
        .expect_err("http.get must be refused for a host outside allowed_hosts");

        assert!(
            err.contains("Scope constraint denied") && err.contains("127.0.0.1"),
            "expected a host scope refusal naming the concrete host, got: {err}"
        );
        assert_never_dialled(&listener);
    }

    #[test]
    fn http_post_text_refuses_host_outside_scope_constraints() {
        let (listener, url) = dead_port();
        let err = call_async(
            "post_text",
            vec![
                KindedSlot::from_string(&url),
                KindedSlot::from_string("payload"),
                KindedSlot::none(),
            ],
            with_net_connect(Some(vec!["*.trusted.io".to_string()])),
        )
        .expect_err("http.post_text must be refused for a host outside allowed_hosts");

        assert!(err.contains("Scope constraint denied"), "got: {err}");
        assert_never_dialled(&listener);
    }

    // ── success: the gate is not simply refusing everything ────────────

    #[test]
    fn http_get_succeeds_with_net_connect() {
        let (url, server) = one_shot_server();
        let ret = call_async(
            "get",
            vec![KindedSlot::from_string(&url), KindedSlot::none()],
            with_net_connect(None),
        )
        .expect("http.get must succeed once NetConnect is granted");

        assert_eq!(status_of(&ret), 200);
        server.join().expect("server thread");
    }

    #[test]
    fn http_get_succeeds_for_host_inside_scope_constraints() {
        let (url, server) = one_shot_server();
        let ret = call_async(
            "get",
            vec![KindedSlot::from_string(&url), KindedSlot::none()],
            with_net_connect(Some(vec!["127.0.0.1".to_string()])),
        )
        .expect("an allowed host must pass the scope check");

        assert_eq!(status_of(&ret), 200);
        server.join().expect("server thread");
    }

    #[test]
    fn http_post_text_succeeds_with_net_connect() {
        let (url, server) = one_shot_server();
        let ret = call_async(
            "post_text",
            vec![
                KindedSlot::from_string(&url),
                KindedSlot::from_string("payload"),
                KindedSlot::none(),
            ],
            with_net_connect(None),
        )
        .expect("http.post_text must succeed once NetConnect is granted");

        assert_eq!(status_of(&ret), 200);
        server.join().expect("server thread");
    }

    // ── the envelope really does reach the body across the await ───────

    #[test]
    fn unrestricted_envelope_still_permits_http() {
        // `granted_permissions: None` is the trusted-local `shape run`
        // posture. It must stay allow-all — the gate is not a blanket
        // refusal for callers that never installed an envelope.
        let (url, server) = one_shot_server();
        let ret = call_async(
            "get",
            vec![KindedSlot::from_string(&url), KindedSlot::none()],
            Arc::new(PermissionContext::unrestricted()),
        )
        .expect("an unrestricted envelope must permit http.get");

        assert_eq!(status_of(&ret), 200);
        server.join().expect("server thread");
    }

    // ── URL → host extraction ──────────────────────────────────────────

    #[test]
    fn request_authority_extracts_host_and_port() {
        assert_eq!(
            request_authority("http://example.com/a/b"),
            Some("example.com")
        );
        assert_eq!(
            request_authority("https://api.example.com:8443/v1?q=1"),
            Some("api.example.com:8443")
        );
        assert_eq!(
            request_authority("https://example.com"),
            Some("example.com")
        );
        assert_eq!(
            request_authority("https://example.com?q=1"),
            Some("example.com")
        );
        assert_eq!(
            request_authority("https://example.com#frag"),
            Some("example.com")
        );
        // userinfo must not be mistaken for the host
        assert_eq!(
            request_authority("https://user:pass@internal.host/x"),
            Some("internal.host")
        );
    }

    #[test]
    fn request_authority_refuses_what_it_cannot_parse() {
        // Fail closed: no authority means no host to scope-check against.
        assert_eq!(request_authority("example.com/path"), None);
        assert_eq!(request_authority("://nohost"), None);
        assert_eq!(request_authority("http:///onlypath"), None);
        assert_eq!(request_authority(""), None);
    }

    #[test]
    fn http_get_refuses_a_url_with_no_parseable_host() {
        let err = call_async(
            "get",
            vec![KindedSlot::from_string("not-a-url"), KindedSlot::none()],
            with_net_connect(Some(vec!["api.example.com".to_string()])),
        )
        .expect_err("an unparseable URL must be refused, not scope-checked against nothing");

        assert!(
            err.contains("cannot determine a host"),
            "expected a fail-closed host-parse refusal, got: {err}"
        );
    }
}
