// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.Threading;

namespace Microsoft.WebUI;

/// <summary>
/// Whether a committed boundary may receive later state updates.
/// </summary>
public enum BoundaryMode
{
    /// <summary>
    /// Hydrate once and release every boundary-local reference after activation.
    /// </summary>
    Final = 0,

    /// <summary>
    /// Retain the boundary roots and compiled state projection until the
    /// terminal record, so <see cref="StreamingSession.Update"/> can patch them.
    /// </summary>
    Updatable = 1,
}

/// <summary>
/// A progressive HTML response written one chunk at a time.
/// </summary>
/// <remarks>
/// <para>Every method returns the bytes it produced instead of writing them, so
/// the caller owns the transport and applies its own backpressure — write each
/// chunk to <c>HttpResponse.Body</c> and <c>await FlushAsync()</c> when ready.
/// The session never blocks on a socket.</para>
/// <para>Ordering is enforced: the shell first, then each boundary exactly once
/// in declaration order, <see cref="Update"/> only after its boundary commits as
/// <see cref="BoundaryMode.Updatable"/>, and <see cref="Finish"/> last. A
/// violation throws before any byte is produced.</para>
/// <para>This type is <b>not</b> thread-safe. Drive one session from one request
/// at a time; independent sessions may run concurrently on the same handler and
/// protocol.</para>
/// <example>
/// <code>
/// using var session = handler.StreamResponse(protocol, "index.html", request.Path);
/// uint weather = session.Boundary("weather-shell");
///
/// await response.Body.WriteAsync(session.WriteShell(shellJson));
/// await response.Body.WriteAsync(
///     session.WriteBoundary(weather, weatherJson, BoundaryMode.Updatable));
/// await response.Body.FlushAsync();
///
/// await response.Body.WriteAsync(session.Update(weather, await forecastTask));
/// await response.Body.WriteAsync(session.Finish("{}"));
/// </code>
/// </example>
/// </remarks>
public sealed class StreamingSession : IDisposable
{
    private readonly NativeBindings.WebUIStreamingSessionSafeHandle _handle;
    private volatile int _disposed;

    internal StreamingSession(
        NativeBindings.WebUIHandlerSafeHandle handler,
        NativeBindings.WebUIProtocolSafeHandle protocol,
        string entryId,
        string requestPath)
    {
        _handle = NativeBindings.CreateStreamingSession(
            handler,
            protocol,
            entryId,
            requestPath);

        if (_handle.IsInvalid)
        {
            string error = NativeBindings.GetLastError()
                ?? "Failed to open a WebUI streaming session.";
            _handle.Dispose();
            throw new WebUIException(error);
        }
    }

    /// <summary>
    /// Gets the number of compile-time boundaries declared by this entry.
    /// </summary>
    /// <exception cref="ObjectDisposedException">Thrown when the session has been disposed.</exception>
    public uint BoundaryCount
    {
        get
        {
            ThrowIfDisposed();
            return NativeBindings.webui_streaming_session_boundary_count(_handle);
        }
    }

    /// <summary>
    /// Gets a value indicating whether the terminal record has been written.
    /// </summary>
    /// <exception cref="ObjectDisposedException">Thrown when the session has been disposed.</exception>
    public bool IsFinished
    {
        get
        {
            ThrowIfDisposed();
            return NativeBindings.webui_streaming_session_is_finished(_handle);
        }
    }

    /// <summary>
    /// Resolves an authored boundary name to a stable integer handle.
    /// </summary>
    /// <param name="name">The <c>name</c> attribute of a compiled <c>&lt;boundary&gt;</c>.</param>
    /// <returns>An integer handle to pass to the write methods.</returns>
    /// <remarks>
    /// Resolve once outside the write loop; reusing the handle costs nothing.
    /// </remarks>
    /// <exception cref="ObjectDisposedException">Thrown when the session has been disposed.</exception>
    /// <exception cref="WebUIException">
    /// Thrown when the entry declares no boundary called <paramref name="name"/>.
    /// The message lists the valid names and suggests the closest match.
    /// </exception>
    public uint Boundary(string name)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(name);

        if (!NativeBindings.webui_streaming_session_boundary(_handle, name, out uint boundary))
        {
            throw new WebUIException(
                NativeBindings.GetLastError() ?? $"Unknown streaming boundary '{name}'.");
        }

