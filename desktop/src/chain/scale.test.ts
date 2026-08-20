import { describe, it, expect } from "vitest";
import {
  hexToBytes,
  bytesToHex,
  u32LE,
  u128LE,
  decodeCompact,
  extractU32KeySuffix,
  formatAgr,
} from "./scale";

describe("hexToBytes / bytesToHex", () => {
  it("round-trips a 0x-prefixed hex string", () => {
    const bytes = hexToBytes("0xdeadbeef");
    expect(Array.from(bytes)).toEqual([0xde, 0xad, 0xbe, 0xef]);
    expect(bytesToHex(bytes)).toBe("0xdeadbeef");
  });

  it("handles hex without a 0x prefix", () => {
    const bytes = hexToBytes("00ff");
    expect(Array.from(bytes)).toEqual([0x00, 0xff]);
  });

  it("bytesToHex supports a start/end slice", () => {
    const bytes = hexToBytes("0x0011223344");
    expect(bytesToHex(bytes, 1, 3)).toBe("0x1122");
  });

  it("hexToBytes on an empty payload returns an empty array", () => {
    expect(Array.from(hexToBytes("0x"))).toEqual([]);
  });
});

describe("u32LE", () => {
  it("reads a little-endian u32 at offset 0", () => {
    // 0x01 0x00 0x00 0x00 -> 1
    expect(u32LE(new Uint8Array([0x01, 0x00, 0x00, 0x00]), 0)).toBe(1);
  });

  it("reads a little-endian u32 at a non-zero offset", () => {
    // offset 2: bytes [0x2a, 0x00, 0x00, 0x00] -> 42
    const bytes = new Uint8Array([0xff, 0xff, 0x2a, 0x00, 0x00, 0x00]);
    expect(u32LE(bytes, 2)).toBe(42);
  });

  it("reads the max u32 value without going negative", () => {
    const bytes = new Uint8Array([0xff, 0xff, 0xff, 0xff]);
    expect(u32LE(bytes, 0)).toBe(0xffffffff);
  });
});

describe("u128LE", () => {
  it("reads a little-endian u128 as a bigint", () => {
    const bytes = new Uint8Array(16);
    bytes[0] = 0x64; // 100 in the low byte
    expect(u128LE(bytes, 0)).toBe(100n);
  });

  it("reads a u128 spanning multiple bytes", () => {
    // 1_000_000_000_000n (1 AGR in Planck) little-endian
    const value = 1_000_000_000_000n;
    const bytes = new Uint8Array(16);
    let v = value;
    for (let i = 0; i < 16; i++) {
      bytes[i] = Number(v & 0xffn);
      v >>= 8n;
    }
    expect(u128LE(bytes, 0)).toBe(value);
  });

  it("reads at a non-zero offset, tolerating a short trailing buffer", () => {
    const bytes = new Uint8Array([0, 0, 5]); // offset 2, only 1 byte available
    expect(u128LE(bytes, 2)).toBe(5n);
  });
});

describe("decodeCompact", () => {
  it("decodes single-byte mode (mode 0)", () => {
    // value 3 encoded as (3 << 2) | 0 = 12
    expect(decodeCompact(new Uint8Array([12]))).toEqual([3, 1]);
  });

  it("decodes two-byte mode (mode 1)", () => {
    // value 100 -> (100 << 2) | 1 = 401 = 0x0191 LE -> [0x91, 0x01]
    const encoded = ((100 << 2) | 1) & 0xffff;
    const bytes = new Uint8Array([encoded & 0xff, (encoded >> 8) & 0xff]);
    expect(decodeCompact(bytes)).toEqual([100, 2]);
  });

  it("decodes four-byte mode (mode 2)", () => {
    const value = 70000;
    const encoded = (value << 2) | 2;
    const bytes = new Uint8Array(4);
    bytes[0] = encoded & 0xff;
    bytes[1] = (encoded >>> 8) & 0xff;
    bytes[2] = (encoded >>> 16) & 0xff;
    bytes[3] = (encoded >>> 24) & 0xff;
    expect(decodeCompact(bytes)).toEqual([value, 4]);
  });

  it("returns [0, 0] on an empty buffer", () => {
    expect(decodeCompact(new Uint8Array([]))).toEqual([0, 0]);
  });
});

describe("extractU32KeySuffix", () => {
  it("reads the trailing 4 bytes of a Blake2_128Concat key as a little-endian u32", () => {
    const keyBytes = new Uint8Array(36);
    keyBytes[32] = 0x07; // low byte of the u32 suffix
    expect(extractU32KeySuffix(keyBytes)).toBe(7);
  });

  it("returns 0 for a key shorter than 4 bytes", () => {
    expect(extractU32KeySuffix(new Uint8Array([1, 2]))).toBe(0);
  });
});

describe("formatAgr", () => {
  it("formats zero", () => {
    expect(formatAgr(0n)).toBe("0 AGR");
  });

  it("formats a whole-number amount with no fraction", () => {
    expect(formatAgr(5_000_000_000_000n)).toBe("5 AGR");
  });

  it("formats a fractional amount to 4 decimal places", () => {
    // 1.5 AGR = 1_500_000_000_000 Planck
    expect(formatAgr(1_500_000_000_000n)).toBe("1.5000 AGR");
  });

  it("pads small fractional remainders", () => {
    // 0.0001 AGR = 100_000_000 Planck
    expect(formatAgr(100_000_000n)).toBe("0.0001 AGR");
  });
});
