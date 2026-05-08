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
//! - Options arg parsing uses `Vec<(Arc<String>, Arc<HeapValue>)>`
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
//!   polymorphic `Vec<(Arc<String>, Arc<HeapValue>)>` tree and producing
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

use crate::marshal::register_typed_async_fn_3_full;
use crate::marshal::register_typed_async_fn_2_full;
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::HeapValue;
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

/// Extract optional headers from an `options: HashMap<string, *>` arg.
/// The options HashMap may contain a `"headers"` key whose value is itself
/// a `HashMap<string, string>` (`HeapValue::HashMap` variant). Walks the
/// outer pair list linearly looking for `"headers"`, then reads the
/// nested HashMap's keys/values buffers.
fn extract_headers(options: &[(Arc<String>, Arc<HeapValue>)]) -> Vec<(String, String)> {
    for (k, v) in options.iter() {
        if k.as_str() == "headers" {
            if let HeapValue::HashMap(d) = &**v {
                return d
                    .keys
                    .data
                    .iter()
                    .zip(d.values.data.iter())
                    .filter_map(|(hk, hv)| match &**hv {
                        HeapValue::String(s) => Some(((**hk).clone(), (**s).clone())),
                        _ => None,
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Extract optional `timeout` (milliseconds) from the options HashMap.
/// Walks linearly for `"timeout"`; if present and integer, converts to a
/// `Duration`.
///
/// Currently accepts `HeapValue::BigInt` (i64-typed integer) values only.
/// `number`-typed (f64) timeout values surface as raw scalar slots in
/// post-bulldozer Shape and don't reach `HeapValue` — supporting them
/// would require either Shape user code passing an int (`5000` not
/// `5000.0`) OR a future `HeapValue::NativeScalar`-aware branch here.
/// Documented for follow-on if a consumer surfaces.
fn extract_timeout(
    options: &[(Arc<String>, Arc<HeapValue>)],
) -> Option<std::time::Duration> {
    for (k, v) in options.iter() {
        if k.as_str() == "timeout" {
            if let HeapValue::BigInt(ms) = &**v {
                let n = **ms;
                if n > 0 {
                    return Some(std::time::Duration::from_millis(n as u64));
                }
            }
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
        description: "Request options: { headers?: HashMap, timeout?: int }"
            .to_string(),
        default_snippet: Some("{}".to_string()),
        ..Default::default()
    };

    let response_ty =
        ConcreteType::Result(Box::new(ConcreteType::Named("HttpResponse".to_string())));

    // http.get(url: string, options?: HashMap) -> Result<HttpResponse>
    register_typed_async_fn_2_full::<_, _, Arc<String>, Vec<(Arc<String>, Arc<HeapValue>)>>(
        &mut module,
        "get",
        "Perform an HTTP GET request",
        [url_param.clone(), options_param.clone()],
        response_ty.clone(),
        |url: Arc<String>, options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
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
    register_typed_async_fn_2_full::<_, _, Arc<String>, Vec<(Arc<String>, Arc<HeapValue>)>>(
        &mut module,
        "delete",
        "Perform an HTTP DELETE request",
        [url_param, options_param],
        response_ty,
        |url: Arc<String>, options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
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
        description: "Request options: { headers?: HashMap, timeout?: int }"
            .to_string(),
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
    register_typed_async_fn_3_full::<
        _,
        _,
        Arc<String>,
        Arc<String>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
    >(
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
         options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
            let mut builder = reqwest::Client::new()
                .post(url.as_str())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "text/plain; charset=utf-8",
                )
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
    register_typed_async_fn_3_full::<
        _,
        _,
        Arc<String>,
        Vec<u8>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
    >(
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
         body: Vec<u8>,
         options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
            let mut builder = reqwest::Client::new()
                .post(url.as_str())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/octet-stream",
                )
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
    register_typed_async_fn_3_full::<
        _,
        _,
        Arc<String>,
        Arc<String>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
    >(
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
         options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
            let mut builder = reqwest::Client::new()
                .put(url.as_str())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "text/plain; charset=utf-8",
                )
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
    register_typed_async_fn_3_full::<
        _,
        _,
        Arc<String>,
        Vec<u8>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
    >(
        &mut module,
        "put_bytes",
        "Perform an HTTP PUT request with a binary body",
        [url_param_3, body_bytes_param, options_param_3],
        response_ty_3,
        |url: Arc<String>,
         body: Vec<u8>,
         options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
            let mut builder = reqwest::Client::new()
                .put(url.as_str())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/octet-stream",
                )
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

    // N7 ε disposition (REFINEMENT-3A γ + a, 2026-05-07): post_json /
    // put_json take `body: object` which lands in this body as
    // `Vec<(Arc<String>, Arc<HeapValue>)>` via the existing FromSlot
    // impl at `crates/shape-runtime/src/marshal.rs:624`
    // (`NATIVE_KIND = NativeKind::Ptr(HeapKind::HashMap)` — supervisor's
    // load-bearing finding: the HashMap container anchors the slot kind
    // structurally, side-stepping the N4-α single-any wildcard refusal
    // that blocks the 5 single-any consumers C7/C10-C13 deferred to the
    // n7-single-any-input-resolution follow-on workstream).
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
        description: "Request options: { headers?: HashMap, timeout?: int }"
            .to_string(),
        default_snippet: Some("{}".to_string()),
        ..Default::default()
    };
    let response_ty_post_json =
        ConcreteType::Result(Box::new(ConcreteType::Named("HttpResponse".to_string())));

    // http.post_json(url: string, body: object, options?: HashMap) -> Result<HttpResponse>
    register_typed_async_fn_3_full::<
        _,
        _,
        Arc<String>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
    >(
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
         body: Vec<(Arc<String>, Arc<HeapValue>)>,
         options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
            let mut json_pairs: Vec<(String, crate::json_value::JsonValue)> =
                Vec::with_capacity(body.len());
            for (k, v) in body.iter() {
                json_pairs.push(((**k).clone(), crate::json_value::heap_to_json_value(v)?));
            }
            let json_value = crate::json_value::JsonValue::Object(json_pairs);
            let serde_json_v = crate::json_value::json_value_to_serde_json(&json_value);
            let body_str = serde_json::to_string(&serde_json_v)
                .map_err(|e| format!("http.post_json() body serialization failed: {}", e))?;

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
    register_typed_async_fn_3_full::<
        _,
        _,
        Arc<String>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
        Vec<(Arc<String>, Arc<HeapValue>)>,
    >(
        &mut module,
        "put_json",
        "Perform an HTTP PUT request with a JSON body",
        [url_param_post_json, body_object_param_put, options_param_post_json],
        response_ty_post_json,
        |url: Arc<String>,
         body: Vec<(Arc<String>, Arc<HeapValue>)>,
         options: Vec<(Arc<String>, Arc<HeapValue>)>| async move {
            let mut json_pairs: Vec<(String, crate::json_value::JsonValue)> =
                Vec::with_capacity(body.len());
            for (k, v) in body.iter() {
                json_pairs.push(((**k).clone(), crate::json_value::heap_to_json_value(v)?));
            }
            let json_value = crate::json_value::JsonValue::Object(json_pairs);
            let serde_json_v = crate::json_value::json_value_to_serde_json(&json_value);
            let body_str = serde_json::to_string(&serde_json_v)
                .map_err(|e| format!("http.put_json() body serialization failed: {}", e))?;

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
        },
    );

    module
}
