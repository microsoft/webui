// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{Map, Value};
#[cfg(feature = "source")]
use webui_desktop::DEFAULT_CSS_FILE_NAME_TEMPLATE;
use webui_desktop::{
    ApiContext, DesktopApp, DesktopBundleConfig, DesktopBundleManifest, DesktopEvent,
    DesktopHttpMethod, DesktopProtocolResponse, EventResponse, RouteContext, RouteStateRegistry,
};
#[cfg(feature = "source")]
use webui_desktop::{DesktopSourceConfig, TitlebarStyle, WindowOptions};

mod state;
use state::{load_state, read_state, SharedState};
use webui_desktop::DesktopFrame;

#[derive(Debug)]
struct ContactApiError {
    status: u16,
    message: String,
}

fn main() -> Result<()> {
    #[cfg(feature = "source")]
    let frame = match packaged_resources_dir() {
        Some(resources) => packaged_frame(&resources)?,
        None => source_frame(workspace_root().join("examples/app/contact-book-manager"))?,
    };
    #[cfg(not(feature = "source"))]
    let frame = {
        let resources = packaged_resources_dir().ok_or_else(|| {
            anyhow::anyhow!(
                "packaged desktop resources were not found; rebuild with the source feature for development"
            )
        })?;
        packaged_frame(&resources)?
    };

    frame.on_event(|event| match event {
        DesktopEvent::WindowCloseRequested { .. } => {
            eprintln!("Contact Book close requested; unsaved-change checks belong here");
            EventResponse::Continue
        }
        _ => EventResponse::Continue,
    })?;
    webui_desktop::run_frame(frame)?;
    Ok(())
}

#[cfg(feature = "source")]
fn source_frame(app_root: PathBuf) -> Result<DesktopFrame> {
    let app_dir = app_root.join("src");
    let state_path = app_root.join("data/state.json");
    let assets = app_root.join("dist");

    let (seed, state) = load_state(&state_path)?;
    let mut config = DesktopSourceConfig::new(contact_book_build_options(app_dir));
    config.state = Some(seed);
    config.asset_root = Some(assets);
    config.theme = Some(("@microsoft/webui-examples-theme".to_string(), app_root));
    register_routes(&mut config.route_state, Arc::clone(&state))?;
    register_api_routes(&mut config.api_routes, Arc::clone(&state))?;
    config.window = WindowOptions {
        title: "Contact Book Manager".to_string(),
        width: 1200,
        height: 800,
        devtools: true,
        titlebar: TitlebarStyle::Overlay { height: 48 },
        background: Some("#f8fafc".parse()?),
        remember_state: true,
        ..WindowOptions::default()
    };
    Ok(DesktopApp::from_source(config)
        .app_id("com.microsoft.webui.contactbook")
        .build()?)
}

fn packaged_frame(resources: &std::path::Path) -> Result<DesktopFrame> {
    let manifest = DesktopBundleManifest::load(&resources.join("manifest.webui-desktop.json"))?;
    let state_path = resources.join("state.json");
    let (seed, state) = load_state(&state_path)?;
    let mut config = DesktopBundleConfig::new(resources.to_path_buf());
    config.state = Some(seed);
    register_routes(&mut config.route_state, Arc::clone(&state))?;
    register_api_routes(&mut config.api_routes, Arc::clone(&state))?;
    Ok(DesktopApp::from_bundle_config_and_manifest(config, manifest).build()?)
}

fn packaged_resources_dir() -> Option<PathBuf> {
    webui_desktop::find_packaged_resources_dir()
}

#[cfg(feature = "source")]
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(feature = "source")]
fn contact_book_build_options(app_dir: PathBuf) -> webui_desktop::BuildOptions {
    let projection_manifest = app_dir.join("../dist/webui-projection.json");
    webui_desktop::BuildOptions {
        app_dir,
        entry: "index.html".to_string(),
        css: webui_desktop::CssStrategy::Link,
        dom: webui_desktop::DomStrategy::Shadow,
        plugin: Some(webui_desktop::Plugin::WebUI),
        css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
        projection_manifests: vec![projection_manifest.into()],
        ..webui_desktop::BuildOptions::default()
    }
}

