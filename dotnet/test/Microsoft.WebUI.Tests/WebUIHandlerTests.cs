// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.IO;
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
        var handler = new WebUIHandler();
        handler.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            handler.Render(Array.Empty<byte>(), "{}", "index.html", "/"));
    }

    [Fact]
    public void Handler_Render_ProjectsStateToHydrationSchema()
    {
        // The protocol is compiled at build time by the webui CLI from the source
        // app under fixtures/projection-app (see the BuildProjectionFixtureProtocol
        // target). That app's single component binds only `{{kept}}`, so the
        // emitted hydration schema is ["kept"]. The WebUI plugin must project the
        // render state down to that allowlist before emitting the #webui-data
        // bootstrap block, dropping server-only fields.
        byte[] protocol = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));

        using var handler = new WebUIHandler("webui");
        string html = handler.Render(
            protocol,
            "{\"kept\":\"KEPT_VALUE\",\"dropped\":\"DROPPED_VALUE\"}",
            "index.html",
            "/");

        // The hydratable key survives...
        Assert.Contains("\"kept\":\"KEPT_VALUE\"", html);
        // ...and the server-only key is projected out entirely.
        Assert.DoesNotContain("DROPPED_VALUE", html);
        Assert.DoesNotContain("dropped", html);
    }

    [Fact]
    public void Handler_RenderPrepared_ReusesDecodedProtocol()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));

        using var protocol = new PreparedProtocol(protocolBytes);
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
        Assert.DoesNotContain("SECRET", first);
        Assert.DoesNotContain("SECRET", second);
    }

    [Fact]
    public void Handler_RenderWithDisposedPreparedProtocol_ThrowsObjectDisposedException()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));
        var protocol = new PreparedProtocol(protocolBytes);
        protocol.Dispose();

        using var handler = new WebUIHandler();
        Assert.Throws<ObjectDisposedException>(() =>
            handler.Render(protocol, "{}", "index.html", "/"));
    }
}
