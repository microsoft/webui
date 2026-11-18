// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

package main

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCompressedArchiveDigestAndGzipIntegrity(t *testing.T) {
	spec := testSpec(t)
	body := []byte("compressed validation fixture")
	var buffer bytes.Buffer
	writer := gzip.NewWriter(&buffer)
	if _, err := writer.Write(testTar(t, body, testMetadata(t, spec, body), nil)); err != nil {
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	spec.Archive = filepath.Join(spec.Output, "fixture.tar.gz")
	digest := sha256.Sum256(buffer.Bytes())
	spec.SHA256 = hex.EncodeToString(digest[:])
	if err := os.WriteFile(spec.Archive, buffer.Bytes(), 0600); err != nil {
		t.Fatal(err)
	}
	if err := validate(spec); err != nil {
		t.Fatal(err)
	}
	spec.SHA256 = strings.Repeat("0", 64)
	if validate(spec) == nil {
		t.Fatal("compressed digest mismatch accepted")
	}
	corrupt := buffer.Bytes()
	corrupt[len(corrupt)-1] ^= 0xff
	digest = sha256.Sum256(corrupt)
	spec.SHA256 = hex.EncodeToString(digest[:])
	if err := os.WriteFile(spec.Archive, corrupt, 0600); err != nil {
		t.Fatal(err)
	}
	if validate(spec) == nil {
		t.Fatal("corrupt gzip trailer accepted")
	}
}

func testSpec(t *testing.T) specification {
	return specification{
		Output: t.TempDir(), Version: "3.1.2", Target: "foreign-fixture",
		TypeScriptVersion: "7.0.2",
		Files:             map[string]int64{"compiler/lib.d.ts": 1024},
	}
}

func testMetadata(t *testing.T, spec specification, content []byte) []byte {
	t.Helper()
	hash := sha256.Sum256(content)
	data, err := json.Marshal(map[string]any{
		"schemaVersion": 1, "version": spec.Version, "target": spec.Target,
		"typescriptVersion": spec.TypeScriptVersion,
		"files":             map[string]string{"compiler/lib.d.ts": hex.EncodeToString(hash[:])},
	})
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func testTar(t *testing.T, body, metadata []byte, extra *tar.Header) []byte {
	t.Helper()
	var buffer bytes.Buffer
	writer := tar.NewWriter(&buffer)
	for _, entry := range []struct {
		name string
		data []byte
	}{{"compiler/lib.d.ts", body}, {"toolchain.json", metadata}} {
		if err := writer.WriteHeader(&tar.Header{Name: entry.name, Mode: 0600, Size: int64(len(entry.data))}); err != nil {
			t.Fatal(err)
		}
		if _, err := writer.Write(entry.data); err != nil {
			t.Fatal(err)
		}
	}
	if extra != nil {
		if err := writer.WriteHeader(extra); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return buffer.Bytes()
}

func TestArchiveMembershipIdentityHashesAndBounds(t *testing.T) {
	spec := testSpec(t)
	body := []byte("synthetic hash fixture, not a native artifact")
	metadata := testMetadata(t, spec, body)
	if err := validateTar(spec, bytes.NewReader(testTar(t, body, metadata, nil))); err != nil {
		t.Fatal(err)
	}
	cases := map[string][]byte{
		"corrupt": bytes.Replace(metadata, []byte(`"files"`), []byte(`"unknown"`), 1),
		"version": bytes.Replace(metadata, []byte(`"3.1.2"`), []byte(`"0.0.0"`), 1),
		"target":  bytes.Replace(metadata, []byte(`"foreign-fixture"`), []byte(`"wrong"`), 1),
		"checker": bytes.Replace(metadata, []byte(`"7.0.2"`), []byte(`"3.1.2"`), 1),
		"duplicate": []byte(strings.Replace(string(metadata),
			`"schemaVersion":1`, `"schemaVersion":1,"schemaVersion":1`, 1)),
	}
	for name, data := range cases {
		t.Run(name, func(t *testing.T) {
			if validateTar(spec, bytes.NewReader(testTar(t, body, data, nil))) == nil {
				t.Fatal("invalid metadata accepted")
			}
		})
	}
	if validateTar(spec, bytes.NewReader(testTar(t, []byte("corruption"), metadata, nil))) == nil {
		t.Fatal("payload corruption accepted")
	}
	for _, header := range []*tar.Header{
		{Name: "../escape", Mode: 0600},
		{Name: "compiler/unknown", Mode: 0600},
		{Name: "compiler/lib.d.ts", Mode: 0600},
		{Name: "compiler/link", Typeflag: tar.TypeSymlink, Linkname: "/outside"},
		{Name: "compiler/link", Typeflag: tar.TypeLink, Linkname: "/outside"},
	} {
		if validateTar(spec, bytes.NewReader(testTar(t, body, metadata, header))) == nil {
			t.Fatalf("unsafe member accepted: %s", header.Name)
		}
	}
	spec.Files["compiler/lib.d.ts"] = 1
	if validateTar(spec, bytes.NewReader(testTar(t, body, metadata, nil))) == nil {
		t.Fatal("oversized member accepted")
	}
}

func TestMetadataAndTarTrailingDataAreRejected(t *testing.T) {
	if _, err := object([]byte(`{"files":{"x":"a","x":"b"}}`)); err != nil {
		t.Fatal("outer object should leave nested validation to its files owner")
	}
	if _, err := object([]byte(`{"x":"a","x":"b"}`)); err == nil {
		t.Fatal("duplicate file hash key accepted")
	}
	if _, err := object([]byte(`{"x":1} {"x":2}`)); err == nil {
		t.Fatal("trailing metadata accepted")
	}
	spec := testSpec(t)
	body := []byte("fixture")
	data := append(testTar(t, body, testMetadata(t, spec, body), nil), []byte("hidden payload")...)
	if validateTar(spec, bytes.NewReader(data)) == nil {
		t.Fatal("nonzero trailing tar data accepted")
	}
}
