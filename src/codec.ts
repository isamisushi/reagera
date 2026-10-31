/**
 * Binary codec in the spirit of minbin: explicit, auditable, zero dependencies.
 *
 * Wire format (no header, no schema metadata, no padding):
 *   u8/u16/u32  fixed width, big endian
 *   i64         fixed width, big endian, two's complement (JavaScript number, safe-integer range)
 *   f64         IEEE 754, big endian
 *   bool        one byte, 0 or 1; other values are invalid
 *   string      u32 UTF-8 byte length, then the bytes
 *   list<T>     u32 element count, then the elements in order
 *   option<T>   one tag byte, 0 for none, 1 followed by the value
 *   struct      fields in the declared order
 *   variant     u8 tag, then the fields of that variant in the declared order
 *
 * Every codec is a plain object you can read in a minute. There is no reflection
 * and no automatic derivation, so the bytes of an event are decided by the
 * schema you wrote, not by property order in a JavaScript object.
 */

export class CodecError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CodecError";
  }
}

export class BytesWriter {
  private buf: Uint8Array;
  private view: DataView;
  private pos = 0;

  constructor(capacity = 256) {
    this.buf = new Uint8Array(capacity);
    this.view = new DataView(this.buf.buffer);
  }

  private ensure(extra: number): void {
    if (this.pos + extra <= this.buf.length) return;
    let next = this.buf.length * 2;
    while (next < this.pos + extra) next *= 2;
    const grown = new Uint8Array(next);
    grown.set(this.buf);
    this.buf = grown;
    this.view = new DataView(grown.buffer);
  }

  u8(v: number): void {
    this.ensure(1);
    this.view.setUint8(this.pos, v);
    this.pos += 1;
  }
  u16(v: number): void {
    this.ensure(2);
    this.view.setUint16(this.pos, v, false);
    this.pos += 2;
  }
  u32(v: number): void {
    this.ensure(4);
    this.view.setUint32(this.pos, v, false);
    this.pos += 4;
  }
  i64(v: number): void {
    if (!Number.isSafeInteger(v)) throw new CodecError(`i64 expects a safe integer, got ${v}`);
    this.ensure(8);
    this.view.setBigInt64(this.pos, BigInt(v), false);
    this.pos += 8;
  }
  f64(v: number): void {
    this.ensure(8);
    this.view.setFloat64(this.pos, v, false);
    this.pos += 8;
  }
  bool(v: boolean): void {
    this.u8(v ? 1 : 0);
  }
  bytes(src: Uint8Array): void {
    this.ensure(src.length);
    this.buf.set(src, this.pos);
    this.pos += src.length;
  }
  string(v: string): void {
    const encoded = textEncoder.encode(v);
    this.u32(encoded.length);
    this.bytes(encoded);
  }
  finish(): Uint8Array {
    return this.buf.slice(0, this.pos);
  }
  get length(): number {
    return this.pos;
  }
}

export class BytesReader {
  private view: DataView;
  pos = 0;

  constructor(private readonly buf: Uint8Array) {
    this.view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  }

  private need(n: number): void {
    if (this.pos + n > this.buf.length) {
      throw new CodecError(`unexpected end of input at byte ${this.pos}, need ${n} more`);
    }
  }

  u8(): number {
    this.need(1);
    const v = this.view.getUint8(this.pos);
    this.pos += 1;
    return v;
  }
  u16(): number {
    this.need(2);
    const v = this.view.getUint16(this.pos, false);
    this.pos += 2;
    return v;
  }
  u32(): number {
    this.need(4);
    const v = this.view.getUint32(this.pos, false);
    this.pos += 4;
    return v;
  }
  i64(): number {
    this.need(8);
    const v = this.view.getBigInt64(this.pos, false);
    this.pos += 8;
    if (v > BigInt(Number.MAX_SAFE_INTEGER) || v < BigInt(Number.MIN_SAFE_INTEGER)) {
      throw new CodecError(`i64 value ${v} is outside the safe integer range`);
    }
    return Number(v);
  }
  f64(): number {
    this.need(8);
    const v = this.view.getFloat64(this.pos, false);
    this.pos += 8;
    return v;
  }
  bool(): boolean {
    const v = this.u8();
    if (v > 1) throw new CodecError(`invalid bool byte ${v}`);
    return v === 1;
  }
  bytes(n: number): Uint8Array {
    this.need(n);
    const v = this.buf.subarray(this.pos, this.pos + n);
    this.pos += n;
    return v;
  }
  string(): string {
    const len = this.u32();
    return textDecoder.decode(this.bytes(len));
  }
  get remaining(): number {
    return this.buf.length - this.pos;
  }
}

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: true });

/** A codec knows how to write a value and how to read one back. */
export interface Codec<T> {
  write(w: BytesWriter, value: T): void;
  read(r: BytesReader): T;
}

