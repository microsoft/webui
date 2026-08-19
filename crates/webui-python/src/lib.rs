// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::pybacked::{PyBackedBytes, PyBackedStr};
use pyo3::types::{PyAny, PyByteArray, PyBytes, PyModule, PyString, PyType};
use serde_json::Value;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryId, BoundaryMode, HandlerError, Protocol, RenderOptions, ResponseWriter,
    SessionOptions, StreamingSession as HandlerStreamingSession, WebUIHandler,
};

// PyO3's exception macro uses `Result::expect` inside its one-time type initializer.
#[allow(clippy::disallowed_methods)]
mod exceptions {
    use pyo3::create_exception;
    use pyo3::exceptions::PyException;

    create_exception!(microsoft_webui._native, WebUIError, PyException);
    create_exception!(microsoft_webui._native, ProtocolError, WebUIError);
    create_exception!(microsoft_webui._native, StateError, WebUIError);
    create_exception!(microsoft_webui._native, RenderError, WebUIError);
    create_exception!(microsoft_webui._native, StreamingError, WebUIError);
}

use exceptions::{ProtocolError, RenderError, StateError, StreamingError, WebUIError};

type PyRenderOptions = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);
type PyPartialOptions = (String, String, String);
type PyTemplateOptions = (Vec<String>, String);

enum JsonInput {
    Text(PyBackedStr),
    Bytes(PyBackedBytes),
}

impl JsonInput {
    fn extract(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if value.is_instance_of::<PyString>() {
            return value.extract::<PyBackedStr>().map(Self::Text);
        }
        if value.is_instance_of::<PyBytes>() || value.is_instance_of::<PyByteArray>() {
            return Ok(value.extract::<PyBackedBytes>().map(Self::Bytes)?);
        }
        Err(PyTypeError::new_err(
            "state JSON must be str, bytes, or bytearray",
        ))
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(value) => value.as_bytes(),
            Self::Bytes(value) => value,
        }
    }

    fn as_str(&self) -> Result<&str, BindingError> {
        match self {
            Self::Text(value) => Ok(value),
            Self::Bytes(value) => std::str::from_utf8(value).map_err(|error| {
                BindingError::state(format!("state JSON is not valid UTF-8: {error}"))
            }),
        }
    }
}

struct OwnedRenderOptions {
    entry_id: String,
    request_path: String,
    nonce: Option<String>,
    head_inject: Option<String>,
    body_inject: Option<String>,
}

impl OwnedRenderOptions {
    fn borrowed(&self) -> RenderOptions<'_> {
        RenderOptions {
            entry_id: &self.entry_id,
            request_path: &self.request_path,
            nonce: self.nonce.as_deref(),
            head_inject: self.head_inject.as_deref(),
            body_inject: self.body_inject.as_deref(),
        }
    }

    fn session(self) -> SessionOptions {
        SessionOptions {
            entry_id: self.entry_id,
            request_path: self.request_path,
            nonce: self.nonce,
            head_inject: self.head_inject,
            body_inject: self.body_inject,
        }
    }
}

impl From<PyRenderOptions> for OwnedRenderOptions {
    fn from(options: PyRenderOptions) -> Self {
        Self {
            entry_id: options.0,
            request_path: options.1,
            nonce: options.2.filter(|value| !value.is_empty()),
            head_inject: options.3.filter(|value| !value.is_empty()),
            body_inject: options.4.filter(|value| !value.is_empty()),
        }
    }
}

enum BindingErrorKind {
    Protocol,
    State,
    Render,
    Streaming,
}

struct BindingError {
    kind: BindingErrorKind,
    message: String,
}

impl BindingError {
    #[cold]
    #[inline(never)]
    fn protocol(message: String) -> Self {
        Self {
            kind: BindingErrorKind::Protocol,
            message,
        }
    }

    #[cold]
    #[inline(never)]
    fn state(message: String) -> Self {
        Self {
            kind: BindingErrorKind::State,
            message,
        }
    }

    #[cold]
    #[inline(never)]
    fn render(message: String) -> Self {
        Self {
            kind: BindingErrorKind::Render,
            message,
        }
    }

    #[cold]
    #[inline(never)]
    fn streaming(message: String) -> Self {
        Self {
            kind: BindingErrorKind::Streaming,
            message,
        }
    }

    #[cold]
    #[inline(never)]
    fn into_py_error(self) -> PyErr {
        match self.kind {
            BindingErrorKind::Protocol => ProtocolError::new_err(self.message),
            BindingErrorKind::State => StateError::new_err(self.message),
            BindingErrorKind::Render => RenderError::new_err(self.message),
            BindingErrorKind::Streaming => StreamingError::new_err(self.message),
        }
    }
}

