//! Native `http` module for making HTTP requests.
//!
//! Exports: http.get, http.delete (migrated to typed marshal layer in
//! Stage C).
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
//! `http.post` and `http.put` remain DEFERRED pending the **N4 — any-input
//! typed marshal** architectural surface. The `body: any` parameter
//! cannot be typed-marshaled cleanly without either (a) the N4 any-input
//! shape landing OR (b) the Shape API splitting into
//! `http.post_json(url, body: HashMap)` / `http.post_text(url, body: string)`
//! overloads (eliminating the any-input). Both are supervisor-level
//! decisions; surfaced via team-lead's relay batch alongside Dev 1's
//! N1/N2/N3 cascade. Deferral pattern mirrors `csv_module.rs:7-8/183-189`'s
//! `parse_records`/`stringify_records` breadcrumb.
//!
//! Tests deleted along with the legacy ValueWord-based fixtures, mirroring
//! the csv_module migration (commit `9f6b1d3`). New typed-marshal test
//! harness arrives with the shape-vm cleanup workstream.

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
                if *ms > 0 {
                    return Some(std::time::Duration::from_millis(*ms as u64));
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

    // Deferred: http.post, http.put.
    //
    // Both functions take a `body: any` parameter that maps to the
    // post-bulldozer typed marshal layer's not-yet-existing any-input
    // shape (N4 architectural sub-decision). Currently:
    // - There is no `FromSlot` impl for an `any`-typed input.
    //   `ConcreteType::Any` exists as a RETURN type (json/msgpack
    //   returns) but has no input side at the marshal layer.
    // - The legacy body code did
    //   `body_arg.is_none() / .as_str() / .to_json_value()` —
    //   all four are deleted ValueWord methods.
    //
    // Resolution requires either:
    //   (a) N4 lands `Vec<(Arc<String>, Arc<HeapValue>)>`-equivalent
    //       discriminated body shape, OR
    //   (b) Shape API splits into
    //       `http.post_json(url, body: HashMap)` and
    //       `http.post_text(url, body: string)` overloads.
    //
    // Held until supervisor sign-off via team-lead's relay batch.
    // Mirrors the deferral pattern from `csv_module.rs`'s
    // `parse_records`/`stringify_records` (lines 7-8 / 183-189).

    module
}
