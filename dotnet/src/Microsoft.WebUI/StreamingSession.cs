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
    /// Retain the boundary roots and compiled state projection until terminal.
    /// </summary>
    Updatable = 1,
}

/// <summary>
/// The JSON type of an evaluated runtime boundary key.
/// </summary>
public enum BoundaryKeyType
{
    /// <summary>The boundary declaration has no key.</summary>
    None = 0,

    /// <summary>The boundary key is a string.</summary>
    String = 1,

    /// <summary>The boundary key is a finite number.</summary>
    Number = 2,
}

/// <summary>
/// A typed runtime boundary key.
/// </summary>
public readonly struct BoundaryKey
{
    internal BoundaryKey(BoundaryKeyType type, string? stringValue, double? numberValue)
    {
        Type = type;
        StringValue = stringValue;
        NumberValue = numberValue;
    }

    /// <summary>
    /// Gets the key's JSON type.
    /// </summary>
    public BoundaryKeyType Type { get; }

    /// <summary>
    /// Gets the string value when <see cref="Type"/> is
    /// <see cref="BoundaryKeyType.String"/>; otherwise <c>null</c>.
    /// </summary>
    public string? StringValue { get; }

    /// <summary>
    /// Gets the numeric value when <see cref="Type"/> is
    /// <see cref="BoundaryKeyType.Number"/>; otherwise <c>null</c>.
    /// </summary>
    public double? NumberValue { get; }
}

/// <summary>
/// One runtime occurrence discovered while traversing a streaming response.
/// </summary>
public sealed class BoundaryDescriptor
{
    internal BoundaryDescriptor(
        uint instanceId,
        uint declarationId,
        string owner,
        string name,
        BoundaryKey key)
    {
        InstanceId = instanceId;
        DeclarationId = declarationId;
        Owner = owner;
        Name = name;
        Key = key;
    }

    /// <summary>
    /// Gets the gapless response-local occurrence ID passed to
    /// <see cref="StreamingSession.Resume"/> and <see cref="StreamingSession.Update"/>.
    /// </summary>
    public uint InstanceId { get; }

    /// <summary>
    /// Gets the stable compiler declaration ID.
    /// </summary>
    public uint DeclarationId { get; }

    /// <summary>
    /// Gets the entry or component template that owns the declaration.
    /// </summary>
    public string Owner { get; }

    /// <summary>
    /// Gets the authored boundary name.
    /// </summary>
    public string Name { get; }

    /// <summary>
    /// Gets the evaluated typed key, or <see cref="BoundaryKeyType.None"/>.
    /// </summary>
    public BoundaryKey Key { get; }
}

/// <summary>
/// The immutable managed result of one streaming start, resume, or advance operation.
/// </summary>
public sealed class StreamingStep
{
    internal StreamingStep(byte[] bytes, bool done, BoundaryDescriptor? boundary)
    {
        Bytes = bytes;
        Done = done;
        Boundary = boundary;
    }

    /// <summary>
    /// Gets the UTF-8 bytes produced by this semantic step.
    /// </summary>
    public byte[] Bytes { get; }

    /// <summary>
    /// Gets a value indicating whether terminal emission completed.
    /// </summary>
    public bool Done { get; }

    /// <summary>
    /// Gets the next runtime occurrence waiting for resume, or <c>null</c>.
    /// </summary>
    public BoundaryDescriptor? Boundary { get; }
}

