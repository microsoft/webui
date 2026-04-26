// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Markdown error: {0}")]
    Markdown(String),
    #[error("Build error: {0}")]
    Build(String),
    #[error("Render error: {0}")]
    Render(String),
}

pub type Result<T> = std::result::Result<T, Error>;
