// The trait-independent half of a web-rpc Javascript endpoint. This file is embedded verbatim
// at the top of every module rendered by `web_rpc::js::endpoint!`, ahead of the generated
// schemas, method tables and class. It has no imports and no top-level side effects.
//
// The generated part is data: every type reachable from the traits becomes a schema value
// (see `Codec`), and every method becomes an entry in a method table. This half interprets
// them.

const MASK_64 = 0xffffffffffffffffn;
const MASK_128 = 0xffffffffffffffffffffffffffffffffn;

// `MessageHeader`, by declaration order.
const REQUEST = 0;
const ABORT = 1;
const RESPONSE = 2;
const STREAM_ITEM = 3;
const STREAM_END = 4;

// `WireArg`, by declaration order.
const WIRE_JS = 0;
const WIRE_BYTES = 1;
const WIRE_NONE = 2;
const WIRE_SOME = 3;
const WIRE_OK = 4;
const WIRE_ERR = 5;

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: true });

/** A growable postcard writer. */
export class Writer {
  constructor() {
    this.buffer = new Uint8Array(64);
    this.length = 0;
    this.view = new DataView(this.buffer.buffer);
  }

  reserve(extra) {
    if (this.length + extra <= this.buffer.length) return;
    let capacity = this.buffer.length * 2;
    while (capacity < this.length + extra) capacity *= 2;
    const grown = new Uint8Array(capacity);
    grown.set(this.buffer.subarray(0, this.length));
    this.buffer = grown;
    this.view = new DataView(this.buffer.buffer);
  }

  /** The bytes written so far, in a buffer of exactly that length. */
  take() {
    return this.buffer.slice(0, this.length);
  }

  u8(value) {
    this.reserve(1);
    this.buffer[this.length++] = value & 0xff;
  }

  i8(value) {
    this.u8(value < 0 ? value + 256 : value);
  }

  bool(value) {
    this.u8(value ? 1 : 0);
  }

  varint32(value) {
    let rest = value >>> 0;
    while (rest >= 0x80) {
      this.u8((rest & 0x7f) | 0x80);
      rest = rest >>> 7;
    }
    this.u8(rest);
  }

  zigzag32(value) {
    this.varint32(((value << 1) ^ (value >> 31)) >>> 0);
  }

  varintBig(value, mask) {
    let rest = BigInt(value) & mask;
    while (rest >= 0x80n) {
      this.u8(Number(rest & 0x7fn) | 0x80);
      rest >>= 7n;
    }
    this.u8(Number(rest));
  }

  varint64(value) {
    this.varintBig(value, MASK_64);
  }

  zigzag64(value) {
    const wide = BigInt(value);
    this.varintBig((wide << 1n) ^ (wide >> 63n), MASK_64);
  }

  varint128(value) {
    this.varintBig(value, MASK_128);
  }

  zigzag128(value) {
    const wide = BigInt(value);
    this.varintBig((wide << 1n) ^ (wide >> 127n), MASK_128);
  }

  f32(value) {
    this.reserve(4);
    this.view.setFloat32(this.length, value, true);
    this.length += 4;
  }

  f64(value) {
    this.reserve(8);
    this.view.setFloat64(this.length, value, true);
    this.length += 8;
  }

  /** A length-prefixed run of bytes, from a `Uint8Array`, any other view, or an `ArrayBuffer`. */
  bytes(value) {
    const view = ArrayBuffer.isView(value)
      ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
      : new Uint8Array(value);
    this.varint32(view.length);
    this.reserve(view.length);
    this.buffer.set(view, this.length);
    this.length += view.length;
  }

  string(value) {
    this.bytes(textEncoder.encode(value));
  }
}

/** A postcard reader over a byte view. */
export class Reader {
  constructor(view) {
    this.buffer = view;
    this.position = 0;
    this.view = new DataView(view.buffer, view.byteOffset, view.byteLength);
  }

  u8() {
    if (this.position >= this.buffer.length) {
      throw new RangeError("web-rpc: message ended mid-value");
    }
    return this.buffer[this.position++];
  }

  i8() {
    const byte = this.u8();
    return byte > 127 ? byte - 256 : byte;
  }

