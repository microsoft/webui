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
        """{"show":true,"stringKey":"alpha\u0000omega","numberKey":7,"firstLabel":"first","secondLabel":"second"}""";

    private static byte[] StreamingProtocolBytes() =>
        File.ReadAllBytes(Path.Combine(
            AppContext.BaseDirectory,
            "fixtures",
            "streaming-app",
            "protocol.bin"));

    private static string Text(byte[] chunk) => Encoding.UTF8.GetString(chunk);

    [Fact]
    public void Session_StartResumeUpdateAndAdvanceExposeTypedDescriptorsAndComplete()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        StreamingStep first = session.Start(StreamingState);
        Assert.False(first.Done);
        BoundaryDescriptor firstBoundary = Assert.IsType<BoundaryDescriptor>(first.Boundary);
        Assert.Equal(0u, firstBoundary.InstanceId);
        Assert.Equal("index.html", firstBoundary.Owner);
        Assert.Equal("string-row", firstBoundary.Name);
        Assert.Equal(BoundaryKeyType.String, firstBoundary.Key.Type);
        Assert.Equal("alpha\0omega", firstBoundary.Key.StringValue);
        Assert.Null(firstBoundary.Key.NumberValue);

        StreamingStep firstCommit = session.Resume(
            firstBoundary.InstanceId,
            StreamingState,
            BoundaryMode.Updatable);
        Assert.False(firstCommit.Done);
        Assert.Null(firstCommit.Boundary);
        string firstCommitText = Text(firstCommit.Bytes);
        Assert.Contains("label=\"first\"", firstCommitText, StringComparison.Ordinal);
        Assert.DoesNotContain("between-boundaries", firstCommitText, StringComparison.Ordinal);
        Assert.DoesNotContain("label=\"second\"", firstCommitText, StringComparison.Ordinal);
        Assert.DoesNotContain("always-complete", firstCommitText, StringComparison.Ordinal);

        byte[] update = session.Update(firstBoundary.InstanceId, """{"firstLabel":"updated"}""");
        Assert.NotEmpty(update);
        Assert.Contains(@"""firstLabel"":""updated""", Text(update), StringComparison.Ordinal);

        StreamingStep second = session.Advance();
        Assert.False(second.Done);
        BoundaryDescriptor secondBoundary = Assert.IsType<BoundaryDescriptor>(second.Boundary);
        Assert.Equal(1u, secondBoundary.InstanceId);
        Assert.NotEqual(firstBoundary.DeclarationId, secondBoundary.DeclarationId);
        Assert.Equal("number-row", secondBoundary.Name);
        Assert.Equal(BoundaryKeyType.Number, secondBoundary.Key.Type);
        Assert.Null(secondBoundary.Key.StringValue);
        Assert.Equal(7.0, secondBoundary.Key.NumberValue);
        string secondDiscoveryText = Text(second.Bytes);
        Assert.Contains("between-boundaries", secondDiscoveryText, StringComparison.Ordinal);
        Assert.DoesNotContain("label=\"second\"", secondDiscoveryText, StringComparison.Ordinal);
        Assert.DoesNotContain("always-complete", secondDiscoveryText, StringComparison.Ordinal);

        StreamingStep secondCommit = session.Resume(
            secondBoundary.InstanceId,
            StreamingState,
            BoundaryMode.Final);
        Assert.False(secondCommit.Done);
        Assert.Null(secondCommit.Boundary);
        string secondCommitText = Text(secondCommit.Bytes);
        Assert.Contains("label=\"second\"", secondCommitText, StringComparison.Ordinal);
        Assert.DoesNotContain("always-complete", secondCommitText, StringComparison.Ordinal);
        Assert.DoesNotContain("</html>", secondCommitText, StringComparison.OrdinalIgnoreCase);

        StreamingStep done = session.Advance();
        Assert.True(done.Done);
        Assert.Null(done.Boundary);
        Assert.Contains("always-complete", Text(done.Bytes), StringComparison.Ordinal);

        string document =
            Text(first.Bytes) +
            firstCommitText +
            Text(update) +
            secondDiscoveryText +
            secondCommitText +
            Text(done.Bytes);
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

        StreamingStep step = session.Start("""{"show":false}""");

        Assert.True(step.Done);
        Assert.Null(step.Boundary);
        Assert.Contains("always-complete", Text(step.Bytes), StringComparison.Ordinal);
    }

    [Fact]
    public void Session_OutOfOrderCallsThrowAndLeavePendingBoundaryUsable()
    {
        using var protocol = new Protocol(StreamingProtocolBytes());
        using var handler = new WebUIHandler("webui");
        using StreamingSession session = handler.StreamResponse(protocol, "index.html", "/");

        StreamingStep first = session.Start(StreamingState);
        BoundaryDescriptor boundary = Assert.IsType<BoundaryDescriptor>(first.Boundary);

        WebUIException advanceError = Assert.Throws<WebUIException>(() => session.Advance());
        Assert.Contains("no committed boundary", advanceError.Message, StringComparison.OrdinalIgnoreCase);

        WebUIException error = Assert.Throws<WebUIException>(() =>
            session.Resume(99, StreamingState));
        Assert.Contains("stale", error.Message, StringComparison.OrdinalIgnoreCase);

        StreamingStep commit = session.Resume(boundary.InstanceId, StreamingState);
        Assert.False(commit.Done);
        Assert.Null(commit.Boundary);
        StreamingStep second = session.Advance();
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
        StreamingStep firstCommit = session.Resume(first.InstanceId, StreamingState);
        Assert.False(firstCommit.Done);
        Assert.Null(firstCommit.Boundary);

        Assert.Throws<WebUIException>(() =>
            session.Update(first.InstanceId, """{"label":"ignored"}"""));

        StreamingStep secondStep = session.Advance();
        BoundaryDescriptor second = Assert.IsType<BoundaryDescriptor>(secondStep.Boundary);
        StreamingStep secondCommit = session.Resume(second.InstanceId, StreamingState);
        Assert.False(secondCommit.Done);
        Assert.Null(secondCommit.Boundary);
        Assert.True(session.Advance().Done);
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
        StreamingStep firstCommit = session.Resume(first.InstanceId, StreamingState);
        Assert.False(firstCommit.Done);
        Assert.Null(firstCommit.Boundary);
        BoundaryDescriptor second = Assert.IsType<BoundaryDescriptor>(
            session.Advance().Boundary);
        StreamingStep secondCommit = session.Resume(second.InstanceId, StreamingState);
        Assert.False(secondCommit.Done);
        Assert.Null(secondCommit.Boundary);
        Assert.True(session.Advance().Done);

        Assert.Throws<WebUIException>(() => session.Start(StreamingState));
        Assert.Throws<WebUIException>(() =>
            session.Resume(second.InstanceId, StreamingState));
        Assert.Throws<WebUIException>(() => session.Advance());
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
        string stateB = StreamingState.Replace(
            @"""first""",
            @"""from-b""",
            StringComparison.Ordinal);
        BoundaryDescriptor firstB = Assert.IsType<BoundaryDescriptor>(
            b.Start(stateB).Boundary);

        string chunkA = Text(a.Resume(firstA.InstanceId, StreamingState).Bytes);
        string chunkB = Text(b.Resume(firstB.InstanceId, stateB).Bytes);

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
        Assert.Throws<ObjectDisposedException>(() => session.Advance());
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
