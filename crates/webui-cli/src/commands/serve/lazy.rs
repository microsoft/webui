// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{accept_media_q, fetch_api_state, rebuild_error_json_response, ServerContext};
use actix_web::{http::StatusCode, web, HttpRequest, HttpResponse};
use bytes::Bytes;
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use webui::{HandlerError, LazyOptions, LazyRequest, Protocol};

#[derive(Deserialize)]
struct Target {
    owner: String,
    id: String,
}

struct Snapshot {
    protocol: Arc<Protocol>,
    state: Arc<Value>,
    tokens: Option<Arc<HashMap<String, String>>>,
}

enum State {
    Shared(Arc<Value>),
    Owned(Value),
}

impl State {
    fn value(&self) -> &Value {
        match self {
            Self::Shared(value) => value,
            Self::Owned(value) => value,
        }
    }
}

struct StateTask(tokio::task::JoinHandle<Result<State, String>>);

impl StateTask {
    async fn value(mut self) -> Result<State, String> {
        (&mut self.0)
            .await
            .map_err(|error| format!("lazy state task failed: {error}"))?
    }
}

impl Drop for StateTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn snapshot(context: &ServerContext) -> Result<Snapshot, Box<HttpResponse>> {
    let state = context.state.lock().map_err(|_| {
        Box::new(failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "lazy-state-unavailable",
            "state lock poisoned",
        ))
    })?;
    if let Some(error) = &state.rebuild_error {
        return Err(Box::new(rebuild_error_json_response(error)));
    }
    let protocol = state.protocol.clone().ok_or_else(|| {
        Box::new(failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "lazy-protocol-unavailable",
            "no compiled protocol",
        ))
    })?;
    Ok(Snapshot {
        protocol,
        state: state
            .state_data
            .clone()
            .unwrap_or_else(|| Arc::new(Value::Object(serde_json::Map::new()))),
        tokens: state.token_css.clone(),
    })
}

fn request_options(req: &HttpRequest) -> Result<LazyOptions, Box<HttpResponse>> {
    fn header(req: &HttpRequest, name: &str) -> Result<String, Box<HttpResponse>> {
        req.headers().get(name).map_or(Ok(String::new()), |value| {
            value.to_str().map(str::to_owned).map_err(|_| {
                Box::new(failure(
                    StatusCode::BAD_REQUEST,
                    "lazy-invalid-header",
                    "resource headers must be valid text",
                ))
            })
        })
    }
    Ok(LazyOptions {
        inventory: header(req, "x-webui-inventory")?,
        style_inventory: header(req, "x-webui-style-inventory")?,
        build_id: header(req, "x-webui-build")?,
    })
}

fn target(req: &HttpRequest) -> Result<Target, Box<HttpResponse>> {
    web::Query::<Target>::from_query(req.query_string())
        .map(web::Query::into_inner)
        .map_err(|_| {
            Box::new(failure(
                StatusCode::BAD_REQUEST,
                "lazy-invalid-target",
                "provide ?owner=template&id=region",
            ))
        })
}

pub(super) async fn resources(req: HttpRequest, context: web::Data<ServerContext>) -> HttpResponse {
    let target = match target(&req) {
        Ok(target) => target,
        Err(error) => return *error,
    };
    let options = match request_options(&req) {
        Ok(options) => options,
        Err(error) => return *error,
    };
    let snapshot = match snapshot(&context) {
        Ok(snapshot) => snapshot,
        Err(error) => return *error,
    };
    match snapshot
        .protocol
        .render_lazy_resources(LazyRequest::new(&target.owner, &target.id).with_options(&options))
    {
        Ok(body) => HttpResponse::Ok()
            .content_type("application/json")
            .body(body),
        Err(error) => request_failure(error),
    }
}