  bool() {
    return this.u8() !== 0;
  }

  varint32() {
    let value = 0;
    let shift = 0;
    for (;;) {
      const byte = this.u8();
      value += (byte & 0x7f) * 2 ** shift;
      if ((byte & 0x80) === 0) break;
      shift += 7;
    }
    return value >>> 0;
  }

  zigzag32() {
    const value = this.varint32();
    return (value >>> 1) ^ -(value & 1);
  }

  varintBig() {
    let value = 0n;
    let shift = 0n;
    for (;;) {
      const byte = this.u8();
      value |= BigInt(byte & 0x7f) << shift;
      if ((byte & 0x80) === 0) break;
      shift += 7n;
    }
    return value;
  }

  varint64() {
    return this.varintBig() & MASK_64;
  }

  zigzag64() {
    const value = this.varintBig() & MASK_64;
    return BigInt.asIntN(64, (value >> 1n) ^ -(value & 1n));
  }

  varint128() {
    return this.varintBig() & MASK_128;
  }

  zigzag128() {
    const value = this.varintBig() & MASK_128;
    return BigInt.asIntN(128, (value >> 1n) ^ -(value & 1n));
  }

  f32() {
    const value = this.view.getFloat32(this.position, true);
    this.position += 4;
    return value;
  }

  f64() {
    const value = this.view.getFloat64(this.position, true);
    this.position += 8;
    return value;
  }

  bytes() {
    const length = this.varint32();
    if (this.position + length > this.buffer.length) {
      throw new RangeError("web-rpc: message ended mid-value");
    }
    const slice = this.buffer.subarray(this.position, this.position + length);
    this.position += length;
    return slice;
  }

  string() {
    return textDecoder.decode(this.bytes());
  }

  /** A reader over the next length-prefixed run of bytes. */
  sub() {
    return new Reader(this.bytes());
  }
}

/** The `Writer` and `Reader` method that carries each primitive schema. */
const PRIMITIVES = {
  bool: "bool",
  u8: "u8",
  i8: "i8",
  u16: "varint32",
  u32: "varint32",
  i16: "zigzag32",
  i32: "zigzag32",
  u64: "varint64",
  i64: "zigzag64",
  u128: "varint128",
  i128: "zigzag128",
  f32: "f32",
  f64: "f64",
  char: "string",
  string: "string",
  bytes: "bytes",
};

function isUnitOnly(variants) {
  return variants.every(([, variant]) => variant === "unit");
}

function expectTag(actual, expected, what) {
  if (actual !== expected) {
    throw new TypeError(`web-rpc: expected ${what}, found wire tag ${actual}`);
  }
}

/**
 * Postcard and `WireArg` codecs driven by schema values.
 *
 * A schema is either a string naming a primitive, or an object with a `kind`:
 *
 *   "bool" "u8" "i8" "u16" "i16" "u32" "i32" "u64" "i64" "u128" "i128"
 *   "f32" "f64" "char" "string" "bytes" "unit"
 *   { kind: "option", inner }      `undefined` or `null` is `None`
 *   { kind: "seq", inner }         an array, or a `Uint8Array` when `inner` is "u8"
 *   { kind: "tuple", items }       an array of fixed length
 *   { kind: "map", key, value }    a `Map`
 *   { kind: "struct", fields }     `fields` is `[[name, schema], ...]`
 *   { kind: "enum", variants }     `variants` is `[[name, variant], ...]`
 *   { kind: "ref", name }          a schema declared under `name` in the table given to the
 *                                  constructor
 *
 * A variant is "unit", `{ kind: "newtype", inner }`, or a tuple or struct schema. An enum
 * whose variants are all unit is a string; any other enum is `{ tag: name, value }`, or
 * `{ tag: name, ...fields }` for a struct variant.
 *
 * A wire description says how one argument or return value crosses the channel:
 *
 *   { kind: "js", transfer }       a Javascript value in the message, transferred if asked
 *   { kind: "postcard", schema }   postcard bytes behind a tag and a length
 *   { kind: "inline", schema }     postcard bytes with no tag and no length
 *   { kind: "option", inner }
 *   { kind: "result", ok, err }    `{ tag: "Ok", value }` or `{ tag: "Err", value }`
 */
