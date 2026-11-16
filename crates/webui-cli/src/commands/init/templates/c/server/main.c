// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#include "host.h"

static int render_page(struct mg_connection *connection, struct host *app, const char *path, int head) {
    const char *state = strcmp(path, "/") == 0
        ? "{\"title\":\"Hello WebUI\",\"count\":0,\"ready\":false,\"isHome\":true}"
        : "{\"title\":\"About WebUI\",\"isHome\":false}";
    webui_render_t *result = webui_handler_render(app->handler, app->protocol, state, "index.html", path);
    if (!result) {
        host_log_native("Render document");
        return host_error(connection, head);
    }
    uintptr_t length = 0;
    const uint8_t *bytes = webui_render_bytes(result, &length);
    int status;
    if (!bytes) {
        host_log_native("Read render result");
        status = host_error(connection, head);
    } else {
        status = host_reply(connection, 200, "text/html; charset=utf-8", bytes, (size_t)length, head);
    }
    webui_render_free(result);
    return status;
}

int main(void) {
    struct host app;
    memset(&app, 0, sizeof(app));
    app.protocol = host_load_protocol();
    if (!app.protocol) return 1;
    app.handler = webui_handler_create_with_plugin("webui");
    if (!app.handler) {
        host_log_native("Create WebUI handler");
        webui_protocol_destroy(app.protocol);
        return 1;
    }
    app.page = render_page;
    int result = host_serve(&app);
    host_free_assets(&app);
    webui_handler_destroy(app.handler);
    webui_protocol_destroy(app.protocol);
    return result;
}
