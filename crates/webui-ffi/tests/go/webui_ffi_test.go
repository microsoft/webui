// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package webui_ffi

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"unsafe"
)

func fixtureProtocol(t *testing.T) []byte {
	t.Helper()
	_, filename, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not locate test source")
	}
	bytes, err := os.ReadFile(filepath.Join(filepath.Dir(filename), "..", "fixtures", "protocol.bin"))
	if err != nil {
		t.Fatalf("read protocol fixture: %v", err)
	}
	return bytes
}

func loadedRuntime(t *testing.T) (handler, protocol unsafe.Pointer) {
	t.Helper()
	runtime.LockOSThread()
	t.Cleanup(runtime.UnlockOSThread)
	handler = HandlerCreate()
	if handler == nil {
		t.Fatal("handler should not be nil")
	}
	protocol = ProtocolCreate(fixtureProtocol(t))
	if protocol == nil {
		HandlerDestroy(handler)
		t.Fatalf("protocol load failed: %s", LastError())
	}
	t.Cleanup(func() {
		ProtocolDestroy(protocol)
		HandlerDestroy(handler)
	})
	return handler, protocol
}

func TestSignalSubstitution(t *testing.T) {
	handler, protocol := loadedRuntime(t)
	result, err := Render(handler, protocol, `{"name":"WebUI"}`)
	if err != nil {
		t.Fatal(err)
	}
	if result != "<p>Hello, WebUI!</p>" {
		t.Fatalf("got %q", result)
	}
}

func TestReusesLoadedProtocol(t *testing.T) {
	handler, protocol := loadedRuntime(t)
	first, firstErr := Render(handler, protocol, `{"name":"First"}`)
	second, secondErr := Render(handler, protocol, `{"name":"Second"}`)
	if firstErr != nil || secondErr != nil {
		t.Fatalf("render failed: %v / %v", firstErr, secondErr)
	}
	if !strings.Contains(first, "First") || !strings.Contains(second, "Second") {
		t.Fatalf("unexpected outputs: %q / %q", first, second)
	}
}

func TestInvalidJSONSetsError(t *testing.T) {
	handler, protocol := loadedRuntime(t)
	pointer := RenderRaw(handler, protocol, "NOT JSON")
	if pointer != nil {
		ResultDestroy(pointer)
		t.Fatal("expected invalid JSON to fail")
	}
	if !strings.Contains(LastError(), "JSON") {
		t.Fatalf("unexpected error: %s", LastError())
	}
}

func TestNullProtocolSetsError(t *testing.T) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	handler := HandlerCreate()
	t.Cleanup(func() { HandlerDestroy(handler) })
	pointer := RenderRaw(handler, nil, "{}")
	if pointer != nil {
		ResultDestroy(pointer)
		t.Fatal("expected null protocol to fail")
	}
	if LastError() == "" {
		t.Fatal("expected error message")
	}
}

func TestNullDestroyAndFreeAreSafe(t *testing.T) {
	HandlerDestroy(nil)
	ProtocolDestroy(nil)
	ResultDestroy(nil)
	Free(nil)
}

func TestPreservesNULAndUTF8AsOutputSizeChanges(t *testing.T) {
	handler, protocol := loadedRuntime(t)
	for _, name := range []string{strings.Repeat("é", 65536), "before\x00after", "", strings.Repeat("é", 65536)} {
		state, err := json.Marshal(map[string]string{"name": name})
		if err != nil {
			t.Fatal(err)
		}
		result, err := Render(handler, protocol, string(state))
		if err != nil {
			t.Fatal(err)
		}
		if result != "<p>Hello, "+name+"!</p>" {
			t.Fatalf("output mismatch for name length %d", len(name))
		}
	}
}