struct BytesWriter {
    bytes: Vec<u8>,
}

impl BytesWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(4096),
        }
    }
}

impl ResponseWriter for BytesWriter {
    fn write(&mut self, content: &str) -> Result<(), HandlerError> {
        self.bytes.extend_from_slice(content.as_bytes());
        Ok(())
    }

    fn end(&mut self) -> Result<(), HandlerError> {
        Ok(())
    }
}

#[pyclass(name = "_Renderer", frozen, module = "microsoft_webui._native")]
struct NativeRenderer {
    protocol: Arc<Protocol>,
    handler: Arc<WebUIHandler>,
}

#[pymethods]
impl NativeRenderer {
    #[new]
    #[pyo3(signature = (protocol, *, plugin = None))]
    fn new(py: Python<'_>, protocol: &Bound<'_, PyAny>, plugin: Option<String>) -> PyResult<Self> {
        let bytes = protocol
            .extract::<PyBackedBytes>()
            .map_err(|_| PyTypeError::new_err("protocol must be bytes or bytearray"))?;
        Self::from_bytes(py, bytes.as_ref(), plugin.as_deref())
    }

    #[classmethod]
    #[pyo3(signature = (path, *, plugin = None))]
    fn from_file(
        _class: &Bound<'_, PyType>,
        py: Python<'_>,
        path: PathBuf,
        plugin: Option<String>,
    ) -> PyResult<Self> {
        let bytes = py.detach(|| fs::read(path)).map_err(PyErr::from)?;
        Self::from_bytes(py, &bytes, plugin.as_deref())
    }

