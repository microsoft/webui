// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { IpcError } from './errors.js';
import { BoundaryReader, readDelimited, readUint32, readUint64, skipField } from './boundary.js';
import type { IpcLimits } from './limits.js';

/** Descriptor metadata emitted by the application generator, not a proto parser. */
export interface MessageShape { readonly fields: readonly FieldShape[] }
export interface FieldShape {
  readonly number: number;
  readonly name: string;
  readonly kind: string;
  readonly message?: number;
  readonly repeated: boolean;
  readonly packed?: boolean;
  readonly optional: boolean;
  readonly oneof?: string;
  readonly mapKey: boolean;
  readonly map?: boolean;
}
const signed64 = new Set(['int64', 'sint64', 'sfixed64']);
const unsigned64 = new Set(['uint64', 'fixed64']);
const signed32 = new Set(['int32', 'sint32', 'sfixed32', 'enum']);
const unsigned32 = new Set(['uint32', 'fixed32']);
const fixed32 = new Set(['fixed32', 'sfixed32', 'float']);
const fixed64 = new Set(['fixed64', 'sfixed64', 'double']);
const utf8 = new TextEncoder();
const utf8Decoder = new TextDecoder('utf-8', { fatal: true });
function invalid(): never { throw new IpcError('invalid-payload'); }
function shapeAt(messages: readonly MessageShape[], index: number | undefined): MessageShape {
  const shape = index === undefined ? undefined : messages[index];
  if (!shape) return invalid();
  return shape;
}

function scalar(value: unknown, kind: string): void {
  if (signed64.has(kind) || unsigned64.has(kind)) {
    if (typeof value !== 'bigint') invalid();
    const signed = signed64.has(kind);
    if (value < (signed ? -0x8000000000000000n : 0n) ||
        value > (signed ? 0x7fffffffffffffffn : 0xffffffffffffffffn)) invalid();
  } else if (signed32.has(kind) || unsigned32.has(kind)) {
    if (typeof value !== 'number' || !Number.isInteger(value)) invalid();
    const signed = signed32.has(kind);
    if (value < (signed ? -2147483648 : 0) || value > (signed ? 2147483647 : 4294967295)) invalid();
  } else if (kind === 'double' || kind === 'float') {
    if (typeof value !== 'number') invalid();
  } else if (kind === 'bytes') {
    if (!(value instanceof Uint8Array)) invalid();
  } else if (kind === 'bool') {
    if (typeof value !== 'boolean') invalid();
  } else if (kind === 'string') {
    if (typeof value !== 'string') invalid();
  } else invalid();
}

/** Validate JS values before a codec can coerce or truncate them. */
export function validateValue(value: unknown, root: number, messages: readonly MessageShape[], limits: IpcLimits): void {
  const stack: { value: unknown; shape: MessageShape; depth: number }[] = [{ value, shape: shapeAt(messages, root), depth: 1 }];
  let entries = 0;
  let scalarBytes = 0;
  const add = (item: unknown, field: FieldShape, depth: number) => {
    if (field.kind === 'message') {
      if (depth >= limits.maxSchemaDepth) invalid();
      stack.push({ value: item, shape: shapeAt(messages, field.message), depth: depth + 1 });
    } else {
      scalar(item, field.kind);
      if (typeof item === 'string') {
        if (item.length > limits.maxFrameBytes) throw new IpcError('payload-too-large');
        scalarBytes += utf8.encode(item).byteLength;
      }
      else if (item instanceof Uint8Array) scalarBytes += item.byteLength;
      if (scalarBytes > limits.maxFrameBytes) throw new IpcError('payload-too-large');
    }
  };
  while (stack.length) {
    const current = stack.pop()!;
    if (!current.value || typeof current.value !== 'object' || Array.isArray(current.value)) invalid();
    const object = current.value as Record<string, unknown>;
    const oneofs = new Set<string>();
    for (const field of current.shape.fields) {
      let item = object[field.name];
      if (field.oneof) {
        if (object[field.name] !== undefined) invalid();
        const union = object[field.oneof] as { $case?: unknown; value?: unknown } | undefined;
        if (union === undefined) continue;
        if (typeof union !== 'object' || union === null) invalid();
        if (!current.shape.fields.some(f => f.oneof === field.oneof && f.name === union.$case)) invalid();
        if (union.$case !== field.name) continue;
        if (oneofs.has(field.oneof) || object[field.name] !== undefined) invalid();
        oneofs.add(field.oneof);
        item = union.value;
      } else if (item === undefined && field.optional) continue;
      if (field.map) {
        if (!(item instanceof Map)) invalid();
        if (current.depth >= limits.maxSchemaDepth) invalid();
        if (entries + item.size > limits.maxCollectionEntriesPerMessage) invalid();
        entries += item.size;
        const entry = shapeAt(messages, field.message);
        const keyField = entry.fields.find(f => f.mapKey || f.number === 1);
        const valueField = entry.fields.find(f => f.number === 2);
        if (!keyField || !valueField) invalid();
        for (const [key, value] of item) {
          scalar(key, keyField.kind);
          if (typeof key === 'string') {
            if (key.length > limits.maxFrameBytes) throw new IpcError('payload-too-large');
            scalarBytes += utf8.encode(key).byteLength;
            if (scalarBytes > limits.maxFrameBytes) throw new IpcError('payload-too-large');
          }
          add(value, valueField, current.depth + 1);
        }
      } else if (field.repeated) {
        if (!Array.isArray(item)) invalid();
        if (entries + item.length > limits.maxCollectionEntriesPerMessage) invalid();
        entries += item.length;
        for (const element of item) add(element, field, current.depth);
      } else add(item, field, current.depth);
    }
  }
}

