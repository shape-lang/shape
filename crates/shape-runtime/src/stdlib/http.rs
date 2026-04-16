//! Native `http` module for making HTTP requests.
//!
//! Exports: http.get, http.post, http.put, http.delete
//!
//! All functions are async. Uses reqwest under the hood.
//! Policy gated: requires NetConnect permission.

use crate::module_exports::{ModuleExports, ModuleFunction, ModuleParam};
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

/// Build an HttpResponse ValueWord HashMap from the response parts.
/// Fields: status (number), headers (HashMap), body (string), ok (bool)
fn build_response(status: u16, headers: Vec<(String, String)>, body: String) -> ValueWord {
    // headers as HashMap
    let mut h_keys = Vec::with_capacity(headers.len());
    let mut h_values = Vec::with_capacity(headers.len());
    for (hk, hv) in headers.iter() {
        h_keys.push(ValueWord::from_string(Arc::new(hk.clone())));
        h_values.push(ValueWord::from_string(Arc::new(hv.clone())));
    }

    let keys = vec![
        ValueWord::from_string(Arc::new("status".to_string())),
        ValueWord::from_string(Arc::new("headers".to_string())),
        ValueWord::from_string(Arc::new("body".to_string())),
        ValueWord::from_string(Arc::new("ok".to_string())),
    ];
    let values = vec![
        ValueWord::from_f64(status as f64),
        ValueWord::from_hashmap_pairs(h_keys, h_values),
        ValueWord::from_string(Arc::new(body)),
        ValueWord::from_bool((200..300).contains(&status)),
    ];
    ValueWord::from_hashmap_pairs(keys, values)
}

