/**
 * Tests the `CommitteeCrypto` injection boundary itself (the default stub throws a
 * clear "not implemented" error and never fabricates a result, and
 * `setCommitteeCrypto`/`resetCommitteeCrypto` correctly swap the module-wide instance —
 * no real cryptography in that part, there is none in `CommitteeCrypto.ts` by design),
 * **plus** the real `wasmCommitteeCrypto` implementation's correctness (changelog #084).
 *
 * # Ground-truth fixture
 *
 * `GROUND_TRUTH_INPUT_HEX`/`GROUND_TRUTH_OUTPUT_HEX` below are byte-for-byte
 * `oprf-committee-dev/src/ffi.rs`'s own `ffi::tests::sample_input()` fixture (same
 * `sk`/`blinded_query`/`ds_dlog`/seed `evaluate_query_matches_direct_native_call`
 * checks) and its real output, extracted by running that exact fixture through the real
 * Rust crate via a temporary, throwaway `cargo run --example` (not committed — see
 * `docs/project/changelog/084.md` for the full extraction method). This is the
 * strongest correctness check available without a real device: it doesn't just check
 * this file's own internal consistency, it asserts byte-identical output against an
 * independently-computed native Rust ground truth.
 */
import {
  getCommitteeCrypto,
  notImplementedCommitteeCrypto,
  resetCommitteeCrypto,
  setCommitteeCrypto,
  type CommitteeCrypto,
} from './CommitteeCrypto';
import { buildOprfInput, evaluateQueryWithSeed, wasmCommitteeCrypto } from './wasmCommitteeCrypto';

/** hex string -> Uint8Array. Throws on odd-length input (a real bug, not a valid fixture). */
function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error(`hexToBytes: odd-length hex string (${hex.length} chars)`);
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.substr(i * 2, 2), 16);
  return out;
}
function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

// oprf-committee-dev/src/ffi.rs::tests::sample_input() — sk=778899112233 (decimal),
// beta=4242424242424242, client_input=999999999999999999999999999,
// ds_dlog=1523098184080632582082867317389990410064981862, seed=[7u8; 32].
const GROUND_TRUTH_INPUT_HEX =
  '000000000000000000000000000000000000000000000000000000b55a01412905de00b0161084201f474cdc39fbb59c22c08da493dceebd9693829fd8b1d03f0d23a470130e100b767a45f24eef928ec81855dd242ff827772057aae6c5a30600000000000000000000000000444c4f4720457175616c6974792050726f6f660707070707070707070707070707070707070707070707070707070707070707';
const GROUND_TRUTH_OUTPUT_HEX =
  '17bd796d98b3e92244fd98b0b4f23b5bfb718cfe6bc828eef43448492addf9442b474454a692b4631c236af872074c94cc4907da424cc797e6e87f343ea99eea20b9a7a7c37ba9da6be116aa4e73be015b5d07ea12a76952d559a2f2afc12118045770af672c02c1d40a405e7af88e052a65be5c7b3e58895e80566288d476f924f077b1acea62862d5ac13eed291e7a95fc1c235a7c68b9fdbfcf56e12e6eb629addcee3a73fa5b76ed6ad753797458f145d64e37fe126c99fb47bec8e720f4';

const GROUND_TRUTH_SK = hexToBytes(GROUND_TRUTH_INPUT_HEX.slice(0, 64)); // bytes [0, 32)
const GROUND_TRUTH_BLINDED_QUERY = hexToBytes(GROUND_TRUTH_INPUT_HEX.slice(64, 192)); // bytes [32, 96)
const GROUND_TRUTH_SEED = hexToBytes(GROUND_TRUTH_INPUT_HEX.slice(256, 320)); // bytes [128, 160), all 0x07
const GROUND_TRUTH_OUTPUT = hexToBytes(GROUND_TRUTH_OUTPUT_HEX);

afterEach(() => {
  resetCommitteeCrypto();
});

describe('notImplementedCommitteeCrypto', () => {
  it('rejects with a "not implemented" error rather than returning a fabricated result', async () => {
    await expect(
      notImplementedCommitteeCrypto.evaluateQuery(new Uint8Array(32), new Uint8Array(64)),
    ).rejects.toThrow(/not implemented/);
  });
});

describe('getCommitteeCrypto / setCommitteeCrypto / resetCommitteeCrypto', () => {
  it('defaults to notImplementedCommitteeCrypto', () => {
    expect(getCommitteeCrypto()).toBe(notImplementedCommitteeCrypto);
  });

  it('returns whatever implementation was installed via setCommitteeCrypto', () => {
    const fixture: CommitteeCrypto = {
      evaluateQuery: jest.fn().mockResolvedValue({
        pk: new Uint8Array(64),
        evaluation: new Uint8Array(64),
        dlogProof: new Uint8Array(64),
      }),
    };
    setCommitteeCrypto(fixture);
    expect(getCommitteeCrypto()).toBe(fixture);
  });

  it('resetCommitteeCrypto restores the default stub', () => {
    setCommitteeCrypto({ evaluateQuery: jest.fn() });
    resetCommitteeCrypto();
    expect(getCommitteeCrypto()).toBe(notImplementedCommitteeCrypto);
  });
});

