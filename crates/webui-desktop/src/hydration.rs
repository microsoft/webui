// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::WebUIHandler;

pub(crate) fn handler_for_name(plugin: Option<&str>) -> WebUIHandler {
    match plugin {
        Some("fast" | "fast-v2") => {
            WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some("fast-v3") => WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new())),
        Some("webui") => WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new())),
        _ => WebUIHandler::new(),
    }
}

#[cfg(feature = "source")]
pub(crate) fn handler_for_plugin(plugin: Option<webui::Plugin>) -> WebUIHandler {
    handler_for_name(plugin.map(plugin_name))
}

#[cfg(feature = "source")]
pub(crate) fn plugin_name(plugin: webui::Plugin) -> &'static str {
    match plugin {
        webui::Plugin::Fast | webui::Plugin::FastV2 => "fast",
        webui::Plugin::FastV3 => "fast-v3",
        webui::Plugin::WebUI => "webui",
    }
}
