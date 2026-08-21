// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-control stream consumed from a `webui serve --api-port` backend.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use actix_web::http::header::{HeaderMap, CONTENT_TYPE};
use actix_web::HttpResponse;
use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{Stream, StreamExt};
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{
    BoundaryDescriptor, BoundaryInstanceId, BoundaryKey, BoundaryMode, HandlerError, Protocol,
    RenderOptions, StreamStatus, StreamingResponse,
};

use super::create_handler;
use crate::commands::common::Plugin;

pub(super) const MEDIA_TYPE: &str = "application/x-webui-stream";
pub(super) const ACCEPT: &str = "application/x-webui-stream, application/json";

const VERSION: u8 = 2;
const MAX_RECORD_BYTES: usize = 2_000_000;
const MAX_PRECOMMIT_BYTES: usize = 4_000_000;
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
    Start {
        version: u8,
        state: Value,
    },
    Resume {
        boundary: ApiBoundaryTarget,
        #[serde(default)]
        mode: ApiBoundaryMode,
        state: Value,
    },
    Update {
        boundary: ApiBoundaryTarget,
        state: Value,
    },
}

impl ApiStreamCommand {
    fn prepare(&mut self, defaults: &StateDefaults, record: usize) -> Result<(), ApiStreamError> {
        match self {
            Self::Start { state, .. } | Self::Resume { state, .. } => {
                defaults.apply_record(state, record)
            }
            Self::Update { state, .. } if !state.is_object() => {
                Err(ApiStreamError::StateMustBeObject { record })
            }
            Self::Update { .. } => Ok(()),
        }
    }

    fn start_version(&self) -> Option<u8> {
        match self {
            Self::Start { version, .. } => Some(*version),
            _ => None,
        }
    }

    const fn name(&self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::Resume { .. } => "resume",
            Self::Update { .. } => "update",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApiBoundaryTarget {
    owner: String,
    name: String,
    #[serde(default, deserialize_with = "deserialize_boundary_key")]
    key: Option<ApiBoundaryKey>,
    #[serde(default)]
    declaration_id: Option<u32>,
}

impl ApiBoundaryTarget {
    fn matches(&self, boundary: &BoundaryDescriptor) -> bool {
        self.owner.as_str() == boundary.owner.as_ref()
            && self.name.as_str() == boundary.name.as_ref()
            && self.key.matches(&boundary.key)
            && self
                .declaration_id
                .is_none_or(|id| id == boundary.declaration_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
enum ApiBoundaryKey {
    String(String),
    Number(serde_json::Number),
}

fn deserialize_boundary_key<'de, D>(deserializer: D) -> Result<Option<ApiBoundaryKey>, D::Error>
where
    D: Deserializer<'de>,
{
    ApiBoundaryKey::deserialize(deserializer).map(Some)
}

trait ApiBoundaryKeyOptionExt {
    fn matches(&self, key: &Option<BoundaryKey>) -> bool;
}

impl ApiBoundaryKeyOptionExt for Option<ApiBoundaryKey> {
    fn matches(&self, key: &Option<BoundaryKey>) -> bool {
        match (self, key) {
            (None, None) => true,
            (Some(ApiBoundaryKey::String(left)), Some(BoundaryKey::String(right))) => left == right,
            (Some(ApiBoundaryKey::Number(left)), Some(BoundaryKey::Number(right))) => left == right,
            _ => false,
        }
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
    #[error("WebUI API stream record {record} must be the initial start record")]
    MissingStart { record: usize },
    #[error("WebUI API stream record {record} repeats the start record")]
    DuplicateStart { record: usize },
    #[error(
        "WebUI API stream record {record} uses unsupported version {version}; send version {VERSION}"
    )]
    UnsupportedVersion { record: usize, version: u8 },
    #[error("WebUI API stream transport failed: {0}")]
    Backend(String),
    #[error("WebUI API stream renderer stopped before accepting record {record}")]
    RendererStopped { record: usize },
}

#[derive(Debug, Error)]
enum StreamInitializationError {
    #[error("{0}")]
    Render(String),
    #[error("WebUI streaming renderer stopped during start")]
    RendererStopped,
    #[error(
        "WebUI initial streaming output exceeds the 4,000,000-byte precommit limit; reduce the initial state or move content behind a boundary"
    )]
    TooLarge,
}

#[derive(Debug, Error)]
enum RendererError {
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error("WebUI API stream renderer expected the validated initial start record")]
    ExpectedStart,
    #[error("WebUI API stream renderer stopped before receiving the initial start record")]
    MissingStart,
    #[error("WebUI API stream record {record} repeats the start record")]
    DuplicateStart { record: usize },
    #[error(
        "WebUI API stream record {record} resumes {received}, but the current cursor is {expected}; echo the pending boundary owner, name, and typed key exactly"
    )]
    ResumeMismatch {
        record: usize,
        received: String,
        expected: String,
    },
    #[error(
        "WebUI API stream record {record} updates {target}, but no matching occurrence was committed as updatable; target an earlier resume with the same owner, name, and typed key"
    )]
    UpdateNotCommitted { record: usize, target: String },
    #[error(
        "WebUI API stream record {record} updates {target}, but the matching occurrence was committed as final; resume it with mode \"updatable\" before sending updates"
    )]
    UpdateFinal { record: usize, target: String },
    #[error(
        "WebUI API stream record {record} updates {target}, which matches multiple updatable occurrences; add a stable <boundary key> and include its typed value"
    )]
    AmbiguousUpdate { record: usize, target: String },
    #[error(
        "WebUI API stream ended while waiting to resume {pending}; send the matching resume record before closing the response body"
    )]
    Truncated { pending: String },
    #[error(
        "WebUI API stream record {record} sends {command} after the streaming response completed; close the response body after the final resume or update control"
    )]
    CommandAfterDone {
        record: usize,
        command: &'static str,
    },
    #[error("WebUI streaming session returned an unfinished step without a boundary descriptor")]
    MissingDescriptor,
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
    sender: mpsc::Sender<ApiStreamRecord>,
    state_defaults: StateDefaults,
    record: usize,
    start_seen: bool,
}

