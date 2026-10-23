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
    public void Protocol_LazyResourcesAndState_PreserveAcknowledgementAndProjection()
    {
        using var protocol = new Protocol(File.ReadAllBytes(Path.Combine(
            AppContext.BaseDirectory, "fixtures", "lazy-app", "protocol.bin")));
        var prepared = protocol.PrepareLazyResources("test-panel", "details");
        using var metadata = JsonDocument.Parse(prepared.Json);
        Assert.Equal("webui-lazy", metadata.RootElement.GetProperty("type").GetString());
        Assert.False(metadata.RootElement.TryGetProperty("state", out _));
        Assert.Equal(prepared.Json, protocol.RenderLazyResources("test-panel", "details"));
        string body = protocol.RenderLazyPartial(
            """{"message":"Fresh","items":["one"],"unused":"not sent"}""",
            "test-panel", "details", prepared.Options);
        using var response = JsonDocument.Parse(body);
        Assert.False(response.RootElement.TryGetProperty("templates", out _));
        Assert.Equal("Fresh", response.RootElement.GetProperty("state").GetProperty("message").GetString());
        Assert.False(response.RootElement.GetProperty("state").TryGetProperty("unused", out _));
    }

    [Fact]
    public void Protocol_LazyRequests_RejectMissingTargetsStaleBuildsAndDisposedHandles()
    {
        var protocol = new Protocol(File.ReadAllBytes(Path.Combine(
            AppContext.BaseDirectory, "fixtures", "lazy-app", "protocol.bin")));
        Assert.Throws<WebUIException>(() => protocol.RenderLazyResources("test-panel", "missing"));
        Assert.Throws<WebUIException>(() => protocol.RenderLazyResources("test-panel", "details",
            new LazyOptions { BuildId = "different" }));
        protocol.Dispose();
        Assert.Throws<ObjectDisposedException>(() => protocol.RenderLazyPartial("{}", "test-panel", "details"));
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