pub(super) async fn state(req: HttpRequest, context: web::Data<ServerContext>) -> HttpResponse {
    let mode = match req
        .headers()
        .get("accept")
        .map(|value| value.to_str())
        .transpose()
    {
        Ok(value) => match delivery(value) {
            Some(mode) => mode,
            None => {
                return failure(
                    StatusCode::NOT_ACCEPTABLE,
                    "lazy-not-acceptable",
                    "accept application/json or application/x-ndjson",
                )
            }
        },
        Err(_) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "lazy-invalid-header",
                "Accept must be valid text",
            )
        }
    };
    let target = match target(&req) {
        Ok(target) => target,
        Err(error) => return *error,
    };
    let options = match request_options(&req) {
        Ok(options) => options,
        Err(error) => return *error,
    };
    let snapshot = match snapshot(&context) {
        Ok(snapshot) => snapshot,
        Err(error) => return *error,
    };
    let path = req
        .uri()
        .path_and_query()
        .map_or(req.path(), |path| path.as_str())
        .to_owned();
    let protocol = snapshot.protocol.clone();
    let task = StateTask(actix_web::rt::spawn(resolve_state(context, path, snapshot)));
    if mode == Delivery::Json {
        let state = match task.value().await {
            Ok(state) => state,
            Err(error) => return failure(StatusCode::BAD_GATEWAY, "lazy-state-failed", &error),
        };
        return match protocol.render_lazy_partial(
            state.value(),
            LazyRequest::new(&target.owner, &target.id).with_options(&options),
        ) {
            Ok(body) => HttpResponse::Ok()
                .content_type("application/json")
                .body(body),
            Err(error) => request_failure(error),
        };
    }
    // Let the data operation begin before serializing the metadata record.
    tokio::task::yield_now().await;
    let prepared = match protocol
        .prepare_lazy_resources(LazyRequest::new(&target.owner, &target.id).with_options(&options))
    {
        Ok(prepared) => prepared,
        Err(error) => return request_failure(error),
    };
    let mut metadata = prepared.json;
    metadata.push('\n');
    let first = stream::once(async move { Ok::<_, actix_web::Error>(Bytes::from(metadata)) });
    let last = stream::once(async move {
        let result = match task.value().await {
            Ok(state) => protocol
                .render_lazy_partial(
                    state.value(),
                    LazyRequest::new(&target.owner, &target.id).with_options(&prepared.options),
                )
                .map_err(|error| error.to_string()),
            Err(error) => Err(error),
        };
        let mut body = match result {
            Ok(body) => body.into_bytes(),
            Err(message) => {
                log::error!("Lazy state delivery failed: {message}");
                serde_json::to_vec(&ErrorRecord {
                    error: ErrorDetail {
                        code: "lazy-state-failed",
                        message: &message,
                    },
                })
                .map_err(actix_web::error::ErrorInternalServerError)?
            }
        };
        body.push(b'\n');
        Ok::<_, actix_web::Error>(Bytes::from(body))
    });
    HttpResponse::Ok()
        .content_type("application/x-ndjson")
        .streaming(first.chain(last))
}

async fn resolve_state(
    context: web::Data<ServerContext>,
    path: String,
    snapshot: Snapshot,
) -> Result<State, String> {
    let Some(port) = context.api_port else {
        return Ok(State::Shared(snapshot.state));
    };
    let mut state = fetch_api_state(port, &path).await?;
    if let Some(tokens) = &snapshot.tokens {
        webui_tokens::inject_token_css(&mut state, tokens);
    }
    if let Value::Object(values) = &mut state {
        values.insert(
            "basePath".into(),
            Value::String(context.base_path.as_deref().unwrap_or("/").into()),
        );
    }
    Ok(State::Owned(state))
}

#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    Json,
    Ndjson,
}

fn delivery(accept: Option<&str>) -> Option<Delivery> {
    let Some(accept) = accept.filter(|value| !value.trim().is_empty()) else {
        return Some(Delivery::Json);
    };
    let mut json: Option<f32> = None;
    let mut ndjson: Option<f32> = None;
    let mut wildcard: Option<f32> = None;
    for entry in accept.split(',') {
        let (media, parameters) = entry.split_once(';').unwrap_or((entry, ""));
        let quality = accept_media_q(parameters);
        if media.trim().eq_ignore_ascii_case("application/x-ndjson") {
            ndjson = Some(ndjson.map_or(quality, |previous| previous.max(quality)));
        } else if media.trim().eq_ignore_ascii_case("application/json") {
            json = Some(json.map_or(quality, |previous| previous.max(quality)));
        } else if media.trim() == "*/*" || media.trim().eq_ignore_ascii_case("application/*") {
            wildcard = Some(wildcard.map_or(quality, |previous| previous.max(quality)));
        }
    }
    let json = json.or(wildcard).unwrap_or(0.0);
    let ndjson = ndjson.unwrap_or(0.0);
    if ndjson > 0.0 && ndjson >= json {
        Some(Delivery::Ndjson)
    } else if json > 0.0 {
        Some(Delivery::Json)
    } else {
        None
    }
}

#[derive(Serialize)]
struct ErrorRecord<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
}

fn failure(status: StatusCode, code: &str, message: &str) -> HttpResponse {
    HttpResponse::build(status)
        .content_type("application/json")
        .json(ErrorRecord {
            error: ErrorDetail { code, message },
        })
}

