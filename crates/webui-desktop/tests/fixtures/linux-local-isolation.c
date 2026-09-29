// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Actual WebKitGTK named-world trust probe using the SDK's GENERATED mediator.
// This separate permissive test view loads a cross-origin sandboxed iframe.
// The product local-server view still denies subframe document navigation.
#include <gtk/gtk.h>
#include <webkit/webkit.h>
#include <jsc/jsc.h>
#include <stdio.h>
#include <string.h>

#define WORLD "webui.local.ipc.private"
#define MARKER "webui.linux.local.ipc.v1"

static GMainLoop *loop;
static guint controls, data, reports;
static gboolean child_clean;
static guint top_port, child_port;
static gboolean finishing;

static gboolean finish(gpointer unused) {
    printf("LINUX_IPC_CHILD_PROBE controls=%u data=%u reports=%u child_clean=%d\n",
           controls, data, reports, child_clean);
    fflush(stdout);
    g_main_loop_quit(loop);
    return G_SOURCE_REMOVE;
}

static void on_message(WebKitUserContentManager *manager, JSCValue *value,
                       WebKitScriptMessageReply *reply, gpointer lane) {
    gchar *json = jsc_value_to_json(value, 0);
    if (json && strlen(json) < 2048) {
        if (lane == GINT_TO_POINTER(1)) {
            if (strstr(json, "\"kind\":\"disconnect\"")) controls++;
            else if (strstr(json, "\"kind\":\"testReport\"")) {
                reports++;
                child_clean = strstr(json, "\"childRaw\":\"undefined\"") &&
                              strstr(json, "\"childData\":\"undefined\"") &&
                              strstr(json, "\"childBootstrap\":\"undefined\"") &&
                              strstr(json, "\"topRaw\":\"undefined\"");
            }
        } else if (strstr(json, "\"kind\":\"ipcData\"")) data++;
    }
    g_free(json);
    JSCContext *context = jsc_value_get_context(value);
    JSCValue *ack = jsc_value_new_string(context, "ack");
    webkit_script_message_reply_return_value(reply, ack);
    g_object_unref(ack);
    if (!finishing && controls >= 1 && data >= 1 && reports >= 1) {
        finishing = TRUE;
        g_timeout_add(300, finish, NULL);
    }
}

