//! OpenAPI adapter — merge health paths and mount Swagger UI.

use axum::Router;

#[cfg(feature = "prometheus-metrics")]
use std::sync::Arc;

#[cfg(feature = "openapi")]
pub(crate) fn mount_openapi(
    router: Router,
    api: utoipa::openapi::OpenApi,
    spec_path: &str,
    ui_path: &str,
) -> Router {
    use utoipa_swagger_ui::SwaggerUi;
    router.merge(SwaggerUi::new(ui_path.to_string()).url(spec_path.to_string(), api))
}

#[cfg(feature = "openapi")]
pub(crate) fn merge_health_paths(
    mut api: utoipa::openapi::OpenApi,
    health_path: &str,
) -> utoipa::openapi::OpenApi {
    use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItem};
    use utoipa::openapi::response::ResponseBuilder;
    use utoipa::openapi::tag::Tag;

    let liveness_op = OperationBuilder::new()
        .operation_id(Some("liveness"))
        .tag("health")
        .summary(Some("Liveness probe"))
        .description(Some(
            "Returns `200 OK` when the process is alive. \
             Always succeeds as long as the server is running.",
        ))
        .response(
            "200",
            ResponseBuilder::new()
                .description("Service is alive")
                .build(),
        )
        .build();

    let readiness_op = OperationBuilder::new()
        .operation_id(Some("readiness"))
        .tag("health")
        .summary(Some("Readiness probe"))
        .description(Some(
            "Returns `200 OK` when all registered readiness checks pass, \
             `503 Service Unavailable` when any check fails.",
        ))
        .response(
            "200",
            ResponseBuilder::new()
                .description("Service is ready")
                .build(),
        )
        .response(
            "503",
            ResponseBuilder::new()
                .description("Service is not ready — one or more checks failed")
                .build(),
        )
        .build();

    let live_path = format!("{health_path}/live");
    let ready_path = format!("{health_path}/ready");

    api.paths
        .paths
        .entry(live_path)
        .and_modify(|item| {
            item.merge_operations(PathItem::new(HttpMethod::Get, liveness_op.clone()))
        })
        .or_insert_with(|| PathItem::new(HttpMethod::Get, liveness_op));

    api.paths
        .paths
        .entry(ready_path)
        .and_modify(|item| {
            item.merge_operations(PathItem::new(HttpMethod::Get, readiness_op.clone()))
        })
        .or_insert_with(|| PathItem::new(HttpMethod::Get, readiness_op));

    if !api
        .tags
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|t| t.name == "health")
    {
        let tags = api.tags.get_or_insert_with(Vec::new);
        tags.push(Tag::new("health"));
    }

    api
}

/// Rewrite OpenAPI 3.1 nullable type arrays to OpenAPI 3.0 `nullable: true` form.
#[cfg(feature = "openapi")]
pub(crate) fn rewrite_nullable_for_progenitor(json: String) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&json) else {
        return json;
    };
    if let Some(obj) = value.as_object_mut()
        && let Some(v) = obj.get("openapi")
        && v.as_str().map(|s| s.starts_with("3.1")).unwrap_or(false)
    {
        obj.insert(
            "openapi".to_string(),
            serde_json::Value::String("3.0.3".to_string()),
        );
    }
    if let Some(obj) = value.as_object_mut()
        && let Some(info) = obj.get_mut("info")
        && let Some(license) = info.as_object_mut().and_then(|o| o.get_mut("license"))
        && let Some(lic_obj) = license.as_object_mut()
    {
        lic_obj.remove("identifier");
    }
    rewrite_nullable_recursive(&mut value);
    serde_json::to_string_pretty(&value).unwrap_or(json)
}

