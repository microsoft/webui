# microsoft-webui-node-build

Build-only native addon behind `@microsoft/webui/build`. Exposes `build`,
`prepareBuild`, and `bundleClient` through napi-rs. The separate
`microsoft-webui-node` addon owns protocol loading and rendering; compiler
results cross that boundary as ordinary owned protocol buffers, not native
handles.

The library name is `webui_node_build`. Cargo emits
`libwebui_node_build.dylib`, `webui_node_build.dll`, or
`libwebui_node_build.so` for the package's platform-specific native loader.

## License

MIT - Copyright (c) Microsoft Corporation.
