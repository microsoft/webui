// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};
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
}

impl ResponseError for ServerError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::UnknownProduct => StatusCode::BAD_REQUEST,
            Self::CsrfRejected => StatusCode::FORBIDDEN,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).body(self.to_string())
    }
}
