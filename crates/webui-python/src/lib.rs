// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

mod version;

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::pybacked::{PyBackedBytes, PyBackedStr};
use pyo3::types::{PyAny, PyByteArray, PyBytes, PyDict, PyModule, PyString, PyType};
use serde_json::Value;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryDescriptor, BoundaryInstanceId, BoundaryKey, BoundaryMode, HandlerError, Protocol,
    RenderOptions, SessionOptions, StreamStep as HandlerStreamStep,
    StreamingSession as HandlerStreamingSession, WebUIHandler,
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

webui_handler::define_bytes_response_writer!(BytesWriter, bytes);

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
        let mut writer = BytesWriter::with_capacity(4096);
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
    fn start<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let input = JsonInput::extract(state_json)?;
        let step = py
            .detach(|| {
                let state = parse_state(&input)?;
                self.session_binding()?
                    .start(&state)
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        stream_step_dict(py, step)
    }

    fn resume<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
        instance_id: u32,
        updatable: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let input = JsonInput::extract(state_json)?;
        let step = py
            .detach(|| {
                let state = parse_state(&input)?;
                let mode = if updatable {
                    BoundaryMode::Updatable
                } else {
                    BoundaryMode::Final
                };
                self.session_binding()?
                    .resume(BoundaryInstanceId::from_raw(instance_id), &state, mode)
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        stream_step_dict(py, step)
    }

    fn advance<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let step = py
            .detach(|| {
                self.session_binding()?
                    .advance()
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        stream_step_dict(py, step)
    }

    fn update<'py>(
        &self,
        py: Python<'py>,
        state_json: &Bound<'py, PyAny>,
        instance_id: u32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let input = JsonInput::extract(state_json)?;
        let result = py
            .detach(|| {
                let state = parse_state(&input)?;
                self.session_binding()?
                    .update(BoundaryInstanceId::from_raw(instance_id), &state)
                    .map_err(streaming_binding_error)
            })
            .map_err(BindingError::into_py_error)?;
        Ok(PyBytes::new(py, &result))
    }
}

impl NativeStreamingSession {
    fn session_binding(&self) -> Result<MutexGuard<'_, HandlerStreamingSession>, BindingError> {
        self.inner.lock().map_err(|_| {
            BindingError::streaming(
                "streaming session lock was poisoned by a previous panic".to_string(),
            )
        })
    }
}

fn stream_step_dict(py: Python<'_>, step: HandlerStreamStep) -> PyResult<Bound<'_, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("bytes", PyBytes::new(py, &step.bytes))?;
    result.set_item("done", step.done)?;
    match step.boundary {
        Some(boundary) => result.set_item("boundary", boundary_descriptor_dict(py, boundary)?)?,
        None => result.set_item("boundary", py.None())?,
    }
    Ok(result)
}

fn boundary_descriptor_dict(
    py: Python<'_>,
    descriptor: BoundaryDescriptor,
) -> PyResult<Bound<'_, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("instance_id", descriptor.instance_id.raw())?;
    result.set_item("declaration_id", descriptor.declaration_id)?;
    result.set_item("owner", &*descriptor.owner)?;
    result.set_item("name", &*descriptor.name)?;
    match descriptor.key {
        Some(BoundaryKey::String(key)) => result.set_item("key", key)?,
        Some(BoundaryKey::Number(key)) if key.is_i64() => {
            let value = key
                .as_i64()
                .ok_or_else(|| impossible_boundary_key_error(&key))?;
            result.set_item("key", value)?;
        }
        Some(BoundaryKey::Number(key)) if key.is_u64() => {
            let value = key
                .as_u64()
                .ok_or_else(|| impossible_boundary_key_error(&key))?;
            result.set_item("key", value)?;
        }
        Some(BoundaryKey::Number(key)) if key.is_f64() => {
            let value = key
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| impossible_boundary_key_error(&key))?;
            result.set_item("key", value)?;
        }
        Some(BoundaryKey::Number(key)) => return Err(impossible_boundary_key_error(&key)),
        None => result.set_item("key", py.None())?,
    }
    Ok(result)
}

#[cold]
#[inline(never)]
fn impossible_boundary_key_error(key: &serde_json::Number) -> PyErr {
    BindingError::streaming(format!(
        "WebUI returned an unsupported boundary key number `{key}`"
    ))
    .into_py_error()
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
        error @ HandlerError::InvalidState(_) => BindingError::state(error.to_string()),
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
    module.add("__version__", version::PYTHON_PACKAGE_VERSION)?;
    Ok(())
}
