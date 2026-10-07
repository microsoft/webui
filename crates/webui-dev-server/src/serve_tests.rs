// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;
use std::sync::Arc;

use actix_web::body::to_bytes;
use actix_web::http::header::{CONTENT_TYPE, LOCATION};
use actix_web::http::StatusCode;
use actix_web::test::TestRequest;

use crate::livereload::LiveReload;

use super::{serve_static_file, NotFoundStrategy, StaticServeConfig};

#[cfg(unix)]
fn symlink_file(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn symlink_file(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

#[cfg(unix)]
fn symlink_directory(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn symlink_directory(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(original, link)
}

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
async fn serves_index_files_for_directory_urls() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root_bytes = b"root index";
    let nested_bytes = b"nested index";
    std::fs::write(directory.path().join("index.html"), root_bytes)?;
    std::fs::create_dir(directory.path().join("guide"))?;
    std::fs::write(directory.path().join("guide/index.html"), nested_bytes)?;
    let cfg = config(directory.path().to_path_buf(), NotFoundStrategy::Plain);

    let root_response = serve_static_file(
        &TestRequest::with_uri("/").to_http_request(),
        Arc::clone(&cfg),
    )
    .await;
    let nested_response =
        serve_static_file(&TestRequest::with_uri("/guide/").to_http_request(), cfg).await;

    assert_eq!(root_response.status(), StatusCode::OK);
    assert!(to_bytes(root_response.into_body())
        .await?
        .starts_with(root_bytes));
    assert_eq!(nested_response.status(), StatusCode::OK);
    assert!(to_bytes(nested_response.into_body())
        .await?
        .starts_with(nested_bytes));
    Ok(())
}

#[cfg(any(unix, windows))]
#[actix_web::test]
async fn serves_in_root_directory_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let assets = directory.path().join("linked-assets");
    let bytes = b"body { color: blue; }";
    std::fs::create_dir(&assets)?;
    std::fs::write(assets.join("app.css"), bytes)?;
    symlink_directory(&assets, &directory.path().join("assets"))?;
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

#[cfg(any(unix, windows))]
#[actix_web::test]
async fn uses_requested_symlink_extension_for_mime() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("asset");
    let bytes = b"body { color: purple; }";
    std::fs::write(&target, bytes)?;
    symlink_file(&target, &directory.path().join("style.css"))?;
    let request = TestRequest::with_uri("/style.css").to_http_request();

    let response = serve_static_file(
        &request,
        config(directory.path().to_path_buf(), NotFoundStrategy::Plain),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&"text/css".parse()?)
    );
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

#[cfg(any(unix, windows))]
#[actix_web::test]
async fn serves_in_root_symlinked_not_found_file() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("not-found-body");
    let bytes = b"linked not found";
    std::fs::write(&target, bytes)?;
    symlink_file(&target, &directory.path().join("404.txt"))?;
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
