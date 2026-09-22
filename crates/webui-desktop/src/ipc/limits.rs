// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{error::fail, Endpoint, IpcError, IpcErrorCode, IpcSchema};

macro_rules! limits {
    ($($field:ident = $value:expr),+ $(,)?) => {
        /// Numeric transport and execution budgets. Values are safety defaults,
        /// not measured performance thresholds.
        #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct IpcLimits { $(#[doc = stringify!($field)] pub $field: usize),+ }
        impl Default for IpcLimits {
            fn default() -> Self { Self { $($field: $value),+ } }
        }
        impl IpcLimits {
            /// Reject zero, excessive, or inconsistent capacities before startup.
            pub fn validate(&self) -> Result<(), IpcError> {
                if $(self.$field == 0 ||)+ false { return Err(fail(IpcErrorCode::InvalidPayload)); }
                self.validate_ceiling()
            }
        }
    }
}
limits! {
    max_frame_bytes = 1_048_576,
    max_native_control_bytes = 4096,
    max_pending_calls_per_direction = 64,
    max_outstanding_notifications_per_direction = 64,
    max_queued_frames_per_direction = 128,
    max_queued_bytes_per_direction = 8_388_608,
    max_admitted_input_bytes_per_frame = 8_388_608,
    max_retained_bytes_per_frame = 16_777_216,
    max_worker_tasks_per_frame_including_retired_documents = 128,
    max_callbacks_per_event = 16,
    max_callbacks_per_document = 128,
    max_callback_tasks_per_document = 128,
    max_collection_entries_per_message = 16_384,
    max_schema_depth = 16,
    max_readiness_waiters = 16,
    default_timeout_ms = 30_000,
    max_timeout_ms = 300_000,
    notification_accept_timeout_ms = 30_000,
    handshake_timeout_ms = 5000,
    max_error_text_bytes_total = 2048,
    reserved_control_frames_per_direction = 128,
}

impl IpcLimits {
    fn validate_ceiling(&self) -> Result<(), IpcError> {
        if self.max_frame_bytes > 16 * 1024 * 1024
            || self.max_frame_bytes < self.max_error_text_bytes_total.saturating_add(128)
            || self.max_native_control_bytes > 64 * 1024
            || self.max_pending_calls_per_direction > 4096
            || self.max_outstanding_notifications_per_direction > 4096
            || self.max_callbacks_per_document > 1024
            || self.max_callbacks_per_event > self.max_callbacks_per_document
            || self.max_queued_bytes_per_direction > 64 * 1024 * 1024
            || self.max_queued_bytes_per_direction < self.max_frame_bytes
            || self.max_admitted_input_bytes_per_frame < self.max_frame_bytes
            || self.max_admitted_input_bytes_per_frame > 64 * 1024 * 1024
            || self.max_retained_bytes_per_frame < self.max_admitted_input_bytes_per_frame
            || self.max_retained_bytes_per_frame > 128 * 1024 * 1024
            || self.max_schema_depth > 16
            || self.max_readiness_waiters > 16
            || self.max_timeout_ms > 300_000
            || self.default_timeout_ms > self.max_timeout_ms
            || self.notification_accept_timeout_ms > self.max_timeout_ms
            || self.max_worker_tasks_per_frame_including_retired_documents > 4096
            || self.max_callback_tasks_per_document > 4096
            || self.max_queued_frames_per_direction > 8192
            || self.reserved_control_frames_per_direction > 8192
            || self.max_collection_entries_per_message > 1_048_576
            || self.max_error_text_bytes_total > 2048
            || self.handshake_timeout_ms > 5000
        {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        Ok(())
    }
}

/// Explicit per-frame permissions and execution policy.
#[derive(Clone, Debug)]
pub struct IpcOptions {
    /// Validated safety budgets.
    pub limits: IpcLimits,
    /// Lazily started workers. Blocking application code occupies a worker.
    pub worker_threads: usize,
    /// IDs the host may send to the renderer.
    pub allow_renderer: Vec<u32>,
    /// IDs the renderer may send to the host.
    pub allow_host: Vec<u32>,
    /// Enables development methods only on a source host.
    pub development: bool,
}

impl Default for IpcOptions {
    fn default() -> Self {
        Self {
            limits: IpcLimits::default(),
            worker_threads: 1,
            allow_renderer: Vec::new(),
            allow_host: Vec::new(),
            development: false,
        }
    }
}
impl IpcOptions {
    /// Explicitly grant all declared IDs; development-only methods still require
    /// a source host and `development = true`.
    pub fn for_schema(schema: &'static IpcSchema) -> Self {
        let mut options = Self::default();
        for method in schema.methods {
            match method.receiver {
                Endpoint::Host => options.allow_host.push(method.id),
                Endpoint::Renderer => options.allow_renderer.push(method.id),
            }
        }
        options
    }
    pub(super) fn validate(&self) -> Result<(), IpcError> {
        self.limits.validate()?;
        if self.worker_threads == 0 || self.worker_threads > 32 {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        Ok(())
    }
}
