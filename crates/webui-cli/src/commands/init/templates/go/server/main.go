// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"

	webui "__WEBUI_GO_MODULE__"
)

type asset struct {
	mime  string
	bytes []byte
}

var mimes = map[string]string{
	".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8",
	".json": "application/json", ".map": "application/json", ".txt": "text/plain; charset=utf-8",
	".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg",
	".jpeg": "image/jpeg", ".gif": "image/gif", ".webp": "image/webp",
	".avif": "image/avif", ".ico": "image/x-icon", ".woff": "font/woff", ".woff2": "font/woff2",
}

func snapshotAssets(directory string) (map[string]asset, error) {
	absolute, err := filepath.Abs(directory)
	if err != nil {
		return nil, err
	}
	// Do not permit a symlink ancestor, including dist/public itself.
	for path := absolute; ; path = filepath.Dir(path) {
		info, err := os.Lstat(path)
		if err != nil {
			return nil, err
		}
		if info.Mode()&os.ModeSymlink != 0 {
			return nil, fmt.Errorf("public asset directory must not contain symlink ancestors: %s", path)
		}
		if filepath.Dir(path) == path {
			break
		}
	}
	root, err := os.OpenRoot(directory)
	if err != nil {
		return nil, err
	}
	defer root.Close()
	assets := make(map[string]asset)
	var total int64
	err = filepath.WalkDir(directory, func(path string, entry fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if path == directory {
			return nil
		}
		if strings.HasPrefix(entry.Name(), ".") || entry.Type()&os.ModeSymlink != 0 {
			if entry.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		if entry.IsDir() {
			return nil
		}
		mime, ok := mimes[strings.ToLower(filepath.Ext(path))]
		if !ok || !entry.Type().IsRegular() {
			return nil
		}
		relative, err := filepath.Rel(directory, path)
		if err != nil {
			return err
		}
		// Root prevents an ancestor swap from escaping the public directory.
		file, err := root.Open(relative)
		if err != nil {
			return err
		}
		defer file.Close()
		info, err := file.Stat()
		if err != nil {
			return err
		}
		if !info.Mode().IsRegular() {
			return nil
		}
		bytes, err := io.ReadAll(io.LimitReader(file, 16*1024*1024+1))
		if err != nil {
			return err
		}
		total += int64(len(bytes))
		if len(bytes) > 16*1024*1024 || total > 64*1024*1024 || len(assets) >= 4096 {
			return errors.New("public assets exceed starter limits (16 MiB/file, 64 MiB total, 4096 files)")
		}
		assets["/assets/"+filepath.ToSlash(relative)] = asset{mime, bytes}
		return nil
	})
	return assets, err
}

func appHandler(r *webui.Renderer, assets map[string]asset) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, request *http.Request) {
		w.Header().Set("X-Content-Type-Options", "nosniff")
		reply := func(status int, mime string, bytes []byte) {
			w.Header().Set("Content-Type", mime)
			w.Header().Set("Content-Length", strconv.Itoa(len(bytes)))
			w.WriteHeader(status)
			if request.Method != http.MethodHead {
				if _, err := w.Write(bytes); err != nil {
					log.Printf("response write: %v", err)
				}
			}
		}
		if request.Method != http.MethodGet && request.Method != http.MethodHead {
			w.Header().Set("Allow", "GET, HEAD")
			reply(405, "text/plain; charset=utf-8", []byte("Method not allowed\n"))
			return
		}
		path := request.URL.Path
		if path == "/" || path == "/about" {
			state := `{"title":"About WebUI","isHome":false}`
			if path == "/" {
				state = `{"title":"Hello WebUI","count":0,"ready":false,"isHome":true}`
			}
			bytes, err := r.Render([]byte(state), webui.RenderOptions{RequestPath: request.URL.EscapedPath()})
			if err != nil {
				log.Printf("WebUI render %s: %v", path, err)
				reply(500, "text/plain; charset=utf-8", []byte("Internal server error\n"))
				return
			}
			reply(200, "text/html; charset=utf-8", bytes)
		} else if asset, ok := assets[path]; ok {
			reply(200, asset.mime, asset.bytes)
		} else {
			reply(404, "text/plain; charset=utf-8", []byte("Not found\n"))
		}
	})
}

func run(ctx context.Context) error {
	if webui.Version != "__WEBUI_VERSION__" {
		return fmt.Errorf("WebUI binding version %s does not match compiler/SDK __WEBUI_VERSION__; align the module and native tooling", webui.Version)
	}
	// The launcher supplies an explicit root; direct commands use the
	// working directory documented for this project layout.
	root := os.Getenv("WEBUI_APP_ROOT")
	if root == "" {
		root = "__APP_ROOT_DEFAULT__"
	}
	assets, err := snapshotAssets(filepath.Join(root, "dist", "public"))
	if err != nil {
		return fmt.Errorf("load public assets (run the app build first): %w", err)
	}
	bytes, err := os.ReadFile(filepath.Join(root, "dist", "protocol.bin"))
	if err != nil {
		return fmt.Errorf("load protocol (run the app build first): %w", err)
	}
	renderer, err := webui.NewRenderer(bytes)
	if err != nil {
		return fmt.Errorf("load WebUI protocol: %w", err)
	}
	defer renderer.Close()
	host, port := os.Getenv("HOST"), os.Getenv("PORT")
	if host == "" {
		host = "127.0.0.1"
	}
	if port == "" {
		port = "3000"
	}
	number, err := strconv.Atoi(port)
	if err != nil || number < 1 || number > 65535 || (net.ParseIP(host) == nil && host != "localhost") {
		return errors.New("set HOST to an IP address (or localhost) and PORT to 1-65535")
	}
	server := &http.Server{
		Addr: net.JoinHostPort(host, port), Handler: appHandler(renderer, assets),
		ReadHeaderTimeout: 10 * time.Second, ReadTimeout: 10 * time.Second,
		WriteTimeout: 15 * time.Second, IdleTimeout: 30 * time.Second,
		MaxHeaderBytes: 16 * 1024,
	}
	listener, err := net.Listen("tcp", server.Addr)
	if err != nil {
		return err
	}
	return serve(ctx, server, listener)
}

func serve(ctx context.Context, server *http.Server, listener net.Listener) error {
	result := make(chan error, 1)
	go func() { result <- server.Serve(listener) }()
	log.Printf("WebUI listening on http://%s", server.Addr)
	select {
	case err := <-result:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
	case <-ctx.Done():
		drain, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		if err := server.Shutdown(drain); err != nil {
			_ = server.Close()
			return fmt.Errorf("HTTP shutdown: %w", err)
		}
	}
	return nil
}

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	if os.Getenv("_WEBUI_APP_CONTROL") == "stdin-v1" {
		controlled, cancel := context.WithCancel(ctx)
		defer cancel()
		ctx = controlled
		go func() {
			var command [1]byte
			_, err := io.ReadFull(os.Stdin, command[:])
			if err != nil {
				log.Printf("WebUI supervisor disconnected: %v; shutting down", err)
			} else if command[0] != 'S' {
				log.Printf("Invalid WebUI supervisor command; shutting down")
			}
			cancel()
		}()
	}
	err := run(ctx)
	stop()
	if err != nil {
		log.Printf("Unable to start WebUI: %v", err)
		os.Exit(1)
	}
}
