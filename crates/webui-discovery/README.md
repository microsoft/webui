# microsoft-webui-discovery

External component discovery for the [WebUI](https://github.com/microsoft/webui) framework. Resolves web component definitions from npm packages and local paths for use during the build step.

`discover_source` uses WebUI's native package layout. Framework integrations
can implement `DiscoveryPlugin` and call `discover_source_with_plugin` to map a
different validated package layout into the same `DiscoveredComponent` runtime
contract. Built-in WebUI and FAST discovery plugins are provided.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