fn register_routes(
    routes: &mut RouteStateRegistry,
    state: SharedState,
) -> webui_desktop::Result<()> {
    routes.route("/", {
        let state = Arc::clone(&state);
        move |_| Ok(dashboard_state(&*read_state(&state)?))
    })?;
    routes.route("/contacts", {
        let state = Arc::clone(&state);
        move |_| Ok(contacts_state(&*read_state(&state)?))
    })?;
    routes.route("/contacts/add", {
        let state = Arc::clone(&state);
        move |_| Ok(add_contact_state(&*read_state(&state)?))
    })?;
    routes.route("/contacts/:id/edit", {
        let state = Arc::clone(&state);
        move |ctx| edit_contact_state(&*read_state(&state)?, &ctx)
    })?;
    routes.route("/contacts/:id", {
        let state = Arc::clone(&state);
        move |ctx| contact_detail_state(&*read_state(&state)?, &ctx)
    })?;
    routes.route("/favorites", {
        let state = Arc::clone(&state);
        move |_| Ok(favorites_state(&*read_state(&state)?))
    })?;
    routes.route("/groups/:group", move |ctx| {
        group_state(&*read_state(&state)?, &ctx)
    })?;
    Ok(())
}

fn register_api_routes(
    routes: &mut webui_desktop::ApiRouteRegistry,
    state: SharedState,
) -> webui_desktop::Result<()> {
    routes.route("/api/contacts", {
        let state = Arc::clone(&state);
        move |ctx| contacts_collection_api(&state, &ctx)
    })?;
    routes.route("/api/contacts/:id", {
        let state = Arc::clone(&state);
        move |ctx| contact_item_api(&state, &ctx)
    })?;
    routes.route("/api/stats", move |ctx| stats_api(&state, &ctx))?;
    Ok(())
}

fn contacts_collection_api(
    state: &SharedState,
    ctx: &ApiContext<'_>,
) -> webui_desktop::Result<DesktopProtocolResponse> {
    match ctx.method {
        DesktopHttpMethod::Get => {
            let state = read_state(state)?;
            json_response(200, Value::Array(contacts(&state).to_vec()))
        }
        DesktopHttpMethod::Post => mutate_api(state, |store| create_contact(store, ctx.body)),
        _ => Ok(method_not_allowed()),
    }
}

fn contact_item_api(
    state: &SharedState,
    ctx: &ApiContext<'_>,
) -> webui_desktop::Result<DesktopProtocolResponse> {
    match ctx.method {
        DesktopHttpMethod::Get => {
            let state = read_state(state)?;
            let Some(contact) = find_contact(contacts(&state), ctx.param("id").unwrap_or_default())
            else {
                return json_error(404, "Contact not found");
            };
            json_response(200, contact.clone())
        }
        DesktopHttpMethod::Other(method) if method == "PUT" => mutate_api(state, |store| {
            update_contact(store, ctx.param("id"), ctx.body)
        }),
        DesktopHttpMethod::Other(method) if method == "DELETE" => {
            mutate_api(state, |store| delete_contact(store, ctx.param("id")))
        }
        _ => Ok(method_not_allowed()),
    }
}

fn stats_api(
    state: &SharedState,
    ctx: &ApiContext<'_>,
) -> webui_desktop::Result<DesktopProtocolResponse> {
    match ctx.method {
        DesktopHttpMethod::Get => {
            let state = read_state(state)?;
            json_response(
                200,
                Value::Object(sidebar_state(contacts(&state), groups(&state))),
            )
        }
        _ => Ok(method_not_allowed()),
    }
}

fn mutate_api<F>(state: &SharedState, mutate: F) -> webui_desktop::Result<DesktopProtocolResponse>
where
    F: FnOnce(&mut Value) -> std::result::Result<Option<Value>, ContactApiError>,
{
    let mut guard = state
        .write()
        .map_err(|_| webui_desktop::DesktopError::UnsupportedRuntime {
            message: "contact book state lock is poisoned".to_string(),
            help: "restart the desktop app to reinitialize the in-memory state".to_string(),
        })?;
    match mutate(&mut guard) {
        Ok(Some(value)) => json_response(200, value),
        Ok(None) => Ok(DesktopProtocolResponse::new(
            204,
            "application/json",
            Vec::new(),
        )),
        Err(err) => json_error(err.status, &err.message),
    }
}

fn json_response(status: u16, value: Value) -> webui_desktop::Result<DesktopProtocolResponse> {
    serde_json::to_vec(&value)
        .map(|body| DesktopProtocolResponse::new(status, "application/json", body))
        .map_err(|source| webui_desktop::DesktopError::Serialization {
            context: "serializing Contact Book API response".to_string(),
            source,
        })
}