export class Codec {
  constructor(schemas) {
    this.schemas = schemas;
  }

  encode(schema, writer, value) {
    if (typeof schema === "string") {
      if (schema === "unit") return;
      const method = PRIMITIVES[schema];
      if (!method) throw new TypeError(`web-rpc: unknown schema ${schema}`);
      writer[method](value);
      return;
    }
    switch (schema.kind) {
      case "ref":
        this.encode(this.schemas[schema.name], writer, value);
        return;
      case "option":
        if (value === undefined || value === null) {
          writer.u8(0);
        } else {
          writer.u8(1);
          this.encode(schema.inner, writer, value);
        }
        return;
      case "seq":
        if (schema.inner === "u8") {
          writer.bytes(value);
          return;
        }
        writer.varint32(value.length);
        for (const item of value) this.encode(schema.inner, writer, item);
        return;
      case "tuple":
        schema.items.forEach((item, index) => this.encode(item, writer, value[index]));
        return;
      case "map":
        writer.varint32(value.size);
        for (const [key, item] of value) {
          this.encode(schema.key, writer, key);
          this.encode(schema.value, writer, item);
        }
        return;
      case "struct":
        for (const [name, field] of schema.fields) this.encode(field, writer, value[name]);
        return;
      case "enum":
        this.encodeEnum(schema, writer, value);
        return;
      default:
        throw new TypeError(`web-rpc: unknown schema kind ${schema.kind}`);
    }
  }

  encodeEnum(schema, writer, value) {
    const tag = isUnitOnly(schema.variants) ? value : value.tag;
    const index = schema.variants.findIndex(([name]) => name === tag);
    if (index < 0) throw new TypeError(`web-rpc: unknown variant ${tag}`);
    writer.varint32(index);
    const variant = schema.variants[index][1];
    if (variant === "unit") return;
    switch (variant.kind) {
      case "newtype":
        this.encode(variant.inner, writer, value.value);
        return;
      case "tuple":
        this.encode(variant, writer, value.value);
        return;
      case "struct":
        this.encode(variant, writer, value);
        return;
      default:
        throw new TypeError(`web-rpc: unknown variant kind ${variant.kind}`);
    }
  }

  decode(schema, reader) {
    if (typeof schema === "string") {
      if (schema === "unit") return undefined;
      const method = PRIMITIVES[schema];
      if (!method) throw new TypeError(`web-rpc: unknown schema ${schema}`);
      return reader[method]();
    }
    switch (schema.kind) {
      case "ref":
        return this.decode(this.schemas[schema.name], reader);
      case "option":
        return reader.u8() === 0 ? undefined : this.decode(schema.inner, reader);
      case "seq": {
        if (schema.inner === "u8") return reader.bytes();
        const length = reader.varint32();
        const items = [];
        for (let index = 0; index < length; index += 1) {
          items.push(this.decode(schema.inner, reader));
        }
        return items;
      }
      case "tuple":
        return schema.items.map((item) => this.decode(item, reader));
      case "map": {
        const length = reader.varint32();
        const map = new Map();
        for (let index = 0; index < length; index += 1) {
          const key = this.decode(schema.key, reader);
          map.set(key, this.decode(schema.value, reader));
        }
        return map;
      }
      case "struct": {
        const object = {};
        for (const [name, field] of schema.fields) object[name] = this.decode(field, reader);
        return object;
      }
      case "enum":
        return this.decodeEnum(schema, reader);
      default:
        throw new TypeError(`web-rpc: unknown schema kind ${schema.kind}`);
    }
  }

  decodeEnum(schema, reader) {
    const index = reader.varint32();
    const entry = schema.variants[index];
    if (!entry) throw new RangeError(`web-rpc: unknown variant index ${index}`);
    const [tag, variant] = entry;
    if (variant === "unit") return isUnitOnly(schema.variants) ? tag : { tag };
    switch (variant.kind) {
      case "newtype":
        return { tag, value: this.decode(variant.inner, reader) };
      case "tuple":
        return { tag, value: this.decode(variant, reader) };
      case "struct":
        return { tag, ...this.decode(variant, reader) };
      default:
        throw new TypeError(`web-rpc: unknown variant kind ${variant.kind}`);
    }
  }

