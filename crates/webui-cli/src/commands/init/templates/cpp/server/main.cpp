// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#include "host.h"
#include <cstring>
#include <memory>

namespace {
using Protocol = std::unique_ptr<webui_protocol_t, decltype(&webui_protocol_destroy)>;
using Handler = std::unique_ptr<void, decltype(&webui_handler_destroy)>;
using Document = std::unique_ptr<webui_render_t, decltype(&webui_render_free)>;

struct Assets {
    host &app;
    ~Assets() { host_free_assets(&app); }
    Assets(const Assets &) = delete;
    Assets &operator=(const Assets &) = delete;
    explicit Assets(host &value) : app(value) {}
};

int render_page(mg_connection *connection, host *app, const char *path, int head) {
    const char *state = std::strcmp(path, "/") == 0
        ? R"({"title":"Hello WebUI","count":0,"ready":false,"isHome":true})"
        : R"({"title":"About WebUI","isHome":false})";
    Document document(webui_handler_render(app->handler, app->protocol, state, "index.html", path), webui_render_free);
    if (!document) {
        host_log_native("Render document");
        return host_error(connection, head);
    }
    uintptr_t length = 0;
    const uint8_t *bytes = webui_render_bytes(document.get(), &length);
    if (!bytes) {
        host_log_native("Read render result");
        return host_error(connection, head);
    }
    return host_reply(connection, 200, "text/html; charset=utf-8", bytes, static_cast<size_t>(length), head);
}
} // namespace

int main() {
    Protocol protocol(host_load_protocol(), webui_protocol_destroy);
    if (!protocol) return 1;
    Handler handler(webui_handler_create_with_plugin("webui"), webui_handler_destroy);
    if (!handler) {
        host_log_native("Create WebUI handler");
        return 1;
    }
    host app{};
    app.protocol = protocol.get();
    app.handler = handler.get();
    app.page = render_page;
    Assets assets(app);
    return host_serve(&app);
}
