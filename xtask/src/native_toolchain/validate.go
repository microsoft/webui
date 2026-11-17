// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Host-only streaming archive verifier. Never executes payload binaries.
package main

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
)

type specification struct {
	Archive           string
	SHA256            string
	Output            string
	Version           string
	Target            string
	TypeScriptVersion string
	Files             map[string]int64
	Binaries          []string
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "expected release specification")
		os.Exit(2)
	}
	if err := run(os.Args[1]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run(path string) error {
	data, err := os.ReadFile(path) // Rust generates this small private specification.
	if err != nil {
		return err
	}
	var specifications []specification
	if err := json.Unmarshal(data, &specifications); err != nil {
		return err
	}
	for _, spec := range specifications {
		if err := validate(spec); err != nil {
			return fmt.Errorf("%s: %w", spec.Target, err)
		}
	}
	return nil
}

func validate(spec specification) error {
	file, err := os.Open(spec.Archive)
	if err != nil {
		return err
	}
	defer file.Close()
	digest := sha256.New()
	compressed := &io.LimitedReader{R: io.TeeReader(file, digest), N: 1024*1024*1024 + 1}
	zipped, err := gzip.NewReader(compressed)
	if err != nil {
		return err
	}
	defer zipped.Close()
	// Bound expansion too, including PAX headers consumed internally by tar.
	// The Go standard reader separately caps each special header to 1 MiB.
	maximum := int64(128*1024) + int64(len(spec.Files)+4)*(1024*1024)
	for _, limit := range spec.Files {
		maximum += limit
	}
	expanded := &io.LimitedReader{R: zipped, N: maximum + 1}
	err = validateTar(spec, expanded)
	if expanded.N == 0 {
		return errors.New("expanded archive exceeds its byte budget")
	}
	if err != nil {
		return err
	}
	if _, err := io.CopyBuffer(io.Discard, compressed, make([]byte, 16*1024)); err != nil {
		return err
	}
	if compressed.N == 0 || hex.EncodeToString(digest.Sum(nil)) != spec.SHA256 {
		return errors.New("compressed archive changed or exceeded its byte budget")
	}
	return nil
}

func validateTar(spec specification, expanded io.Reader) error {
	reader := tar.NewReader(expanded)
	hashes := make(map[string]string, len(spec.Files))
	seen := make(map[string]bool, len(spec.Files)+4)
	buffer := make([]byte, 16*1024)
	var metadata []byte
	for {
		header, err := reader.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return err
		}
		if seen[header.Name] || len(seen) >= len(spec.Files)+4 {
			return errors.New("duplicate or excessive archive members")
		}
		seen[header.Name] = true
		if header.Typeflag == tar.TypeDir {
			if header.Name != "bin/" && header.Name != "compiler/" && header.Name != "notices/" {
				return errors.New("unknown archive directory")
			}
			continue
		}
		if header.Typeflag != tar.TypeReg && header.Typeflag != tar.TypeRegA {
			return errors.New("archive members must be regular files, never links")
		}
		if header.Name == "toolchain.json" {
			if header.Size > 65536 || header.Size < 0 {
				return errors.New("metadata exceeds 64 KiB")
			}
			metadata, err = io.ReadAll(reader)
			if err != nil {
				return err
			}
			continue
		}
		limit, ok := spec.Files[header.Name]
		if !ok || header.Size < 0 || header.Size > limit {
			return fmt.Errorf("unknown, escaping or oversized inventory file %q", header.Name)
		}
		digest := sha256.New()
		var output *os.File
		var writer io.Writer = digest
		for _, binary := range spec.Binaries {
			if header.Name == binary {
				path := filepath.Join(spec.Output, filepath.FromSlash(binary))
				if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
					return err
				}
				output, err = os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
				if err != nil {
					return err
				}
				writer = io.MultiWriter(digest, output)
			}
		}
		_, err = io.CopyBuffer(writer, reader, buffer)
		if output != nil {
			closeErr := output.Close()
			if err == nil {
				err = closeErr
			}
		}
		if err != nil {
			return err
		}
		hashes[header.Name] = hex.EncodeToString(digest.Sum(nil))
	}
	// Consume gzip trailers/CRC and reject a hidden second tar or nonzero data.
	padding, err := io.ReadAll(io.LimitReader(expanded, 65537))
	if err != nil {
		return err
	}
	if len(padding) > 65536 || !bytes.Equal(padding, make([]byte, len(padding))) {
		return errors.New("unexpected trailing archive data")
	}
	return validateMetadata(spec, metadata, hashes)
}

func object(data []byte) (map[string]json.RawMessage, error) {
	decoder := json.NewDecoder(bytes.NewReader(data))
	first, err := decoder.Token()
	if err != nil || first != json.Delim('{') {
		return nil, errors.New("expected metadata object")
	}
	values := make(map[string]json.RawMessage)
	for decoder.More() {
		token, err := decoder.Token()
		if err != nil {
			return nil, err
		}
		key, ok := token.(string)
		if !ok || values[key] != nil {
			return nil, errors.New("duplicate metadata key")
		}
		var value json.RawMessage
		if err := decoder.Decode(&value); err != nil {
			return nil, err
		}
		values[key] = value
	}
	if _, err := decoder.Token(); err != nil {
		return nil, err
	}
	var extra any
	if decoder.Decode(&extra) != io.EOF {
		return nil, errors.New("unexpected trailing metadata")
	}
	return values, nil
}

func validateMetadata(spec specification, data []byte, hashes map[string]string) error {
	metadata, err := object(data)
	if err != nil {
		return err
	}
	if len(metadata) != 5 || len(hashes) != len(spec.Files) {
		return errors.New("incomplete metadata or file inventory")
	}
	var schema int
	if json.Unmarshal(metadata["schemaVersion"], &schema) != nil || schema != 1 {
		return errors.New("expected toolchain schema 1")
	}
	for key, expected := range map[string]string{
		"version": spec.Version, "target": spec.Target, "typescriptVersion": spec.TypeScriptVersion,
	} {
		var actual string
		if json.Unmarshal(metadata[key], &actual) != nil || actual != expected {
			return fmt.Errorf("toolchain %s mismatch", key)
		}
	}
	files, err := object(metadata["files"])
	if err != nil || len(files) != len(spec.Files) {
		return errors.New("invalid or duplicate file inventory")
	}
	for name := range spec.Files {
		var expected string
		actual, present := hashes[name]
		if json.Unmarshal(files[name], &expected) != nil || len(expected) != 64 || !present || expected != actual {
			return fmt.Errorf("payload SHA-256 mismatch: %s", name)
		}
	}
	return nil
}
