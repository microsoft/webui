// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#ifndef WEBUI_STARTER_HOST_H
#define WEBUI_STARTER_HOST_H

#include "civetweb.h"
#include "webui_ffi.h"
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <limits.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <dirent.h>
#include <fcntl.h>
#include <poll.h>
#include <sys/stat.h>
#include <unistd.h>
#endif
#include "shutdown.h"

#define HOST_MAX_ASSETS 4096
#define HOST_MAX_FILE (16u * 1024u * 1024u)
#define HOST_MAX_TOTAL (64u * 1024u * 1024u)

struct host_asset {
    char *url;
    const char *mime;
    unsigned char *bytes;
    size_t length;
};

struct host {
    void *handler;
    webui_protocol_t *protocol;
    struct host_asset *assets;
    size_t count;
    size_t total;
    int (*page)(struct mg_connection *, struct host *, const char *, int);
};

static volatile sig_atomic_t host_stopping = 0;
static void host_signal(int number) { (void)number; host_stopping = 1; }

static int host_control_tick(void) {
    unsigned char command = 0;
#ifdef _WIN32
    HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
    DWORD available = 0, count = 0;
    if (!PeekNamedPipe(input, NULL, 0, NULL, &available, NULL)) {
        if (GetLastError() == ERROR_BROKEN_PIPE) {
            fprintf(stderr, "WebUI supervisor disconnected; shutting down\n"); return 1;
        }
        fprintf(stderr, "Cannot read WebUI supervisor pipe: %lu\n", (unsigned long)GetLastError()); return -1;
    }
    if (!available) return 0;
    if (!ReadFile(input, &command, 1, &count, NULL) || count != 1) {
        fprintf(stderr, "Cannot read WebUI supervisor command\n"); return -1;
    }
#else
    struct pollfd input = {STDIN_FILENO, POLLIN, 0};
    int ready = poll(&input, 1, 0);
    if (ready < 0 && errno == EINTR) return 0;
    if (ready < 0) { fprintf(stderr, "Cannot poll WebUI supervisor pipe: %s\n", strerror(errno)); return -1; }
    if (!ready) return 0;
    ssize_t count = read(STDIN_FILENO, &command, 1);
    if (count == 0) { fprintf(stderr, "WebUI supervisor disconnected; shutting down\n"); return 1; }
    if (count < 0 && errno == EINTR) return 0;
    if (count != 1) { fprintf(stderr, "Cannot read WebUI supervisor command\n"); return -1; }
#endif
    if (command != 'S') { fprintf(stderr, "Invalid WebUI supervisor command; shutting down\n"); return -1; }
    return 1;
}

static const char *host_mime(const char *name) {
    const char *extension = strrchr(name, '.');
    static const char *const types[][2] = {
        {".js", "text/javascript; charset=utf-8"}, {".css", "text/css; charset=utf-8"},
        {".json", "application/json"}, {".map", "application/json"}, {".txt", "text/plain; charset=utf-8"},
        {".svg", "image/svg+xml"}, {".png", "image/png"}, {".jpg", "image/jpeg"},
        {".jpeg", "image/jpeg"}, {".gif", "image/gif"}, {".webp", "image/webp"},
        {".avif", "image/avif"}, {".ico", "image/x-icon"},
        {".woff", "font/woff"}, {".woff2", "font/woff2"}
    };
    if (!extension) return NULL;
    for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); ++i)
        if (strcmp(extension, types[i][0]) == 0) return types[i][1];
    return NULL;
}

static int host_reply(struct mg_connection *connection, int status, const char *mime,
                      const void *bytes, size_t length, int head) {
    char content_length[32];
    snprintf(content_length, sizeof(content_length), "%zu", length);
    if (mg_response_header_start(connection, status) != 0 ||
        mg_response_header_add(connection, "Content-Type", mime, -1) != 0 ||
        mg_response_header_add(connection, "Content-Length", content_length, -1) != 0 ||
        mg_response_header_add(connection, "X-Content-Type-Options", "nosniff", -1) != 0 ||
        (status == 405 && mg_response_header_add(connection, "Allow", "GET, HEAD", -1) != 0) ||
        mg_response_header_send(connection) != 0) {
        mg_disable_connection_keep_alive(connection);
        return status;
    }
    if (!head && length) {
        size_t written = 0;
        while (written < length) {
            int amount = mg_write(connection, (const unsigned char *)bytes + written, length - written);
            if (amount <= 0) {
                mg_disable_connection_keep_alive(connection);
                break;
            }
            written += (size_t)amount;
        }
    }
    return status;
}

