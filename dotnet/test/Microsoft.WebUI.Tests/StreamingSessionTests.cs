// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.IO;
using System.Text;
using Xunit;

namespace Microsoft.WebUI.Tests;

public class StreamingSessionTests
{
    private static byte[] StreamingProtocolBytes() =>
        File.ReadAllBytes(Path.Combine(
            AppContext.BaseDirectory,
            "fixtures",
            "streaming-app",
            "protocol.bin"));

    private static string Text(byte[] chunk) => Encoding.UTF8.GetString(chunk);

    [Fact]
    public void Session_ReturnsOneChunkPerCallAndReassemblesTheDocument()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        Assert.Equal(2u, session.BoundaryCount);

        uint first = session.Boundary("first");
        uint second = session.Boundary("second");
        Assert.Equal(0u, first);
        Assert.Equal(1u, second);

        byte[][] chunks =
        [
            session.WriteShell("{}"),
            session.WriteBoundary(first, "{\"firstLabel\":\"alpha\"}", BoundaryMode.Updatable),
            session.Update(first, "{\"firstLabel\":\"alpha-2\"}"),
            session.WriteBoundary(second, "{\"secondLabel\":\"beta\"}"),
            session.Finish(),
        ];

        Assert.True(session.IsFinished);

        var document = new StringBuilder();
        foreach (byte[] chunk in chunks)
        {
            Assert.NotEmpty(chunk);
            document.Append(Text(chunk));
        }

        string html = document.ToString();
        Assert.StartsWith("<!DOCTYPE html>", html, StringComparison.OrdinalIgnoreCase);
        Assert.EndsWith("</html>", html.TrimEnd());
        Assert.Contains("alpha", html, StringComparison.Ordinal);
        Assert.Contains("alpha-2", html, StringComparison.Ordinal);
        Assert.Contains("beta", html, StringComparison.Ordinal);

        // Each boundary is rendered exactly once, no matter how many chunks it took.
        Assert.Equal(2, CountOccurrences(html, "<stream-item"));
    }

    [Fact]
    public void Session_UnknownBoundaryName_ThrowsWithSuggestions()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        var error = Assert.Throws<WebUIException>(() => session.Boundary("frist"));
        Assert.Contains("frist", error.Message, StringComparison.Ordinal);
        Assert.Contains("first", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void Session_OutOfOrderBoundary_Throws()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        uint second = session.Boundary("second");
        session.WriteShell("{}");

        Assert.Throws<WebUIException>(() =>
            session.WriteBoundary(second, "{\"secondLabel\":\"beta\"}"));
    }

    [Fact]
    public void Session_UpdateToFinalBoundary_Throws()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        uint first = session.Boundary("first");
        session.WriteShell("{}");
        session.WriteBoundary(first, "{\"firstLabel\":\"alpha\"}");

        Assert.Throws<WebUIException>(() =>
            session.Update(first, "{\"firstLabel\":\"alpha-2\"}"));
    }

    [Fact]
    public void Session_CallAfterFinish_Throws()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        uint first = session.Boundary("first");
        uint second = session.Boundary("second");
        session.WriteShell("{}");
        session.WriteBoundary(first, "{\"firstLabel\":\"alpha\"}");
        session.WriteBoundary(second, "{\"secondLabel\":\"beta\"}");
        session.Finish();

        Assert.Throws<WebUIException>(() => session.Finish());
        Assert.Throws<WebUIException>(() =>
            session.WriteBoundary(second, "{\"secondLabel\":\"gamma\"}"));
    }

    [Fact]
    public void Session_InvalidStateJson_Throws()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        Assert.Throws<WebUIException>(() => session.WriteShell("not json"));
    }

    [Fact]
    public void Session_StaysUsableAfterARejectedCall()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        uint first = session.Boundary("first");
        uint second = session.Boundary("second");

        Assert.Throws<WebUIException>(() => session.Boundary("missing"));

        session.WriteShell("{}");
        session.WriteBoundary(first, "{\"firstLabel\":\"alpha\"}");
        session.WriteBoundary(second, "{\"secondLabel\":\"beta\"}");
        byte[] tail = session.Finish();

        Assert.NotEmpty(tail);
        Assert.True(session.IsFinished);
    }

    [Fact]
    public void Session_OutOfOrderFinish_LeavesSessionUsable()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        uint first = session.Boundary("first");
        uint second = session.Boundary("second");

        session.WriteShell("{}");
        session.WriteBoundary(first, "{\"firstLabel\":\"alpha\"}");

        // Rejected before any byte is written, so the open response survives.
        Assert.Throws<WebUIException>(() => session.Finish());
        Assert.False(session.IsFinished);

        session.WriteBoundary(second, "{\"secondLabel\":\"beta\"}");
        Assert.NotEmpty(session.Finish());
        Assert.True(session.IsFinished);
    }

    [Fact]
    public void Session_ConcurrentSessionsAreIndependent()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession a = handler.StreamResponse(protocol, "index.html", "/");
        using StreamingSession b = handler.StreamResponse(protocol, "index.html", "/");

        uint firstA = a.Boundary("first");
        uint firstB = b.Boundary("first");

        a.WriteShell("{}");
        b.WriteShell("{}");

        string chunkA = Text(a.WriteBoundary(firstA, "{\"firstLabel\":\"from-a\"}"));
        string chunkB = Text(b.WriteBoundary(firstB, "{\"firstLabel\":\"from-b\"}"));

        Assert.Contains("from-a", chunkA, StringComparison.Ordinal);
        Assert.DoesNotContain("from-b", chunkA, StringComparison.Ordinal);
        Assert.Contains("from-b", chunkB, StringComparison.Ordinal);
        Assert.DoesNotContain("from-a", chunkB, StringComparison.Ordinal);
    }

    [Fact]
    public void Session_UseAfterDispose_ThrowsObjectDisposedException()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");
        session.Dispose();
        session.Dispose(); // Idempotent.

        Assert.Throws<ObjectDisposedException>(() => session.WriteShell("{}"));
        Assert.Throws<ObjectDisposedException>(() => session.Boundary("first"));
        Assert.Throws<ObjectDisposedException>(() => session.BoundaryCount);
    }

    [Fact]
    public void Session_UnfinishedDisposeDoesNotThrow()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");
        session.WriteShell("{}");
        session.Dispose();
    }

    [Fact]
    public void StreamResponse_WithDisposedHandler_ThrowsObjectDisposedException()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        var handler = new WebUIHandler("webui");
        handler.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            handler.StreamResponse(protocol, "index.html", "/"));
    }

    [Fact]
    public void StreamResponse_WithDisposedProtocol_ThrowsObjectDisposedException()
    {
        var protocol = new Protocol(StreamingProtocolBytes());
        protocol.Dispose();

        using var handler = new WebUIHandler("webui");
        Assert.Throws<ObjectDisposedException>(() =>
            handler.StreamResponse(protocol, "index.html", "/"));
    }

    [Fact]
    public void StreamResponse_WithUnknownEntry_Throws()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");

        Assert.Throws<WebUIException>(() =>
            handler.StreamResponse(protocol, "missing.html", "/"));
    }

    private static int CountOccurrences(string haystack, string needle)
    {
        int count = 0;
        int index = haystack.IndexOf(needle, StringComparison.Ordinal);
        while (index >= 0)
        {
            count += 1;
            index = haystack.IndexOf(needle, index + needle.Length, StringComparison.Ordinal);
        }

        return count;
    }
}