fn json_error(status: u16, message: &str) -> webui_desktop::Result<DesktopProtocolResponse> {
    let mut map = Map::new();
    map.insert("error".to_string(), Value::String(message.to_string()));
    json_response(status, Value::Object(map))
}

fn method_not_allowed() -> DesktopProtocolResponse {
    DesktopProtocolResponse::new(
        405,
        "application/json",
        br#"{"error":"Method not allowed"}"#.to_vec(),
    )
}

fn dashboard_state(state: &Value) -> Value {
    let contacts = contacts(state);
    let mut out = sidebar_state(contacts, groups(state));
    out.insert("page".to_string(), Value::String("dashboard".to_string()));
    out.insert(
        "recentContacts".to_string(),
        Value::Array(recent_contacts(contacts, 5)),
    );
    Value::Object(out)
}

fn contacts_state(state: &Value) -> Value {
    let contacts = contacts(state);
    let mut out = sidebar_state(contacts, groups(state));
    out.insert("page".to_string(), Value::String("contacts".to_string()));
    out.insert("contacts".to_string(), Value::Array(contacts.to_vec()));
    Value::Object(out)
}

fn add_contact_state(state: &Value) -> Value {
    let group_list = groups(state);
    let mut out = sidebar_state(contacts(state), group_list);
    out.insert("page".to_string(), Value::String("contacts".to_string()));
    out.insert(
        "selectedGroup".to_string(),
        group_list.first().cloned().unwrap_or_default(),
    );
    out.insert(
        "formTitle".to_string(),
        Value::String("Add Contact".to_string()),
    );
    Value::Object(out)
}

fn contact_detail_state(state: &Value, ctx: &RouteContext<'_>) -> webui_desktop::Result<Value> {
    let contacts = contacts(state);
    let mut out = sidebar_state(contacts, groups(state));
    out.insert("page".to_string(), Value::String("contacts".to_string()));
    let id = ctx.param("id").unwrap_or_default();
    if let Some(contact) = find_contact(contacts, id) {
        merge_contact(&mut out, contact);
        out.insert("selectedContact".to_string(), contact.clone());
    }
    Ok(Value::Object(out))
}

fn edit_contact_state(state: &Value, ctx: &RouteContext<'_>) -> webui_desktop::Result<Value> {
    let contacts = contacts(state);
    let mut out = sidebar_state(contacts, groups(state));
    out.insert("page".to_string(), Value::String("contacts".to_string()));
    let id = ctx.param("id").unwrap_or_default();
    if let Some(contact) = find_contact(contacts, id) {
        merge_contact(&mut out, contact);
        out.insert("editId".to_string(), Value::String(id.to_string()));
        out.insert(
            "selectedGroup".to_string(),
            contact.get("group").cloned().unwrap_or_default(),
        );
        out.insert(
            "formTitle".to_string(),
            Value::String("Edit Contact".to_string()),
        );
    }
    Ok(Value::Object(out))
}

fn favorites_state(state: &Value) -> Value {
    let contacts = contacts(state);
    let mut out = sidebar_state(contacts, groups(state));
    out.insert("page".to_string(), Value::String("favorites".to_string()));
    out.insert(
        "contacts".to_string(),
        Value::Array(
            contacts
                .iter()
                .filter(|contact| contact.get("favorite").and_then(Value::as_bool) == Some(true))
                .cloned()
                .collect(),
        ),
    );
    Value::Object(out)
}

fn group_state(state: &Value, ctx: &RouteContext<'_>) -> webui_desktop::Result<Value> {
    let contacts = contacts(state);
    let group = ctx.param("group").unwrap_or_default();
    let filtered: Vec<Value> = contacts
        .iter()
        .filter(|contact| {
            contact
                .get("group")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(group))
        })
        .cloned()
        .collect();
    let display = filtered
        .first()
        .and_then(|contact| contact.get("group"))
        .and_then(Value::as_str)
        .unwrap_or(group);
    let mut out = sidebar_state(contacts, groups(state));
    out.insert("page".to_string(), Value::String("group".to_string()));
    out.insert(
        "activeGroup".to_string(),
        Value::String(display.to_string()),
    );
    out.insert("groupName".to_string(), Value::String(display.to_string()));
    out.insert("contacts".to_string(), Value::Array(filtered));
    Ok(Value::Object(out))
}

