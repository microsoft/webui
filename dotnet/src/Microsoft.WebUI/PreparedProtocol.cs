// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.Threading;

namespace Microsoft.WebUI;

/// <summary>
/// Owns a decoded WebUI protocol for repeated rendering.
/// </summary>
/// <remarks>
/// Create one instance when the server loads <c>protocol.bin</c> and reuse it
/// across requests. This type is thread-safe.
/// </remarks>
public sealed class PreparedProtocol : IDisposable
{
    private readonly NativeBindings.WebUIProtocolSafeHandle _handle;
    private volatile int _disposed;

    /// <summary>
    /// Decodes and prepares a compiled WebUI protocol.
    /// </summary>
    /// <param name="protocol">Pre-compiled protobuf protocol bytes.</param>
    /// <exception cref="ArgumentNullException">
    /// Thrown when <paramref name="protocol"/> is <see langword="null"/>.
    /// </exception>
    /// <exception cref="WebUIException">Thrown when the protocol is invalid.</exception>
    public PreparedProtocol(byte[] protocol)
    {
        ArgumentNullException.ThrowIfNull(protocol);

        _handle = NativeBindings.CreateProtocol(protocol);
        if (_handle.IsInvalid)
        {
            string error = NativeBindings.GetLastError() ?? "Failed to prepare WebUI protocol.";
            _handle.Dispose();
            throw new WebUIException(error);
        }
    }

    internal NativeBindings.WebUIProtocolSafeHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    /// <summary>
    /// Releases the decoded protocol and its reusable indices.
    /// </summary>
    public void Dispose()
    {
        if (Interlocked.CompareExchange(ref _disposed, 1, 0) != 0)
        {
            return;
        }

        _handle.Dispose();
    }

    internal void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(
            _disposed != 0 || _handle.IsClosed || _handle.IsInvalid,
            this);
    }
}