static int host_error(struct mg_connection *connection, int head) {
    const char message[] = "Internal server error\n";
    return host_reply(connection, 500, "text/plain; charset=utf-8", message, sizeof(message) - 1, head);
}

static void host_log_native(const char *operation) {
    const char *error = webui_last_error();
    fprintf(stderr, "%s: %s\n", operation, error ? error : "native operation failed without a diagnostic");
}

static int host_dispatch(struct mg_connection *connection) {
    const struct mg_request_info *request = mg_get_request_info(connection);
    struct host *app = (struct host *)request->user_data;
    /* No endpoint consumes bodies. Close instead of draining unbounded or
       incomplete input, or treating leftover body bytes as another request. */
    if (request->content_length > 0 || mg_get_header(connection, "Transfer-Encoding") ||
        mg_get_header(connection, "Expect"))
        mg_disable_connection_keep_alive(connection);
    int head = strcmp(request->request_method, "HEAD") == 0;
    if (!head && strcmp(request->request_method, "GET") != 0) {
        const char message[] = "Method not allowed\n";
        return host_reply(connection, 405, "text/plain; charset=utf-8", message, sizeof(message) - 1, 0);
    }
    /* Compare the decoded, uncleaned path. Never normalize traversal into a hit. */
    const char *path = request->local_uri_raw;
    if (path && (strcmp(path, "/") == 0 || strcmp(path, "/about") == 0))
        return app->page(connection, app, path, head);
    if (path)
        for (size_t i = 0; i < app->count; ++i)
            if (strcmp(path, app->assets[i].url) == 0)
                return host_reply(connection, 200, app->assets[i].mime,
                                  app->assets[i].bytes, app->assets[i].length, head);
    const char message[] = "Not found\n";
    return host_reply(connection, 404, "text/plain; charset=utf-8", message, sizeof(message) - 1, head);
}

static int host_request(struct mg_connection *connection) {
    host_lock_requests();
    if (host_draining) {
        host_unlock_requests();
        mg_disable_connection_keep_alive(connection);
        const char message[] = "Server shutting down\n";
        const struct mg_request_info *request = mg_get_request_info(connection);
        return host_reply(connection, 503, "text/plain; charset=utf-8", message,
                          sizeof(message) - 1, strcmp(request->request_method, "HEAD") == 0);
    }
    ++host_active_requests;
    host_unlock_requests();
    int status = host_dispatch(connection);
    host_lock_requests();
    if (--host_active_requests == 0) host_signal_idle();
    host_unlock_requests();
    return status;
}

static unsigned char *host_read(FILE *file, size_t *length) {
    if (fseek(file, 0, SEEK_END) != 0) return NULL;
    long size = ftell(file);
    if (size < 0 || (unsigned long)size > HOST_MAX_FILE || fseek(file, 0, SEEK_SET) != 0) return NULL;
    *length = (size_t)size;
    unsigned char *bytes = (unsigned char *)malloc(*length ? *length : 1);
    if (!bytes) return NULL;
    if (fread(bytes, 1, *length, file) != *length || fgetc(file) != EOF) {
        free(bytes);
        return NULL;
    }
    return bytes;
}

static int host_add_asset(struct host *app, const char *relative, FILE *file) {
    const char *mime = host_mime(relative);
    if (!mime) return 1;
    if (app->count == HOST_MAX_ASSETS) return 0;
    size_t length = 0;
    unsigned char *bytes = host_read(file, &length);
    if (!bytes) return 0;
    if (app->total + length > HOST_MAX_TOTAL) { free(bytes); return 0; }
    char *url = (char *)malloc(strlen(relative) + sizeof("/assets/"));
    if (!url) { free(bytes); return 0; }
    snprintf(url, strlen(relative) + sizeof("/assets/"), "/assets/%s", relative);
    struct host_asset *asset = &app->assets[app->count++];
    asset->url = url; asset->mime = mime; asset->bytes = bytes; asset->length = length;
    app->total += length;
    return 1;
}

#ifndef _WIN32
struct host_directory { DIR *directory; char relative[4096]; };

