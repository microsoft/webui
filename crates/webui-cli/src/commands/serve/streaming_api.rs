// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-control stream consumed from a `webui serve --api-port` backend.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::header::{HeaderMap, CONTENT_TYPE};
use actix_web::HttpResponse;
use bytes::{Bytes, BytesMut};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{Stream, StreamExt};
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{BoundaryId, BoundaryMode, HandlerError, Protocol, RenderOptions, StreamingResponse};

use super::create_handler;
use crate::commands::common::Plugin;

pub(super) const MEDIA_TYPE: &str = "application/x-webui-stream";
pub(super) const ACCEPT: &str = "application/x-webui-stream, application/json";

const VERSION: u8 = 1;
const MAX_RECORD_BYTES: usize = 2_000_000;
const INITIAL_RECORD_CAPACITY: usize = 4 * 1024;

pub(super) struct StateDefaults {
    token_css: Option<HashMap<String, String>>,
    base_path: String,
    route_params: HashMap<String, String>,
}

impl StateDefaults {
    pub(super) fn new(
        token_css: Option<HashMap<String, String>>,
        base_path: String,
        route_params: HashMap<String, String>,
    ) -> Self {
        Self {
            token_css,
            base_path,
            route_params,
        }
    }

    pub(super) fn apply(&self, state: &mut Value) {
        if let Some(token_css) = &self.token_css {
            webui_tokens::inject_token_css(state, token_css);
        }
        let Value::Object(map) = state else {
            return;
        };
        map.insert("basePath".to_owned(), Value::String(self.base_path.clone()));
        for (key, value) in &self.route_params {
            map.insert(key.clone(), Value::String(value.clone()));
        }
    }

    fn apply_record(&self, state: &mut Value, record: usize) -> Result<(), ApiStreamError> {
        if !state.is_object() {
            return Err(ApiStreamError::StateMustBeObject { record });
        }
        self.apply(state);
        Ok(())
    }
}

pub(super) struct RenderConfig {
    pub(super) protocol: Arc<Protocol>,
    pub(super) entry: String,
    pub(super) route_path: String,
    pub(super) plugin: Option<Plugin>,
    pub(super) body_inject: Option<Arc<str>>,
    pub(super) chunk_pool: Arc<ChunkPool>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum ApiStreamCommand {
    Shell {
        version: u8,
        state: Value,
    },
    Boundary {
        name: String,
        #[serde(default)]
        mode: ApiBoundaryMode,
        state: Option<Value>,
    },
    Update {
        name: String,
        state: Value,
    },
    Finish {
        state: Option<Value>,
    },
}

impl ApiStreamCommand {
    fn prepare(&mut self, defaults: &StateDefaults, record: usize) -> Result<(), ApiStreamError> {
        match self {
            Self::Shell { state, .. } => defaults.apply_record(state, record),
            Self::Boundary {
                state: Some(state), ..
            }
            | Self::Finish { state: Some(state) } => defaults.apply_record(state, record),
            Self::Update { state, .. } if !state.is_object() => {
                Err(ApiStreamError::StateMustBeObject { record })
            }
            Self::Boundary { state: None, .. }
            | Self::Finish { state: None }
            | Self::Update { .. } => Ok(()),
        }
    }

    fn shell_version(&self) -> Option<u8> {
        match self {
            Self::Shell { version, .. } => Some(*version),
            _ => None,
        }
    }

    fn is_finish(&self) -> bool {
        matches!(self, Self::Finish { .. })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ApiBoundaryMode {
    #[default]
    Final,
    Updatable,
}

impl From<ApiBoundaryMode> for BoundaryMode {
    fn from(value: ApiBoundaryMode) -> Self {
        match value {
            ApiBoundaryMode::Final => Self::Final,
            ApiBoundaryMode::Updatable => Self::Updatable,
        }
    }
}

#[derive(Debug, Error)]
enum ApiStreamError {
    #[error(
        "WebUI API stream record {record} exceeds the {MAX_RECORD_BYTES}-byte limit; reduce the state payload"
    )]
    RecordTooLarge { record: usize },
    #[error("WebUI API stream record {record} is not valid JSON: {source}")]
    InvalidJson {
        record: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("WebUI API stream record {record} must contain an object-valued state")]
    StateMustBeObject { record: usize },
    #[error("WebUI API stream record {record} must be the initial shell record")]
    MissingShell { record: usize },
    #[error("WebUI API stream record {record} repeats the shell record")]
    DuplicateShell { record: usize },
    #[error(
        "WebUI API stream record {record} uses unsupported version {version}; send version {VERSION}"
    )]
    UnsupportedVersion { record: usize, version: u8 },
    #[error("WebUI API stream ended before a finish record; send {{\"type\":\"finish\"}}")]
    MissingFinish,
    #[error("WebUI API stream transport failed: {0}")]
    Backend(String),
    #[error("WebUI API stream renderer stopped before accepting record {record}")]
    RendererStopped { record: usize },
}