function wireType(kind: string): number {
  return fixed64.has(kind) ? 1 : fixed32.has(kind) ? 5
    : kind === 'string' || kind === 'bytes' || kind === 'message' ? 2 : 0;
}

function readScalar(reader: BoundaryReader, field: FieldShape): void {
  switch (field.kind) {
    case 'string': utf8Decoder.decode(readDelimited(reader)); return;
    case 'bytes': readDelimited(reader); return;
    case 'bool': {
      const value = readUint32(reader);
      if (value !== 0 && value !== 1) invalid();
      return;
    }
    case 'int64': case 'uint64': case 'sint64': readUint64(reader); return;
    case 'fixed64': reader.fixed64(); return;
    case 'sfixed64': reader.sfixed64(); return;
    case 'int32': case 'enum': {
      const raw = readUint64(reader);
      if (raw > 0x7fffffffn && raw < 0xffffffff80000000n) invalid();
      return;
    }
    case 'uint32': case 'sint32': readUint32(reader); return;
    case 'fixed32': reader.fixed32(); return;
    case 'sfixed32': reader.sfixed32(); return;
    case 'float': reader.float(); return;
    case 'double': reader.double(); return;
    default: invalid();
  }
}

/** Iterative bounded wire guard before generated payload decoding. */
export function validateMessage(bytes: Uint8Array, root: number, messages: readonly MessageShape[], limits: IpcLimits): void {
  if (bytes.byteLength > limits.maxFrameBytes) throw new IpcError('payload-too-large');
  const stack = [{ bytes, shape: shapeAt(messages, root), depth: 1 }];
  let entries = 0;
  try {
    while (stack.length) {
      const current = stack.pop()!;
      const reader = new BoundaryReader(current.bytes);
      const oneofs = new Set<string>();
      while (reader.pos < reader.len) {
        const [number, type] = reader.tag();
        if (type === 3 || type === 4) invalid();
        const field = current.shape.fields.find(f => f.number === number);
        if (!field) { skipField(reader, type); continue; }
        if (field.oneof) {
          if (oneofs.has(field.oneof)) invalid();
          oneofs.add(field.oneof);
        }
        if (field.kind === 'message') {
          if (field.repeated && ++entries > limits.maxCollectionEntriesPerMessage) invalid();
          if (type !== 2 || current.depth >= limits.maxSchemaDepth) invalid();
          stack.push({ bytes: readDelimited(reader), shape: shapeAt(messages, field.message), depth: current.depth + 1 });
        } else if (type === 2 && field.repeated && field.packed === true && wireType(field.kind) !== 2) {
          const packed = new BoundaryReader(readDelimited(reader));
          while (packed.pos < packed.len) {
            if (++entries > limits.maxCollectionEntriesPerMessage) invalid();
            readScalar(packed, field);
          }
        } else {
          if (field.repeated && ++entries > limits.maxCollectionEntriesPerMessage) invalid();
          if (type !== wireType(field.kind)) invalid();
          readScalar(reader, field);
        }
      }
    }
  } catch (error) {
    if (error instanceof IpcError) throw error;
    invalid();
  }
}
