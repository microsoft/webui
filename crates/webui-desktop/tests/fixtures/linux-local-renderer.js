// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.


// crates/webui-desktop/tests/fixtures/linux-local-renderer.ts
import { createNativeDesktopTransport } from "/_webui/ipc/local-runtime.js";

// crates/webui-desktop/tests/fixtures/native-ipc/generated/ts/ipc-runtime.ts
import { connect, validateMessage, validateValue, IpcError as IpcError2 } from "/_webui/ipc/local-runtime.js";

// crates/webui-desktop/tests/fixtures/native-ipc/generated/ts/application.ts
import { BoundaryReader, PayloadWriter, readDelimited, readUint32, readUint64, skipField } from "/_webui/ipc/local-runtime.js";
function createBaseGeneration() {
  return {
    value: 0n,
    phase: ""
  };
}
var Generation = {
  encode(message) {
    const writer = new PayloadWriter();
    if (message.value !== 0n) {
      writer.uint64(1, message.value);
    }
    if (message.phase !== "") {
      writer.string(2, message.phase);
    }
    return writer.finish();
  },
  decode(bytes) {
    const reader = new BoundaryReader(bytes);
    const message = createBaseGeneration();
    while (reader.pos < reader.len) {
      const [number, type] = reader.tag();
      switch (number) {
        case 1: {
          message.value = readUint64(reader);
          break;
        }
        case 2: {
          message.phase = new TextDecoder("utf-8", { fatal: true }).decode(readDelimited(reader));
          break;
        }
        default:
          skipField(reader, type);
          break;
      }
    }
    return message;
  }
};
function createBaseItem() {
  return {
    id: 0n,
    image: new Uint8Array(0),
    phase: ""
  };
}
var Item = {
  encode(message) {
    const writer = new PayloadWriter();
    if (message.id !== 0n) {
      writer.uint64(1, message.id);
    }
    if (message.image.byteLength !== 0) {
      writer.bytesField(2, message.image);
    }
    if (message.phase !== "") {
      writer.string(3, message.phase);
    }
    return writer.finish();
  },
  decode(bytes) {
    const reader = new BoundaryReader(bytes);
    const message = createBaseItem();
    while (reader.pos < reader.len) {
      const [number, type] = reader.tag();
      switch (number) {
        case 1: {
          message.id = readUint64(reader);
          break;
        }
        case 2: {
          message.image = new Uint8Array(readDelimited(reader));
          break;
        }
        case 3: {
          message.phase = new TextDecoder("utf-8", { fatal: true }).decode(readDelimited(reader));
          break;
        }
        default:
          skipField(reader, type);
          break;
      }
    }
    return message;
  }
};
function createBaseLabel() {
  return {
    text: ""
  };
}
var Label = {
  encode(message) {
    const writer = new PayloadWriter();
    if (message.text !== "") {
      writer.string(1, message.text);
    }
    return writer.finish();
  },
  decode(bytes) {
    const reader = new BoundaryReader(bytes);
    const message = createBaseLabel();
    while (reader.pos < reader.len) {
      const [number, type] = reader.tag();
      switch (number) {
        case 1: {
          message.text = new TextDecoder("utf-8", { fatal: true }).decode(readDelimited(reader));
          break;
        }
        default:
          skipField(reader, type);
          break;
      }
    }
    return message;
  }
};
function createBaseReport() {
  return {
    labels: 0,
    changes: 0,
    confirmations: 0,
    invalidRejected: false,
    cancelled: false,
    unsubscribed: false,
    voidCompleted: false,
    notificationAccepted: false,
    handlerRejected: false,
    visibility: "",
    userAgent: ""
  };
}
var Report = {
  encode(message) {
    const writer = new PayloadWriter();
    if (message.labels !== 0) {
      writer.uint32(1, message.labels);
    }
    if (message.changes !== 0) {
      writer.uint32(2, message.changes);
    }
    if (message.confirmations !== 0) {
      writer.uint32(3, message.confirmations);
    }
    if (message.invalidRejected) {
      writer.bool(4, message.invalidRejected);
    }
    if (message.cancelled) {
      writer.bool(5, message.cancelled);
    }
    if (message.unsubscribed) {
      writer.bool(6, message.unsubscribed);
    }
    if (message.voidCompleted) {
      writer.bool(7, message.voidCompleted);
    }
    if (message.notificationAccepted) {
      writer.bool(8, message.notificationAccepted);
    }
    if (message.handlerRejected) {
      writer.bool(9, message.handlerRejected);
    }
    if (message.visibility !== "") {
      writer.string(10, message.visibility);
    }
    if (message.userAgent !== "") {
      writer.string(11, message.userAgent);
    }
    return writer.finish();
  },
  decode(bytes) {
    const reader = new BoundaryReader(bytes);
    const message = createBaseReport();
    while (reader.pos < reader.len) {
      const [number, type] = reader.tag();
      switch (number) {
        case 1: {
          message.labels = readUint32(reader);
          break;
        }
        case 2: {
          message.changes = readUint32(reader);
          break;
        }
        case 3: {
          message.confirmations = readUint32(reader);
          break;
        }
        case 4: {
          message.invalidRejected = readUint32(reader) === 1;
          break;
        }
        case 5: {
          message.cancelled = readUint32(reader) === 1;
          break;
        }
        case 6: {
          message.unsubscribed = readUint32(reader) === 1;
          break;
        }
        case 7: {
          message.voidCompleted = readUint32(reader) === 1;
          break;
        }
        case 8: {
          message.notificationAccepted = readUint32(reader) === 1;
          break;
        }
        case 9: {
          message.handlerRejected = readUint32(reader) === 1;
          break;
        }
        case 10: {
          message.visibility = new TextDecoder("utf-8", { fatal: true }).decode(readDelimited(reader));
          break;
        }
        case 11: {
          message.userAgent = new TextDecoder("utf-8", { fatal: true }).decode(readDelimited(reader));
          break;
        }
        default:
          skipField(reader, type);
          break;
      }
    }
    return message;
  }
};

