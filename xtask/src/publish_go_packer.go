// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Release-only helper: use Go's standard ZIP writer, not a bespoke ZIP format
// or a new Rust dependency. Invoked from a temporary directory, never in go/.
package main

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode"
)

const module = "github.com/microsoft/webui/go/v3"

func main() {
	if err := pack(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func pack() error {
	source, license, output, version := os.Args[1], os.Args[2], os.Args[3], "v"+os.Args[5]
	manifest, err := os.ReadFile(filepath.Join(source, "go.mod"))
	if err != nil {
		return err
	}
	cmd := exec.Command("go", "mod", "edit", "-json")
	cmd.Dir = source
	data, err := cmd.Output()
	if err != nil {
		return fmt.Errorf("invalid Go module manifest: %w", err)
	}
	var mod struct{ Module struct{ Path string } }
	if err := json.Unmarshal(data, &mod); err != nil {
		return err
	}
	if mod.Module.Path != module {
		return fmt.Errorf("Go module path mismatch: expected %s, got %s", module, mod.Module.Path)
	}
	files := make(map[string]string)
	folded := make(map[string]bool)
	var total int64
	var bindingVersions []string
	err = filepath.WalkDir(source, func(path string, entry fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if path == source {
			if entry.Type()&os.ModeSymlink != 0 {
				return fmt.Errorf("Go source root must not be a symlink")
			}
			return nil
		}
		relative, err := filepath.Rel(source, path)
		if err != nil {
			return err
		}
		relative = filepath.ToSlash(relative)
		if err := validPath(relative); err != nil {
			return err
		}
		key := strings.ToLower(relative)
		if folded[key] {
			return fmt.Errorf("case-insensitive Go source path collision: %s", relative)
		}
		folded[key] = true
		if entry.Type()&os.ModeSymlink != 0 {
			return fmt.Errorf("Go source symlink is forbidden: %s", relative)
		}
		if entry.IsDir() {
			switch entry.Name() {
			case ".git", ".hg", ".svn", ".bzr", "node_modules", "vendor", "sdk":
				return fmt.Errorf("non-source Go module directory forbidden: %s", relative)
			}
			return nil
		}
		if !entry.Type().IsRegular() {
			return fmt.Errorf("Go source must be a regular file: %s", relative)
		}
		if relative != "go.mod" && strings.EqualFold(entry.Name(), "go.mod") {
			return fmt.Errorf("nested Go module is forbidden: %s", relative)
		}
		switch strings.ToLower(filepath.Ext(relative)) {
		case ".a", ".o", ".so", ".dylib", ".dll", ".lib", ".exe", ".node", ".zip", ".gz", ".bin":
			return fmt.Errorf("generated native/archive payload forbidden in Go source: %s", relative)
		}
		if info, err := entry.Info(); err != nil {
			return err
		} else if info.Size() > 16<<20 {
			return fmt.Errorf("Go source file exceeds 16 MiB: %s", relative)
		} else {
			total += info.Size()
			if total > 500<<20 {
				return fmt.Errorf("Go source exceeds module ZIP limit of 500 MiB")
			}
		}
		if strings.HasSuffix(relative, ".go") && !strings.HasSuffix(relative, "_test.go") {
			versions, err := sourceVersions(path)
			if err != nil {
				return err
			}
			bindingVersions = append(bindingVersions, versions...)
		}
		files[relative] = path
		return nil
	})
	if err != nil {
		return err
	}
	if len(bindingVersions) != 1 || bindingVersions[0] != os.Args[4] {
		return fmt.Errorf("Go binding Version must match release %s, got %v", os.Args[4], bindingVersions)
	}
	licenseBytes, err := os.ReadFile(license)
	if err != nil {
		return err
	}
	if local, ok := files["LICENSE"]; ok {
		localBytes, err := os.ReadFile(local)
		if err != nil {
			return err
		}
		if !bytes.Equal(localBytes, licenseBytes) {
			return fmt.Errorf("Go LICENSE differs from release LICENSE")
		}
	} else if folded["license"] {
		return fmt.Errorf("case-insensitive Go source path collision: LICENSE")
	}
	files["LICENSE"] = license
	var names []string
	for name := range files {
		names = append(names, name)
	}
	sort.Strings(names)
	if err := os.MkdirAll(output, 0755); err != nil {
		return err
	}
	archive, err := os.Create(filepath.Join(output, version+".zip"))
	if err != nil {
		return err
	}
	defer archive.Close()
	writer := zip.NewWriter(archive)
	for _, name := range names {
		header := &zip.FileHeader{Name: module + "@" + version + "/" + name, Method: zip.Deflate}
		header.SetMode(0644)
		header.SetModTime(time.Date(1980, 1, 1, 0, 0, 0, 0, time.UTC))
		file, err := writer.CreateHeader(header)
		if err != nil {
			return err
		}
		if err := copyFile(file, files[name]); err != nil {
			return err
		}
	}
	if err := writer.Close(); err != nil {
		return err
	}
	if err := archive.Close(); err != nil {
		return err
	}
	// This is a local prepublication proxy, not a published VCS tag. Record the
	// source snapshot time without inventing a tag timestamp or doing a lookup.
	sourceInfo, err := os.Stat(filepath.Join(source, "go.mod"))
	if err != nil {
		return err
	}
	info := fmt.Sprintf("{\"Version\":%q,\"Time\":%q}\n", version,
		sourceInfo.ModTime().UTC().Truncate(time.Second).Format(time.RFC3339))
	for name, content := range map[string][]byte{
		version + ".mod": manifest, version + ".info": []byte(info),
		"list": []byte(version + "\n"),
	} {
		if err := os.WriteFile(filepath.Join(output, name), content, 0644); err != nil {
			return err
		}
	}
	return nil
}

func sourceVersions(path string) ([]string, error) {
	source, err := parser.ParseFile(token.NewFileSet(), path, nil, parser.SkipObjectResolution)
	if err != nil {
		return nil, err
	}
	var versions []string
	for _, decl := range source.Decls {
		group, ok := decl.(*ast.GenDecl)
		if !ok || group.Tok != token.CONST {
			continue
		}
		for _, spec := range group.Specs {
			value, ok := spec.(*ast.ValueSpec)
			if !ok {
				continue
			}
			for index, name := range value.Names {
				if name.Name != "Version" {
					continue
				}
				if index >= len(value.Values) {
					return nil, fmt.Errorf("binding Version must be an explicit string constant")
				}
				literal, ok := value.Values[index].(*ast.BasicLit)
				if !ok || literal.Kind != token.STRING {
					return nil, fmt.Errorf("binding Version must be a string literal")
				}
				version, err := strconv.Unquote(literal.Value)
				if err != nil {
					return nil, err
				}
				versions = append(versions, version)
			}
		}
	}
	return versions, nil
}

func copyFile(output io.Writer, path string) error {
	input, err := os.Open(path)
	if err != nil {
		return err
	}
	defer input.Close()
	_, err = io.Copy(output, input)
	return err
}

func validPath(path string) error {
	for _, part := range strings.Split(path, "/") {
		if part == "" || part == "." || part == ".." || strings.HasSuffix(part, ".") {
			return fmt.Errorf("invalid Go source path: %s", path)
		}
		base := strings.ToUpper(strings.SplitN(part, ".", 2)[0])
		switch base {
		case "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
			"LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9":
			return fmt.Errorf("invalid Go source path: %s", path)
		}
		for _, c := range part {
			if !unicode.IsLetter(c) && !unicode.IsDigit(c) && !strings.ContainsRune("!#$%&()+,-.=@[]^_{}~ ", c) {
				return fmt.Errorf("invalid Go source path: %s", path)
			}
		}
	}
	return nil
}