  encodeWire(description, writer, value, jsValues, transferList) {
    switch (description.kind) {
      case "js":
        writer.varint32(WIRE_JS);
        jsValues.push(value);
        if (description.transfer) transferList.push(value);
        return;
      case "postcard": {
        writer.varint32(WIRE_BYTES);
        const inner = new Writer();
        this.encode(description.schema, inner, value);
        writer.bytes(inner.take());
        return;
      }
      case "inline":
        this.encode(description.schema, writer, value);
        return;
      case "option":
        if (value === undefined || value === null) {
          writer.varint32(WIRE_NONE);
        } else {
          writer.varint32(WIRE_SOME);
          this.encodeWire(description.inner, writer, value, jsValues, transferList);
        }
        return;
      case "result":
        if (value && value.tag === "Ok") {
          writer.varint32(WIRE_OK);
          this.encodeWire(description.ok, writer, value.value, jsValues, transferList);
        } else if (value && value.tag === "Err") {
          writer.varint32(WIRE_ERR);
          this.encodeWire(description.err, writer, value.value, jsValues, transferList);
        } else {
          throw new TypeError('web-rpc: expected { tag: "Ok" } or { tag: "Err" }');
        }
        return;
      default:
        throw new TypeError(`web-rpc: unknown wire kind ${description.kind}`);
    }
  }

  decodeWire(description, reader, jsValues) {
    if (description.kind === "inline") return this.decode(description.schema, reader);
    const tag = reader.varint32();
    switch (description.kind) {
      case "js":
        expectTag(tag, WIRE_JS, "a Javascript value");
        return jsValues.shift();
      case "postcard":
        expectTag(tag, WIRE_BYTES, "postcard bytes");
        return this.decode(description.schema, reader.sub());
      case "option":
        if (tag === WIRE_NONE) return undefined;
        expectTag(tag, WIRE_SOME, "an option");
        return this.decodeWire(description.inner, reader, jsValues);
      case "result":
        if (tag === WIRE_OK) {
          return { tag: "Ok", value: this.decodeWire(description.ok, reader, jsValues) };
        }
        expectTag(tag, WIRE_ERR, "a result");
        return { tag: "Err", value: this.decodeWire(description.err, reader, jsValues) };
      default:
        throw new TypeError(`web-rpc: unknown wire kind ${description.kind}`);
    }
  }
}

/** True when a method's promise resolves the `Ok` payload and rejects with the `Err`. */
function isFallible(method) {
  return method.kind === "value" && method.returns.kind === "result";
}

/**
 * A `Worker`, `MessagePort` or `DedicatedWorkerGlobalScope`, wrapped so that the endpoint has
 * one way to post and one way to listen.
 *
 * The transport is used, never owned: nothing here creates it, terminates it, or calls
 * `start()` on a `MessagePort`. An unstarted port delivers nothing to these listeners.
 */
class Transport {
  constructor(target) {
    if (
      !target ||
      typeof target.postMessage !== "function" ||
      typeof target.addEventListener !== "function"
    ) {
      throw new TypeError("web-rpc: endpoint must be a Worker, MessagePort or worker scope");
    }
    this.target = target;
    this.listeners = [];
  }

  post(message, transferList) {
    this.target.postMessage(message, transferList);
  }

  on(type, handler) {
    this.target.addEventListener(type, handler);
    this.listeners.push([type, handler]);
  }

  detach() {
    for (const [type, handler] of this.listeners) {
      this.target.removeEventListener(type, handler);
    }
    this.listeners = [];
  }
}

function abortError(message) {
  const error = new Error(message);
  error.name = "AbortError";
  return error;
}

/** An `Error` becomes its message; anything else is encoded as thrown. */
function reduceThrown(thrown) {
  return thrown instanceof Error ? thrown.message : thrown;
}

/**
 * The base class of every generated endpoint.
 *
 * `methods` describes the trait this endpoint calls and `handlers` the trait it implements.
 * Each entry is `{ name, kind, args, returns }`, where `kind` is "value", "notify" or
 * "stream", `args` is a list of wire descriptions, and `returns` is the wire description of
 * the response or stream item, or null for a notification. An entry's position is the
 * method's index on the wire.
 */
