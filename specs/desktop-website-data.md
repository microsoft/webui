# Desktop website data boundary

The `local-server` builder's persistent website-data request is optional.
With no request, macOS uses `nonPersistentDataStore` regardless of `app_id`.
The bundled/source macOS frame remains ephemeral. A local-server request
must validate a path-safe `app_id` and prove that it equals
`NSBundle.mainBundle.bundleIdentifier` of the running packaged `.app`
executable before selecting the public `WKWebsiteDataStore.defaultDataStore`.
Recheck at startup before opening a WebView. Reject missing identity,
unbundled executables, and mismatches; never fall back to a process-name
store or quietly open an ephemeral view after a persistence request.

On macOS 13 the public persistent store is the one application-wide default,
not an identifier-named or per-window store. Opted-in windows and their
allowed frame origins share website data subject to standard origin and cookie
rules; an unopted-in window retains its ephemeral configuration.
Neither an SDK cookie setter nor direct WebKit directory access belongs to
this boundary. Windows continues to select its existing app-ID-owned
WebView2 directory; anonymous profiles remain temporary when not opting in.
Other platforms reject an explicit persistence request. This storage choice
does not authenticate HTTP, import old browser profiles, or widen native IPC
admission.
