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
        // The fixture is a compiled protocol whose only hydration key is `kept`
        // (see fixtures/projected_protocol.bin, schema == ["kept"]). The WebUI
        // plugin must project the render state down to that allowlist before
        // emitting the #webui-data bootstrap block, dropping server-only fields.
        byte[] protocol = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projected_protocol.bin"));

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
}
