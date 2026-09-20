// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.IO;
using System.Text.Json;
using Xunit;

namespace Microsoft.WebUI.Tests;

public class ProtocolTests
{
    [Fact]
    public void Protocol_RenderPartial_ReturnsCompleteResponse()
    {
        using var protocol = LoadProtocol();
        string response = protocol.RenderPartial(
            """{"kept":"VALUE"}""",
            "index.html",
            "/",
            "");

        using JsonDocument json = JsonDocument.Parse(response);
        Assert.Equal(JsonValueKind.Object, json.RootElement.GetProperty("state").ValueKind);
        Assert.True(json.RootElement.TryGetProperty("templates", out _));
        Assert.True(json.RootElement.TryGetProperty("inventory", out _));
    }

    [Fact]
    public void Protocol_RenderComponentTemplates_ReturnsRequestedTemplate()
    {
        using var protocol = LoadProtocol();
        string response = protocol.RenderComponentTemplates(
            ["kept-widget"],
            "");

        using JsonDocument json = JsonDocument.Parse(response);
        Assert.True(
            json.RootElement
                .GetProperty("templates")
                .TryGetProperty("kept-widget", out _));
    }

    [Fact]
    public void Protocol_Tokens_ReturnsBuildOrder()
    {
        using var protocol = LoadProtocol();
        Assert.Empty(protocol.Tokens());
    }

    [Fact]
    public void Protocol_OperationAfterDispose_ThrowsObjectDisposedException()
    {
        var protocol = LoadProtocol();
        protocol.Dispose();
        Assert.Throws<ObjectDisposedException>(() => protocol.Tokens());
    }

    private static Protocol LoadProtocol()
    {
        byte[] protocolBytes = File.ReadAllBytes(
            Path.Combine(AppContext.BaseDirectory, "fixtures", "projection-app", "protocol.bin"));
        return new Protocol(protocolBytes);
    }
}