fn sidebar_state(contacts: &[Value], groups: &[Value]) -> Map<String, Value> {
    let favorite_count = contacts
        .iter()
        .filter(|contact| contact.get("favorite").and_then(Value::as_bool) == Some(true))
        .count();
    let mut out = Map::new();
    out.insert("mode".to_string(), Value::String("desktop".to_string()));
    out.insert("totalContacts".to_string(), Value::from(contacts.len()));
    out.insert("totalFavorites".to_string(), Value::from(favorite_count));
    out.insert("totalGroups".to_string(), Value::from(groups.len()));
    out.insert("groups".to_string(), Value::Array(groups.to_vec()));
    out
}

fn contacts(state: &Value) -> &[Value] {
    state
        .get("contacts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn groups(state: &Value) -> &[Value] {
    state
        .get("groups")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn recent_contacts(contacts: &[Value], count: usize) -> Vec<Value> {
    let start = contacts.len().saturating_sub(count);
    let mut recent = Vec::with_capacity(contacts.len() - start);
    for contact in contacts[start..].iter().rev() {
        recent.push(contact.clone());
    }
    recent
}

fn find_contact<'a>(contacts: &'a [Value], id: &str) -> Option<&'a Value> {
    contacts
        .iter()
        .find(|contact| contact.get("id").and_then(Value::as_str) == Some(id))
}

fn merge_contact(out: &mut Map<String, Value>, contact: &Value) {
    let Some(map) = contact.as_object() else {
        return;
    };
    for (key, value) in map {
        out.insert(key.clone(), value.clone());
    }
}

fn create_contact(
    state: &mut Value,
    payload: &[u8],
) -> std::result::Result<Option<Value>, ContactApiError> {
    let body = parse_payload(payload)?;
    let contact_count = contacts(state).len();
    let contact = contact_from_body(&body, None, contact_count);
    ensure_group(
        state,
        contact
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("Other"),
    );
    contacts_mut(state)?.push(contact.clone());
    Ok(Some(contact))
}

fn update_contact(
    state: &mut Value,
    id: Option<&str>,
    payload: &[u8],
) -> std::result::Result<Option<Value>, ContactApiError> {
    let body = parse_payload(payload)?;
    let id = id.ok_or_else(|| missing_field("id"))?;
    let data = &body;
    let contacts = contacts_mut(state)?;
    let Some(index) = contacts
        .iter()
        .position(|contact| contact.get("id").and_then(Value::as_str) == Some(id))
    else {
        return Err(not_found(id));
    };
    let updated = contact_from_body(data, Some(&contacts[index]), contacts.len());
    contacts[index] = updated.clone();
    ensure_group(
        state,
        updated
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("Other"),
    );
    Ok(Some(updated))
}

fn delete_contact(
    state: &mut Value,
    id: Option<&str>,
) -> std::result::Result<Option<Value>, ContactApiError> {
    let id = id.ok_or_else(|| missing_field("id"))?;
    let contacts = contacts_mut(state)?;
    let Some(index) = contacts
        .iter()
        .position(|contact| contact.get("id").and_then(Value::as_str) == Some(id))
    else {
        return Err(not_found(id));
    };
    contacts.remove(index);
    Ok(None)
}

fn parse_payload(payload: &[u8]) -> std::result::Result<Value, ContactApiError> {
    serde_json::from_slice(payload).map_err(|err| ContactApiError {
        status: 400,
        message: format!("failed to parse contact mutation payload: {err}"),
    })
}