#[cfg(feature = "openapi")]
fn rewrite_nullable_recursive(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let rewrite_type = if let Some(type_val) = map.get("type") {
                if let Some(arr) = type_val.as_array() {
                    let non_null: Vec<&str> = arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter(|s| *s != "null")
                        .collect();
                    let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
                    if non_null.len() == 1 {
                        Some((non_null[0].to_string(), has_null))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((t, has_null)) = rewrite_type {
                map.insert("type".to_string(), serde_json::Value::String(t));
                if has_null {
                    map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                }
            }

            for of_key in &["oneOf", "anyOf"] {
                let rewrite = if let Some(variants) = map.get(*of_key) {
                    if let Some(arr) = variants.as_array() {
                        let has_null_arm = arr.iter().any(|v| {
                            v.as_object()
                                .and_then(|o| o.get("type"))
                                .and_then(|t| t.as_str())
                                == Some("null")
                        });
                        if has_null_arm {
                            let remaining: Vec<serde_json::Value> = arr
                                .iter()
                                .filter(|v| {
                                    v.as_object()
                                        .and_then(|o| o.get("type"))
                                        .and_then(|t| t.as_str())
                                        != Some("null")
                                })
                                .cloned()
                                .collect();
                            Some(remaining)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(remaining) = rewrite {
                    map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    if remaining.len() == 1 {
                        let arm = remaining.into_iter().next().unwrap();
                        map.remove(*of_key);
                        if let serde_json::Value::Object(arm_map) = arm {
                            for (k, v) in arm_map {
                                map.entry(k).or_insert(v);
                            }
                        }
                    } else {
                        map.insert(of_key.to_string(), serde_json::Value::Array(remaining));
                    }
                    break;
                }
            }

            map.remove("propertyNames");

            if let Some(examples) = map.remove("examples")
                && let Some(arr) = examples.as_array()
                && let Some(first) = arr.first()
            {
                map.entry("example".to_string())
                    .or_insert_with(|| first.clone());
            }

            for v in map.values_mut() {
                rewrite_nullable_recursive(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                rewrite_nullable_recursive(v);
            }
        }
        _ => {}
    }
}

#[cfg(feature = "openapi")]
fn coerce_boolean_and_2020_schemas(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            let schema_keys = [
                "additionalProperties",
                "additionalItems",
                "items",
                "not",
                "if",
                "then",
                "else",
                "contains",
                "propertyNames",
            ];

            for key in &schema_keys {
                if let Some(&serde_json::Value::Bool(b)) = map.get(*key) {
                    if b {
                        map.insert(
                            key.to_string(),
                            serde_json::Value::Object(Default::default()),
                        );
                    } else {
                        map.remove(*key);
                    }
                }
            }

            map.remove("unevaluatedProperties");
            map.remove("unevaluatedItems");
            map.remove("$schema");

            for v in map.values_mut() {
                coerce_boolean_and_2020_schemas(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                coerce_boolean_and_2020_schemas(v);
            }
        }
        _ => {}
    }
}

#[cfg(feature = "openapi")]
fn strip_non_success_response_content(val: &mut serde_json::Value) {
    if let Some(paths) = val
        .as_object_mut()
        .and_then(|root| root.get_mut("paths"))
        .and_then(|p| p.as_object_mut())
    {
        for path_item in paths.values_mut() {
            if let serde_json::Value::Object(path_obj) = path_item {
                let http_methods = ["get", "post", "put", "patch", "delete", "head", "options"];
                for method in &http_methods {
                    if let Some(serde_json::Value::Object(operation)) = path_obj.get_mut(*method)
                        && let Some(serde_json::Value::Object(responses)) =
                            operation.get_mut("responses")
                    {
                        for (status_code, response) in responses.iter_mut() {
                            if status_code.starts_with('2') {
                                continue;
                            }
                            if let serde_json::Value::Object(resp_obj) = response {
                                resp_obj.remove("content");
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Serialize `api` as a valid OpenAPI **3.0.3** pretty-printed JSON string.
#[cfg(feature = "openapi")]
pub(crate) fn to_3_0_pretty_json(api: utoipa::openapi::OpenApi) -> serde_json::Result<String> {
    let json = serde_json::to_string_pretty(&api)?;
    let mut val: serde_json::Value = serde_json::from_str(&json)?;
    coerce_boolean_and_2020_schemas(&mut val);
    rewrite_nullable_recursive(&mut val);
    strip_non_success_response_content(&mut val);
    serde_json::to_string_pretty(&val)
}

/// Encode Prometheus metrics in text exposition format.
#[cfg(feature = "prometheus-metrics")]
pub(crate) async fn prometheus_scrape_handler(
    axum::extract::State(registry): axum::extract::State<Arc<prometheus::Registry>>,
) -> axum::response::Response {
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;
    use prometheus::{Encoder, TextEncoder};

    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    let mut buf = Vec::new();
    if let Err(e) = encoder.encode(&metric_families, &mut buf) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("prometheus encode failed: {e}"),
        )
            .into_response();
    }
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], buf).into_response()
}
