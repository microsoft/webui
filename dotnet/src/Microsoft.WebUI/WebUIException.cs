// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System;

namespace Microsoft.WebUI;

/// <summary>
/// Represents an error returned by the native WebUI library.
/// </summary>
public class WebUIException : Exception
{
    /// <summary>
    /// Initializes a new instance of the <see cref="WebUIException"/> class
    /// with the specified error message.
    /// </summary>
    /// <param name="message">The error message from the native library.</param>
    public WebUIException(string message) : base(message)
    {
    }
}