struct RecordDecoder {
    bytes: BytesMut,
    next_record: usize,
    search_from: usize,
}

impl RecordDecoder {
    fn new() -> Self {
        Self {
            bytes: BytesMut::with_capacity(INITIAL_RECORD_CAPACITY),
            next_record: 0,
            search_from: 0,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
    }

    fn next(&mut self) -> Result<Option<Bytes>, ApiStreamError> {
        let Some(relative_end) = memchr::memchr(b'\n', &self.bytes[self.search_from..]) else {
            self.search_from = self.bytes.len();
            self.ensure_pending_limit()?;
            return Ok(None);
        };
        let end = self.search_from + relative_end;
        self.ensure_line_limit(end)?;
        let mut line = self.bytes.split_to(end + 1);
        line.truncate(end);
        self.next_record += 1;
        self.search_from = 0;
        Ok(Some(line.freeze()))
    }

    fn finish(&mut self) -> Result<Option<Bytes>, ApiStreamError> {
        if self.bytes.is_empty() {
            return Ok(None);
        }
        self.ensure_pending_limit()?;
        self.next_record += 1;
        Ok(Some(self.bytes.split().freeze()))
    }

    fn ensure_pending_limit(&self) -> Result<(), ApiStreamError> {
        self.ensure_line_limit(self.bytes.len())
    }

    fn ensure_line_limit(&self, length: usize) -> Result<(), ApiStreamError> {
        if length > MAX_RECORD_BYTES {
            return Err(record_too_large(self.next_record));
        }
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn record_too_large(record: usize) -> ApiStreamError {
    ApiStreamError::RecordTooLarge { record }
}

struct CommandIngest {
    sender: mpsc::Sender<ApiStreamCommand>,
    state_defaults: StateDefaults,
    record: usize,
    shell_seen: bool,
}

impl CommandIngest {
    async fn dispatch(&mut self, bytes: &[u8]) -> Result<bool, ApiStreamError> {
        let record = self.record;
        self.record += 1;
        let mut command = serde_json::from_slice::<ApiStreamCommand>(bytes)
            .map_err(|source| ApiStreamError::InvalidJson { record, source })?;
        self.validate_order(&command, record)?;
        command.prepare(&self.state_defaults, record)?;
        let finished = command.is_finish();
        self.sender
            .send(command)
            .await
            .map_err(|_| ApiStreamError::RendererStopped { record })?;
        Ok(finished)
    }

    fn validate_order(
        &mut self,
        command: &ApiStreamCommand,
        record: usize,
    ) -> Result<(), ApiStreamError> {
        if let Some(version) = command.shell_version() {
            if self.shell_seen {
                return Err(duplicate_shell(record));
            }
            if version != VERSION {
                return Err(unsupported_version(record, version));
            }
            self.shell_seen = true;
            return Ok(());
        }
        if !self.shell_seen {
            return Err(missing_shell(record));
        }
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn duplicate_shell(record: usize) -> ApiStreamError {
    ApiStreamError::DuplicateShell { record }
}

#[cold]
#[inline(never)]
fn missing_shell(record: usize) -> ApiStreamError {
    ApiStreamError::MissingShell { record }
}

#[cold]
#[inline(never)]
fn unsupported_version(record: usize, version: u8) -> ApiStreamError {
    ApiStreamError::UnsupportedVersion { record, version }
}

pub(super) fn is_stream(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(MEDIA_TYPE))
}

pub(super) async fn render<S>(
    mut backend: S,
    config: RenderConfig,
    state_defaults: StateDefaults,
) -> HttpResponse
where
    S: Stream<Item = Result<Bytes, String>> + Unpin + 'static,
{
    let (command_tx, command_rx) = mpsc::channel(1);
    let (html_tx, html_rx) = mpsc::channel(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    let (ready_tx, ready_rx) = oneshot::channel();
    let render_route_path = config.route_path.clone();
    let ingest_route_path = config.route_path.clone();
    let mut decoder = RecordDecoder::new();
    let mut ingest = CommandIngest {
        sender: command_tx,
        state_defaults,
        record: 0,
        shell_seen: false,
    };

    if let Err(error) = ingest_first(&mut backend, &mut decoder, &mut ingest).await {
        log::error!("streaming API command stream failed for {ingest_route_path}: {error}");
        return HttpResponse::BadGateway()
            .content_type("text/plain; charset=utf-8")
            .body(error.to_string());
    }

    actix_web::rt::task::spawn_blocking(move || {
        let mut writer = StreamingWriter::new_pooled(html_tx, Arc::clone(&config.chunk_pool))
            .with_flush_timeout(Duration::from_secs(30));
        if let Err(error) = run_renderer(config, command_rx, &mut writer, ready_tx) {
            log::error!("streaming API render failed for {render_route_path}: {error}");
            // Drop the writer without `end()`: its pending buffer may contain a
            // partial HTML or JSON record. Already-flushed boundaries remain
            // valid, while closing the sender truncates the response at the
            // last semantic flush instead of publishing corrupt tail bytes.
        }
    });

    match ready_rx.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return HttpResponse::InternalServerError()
                .content_type("text/plain; charset=utf-8")
                .body(error);
        }
        Err(_) => {
            return HttpResponse::InternalServerError()
                .content_type("text/plain; charset=utf-8")
                .body("WebUI streaming renderer stopped during initialization");
        }
    }

    actix_web::rt::spawn(async move {
        if let Err(error) = ingest_remaining(backend, decoder, ingest).await {
            log::error!("streaming API command stream failed for {ingest_route_path}: {error}");
        }
    });

    let stream =
        tokio_stream::wrappers::ReceiverStream::new(html_rx).map(Ok::<Bytes, actix_web::Error>);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"))
        .streaming(stream)
}

fn run_renderer(
    config: RenderConfig,
    mut commands: mpsc::Receiver<ApiStreamCommand>,
    writer: &mut StreamingWriter,
    ready: oneshot::Sender<Result<(), String>>,
) -> Result<(), HandlerError> {
    let handler = create_handler(config.plugin);
    let options = RenderOptions::new(&config.entry, &config.route_path);
    let options = match config.body_inject.as_deref() {
        Some(body) => options.with_body_inject(body),
        None => options,
    };
    let mut response = match handler.stream_response(&config.protocol, &options, writer) {
        Ok(response) => response,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return Err(error);
        }
    };
    let mut boundaries = HashMap::with_capacity(response.boundary_count());
    let base_state = match commands.blocking_recv() {
        Some(ApiStreamCommand::Shell { state, .. }) => {
            if let Err(error) = response.write_shell(&state) {
                let _ = ready.send(Err(error.to_string()));
                return Err(error);
            }
            Some(state)
        }
        Some(_) => {
            let error = HandlerError::Invariant(
                "streaming API renderer did not receive the validated shell first".to_owned(),
            );
            let _ = ready.send(Err(error.to_string()));
            return Err(error);
        }
        None => {
            let error = HandlerError::Writer(
                "streaming API command producer stopped before shell rendering".to_owned(),
            );
            let _ = ready.send(Err(error.to_string()));
            return Err(error);
        }
    };
    let _ = ready.send(Ok(()));

    while let Some(command) = commands.blocking_recv() {
        match command {
            ApiStreamCommand::Shell { .. } => {
                return Err(HandlerError::Invariant(
                    "streaming API renderer received a duplicate shell".to_owned(),
                ));
            }
            ApiStreamCommand::Boundary { name, mode, state } => {
                let boundary = resolve_boundary(&response, &mut boundaries, name)?;
                let state = state.as_ref().or(base_state.as_ref()).ok_or_else(|| {
                    HandlerError::Invariant(
                        "streaming API boundary has no shell or explicit state".to_owned(),
                    )
                })?;
                response.write_boundary(boundary, state, mode.into())?;
            }
            ApiStreamCommand::Update { name, state } => {
                let boundary = resolve_boundary(&response, &mut boundaries, name)?;
                response.update(boundary, &state)?;
            }
            ApiStreamCommand::Finish { state } => {
                let state = state.as_ref().or(base_state.as_ref()).ok_or_else(|| {
                    HandlerError::Invariant(
                        "streaming API finish has no shell or explicit state".to_owned(),
                    )
                })?;
                return response.finish(state);
            }
        }
    }

    Err(HandlerError::Writer(
        "streaming API command producer stopped before finish".to_owned(),
    ))
}

fn resolve_boundary(
    response: &StreamingResponse<'_, StreamingWriter>,
    boundaries: &mut HashMap<String, BoundaryId>,
    name: String,
) -> Result<BoundaryId, HandlerError> {
    if let Some(boundary) = boundaries.get(&name) {
        return Ok(*boundary);
    }
    let boundary = response.boundary(&name)?;
    boundaries.insert(name, boundary);
    Ok(boundary)
}

async fn ingest_first<S>(
    backend: &mut S,
    decoder: &mut RecordDecoder,
    ingest: &mut CommandIngest,
) -> Result<(), ApiStreamError>
where
    S: Stream<Item = Result<Bytes, String>> + Unpin,
{
    loop {
        if let Some(record) = decoder.next()? {
            ingest.dispatch(&record).await?;
            return Ok(());
        }
        let Some(chunk) = backend.next().await else {
            if let Some(record) = decoder.finish()? {
                ingest.dispatch(&record).await?;
                return Ok(());
            }
            return Err(missing_shell(0));
        };
        decoder.push(&chunk.map_err(ApiStreamError::Backend)?);
    }
}

async fn ingest_remaining<S>(
    mut backend: S,
    mut decoder: RecordDecoder,
    mut ingest: CommandIngest,
) -> Result<(), ApiStreamError>
where
    S: Stream<Item = Result<Bytes, String>> + Unpin,
{
    while let Some(record) = decoder.next()? {
        if ingest.dispatch(&record).await? {
            return Ok(());
        }
    }
    while let Some(chunk) = backend.next().await {
        let chunk = chunk.map_err(ApiStreamError::Backend)?;
        decoder.push(&chunk);
        while let Some(record) = decoder.next()? {
            if ingest.dispatch(&record).await? {
                return Ok(());
            }
        }
    }

    if let Some(record) = decoder.finish()? {
        if ingest.dispatch(&record).await? {
            return Ok(());
        }
    }
    Err(ApiStreamError::MissingFinish)
}

#[cfg(test)]
async fn ingest<S>(
    mut backend: S,
    sender: mpsc::Sender<ApiStreamCommand>,
    state_defaults: StateDefaults,
) -> Result<(), ApiStreamError>
where
    S: Stream<Item = Result<Bytes, String>> + Unpin,
{
    let mut decoder = RecordDecoder::new();
    let mut ingest = CommandIngest {
        sender,
        state_defaults,
        record: 0,
        shell_seen: false,
    };
    ingest_first(&mut backend, &mut decoder, &mut ingest).await?;
    ingest_remaining(backend, decoder, ingest).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header::HeaderValue;
    use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

    fn defaults() -> StateDefaults {
        StateDefaults::new(None, "/".to_owned(), HashMap::new())
    }

    fn shell_render_failure_config() -> RenderConfig {
        fn structural(value: &str) -> WebUIFragment {
            let mut signal = String::with_capacity(value.len() + 9);
            signal.push_str("}}}webui:");
            signal.push_str(value);
            WebUIFragment::signal(signal, true)
        }

        let fragments = vec![
            WebUIFragment::raw("<html><head>"),
            structural("head_start"),
            structural("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural("body_start"),
            structural("streaming_root:outside-boundary"),
            structural("boundary_start:0"),
            WebUIFragment::raw("<main>ready</main>"),
            structural("boundary_end:0"),
            structural("body_end"),
            WebUIFragment::raw("</body></html>"),
        ];
        let document = WebUIProtocol::new(HashMap::from([(
            "index.html".to_owned(),
            FragmentList { fragments },
        )]));
        RenderConfig {
            protocol: Arc::new(Protocol::new(document)),
            entry: "index.html".to_owned(),
            route_path: "/".to_owned(),
            plugin: None,
            body_inject: None,
            chunk_pool: Arc::new(ChunkPool::new(
                StreamingWriter::DEFAULT_CHANNEL_CAPACITY,
                StreamingWriter::CHUNK_TARGET + 1024,
            )),
        }
    }

    fn next_record(decoder: &mut RecordDecoder) -> Option<Bytes> {
        decoder
            .next()
            .unwrap_or_else(|error| panic!("decode failed: {error}"))
    }

    #[test]
    fn decoder_reassembles_split_records() {
        let mut decoder = RecordDecoder::new();
        decoder.push(br#"{"type":"shell","#);
        assert!(next_record(&mut decoder).is_none());
        decoder.push(
            br#""state":{}}
{"type":"finish"}
"#,
        );

        assert_eq!(
            next_record(&mut decoder).unwrap_or_else(|| panic!("missing shell record")),
            br#"{"type":"shell","state":{}}"#[..]
        );
        assert_eq!(
            next_record(&mut decoder).unwrap_or_else(|| panic!("missing finish record")),
            br#"{"type":"finish"}"#[..]
        );
        assert!(next_record(&mut decoder).is_none());
        assert!(decoder
            .finish()
            .unwrap_or_else(|error| panic!("finish failed: {error}"))
            .is_none());
    }

    #[tokio::test]
    async fn command_stream_requires_shell_and_finish() {
        let chunks = tokio_stream::iter([Ok(Bytes::from_static(
            br#"{"type":"shell","version":1,"state":{}}
{"type":"boundary","name":"ready"}
{"type":"finish"}
"#,
        ))]);
        let (sender, mut receiver) = mpsc::channel(4);
        ingest(chunks, sender, defaults())
            .await
            .unwrap_or_else(|error| panic!("ingest failed: {error}"));

        assert!(matches!(
            receiver.recv().await,
            Some(ApiStreamCommand::Shell {
                version: VERSION,
                ..
            })
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(ApiStreamCommand::Boundary { .. })
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(ApiStreamCommand::Finish { .. })
        ));
    }

    #[tokio::test]
    async fn command_before_shell_is_rejected() {
        let chunks = tokio_stream::iter([Ok(Bytes::from_static(
            br#"{"type":"boundary","name":"ready"}
"#,
        ))]);
        let (sender, _receiver) = mpsc::channel(1);
        let error = ingest(chunks, sender, defaults())
            .await
            .err()
            .unwrap_or_else(|| panic!("command before shell was accepted"));
        assert!(matches!(error, ApiStreamError::MissingShell { record: 0 }));
    }

    #[tokio::test]
    async fn unsupported_stream_version_is_rejected() {
        let chunks = tokio_stream::iter([Ok(Bytes::from_static(
            br#"{"type":"shell","version":2,"state":{}}
"#,
        ))]);
        let (sender, _receiver) = mpsc::channel(1);
        let error = ingest(chunks, sender, defaults())
            .await
            .err()
            .unwrap_or_else(|| panic!("unsupported version was accepted"));
        assert!(matches!(
            error,
            ApiStreamError::UnsupportedVersion {
                record: 0,
                version: 2
            }
        ));
    }

    #[tokio::test]
    async fn empty_stream_is_rejected_before_rendering() {
        let chunks = tokio_stream::empty();
        let (sender, _receiver) = mpsc::channel(1);
        let error = ingest(chunks, sender, defaults())
            .await
            .err()
            .unwrap_or_else(|| panic!("empty stream was accepted"));
        assert!(matches!(error, ApiStreamError::MissingShell { record: 0 }));
    }

    #[actix_web::test]
    async fn shell_render_failure_is_reported_before_http_success() {
        let chunks = tokio_stream::iter([Ok::<Bytes, String>(Bytes::from_static(
            br#"{"type":"shell","version":1,"state":{}}
{"type":"finish"}
"#,
        ))]);

        let response = render(chunks, shell_render_failure_config(), defaults()).await;

        assert_eq!(
            response.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn stream_content_type_accepts_parameters_case_insensitively() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("Application/X-WebUI-Stream; charset=utf-8"),
        );
        assert!(is_stream(&headers));
    }

    #[test]
    fn decoder_rejects_an_oversized_record() {
        let mut decoder = RecordDecoder::new();
        decoder.push(&vec![b'x'; MAX_RECORD_BYTES + 1]);
        assert!(matches!(
            decoder.next(),
            Err(ApiStreamError::RecordTooLarge { record: 0 })
        ));
    }
}