// crates/webui-desktop/tests/fixtures/native-ipc/generated/ts/ipc-runtime.ts
var schemaHash = "ab014d39db1e9865769dbb911c250bee14c6bc4cc9d93d67324c39099b13342d";
var messageShapes = [
  { fields: [
    { number: 1, name: "value", kind: "uint64", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 2, name: "phase", kind: "string", repeated: false, packed: false, optional: false, mapKey: false, map: false }
  ] },
  { fields: [
    { number: 1, name: "id", kind: "uint64", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 2, name: "image", kind: "bytes", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 3, name: "phase", kind: "string", repeated: false, packed: false, optional: false, mapKey: false, map: false }
  ] },
  { fields: [
    { number: 1, name: "text", kind: "string", repeated: false, packed: false, optional: false, mapKey: false, map: false }
  ] },
  { fields: [
    { number: 1, name: "labels", kind: "uint32", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 2, name: "changes", kind: "uint32", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 3, name: "confirmations", kind: "uint32", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 4, name: "invalidRejected", kind: "bool", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 5, name: "cancelled", kind: "bool", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 6, name: "unsubscribed", kind: "bool", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 7, name: "voidCompleted", kind: "bool", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 8, name: "notificationAccepted", kind: "bool", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 9, name: "handlerRejected", kind: "bool", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 10, name: "visibility", kind: "string", repeated: false, packed: false, optional: false, mapKey: false, map: false },
    { number: 11, name: "userAgent", kind: "string", repeated: false, packed: false, optional: false, mapKey: false, map: false }
  ] },
  { fields: [] }
];
var message_0 = {
  encode: (value) => Generation.encode(value),
  decode: (bytes) => Generation.decode(bytes),
  validate: (value, limits) => validateValue(value, 0, messageShapes, limits),
  validateBytes: (bytes, limits) => validateMessage(bytes, 0, messageShapes, limits)
};
var message_1 = {
  encode: (value) => Item.encode(value),
  decode: (bytes) => Item.decode(bytes),
  validate: (value, limits) => validateValue(value, 1, messageShapes, limits),
  validateBytes: (bytes, limits) => validateMessage(bytes, 1, messageShapes, limits)
};
var message_2 = {
  encode: (value) => Label.encode(value),
  decode: (bytes) => Label.decode(bytes),
  validate: (value, limits) => validateValue(value, 2, messageShapes, limits),
  validateBytes: (bytes, limits) => validateMessage(bytes, 2, messageShapes, limits)
};
var message_3 = {
  encode: (value) => Report.encode(value),
  decode: (bytes) => Report.decode(bytes),
  validate: (value, limits) => validateValue(value, 3, messageShapes, limits),
  validateBytes: (bytes, limits) => validateMessage(bytes, 3, messageShapes, limits)
};
var message_4 = {
  encode: () => new Uint8Array(0),
  decode: (bytes) => {
    if (bytes.byteLength !== 0) throw new IpcError2("invalid-payload", "Empty requires empty bytes");
  },
  validate: (value) => {
    if (value !== void 0) throw new IpcError2("invalid-payload", "Empty requires undefined");
  },
  validateBytes: (bytes, limits) => validateMessage(bytes, 4, messageShapes, limits)
};
var schema = { hello: { wireVersion: 3, contractName: "webui.test.native", contractMajor: 1, schemaHash }, methods: [
  { id: 1101, name: "fixture.native.Host.Save", receiver: "host", kind: "rpc", developmentOnly: false, request: message_1, response: message_4 },
  { id: 1102, name: "fixture.native.Host.Selected", receiver: "host", kind: "notification", developmentOnly: false, request: message_1 },
  { id: 1103, name: "fixture.native.Host.Release", receiver: "host", kind: "rpc", developmentOnly: false, request: message_4, response: message_4 },
  { id: 1104, name: "fixture.native.Host.Wait", receiver: "host", kind: "rpc", developmentOnly: false, request: message_1, response: message_4 },
  { id: 1105, name: "fixture.native.Host.Finish", receiver: "host", kind: "rpc", developmentOnly: false, request: message_3, response: message_4 },
  { id: 1106, name: "fixture.native.Host.Done", receiver: "host", kind: "notification", developmentOnly: false, request: message_4 },
  { id: 1107, name: "fixture.native.Host.CancellationObserved", receiver: "host", kind: "rpc", developmentOnly: false, request: message_4, response: message_4 },
  { id: 1108, name: "fixture.native.Host.LifecycleHold", receiver: "host", kind: "rpc", developmentOnly: false, request: message_1, response: message_4 },
  { id: 1109, name: "fixture.native.Host.LifecycleCheck", receiver: "host", kind: "rpc", developmentOnly: false, request: message_1, response: message_4 },
  { id: 1110, name: "fixture.native.Host.SessionGeneration", receiver: "host", kind: "rpc", developmentOnly: false, request: message_4, response: message_0 },
  { id: 1111, name: "fixture.native.Host.SameDocument", receiver: "host", kind: "rpc", developmentOnly: false, request: message_0, response: message_4 },
  { id: 1112, name: "fixture.native.Host.HistoryProbe", receiver: "host", kind: "rpc", developmentOnly: false, request: message_0, response: message_4 },
  { id: 1113, name: "fixture.native.Host.HistoryVerified", receiver: "host", kind: "rpc", developmentOnly: false, request: message_0, response: message_4 },
  { id: 2001, name: "fixture.native.Renderer.LabelFor", receiver: "renderer", kind: "rpc", developmentOnly: false, request: message_1, response: message_2 },
  { id: 2002, name: "fixture.native.Renderer.Changed", receiver: "renderer", kind: "notification", developmentOnly: false, request: message_1 }
] };
function handlersMap(handlers) {
  if (!handlers || typeof handlers !== "object") throw new IpcError2("invalid-payload", "Renderer handlers must be an object");
  if (typeof handlers.labelFor !== "function") throw new IpcError2("invalid-payload", "Missing renderer handler: labelFor");
  return /* @__PURE__ */ new Map([
    [2001, (value, context) => handlers.labelFor(value, context)]
  ]);
}
async function connectDesktop(transport, options) {
  const connection = await connect(transport, schema, { ...options?.renderer ? { handlers: handlersMap(options.renderer) } : {}, ...options?.onError ? { onError: options.onError } : {} });
  return { close: () => connection.close(), closed: connection.closed, setRenderer: (handlers) => connection.register(handlersMap(handlers)), host: {
    save: (request, options2) => connection.call(1101, request, options2),
    selected: (request) => connection.notify(1102, request),
    release: (request, options2) => connection.call(1103, request, options2),
    wait: (request, options2) => connection.call(1104, request, options2),
    finish: (request, options2) => connection.call(1105, request, options2),
    done: (request) => connection.notify(1106, request),
    cancellationObserved: (request, options2) => connection.call(1107, request, options2),
    lifecycleHold: (request, options2) => connection.call(1108, request, options2),
    lifecycleCheck: (request, options2) => connection.call(1109, request, options2),
    sessionGeneration: (request, options2) => connection.call(1110, request, options2),
    sameDocument: (request, options2) => connection.call(1111, request, options2),
    historyProbe: (request, options2) => connection.call(1112, request, options2),
    historyVerified: (request, options2) => connection.call(1113, request, options2)
  }, renderer: {
    onChanged: (callback) => connection.subscribe(2002, callback)
  } };
}

