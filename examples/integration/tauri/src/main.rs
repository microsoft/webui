// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod assets;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use http::{header, Method, Request, Response};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Plugin {
    Webui,
}

#[derive(Parser)]
#[command(about = "Open a pre-built WebUI app in Tauri; no Node or HTTP server")]
struct Options {
    /// Directory containing protocol.bin and browser assets.
    dist_dir: PathBuf,
    /// JSON state used for the initial render.
    state: PathBuf,
    /// Enable WebUI browser hydration.
    #[arg(long)]
    plugin: Option<Plugin>,
    /// Theme token file or npm package; packages resolve from the state directory.
    #[arg(long)]
    theme: Option<String>,
}

struct Snapshot {
    root: PathBuf,
    html: Vec<u8>,
}

impl Snapshot {
    fn load(options: &Options) -> Result<Self> {
        let root = options
            .dist_dir
            .canonicalize()
            .context("build directory not found; build the WebUI app first")?;
        let protocol = Protocol::from_protobuf(&assets::read_bounded(&root.join("protocol.bin"))?)
            .context("invalid protocol.bin; rebuild with this checkout's WebUI CLI")?;
        let mut state: serde_json::Value =
            serde_json::from_slice(&assets::read_bounded(&options.state)?)
                .context("invalid state JSON; provide a JSON object")?;
        if !state.is_object() {
            bail!("state must be a JSON object");
        }
        state["basePath"] = "/".into();
        if let Some(theme) = &options.theme {
            if state
                .get("tokens")
                .is_some_and(|tokens| !tokens.is_object())
            {
                bail!("state.tokens must be an object; remove it or supply per-theme CSS strings");
            }
            let state_path = options.state.canonicalize()?;
            let theme_path = webui_tokens::resolve_theme_path(
                theme,
                state_path
                    .parent()
                    .context("state file has no parent directory")?,
            )?;
            let bytes = assets::read_bounded(&theme_path)?;
            let tokens = webui_tokens::parse_token_content(
                std::str::from_utf8(&bytes).context("theme must contain UTF-8 JSON")?,
                &theme_path,
            )?;
            let resolved = webui_tokens::resolve_tokens(protocol.tokens(), &tokens)?;
            webui_tokens::inject_into_state(&mut state, &resolved);
        }
        let handler = match options.plugin {
            Some(Plugin::Webui) => {
                WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
            }
            None => WebUIHandler::new(),
        };
        let mut writer = HtmlWriter(Vec::with_capacity(16 * 1024));
        handler.render(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        Ok(Self {
            root,
            html: writer.0,
        })
    }

    fn respond(&self, request: &Request<Vec<u8>>) -> Result<Response<Vec<u8>>> {
        if !is_app_url(&url::Url::parse(&request.uri().to_string())?) {
            return response(403, "text/plain", b"Foreign origin denied".to_vec());
        }
        if request.method() != Method::GET {
            let mut result = response(
                405,
                "text/plain",
                b"This example is a read-only snapshot.".to_vec(),
            )?;
            result
                .headers_mut()
                .insert(header::ALLOW, header::HeaderValue::from_static("GET"));
            return Ok(result);
        }
        if request.uri().path() == "/" {
            return response(200, "text/html; charset=utf-8", self.html.clone());
        }
        match assets::asset_path(&self.root, request.uri().path())? {
            Some(path) => response(
                200,
                mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .as_ref(),
                assets::read_bounded(&path)?,
            ),
            None => response(404, "text/plain", b"Not Found".to_vec()),
        }
    }
}

fn response(status: u16, content_type: &str, body: Vec<u8>) -> Result<Response<Vec<u8>>> {
    Ok(Response::builder().status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; frame-src 'none'; base-uri 'self'; form-action 'none'")
        .body(body)?)
}

fn is_app_url(url: &url::Url) -> bool {
    let local = (url.scheme() == "webui" && url.host_str() == Some("app"))
        || (cfg!(target_os = "windows")
            && url.scheme() == "https"
            && url.host_str() == Some("webui.app"));
    local && url.port().is_none() && url.username().is_empty() && url.password().is_none()
}

struct HtmlWriter(Vec<u8>);

impl ResponseWriter for HtmlWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.0.extend_from_slice(content.as_bytes());
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn main() -> Result<()> {
    let snapshot = Snapshot::load(&Options::parse())?;
    #[cfg(feature = "desktop")]
    return open_window(snapshot);
    #[cfg(not(feature = "desktop"))]
    {
        drop(snapshot);
        bail!("no desktop backend in this build; run with the default features")
    }
}

#[cfg(feature = "desktop")]
fn open_window(snapshot: Snapshot) -> Result<()> {
    use std::sync::Arc;
    use tauri::{webview::NewWindowResponse, WebviewUrl, WebviewWindowBuilder};

    let snapshot = Arc::new(snapshot);
    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("webui", move |_context, request, responder| {
            let snapshot = Arc::clone(&snapshot);
            // Use Tauri's blocking pool so file reads never block the native UI.
            tauri::async_runtime::spawn_blocking(move || {
                let result = snapshot.respond(&request).unwrap_or_else(|error| {
                    eprintln!("WebUI request failed: {error:#}");
                    let mut result =
                        Response::new(b"Cannot serve this request; see host output.".to_vec());
                    *result.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
                    result
                });
                responder.respond(result);
            });
        })
        .setup(|app| {
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::CustomProtocol("webui://app/".parse()?),
            )
            .title("WebUI Tauri")
            .inner_size(1200.0, 800.0)
            .incognito(true)
            .use_https_scheme(true)
            .on_navigation(is_app_url)
            .on_new_window(|_, _| NewWindowResponse::Deny)
            .build()?;
            Ok(())
        })
        .run(context())?;
    Ok(())
}

