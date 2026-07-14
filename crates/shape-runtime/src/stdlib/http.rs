//! Native `http` module for making HTTP requests.
//!
//! Exports: http.get, http.delete (Stage C); http.post_text,
//! http.post_bytes, http.put_text, http.put_bytes (Stage D).
//!
//! All functions are async. Uses reqwest under the hood.
//! Policy gated: requires NetConnect permission.
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
        |url: Arc<String>, options: JsonValue| async move {
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
        |url: Arc<String>, options: JsonValue| async move {
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
        |url: Arc<String>, body: Arc<String>, options: JsonValue| async move {
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
        |url: Arc<String>, body: Vec<i64>, options: JsonValue| async move {
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
        |url: Arc<String>, body: Arc<String>, options: JsonValue| async move {
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
        |url: Arc<String>, body: Vec<i64>, options: JsonValue| async move {
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
