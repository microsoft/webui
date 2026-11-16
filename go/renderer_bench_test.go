// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package webui_test

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"html"
	"strings"
	"testing"

	webui "github.com/microsoft/webui/go/v3"
)

func BenchmarkRender(b *testing.B) {
	for _, workload := range []struct {
		name    string
		message string
	}{
		{"small", "Go & WebUI"},
		{"64KiB", strings.Repeat("x", 64<<10)},
	} {
		b.Run(workload.name, func(b *testing.B) {
			r, err := webui.NewRenderer(testProtocol)
			if err != nil {
				b.Fatal(err)
			}
			b.Cleanup(func() {
				if err := r.Close(); err != nil {
					b.Error(err)
				}
			})
			state, err := json.Marshal(struct {
				Title   string `json:"title"`
				Message string `json:"message"`
			}{"Go benchmark", workload.message})
			if err != nil {
				b.Fatal(err)
			}
			stateHash := sha256.Sum256(state)
			output, err := r.Render(state, webui.RenderOptions{})
			if err != nil {
				b.Fatal(err)
			}
			for _, expected := range []string{
				"<title>Go benchmark</title>",
				"<h1>" + html.EscapeString(workload.message) + "</h1>",
				"<p>home-route</p>",
			} {
				if !bytes.Contains(output, []byte(expected)) {
					b.Fatalf("incorrect benchmark output: missing content of length %d", len(expected))
				}
			}
			outputHash := sha256.Sum256(output)
			b.Logf("state=%d bytes, output=%d bytes, output SHA-256=%x", len(state), len(output), outputHash)
			b.ReportAllocs()
			b.SetBytes(int64(len(state)))

			// b.Loop excludes setup and the correctness/hash checks below.
			for b.Loop() {
				output, err = r.Render(state, webui.RenderOptions{})
				if err != nil {
					b.Fatal(err)
				}
			}

			if sha256.Sum256(output) != outputHash {
				b.Fatal("render output changed during benchmark")
			}
			if sha256.Sum256(state) != stateHash {
				b.Fatal("Render mutated caller-owned state")
			}
			b.ReportMetric(float64(len(output)), "output-B")
		})
	}
}
