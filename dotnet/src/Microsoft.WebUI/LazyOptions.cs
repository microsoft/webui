// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

using System.Text.Json.Serialization;

namespace Microsoft.WebUI;

/// <summary>Client availability for a named lazy template body.</summary>
public sealed record LazyOptions
{
    /// <summary>Loaded template-resource bits.</summary>
    [JsonPropertyName("inventory")]
    public string Inventory { get; init; } = "";

    /// <summary>Loaded stylesheet-definition bits.</summary>
    [JsonPropertyName("styleInventory")]
    public string StyleInventory { get; init; } = "";

    /// <summary>The artifact identity associated with these inventories.</summary>
    [JsonPropertyName("buildId")]
    public string BuildId { get; init; } = "";
}

/// <summary>A resource record and acknowledgement for host-owned two-phase delivery.</summary>
/// <param name="Json">Resource JSON to flush before awaiting application state.</param>
/// <param name="Options">Availability to pass to the terminal state call.</param>
public sealed record PreparedLazyResources(string Json, LazyOptions Options);
