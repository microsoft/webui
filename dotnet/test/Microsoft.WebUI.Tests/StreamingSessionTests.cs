// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.IO;
using System.Text;
using Xunit;

namespace Microsoft.WebUI.Tests;

public class StreamingSessionTests
{
    private const string StreamingState =
        """{"show":true,"items":[{"id":"alpha\u0000omega","label":"first"},{"id":7,"label":"second"}]}""";

    private static byte[] StreamingProtocolBytes() =>
        File.ReadAllBytes(Path.Combine(
            AppContext.BaseDirectory,
            "fixtures",
            "streaming-app",
            "protocol.bin"));

    private static string Text(byte[] chunk) => Encoding.UTF8.GetString(chunk);

    [Fact]
    public void Session_StartResumeAndUpdateExposeTypedDescriptorsAndComplete()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        StreamingStep first = session.Start(StreamingState);
        Assert.False(first.Done);
        BoundaryDescriptor firstBoundary = Assert.IsType<BoundaryDescriptor>(first.Boundary);
        Assert.Equal(0u, firstBoundary.InstanceId);
        Assert.Equal("index.html", firstBoundary.Owner);
        Assert.Equal("row", firstBoundary.Name);
        Assert.Equal(BoundaryKeyType.String, firstBoundary.Key.Type);
        Assert.Equal("alpha\0omega", firstBoundary.Key.StringValue);
        Assert.Null(firstBoundary.Key.NumberValue);

        StreamingStep second = session.Resume(
            firstBoundary.InstanceId,
            StreamingState,
            BoundaryMode.Updatable);
        Assert.False(second.Done);
        BoundaryDescriptor secondBoundary = Assert.IsType<BoundaryDescriptor>(second.Boundary);
        Assert.Equal(1u, secondBoundary.InstanceId);
        Assert.Equal(firstBoundary.DeclarationId, secondBoundary.DeclarationId);
        Assert.Equal(BoundaryKeyType.Number, secondBoundary.Key.Type);
        Assert.Null(secondBoundary.Key.StringValue);
        Assert.Equal(7.0, secondBoundary.Key.NumberValue);

        byte[] update = session.Update(firstBoundary.InstanceId, """{"label":"updated"}""");
        Assert.NotEmpty(update);
        Assert.Contains(@"""label"":""updated""", Text(update), StringComparison.Ordinal);

        StreamingStep done = session.Resume(
            secondBoundary.InstanceId,
            StreamingState,
            BoundaryMode.Final);
        Assert.True(done.Done);
        Assert.Null(done.Boundary);

        string document = Text(first.Bytes) + Text(second.Bytes) + Text(update) + Text(done.Bytes);
        Assert.StartsWith("<!DOCTYPE html>", document, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("first", document, StringComparison.Ordinal);
        Assert.Contains("second", document, StringComparison.Ordinal);
        Assert.Contains("</html>", document, StringComparison.Ordinal);
        Assert.Equal(2, CountOccurrences(document, "<stream-item"));
    }

    [Fact]
    public void Session_BoundaryFreeStateCompletesFromStart()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        StreamingStep step = session.Start("""{"show":false,"items":[]}""");

