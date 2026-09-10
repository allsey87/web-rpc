// The shell's codec against the byte strings in WIRE.md, which tests/wire.rs asserts for the
// Rust side. Run by tests/js/check.py after extraction, since the shell only exists inside a
// generated module.

import assert from "node:assert/strict";
import { test } from "node:test";

const { Writer, Reader, Codec } = await import("./generated/clock_client.mjs");

function encoded(schema, value) {
  const writer = new Writer();
  new Codec({}).encode(schema, writer, value);
  return Array.from(writer.take());
}

function decoded(schema, bytes) {
  return new Codec({}).decode(schema, new Reader(new Uint8Array(bytes)));
}

function roundTrips(schema, value, bytes) {
  assert.deepEqual(encoded(schema, value), bytes);
  assert.deepEqual(decoded(schema, bytes), value);
}

test("primitives", () => {
  roundTrips("bool", true, [0x01]);
  roundTrips("bool", false, [0x00]);
  roundTrips("i8", -1, [0xff]);
  roundTrips("u32", 300, [0xac, 0x02]);
  roundTrips("i32", -1, [0x01]);
  roundTrips("i32", 1, [0x02]);
  roundTrips("i32", -150, [0xab, 0x02]);
  roundTrips("u64", 1n << 40n, [0x80, 0x80, 0x80, 0x80, 0x80, 0x20]);
  roundTrips("i64", -1n, [0x01]);
  roundTrips("i128", -(1n << 100n), [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x07,
  ]);
  roundTrips("f32", 1.0, [0x00, 0x00, 0x80, 0x3f]);
  roundTrips("string", "hi", [0x02, 0x68, 0x69]);
  roundTrips("char", "a", [0x01, 0x61]);
  roundTrips("unit", undefined, []);
});

test("containers", () => {
  const point = { kind: "struct", fields: [["left", "u32"], ["right", "string"]] };
  roundTrips(point, { left: 1, right: "x" }, [0x01, 0x01, 0x78]);
  roundTrips({ kind: "option", inner: "u32" }, undefined, [0x00]);
  roundTrips({ kind: "option", inner: "u32" }, 1, [0x01, 0x01]);
  roundTrips({ kind: "seq", inner: "u32" }, [1, 2], [0x02, 0x01, 0x02]);
  roundTrips({ kind: "tuple", items: ["u32", "u32"] }, [1, 2], [0x01, 0x02]);
  assert.deepEqual(decoded({ kind: "seq", inner: "u8" }, [0x02, 0x07, 0x08]), new Uint8Array([7, 8]));
  const map = { kind: "map", key: "string", value: "u32" };
  roundTrips(map, new Map([["a", 1]]), [0x01, 0x01, 0x61, 0x01]);
});

test("enums", () => {
  const colour = { kind: "enum", variants: [["Red", "unit"], ["Green", "unit"]] };
  roundTrips(colour, "Green", [0x01]);
  const shape = {
    kind: "enum",
    variants: [
      ["Empty", "unit"],
      ["Circle", { kind: "newtype", inner: "u32" }],
      ["Segment", { kind: "tuple", items: ["u32", "u32"] }],
      ["Label", { kind: "struct", fields: [["text", "string"]] }],
    ],
  };
  roundTrips(shape, { tag: "Empty" }, [0x00]);
  roundTrips(shape, { tag: "Circle", value: 3 }, [0x01, 0x03]);
  roundTrips(shape, { tag: "Segment", value: [1, 2] }, [0x02, 0x01, 0x02]);
  roundTrips(shape, { tag: "Label", text: "x" }, [0x03, 0x01, 0x78]);
  assert.throws(() => encoded(shape, { tag: "Nope" }), /unknown variant/);
});

test("declared references", () => {
  const schemas = { Pair: { kind: "struct", fields: [["left", "u32"], ["right", "string"]] } };
  const codec = new Codec(schemas);
  const writer = new Writer();
  codec.encode({ kind: "ref", name: "Pair" }, writer, { left: 2, right: "hi" });
  assert.deepEqual(Array.from(writer.take()), [0x02, 0x02, 0x68, 0x69]);
});

test("wire arguments", () => {
  const codec = new Codec({});
  const jsValues = [];
  const transferList = [];
  const writer = new Writer();
  codec.encodeWire({ kind: "postcard", schema: "u32" }, writer, 1, jsValues, transferList);
  codec.encodeWire({ kind: "js", transfer: true }, writer, "handle", jsValues, transferList);
  codec.encodeWire({ kind: "option", inner: { kind: "js", transfer: false } }, writer, undefined, jsValues, transferList);
  codec.encodeWire(
    { kind: "result", ok: { kind: "option", inner: { kind: "js", transfer: false } }, err: { kind: "postcard", schema: "u8" } },
    writer,
    { tag: "Ok", value: "value" },
    jsValues,
    transferList,
  );
  assert.deepEqual(Array.from(writer.take()), [0x01, 0x01, 0x01, 0x00, 0x02, 0x04, 0x03, 0x00]);
  assert.deepEqual(jsValues, ["handle", "value"]);
  assert.deepEqual(transferList, ["handle"]);

  const reader = new Reader(new Uint8Array([0x05, 0x01, 0x01, 0x07]));
  const result = codec.decodeWire(
    { kind: "result", ok: { kind: "js", transfer: false }, err: { kind: "postcard", schema: "u8" } },
    reader,
    [],
  );
  assert.deepEqual(result, { tag: "Err", value: 7 });
});