/// Extract optional headers from an options HashMap argument.
fn extract_headers(options: &ValueWord) -> Vec<(String, String)> {
    if let Some((keys, values, _)) = options.as_hashmap() {
        // Look for a "headers" key
        for (i, k) in keys.iter().enumerate() {
            if k.as_str() == Some("headers") {
                if let Some((hk, hv, _)) = values[i].as_hashmap() {
                    return hk
                        .iter()
                        .zip(hv.iter())
                        .filter_map(|(k, v)| {
                            Some((k.as_str()?.to_string(), v.as_str()?.to_string()))
                        })
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

/// Extract optional timeout from an options HashMap argument (in milliseconds).
fn extract_timeout(options: &ValueWord) -> Option<std::time::Duration> {
    if let Some((keys, values, _)) = options.as_hashmap() {
        for (i, k) in keys.iter().enumerate() {
            if k.as_str() == Some("timeout") {
                if let Some(ms) = values[i].as_number_coerce() {
                    if ms > 0.0 {
                        return Some(std::time::Duration::from_millis(ms as u64));
                    }
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
        type_name: "object".to_string(),
        required: false,
        description: "Request options: { headers?: HashMap, timeout?: number }".to_string(),
        ..Default::default()
    };

    let body_param = ModuleParam {
        name: "body".to_string(),
        type_name: "any".to_string(),
        required: false,
        description: "Request body (string or value to serialize as JSON)".to_string(),
        ..Default::default()
    };

    // http.get(url: string, options?: object) -> Result<HttpResponse>
    module.add_async_function_with_schema(
        "get",
        |args: Vec<ValueWord>| async move {
            let url = args
                .first()
                .and_then(|a| a.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| "http.get() requires a URL string".to_string())?;

            let mut builder = reqwest::Client::new().get(&url);

            if let Some(options) = args.get(1) {
                for (k, v) in extract_headers(options) {
                    builder = builder.header(&k, &v);
                }
                if let Some(timeout) = extract_timeout(options) {
                    builder = builder.timeout(timeout);
                }
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

            Ok(ValueWord::from_ok(build_response(status, headers, body)))
        },
        ModuleFunction {
            description: "Perform an HTTP GET request".to_string(),
            params: vec![url_param.clone(), options_param.clone()],
            return_type: Some("Result<HttpResponse>".to_string()),
        },
    );

    // http.post(url: string, body?: any, options?: object) -> Result<HttpResponse>
    module.add_async_function_with_schema(
        "post",
        |args: Vec<ValueWord>| async move {
            let url = args
                .first()
                .and_then(|a| a.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| "http.post() requires a URL string".to_string())?;

            let mut builder = reqwest::Client::new().post(&url);

            // Body
            if let Some(body_arg) = args.get(1) {
                if !body_arg.is_none() && !body_arg.is_unit() {
                    if let Some(s) = body_arg.as_str() {
                        builder = builder.body(s.to_string());
                    } else {
                        let json = body_arg.to_json_value();
                        builder = builder
                            .header("Content-Type", "application/json")
                            .body(serde_json::to_string(&json).unwrap_or_default());
                    }
                }
            }

            // Options
            if let Some(options) = args.get(2) {
                for (k, v) in extract_headers(options) {
                    builder = builder.header(&k, &v);
                }
                if let Some(timeout) = extract_timeout(options) {
                    builder = builder.timeout(timeout);
                }
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.post() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body = resp
                .text()
                .await
                .map_err(|e| format!("http.post() body read failed: {}", e))?;

            Ok(ValueWord::from_ok(build_response(status, headers, body)))
        },
        ModuleFunction {
            description: "Perform an HTTP POST request".to_string(),
            params: vec![url_param.clone(), body_param.clone(), options_param.clone()],
            return_type: Some("Result<HttpResponse>".to_string()),
        },
    );

    // http.put(url: string, body?: any, options?: object) -> Result<HttpResponse>
    module.add_async_function_with_schema(
        "put",
        |args: Vec<ValueWord>| async move {
            let url = args
                .first()
                .and_then(|a| a.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| "http.put() requires a URL string".to_string())?;

            let mut builder = reqwest::Client::new().put(&url);

            if let Some(body_arg) = args.get(1) {
                if !body_arg.is_none() && !body_arg.is_unit() {
                    if let Some(s) = body_arg.as_str() {
                        builder = builder.body(s.to_string());
                    } else {
                        let json = body_arg.to_json_value();
                        builder = builder
                            .header("Content-Type", "application/json")
                            .body(serde_json::to_string(&json).unwrap_or_default());
                    }
                }
            }

            if let Some(options) = args.get(2) {
                for (k, v) in extract_headers(options) {
                    builder = builder.header(&k, &v);
                }
                if let Some(timeout) = extract_timeout(options) {
                    builder = builder.timeout(timeout);
                }
            }

            let resp = builder
                .send()
                .await
                .map_err(|e| format!("http.put() failed: {}", e))?;

            let status = resp.status().as_u16();
            let headers: Vec<(String, String)> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let body = resp
                .text()
                .await
                .map_err(|e| format!("http.put() body read failed: {}", e))?;

            Ok(ValueWord::from_ok(build_response(status, headers, body)))
        },
        ModuleFunction {
            description: "Perform an HTTP PUT request".to_string(),
            params: vec![url_param.clone(), body_param, options_param.clone()],
            return_type: Some("Result<HttpResponse>".to_string()),
        },
    );

    // http.delete(url: string, options?: object) -> Result<HttpResponse>
    module.add_async_function_with_schema(
        "delete",
        |args: Vec<ValueWord>| async move {
            let url = args
                .first()
                .and_then(|a| a.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| "http.delete() requires a URL string".to_string())?;

            let mut builder = reqwest::Client::new().delete(&url);

            if let Some(options) = args.get(1) {
                for (k, v) in extract_headers(options) {
                    builder = builder.header(&k, &v);
                }
                if let Some(timeout) = extract_timeout(options) {
                    builder = builder.timeout(timeout);
                }
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

            Ok(ValueWord::from_ok(build_response(status, headers, body)))
        },
        ModuleFunction {
            description: "Perform an HTTP DELETE request".to_string(),
            params: vec![url_param, options_param],
            return_type: Some("Result<HttpResponse>".to_string()),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_module_creation() {
        let module = create_http_module();
        assert_eq!(module.name, "std::core::http");
        assert!(module.has_export("get"));
        assert!(module.has_export("post"));
        assert!(module.has_export("put"));
        assert!(module.has_export("delete"));
    }

    #[test]
    fn test_http_all_async() {
        let module = create_http_module();
        assert!(module.is_async("get"));
        assert!(module.is_async("post"));
        assert!(module.is_async("put"));
        assert!(module.is_async("delete"));
    }

    #[test]
    fn test_http_schemas() {
        let module = create_http_module();

        let get_schema = module.get_schema("get").unwrap();
        assert_eq!(get_schema.params.len(), 2);
        assert_eq!(get_schema.params[0].name, "url");
        assert!(get_schema.params[0].required);
        assert!(!get_schema.params[1].required);
        assert_eq!(
            get_schema.return_type.as_deref(),
            Some("Result<HttpResponse>")
        );

        let post_schema = module.get_schema("post").unwrap();
        assert_eq!(post_schema.params.len(), 3);
        assert_eq!(post_schema.params[1].name, "body");

        let delete_schema = module.get_schema("delete").unwrap();
        assert_eq!(delete_schema.params.len(), 2);
    }

    #[test]
    fn test_build_response() {
        let resp = build_response(
            200,
            vec![("content-type".to_string(), "text/html".to_string())],
            "hello".to_string(),
        );
        let (keys, values, _) = resp.as_hashmap().expect("should be hashmap");

        // Find status
        let status_idx = keys
            .iter()
            .position(|k| k.as_str() == Some("status"))
            .unwrap();
        assert_eq!(values[status_idx].as_f64(), Some(200.0));

        // Find ok
        let ok_idx = keys.iter().position(|k| k.as_str() == Some("ok")).unwrap();
        assert_eq!(values[ok_idx].as_bool(), Some(true));

        // Find body
        let body_idx = keys
            .iter()
            .position(|k| k.as_str() == Some("body"))
            .unwrap();
        assert_eq!(values[body_idx].as_str(), Some("hello"));

        // Find headers
        let headers_idx = keys
            .iter()
            .position(|k| k.as_str() == Some("headers"))
            .unwrap();
        assert!(values[headers_idx].as_hashmap().is_some());
    }

    #[test]
    fn test_build_response_error_status() {
        let resp = build_response(404, vec![], "not found".to_string());
        let (keys, values, _) = resp.as_hashmap().expect("should be hashmap");

        let ok_idx = keys.iter().position(|k| k.as_str() == Some("ok")).unwrap();
        assert_eq!(values[ok_idx].as_bool(), Some(false));
    }

    #[test]
    fn test_extract_headers_from_options() {
        let hk = vec![ValueWord::from_string(Arc::new(
            "Authorization".to_string(),
        ))];
        let hv = vec![ValueWord::from_string(Arc::new(
            "Bearer token123".to_string(),
        ))];
        let headers_map = ValueWord::from_hashmap_pairs(hk, hv);

        let ok = vec![ValueWord::from_string(Arc::new("headers".to_string()))];
        let ov = vec![headers_map];
        let options = ValueWord::from_hashmap_pairs(ok, ov);

        let extracted = extract_headers(&options);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].0, "Authorization");
        assert_eq!(extracted[0].1, "Bearer token123");
    }

    #[test]
    fn test_extract_timeout_from_options() {
        let ok = vec![ValueWord::from_string(Arc::new("timeout".to_string()))];
        let ov = vec![ValueWord::from_f64(5000.0)];
        let options = ValueWord::from_hashmap_pairs(ok, ov);

        let timeout = extract_timeout(&options);
        assert_eq!(timeout, Some(std::time::Duration::from_millis(5000)));
    }

    #[test]
    fn test_extract_timeout_none() {
        let options = ValueWord::empty_hashmap();
        assert_eq!(extract_timeout(&options), None);
    }
}