fn contact_from_body(body: &Value, existing: Option<&Value>, contact_count: usize) -> Value {
    let first_name = contact_string(body, existing, "firstName", "");
    let last_name = contact_string(body, existing, "lastName", "");
    let mut contact = Map::new();
    contact.insert(
        "id".to_string(),
        Value::String(
            existing
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| next_contact_id(contact_count)),
        ),
    );
    contact.insert("firstName".to_string(), Value::String(first_name.clone()));
    contact.insert("lastName".to_string(), Value::String(last_name.clone()));
    contact.insert(
        "email".to_string(),
        Value::String(contact_string(body, existing, "email", "")),
    );
    contact.insert(
        "phone".to_string(),
        Value::String(contact_string(body, existing, "phone", "")),
    );
    contact.insert(
        "company".to_string(),
        Value::String(contact_string(body, existing, "company", "")),
    );
    contact.insert(
        "group".to_string(),
        Value::String(contact_string(body, existing, "group", "Other")),
    );
    contact.insert(
        "favorite".to_string(),
        Value::Bool(contact_bool(body, existing, "favorite", false)),
    );
    contact.insert(
        "initials".to_string(),
        Value::String(compute_initials(&first_name, &last_name)),
    );
    contact.insert(
        "avatarColor".to_string(),
        Value::String(contact_string(
            body,
            existing,
            "avatarColor",
            avatar_color(contact_count),
        )),
    );
    contact.insert(
        "notes".to_string(),
        Value::String(contact_string(body, existing, "notes", "")),
    );
    contact.insert(
        "address".to_string(),
        Value::String(contact_string(body, existing, "address", "")),
    );
    Value::Object(contact)
}

fn contact_string(body: &Value, existing: Option<&Value>, key: &str, default: &str) -> String {
    body.get(key)
        .and_then(Value::as_str)
        .or_else(|| {
            existing
                .and_then(|value| value.get(key))
                .and_then(Value::as_str)
        })
        .unwrap_or(default)
        .to_string()
}

fn contact_bool(body: &Value, existing: Option<&Value>, key: &str, default: bool) -> bool {
    body.get(key)
        .and_then(Value::as_bool)
        .or_else(|| {
            existing
                .and_then(|value| value.get(key))
                .and_then(Value::as_bool)
        })
        .unwrap_or(default)
}

fn next_contact_id(contact_count: usize) -> String {
    let mut id = String::with_capacity(16);
    id.push_str("desktop-");
    id.push_str(&(contact_count + 1).to_string());
    id
}

fn compute_initials(first_name: &str, last_name: &str) -> String {
    let mut initials = String::with_capacity(2);
    if let Some(ch) = first_name.chars().next() {
        initials.push(ch.to_ascii_uppercase());
    }
    if let Some(ch) = last_name.chars().next() {
        initials.push(ch.to_ascii_uppercase());
    }
    initials
}

fn avatar_color(contact_count: usize) -> &'static str {
    const AVATAR_COLORS: [&str; 10] = [
        "#4A90D9", "#E74C3C", "#2ECC71", "#F39C12", "#9B59B6", "#1ABC9C", "#E67E22", "#3498DB",
        "#E91E63", "#00BCD4",
    ];
    AVATAR_COLORS[contact_count % AVATAR_COLORS.len()]
}

fn contacts_mut(state: &mut Value) -> std::result::Result<&mut Vec<Value>, ContactApiError> {
    state
        .get_mut("contacts")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ContactApiError {
            status: 500,
            message: "contact book state does not contain a contacts array".to_string(),
        })
}

fn ensure_group(state: &mut Value, group: &str) {
    if group.is_empty() {
        return;
    }
    let Some(groups) = state.get_mut("groups").and_then(Value::as_array_mut) else {
        return;
    };
    if !groups.iter().any(|value| value.as_str() == Some(group)) {
        groups.push(Value::String(group.to_string()));
    }
}

fn missing_field(field: &str) -> ContactApiError {
    ContactApiError {
        status: 400,
        message: format!("missing required field '{field}'"),
    }
}