fn request_failure(error: HandlerError) -> HttpResponse {
    if let HandlerError::LazyRequest(error) = error {
        let status = match error.code {
            "lazy-not-found" => StatusCode::NOT_FOUND,
            "lazy-build-mismatch" => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        return failure(status, error.code, &error.message);
    }
    log::error!("Lazy resource delivery failed: {error}");
    failure(
        StatusCode::INTERNAL_SERVER_ERROR,
        "lazy-render-failed",
        &error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test as actix_test, App};
    use webui_protocol::{ClientFragmentData, FragmentList, StateProjectionMode, WebUIProtocol};

    fn context() -> web::Data<ServerContext> {
        let root = "app-shell::lazy:details";
        let mut document =
            WebUIProtocol::new(HashMap::from([(root.into(), FragmentList::default())]));
        document.build_id = "fixture".into();
        document.client_fragments.insert(
            root.into(),
            ClientFragmentData {
                owner: "app-shell".into(),
                name: "details".into(),
                entry: true,
                template_json: r#"{"h":"<p>Body</p>","v":3}"#.into(),
                state_mode: Some(StateProjectionMode::Keys as i32),
                state_keys: vec!["message".into()],
                ..Default::default()
            },
        );
        let context = super::super::tests::test_route_context(Arc::new(Protocol::new(document)));
        context.state.lock().unwrap().state_data = Some(Arc::new(
            serde_json::from_str(r#"{"message":"fresh","unused":"not sent"}"#).unwrap(),
        ));
        context
    }

    #[test]
    fn ndjson_is_explicit_and_respects_quality_preferences() {
        assert_eq!(
            delivery(Some("application/x-ndjson, application/json")),
            Some(Delivery::Ndjson)
        );
        assert_eq!(
            delivery(Some("application/x-ndjson;q=0, application/json")),
            Some(Delivery::Json)
        );
        assert_eq!(
            delivery(Some("application/x-ndjson;q=0.2, application/json;q=0.8")),
            Some(Delivery::Json)
        );
        assert_eq!(delivery(None), Some(Delivery::Json));
        assert_eq!(delivery(Some("application/json;q=0,*/*")), None);
        assert_eq!(delivery(Some("image/png")), None);
    }

    #[test]
    fn static_state_snapshot_borrows_the_existing_state_tree() {
        let context = context();
        let original = context.state.lock().unwrap().state_data.clone().unwrap();
        let snapshot = snapshot(&context).ok().unwrap();
        assert!(Arc::ptr_eq(&original, &snapshot.state));
    }

    #[actix_web::test]
    async fn resource_endpoint_decodes_names_and_returns_no_state() {
        let app = actix_test::init_service(
            App::new()
                .app_data(context())
                .route("/_webui/lazy", web::get().to(resources)),
        )
        .await;
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/_webui/lazy?owner=app%2Dshell&id=details")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = actix_test::read_body_json(response).await;
        assert_eq!(payload["type"], "webui-lazy");
        assert!(payload.get("state").is_none());
        assert!(payload["templates"]["app-shell::lazy:details"].is_object());
    }

    #[actix_web::test]
    async fn ndjson_state_uses_metadata_acknowledgement_without_resending_body() {
        let app = actix_test::init_service(
            App::new()
                .app_data(context())
                .route("/_webui/lazy/state", web::get().to(state)),
        )
        .await;
        let response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/_webui/lazy/state?owner=app-shell&id=details")
                .insert_header(("Accept", "application/x-ndjson, application/json"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = actix_test::read_body(response).await;
        let records: Vec<Value> = body
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert!(records[0].get("state").is_none());
        assert!(records[1].get("templates").is_none());
        assert_eq!(records[1]["state"]["message"], "fresh");
        assert!(records[1]["state"].get("unused").is_none());
    }

    #[actix_web::test]
    async fn stale_build_unknown_region_and_bad_inventory_have_error_statuses() {
        let app = actix_test::init_service(
            App::new()
                .app_data(context())
                .route("/_webui/lazy", web::get().to(resources)),
        )
        .await;
        for (id, build, inventory, status) in [
            ("details", "old", "", StatusCode::CONFLICT),
            ("missing", "fixture", "", StatusCode::NOT_FOUND),
            ("details", "fixture", "zz", StatusCode::BAD_REQUEST),
        ] {
            let response = actix_test::call_service(
                &app,
                actix_test::TestRequest::get()
                    .uri(&format!("/_webui/lazy?owner=app-shell&id={id}"))
                    .insert_header(("X-WebUI-Build", build))
                    .insert_header(("X-WebUI-Inventory", inventory))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), status);
        }
    }
}
