// Minimal, dependency-free SCALE/byte-layout helpers for decoding the handful of storage
// items the desktop app reads directly. These are a straight TypeScript port of the same
// hand-rolled byte parsing that used to live in `desktop/src-tauri/src/commands/chain.rs`
// (see the doc comments on each `fetch_*` command there for the authoritative byte layouts
// this mirrors) — kept dependency-free (no generated pallet types) for the same reason the
// Rust version was: these are read-only UI projections, not consensus-critical decoding, and
// a hand-rolled decoder avoids pulling in `@polkadot/types` metadata plumbing for a handful of
// fixed-width struct layouts.

export function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  const bytes = new Uint8Array(Math.floor(clean.length / 2));
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(clean.substr(i * 2, 2), 16);
  }
  return bytes;
}

export function bytesToHex(bytes: Uint8Array, start = 0, end = bytes.length): string {
  let out = "0x";
  for (let i = start; i < end; i++) {
    out += bytes[i].toString(16).padStart(2, "0");
  }
  return out;
}

/** Little-endian u32 read at `offset`. */
export function u32LE(bytes: Uint8Array, offset: number): number {
  return (
    (bytes[offset] | (bytes[offset + 1] << 8) | (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >>> 0
  );
}

/** Little-endian u128 read at `offset`, as a bigint (mirrors Rust's `u128::from_le_bytes`). */
export function u128LE(bytes: Uint8Array, offset: number): bigint {
  let v = 0n;
  for (let i = 15; i >= 0; i--) {
    v = (v << 8n) | BigInt(bytes[offset + i] ?? 0);
  }
  return v;
}

/**
 * Decodes a SCALE compact-encoded integer from the start of `bytes`.
 * Returns [value, bytesConsumed]. Mirrors `decode_compact` in chain.rs — big-integer mode
 * (mode 3) isn't used by any of the pallets this reads, same caveat as the Rust original.
 */
export function decodeCompact(bytes: Uint8Array): [number, number] {
  if (bytes.length === 0) return [0, 0];
  const mode = bytes[0] & 0b11;
  if (mode === 0) return [bytes[0] >> 2, 1];
  if (mode === 1) {
    if (bytes.length < 2) return [0, 2];
    return [(bytes[0] | (bytes[1] << 8)) >> 2, 2];
  }
  if (mode === 2) {
    if (bytes.length < 4) return [0, 4];
    return [u32LE(bytes, 0) >>> 2, 4];
  }
  return [0, 1];
}

/**
 * Extracts a u32 key from the last 4 bytes of a Blake2_128Concat storage key.
 * Blake2_128Concat layout: prefix(32) + hash(16) + key_bytes(N) — the raw key value is
 * always the suffix, so for a u32 key the last 4 bytes are it.
 */
export function extractU32KeySuffix(keyBytes: Uint8Array): number {
  const s = keyBytes.length;
  if (s < 4) return 0;
  return u32LE(keyBytes, s - 4);
}

/** Formats a u128 Balance (in Planck = 1e-12 AGR) as a human-readable AGR string. */
export function formatAgr(planck: bigint): string {
  if (planck === 0n) return "0 AGR";
  const UNIT = 1_000_000_000_000n;
  const whole = planck / UNIT;
  const frac = planck % UNIT;
  if (frac === 0n) return `${whole} AGR`;
  const frac4 = frac / (UNIT / 10_000n);
  return `${whole}.${frac4.toString().padStart(4, "0")} AGR`;
}