fn not_found(id: &str) -> ContactApiError {
    ContactApiError {
        status: 404,
        message: format!("contact '{id}' was not found"),
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    fn test_state() -> Value {
        serde_json::from_str(
            r##"{
              "groups":["Work","Friends"],
              "contacts":[
                {"id":"1","firstName":"Ada","lastName":"Lovelace","email":"ada@example.com","phone":"1","company":"Analytical Engines","group":"Work","favorite":false,"initials":"AL","avatarColor":"#111","notes":"","address":""},
                {"id":"2","firstName":"Grace","lastName":"Hopper","email":"grace@example.com","phone":"2","company":"","group":"Friends","favorite":true,"initials":"GH","avatarColor":"#222","notes":"Compiler pioneer","address":"Arlington"}
              ]
            }"##,
        )
        .unwrap()
    }

    fn payload(value: &str) -> Vec<u8> {
        value.as_bytes().to_vec()
    }

    #[cfg(feature = "source")]
    fn source_fixture(initial: &Value) -> tempfile::TempDir {
        use webui_protocol::projection_manifest::{
            ProjectionAdapter, ProjectionManifest, ProjectionProducer, PRODUCER_NAME, SCHEMA_ID,
        };

        let root = tempfile::tempdir().unwrap();
        for (path, content) in [
            (
                "src/index.html",
                "<!doctype html><html><head><style>\
                 :root{/*{{{tokens.light}}}*/}\
                 @media (prefers-color-scheme:dark){:root{/*{{{tokens.dark}}}*/}}\
                 </style></head><body>\
                 <route path=\"/\" component=\"contact-shell\">\
                 <route path=\"\" component=\"contact-view\" exact />\
                 <route path=\"contacts\" component=\"contact-view\" exact />\
                 <route path=\"contacts/add\" component=\"contact-view\" exact />\
                 <route path=\"contacts/:id\" component=\"contact-view\" exact />\
                 <route path=\"contacts/:id/edit\" component=\"contact-view\" exact />\
                 <route path=\"favorites\" component=\"contact-view\" exact />\
                 <route path=\"groups/:group\" component=\"contact-view\" exact />\
                 </route><script type=\"module\" src=\"/app.js\"></script></body></html>",
            ),
            (
                "src/contact-shell.html",
                "<header data-mode=\"{{mode}}\" webui-drag>Contact Book</header><outlet />",
            ),
            (
                "src/contact-view.html",
                "<h1>{{firstName}}</h1><p>{{selectedGroup}}</p>\
                 <for each=\"contact in contacts\"><p>{{contact.firstName}}</p></for>",
            ),
            ("dist/app.js", "export {};"),
            (
                "node_modules/@microsoft/webui-examples-theme/tokens.json",
                r#"{"themes":{"light":{},"dark":{}}}"#,
            ),
        ] {
            let path = root.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        std::fs::create_dir(root.path().join("data")).unwrap();
        std::fs::write(
            root.path().join("data/state.json"),
            serde_json::to_vec(initial).unwrap(),
        )
        .unwrap();
        // These scriptless fixture components use exact template-derived state
        // keys. Validate a real manifest without requiring a frontend bundler.
        let mut manifest = ProjectionManifest {
            schema: SCHEMA_ID.into(),
            producer: ProjectionProducer {
                name: PRODUCER_NAME.into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            adapter: ProjectionAdapter {
                name: "test".into(),
                bundler: "test@1.0.0".into(),
            },
            root: ".".into(),
            analysis_hash: format!("sha256:{}", "1".repeat(64)),
            build_id: String::new(),
            inputs: Default::default(),
            outputs: Default::default(),
            components: Default::default(),
            entry_closures: Default::default(),
        };
        manifest.build_id = manifest.compute_build_id();
        std::fs::write(
            root.path().join("dist/webui-projection.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        root
    }

    #[test]
    fn route_state_is_unchanged_when_the_store_is_borrowed() {
        let original = test_state();
        let (_, store) = state::parse_state(&serde_json::to_vec(&original).unwrap()).unwrap();
        {
            let first = read_state(&store).unwrap();
            let second = read_state(&store).unwrap();
            assert!(std::ptr::eq(&*first, &*second));
            assert_eq!(dashboard_state(&first), dashboard_state(&original));
            assert_eq!(contacts_state(&first), contacts_state(&original));
            assert_eq!(favorites_state(&first), favorites_state(&original));
            assert_eq!(add_contact_state(&first), add_contact_state(&original));
        }

        update_contact(
            &mut store.write().unwrap(),
            Some("1"),
            br#"{"favorite":true}"#,
        )
        .unwrap();
        let state = read_state(&store).unwrap();
        assert_eq!(dashboard_state(&state)["totalFavorites"], 2);
        assert_eq!(
            favorites_state(&state)["contacts"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[cfg(feature = "source")]
    #[test]
    fn source_and_packaged_frames_render_native_overlay_chrome() {
        use webui_desktop::{build_desktop_bundle, DesktopBundleOptions, DesktopShellConfig};

        let fixture = source_fixture(&test_state());
        let app_root = fixture.path();
        let source = source_frame(app_root.to_path_buf()).unwrap();
        assert_eq!(
            source.window().titlebar,
            TitlebarStyle::Overlay { height: 48 }
        );
        assert!(source.runtime().startup_html().contains("mode=\"desktop\""));
        assert!(source
            .runtime()
            .startup_html()
            .contains("--webui-titlebar-height:48px"));
        let response = source
            .runtime()
            .handle_request(&webui_desktop::DesktopProtocolRequest::get("/app.js"))
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body.into_bytes().unwrap().as_slice(),
            b"export {};"
        );

        let package: Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
        let window: WindowOptions =
            serde_json::from_value(package["webuiDesktop"].clone()).unwrap();
        assert_eq!(window.titlebar, TitlebarStyle::Overlay { height: 48 });
        let bundle = tempfile::tempdir().unwrap();
        build_desktop_bundle(DesktopBundleOptions {
            build_options: contact_book_build_options(app_root.join("src")),
            out_dir: bundle.path().to_path_buf(),
            state_file: Some(app_root.join("data/state.json")),
            asset_root: Some(app_root.join("dist")),
            token_css: None,
            app_id: "com.microsoft.webui.contactbook.test".to_string(),
            app_name: "Contact Book Manager".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window,
            icon_file: None,
            shell: DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap();

        let packaged = packaged_frame(bundle.path()).unwrap();
        assert_eq!(
            packaged.window().titlebar,
            TitlebarStyle::Overlay { height: 48 }
        );
        assert!(packaged
            .runtime()
            .startup_html()
            .contains("--webui-titlebar-height:48px"));
        assert!(packaged
            .runtime()
            .startup_html()
            .contains("mode=\"desktop\""));
        let response = packaged
            .runtime()
            .handle_request(&webui_desktop::DesktopProtocolRequest::get("/app.js"))
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body.into_bytes().unwrap().as_slice(),
            b"export {};"
        );
        let response = packaged
            .runtime()
            .handle_request(&webui_desktop::DesktopProtocolRequest {
                method: DesktopHttpMethod::Get,
                path: "/contacts",
                body: &[],
                wants_json: true,
            })
            .unwrap();
        let partial: Value = serde_json::from_slice(response.body.as_bytes().unwrap()).unwrap();
        assert_eq!(partial["state"]["mode"], "desktop");
    }

    #[cfg(feature = "source")]
    #[test]
    fn source_launch_still_requires_completed_projection_metadata() {
        let fixture = source_fixture(&test_state());
        std::fs::remove_file(fixture.path().join("dist/webui-projection.json")).unwrap();
        let error = match source_frame(fixture.path().to_path_buf()) {
            Ok(_) => panic!("missing projection metadata must not silently disable projection"),
            Err(error) => error,
        };
        assert!(format!("{error:#}").contains("PROJ-M001"));
    }

    #[cfg(feature = "source")]
    #[test]
    fn registered_api_mutations_reach_parameterized_routes_and_preserve_tokens() {
        use webui_desktop::DesktopProtocolRequest;

        let mut initial = test_state();
        initial["tokens"] = serde_json::json!({
            "light": "--desktop-test-light:1;",
            "dark": "--desktop-test-dark:1;"
        });
        let (seed, store) = state::parse_state(&serde_json::to_vec(&initial).unwrap()).unwrap();
        let fixture = source_fixture(&initial);
        let app = fixture.path().join("src");
        let mut config = DesktopSourceConfig::new(contact_book_build_options(app));
        config.state = Some(seed);
        register_routes(&mut config.route_state, Arc::clone(&store)).unwrap();
        register_api_routes(&mut config.api_routes, store).unwrap();
        let runtime = webui_desktop::DesktopRuntime::from_source(config).unwrap();
        let request_json = |method, path: &str, body: &[u8], status| {
            let response = runtime
                .handle_request(&DesktopProtocolRequest {
                    method,
                    path,
                    body,
                    wants_json: true,
                })
                .unwrap();
            assert_eq!(response.status, status, "{path}");
            if status == 204 {
                assert!(response.body.as_bytes().unwrap().is_empty());
                Value::Null
            } else {
                serde_json::from_slice::<Value>(response.body.as_bytes().unwrap()).unwrap()
            }
        };

        let created = request_json(
            DesktopHttpMethod::Post,
            "/api/contacts",
            br#"{"firstName":"Regression","lastName":"Contact","group":"Space"}"#,
            200,
        );
        let id = created["id"].as_str().unwrap();
        let api_path = format!("/api/contacts/{id}");
        request_json(
            DesktopHttpMethod::Other("PUT".to_string()),
            &api_path,
            br#"{"firstName":"Updated","favorite":true,"group":"Systems"}"#,
            200,
        );
        let edit_path = format!("/contacts/{id}/edit");
        let edit = request_json(DesktopHttpMethod::Get, &edit_path, &[], 200);
        assert_eq!(edit["state"]["firstName"], "Updated");
        assert_eq!(edit["state"]["selectedGroup"], "Systems");
        assert!(edit["state"].get("tokens").is_none());
        assert_eq!(edit["state"]["mode"], "desktop");

        let group = request_json(DesktopHttpMethod::Get, "/groups/Systems", &[], 200);
        assert_eq!(group["state"]["mode"], "desktop");
        assert_eq!(group["state"]["contacts"].as_array().unwrap().len(), 1);
        assert_eq!(group["state"]["contacts"][0]["id"], id);
        let favorites = request_json(DesktopHttpMethod::Get, "/favorites", &[], 200);
        assert_eq!(favorites["state"]["mode"], "desktop");
        assert_eq!(favorites["state"]["contacts"].as_array().unwrap().len(), 2);
        let stats = request_json(DesktopHttpMethod::Get, "/api/stats", &[], 200);
        assert_eq!(stats["totalContacts"], 3);
        assert_eq!(stats["totalFavorites"], 2);

        let response = runtime
            .handle_request(&DesktopProtocolRequest {
                method: DesktopHttpMethod::Get,
                path: &edit_path,
                body: &[],
                wants_json: false,
            })
            .unwrap();
        let html = std::str::from_utf8(response.body.as_bytes().unwrap()).unwrap();
        let head = html.split("</head>").next().unwrap();
        assert!(head.contains("--desktop-test-light:1;"));
        assert!(head.contains("--desktop-test-dark:1;"));
        assert!(html.contains("Updated"));

        request_json(
            DesktopHttpMethod::Other("DELETE".to_string()),
            &api_path,
            &[],
            204,
        );
        let favorites = request_json(DesktopHttpMethod::Get, "/favorites", &[], 200);
        assert_eq!(favorites["state"]["contacts"].as_array().unwrap().len(), 1);
        let stats = request_json(DesktopHttpMethod::Get, "/api/stats", &[], 200);
        assert_eq!(stats["totalContacts"], 2);
        assert_eq!(stats["totalFavorites"], 1);
    }

    #[test]
    fn create_contact_mutates_state_and_adds_group() {
        let mut state = test_state();

        let created = create_contact(
            &mut state,
            &payload(
                r#"{"firstName":"Katherine","lastName":"Johnson","email":"kj@example.com","group":"Space"}"#,
            ),
        )
        .unwrap()
        .unwrap();

        assert_eq!(created["firstName"], "Katherine");
        assert_eq!(created["initials"], "KJ");
        assert_eq!(contacts(&state).len(), 3);
        assert!(groups(&state)
            .iter()
            .any(|group| group.as_str() == Some("Space")));
    }

    #[test]
    fn update_contact_preserves_missing_fields_and_route_state_reflects_change() {
        let mut state = test_state();

        let updated = update_contact(
            &mut state,
            Some("1"),
            &payload(r#"{"firstName":"Augusta","group":"Math"}"#),
        )
        .unwrap()
        .unwrap();

        assert_eq!(updated["id"], "1");
        assert_eq!(updated["firstName"], "Augusta");
        assert_eq!(updated["lastName"], "Lovelace");
        assert_eq!(updated["initials"], "AL");
        assert!(groups(&state)
            .iter()
            .any(|group| group.as_str() == Some("Math")));

        let changed = find_contact(contacts(&state), "1").unwrap();
        assert_eq!(changed["firstName"], "Augusta");
        assert_eq!(changed["group"], "Math");
    }

    #[test]
    fn update_contact_can_toggle_favorite() {
        let mut state = test_state();

        let toggled = update_contact(&mut state, Some("1"), &payload(r#"{"favorite":true}"#))
            .unwrap()
            .unwrap();

        assert_eq!(toggled["favorite"], true);
        let dashboard = dashboard_state(&state);
        assert_eq!(dashboard["totalFavorites"], 2);
    }

    #[test]
    fn delete_contact_removes_from_route_state() {
        let mut state = test_state();

        let deleted = delete_contact(&mut state, Some("2")).unwrap();

        assert!(deleted.is_none());
        assert_eq!(contacts(&state).len(), 1);
        let favorites = favorites_state(&state);
        assert_eq!(favorites["contacts"].as_array().map(Vec::len), Some(0));
    }
}
