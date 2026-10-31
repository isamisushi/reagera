import { test } from "node:test";
import assert from "node:assert/strict";
import { codec, decode, encode, fnv1a64, fromHex, toHex, CodecError } from "../src/index.js";

test("integers are big endian and fixed width", () => {
  assert.equal(toHex(encode(codec.u8, 7)), "07");
  assert.equal(toHex(encode(codec.u16, 258)), "0102");
  assert.equal(toHex(encode(codec.u32, 1)), "00000001");
  assert.equal(toHex(encode(codec.i64, -1)), "ffffffffffffffff");
  assert.equal(decode(codec.i64, fromHex("ffffffffffffffff")), -1);
});

test("strings carry a u32 length prefix", () => {
  assert.equal(toHex(encode(codec.string, "hi")), "000000026869");
  assert.equal(decode(codec.string, fromHex("000000026869")), "hi");
});

test("struct fields follow declaration order, not object key order", () => {
  const Point = codec.struct({ x: codec.u8, y: codec.u8 });
  assert.equal(toHex(encode(Point, { y: 2, x: 1 })), "0102");
});

test("variant, list and option round trip", () => {
  const Shape = codec.variant({
    dot: codec.unit,
    line: codec.struct({ len: codec.f64, name: codec.option(codec.string) }),
    poly: codec.list(codec.u16),
  });
  const cases: Array<ReturnType<typeof Shape.read>> = [
    { type: "dot", payload: undefined },
    { type: "line", payload: { len: 1.5, name: undefined } },
    { type: "line", payload: { len: -0.25, name: "émoji 🎉" } },
    { type: "poly", payload: [1, 2, 65535] },
  ];
  for (const c of cases) assert.deepEqual(decode(Shape, encode(Shape, c)), c);
  assert.equal(toHex(encode(Shape, { type: "dot", payload: undefined })), "00");
});

test("decoding rejects bad tags, truncation and trailing bytes", () => {
  assert.throws(() => decode(codec.bool, fromHex("02")), CodecError);
  assert.throws(() => decode(codec.u32, fromHex("0000")), CodecError);
  assert.throws(() => decode(codec.u8, fromHex("0000")), CodecError);
  assert.throws(() => decode(codec.option(codec.u8), fromHex("05")), CodecError);
});

test("fnv1a64 matches the reference vector", () => {
  assert.equal(fnv1a64(new Uint8Array()), "cbf29ce484222325");
  assert.equal(fnv1a64(new TextEncoder().encode("a")), "af63dc4c8601ec8c");
});