static int host_load_assets(struct host *app) {
    /* Descriptor-relative opens reject symlinks, including ancestor swaps. */
    int dist = open("dist", O_RDONLY | O_DIRECTORY | O_NOFOLLOW);
    if (dist < 0) return 0;
    int public_fd = openat(dist, "public", O_RDONLY | O_DIRECTORY | O_NOFOLLOW);
    close(dist);
    if (public_fd < 0) return 0;
    DIR *root = fdopendir(public_fd);
    if (!root) { close(public_fd); return 0; }
    struct host_directory *stack = (struct host_directory *)calloc(HOST_MAX_ASSETS, sizeof(*stack));
    if (!stack) { closedir(root); return 0; }
    stack[0].directory = root;
    size_t depth = 1;
    int success = 1;
    while (depth && success) {
        struct host_directory *current = &stack[depth - 1];
        errno = 0;
        struct dirent *entry = readdir(current->directory);
        if (!entry) {
            if (errno) success = 0;
            closedir(current->directory);
            --depth;
            continue;
        }
        if (entry->d_name[0] == '.') continue;
        char relative[4096];
        int size = snprintf(relative, sizeof(relative), "%s%s%s", current->relative,
                            current->relative[0] ? "/" : "", entry->d_name);
        if (size < 0 || (size_t)size >= sizeof(relative)) { success = 0; break; }
        struct stat info;
        int parent = dirfd(current->directory);
        if (fstatat(parent, entry->d_name, &info, AT_SYMLINK_NOFOLLOW) != 0) { success = 0; break; }
        if (S_ISDIR(info.st_mode)) {
            if (depth == HOST_MAX_ASSETS) { success = 0; break; }
            int fd = openat(parent, entry->d_name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW);
            if (fd < 0) { success = 0; break; }
            DIR *directory = fdopendir(fd);
            if (!directory) { close(fd); success = 0; break; }
            stack[depth].directory = directory;
            strcpy(stack[depth++].relative, relative);
        } else if (S_ISREG(info.st_mode) && host_mime(relative)) {
            int fd = openat(parent, entry->d_name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK);
            if (fd < 0) { success = 0; break; }
            if (fstat(fd, &info) != 0 || !S_ISREG(info.st_mode)) { close(fd); success = 0; break; }
            FILE *file = fdopen(fd, "rb");
            if (!file) { close(fd); success = 0; break; }
            success = host_add_asset(app, relative, file);
            fclose(file);
        }
    }
    while (depth) closedir(stack[--depth].directory);
    free(stack);
    return success;
}
#else
/* Snapshotting avoids any request-time Win32 path interpretation or link access. */
static int host_load_assets(struct host *app) {
    char (*stack)[4096] = (char (*)[4096])calloc(HOST_MAX_ASSETS, 4096);
    if (!stack) return 0;
    DWORD dist = GetFileAttributesA("dist"), public_dir = GetFileAttributesA("dist/public");
    if (dist == INVALID_FILE_ATTRIBUTES || public_dir == INVALID_FILE_ATTRIBUTES ||
        (dist & FILE_ATTRIBUTE_REPARSE_POINT) || (public_dir & FILE_ATTRIBUTE_REPARSE_POINT)) {
        free(stack); return 0;
    }
    size_t count = 1;
    int success = 1;
    while (count && success) {
        char relative[4096], search[8192];
        strcpy(relative, stack[--count]);
        snprintf(search, sizeof(search), "dist/public/%s%s*", relative, relative[0] ? "/" : "");
        WIN32_FIND_DATAA data;
        HANDLE scan = FindFirstFileA(search, &data);
        if (scan == INVALID_HANDLE_VALUE) { success = GetLastError() == ERROR_FILE_NOT_FOUND; continue; }
        do {
            if (data.cFileName[0] == '.' || (data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT)) continue;
            char child[4096], path[8192];
            int size = snprintf(child, sizeof(child), "%s%s%s", relative, relative[0] ? "/" : "", data.cFileName);
            if (size < 0 || (size_t)size >= sizeof(child)) { success = 0; break; }
            if (data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
                if (count == HOST_MAX_ASSETS) { success = 0; break; }
                strcpy(stack[count++], child);
            } else if (host_mime(child)) {
                snprintf(path, sizeof(path), "dist/public/%s", child);
                FILE *file = fopen(path, "rb");
                if (!file) { success = 0; break; }
                success = host_add_asset(app, child, file);
                fclose(file);
            }
        } while (success && FindNextFileA(scan, &data));
        if (success && GetLastError() != ERROR_NO_MORE_FILES) success = 0;
        FindClose(scan);
    }
    free(stack);
    return success;
}
#endif

