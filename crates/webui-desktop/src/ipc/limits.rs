// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{error::fail, Endpoint, IpcError, IpcErrorCode, IpcSchema};
use std::time::Duration;

macro_rules! limits {
    ($($field:ident = $value:expr),+ $(,)?) => {
        /// Bounded application IPC policy.
        ///
        /// Start with [`Default`] and optionally tune the encoded frame size or
        /// renderer's default RPC timeout. Queue, memory, callback, worker-task,
        /// and control-reservation budgets are SDK-owned, not independent knobs.
        /// Serialization is for native admission metadata, not configuration input.
        ///
        /// ```compile_fail
        /// use webui_desktop::ipc::IpcLimits;
        /// let mut limits = IpcLimits::default();
        /// limits.reserved_control_frames_per_direction = 0;
        /// ```
        ///
        /// ```compile_fail
        /// use webui_desktop::ipc::IpcLimits;
        /// let limits: IpcLimits = serde_json::from_str("{}").unwrap();
        /// ```
        #[derive(Clone, Debug, serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        pub struct IpcLimits { $(pub(crate) $field: usize),+ }
        impl Default for IpcLimits {
            fn default() -> Self { Self { $($field: $value),+ } }
        }
        impl IpcLimits {
            pub(crate) fn validate(&self) -> Result<(), IpcError> {
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
    /// Set the complete encoded frame limit (default: 1 MiB).
    ///
    /// # Errors
    ///
    /// Rejects values outside 2,176 bytes through 8 MiB. This preserves room for
    /// bounded error responses and never enlarges the SDK's memory budgets.
    pub fn with_max_frame_bytes(mut self, bytes: usize) -> Result<Self, IpcError> {
        if !(2176..=8 * 1024 * 1024).contains(&bytes) {
            return Err(IpcError::new(
                IpcErrorCode::InvalidPayload,
                "IPC frame size is outside the supported range",
                "choose an encoded frame limit from 2176 bytes through 8 MiB",
            ));
        }
        self.max_frame_bytes = bytes;
        self.validate()?;
        Ok(self)
    }

    /// Set the renderer's default RPC timeout (default: 30 seconds).
    ///
    /// Rust calls use [`super::CallOptions`]. Per-call overrides remain bounded
    /// by the SDK's five-minute maximum; admission and notification deadlines
    /// are unchanged.
    ///
    /// # Errors
    ///
    /// Rejects durations outside 1 ms through 5 minutes or fractional milliseconds.
    pub fn with_default_timeout(mut self, timeout: Duration) -> Result<Self, IpcError> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(300)
            || !timeout.subsec_nanos().is_multiple_of(1_000_000)
        {
            return Err(IpcError::new(
                IpcErrorCode::InvalidPayload,
                "IPC default timeout is outside the supported range or precision",
                "choose a whole number of milliseconds from 1 through 300000",
            ));
        }
        self.default_timeout_ms =
            usize::try_from(timeout.as_millis()).map_err(|_| fail(IpcErrorCode::InvalidPayload))?;
        self.validate()?;
        Ok(self)
    }

    /// Maximum bytes in one complete encoded frame.
    #[must_use]
    pub const fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    /// Default renderer RPC timeout.
    #[must_use]
    pub fn default_timeout(&self) -> Duration {
        Duration::from_millis(self.default_timeout_ms as u64)
    }

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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn policy_builders_preserve_all_private_wire_budgets() {
        let baseline = serde_json::to_value(IpcLimits::default()).unwrap();
        assert_eq!(baseline.as_object().unwrap().len(), 21);
        for bytes in [2176, 256 * 1024, 8 * 1024 * 1024] {
            for timeout_ms in [1, 10_000, 300_000] {
                let limits = IpcLimits::default()
                    .with_max_frame_bytes(bytes)
                    .unwrap()
                    .with_default_timeout(Duration::from_millis(timeout_ms))
                    .unwrap();
                limits.validate().unwrap();
                let actual = serde_json::to_value(&limits).unwrap();
                let mut expected = baseline.clone();
                expected["maxFrameBytes"] = bytes.into();
                expected["defaultTimeoutMs"] = timeout_ms.into();
                assert_eq!(actual, expected);
                assert_eq!(limits.max_queued_bytes_per_direction, 8 * 1024 * 1024);
                assert_eq!(limits.max_admitted_input_bytes_per_frame, 8 * 1024 * 1024);
                assert_eq!(limits.max_retained_bytes_per_frame, 16 * 1024 * 1024);
                assert_eq!(limits.reserved_control_frames_per_direction, 128);
                assert_eq!(limits.max_callbacks_per_event, 16);
                assert_eq!(limits.max_callbacks_per_document, 128);
            }
        }
    }
}
