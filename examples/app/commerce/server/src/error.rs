// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};
use anyhow::Error as AnyhowError;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ServerError {
    #[error("Not Found")]
    NotFound,
    #[error("Unknown product")]
    UnknownProduct,
    #[error("Cross-site request rejected")]
    CsrfRejected,
    #[error("Too many requests")]
    RateLimited,
    #[error("Failed to render the requested page")]
    RenderFailed(#[source] AnyhowError),
}

impl ResponseError for ServerError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::UnknownProduct => StatusCode::BAD_REQUEST,
            Self::CsrfRejected => StatusCode::FORBIDDEN,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::RenderFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).body(self.to_string())
    }
}