        return boundary;
    }

    /// <summary>
    /// Renders everything before the first boundary.
    /// </summary>
    /// <param name="stateJson">JSON-encoded state for the document prefix.</param>
    /// <returns>The UTF-8 bytes to write to the response.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the session has been disposed.</exception>
    /// <exception cref="WebUIException">Thrown when called out of order or when rendering fails.</exception>
    public byte[] WriteShell(string stateJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(stateJson);

        IntPtr chunk = NativeBindings.webui_streaming_session_write_shell(
            _handle,
            stateJson,
            out nuint length);
        return TakeChunk(chunk, length, "WriteShell");
    }

    /// <summary>
    /// Renders and commits the next boundary in declaration order.
    /// </summary>
    /// <param name="boundary">A handle from <see cref="Boundary"/>.</param>
    /// <param name="stateJson">JSON-encoded state for this boundary.</param>
    /// <param name="mode">
    /// Pass <see cref="BoundaryMode.Updatable"/> only for boundaries you intend
    /// to patch later; an updatable boundary retains its roots and projection
    /// until the terminal record.
    /// </param>
    /// <returns>The UTF-8 bytes to write to the response.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the session has been disposed.</exception>
    /// <exception cref="WebUIException">
    /// Thrown when boundaries are written out of declaration order or when rendering fails.
    /// </exception>
    public byte[] WriteBoundary(
        uint boundary,
        string stateJson,
        BoundaryMode mode = BoundaryMode.Final)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(stateJson);

        IntPtr chunk = NativeBindings.webui_streaming_session_write_boundary(
            _handle,
            boundary,
            stateJson,
            mode == BoundaryMode.Updatable,
            out nuint length);
        return TakeChunk(chunk, length, "WriteBoundary");
    }

    /// <summary>
    /// Pushes a projected state patch to a committed updatable boundary.
    /// </summary>
    /// <param name="boundary">A handle from <see cref="Boundary"/>.</param>
    /// <param name="stateJson">JSON object holding the changed values.</param>
    /// <returns>The UTF-8 bytes to write to the response.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the session has been disposed.</exception>
    /// <exception cref="WebUIException">
    /// Thrown when the boundary has not committed, was committed as
    /// <see cref="BoundaryMode.Final"/>, or when the state is not a JSON object.
    /// </exception>
    public byte[] Update(uint boundary, string stateJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(stateJson);

        IntPtr chunk = NativeBindings.webui_streaming_session_update(
            _handle,
            boundary,
            stateJson,
            out nuint length);
        return TakeChunk(chunk, length, "Update");
    }

    /// <summary>
    /// Renders the document tail and emits the terminal record.
    /// </summary>
    /// <param name="stateJson">JSON-encoded state for the document tail.</param>
    /// <returns>The UTF-8 bytes to write to the response.</returns>
    /// <remarks>Every later call throws. Dispose the session afterwards.</remarks>
    /// <exception cref="ObjectDisposedException">Thrown when the session has been disposed.</exception>
    /// <exception cref="WebUIException">
    /// Thrown when boundaries remain uncommitted or when rendering fails.
    /// </exception>
    public byte[] Finish(string stateJson = "{}")
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(stateJson);

        IntPtr chunk = NativeBindings.webui_streaming_session_finish(
            _handle,
            stateJson,
            out nuint length);
        return TakeChunk(chunk, length, "Finish");
    }

    /// <summary>
    /// Releases the native session. Safe to call on an unfinished session; any
    /// buffered bytes are dropped.
    /// </summary>
    public void Dispose()
    {
        if (Interlocked.CompareExchange(ref _disposed, 1, 0) != 0)
        {
            return;
        }

        _handle.Dispose();
    }

    private static byte[] TakeChunk(IntPtr chunk, nuint length, string operation)
    {
        if (chunk == IntPtr.Zero)
        {
            throw new WebUIException(
                NativeBindings.GetLastError() ?? $"{operation} failed.");
        }

        return NativeBindings.ReadAndFreeBytes(chunk, length)!;
    }

    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(
            _disposed != 0 || _handle.IsClosed || _handle.IsInvalid,
            this);
    }
}