        Assert.True(step.Done);
        Assert.Null(step.Boundary);
        Assert.Contains("always-complete", Text(step.Bytes), StringComparison.Ordinal);
    }

    [Fact]
    public void Session_StaleResumeThrowsAndLeavesPendingBoundaryUsable()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        StreamingStep first = session.Start(StreamingState);
        BoundaryDescriptor boundary = Assert.IsType<BoundaryDescriptor>(first.Boundary);

        WebUIException error = Assert.Throws<WebUIException>(() =>
            session.Resume(99, StreamingState));
        Assert.Contains("stale", error.Message, StringComparison.OrdinalIgnoreCase);

        StreamingStep second = session.Resume(boundary.InstanceId, StreamingState);
        Assert.NotNull(second.Boundary);
    }

    [Fact]
    public void Session_UpdateToFinalBoundaryThrows()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        BoundaryDescriptor first = Assert.IsType<BoundaryDescriptor>(
            session.Start(StreamingState).Boundary);
        StreamingStep secondStep = session.Resume(first.InstanceId, StreamingState);
        BoundaryDescriptor second = Assert.IsType<BoundaryDescriptor>(secondStep.Boundary);

        Assert.Throws<WebUIException>(() =>
            session.Update(first.InstanceId, """{"label":"ignored"}"""));

        Assert.True(session.Resume(second.InstanceId, StreamingState).Done);
    }

    [Fact]
    public void Session_InvalidJsonThrowsAndSessionCanRetry()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        Assert.Throws<WebUIException>(() => session.Start("not json"));
        Assert.NotNull(session.Start(StreamingState).Boundary);
    }

    [Fact]
    public void Session_NullJsonArgumentsThrow()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        Assert.Throws<ArgumentNullException>(() => session.Start(null!));
        BoundaryDescriptor first = Assert.IsType<BoundaryDescriptor>(
            session.Start(StreamingState).Boundary);
        Assert.Throws<ArgumentNullException>(() =>
            session.Resume(first.InstanceId, null!));
        Assert.Throws<ArgumentNullException>(() =>
            session.Update(first.InstanceId, null!));
    }

    [Fact]
    public void Session_InvalidBoundaryModeThrows()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        BoundaryDescriptor first = Assert.IsType<BoundaryDescriptor>(
            session.Start(StreamingState).Boundary);
        Assert.Throws<ArgumentOutOfRangeException>(() =>
            session.Resume(first.InstanceId, StreamingState, (BoundaryMode)99));
    }

    [Fact]
    public void Session_CallAfterDoneThrows()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        BoundaryDescriptor first = Assert.IsType<BoundaryDescriptor>(
            session.Start(StreamingState).Boundary);
        BoundaryDescriptor second = Assert.IsType<BoundaryDescriptor>(
            session.Resume(first.InstanceId, StreamingState).Boundary);
        Assert.True(session.Resume(second.InstanceId, StreamingState).Done);

        Assert.Throws<WebUIException>(() => session.Start(StreamingState));
        Assert.Throws<WebUIException>(() =>
            session.Resume(second.InstanceId, StreamingState));
    }

    [Fact]
    public void Session_ConcurrentSessionsAreIndependent()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession a = handler.StreamResponse(protocol, "index.html", "/");
        using StreamingSession b = handler.StreamResponse(protocol, "index.html", "/");

        BoundaryDescriptor firstA = Assert.IsType<BoundaryDescriptor>(
            a.Start(StreamingState).Boundary);
        BoundaryDescriptor firstB = Assert.IsType<BoundaryDescriptor>(
            b.Start(StreamingState.Replace("first", "from-b", StringComparison.Ordinal)).Boundary);

        string chunkA = Text(a.Resume(firstA.InstanceId, StreamingState).Bytes);
        string chunkB = Text(b.Resume(
            firstB.InstanceId,
            StreamingState.Replace("first", "from-b", StringComparison.Ordinal)).Bytes);

        Assert.Contains("first", chunkA, StringComparison.Ordinal);
        Assert.DoesNotContain("from-b", chunkA, StringComparison.Ordinal);
        Assert.Contains("from-b", chunkB, StringComparison.Ordinal);
    }

    [Fact]
    public void Session_EagerStepCopySurvivesSessionDisposal()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        StreamingStep step = session.Start(StreamingState);
        session.Dispose();

        Assert.NotEmpty(step.Bytes);
        Assert.Equal("alpha\0omega", step.Boundary?.Key.StringValue);
    }

    [Fact]
    public void Session_UseAfterDisposeThrowsObjectDisposedException()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");
        session.Dispose();
        session.Dispose();

        Assert.Throws<ObjectDisposedException>(() => session.Start(StreamingState));
        Assert.Throws<ObjectDisposedException>(() => session.Update(0, "{}"));
    }

    [Fact]
    public void Session_UnfinishedDisposeDoesNotThrow()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");
        session.Start(StreamingState);
        session.Dispose();
    }

    [Fact]
    public void StreamResponse_WithDisposedHandlerThrowsObjectDisposedException()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        var handler = new WebUIHandler("webui");
        handler.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            handler.StreamResponse(protocol, "index.html", "/"));
    }

    [Fact]
    public void StreamResponse_WithDisposedProtocolThrowsObjectDisposedException()
    {
        var protocol = new Protocol(StreamingProtocolBytes());
        protocol.Dispose();

        using var handler = new WebUIHandler("webui");
        Assert.Throws<ObjectDisposedException>(() =>
            handler.StreamResponse(protocol, "index.html", "/"));
    }

    [Fact]
    public void StreamResponse_WithUnknownEntryThrows()
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
