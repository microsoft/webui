// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.IO;
using System.Linq;
using Xunit;

namespace Microsoft.WebUI.Tests;

public class WebUIHandlerTests
{
    [Fact]
    public void Handler_CreateAndDispose_DoesNotThrow()
    {
        using var handler = new WebUIHandler();
        // Handler created successfully — dispose should clean up
    }

    [Fact]
    public void Handler_CreateWithPlugin_DoesNotThrow()
    {
        using var handler = new WebUIHandler("fast");
    }

    [Fact]
    public void Handler_DoubleDispose_DoesNotThrow()
    {
        var handler = new WebUIHandler();
        handler.Dispose();
        handler.Dispose(); // Should not throw
    }

    [Fact]
    public void Handler_RenderAfterDispose_ThrowsObjectDisposedException()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));
        using var protocol = new Protocol(protocolBytes);
        var handler = new WebUIHandler();
        handler.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            handler.Render(protocol, "{}", "index.html", "/"));
    }

    [Fact]
    public void Handler_Render_PreservesFullStateWithoutManifest()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Join(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));

        using var protocol = new Protocol(protocolBytes);
        using var handler = new WebUIHandler("webui");
        string html = handler.Render(
            protocol,
            "{\"kept\":\"KEPT_VALUE\",\"dropped\":\"DROPPED_VALUE\"}",
            "index.html",
            "/");

        Assert.Contains("\"kept\":\"KEPT_VALUE\"", html);
        Assert.Contains("\"dropped\":\"DROPPED_VALUE\"", html);
    }

    [Fact]
    public void Handler_Render_ReusesDecodedProtocol()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));

        using var protocol = new Protocol(protocolBytes);
        using var handler = new WebUIHandler("webui");

        string first = handler.Render(
            protocol,
            "{\"kept\":\"FIRST\",\"dropped\":\"SECRET\"}",
            "index.html",
            "/");
        string second = handler.Render(
            protocol,
            "{\"kept\":\"SECOND\",\"dropped\":\"SECRET\"}",
            "index.html",
            "/");

        Assert.Contains("\"kept\":\"FIRST\"", first);
        Assert.Contains("\"kept\":\"SECOND\"", second);
        Assert.Contains("\"dropped\":\"SECRET\"", first);
        Assert.Contains("\"dropped\":\"SECRET\"", second);
    }

    [Fact]
    public void Handler_Render_DecodesLargeMultiByteDocumentWhole()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Join(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));
        string kept = string.Concat(Enumerable.Repeat("Zoë 🐍 ✓ ", 20_000));

        using var protocol = new Protocol(protocolBytes);
        using var handler = new WebUIHandler("webui");
        string html = handler.Render(protocol, "{\"kept\":\"" + kept + "\"}", "index.html", "/");

        Assert.Contains(kept, html);
        Assert.EndsWith("</html>", html.TrimEnd());
    }

    [Fact]
    public void Handler_Render_ReturnsNulCharactersInOutput()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Join(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));

        using var protocol = new Protocol(protocolBytes);
        using var handler = new WebUIHandler("webui");
        string html = handler.Render(
            protocol,
            "{\"kept\":\"before\\u0000after\"}",
            "index.html",
            "/");

        Assert.Contains("before\0after", html);
        Assert.EndsWith("</html>", html.TrimEnd());
    }

    [Fact]
    public void Handler_RenderWithDisposedProtocol_ThrowsObjectDisposedException()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));
        var protocol = new Protocol(protocolBytes);
        protocol.Dispose();

        using var handler = new WebUIHandler();
        Assert.Throws<ObjectDisposedException>(() =>
            handler.Render(protocol, "{}", "index.html", "/"));
    }
}
