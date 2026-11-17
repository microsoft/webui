// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::time::Duration;

use anyhow::Result;
use tokio::process::Child;
use webui_dev_server::shutdown::Control;

use super::NativeError;

pub(crate) struct Stop {
    control: Control,
    #[cfg(unix)]
    _terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    _interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    _hangup: tokio::signal::unix::Signal,
    #[cfg(windows)]
    _break: tokio::signal::windows::CtrlBreak,
}

impl Stop {
    pub(crate) fn new(control: Control) -> Result<Self> {
        Ok(Self {
            control,
            #[cfg(unix)]
            _terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            #[cfg(unix)]
            _interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            #[cfg(unix)]
            _hangup: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?,
            #[cfg(windows)]
            _break: tokio::signal::windows::ctrl_break()?,
        })
    }

    pub(crate) async fn requested(&mut self) -> Result<()> {
        self.control
            .shutdown_requested()
            .await
            .map_err(|error| NativeError::Cleanup(error.to_string()).into())
    }

    pub(crate) async fn abort_build(&self, child: &mut Child) -> Result<()> {
        self.control
            .interrupt_build()
            .map_err(|error| NativeError::Cleanup(error.to_string()))?;
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            result => Err(NativeError::Cleanup(format!(
                "native compiler did not confirm cancellation: {result:?}"
            ))
            .into()),
        }
    }
}