#[cfg(feature = "desktop")]
// Tauri's macro generates unwrap/expect calls for validated build-time inputs.
#[allow(clippy::disallowed_methods)]
fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_test_utils::webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

    #[test]
    fn renders_once_and_serves_only_local_read_only_content() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let protocol = WebUIProtocol::new(std::collections::HashMap::from([(
            "index.html".into(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("title", false)],
                ..Default::default()
            },
        )]));
        std::fs::write(dir.path().join("protocol.bin"), protocol.to_protobuf()?)?;
        let state = dir.path().join("state.json");
        std::fs::write(&state, br#"{"title":"Hello <Tauri>"}"#)?;
        let snapshot = Snapshot::load(&Options {
            dist_dir: dir.path().into(),
            state: state.clone(),
            plugin: None,
            theme: None,
        })?;
        std::fs::write(&state, br#"{"title":"Changed"}"#)?;
        std::fs::create_dir(dir.path().join("assets"))?;
        std::fs::write(dir.path().join("assets/app.js"), "export {};")?;
        let page = snapshot.respond(&Request::builder().uri("webui://app/").body(Vec::new())?)?;
        assert_eq!(page.body(), b"Hello &lt;Tauri&gt;");
        assert_eq!(
            page.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        for (path, expected) in [
            ("assets/app.js", 200),
            ("protocol.bin", 404),
            ("state.json", 404),
            ("contacts/1", 404),
        ] {
            let result = snapshot.respond(
                &Request::builder()
                    .uri(format!("webui://app/{path}"))
                    .body(Vec::new())?,
            )?;
            assert_eq!(result.status().as_u16(), expected, "{path}");
        }
        let mutation = snapshot.respond(
            &Request::builder()
                .method("POST")
                .uri("webui://app/")
                .body(Vec::new())?,
        )?;
        assert_eq!(mutation.status(), 405);
        assert_eq!(mutation.headers()[header::ALLOW], "GET");
        let foreign =
            snapshot.respond(&Request::builder().uri("webui://evil/").body(Vec::new())?)?;
        assert_eq!(foreign.status(), 403);
        std::fs::write(&state, "null")?;
        assert!(Snapshot::load(&Options {
            dist_dir: dir.path().into(),
            state,
            plugin: None,
            theme: None,
        })
        .is_err());
        Ok(())
    }

    #[test]
    fn rejects_foreign_navigation_and_invalid_arguments() -> Result<()> {
        assert!(is_app_url(&url::Url::parse("webui://app/")?));
        assert_eq!(
            is_app_url(&url::Url::parse("https://webui.app/")?),
            cfg!(target_os = "windows")
        );
        for input in [
            "https://example.com",
            "webui://app:80/",
            "webui://user@app/",
            "file:///etc/passwd",
        ] {
            assert!(!is_app_url(&url::Url::parse(input)?));
        }
        assert!(Options::try_parse_from(["tauri", "dist"]).is_err());
        assert!(
            Options::try_parse_from(["tauri", "dist", "state.json", "--plugin=other"]).is_err()
        );
        Ok(())
    }

    fn themed_fixture() -> Result<(tempfile::TempDir, Options)> {
        let dir = tempfile::tempdir()?;
        let mut protocol = WebUIProtocol::new(std::collections::HashMap::from([(
            "index.html".into(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<style>:root{"),
                    WebUIFragment::signal("tokens.light", true),
                    WebUIFragment::raw("}@media(prefers-color-scheme:dark){:root{"),
                    WebUIFragment::signal("tokens.dark", true),
                    WebUIFragment::raw("}}</style>"),
                ],
                ..Default::default()
            },
        )]));
        protocol.tokens = vec!["color".into()];
        std::fs::write(dir.path().join("protocol.bin"), protocol.to_protobuf()?)?;
        let state = dir.path().join("state.json");
        std::fs::write(&state, br#"{"tokens":{"light":"--color:seed;"}}"#)?;
        let options = Options {
            dist_dir: dir.path().into(),
            state,
            plugin: None,
            theme: None,
        };
        Ok((dir, options))
    }

    #[test]
    fn resolves_file_and_package_themes_once_before_rendering() -> Result<()> {
        let (dir, mut options) = themed_fixture()?;
        assert!(std::str::from_utf8(&Snapshot::load(&options)?.html)?.contains("--color:seed;"));
        std::fs::write(&options.state, "{}")?;
        let package = dir.path().join("node_modules/@example/theme");
        std::fs::create_dir_all(&package)?;
        let theme = package.join("tokens.json");
        std::fs::write(
            &theme,
            r##"{"themes":{"light":{"color":"#123456"},"dark":{"color":"#abcdef"}}}"##,
        )?;
        for name in [
            theme.to_string_lossy().into_owned(),
            "@example/theme".into(),
        ] {
            options.theme = Some(name);
            let snapshot = Snapshot::load(&options)?;
            let html = std::str::from_utf8(&snapshot.html)?;
            assert!(html.contains(":root{--color: #123456;}"), "{html}");
            assert!(
                html.contains("@media(prefers-color-scheme:dark){:root{--color: #abcdef;}}"),
                "{html}"
            );
            assert_eq!(std::fs::read_to_string(&options.state)?, "{}");
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_or_oversized_theme_inputs_before_opening_a_window() -> Result<()> {
        let (dir, mut options) = themed_fixture()?;
        options.theme = Some("@missing/theme".into());
        assert!(Snapshot::load(&options).is_err());
        let theme = dir.path().join("tokens.json");
        options.theme = Some(theme.to_string_lossy().into_owned());
        for invalid in [b"null".as_slice(), b"{", b"\xff"] {
            std::fs::write(&theme, invalid)?;
            assert!(Snapshot::load(&options).is_err());
        }
        std::fs::File::create(&theme)?.set_len(assets::MAX_FILE_BYTES + 1)?;
        assert!(Snapshot::load(&options).is_err());
        std::fs::write(&theme, r##"{"color":"#123456"}"##)?;
        std::fs::write(&options.state, r#"{"tokens":null}"#)?;
        assert!(Snapshot::load(&options).is_err());
        Ok(())
    }
}
