// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use webui::DEFAULT_CSS_FILE_NAME_TEMPLATE;
use webui_desktop::{
    ApiContext, DesktopBundleConfig, DesktopBundleManifest, DesktopHttpMethod,
    DesktopProtocolResponse, DesktopRuntime, DesktopSourceConfig, RouteContext, RouteStateRegistry,
    WindowOptions,
};

type SharedState = Arc<RwLock<Value>>;

#[derive(Debug)]
struct ContactApiError {
    status: u16,
    message: String,
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn main() -> Result<()> {
    let (runtime, window) = match packaged_resources_dir() {
        Some(resources) => packaged_runtime(&resources)?,
        None => source_runtime()?,
    };

    run_platform_runtime(Arc::new(runtime), window)
}

#[cfg(target_os = "macos")]
fn run_platform_runtime(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Result<()> {
    webui_desktop_runner::macos::run_runtime(runtime, window)
}

#[cfg(target_os = "windows")]
fn run_platform_runtime(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Result<()> {
    webui_desktop_runner::windows::run_runtime(runtime, window)
}

#[cfg(target_os = "linux")]
fn run_platform_runtime(runtime: Arc<DesktopRuntime>, window: WindowOptions) -> Result<()> {
    webui_desktop_runner::linux::run_runtime(runtime, window)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn source_runtime() -> Result<(DesktopRuntime, WindowOptions)> {
    let root = workspace_root();
    let app_root = root.join("examples/app/contact-book-manager");
    let app_dir = root.join("examples/app/contact-book-manager/src");
    let state_path = root.join("examples/app/contact-book-manager/data/state.json");
    let assets = root.join("examples/app/contact-book-manager/dist");

    let state = Arc::new(RwLock::new(load_state(&state_path)?));
    let mut config = DesktopSourceConfig::new(contact_book_build_options(app_dir));
    config.state = Some(snapshot_state(&state)?);
    config.asset_root = Some(assets);
    config.theme = Some(("@microsoft/webui-examples-theme".to_string(), app_root));
    register_routes(&mut config.route_state, Arc::clone(&state))?;
    register_api_routes(&mut config.api_routes, Arc::clone(&state))?;
    let runtime = DesktopRuntime::from_source(config)?;

    let window = WindowOptions {
        title: "Contact Book Manager".to_string(),
        width: 1200,
        height: 800,
        devtools: true,
        ..WindowOptions::default()
    };
    Ok((runtime, window))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn packaged_runtime(resources: &std::path::Path) -> Result<(DesktopRuntime, WindowOptions)> {
    let manifest = DesktopBundleManifest::load(&resources.join("manifest.webui-desktop.json"))?;
    let state_path = resources.join("state.json");
    let state = Arc::new(RwLock::new(load_state(&state_path)?));
    let mut config = DesktopBundleConfig::new(resources.to_path_buf());
    config.state = Some(snapshot_state(&state)?);
    register_routes(&mut config.route_state, Arc::clone(&state))?;
    register_api_routes(&mut config.api_routes, Arc::clone(&state))?;
    let runtime = DesktopRuntime::from_bundle_config(config)?;
    Ok((runtime, manifest.window))
}

#[cfg(target_os = "macos")]
fn packaged_resources_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let contents = exe.parent().and_then(std::path::Path::parent)?;
    let resources = contents.join("Resources").join("webui");
    resources
        .join("manifest.webui-desktop.json")
        .is_file()
        .then_some(resources)
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn packaged_resources_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let resources = exe.parent()?.join("resources").join("webui");
    resources
        .join("manifest.webui-desktop.json")
        .is_file()
        .then_some(resources)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn main() -> Result<()> {
    println!("contact-book-desktop is not supported on this platform yet");
    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn contact_book_build_options(app_dir: PathBuf) -> webui::BuildOptions {
    webui::BuildOptions {
        app_dir,
        entry: "index.html".to_string(),
        css: webui::CssStrategy::Link,
        dom: webui::DomStrategy::Shadow,
        plugin: Some(webui::Plugin::WebUI),
        components: Vec::new(),
        component_asset_roots: Vec::new(),
        css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
        css_public_base: None,
        legal_comments: webui::LegalComments::Inline,
    }
}

fn register_routes(
    routes: &mut RouteStateRegistry,
    state: SharedState,
) -> webui_desktop::Result<()> {
    routes.route("/", {
        let state = Arc::clone(&state);
        move |_| Ok(dashboard_state(&snapshot_state(&state)?))
    })?;
    routes.route("/contacts", {
        let state = Arc::clone(&state);
        move |_| Ok(contacts_state(&snapshot_state(&state)?))
    })?;
    routes.route("/contacts/add", {
        let state = Arc::clone(&state);
        move |_| Ok(add_contact_state(&snapshot_state(&state)?))
    })?;
    routes.route("/contacts/:id/edit", {
        let state = Arc::clone(&state);
        move |ctx| edit_contact_state(&snapshot_state(&state)?, &ctx)
    })?;
    routes.route("/contacts/:id", {
        let state = Arc::clone(&state);
        move |ctx| contact_detail_state(&snapshot_state(&state)?, &ctx)
    })?;
    routes.route("/favorites", {
        let state = Arc::clone(&state);
        move |_| Ok(favorites_state(&snapshot_state(&state)?))
    })?;
    routes.route("/groups/:group", move |ctx| {
        group_state(&snapshot_state(&state)?, &ctx)
    })?;
    Ok(())
}

fn snapshot_state(state: &SharedState) -> webui_desktop::Result<Value> {
    state.read().map(|guard| guard.clone()).map_err(|_| {
        webui_desktop::DesktopError::UnsupportedRuntime {
            message: "contact book state lock is poisoned".to_string(),
            help: "restart the desktop app to reinitialize the in-memory state".to_string(),
        }
    })
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

fn load_state(path: &std::path::Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn contacts_collection_api(
    state: &SharedState,
    ctx: &ApiContext<'_>,
) -> webui_desktop::Result<DesktopProtocolResponse> {
    match ctx.method {
        DesktopHttpMethod::Get => {
            let state = snapshot_state(state)?;
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
            let state = snapshot_state(state)?;
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
            let state = snapshot_state(state)?;
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
