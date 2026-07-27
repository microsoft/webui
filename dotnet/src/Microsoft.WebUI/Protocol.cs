// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;
using System.Text.Json;
using System.Threading;

namespace Microsoft.WebUI;

/// <summary>
/// Owns a decoded WebUI protocol for repeated rendering.
/// </summary>
/// <remarks>
/// Create one instance when the server loads <c>protocol.bin</c> and reuse it
/// across requests. This type is thread-safe.
/// </remarks>
public sealed class Protocol : IDisposable
{
    private readonly NativeBindings.WebUIProtocolSafeHandle _handle;
    private volatile int _disposed;

    /// <summary>
    /// Decodes and indexes a compiled WebUI protocol.
    /// </summary>
    /// <param name="protocol">Pre-compiled protobuf protocol bytes.</param>
    /// <exception cref="ArgumentNullException">
    /// Thrown when <paramref name="protocol"/> is <see langword="null"/>.
    /// </exception>
    /// <exception cref="WebUIException">Thrown when the protocol is invalid.</exception>
    public Protocol(byte[] protocol)
    {
        ArgumentNullException.ThrowIfNull(protocol);

        _handle = NativeBindings.CreateProtocol(protocol);
        if (_handle.IsInvalid)
        {
            string error = NativeBindings.GetLastError() ?? "Failed to load WebUI protocol.";
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
    /// Produces a complete JSON partial response for client-side navigation.
    /// </summary>
    /// <param name="stateJson">JSON-encoded application state.</param>
    /// <param name="entryId">The persistent entry identifier.</param>
    /// <param name="requestPath">The current route path.</param>
    /// <param name="inventoryHex">Hex-encoded component inventory.</param>
    /// <returns>A JSON string containing state, templates, inventory, path, and chain.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the protocol has been disposed.</exception>
    /// <exception cref="WebUIException">Thrown when the operation fails.</exception>
    public string RenderPartial(
        string stateJson,
        string entryId,
        string requestPath,
        string inventoryHex)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(stateJson);
        ArgumentNullException.ThrowIfNull(entryId);
        ArgumentNullException.ThrowIfNull(requestPath);
        ArgumentNullException.ThrowIfNull(inventoryHex);

        IntPtr resultPtr = NativeBindings.webui_protocol_render_partial(
            _handle,
            stateJson,
            entryId,
            requestPath,
            inventoryHex);

        if (resultPtr == IntPtr.Zero)
        {
            string error = NativeBindings.GetLastError() ?? "RenderPartial failed.";
            throw new WebUIException(error);
        }

        return NativeBindings.ReadAndFreeString(resultPtr)!;
    }

    /// <summary>
    /// Returns component template payloads for the requested tags as JSON.
    /// </summary>
    /// <param name="componentTags">Component tag names to load.</param>
    /// <param name="inventoryHex">Hex-encoded component inventory.</param>
    /// <returns>A JSON string containing templates, styles, functions, and updated inventory.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the protocol has been disposed.</exception>
    /// <exception cref="WebUIException">Thrown when the operation fails.</exception>
    public string RenderComponentTemplates(
        string[] componentTags,
        string inventoryHex)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(componentTags);
        ArgumentNullException.ThrowIfNull(inventoryHex);

        string componentTagsJson = JsonSerializer.Serialize(componentTags);
        IntPtr resultPtr = NativeBindings.webui_protocol_render_component_templates(
            _handle,
            componentTagsJson,
            inventoryHex);

        if (resultPtr == IntPtr.Zero)
        {
            string error = NativeBindings.GetLastError() ?? "RenderComponentTemplates failed.";
            throw new WebUIException(error);
        }

        return NativeBindings.ReadAndFreeString(resultPtr)!;
    }

    /// <summary>
    /// Returns CSS token names in build order.
    /// </summary>
    /// <returns>CSS token names in build order, preserving duplicates.</returns>
    /// <exception cref="ObjectDisposedException">Thrown when the protocol has been disposed.</exception>
    /// <exception cref="WebUIException">Thrown when the operation fails.</exception>
    public string[] Tokens()
    {
        ThrowIfDisposed();
        IntPtr resultPtr = NativeBindings.webui_protocol_tokens(_handle);

        if (resultPtr == IntPtr.Zero)
        {
            string error = NativeBindings.GetLastError() ?? "Tokens failed.";
            throw new WebUIException(error);
        }

        string tokens = NativeBindings.ReadAndFreeString(resultPtr)!;
        return tokens.Length == 0
            ? Array.Empty<string>()
            : tokens.Split('\n');
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