class Endpoint {
  constructor(options, className, codec, methods, handlers) {
    const { endpoint, handlers: implementation } = options ?? {};
    if (!endpoint) {
      throw new TypeError(`${className}: options.endpoint is required`);
    }
    if (handlers.length > 0) {
      if (!implementation || typeof implementation !== "object") {
        throw new TypeError(`${className}: options.handlers is required`);
      }
      const missing = handlers
        .filter((handler) => typeof implementation[handler.name] !== "function")
        .map((handler) => handler.name);
      if (missing.length > 0) {
        throw new TypeError(`${className}: missing handlers: ${missing.join(", ")}`);
      }
    }

    this._className = className;
    this._codec = codec;
    this._methods = methods;
    this._handlers = handlers;
    this._implementation = implementation;
    this._transport = new Transport(endpoint);
    this._sequence = 0;
    this._pendingRequests = new Map();
    this._openStreams = new Map();
    this._runningIterators = new Map();
    this._inflightRequests = new Set();
    this._ready = false;
    this._failure = null;
    this._closed = false;
    this._queuedSends = [];
    this._inbox = [];

    this._transport.on("message", (event) => this._onMessage(event.data));
    this._transport.on("messageerror", () =>
      this._fail(new Error(`${className}: a message could not be deserialized`)),
    );
    this._transport.on("error", (event) =>
      this._fail(
        new Error(`${className}: the transport failed to start: ${event.message ?? event.type}`),
      ),
    );

    this._poll = setInterval(() => {
      if (!this._ready && !this._closed && !this._failure) this._transport.post(null, []);
    }, 10);
    this._transport.post(null, []);
  }

  // -- handshake and transport -------------------------------------------------

  _onMessage(data) {
    if (this._closed) return;
    if (!Array.isArray(data)) {
      // Handshake: the peer is listening. Answer once so it sees us too.
      if (!this._ready) {
        this._ready = true;
        clearInterval(this._poll);
        this._transport.post(null, []);
        const queued = this._queuedSends;
        this._queuedSends = [];
        for (const send of queued) send();
        const inbox = this._inbox;
        this._inbox = [];
        for (const message of inbox) this._dispatch(message);
      }
      return;
    }
    if (!this._ready) {
      this._inbox.push(data);
      return;
    }
    this._dispatch(data);
  }

  _fail(error) {
    if (this._failure || this._closed) return;
    this._failure = error;
    clearInterval(this._poll);
    for (const pending of this._pendingRequests.values()) pending.reject(error);
    this._pendingRequests.clear();
    for (const stream of this._openStreams.values()) stream.rejectDone(error);
    this._openStreams.clear();
    this._queuedSends = [];
  }

  /** Post `[header, payload?, ...jsValues]`, or queue it until the handshake completes. */
  _send(kind, sequence, payload, jsValues, transferList) {
    if (this._closed) return;
    const send = () => {
      const header = new Writer();
      header.varint32(kind);
      header.varint32(sequence);
      const headerBytes = header.take();
      const message = [headerBytes.buffer];
      const transfer = [headerBytes.buffer];
      if (payload) {
        const payloadBytes = payload.take();
        message.push(payloadBytes.buffer);
        transfer.push(payloadBytes.buffer);
      }
      this._transport.post(message.concat(jsValues), transfer.concat(transferList));
    };
    if (this._ready) send();
    else this._queuedSends.push(send);
  }

  _encodeCall(index, args) {
    const method = this._methods[index];
    const payload = new Writer();
    const jsValues = [];
    const transferList = [];
    payload.varint32(index);
    method.args.forEach((description, position) => {
      this._codec.encodeWire(description, payload, args[position], jsValues, transferList);
    });
    return { payload, jsValues, transferList };
  }

  _decodeReturn(index, method, payload, jsValues) {
    const actual = payload.varint32();
    if (actual !== index) {
      throw new TypeError(`${this._className}: response is for another method`);
    }
    return this._codec.decodeWire(method.returns, payload, jsValues);
  }