/// <summary>
/// A progressive HTML response advanced one semantic step at a time.
/// </summary>
/// <remarks>
/// <para><see cref="Start"/>, <see cref="Resume"/>, and <see cref="Advance"/>
/// eagerly copy native step bytes and descriptors into managed objects. The
/// caller owns the transport and applies its own backpressure.</para>
/// <para>This type is <b>not</b> thread-safe. Drive one session from one request
/// at a time; independent sessions may run concurrently.</para>
/// <example>
/// <code>
/// using var session = handler.StreamResponse(protocol, "index.html", request.Path);
/// StreamingStep step = session.Start(shellJson);
/// while (true)
/// {
///     await response.Body.WriteAsync(step.Bytes);
///     if (step.Done) break;
///     step = step.Boundary is BoundaryDescriptor boundary
///         ? session.Resume(
///             boundary.InstanceId,
///             await LoadBoundaryState(boundary),
///             BoundaryMode.Final)
///         : session.Advance();
/// }
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
    /// Renders until the first runtime boundary occurrence or terminal completion.
    /// </summary>
    /// <param name="stateJson">JSON-encoded initial response state.</param>
    /// <returns>Produced bytes, completion state, and the next boundary descriptor.</returns>
    /// <exception cref="ObjectDisposedException">Thrown after disposal.</exception>
    /// <exception cref="WebUIException">Thrown when state parsing or rendering fails.</exception>
    public StreamingStep Start(string stateJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(stateJson);

        return TakeStep(
            NativeBindings.StartStreamingSession(_handle, stateJson),
            nameof(Start));
    }

    /// <summary>
    /// Commits the pending occurrence and returns immediately after its checkpoint.
    /// </summary>
    /// <param name="instanceId">The pending descriptor's response-local instance ID.</param>
    /// <param name="stateJson">JSON-encoded state used while rendering this occurrence.</param>
    /// <param name="mode">Whether this occurrence may receive later updates.</param>
    /// <returns>
    /// The occurrence bytes with no boundary descriptor and <see cref="StreamingStep.Done"/>
    /// set to <c>false</c>.
    /// </returns>
    /// <exception cref="ObjectDisposedException">Thrown after disposal.</exception>
    /// <exception cref="ArgumentOutOfRangeException">Thrown for an unknown boundary mode.</exception>
    /// <exception cref="WebUIException">Thrown for a stale ID or rendering failure.</exception>
    public StreamingStep Resume(
        uint instanceId,
        string stateJson,
        BoundaryMode mode = BoundaryMode.Final)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(stateJson);
        if (mode is not BoundaryMode.Final and not BoundaryMode.Updatable)
        {
            throw new ArgumentOutOfRangeException(nameof(mode));
        }

        return TakeStep(
            NativeBindings.ResumeStreamingSession(
                _handle,
                instanceId,
                stateJson,
                (uint)mode),
            nameof(Resume));
    }

    /// <summary>
    /// Advances through parent bytes to the next occurrence or terminal completion.
    /// </summary>
    /// <returns>Produced bytes, completion state, and the next boundary descriptor.</returns>
    /// <exception cref="ObjectDisposedException">Thrown after disposal.</exception>
    /// <exception cref="WebUIException">
    /// Thrown when called out of order or when rendering fails.
    /// </exception>
    public StreamingStep Advance()
    {
        ThrowIfDisposed();
        return TakeStep(
            NativeBindings.AdvanceStreamingSession(_handle),
            nameof(Advance));
    }

    /// <summary>
    /// Emits a projected patch for a committed updatable runtime occurrence.
    /// </summary>
    /// <param name="instanceId">A committed response-local instance ID.</param>
    /// <param name="patchJson">A JSON object containing changed values.</param>
    /// <returns>The UTF-8 update record bytes.</returns>
    /// <exception cref="ObjectDisposedException">Thrown after disposal.</exception>
    /// <exception cref="WebUIException">
    /// Thrown when the occurrence is not updatable or the patch is invalid.
    /// </exception>
    public byte[] Update(uint instanceId, string patchJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(patchJson);

        IntPtr chunk = NativeBindings.webui_streaming_session_update(
            _handle,
            instanceId,
            patchJson,
            out nuint length);
        return TakeChunk(chunk, length, nameof(Update));
    }

    /// <summary>
    /// Releases the native session. Safe to call before completion.
    /// </summary>
    public void Dispose()
    {
        if (Interlocked.CompareExchange(ref _disposed, 1, 0) != 0)
        {
            return;
        }

        _handle.Dispose();
    }

    private static StreamingStep TakeStep(
        NativeBindings.WebUIStreamingStepSafeHandle nativeStep,
        string operation)
    {
        using (nativeStep)
        {
            if (nativeStep.IsInvalid)
            {
                throw NativeFailure(operation);
            }

            IntPtr bytesPtr = NativeBindings.webui_streaming_step_bytes(
                nativeStep,
                out nuint bytesLength);
            byte[] bytes = NativeBindings.ReadBorrowedBytes(bytesPtr, bytesLength)
                ?? throw NativeFailure($"{operation} byte access");
            bool done = NativeBindings.webui_streaming_step_done(nativeStep);
            bool hasBoundary = NativeBindings.webui_streaming_step_has_boundary(nativeStep);
            BoundaryDescriptor? boundary = hasBoundary ? ReadBoundary(nativeStep) : null;
            return new StreamingStep(bytes, done, boundary);
        }
    }

    private static BoundaryDescriptor ReadBoundary(
        NativeBindings.WebUIStreamingStepSafeHandle nativeStep)
    {
        RequireNative(
            NativeBindings.webui_streaming_step_boundary_instance_id(
                nativeStep,
                out uint instanceId),
            "Read boundary instance ID");
        RequireNative(
            NativeBindings.webui_streaming_step_boundary_declaration_id(
                nativeStep,
                out uint declarationId),
            "Read boundary declaration ID");

        IntPtr ownerPtr = NativeBindings.webui_streaming_step_boundary_owner(
            nativeStep,
            out nuint ownerLength);
        string owner = NativeBindings.ReadBorrowedString(ownerPtr, ownerLength)
            ?? throw NativeFailure("Read boundary owner");
        IntPtr namePtr = NativeBindings.webui_streaming_step_boundary_name(
            nativeStep,
            out nuint nameLength);
        string name = NativeBindings.ReadBorrowedString(namePtr, nameLength)
            ?? throw NativeFailure("Read boundary name");

        RequireNative(
            NativeBindings.webui_streaming_step_boundary_key_type(
                nativeStep,
                out uint keyType),
            "Read boundary key type");
        BoundaryKey key = ReadBoundaryKey(nativeStep, keyType);
        return new BoundaryDescriptor(instanceId, declarationId, owner, name, key);
    }

    private static BoundaryKey ReadBoundaryKey(
        NativeBindings.WebUIStreamingStepSafeHandle nativeStep,
        uint keyType)
    {
        switch ((BoundaryKeyType)keyType)
        {
            case BoundaryKeyType.None:
                return default;
            case BoundaryKeyType.String:
                IntPtr valuePtr = NativeBindings.webui_streaming_step_boundary_key_string(
                    nativeStep,
                    out nuint valueLength);
                string value = NativeBindings.ReadBorrowedString(valuePtr, valueLength)
                    ?? throw NativeFailure("Read boundary string key");
                return new BoundaryKey(BoundaryKeyType.String, value, null);
            case BoundaryKeyType.Number:
                RequireNative(
                    NativeBindings.webui_streaming_step_boundary_key_number(
                        nativeStep,
                        out double number),
                    "Read boundary numeric key");
                return new BoundaryKey(BoundaryKeyType.Number, null, number);
            default:
                throw new WebUIException($"Native streaming step returned unknown key type {keyType}.");
        }
    }

    private static byte[] TakeChunk(IntPtr chunk, nuint length, string operation)
    {
        if (chunk == IntPtr.Zero)
        {
            throw NativeFailure(operation);
        }

        return NativeBindings.ReadAndFreeBytes(chunk, length)
            ?? throw NativeFailure($"{operation} byte copy");
    }

    private static void RequireNative(bool succeeded, string operation)
    {
        if (!succeeded)
        {
            throw NativeFailure(operation);
        }
    }

    private static WebUIException NativeFailure(string operation)
    {
        return new WebUIException(
            NativeBindings.GetLastError() ?? $"{operation} failed.");
    }

    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(
            _disposed != 0 || _handle.IsClosed || _handle.IsInvalid,
            this);
    }
}
