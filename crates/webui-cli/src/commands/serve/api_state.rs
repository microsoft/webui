// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared acquisition policy for document and partial-render API state.

use actix_web::http::StatusCode;
use clap::ValueEnum;
use serde_json::Value;
use std::time::Duration;

/// How failures acquiring render state from the API backend are handled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum ApiStateErrors {
    /// Preserve permissive extraction and let the caller use file-state fallback.
    #[default]
    Fallback,
    /// Require a successful response containing object-valued render state.
    Strict,
}

/// Bound strict buffered-body reads, matching the existing request timeout.
pub(super) const BODY_TIMEOUT: Duration = Duration::from_secs(5);

/// Create a state client that preserves non-success responses in strict mode.
///
/// Fallback retains the default redirect-following behavior.
#[must_use]
pub(super) fn client(mode: ApiStateErrors) -> awc::Client {
    if mode == ApiStateErrors::Strict {
        awc::Client::builder().disable_redirects().finish()
    } else {
        awc::Client::new()
    }
}

/// An API acquisition failure, before a renderer commits its response.
#[derive(Debug, thiserror::Error)]
pub(super) enum ApiStateError {
    /// The backend could not provide HTTP response headers.
    #[error(
        "api-state-request: API request failed: {0}\nhelp: Check that the backend is listening on --api-port and responds promptly."
    )]
    Request(String),
    /// Strict acquisition rejected the backend HTTP status.
    #[error(
        "api-state-status: API returned HTTP {0}\nhelp: Fix the backend route to return a 2xx response containing a JSON state object."
    )]
    Status(StatusCode),
    /// The response body was incomplete, oversized, or timed out.
    #[error(
        "api-state-body: API body could not be read: {0}\nhelp: Check that the backend completes its response promptly within the body-size limit."
    )]
    Body(String),
    /// The response body was not valid JSON.
    #[error(
        "api-state-json: API response is not valid JSON: {0}\nhelp: Return a JSON object or an envelope with an object-valued state member."
    )]
    Json(String),
    /// The selected state was not an object.
    #[error(
        "api-state-shape: API render state is not a JSON object\nhelp: Return a bare JSON object or set the envelope's state member to an object."
    )]
    InvalidState,
}

/// Reject non-success statuses only when strict acquisition is enabled.
pub(super) fn validate_status(
    mode: ApiStateErrors,
    status: StatusCode,
) -> Result<(), ApiStateError> {
    if mode == ApiStateErrors::Strict && !status.is_success() {
        return Err(ApiStateError::Status(status));
    }
    Ok(())
}

/// Parse JSON and move the selected state out of its optional envelope.
///
/// Fallback preserves legacy extraction, including non-object state values.
/// Strict never treats an invalid `state` member as a bare-object response.
pub(super) fn parse(mode: ApiStateErrors, body: &[u8]) -> Result<Value, ApiStateError> {
    let mut json: Value = serde_json::from_slice(body).map_err(json_error)?;
    let state = if let Value::Object(object) = &mut json {
        object.remove("state").unwrap_or(json)
    } else {
        json
    };
    if mode == ApiStateErrors::Strict && !state.is_object() {
        return Err(ApiStateError::InvalidState);
    }
    Ok(state)
}

/// Fetch JSON state from the loopback backend without rewriting the request target.
///
/// Callers own file-state fallback; this helper always returns acquisition errors.
pub(super) async fn fetch(
    mode: ApiStateErrors,
    api_port: u16,
    path: &str,
) -> Result<Value, ApiStateError> {
    fetch_with_timeout(mode, api_port, path, BODY_TIMEOUT).await
}

async fn fetch_with_timeout(
    mode: ApiStateErrors,
    api_port: u16,
    path: &str,
    timeout: Duration,
) -> Result<Value, ApiStateError> {
    use std::fmt::Write;

    let client = client(mode);
    let mut url = String::with_capacity("http://127.0.0.1:65535".len() + path.len());
    url.push_str("http://127.0.0.1:");
    let _ = write!(url, "{api_port}");
    url.push_str(path);
    let response = client
        .get(&url)
        .insert_header(("Accept", "application/json"))
        .timeout(timeout)
        .send()
        .await
        .map_err(request_error)?;
    validate_status(mode, response.status())?;
    let mut response = if mode == ApiStateErrors::Strict {
        response.timeout(timeout)
    } else {
        response
    };
    let body = response.body().await.map_err(body_error)?;
    parse(mode, &body)
}

#[cold]
#[inline(never)]
fn request_error(error: awc::error::SendRequestError) -> ApiStateError {
    ApiStateError::Request(error.to_string())
}

#[cold]
#[inline(never)]
fn body_error(error: awc::error::PayloadError) -> ApiStateError {
    ApiStateError::Body(error.to_string())
}