describe('buildOprfInput', () => {
  it('marshals sk/blindedQuery/seed into the exact 160-byte wire format ffi.rs expects', () => {
    // Checks the marshaling in isolation from the wasm core: the assembled input must be
    // byte-identical to the real fixture's own 160-byte input (which already embeds the
    // hardcoded DS_DLOG constant at bytes [96, 128)) — this would catch a field-order or
    // padding bug even if it happened to not affect a particular fixture's output.
    const input = buildOprfInput(GROUND_TRUTH_SK, GROUND_TRUTH_BLINDED_QUERY, GROUND_TRUTH_SEED);
    expect(bytesToHex(input)).toBe(GROUND_TRUTH_INPUT_HEX);
  });

  it('left-pads a secret key shorter than 32 bytes, matching ffi.rs\'s own left-pad convention', () => {
    const shortSk = new Uint8Array([0x01, 0x02, 0x03]);
    const input = buildOprfInput(shortSk, GROUND_TRUTH_BLINDED_QUERY, GROUND_TRUTH_SEED);
    expect(Array.from(input.slice(0, 29))).toEqual(new Array(29).fill(0));
    expect(Array.from(input.slice(29, 32))).toEqual([0x01, 0x02, 0x03]);
  });

  it('rejects a secret key longer than 32 bytes rather than silently truncating it', () => {
    expect(() => buildOprfInput(new Uint8Array(33), GROUND_TRUTH_BLINDED_QUERY, GROUND_TRUTH_SEED)).toThrow(
      /at most 32/,
    );
  });

  it('rejects a blindedQuery that is not exactly 64 bytes', () => {
    expect(() => buildOprfInput(GROUND_TRUTH_SK, new Uint8Array(63), GROUND_TRUTH_SEED)).toThrow(/64/);
  });
});

describe('evaluateQueryWithSeed (real crypto core, deterministic seed)', () => {
  it('produces byte-identical output to the real Rust ffi::evaluate_query on the same fixture', () => {
    const result = evaluateQueryWithSeed(GROUND_TRUTH_SK, GROUND_TRUTH_BLINDED_QUERY, GROUND_TRUTH_SEED);

    expect(result.pk).toHaveLength(64);
    expect(result.dlogProof).toHaveLength(64);
    expect(result.evaluation).toHaveLength(64);

    const combined = new Uint8Array(192);
    combined.set(result.pk, 0);
    combined.set(result.dlogProof, 64);
    combined.set(result.evaluation, 128);
    expect(bytesToHex(combined)).toBe(GROUND_TRUTH_OUTPUT_HEX);

    // Field-by-field too, so a failure points at exactly which piece diverged rather
    // than just "the 192 bytes don't match".
    expect(bytesToHex(result.pk)).toBe(bytesToHex(GROUND_TRUTH_OUTPUT.slice(0, 64)));
    expect(bytesToHex(result.dlogProof)).toBe(bytesToHex(GROUND_TRUTH_OUTPUT.slice(64, 128)));
    expect(bytesToHex(result.evaluation)).toBe(bytesToHex(GROUND_TRUTH_OUTPUT.slice(128, 192)));
  });

  it('rejects a zero secret key with the real module\'s ERR_ZERO_SECRET_KEY, not a fabricated result', () => {
    expect(() =>
      evaluateQueryWithSeed(new Uint8Array(32), GROUND_TRUTH_BLINDED_QUERY, GROUND_TRUTH_SEED),
    ).toThrow(/ERR_ZERO_SECRET_KEY/);
  });

  it('rejects an off-curve blindedQuery with the real module\'s ERR_BLINDED_QUERY_NOT_ON_CURVE', () => {
    // (1, 1) is not a BabyJubJub point — same off-curve fixture ffi.rs's own
    // `rejects_off_curve_blinded_query` test uses.
    const offCurve = new Uint8Array(64);
    offCurve[31] = 0x01; // x = 1
    offCurve[63] = 0x01; // y = 1
    expect(() => evaluateQueryWithSeed(GROUND_TRUTH_SK, offCurve, GROUND_TRUTH_SEED)).toThrow(
      /ERR_BLINDED_QUERY_NOT_ON_CURVE/,
    );
  });
});

describe('wasmCommitteeCrypto (public async interface, fresh randomness)', () => {
  it('resolves with correctly-shaped output for the ground-truth sk/blindedQuery', async () => {
    const result = await wasmCommitteeCrypto.evaluateQuery(GROUND_TRUTH_SK, GROUND_TRUTH_BLINDED_QUERY);
    expect(result.pk).toHaveLength(64);
    expect(result.dlogProof).toHaveLength(64);
    expect(result.evaluation).toHaveLength(64);
    // pk = sk*G does not depend on the per-call random seed, so it must match the
    // deterministic ground truth even though this call used fresh randomness internally.
    expect(bytesToHex(result.pk)).toBe(bytesToHex(GROUND_TRUTH_OUTPUT.slice(0, 64)));
  });

  it('uses fresh randomness per call — two calls on the same input produce different proofs', async () => {
    const first = await wasmCommitteeCrypto.evaluateQuery(GROUND_TRUTH_SK, GROUND_TRUTH_BLINDED_QUERY);
    const second = await wasmCommitteeCrypto.evaluateQuery(GROUND_TRUTH_SK, GROUND_TRUTH_BLINDED_QUERY);
    // Same secret key/query -> same pk and same blinded evaluation (neither depends on
    // the proof nonce), but a different dlog_e/dlog_s each time — reusing the seed would
    // leak the secret key (classic Chaum-Pedersen nonce-reuse attack), so this is a real
    // security property, not an incidental one.
    expect(bytesToHex(first.pk)).toBe(bytesToHex(second.pk));
    expect(bytesToHex(first.evaluation)).toBe(bytesToHex(second.evaluation));
    expect(bytesToHex(first.dlogProof)).not.toBe(bytesToHex(second.dlogProof));
  });

  it('rejects rather than fabricating a result for a zero secret key', async () => {
    await expect(
      wasmCommitteeCrypto.evaluateQuery(new Uint8Array(32), GROUND_TRUTH_BLINDED_QUERY),
    ).rejects.toThrow(/ERR_ZERO_SECRET_KEY/);
  });
});