// crates/webui-desktop/tests/fixtures/linux-local-renderer.ts
var phase = "entry";
function assert(value, message) {
  if (!value) throw new Error(message);
}
async function run() {
  phase = "bootstrap";
  assert(window === window.top, "unexpected subframe module load");
  assert(
    typeof window.webkit?.messageHandlers?.webuiDesktopIpc === "undefined",
    "raw native handler leaked into main world"
  );
  assert(
    typeof window.webkit?.messageHandlers?.webuiDesktopIpcData === "undefined",
    "raw native data handler leaked into main world"
  );
  assert(window.__webuiDesktopIpcV2, "Linux top-frame bootstrap absent");
  const transport = createNativeDesktopTransport();
  let credentials;
  const start = transport.start.bind(transport);
  transport.start = async (...args) => {
    const session = await start(...args);
    assert(session.nativeCarrierVersion === 1, "Linux native carrier not admitted");
    credentials = session;
    return session;
  };
  const connection = await connectDesktop(transport);
  assert(credentials, "Linux IPC session credential absent");
  phase = "generated-rpc";
  const image = new Uint8Array(262144);
  for (let i = 0; i < image.length; i++) image[i] = (i * 31 + 7) % 256;
  await connection.host.save({ id: 18446744073709551615n, image, phase: location.pathname });
  connection.close();
  if (location.pathname === "/") {
    phase = "navigation";
    location.assign("/next");
    return;
  }
  assert(location.pathname === "/next", "unexpected Linux IPC document");
  phase = "report";
  const response = await fetch("/pass", { method: "POST", body: "" });
  assert(response.ok, `Linux IPC pass report failed: ${response.status}`);
}
run().catch(async (error) => {
  console.error("Linux native IPC fixture failed", error);
  const code = error && typeof error === "object" && typeof error.code === "string" && /^[a-z-]{1,32}$/.test(error.code) ? error.code : "unexpected";
  await fetch(`/fail?step=${phase}&code=${code}`, { method: "POST", body: "" });
});