static gboolean serve(GThreadedSocketService *service, GSocketConnection *connection,
                      GObject *source, gpointer unused) {
    char request[2048] = {0};
    GInputStream *input = g_io_stream_get_input_stream(G_IO_STREAM(connection));
    GOutputStream *output = g_io_stream_get_output_stream(G_IO_STREAM(connection));
    gssize size = g_input_stream_read(input, request, sizeof(request) - 1, NULL, NULL);
    const gboolean child = size > 0 && strstr(request, "GET /child ") != NULL;
    gchar *page;
    if (child) {
        page = g_strdup(
            "<!doctype html><script>"
            "addEventListener('message',e=>{if(e.data?.kind!=='credential')return;"
            "let raw=typeof window.webkit?.messageHandlers?.webuiDesktopIpc;"
            "let data=typeof window.webkit?.messageHandlers?.webuiDesktopIpcData;"
            "let bootstrap=typeof window.__webuiDesktopIpcV2;"
            "parent.postMessage({marker:'" MARKER "',direction:'page-to-native',lane:'control',"
            "id:71,payload:{kind:'disconnect',generation:e.data.generation,token:e.data.token}},'*');"
            "parent.postMessage({marker:'" MARKER "',direction:'page-to-native',lane:'data',"
            "id:72,payload:{kind:'ipcData',version:1,callId:'1',generation:e.data.generation,"
            "token:e.data.token,operation:'receive',offset:0,maxBytes:64}},'*');"
            "parent.postMessage({kind:'childChecked',raw,data,bootstrap},'*')})"
            "</script>");
    } else {
        page = g_strdup_printf(
            "<!doctype html><script>"
            "const marker='" MARKER "',token='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';"
            "const child=document.createElement('iframe');"
            "child.sandbox.add('allow-scripts');child.src='http://127.0.0.1:%u/child';"
            "document.documentElement.append(child);"
            "child.addEventListener('load',()=>child.contentWindow.postMessage("
            "{kind:'credential',generation:'7',token},'*'),{once:true});"
            "addEventListener('message',e=>{if(e.source!==child.contentWindow||"
            "e.origin!=='null'||e.data?.kind!=='childChecked')return;"
            "const payload={kind:'testReport',childRaw:e.data.raw,"
            "childData:e.data.data,childBootstrap:e.data.bootstrap,"
            "topRaw:typeof window.webkit?.messageHandlers?.webuiDesktopIpc};"
            "postMessage({marker,direction:'page-to-native',lane:'control',id:1,payload},location.origin);"
            "postMessage({marker,direction:'page-to-native',lane:'control',id:2,"
            "payload:{kind:'disconnect',generation:'7',token}},location.origin);"
            "postMessage({marker,direction:'page-to-native',lane:'data',id:3,"
            "payload:{kind:'ipcData',version:1,callId:'1',generation:'7',token,"
            "operation:'receive',offset:0,maxBytes:64}},location.origin)})"
            "</script>", child_port);
    }
    gchar *response = g_strdup_printf(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n"
        "Content-Length: %zu\r\nConnection: close\r\n\r\n%s",
        strlen(page), page);
    gsize written;
    g_output_stream_write_all(output, response, strlen(response), &written, NULL, NULL);
    g_io_stream_close(G_IO_STREAM(connection), NULL, NULL);
    g_free(response);
    g_free(page);
    return TRUE;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: linux-local-isolation MEDIATOR_ASSET\n");
        return 2;
    }
    gtk_init();
    GError *error = NULL;
    gchar *mediator = NULL;
    if (!g_file_get_contents(argv[1], &mediator, NULL, &error)) {
        fprintf(stderr, "mediator asset unavailable: %s\n", error->message);
        return 2;
    }
    GThreadedSocketService *server =
        G_THREADED_SOCKET_SERVICE(g_threaded_socket_service_new(4));
    g_signal_connect(server, "run", G_CALLBACK(serve), NULL);
    top_port = g_socket_listener_add_any_inet_port(G_SOCKET_LISTENER(server), NULL, &error);
    if (!top_port) return 2;
    // Separate port is a distinct cross-origin principal. Both listeners
    // serve the same fixture bytes; only /child selects child content.
    GThreadedSocketService *other =
        G_THREADED_SOCKET_SERVICE(g_threaded_socket_service_new(4));
    g_signal_connect(other, "run", G_CALLBACK(serve), NULL);
    child_port = g_socket_listener_add_any_inet_port(G_SOCKET_LISTENER(other), NULL, &error);
    if (!child_port) return 2;
    g_socket_service_start(G_SOCKET_SERVICE(server));
    g_socket_service_start(G_SOCKET_SERVICE(other));
    WebKitUserContentManager *manager = webkit_user_content_manager_new();
    g_signal_connect(manager, "script-message-with-reply-received::webuiDesktopIpc",
                     G_CALLBACK(on_message), GINT_TO_POINTER(1));
    g_signal_connect(manager, "script-message-with-reply-received::webuiDesktopIpcData",
                     G_CALLBACK(on_message), GINT_TO_POINTER(2));
    if (!webkit_user_content_manager_register_script_message_handler_with_reply(
            manager, "webuiDesktopIpc", WORLD) ||
        !webkit_user_content_manager_register_script_message_handler_with_reply(
            manager, "webuiDesktopIpcData", WORLD)) return 2;
    WebKitUserScript *script = webkit_user_script_new_for_world(
        mediator, WEBKIT_USER_CONTENT_INJECT_TOP_FRAME,
        WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START, WORLD, NULL, NULL);
    webkit_user_content_manager_add_script(manager, script);
    webkit_user_script_unref(script);
    g_free(mediator);
    WebKitWebView *view = WEBKIT_WEB_VIEW(g_object_new(
        WEBKIT_TYPE_WEB_VIEW, "user-content-manager", manager, NULL));
    GtkWidget *window = gtk_window_new();
    gtk_window_set_child(GTK_WINDOW(window), GTK_WIDGET(view));
    gtk_window_present(GTK_WINDOW(window));
    loop = g_main_loop_new(NULL, FALSE);
    gchar *url = g_strdup_printf("http://127.0.0.1:%u/", top_port);
    webkit_web_view_load_uri(view, url);
    g_free(url);
    g_timeout_add_seconds(6, finish, NULL);
    g_main_loop_run(loop);
    return controls == 1 && data == 1 && reports == 1 && child_clean ? 0 : 3;
}