  // -- calling the other side --------------------------------------------------

  _request(index, args) {
    const method = this._methods[index];
    const sequence = this._sequence++;
    let settle;
    const promise = new Promise((resolve, reject) => {
      settle = { resolve, reject };
    });
    if (this._closed) {
      settle.reject(abortError(`${this._className}: endpoint closed`));
    } else if (this._failure) {
      settle.reject(this._failure);
    } else {
      const { payload, jsValues, transferList } = this._encodeCall(index, args);
      this._pendingRequests.set(sequence, { ...settle, index, method });
      this._send(REQUEST, sequence, payload, jsValues, transferList);
    }
    promise.abort = () => {
      if (!this._pendingRequests.has(sequence)) return;
      this._pendingRequests.delete(sequence);
      this._send(ABORT, sequence, null, [], []);
      settle.reject(abortError(`${this._className}: request aborted`));
    };
    return promise;
  }

  _notify(index, args) {
    if (this._closed || this._failure) return;
    const { payload, jsValues, transferList } = this._encodeCall(index, args);
    this._send(REQUEST, this._sequence++, payload, jsValues, transferList);
  }

  _stream(index, args, callback) {
    if (typeof callback !== "function") {
      throw new TypeError(`${this._className}: a streaming method needs a callback`);
    }
    const method = this._methods[index];
    const sequence = this._sequence++;
    let resolveDone;
    let rejectDone;
    const done = new Promise((resolve, reject) => {
      resolveDone = resolve;
      rejectDone = reject;
    });
    if (this._closed || this._failure) {
      rejectDone(this._failure ?? abortError(`${this._className}: endpoint closed`));
      return { close() {}, done };
    }
    const state = {
      index,
      method,
      callback,
      queue: Promise.resolve(),
      resolveDone,
      rejectDone,
      closed: false,
    };
    this._openStreams.set(sequence, state);
    const { payload, jsValues, transferList } = this._encodeCall(index, args);
    this._send(REQUEST, sequence, payload, jsValues, transferList);
    return {
      close: () => {
        if (state.closed) return;
        state.closed = true;
        this._send(ABORT, sequence, null, [], []);
      },
      done,
    };
  }

  /** Detach listeners, reject pending requests and close open streams. */
  close() {
    if (this._closed) return;
    this._closed = true;
    clearInterval(this._poll);
    this._transport.detach();
    const error = abortError(`${this._className}: endpoint closed`);
    for (const pending of this._pendingRequests.values()) pending.reject(error);
    this._pendingRequests.clear();
    for (const stream of this._openStreams.values()) {
      stream.queue.then(stream.resolveDone, stream.resolveDone);
    }
    this._openStreams.clear();
    for (const iterator of this._runningIterators.values()) {
      if (typeof iterator.return === "function") iterator.return();
    }
    this._runningIterators.clear();
    this._inflightRequests.clear();
    this._queuedSends = [];
  }

  // -- inbound -----------------------------------------------------------------

  _dispatch(message) {
    const slots = message.slice();
    const header = new Reader(new Uint8Array(slots.shift()));
    const kind = header.varint32();
    const sequence = header.varint32();
    switch (kind) {
      case RESPONSE: {
        const payload = new Reader(new Uint8Array(slots.shift()));
        const pending = this._pendingRequests.get(sequence);
        if (!pending) return;
        this._pendingRequests.delete(sequence);
        try {
          const value = this._decodeReturn(pending.index, pending.method, payload, slots);
          if (!isFallible(pending.method)) pending.resolve(value);
          else if (value.tag === "Ok") pending.resolve(value.value);
          else pending.reject(value.value);
        } catch (error) {
          pending.reject(error);
        }
        return;
      }
      case STREAM_ITEM: {
        const payload = new Reader(new Uint8Array(slots.shift()));
        const state = this._openStreams.get(sequence);
        if (!state) return;
        let item;
        try {
          item = this._decodeReturn(state.index, state.method, payload, slots);
        } catch (error) {
          this._openStreams.delete(sequence);
          state.rejectDone(error);
          return;
        }
        state.queue = state.queue.then(() => state.callback(item));
        return;
      }
      case STREAM_END: {
        const state = this._openStreams.get(sequence);
        if (!state) return;
        this._openStreams.delete(sequence);
        state.queue.then(state.resolveDone, state.rejectDone);
        return;
      }
      case REQUEST: {
        const payload = new Reader(new Uint8Array(slots.shift()));
        this._onRequest(sequence, payload, slots);
        return;
      }
      case ABORT: {
        this._inflightRequests.delete(sequence);
        const iterator = this._runningIterators.get(sequence);
        if (iterator) {
          this._runningIterators.delete(sequence);
          if (typeof iterator.return === "function") iterator.return();
        }
        return;
      }
      default:
        console.error(`${this._className}: unknown message kind ${kind}`);
    }
  }