static void host_free_assets(struct host *app) {
    for (size_t i = 0; i < app->count; ++i) {
        free(app->assets[i].url);
        free(app->assets[i].bytes);
    }
    free(app->assets);
    app->assets = NULL; app->count = 0; app->total = 0;
}

static webui_protocol_t *host_load_protocol(void) {
    FILE *file = fopen("dist/protocol.bin", "rb");
    if (!file) { fprintf(stderr, "Cannot read dist/protocol.bin; run the app build first.\n"); return NULL; }
    size_t length = 0;
    unsigned char *bytes = host_read(file, &length);
    fclose(file);
    if (!bytes) { fprintf(stderr, "Cannot read protocol (maximum 16 MiB).\n"); return NULL; }
    webui_protocol_t *protocol = webui_protocol_create(bytes, (uintptr_t)length);
    if (!protocol) host_log_native("Load protocol");
    free(bytes);
    return protocol;
}

static int host_serve(struct host *app) {
    app->assets = (struct host_asset *)calloc(HOST_MAX_ASSETS, sizeof(*app->assets));
    if (!app->assets || !host_load_assets(app)) {
        fprintf(stderr, "Cannot snapshot dist/public; build first. No symlink roots; limits: 16 MiB/file, 64 MiB total, 4096 assets.\n");
        return 1;
    }
    const char *host = getenv("HOST"), *port = getenv("PORT");
    if (!host || !*host) host = "127.0.0.1";
    if (!port || !*port) port = "3000";
    /* Only IP literals/localhost, never CivetWeb multi-listener option syntax. */
    for (const char *p = host; *p; ++p)
        if (!((*p >= '0' && *p <= '9') || *p == '.') && strcmp(host, "localhost") != 0) {
            fprintf(stderr, "HOST must be an IPv4 address or localhost.\n"); return 1;
        }
    char *end = NULL;
    long number = strtol(port, &end, 10);
    if (!*port || !end || *end || number < 1 || number > 65535) {
        fprintf(stderr, "PORT must be 1-65535.\n"); return 1;
    }
    char address[128];
    int size = snprintf(address, sizeof(address), "%s:%ld", host, number);
    if (size < 0 || (size_t)size >= sizeof(address)) return 1;
    /* CivetWeb dedicates a worker to each live connection. Grow on demand,
       cap admission, and evict idle/partial clients instead of pinning workers. */
    const char *options[] = {"listening_ports", address, "num_threads", "16",
        "connection_queue", "16", "listen_backlog", "32", "enable_keep_alive", "yes",
        "keep_alive_timeout_ms", "100", "request_timeout_ms", "2000",
        "tcp_nodelay", "1", "max_request_size", "16384", NULL};
    struct mg_callbacks callbacks;
    memset(&callbacks, 0, sizeof(callbacks));
    callbacks.begin_request = host_request;
    if (!host_init_requests()) { fprintf(stderr, "Cannot initialize HTTP shutdown coordination\n"); return 1; }
    mg_init_library(0);
    struct mg_context *context = mg_start(&callbacks, app, options);
    if (!context) { fprintf(stderr, "Cannot listen on %s; check HOST/PORT and port availability.\n", address); mg_exit_library(); host_free_requests(); return 1; }
    signal(SIGINT, host_signal);
    signal(SIGTERM, host_signal);
#ifndef _WIN32
    signal(SIGPIPE, SIG_IGN);
#endif
    fprintf(stderr, "WebUI listening on http://%s\n", address);
    const char *control = getenv("_WEBUI_APP_CONTROL");
    int controlled = control && strcmp(control, "stdin-v1") == 0;
    int control_failed = 0;
    while (!host_stopping) {
        if (controlled) {
            int command = host_control_tick();
            if (command) { control_failed = command < 0; break; }
        }
#ifdef _WIN32
        Sleep(100);
#else
        struct timespec delay = {0, 100000000};
        nanosleep(&delay, NULL);
#endif
    }
    int drained = host_drain_requests();
    mg_stop(context); /* Joins workers before releasing any native handles. */
    host_free_requests();
    mg_exit_library();
    return control_failed || !drained ? 1 : 0;
}
#endif
