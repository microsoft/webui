// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;
use std::sync::Arc;

use actix_web::body::to_bytes;
use actix_web::http::header::LOCATION;
use actix_web::http::StatusCode;
use actix_web::test::TestRequest;

use crate::livereload::LiveReload;

use super::{serve_static_file, NotFoundStrategy, StaticServeConfig};

fn config(root: PathBuf, not_found: NotFoundStrategy) -> Arc<StaticServeConfig> {
    Arc::new(
        StaticServeConfig::new(
            root,
            "/".to_owned(),
            LiveReload::new("/__test/livereload"),
            not_found,
        )
        .unwrap_or_else(|error| panic!("test static-file config failed: {error}")),
    )
}

#[actix_web::test]
async fn serves_file_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let bytes = b"console.log('served');";
    std::fs::write(directory.path().join("app.js"), bytes)?;
    let request = TestRequest::with_uri("/app.js").to_http_request();

    let response = serve_static_file(
        &request,
        config(directory.path().to_path_buf(), NotFoundStrategy::Plain),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body()).await?.as_ref(), bytes);
    Ok(())
}

#[actix_web::test]
async fn serves_nested_file_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let bytes = b"body { color: green; }";
    std::fs::create_dir(directory.path().join("assets"))?;
    std::fs::write(directory.path().join("assets/app.css"), bytes)?;
    let request = TestRequest::with_uri("/assets/app.css").to_http_request();

    let response = serve_static_file(
        &request,
        config(directory.path().to_path_buf(), NotFoundStrategy::Plain),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body()).await?.as_ref(), bytes);
    Ok(())
}

#[actix_web::test]
async fn redirects_directory_without_trailing_slash() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::create_dir(directory.path().join("guide"))?;
    let request = TestRequest::with_uri("/guide").to_http_request();

    let response = serve_static_file(
        &request,
        config(directory.path().to_path_buf(), NotFoundStrategy::Plain),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.headers().get(LOCATION), Some(&"/guide/".parse()?));
    Ok(())
}

#[actix_web::test]
async fn serves_custom_not_found_file() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let bytes = b"custom not found";
    std::fs::write(directory.path().join("404.txt"), bytes)?;
    let request = TestRequest::with_uri("/missing.txt").to_http_request();

    let response = serve_static_file(
        &request,
        config(
            directory.path().to_path_buf(),
            NotFoundStrategy::File(PathBuf::from("404.txt")),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(to_bytes(response.into_body()).await?.as_ref(), bytes);
    Ok(())
}

#[cfg(unix)]
#[actix_web::test]
async fn rejects_symlink_that_escapes_root() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    std::fs::write(outside.path().join("secret.txt"), b"not served")?;
    symlink(outside.path(), directory.path().join("escape"))?;
    let request = TestRequest::with_uri("/escape/secret.txt").to_http_request();

    let response = serve_static_file(
        &request,
        config(directory.path().to_path_buf(), NotFoundStrategy::Plain),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        to_bytes(response.into_body()).await?.as_ref(),
        b"404 Not Found"
    );
    Ok(())
}

#[cfg(unix)]
#[actix_web::test]
async fn rejects_fallback_symlink_that_escapes_root() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let outside = tempfile::NamedTempFile::new()?;
    std::fs::write(outside.path(), b"not served")?;
    symlink(outside.path(), directory.path().join("404.txt"))?;
    let request = TestRequest::with_uri("/missing.txt").to_http_request();

    let response = serve_static_file(
        &request,
        config(
            directory.path().to_path_buf(),
            NotFoundStrategy::File(PathBuf::from("404.txt")),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        to_bytes(response.into_body()).await?.as_ref(),
        b"404 Not Found"
    );
    Ok(())
}