struct ApiStreamRecord {
    index: usize,
    command: ApiStreamCommand,
}

impl CommandIngest {
    async fn dispatch(&mut self, bytes: &[u8]) -> Result<(), ApiStreamError> {
        let record = self.record;
        self.record += 1;
        let mut command = serde_json::from_slice::<ApiStreamCommand>(bytes)
            .map_err(|source| ApiStreamError::InvalidJson { record, source })?;
        self.validate_order(&command, record)?;
        command.prepare(&self.state_defaults, record)?;
        self.sender
            .send(ApiStreamRecord {
                index: record,
                command,
            })
            .await
            .map_err(|_| ApiStreamError::RendererStopped { record })?;
        Ok(())
    }

    fn validate_order(
        &mut self,
        command: &ApiStreamCommand,
        record: usize,
    ) -> Result<(), ApiStreamError> {
        if let Some(version) = command.start_version() {
            if self.start_seen {
                return Err(duplicate_start(record));
            }
            if version != VERSION {
                return Err(unsupported_version(record, version));
            }
            self.start_seen = true;
            return Ok(());
        }
        if !self.start_seen {
            return Err(missing_start(record));
        }
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn duplicate_start(record: usize) -> ApiStreamError {
    ApiStreamError::DuplicateStart { record }
}

#[cold]
#[inline(never)]
fn missing_start(record: usize) -> ApiStreamError {
    ApiStreamError::MissingStart { record }
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

struct BrowserHtmlStream {
    initial: std::vec::IntoIter<Bytes>,
    live: tokio_stream::wrappers::ReceiverStream<Bytes>,
    cancel: Option<oneshot::Sender<()>>,
}

impl BrowserHtmlStream {
    fn new(initial: Vec<Bytes>, live: mpsc::Receiver<Bytes>, cancel: oneshot::Sender<()>) -> Self {
        Self {
            initial: initial.into_iter(),
            live: tokio_stream::wrappers::ReceiverStream::new(live),
            cancel: Some(cancel),
        }
    }

    fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

impl Stream for BrowserHtmlStream {
    type Item = Result<Bytes, actix_web::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(chunk) = this.initial.next() {
            return Poll::Ready(Some(Ok(chunk)));
        }
        match Pin::new(&mut this.live).poll_next(cx) {
            Poll::Ready(Some(chunk)) => Poll::Ready(Some(Ok(chunk))),
            Poll::Ready(None) => {
                this.cancel();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for BrowserHtmlStream {
    fn drop(&mut self) {
        self.cancel();
    }
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
    let initialization_route_path = config.route_path.clone();
    let mut decoder = RecordDecoder::new();
    let mut ingest = CommandIngest {
        sender: command_tx,
        state_defaults,
        record: 0,
        start_seen: false,
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

    let (initial_html, html_rx) = match stage_precommit_output(ready_rx, html_rx).await {
        Ok(output) => output,
        Err(error) => {
            log::error!(
                "streaming API initialization failed for {initialization_route_path}: {error}"
            );
            return HttpResponse::InternalServerError()
                .content_type("text/plain; charset=utf-8")
                .body(error.to_string());
        }
    };

    let (cancel_tx, cancel_rx) = oneshot::channel();
    actix_web::rt::spawn(async move {
        tokio::select! {
            result = ingest_remaining(backend, decoder, ingest) => {
                if let Err(error) = result {
                    log::error!(
                        "streaming API command stream failed for {ingest_route_path}: {error}"
                    );
                }
            }
            _ = cancel_rx => {}
        }
    });

    let stream = BrowserHtmlStream::new(initial_html, html_rx, cancel_tx);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header(("Cache-Control", "no-store"))
        .streaming(stream)
}

async fn stage_precommit_output(
    mut ready: oneshot::Receiver<Result<(), String>>,
    mut html: mpsc::Receiver<Bytes>,
) -> Result<(Vec<Bytes>, mpsc::Receiver<Bytes>), StreamInitializationError> {
    let mut initial = Vec::with_capacity(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    let mut total = 0usize;
    loop {
        tokio::select! {
            result = &mut ready => {
                validate_renderer_ready(result)?;
                while let Ok(chunk) = html.try_recv() {
                    stage_initial_chunk(&mut initial, &mut total, chunk)?;
                }
                return Ok((initial, html));
            }
            chunk = html.recv() => {
                let Some(chunk) = chunk else {
                    validate_renderer_ready(ready.await)?;
                    return Ok((initial, html));
                };
                stage_initial_chunk(&mut initial, &mut total, chunk)?;
            }
        }
    }
}

fn validate_renderer_ready(
    result: Result<Result<(), String>, oneshot::error::RecvError>,
) -> Result<(), StreamInitializationError> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(StreamInitializationError::Render(error)),
        Err(_) => Err(StreamInitializationError::RendererStopped),
    }
}

fn stage_initial_chunk(
    initial: &mut Vec<Bytes>,
    total: &mut usize,
    chunk: Bytes,
) -> Result<(), StreamInitializationError> {
    let next_total = total.saturating_add(chunk.len());
    if next_total > MAX_PRECOMMIT_BYTES {
        return Err(StreamInitializationError::TooLarge);
    }
    *total = next_total;
    initial.push(chunk);
    Ok(())
}

fn run_renderer(
    config: RenderConfig,
    mut commands: mpsc::Receiver<ApiStreamRecord>,
    writer: &mut StreamingWriter,
    ready: oneshot::Sender<Result<(), String>>,
) -> Result<(), RendererError> {
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
            return Err(error.into());
        }
    };
    let mut committed = Vec::with_capacity(8);
    let mut status = match commands.blocking_recv() {
        Some(ApiStreamRecord {
            command: ApiStreamCommand::Start { state, .. },
            ..
        }) => match response.start(&state) {
            Ok(status) => status,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return Err(error.into());
            }
        },
        Some(_) => {
            let error = RendererError::ExpectedStart;
            let _ = ready.send(Err(error.to_string()));
            return Err(error);
        }
        None => {
            let error = RendererError::MissingStart;
            let _ = ready.send(Err(error.to_string()));
            return Err(error);
        }
    };
    let _ = ready.send(Ok(()));

    loop {
        let Some(record) = commands.blocking_recv() else {
            advance_committed_cursor(&mut response, &mut status)?;
            if status.done {
                return Ok(());
            }
            return Err(RendererError::Truncated {
                pending: pending_descriptor(&status)?,
            });
        };
        if status.done {
            return Err(RendererError::CommandAfterDone {
                record: record.index,
                command: record.command.name(),
            });
        }

        match record.command {
            ApiStreamCommand::Start { .. } => {
                return Err(RendererError::DuplicateStart {
                    record: record.index,
                });
            }
            ApiStreamCommand::Resume {
                boundary,
                mode,
                state,
            } => {
                advance_committed_cursor(&mut response, &mut status)?;
                if status.done {
                    return Err(RendererError::CommandAfterDone {
                        record: record.index,
                        command: "resume",
                    });
                }
                let pending = status
                    .boundary
                    .as_ref()
                    .ok_or(RendererError::MissingDescriptor)?;
                if !boundary.matches(pending) {
                    return Err(RendererError::ResumeMismatch {
                        record: record.index,
                        received: format_target(&boundary),
                        expected: format_descriptor(pending),
                    });
                }
                let descriptor = pending.clone();
                let mode = BoundaryMode::from(mode);
                status = response.resume(descriptor.instance_id, &state, mode)?;
                committed.push(CommittedBoundary { descriptor, mode });
            }
            ApiStreamCommand::Update { boundary, state } => {
                let instance_id = resolve_update(&committed, &boundary, record.index)?;
                response.update(instance_id, &state)?;
            }
        }
    }
}

fn advance_committed_cursor(
    response: &mut StreamingResponse<'_, StreamingWriter>,
    status: &mut StreamStatus,
) -> Result<(), RendererError> {
    // A following resume (or backend EOF) is the implicit advance control.
    // Updates stay ahead of that point so they can target the just-committed
    // occurrence before its separately flushed parent segment is rendered.
    if !status.done && status.boundary.is_none() {
        *status = response.advance()?;
    }
    Ok(())
}

struct CommittedBoundary {
    descriptor: BoundaryDescriptor,
    mode: BoundaryMode,
}

fn resolve_update(
    committed: &[CommittedBoundary],
    target: &ApiBoundaryTarget,
    record: usize,
) -> Result<BoundaryInstanceId, RendererError> {
    let mut updatable = None;
    let mut final_match = false;
    for boundary in committed {
        if !target.matches(&boundary.descriptor) {
            continue;
        }
        if boundary.mode == BoundaryMode::Final {
            final_match = true;
            continue;
        }
        if updatable.is_some() {
            return Err(RendererError::AmbiguousUpdate {
                record,
                target: format_target(target),
            });
        }
        updatable = Some(boundary.descriptor.instance_id);
    }
    if let Some(instance_id) = updatable {
        return Ok(instance_id);
    }
    let target = format_target(target);
    if final_match {
        Err(RendererError::UpdateFinal { record, target })
    } else {
        Err(RendererError::UpdateNotCommitted { record, target })
    }
}

fn pending_descriptor(status: &webui::StreamStatus) -> Result<String, RendererError> {
    status
        .boundary
        .as_ref()
        .map(format_descriptor)
        .ok_or(RendererError::MissingDescriptor)
}

#[cold]
#[inline(never)]
fn format_target(target: &ApiBoundaryTarget) -> String {
    format_boundary_identity(
        &target.owner,
        &target.name,
        target.key.as_ref().map(ApiBoundaryKeyRef::from),
        target.declaration_id,
    )
}

#[cold]
#[inline(never)]
fn format_descriptor(boundary: &BoundaryDescriptor) -> String {
    format_boundary_identity(
        &boundary.owner,
        &boundary.name,
        boundary.key.as_ref().map(ApiBoundaryKeyRef::from),
        Some(boundary.declaration_id),
    )
}

enum ApiBoundaryKeyRef<'a> {
    String(&'a str),
    Number(&'a serde_json::Number),
}

impl<'a> From<&'a ApiBoundaryKey> for ApiBoundaryKeyRef<'a> {
    fn from(value: &'a ApiBoundaryKey) -> Self {
        match value {
            ApiBoundaryKey::String(value) => Self::String(value),
            ApiBoundaryKey::Number(value) => Self::Number(value),
        }
    }
}

impl<'a> From<&'a BoundaryKey> for ApiBoundaryKeyRef<'a> {
    fn from(value: &'a BoundaryKey) -> Self {
        match value {
            BoundaryKey::String(value) => Self::String(value),
            BoundaryKey::Number(value) => Self::Number(value),
        }
    }
}

fn format_boundary_identity(
    owner: &str,
    name: &str,
    key: Option<ApiBoundaryKeyRef<'_>>,
    declaration_id: Option<u32>,
) -> String {
    let key = match key {
        Some(ApiBoundaryKeyRef::String(value)) => format!("{value:?}"),
        Some(ApiBoundaryKeyRef::Number(value)) => value.to_string(),
        None => "<none>".to_owned(),
    };
    match declaration_id {
        Some(id) => {
            format!("boundary owner={owner:?}, name={name:?}, key={key}, declarationId={id}")
        }
        None => format!("boundary owner={owner:?}, name={name:?}, key={key}"),
    }
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
            return Err(missing_start(0));
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
        ingest.dispatch(&record).await?;
    }
    while let Some(chunk) = backend.next().await {
        let chunk = chunk.map_err(ApiStreamError::Backend)?;
        decoder.push(&chunk);
        while let Some(record) = decoder.next()? {
            ingest.dispatch(&record).await?;
        }
    }

    if let Some(record) = decoder.finish()? {
        ingest.dispatch(&record).await?;
    }
    Ok(())
}

#[cfg(test)]
async fn ingest<S>(
    mut backend: S,
    sender: mpsc::Sender<ApiStreamRecord>,
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
        start_seen: false,
    };
    ingest_first(&mut backend, &mut decoder, &mut ingest).await?;
    ingest_remaining(backend, decoder, ingest).await
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use actix_web::body::to_bytes;
    use actix_web::http::header::HeaderValue;
    use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

    fn defaults() -> StateDefaults {
        StateDefaults::new(None, "/".to_owned(), HashMap::new())
    }

    fn structural(value: &str) -> WebUIFragment {
        let mut signal = String::with_capacity(value.len() + 9);
        signal.push_str("}}}webui:");
        signal.push_str(value);
        WebUIFragment::signal(signal, true)
    }

    fn start_render_failure_config() -> RenderConfig {
        let fragments = vec![
            WebUIFragment::raw("<html><head>"),
            structural("head_start"),
            structural("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural("body_start"),
            structural("streaming_root:outside-boundary"),
            WebUIFragment::boundary(0, "index.html", "content", None),
            WebUIFragment::raw("<main>ready</main>"),
            WebUIFragment::boundary_end(0),
            structural("body_end"),
            WebUIFragment::raw("</body></html>"),
        ];
        let document = WebUIProtocol::new(HashMap::from([(
            "index.html".to_owned(),
            FragmentList {
                fragments,
                contains_boundary: true,
            },
        )]));
        render_config(document)
    }

    fn valid_streaming_config(
        precommit_fragments: Vec<WebUIFragment>,
        boundary_names: &[&str],
    ) -> RenderConfig {
        let mut fragments =
            Vec::with_capacity(precommit_fragments.len() + boundary_names.len() * 3 + 8);
        fragments.extend([
            WebUIFragment::raw("<html><head>"),
            structural("head_start"),
            structural("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural("body_start"),
        ]);
        fragments.extend(precommit_fragments);
        let mut records = HashMap::with_capacity(1);
        for (declaration_id, name) in boundary_names.iter().enumerate() {
            let declaration_id = u32::try_from(declaration_id)
                .unwrap_or_else(|_| panic!("test boundary ID does not fit u32"));
            fragments.push(WebUIFragment::boundary(
                declaration_id,
                "index.html",
                *name,
                None,
            ));
            fragments.push(WebUIFragment::raw(format!(
                "<main data-boundary=\"{name}\">ready</main>"
            )));
            fragments.push(WebUIFragment::boundary_end(declaration_id));
        }
        fragments.extend([
            WebUIFragment::raw("<footer data-page-tail>tail</footer>"),
            structural("body_end"),
            WebUIFragment::raw("</body></html>"),
        ]);
        records.insert(
            "index.html".to_owned(),
            FragmentList {
                fragments,
                contains_boundary: !boundary_names.is_empty(),
            },
        );
        let document = WebUIProtocol::new(records);
        render_config(document)
    }

    fn render_config(document: WebUIProtocol) -> RenderConfig {
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

    struct StalledBackend {
        start: Option<Bytes>,
        dropped: Option<oneshot::Sender<()>>,
    }

    impl Stream for StalledBackend {
        type Item = Result<Bytes, String>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match self.start.take() {
                Some(start) => Poll::Ready(Some(Ok(start))),
                None => Poll::Pending,
            }
        }
    }

    impl Drop for StalledBackend {
        fn drop(&mut self) {
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }

    fn next_record(decoder: &mut RecordDecoder) -> Option<Bytes> {
        decoder
            .next()
            .unwrap_or_else(|error| panic!("decode failed: {error}"))
    }

    fn target(owner: &str, name: &str) -> ApiBoundaryTarget {
        ApiBoundaryTarget {
            owner: owner.to_owned(),
            name: name.to_owned(),
            key: None,
            declaration_id: None,
        }
    }

    fn run_commands(
        config: RenderConfig,
        commands: Vec<ApiStreamCommand>,
    ) -> (Result<(), RendererError>, Vec<Bytes>) {
        let (command_tx, command_rx) = mpsc::channel(commands.len().max(1));
        for (index, command) in commands.into_iter().enumerate() {
            command_tx
                .try_send(ApiStreamRecord { index, command })
                .unwrap_or_else(|error| panic!("failed to stage test command: {error}"));
        }
        drop(command_tx);
        let (html_tx, mut html_rx) = mpsc::channel(32);
        let mut writer = StreamingWriter::new(html_tx);
        let (ready_tx, _ready_rx) = oneshot::channel();
        let result = run_renderer(config, command_rx, &mut writer, ready_tx);
        drop(writer);
        let mut segments = Vec::new();
        while let Ok(chunk) = html_rx.try_recv() {
            segments.push(chunk);
        }
        (result, segments)
    }

    fn join_segments(segments: &[Bytes]) -> Vec<u8> {
        let capacity = segments.iter().map(Bytes::len).sum();
        let mut bytes = Vec::with_capacity(capacity);
        for segment in segments {
            bytes.extend_from_slice(segment);
        }
        bytes
    }

    #[test]
    fn decoder_reassembles_split_records() {
        let mut decoder = RecordDecoder::new();
        decoder.push(br#"{"type":"start","#);
        assert!(next_record(&mut decoder).is_none());
        decoder.push(
            br#""state":{}}
{"type":"resume","boundary":{"owner":"index.html","name":"ready"},"state":{}}
"#,
        );

        assert_eq!(
            next_record(&mut decoder).unwrap_or_else(|| panic!("missing start record")),
            br#"{"type":"start","state":{}}"#[..]
        );
        assert_eq!(
            next_record(&mut decoder).unwrap_or_else(|| panic!("missing resume record")),
            br#"{"type":"resume","boundary":{"owner":"index.html","name":"ready"},"state":{}}"#[..]
        );
        assert!(next_record(&mut decoder).is_none());
        assert!(decoder
            .finish()
            .unwrap_or_else(|error| panic!("decoder finalization failed: {error}"))
            .is_none());
    }

    #[tokio::test]
    async fn command_stream_accepts_start_resume_and_update() {
        let chunks = tokio_stream::iter([Ok(Bytes::from_static(
            br#"{"type":"start","version":2,"state":{}}
{"type":"resume","boundary":{"owner":"index.html","name":"ready","declarationId":0},"mode":"updatable","state":{}}
{"type":"update","boundary":{"owner":"index.html","name":"ready"},"state":{"count":2}}
"#,
        ))]);
        let (sender, mut receiver) = mpsc::channel(4);
        ingest(chunks, sender, defaults())
            .await
            .unwrap_or_else(|error| panic!("ingest failed: {error}"));

        assert!(matches!(
            receiver.recv().await,
            Some(ApiStreamRecord {
                command: ApiStreamCommand::Start {
                    version: VERSION,
                    ..
                },
                index: 0
            })
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(ApiStreamRecord {
                command: ApiStreamCommand::Resume { .. },
                index: 1
            })
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(ApiStreamRecord {
                command: ApiStreamCommand::Update { .. },
                index: 2
            })
        ));
    }

    #[tokio::test]
    async fn command_before_start_is_rejected() {
        let chunks = tokio_stream::iter([Ok(Bytes::from_static(
            br#"{"type":"resume","boundary":{"owner":"index.html","name":"ready"},"state":{}}
"#,
        ))]);
        let (sender, _receiver) = mpsc::channel(1);
        let error = ingest(chunks, sender, defaults())
            .await
            .err()
            .unwrap_or_else(|| panic!("command before start was accepted"));
        assert!(matches!(error, ApiStreamError::MissingStart { record: 0 }));
    }

    #[tokio::test]
    async fn unsupported_stream_version_is_rejected() {
        let chunks = tokio_stream::iter([Ok(Bytes::from_static(
            br#"{"type":"start","version":3,"state":{}}
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
                version: 3
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
        assert!(matches!(error, ApiStreamError::MissingStart { record: 0 }));
    }

    #[test]
    fn boundary_target_preserves_typed_keys_and_optionally_checks_declaration() {
        let descriptor = BoundaryDescriptor {
            instance_id: BoundaryInstanceId::from_raw(4),
            declaration_id: 9,
            owner: Arc::from("feed-list"),
            name: Arc::from("row"),
            key: Some(BoundaryKey::Number(7.into())),
        };
        let mut matching = target("feed-list", "row");
        matching.key = Some(ApiBoundaryKey::Number(7.into()));
        assert!(matching.matches(&descriptor));
        matching.declaration_id = Some(9);
        assert!(matching.matches(&descriptor));
        matching.declaration_id = Some(10);
        assert!(!matching.matches(&descriptor));
        matching.declaration_id = None;
        matching.key = Some(ApiBoundaryKey::String("7".to_owned()));
        assert!(!matching.matches(&descriptor));

        assert!(serde_json::from_str::<ApiStreamCommand>(
            r#"{"type":"update","boundary":{"owner":"feed-list","name":"row","key":null},"state":{}}"#
        )
        .is_err());
    }

    #[test]
    fn renderer_writes_resume_update_and_advance_as_ordered_segments() {
        let commands = vec![
            ApiStreamCommand::Start {
                version: VERSION,
                state: serde_json::json!({}),
            },
            ApiStreamCommand::Resume {
                boundary: target("index.html", "content"),
                mode: ApiBoundaryMode::Updatable,
                state: serde_json::json!({}),
            },
            ApiStreamCommand::Update {
                boundary: target("index.html", "content"),
                state: serde_json::json!({ "count": 2 }),
            },
            ApiStreamCommand::Resume {
                boundary: target("index.html", "tail"),
                mode: ApiBoundaryMode::Final,
                state: serde_json::json!({}),
            },
        ];
        let (result, segments) = run_commands(
            valid_streaming_config(Vec::new(), &["content", "tail"]),
            commands,
        );
        result.unwrap_or_else(|error| panic!("renderer failed: {error}"));
        let content = segments
            .iter()
            .position(|segment| {
                segment
                    .windows(b"data-boundary=\"content\"".len())
                    .any(|window| window == b"data-boundary=\"content\"")
            })
            .unwrap_or_else(|| panic!("content checkpoint segment is missing"));
        let update = segments
            .iter()
            .position(|segment| {
                segment
                    .windows(b"[2,1,2,0,".len())
                    .any(|window| window == b"[2,1,2,0,")
            })
            .unwrap_or_else(|| panic!("update segment is missing"));
        let tail = segments
            .iter()
            .position(|segment| {
                segment
                    .windows(b"data-boundary=\"tail\"".len())
                    .any(|window| window == b"data-boundary=\"tail\"")
            })
            .unwrap_or_else(|| panic!("tail checkpoint segment is missing"));
        let terminal = segments
            .iter()
            .position(|segment| {
                segment
                    .windows(b"[2,3,4,0,{}]".len())
                    .any(|window| window == b"[2,3,4,0,{}]")
            })
            .unwrap_or_else(|| panic!("terminal segment is missing"));
        assert!(content < update);
        assert!(update < tail);
        assert!(tail < terminal);
        assert!(!segments[content]
            .windows(b"data-boundary=\"tail\"".len())
            .any(|window| window == b"data-boundary=\"tail\""));

        let html = String::from_utf8(join_segments(&segments))
            .unwrap_or_else(|error| panic!("renderer produced invalid UTF-8: {error}"));
        assert!(html.contains("data-boundary=\"content\""));
        assert!(html.contains("data-boundary=\"tail\""));
        assert!(html.contains("[2,3,4,0,{}]"));
    }

    #[test]
    fn renderer_flushes_a_single_checkpoint_before_its_tail() {
        let commands = vec![
            ApiStreamCommand::Start {
                version: VERSION,
                state: serde_json::json!({}),
            },
            ApiStreamCommand::Resume {
                boundary: target("index.html", "content"),
                mode: ApiBoundaryMode::Final,
                state: serde_json::json!({}),
            },
        ];
        let (result, segments) =
            run_commands(valid_streaming_config(Vec::new(), &["content"]), commands);
        result.unwrap_or_else(|error| panic!("renderer failed: {error}"));

        let checkpoint = segments
            .iter()
            .position(|segment| {
                segment
                    .windows(b"data-boundary=\"content\"".len())
                    .any(|window| window == b"data-boundary=\"content\"")
            })
            .unwrap_or_else(|| panic!("checkpoint segment is missing"));
        let tail = segments
            .iter()
            .position(|segment| {
                segment
                    .windows(b"<footer data-page-tail>".len())
                    .any(|window| window == b"<footer data-page-tail>")
            })
            .unwrap_or_else(|| panic!("tail segment is missing"));
        assert!(checkpoint < tail);
        assert!(!segments[checkpoint]
            .windows(b"<footer data-page-tail>".len())
            .any(|window| window == b"<footer data-page-tail>"));
        assert!(!segments[checkpoint]
            .windows(b"[2,1,4,0,{}]".len())
            .any(|window| window == b"[2,1,4,0,{}]"));
        assert!(segments[checkpoint].ends_with(b"<webui-hydrate></webui-hydrate>"));
        assert!(segments[tail].starts_with(b"<footer data-page-tail>"));
    }

    #[test]
    fn renderer_rejects_a_resume_that_does_not_match_the_current_cursor() {
        let commands = vec![
            ApiStreamCommand::Start {
                version: VERSION,
                state: serde_json::json!({}),
            },
            ApiStreamCommand::Resume {
                boundary: target("wrong-owner", "content"),
                mode: ApiBoundaryMode::Final,
                state: serde_json::json!({}),
            },
        ];
        let (result, _) = run_commands(valid_streaming_config(Vec::new(), &["content"]), commands);
        assert!(matches!(
            result,
            Err(RendererError::ResumeMismatch { record: 1, .. })
        ));
    }

    #[test]
    fn renderer_rejects_truncation_while_a_cursor_is_pending() {
        let commands = vec![ApiStreamCommand::Start {
            version: VERSION,
            state: serde_json::json!({}),
        }];
        let (result, _) = run_commands(valid_streaming_config(Vec::new(), &["content"]), commands);
        assert!(matches!(result, Err(RendererError::Truncated { .. })));
    }

    #[test]
    fn renderer_rejects_resume_after_automatic_advance_completes() {
        let commands = vec![
            ApiStreamCommand::Start {
                version: VERSION,
                state: serde_json::json!({}),
            },
            ApiStreamCommand::Resume {
                boundary: target("index.html", "content"),
                mode: ApiBoundaryMode::Final,
                state: serde_json::json!({}),
            },
            ApiStreamCommand::Resume {
                boundary: target("index.html", "content"),
                mode: ApiBoundaryMode::Final,
                state: serde_json::json!({}),
            },
        ];
        let (result, _) = run_commands(valid_streaming_config(Vec::new(), &["content"]), commands);
        assert!(matches!(
            result,
            Err(RendererError::CommandAfterDone {
                record: 2,
                command: "resume"
            })
        ));
    }

    #[test]
    fn boundary_free_start_completes_when_the_backend_body_ends() {
        let commands = vec![ApiStreamCommand::Start {
            version: VERSION,
            state: serde_json::json!({}),
        }];
        let (result, segments) = run_commands(valid_streaming_config(Vec::new(), &[]), commands);
        result.unwrap_or_else(|error| panic!("boundary-free render failed: {error}"));
        let html = join_segments(&segments);
        assert!(String::from_utf8_lossy(&html).contains("[2,0,4,0,{}]"));
    }

    #[test]
    fn update_rejects_ambiguous_unkeyed_occurrences() {
        let descriptor = BoundaryDescriptor {
            instance_id: BoundaryInstanceId::from_raw(0),
            declaration_id: 0,
            owner: Arc::from("row-list"),
            name: Arc::from("row"),
            key: None,
        };
        let committed = vec![
            CommittedBoundary {
                descriptor: descriptor.clone(),
                mode: BoundaryMode::Updatable,
            },
            CommittedBoundary {
                descriptor: BoundaryDescriptor {
                    instance_id: BoundaryInstanceId::from_raw(1),
                    ..descriptor
                },
                mode: BoundaryMode::Updatable,
            },
        ];
        assert!(matches!(
            resolve_update(&committed, &target("row-list", "row"), 3),
            Err(RendererError::AmbiguousUpdate { record: 3, .. })
        ));
    }

    #[actix_web::test]
    async fn start_render_failure_is_reported_before_http_success() {
        let chunks = tokio_stream::iter([Ok::<Bytes, String>(Bytes::from_static(
            br#"{"type":"start","version":2,"state":{}}
"#,
        ))]);

        let response = render(chunks, start_render_failure_config(), defaults()).await;

        assert_eq!(
            response.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[actix_web::test]
    async fn precommit_output_larger_than_the_live_channel_is_staged_without_deadlock() {
        let precommit_bytes =
            (StreamingWriter::DEFAULT_CHANNEL_CAPACITY + 1) * StreamingWriter::CHUNK_TARGET;
        let precommit_fragments = (0..=StreamingWriter::DEFAULT_CHANNEL_CAPACITY)
            .map(|_| WebUIFragment::raw("~".repeat(StreamingWriter::CHUNK_TARGET)))
            .collect();
        let chunks = tokio_stream::iter([Ok::<Bytes, String>(Bytes::from_static(
            br#"{"type":"start","version":2,"state":{}}
{"type":"resume","boundary":{"owner":"index.html","name":"content"},"state":{}}
"#,
        ))]);

        let response = tokio::time::timeout(
            Duration::from_secs(2),
            render(
                chunks,
                valid_streaming_config(precommit_fragments, &["content"]),
                defaults(),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("precommit staging deadlocked"));
        assert_eq!(response.status(), actix_web::http::StatusCode::OK);

        let body = tokio::time::timeout(Duration::from_secs(2), to_bytes(response.into_body()))
            .await
            .unwrap_or_else(|_| panic!("staged response did not finish"))
            .unwrap_or_else(|error| panic!("failed to read staged response: {error}"));
        assert_eq!(memchr::memchr_iter(b'~', &body).count(), precommit_bytes);
        let precommit_position = body
            .iter()
            .position(|byte| *byte == b'~')
            .unwrap_or_else(|| panic!("staged precommit bytes are missing"));
        let boundary_position = body
            .windows(b"data-boundary=\"content\"".len())
            .position(|window| window == b"data-boundary=\"content\"")
            .unwrap_or_else(|| panic!("boundary output is missing"));
        assert!(precommit_position < boundary_position);
    }

    #[actix_web::test]
    async fn initial_output_over_precommit_limit_is_rejected() {
        let chunks = tokio_stream::iter([Ok::<Bytes, String>(Bytes::from_static(
            br#"{"type":"start","version":2,"state":{}}
"#,
        ))]);
        let precommit = WebUIFragment::raw("x".repeat(MAX_PRECOMMIT_BYTES + 1));

        let response = tokio::time::timeout(
            Duration::from_secs(2),
            render(
                chunks,
                valid_streaming_config(vec![precommit], &["content"]),
                defaults(),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("oversized precommit output timed out"));
        assert_eq!(
            response.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        let body = to_bytes(response.into_body())
            .await
            .unwrap_or_else(|error| panic!("failed to read error response: {error}"));
        assert!(String::from_utf8_lossy(&body).contains("4,000,000-byte precommit limit"));
    }

    #[actix_web::test]
    async fn dropping_browser_response_cancels_stalled_backend() {
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let backend = StalledBackend {
            start: Some(Bytes::from_static(
                br#"{"type":"start","version":2,"state":{}}
"#,
            )),
            dropped: Some(dropped_tx),
        };

        let response = render(
            backend,
            valid_streaming_config(Vec::new(), &["content"]),
            defaults(),
        )
        .await;
        assert_eq!(response.status(), actix_web::http::StatusCode::OK);

        drop(response);
        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .unwrap_or_else(|_| panic!("stalled backend was not cancelled"))
            .unwrap_or_else(|_| panic!("backend drop signal was lost"));
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