export type Infer<C> = C extends Codec<infer T> ? T : never;

export const u8: Codec<number> = { write: (w, v) => w.u8(v), read: (r) => r.u8() };
export const u16: Codec<number> = { write: (w, v) => w.u16(v), read: (r) => r.u16() };
export const u32: Codec<number> = { write: (w, v) => w.u32(v), read: (r) => r.u32() };
export const i64: Codec<number> = { write: (w, v) => w.i64(v), read: (r) => r.i64() };
export const f64: Codec<number> = { write: (w, v) => w.f64(v), read: (r) => r.f64() };
export const bool: Codec<boolean> = { write: (w, v) => w.bool(v), read: (r) => r.bool() };
export const string: Codec<string> = { write: (w, v) => w.string(v), read: (r) => r.string() };
export const unit: Codec<undefined> = { write: () => {}, read: () => undefined };

export function list<T>(item: Codec<T>): Codec<T[]> {
  return {
    write(w, v) {
      w.u32(v.length);
      for (const x of v) item.write(w, x);
    },
    read(r) {
      const n = r.u32();
      const out: T[] = new Array(n);
      for (let i = 0; i < n; i++) out[i] = item.read(r);
      return out;
    },
  };
}

export function option<T>(inner: Codec<T>): Codec<T | undefined> {
  return {
    write(w, v) {
      if (v === undefined) w.u8(0);
      else {
        w.u8(1);
        inner.write(w, v);
      }
    },
    read(r) {
      const tag = r.u8();
      if (tag === 0) return undefined;
      if (tag === 1) return inner.read(r);
      throw new CodecError(`invalid option tag ${tag}`);
    },
  };
}

type Fields = Record<string, Codec<unknown>>;
type StructOf<F extends Fields> = { [K in keyof F]: Infer<F[K]> };

/** Fields are written in the order they are declared in the object literal. */
export function struct<F extends Fields>(fields: F): Codec<StructOf<F>> {
  const entries = Object.entries(fields) as [keyof F & string, Codec<unknown>][];
  return {
    write(w, v) {
      for (const [k, c] of entries) c.write(w, (v as Record<string, unknown>)[k]);
    },
    read(r) {
      const out: Record<string, unknown> = {};
      for (const [k, c] of entries) out[k] = c.read(r);
      return out as StructOf<F>;
    },
  };
}

/**
 * A tagged union. Each key gets a u8 tag equal to its position (0, 1, 2, ...).
 * Reordering keys changes the format, so append new variants at the end.
 */
export function variant<V extends Record<string, Codec<unknown>>>(
  variants: V,
): Codec<{ [K in keyof V]: { type: K; payload: Infer<V[K]> } }[keyof V]> {
  const keys = Object.keys(variants) as (keyof V & string)[];
  if (keys.length > 255) throw new CodecError("variant supports at most 255 cases");
  const tagOf = new Map(keys.map((k, i) => [k, i]));
  return {
    write(w, v) {
      const tag = tagOf.get(v.type as string);
      if (tag === undefined) throw new CodecError(`unknown variant ${String(v.type)}`);
      w.u8(tag);
      variants[v.type as string].write(w, v.payload);
    },
    read(r) {
      const tag = r.u8();
      const key = keys[tag];
      if (key === undefined) throw new CodecError(`invalid variant tag ${tag}`);
      return { type: key, payload: variants[key].read(r) } as never;
    },
  };
}

export function encode<T>(codec: Codec<T>, value: T): Uint8Array {
  const w = new BytesWriter();
  codec.write(w, value);
  return w.finish();
}

export function decode<T>(codec: Codec<T>, bytes: Uint8Array): T {
  const r = new BytesReader(bytes);
  const v = codec.read(r);
  if (r.remaining !== 0) throw new CodecError(`${r.remaining} trailing bytes after value`);
  return v;
}

/**
 * FNV-1a, 64 bit, returned as 16 lowercase hex characters.
 * Used for content IDs: the same bytes always get the same ID.
 */
export function fnv1a64(bytes: Uint8Array): string {
  let h = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (let i = 0; i < bytes.length; i++) {
    h ^= BigInt(bytes[i]);
    h = (h * prime) & mask;
  }
  return h.toString(16).padStart(16, "0");
}

export function toHex(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += bytes[i].toString(16).padStart(2, "0");
  return s;
}

export function fromHex(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new CodecError("hex string has odd length");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    const b = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    if (Number.isNaN(b)) throw new CodecError(`invalid hex at ${i * 2}`);
    out[i] = b;
  }
  return out;
}

export function toBase64(bytes: Uint8Array): string {
  if (typeof Buffer !== "undefined") return Buffer.from(bytes).toString("base64");
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}

export function fromBase64(text: string): Uint8Array {
  if (typeof Buffer !== "undefined") return new Uint8Array(Buffer.from(text, "base64"));
  const s = atob(text);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}
