# microsoft-webui-discovery

External component discovery for the [WebUI](https://github.com/microsoft/webui) framework. Resolves web component definitions from npm packages and local paths for use during the build step.

`discover_source` uses WebUI's native package layout. Framework integrations
can implement `DiscoveryPlugin` and call `discover_source_with_plugin` to map a
different validated package layout into the same `DiscoveredComponent` runtime
contract. Built-in WebUI and FAST discovery plugins are provided.

Default WebUI discovery uses `<component-name>.html`: the filename is the
custom element name. npm packages are scanned beneath `components/` when
present, otherwise beneath the package root. Nested directories are supported;
their names do not determine component names. Matching `.css` files provide
styles and matching `.ts`/`.js` siblings mark authored components.

Template/style exports and Custom Elements Manifest names are not interpreted
by default discovery. Browser registrations are imported through package module
exports separately. `.spec.ts` files do not make a component scripted or require
a projection entry.

Named packages are resolved from each ancestor's `node_modules`, so a nearer
directory containing unrelated dependencies does not hide an installed catalog.
Bare scopes use the same lookup, selecting the nearest matching scope directory.
Scope searches skip unrelated packages but report failures in packages that
declare components.
The collection spellings `@scope/*` and `@scope/package/*` are equivalent to
`@scope` and `@scope/package`, respectively.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.
Plugin-specific behavior is documented in the [FAST discovery guide](src/plugin/fast/README.md).

## License

MIT — Copyright (c) Microsoft Corporation.