#[cold]
#[inline(never)]
fn json_error(error: serde_json::Error) -> ApiStateError {
    ApiStateError::Json(error.to_string())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
    use bytes::Bytes;
    use serde_json::json;
    use std::net::TcpListener;
    use tokio_stream::StreamExt;

    #[test]
    fn policy_defaults_to_fallback_and_has_exact_cli_values() {
        assert_eq!(ApiStateErrors::default(), ApiStateErrors::Fallback);
        assert_eq!(
            ApiStateErrors::value_variants(),
            &[ApiStateErrors::Fallback, ApiStateErrors::Strict]
        );
        for (name, expected) in [
            ("fallback", ApiStateErrors::Fallback),
            ("strict", ApiStateErrors::Strict),
        ] {
            assert_eq!(ApiStateErrors::from_str(name, false).unwrap(), expected);
        }
        assert!(ApiStateErrors::from_str("lenient", false).is_err());
    }

    #[test]
    fn every_success_status_is_accepted_and_only_strict_rejects_other_statuses() {
        for code in 100..=599 {
            let status = StatusCode::from_u16(code).unwrap();
            assert!(validate_status(ApiStateErrors::Fallback, status).is_ok());
            match validate_status(ApiStateErrors::Strict, status) {
                Ok(()) => assert!((200..300).contains(&code)),
                Err(ApiStateError::Status(actual)) => {
                    assert!(!(200..300).contains(&code));
                    assert_eq!(actual, status);
                }
                Err(error) => panic!("unexpected status error: {error}"),
            }
        }
    }

    #[test]
    fn both_modes_accept_bare_and_enveloped_objects() {
        for mode in ApiStateErrors::value_variants() {
            for (body, expected) in [
                (r#"{}"#, json!({})),
                (r#"{"title":"hello"}"#, json!({"title":"hello"})),
                (r#"{"state":{}}"#, json!({})),
                (
                    r#"{"state":{"title":"hello"},"metadata":"ignored"}"#,
                    json!({"title":"hello"}),
                ),
            ] {
                assert_eq!(parse(*mode, body.as_bytes()).unwrap(), expected);
            }
        }
    }

    #[test]
    fn strict_rejects_nonobjects_and_invalid_envelopes_without_changing_fallback() {
        for state in [
            json!(null),
            json!(false),
            json!(42),
            json!("text"),
            json!([]),
        ] {
            let body = serde_json::to_vec(&state).unwrap();
            let envelope = serde_json::to_vec(&json!({"state": state, "valid": true})).unwrap();
            for body in [&body, &envelope] {
                assert!(matches!(
                    parse(ApiStateErrors::Strict, body),
                    Err(ApiStateError::InvalidState)
                ));
                assert_eq!(parse(ApiStateErrors::Fallback, body).unwrap(), state);
            }
        }
    }

    #[test]
    fn both_modes_reject_empty_malformed_and_non_utf8_json() {
        for body in [
            b"".as_slice(),
            b" \r\n\t",
            b"{",
            b"<html>backend error</html>",
            b"{} trailing",
            b"{\"state\":\xff}",
        ] {
            for mode in ApiStateErrors::value_variants() {
                assert!(matches!(parse(*mode, body), Err(ApiStateError::Json(_))));
            }
        }
    }

    #[test]
    fn errors_have_stable_plain_codes_and_actionable_help() {
        for (error, code) in [
            (
                ApiStateError::Request("connection refused".into()),
                "api-state-request",
            ),
            (
                ApiStateError::Status(StatusCode::BAD_GATEWAY),
                "api-state-status",
            ),
            (
                ApiStateError::Body("incomplete body".into()),
                "api-state-body",
            ),
            (
                ApiStateError::Json("unexpected EOF".into()),
                "api-state-json",
            ),
            (ApiStateError::InvalidState, "api-state-shape"),
        ] {
            let message = error.to_string();
            assert!(message.starts_with(code));
            assert!(message.contains("\nhelp: "));
            assert!(!message.contains('\x1b'));
        }
    }

    fn start_backend() -> (u16, actix_web::dev::ServerHandle) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = HttpServer::new(|| App::new().default_service(web::to(backend)))
            .workers(1)
            .listen(listener)
            .unwrap()
            .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        (port, handle)
    }

    async fn backend(request: HttpRequest) -> HttpResponse {
        let path = request.path();
        if let Some(code) = path.strip_prefix("/status/") {
            let status = StatusCode::from_u16(code.parse().unwrap()).unwrap();
            let state = json!({"target": request.uri().to_string()});
            return HttpResponse::build(status).json(if request.query_string() == "bare" {
                state
            } else {
                json!({"state": state})
            });
        }
        match path {
            "/redirect" => HttpResponse::Found()
                .insert_header(("location", "/status/200"))
                .finish(),
            "/invalid" => HttpResponse::Ok().body("{"),
            "/empty" => HttpResponse::Ok().finish(),
            "/shape" => HttpResponse::Ok().json(json!({"state": null})),
            "/invalid-encoding" => HttpResponse::Ok()
                .insert_header(("content-encoding", "gzip"))
                .body("not gzip"),
            "/slow-headers" => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                HttpResponse::Ok().json(json!({}))
            }
            "/broken-body" | "/slow-body" => {
                let broken = path == "/broken-body";
                let stream = tokio_stream::iter([0, 1]).then(move |index| async move {
                    if index == 0 {
                        return Ok(Bytes::from_static(b"{"));
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    if broken {
                        Err(std::io::Error::other("backend body interrupted"))
                    } else {
                        Ok(Bytes::from_static(b"}"))
                    }
                });
                if broken {
                    HttpResponse::Ok().no_chunking(100).streaming(stream)
                } else {
                    HttpResponse::Ok().streaming(stream)
                }
            }
            _ => HttpResponse::Ok().json(json!({
                "state": {
                    "target": request.uri().to_string(),
                    "accept": request.headers().get("accept").unwrap().to_str().unwrap()
                }
            })),
        }
    }

    #[actix_web::test]
    async fn fetch_accepts_success_objects_and_preserves_raw_targets() {
        let (port, handle) = start_backend();
        for mode in ApiStateErrors::value_variants() {
            for path in [
                "/projects/WebUI%20Fixture?x=space%20value&x=%2f&plus=+",
                "/reviews/ceo%2Fbranch?literal=%25&utf8=Montr%C3%A9al",
                "/?raw=%2F%2f&empty=",
            ] {
                let state = fetch(*mode, port, path).await.unwrap();
                assert_eq!(state["target"], path);
                assert_eq!(state["accept"], "application/json");
            }
            for code in [200, 201, 202, 206, 299] {
                for shape in ["bare", "envelope"] {
                    let path = format!("/status/{code}?{shape}");
                    let state = fetch(*mode, port, &path).await.unwrap();
                    assert_eq!(state, json!({"target": path}));
                }
            }
        }
        handle.stop(true).await;
    }

    #[actix_web::test]
    async fn fetch_keeps_redirects_visible_in_strict_mode() {
        let (port, handle) = start_backend();
        assert!(matches!(
            fetch(ApiStateErrors::Strict, port, "/redirect").await,
            Err(ApiStateError::Status(StatusCode::FOUND))
        ));
        assert_eq!(
            fetch(ApiStateErrors::Fallback, port, "/redirect")
                .await
                .unwrap(),
            json!({"target": "/status/200"})
        );
        handle.stop(true).await;
    }

    #[actix_web::test]
    async fn fetch_strict_rejects_non_success_without_changing_fallback_status_ignoring() {
        let (port, handle) = start_backend();
        for code in [301, 304, 400, 401, 404, 500, 503] {
            let path = format!("/status/{code}");
            let error = fetch(ApiStateErrors::Strict, port, &path)
                .await
                .unwrap_err();
            assert!(matches!(error, ApiStateError::Status(status) if status.as_u16() == code));
            if code != 304 {
                let state = fetch(ApiStateErrors::Fallback, port, &path).await.unwrap();
                assert_eq!(state, json!({"target": path}));
            }
        }
        handle.stop(true).await;
    }

    #[actix_web::test]
    async fn fetch_rejects_empty_malformed_and_nonobject_state() {
        let (port, handle) = start_backend();
        for mode in ApiStateErrors::value_variants() {
            for path in ["/invalid", "/empty", "/status/204"] {
                let result = fetch(*mode, port, path).await;
                assert!(
                    matches!(result, Err(ApiStateError::Json(_))),
                    "{mode:?} {path}: {result:?}"
                );
            }
        }
        assert!(matches!(
            fetch(ApiStateErrors::Strict, port, "/shape").await,
            Err(ApiStateError::InvalidState)
        ));
        assert_eq!(
            fetch(ApiStateErrors::Fallback, port, "/shape")
                .await
                .unwrap(),
            Value::Null
        );
        handle.stop(true).await;
    }

    #[actix_web::test]
    async fn fetch_reports_network_body_and_timeout_failures() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let unavailable_port = listener.local_addr().unwrap().port();
        drop(listener);
        for mode in ApiStateErrors::value_variants() {
            assert!(matches!(
                fetch(*mode, unavailable_port, "/").await,
                Err(ApiStateError::Request(_))
            ));
        }
        let (port, handle) = start_backend();
        for mode in ApiStateErrors::value_variants() {
            let result = fetch(*mode, port, "/broken-body").await;
            assert!(
                matches!(result, Err(ApiStateError::Body(_) | ApiStateError::Json(_))),
                "{mode:?} body transport: {result:?}"
            );
            assert!(matches!(
                fetch(*mode, port, "/invalid-encoding").await,
                Err(ApiStateError::Body(_))
            ));
            assert!(matches!(
                fetch_with_timeout(*mode, port, "/slow-headers", Duration::from_millis(50)).await,
                Err(ApiStateError::Request(_))
            ));
        }
        assert!(matches!(
            fetch_with_timeout(
                ApiStateErrors::Strict,
                port,
                "/slow-body",
                Duration::from_millis(50)
            )
            .await,
            Err(ApiStateError::Body(_))
        ));
        assert_eq!(
            fetch_with_timeout(
                ApiStateErrors::Fallback,
                port,
                "/slow-body",
                Duration::from_millis(50)
            )
            .await
            .unwrap(),
            json!({})
        );
        handle.stop(true).await;
    }
}
