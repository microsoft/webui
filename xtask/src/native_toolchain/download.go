// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Build-only HTTPS producer. No modules, credentials, Node or target execution.
// Rust verifies the exact frozen SHA-512 before interpreting the output.
package main

import (
	"crypto/tls"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"
)

const maxPackageBytes int64 = 128 * 1024 * 1024

func main() {
	if len(os.Args) != 3 {
		fmt.Fprintln(os.Stderr, "expected official GitHub release URL and output path")
		os.Exit(2)
	}
	if err := acquire(os.Args[1], os.Args[2]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func acquire(rawURL, output string) error {
	address, err := url.Parse(rawURL)
	if err != nil {
		return err
	}
	if err := allowedURL(address); err != nil {
		return err
	}
	if address.Hostname() != "github.com" ||
		!strings.HasPrefix(address.Path, "/microsoft/TypeScript/releases/download/") ||
		address.RawQuery != "" {
		return errors.New("initial download URL must be an official microsoft/TypeScript release asset")
	}
	return download(client(), rawURL, output, maxPackageBytes)
}

func client() *http.Client {
	return &http.Client{
		Timeout: 5 * time.Minute,
		Transport: &http.Transport{
			Proxy:                  http.ProxyFromEnvironment,
			DialContext:            (&net.Dialer{Timeout: 30 * time.Second}).DialContext,
			ForceAttemptHTTP2:      true,
			TLSClientConfig:        &tls.Config{MinVersion: tls.VersionTLS12},
			TLSHandshakeTimeout:    30 * time.Second,
			ResponseHeaderTimeout:  30 * time.Second,
			MaxResponseHeaderBytes: 64 * 1024,
			MaxIdleConns:           1,
			MaxConnsPerHost:        2,
			IdleConnTimeout:        30 * time.Second,
		},
		CheckRedirect: redirect,
	}
}

func redirect(request *http.Request, via []*http.Request) error {
	if len(via) >= 10 {
		return errors.New("checker download exceeded ten redirects")
	}
	return allowedURL(request.URL)
}

func allowedURL(address *url.URL) error {
	if address.Scheme != "https" || address.User != nil || address.Fragment != "" ||
		(address.Port() != "" && address.Port() != "443") {
		return errors.New("checker downloads require credential-free HTTPS URLs")
	}
	switch address.Hostname() {
	case "github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com", "github-releases.githubusercontent.com":
		return nil
	default:
		return errors.New("checker download redirected outside the approved GitHub release hosts")
	}
}

func download(client *http.Client, rawURL, output string, maximum int64) (err error) {
	request, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return err
	}
	request.Header.Set("Accept", "application/octet-stream")
	request.Header.Set("User-Agent", "webui-native-toolchain")
	response, err := client.Do(request)
	if err != nil {
		return fmt.Errorf("native HTTPS download failed: %w", err)
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return fmt.Errorf("checker release download returned HTTP %d", response.StatusCode)
	}
	if response.ContentLength > maximum {
		return errors.New("checker release package exceeds its byte limit")
	}
	file, err := os.OpenFile(output, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	defer func() {
		if closeErr := file.Close(); err == nil {
			err = closeErr
		}
		if err != nil {
			_ = os.Remove(output)
		}
	}()
	written, err := io.CopyBuffer(file, io.LimitReader(response.Body, maximum+1), make([]byte, 16*1024))
	if err != nil {
		return err
	}
	if written > maximum {
		return errors.New("checker release package exceeds its byte limit")
	}
	return nil
}