    fn render<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
        options: PyRenderOptions,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let input = JsonInput::extract(state_json)?;
        let options = OwnedRenderOptions::from(options);
        let result = py
            .detach(|| self.render_bytes(&input, &options))
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }

    fn render_partial<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
        options: PyPartialOptions,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let input = JsonInput::extract(state_json)?;
        let result = py
            .detach(|| {
                let state_json = input.as_str()?;
                self.protocol
                    .render_partial(state_json, &options.0, &options.1, &options.2)
                    .map(String::into_bytes)
                    .map_err(partial_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }

    fn render_component_templates<'py>(
        &self,
        py: Python<'py>,
        options: PyTemplateOptions,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let result = py
            .detach(|| {
                let tags = options.0.iter().map(String::as_str).collect::<Vec<_>>();
                let payload = self
                    .protocol
                    .render_component_templates(&tags, &options.1)
                    .map_err(render_binding_error)?;
                serde_json::to_vec(&payload).map_err(|error| {
                    BindingError::render(format!(
                        "failed to serialize component templates: {error}"
                    ))
                })
            })
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }

    fn tokens(&self) -> Vec<String> {
        self.protocol.tokens().to_vec()
    }

    fn stream_response(
        &self,
        py: Python<'_>,
        options: PyRenderOptions,
    ) -> PyResult<NativeStreamingSession> {
        let options = OwnedRenderOptions::from(options).session();
        let session = py
            .detach(|| {
                HandlerStreamingSession::new(
                    Arc::clone(&self.handler),
                    Arc::clone(&self.protocol),
                    options,
                )
                .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        Ok(NativeStreamingSession {
            inner: Mutex::new(session),
        })
    }
}

impl NativeRenderer {
    fn from_bytes(py: Python<'_>, bytes: &[u8], plugin: Option<&str>) -> PyResult<Self> {
        let handler = Arc::new(create_handler(plugin)?);
        let protocol = py
            .detach(|| {
                Protocol::from_protobuf(bytes).map_err(|error| {
                    BindingError::protocol(format!("failed to decode WebUI protocol: {error}"))
                })
            })
            .map_err(BindingError::into_py_error)?;
        Ok(Self {
            protocol: Arc::new(protocol),
            handler,
        })
    }

    fn render_bytes(
        &self,
        input: &JsonInput,
        options: &OwnedRenderOptions,
    ) -> Result<Vec<u8>, BindingError> {
        let state = serde_json::from_slice::<Value>(input.as_bytes())
            .map_err(|error| BindingError::state(format!("failed to parse state JSON: {error}")))?;
        let mut writer = BytesWriter::new();
        self.handler
            .render(&self.protocol, &state, &options.borrowed(), &mut writer)
            .map_err(render_binding_error)?;
        Ok(writer.bytes)
    }
}

#[pyclass(name = "_StreamingSession", module = "microsoft_webui._native")]
struct NativeStreamingSession {
    inner: Mutex<HandlerStreamingSession>,
}

#[pymethods]
impl NativeStreamingSession {
    fn boundary(&self, name: &str) -> PyResult<u32> {
        self.session()?
            .boundary(name)
            .map(BoundaryId::raw)
            .map_err(|error| streaming_binding_error(error).into_py_error())
    }

    #[getter]
    fn boundary_count(&self) -> PyResult<usize> {
        Ok(self.session()?.boundary_count())
    }

    #[getter]
    fn finished(&self) -> PyResult<bool> {
        Ok(self.session()?.is_finished())
    }

    fn write_shell<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let input = JsonInput::extract(state_json)?;
        let result = py
            .detach(|| {
                let state = parse_state(&input)?;
                self.session_binding()?
                    .write_shell(&state)
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }

    fn write_boundary<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
        boundary: u32,
        updatable: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let input = JsonInput::extract(state_json)?;
        let result = py
            .detach(|| {
                let state = parse_state(&input)?;
                let mode = if updatable {
                    BoundaryMode::Updatable
                } else {
                    BoundaryMode::Final
                };
                self.session_binding()?
                    .write_boundary(BoundaryId::from_raw(boundary), &state, mode)
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }

    fn update<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
        boundary: u32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let input = JsonInput::extract(state_json)?;
        let result = py
            .detach(|| {
                let state = parse_state(&input)?;
                self.session_binding()?
                    .update(BoundaryId::from_raw(boundary), &state)
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }

    fn finish<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let input = JsonInput::extract(state_json)?;
        let result = py
            .detach(|| {
                let state = parse_state(&input)?;
                self.session_binding()?
                    .finish(&state)
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }
}

impl NativeStreamingSession {
    fn session(&self) -> PyResult<MutexGuard<'_, HandlerStreamingSession>> {
        self.session_binding().map_err(BindingError::into_py_error)
    }

    fn session_binding(&self) -> Result<MutexGuard<'_, HandlerStreamingSession>, BindingError> {
        self.inner.lock().map_err(|_| {
            BindingError::streaming(
                "streaming session lock was poisoned by a previous panic".to_string(),
            )
        })
    }
}

fn parse_state(input: &JsonInput) -> Result<Value, BindingError> {
    serde_json::from_slice(input.as_bytes())
        .map_err(|error| BindingError::state(format!("failed to parse state JSON: {error}")))
}

fn create_handler(plugin: Option<&str>) -> PyResult<WebUIHandler> {
    match plugin.filter(|value| !value.is_empty()) {
        None => Ok(WebUIHandler::new()),
        Some("fast" | "fast-v2") => Ok(WebUIHandler::with_plugin(|| {
            Box::new(FastV2HydrationPlugin::new())
        })),
        Some("fast-v3") => Ok(WebUIHandler::with_plugin(|| {
            Box::new(FastV3HydrationPlugin::new())
        })),
        Some("webui") => Ok(WebUIHandler::with_plugin(|| {
            Box::new(WebUIHydrationPlugin::new())
        })),
        Some(other) => Err(PyValueError::new_err(format!(
            "unknown plugin '{other}'; expected 'webui', 'fast-v3', 'fast-v2', or 'fast'"
        ))),
    }
}

#[cold]
#[inline(never)]
fn partial_binding_error(error: HandlerError) -> BindingError {
    match error {
        HandlerError::Rendering(message) if message.starts_with("invalid state JSON:") => {
            BindingError::state(message)
        }
        other => render_binding_error(other),
    }
}

#[cold]
#[inline(never)]
fn render_binding_error(error: HandlerError) -> BindingError {
    BindingError::render(format!("WebUI render failed: {error}"))
}

#[cold]
#[inline(never)]
fn streaming_binding_error(error: HandlerError) -> BindingError {
    BindingError::streaming(format!("WebUI streaming operation failed: {error}"))
}

#[pymodule]
#[pyo3(name = "_native")]
fn webui_python(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeRenderer>()?;
    module.add_class::<NativeStreamingSession>()?;
    module.add("WebUIError", py.get_type::<WebUIError>())?;
    module.add("ProtocolError", py.get_type::<ProtocolError>())?;
    module.add("StateError", py.get_type::<StateError>())?;
    module.add("RenderError", py.get_type::<RenderError>())?;
    module.add("StreamingError", py.get_type::<StreamingError>())?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