  _onRequest(sequence, payload, slots) {
    let index;
    let handler;
    let args;
    try {
      index = payload.varint32();
      handler = this._handlers[index];
      if (!handler) throw new RangeError(`unknown method index ${index}`);
      args = handler.args.map((description) =>
        this._codec.decodeWire(description, payload, slots),
      );
    } catch (error) {
      console.error(`${this._className}: could not decode an inbound request:`, error);
      return;
    }
    const implementation = this._implementation[handler.name];
    if (handler.kind === "notify") {
      try {
        implementation.apply(this._implementation, args);
      } catch (error) {
        console.error(`${this._className}: ${handler.name} threw:`, error);
      }
      return;
    }
    if (handler.kind === "stream") {
      this._runStream(sequence, index, handler, implementation, args);
      return;
    }
    this._inflightRequests.add(sequence);
    Promise.resolve()
      .then(() => implementation.apply(this._implementation, args))
      .then(
        (value) => {
          if (!this._inflightRequests.delete(sequence)) return;
          // A handler returns the `Ok` payload.
          const returned = isFallible(handler) ? { tag: "Ok", value } : value;
          this._respond(sequence, index, handler, returned);
        },
        (error) => {
          if (!this._inflightRequests.delete(sequence)) return;
          if (isFallible(handler)) {
            this._respond(sequence, index, handler, { tag: "Err", value: reduceThrown(error) });
          } else {
            console.error(`${this._className}: ${handler.name} threw:`, error);
          }
        },
      );
  }

  _encodeReturn(index, handler, value) {
    const payload = new Writer();
    const jsValues = [];
    const transferList = [];
    payload.varint32(index);
    this._codec.encodeWire(handler.returns, payload, value, jsValues, transferList);
    return { payload, jsValues, transferList };
  }

  _respond(sequence, index, handler, value) {
    let encoded;
    try {
      encoded = this._encodeReturn(index, handler, value);
    } catch (error) {
      console.error(`${this._className}: could not encode the result of ${handler.name}:`, error);
      return;
    }
    this._send(RESPONSE, sequence, encoded.payload, encoded.jsValues, encoded.transferList);
  }

  /**
   * Drive one inbound streaming call.
   *
   * An `Abort` arrives as a message event, so it is only observed between event loop turns: a
   * producer that never awaits a macrotask runs to completion whatever the caller does.
   */
  _runStream(sequence, index, handler, implementation, args) {
    let iterator;
    try {
      const iterable = implementation.apply(this._implementation, args);
      iterator =
        typeof iterable[Symbol.asyncIterator] === "function"
          ? iterable[Symbol.asyncIterator]()
          : iterable[Symbol.iterator]();
    } catch (error) {
      console.error(`${this._className}: ${handler.name} threw:`, error);
      this._send(STREAM_END, sequence, null, [], []);
      return;
    }
    this._runningIterators.set(sequence, iterator);
    (async () => {
      try {
        for (;;) {
          const step = await iterator.next();
          if (step.done) break;
          if (!this._runningIterators.has(sequence)) break;
          const encoded = this._encodeReturn(index, handler, step.value);
          this._send(STREAM_ITEM, sequence, encoded.payload, encoded.jsValues, encoded.transferList);
        }
      } catch (error) {
        console.error(`${this._className}: ${handler.name} threw:`, error);
      } finally {
        this._runningIterators.delete(sequence);
        this._send(STREAM_END, sequence, null, [], []);
      }
    })();
  }
}
