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
import {
  buildOprfInput,
  buildRound1Input,
  buildRound2Input,
  evaluateQueryWithSeed,
  round1WithSeed,
  round2ResponseWithSeed,
  wasmCommitteeCrypto,
} from './wasmCommitteeCrypto';

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

/** A fixture satisfying the full `CommitteeCrypto` interface, for tests that only care
 * about one method — jest.fn() stubs for the rest so TypeScript is satisfied without
 * every test needing to restate the whole shape. */
function fixtureCrypto(overrides: Partial<CommitteeCrypto> = {}): CommitteeCrypto {
  return {
    evaluateQuery: jest.fn(),
    round1: jest.fn(),
    round2Response: jest.fn(),
    ...overrides,
  };
}

describe('notImplementedCommitteeCrypto', () => {
  it('rejects evaluateQuery with a "not implemented" error rather than a fabricated result', async () => {
    await expect(
      notImplementedCommitteeCrypto.evaluateQuery(new Uint8Array(32), new Uint8Array(64)),
    ).rejects.toThrow(/not implemented/);
  });

  it('rejects round1 with a "not implemented" error rather than a fabricated result', async () => {
    await expect(
      notImplementedCommitteeCrypto.round1(new Uint8Array(32), new Uint8Array(64), new Uint8Array(32)),
    ).rejects.toThrow(/not implemented/);
  });

  it('rejects round2Response with a "not implemented" error rather than a fabricated result', async () => {
    await expect(
      notImplementedCommitteeCrypto.round2Response(
        new Uint8Array(32),
        new Uint8Array(32),
        new Uint8Array(32),
        new Uint8Array(32),
        new Uint8Array(32),
      ),
    ).rejects.toThrow(/not implemented/);
  });
});

describe('getCommitteeCrypto / setCommitteeCrypto / resetCommitteeCrypto', () => {
  it('defaults to notImplementedCommitteeCrypto', () => {
    expect(getCommitteeCrypto()).toBe(notImplementedCommitteeCrypto);
  });

  it('returns whatever implementation was installed via setCommitteeCrypto', () => {
    const fixture = fixtureCrypto({
      evaluateQuery: jest.fn().mockResolvedValue({
        pk: new Uint8Array(64),
        evaluation: new Uint8Array(64),
        dlogProof: new Uint8Array(64),
      }),
    });
    setCommitteeCrypto(fixture);
    expect(getCommitteeCrypto()).toBe(fixture);
  });

  it('resetCommitteeCrypto restores the default stub', () => {
    setCommitteeCrypto(fixtureCrypto());
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

// ── Threshold round 1 / round 2 ────────────────────────────────────────────────────
//
// Mirrors `committee-node/src/wasm_host.rs`'s own round1/round2 test suite: these prove
// this file's byte marshaling against the real, compiled `oprf-committee-dev` wasm2js
// artifact, not the threshold cryptography itself (which is proven once, natively, by
// `oprf-committee-dev/src/ffi.rs`'s own
// `round1_then_round2_ffi_calls_produce_a_verifiable_combined_proof`). `rho_i`/
// `lambda_i`/`e` are opaque pass-through bytes as far as this ABI is concerned, so
// arbitrary fixture values (matching `ffi.rs`'s own round-2 tests) are enough to pin
// the marshaling without needing this app's own threshold-aggregation math (which
// doesn't exist — see `CommitteeCrypto.ts`'s module doc).

/** The BabyJubJub base point, big-endian `x` then `y` — copied byte-for-byte from
 * `committee-node/src/wasm_host.rs`'s identical `GENERATOR_XY` fixture, itself from
 * `oprf-committee-dev/src/babyjubjub.rs::Point::generator`. The one genuinely valid,
 * on-curve, in-subgroup point available without doing curve math in this file. */
const GENERATOR_XY = new Uint8Array([
  // x = 5299619240641551281634865583518297030282874472190772894086521144482721001553
  0x0b, 0xb7, 0x7a, 0x6a, 0xd6, 0x3e, 0x73, 0x9b, 0x4e, 0xac, 0xb2, 0xe0, 0x9d, 0x62, 0x77, 0xc1,
  0x2a, 0xb8, 0xd8, 0x01, 0x05, 0x34, 0xe0, 0xb6, 0x28, 0x93, 0xf3, 0xf6, 0xbb, 0x95, 0x70, 0x51,
  // y = 16950150798460657717958625567821834550301663161624707787222815936182638968203
  0x25, 0x79, 0x72, 0x03, 0xf7, 0xa0, 0xb2, 0x49, 0x25, 0x57, 0x2e, 0x1c, 0xd1, 0x6b, 0xf9, 0xed,
  0xfc, 0xe0, 0x05, 0x1f, 0xb9, 0xe1, 0x33, 0x77, 0x4b, 0x3c, 0x25, 0x7a, 0x87, 0x2d, 0x7d, 0x8b,
]);

/** A nonzero 32-byte secret share — copied byte-for-byte from `wasm_host.rs`'s `SHARE`
 * fixture (`committee-node/src/wasm_host.rs`, in its threshold round1/round2 tests):
 * 27 zero bytes followed by `2e 3b 9a c9 f1`. Built via `Array(27).fill(0)` rather than
 * spelled out zero-by-zero, so the byte count is provably right rather than
 * eyeballed. */
const SHARE = new Uint8Array([...new Array(27).fill(0), 0x2e, 0x3b, 0x9a, 0xc9, 0xf1]);

describe('round1WithSeed / round2ResponseWithSeed (real crypto core)', () => {
  it('produces real, non-stub, deterministic output for a given seed', () => {
    const seed = new Uint8Array(32).fill(0x5a);
    const c1 = round1WithSeed(SHARE, GENERATOR_XY, seed);

    for (const [name, p] of [
      ['rI', c1.rI],
      ['dG', c1.dG],
      ['dQ', c1.dQ],
      ['eG', c1.eG],
      ['eQ', c1.eQ],
    ] as const) {
      expect(p).toHaveLength(64);
      expect(p.some((b) => b !== 0)).toBe(true) /* not all-zero */;
    }

    // Determinism: same inputs, same seed, byte-identical output.
    const repeat = round1WithSeed(SHARE, GENERATOR_XY, seed);
    expect(bytesToHex(repeat.rI)).toBe(bytesToHex(c1.rI));
    expect(bytesToHex(repeat.dG)).toBe(bytesToHex(c1.dG));
    expect(bytesToHex(repeat.eQ)).toBe(bytesToHex(c1.eQ));

    // Seed-sensitivity split (see `wasm_host.rs`'s identical assertion and its
    // reasoning): r_i = s_i * b_q does not involve the nonces, so it must be
    // unaffected by the seed, while every nonce commitment must change with it.
    const other = round1WithSeed(SHARE, GENERATOR_XY, new Uint8Array(32).fill(0xa5));
    expect(bytesToHex(other.rI)).toBe(bytesToHex(c1.rI));
    expect(bytesToHex(other.dG)).not.toBe(bytesToHex(c1.dG));
    expect(bytesToHex(other.eQ)).not.toBe(bytesToHex(c1.eQ));
  });

  it('rejects a zero secret share with the real module\'s ERR_ZERO_SECRET_KEY', () => {
    expect(() => round1WithSeed(new Uint8Array(32), GENERATOR_XY, new Uint8Array(32).fill(1))).toThrow(
      /ERR_ZERO_SECRET_KEY/,
    );
  });

  it('rejects an off-curve blindedQuery with the real module\'s ERR_BLINDED_QUERY_NOT_ON_CURVE', () => {
    const offCurve = new Uint8Array(64);
    offCurve[31] = 0x01;
    offCurve[63] = 0x01;
    expect(() => round1WithSeed(SHARE, offCurve, new Uint8Array(32).fill(1))).toThrow(
      /ERR_BLINDED_QUERY_NOT_ON_CURVE/,
    );
  });

  it('round2ResponseWithSeed is deterministic and sensitive to every public input', () => {
    const seed = new Uint8Array(32).fill(0x5a);
    const rhoI = new Uint8Array(32).fill(0x11);
    const lambdaI = new Uint8Array(32).fill(0x22);
    const e = new Uint8Array(32).fill(0x33);

    const r2 = round2ResponseWithSeed(SHARE, seed, rhoI, lambdaI, e);
    expect(r2.zI).toHaveLength(32);
    expect(r2.zI.some((b) => b !== 0)).toBe(true);

    const again = round2ResponseWithSeed(SHARE, seed, rhoI, lambdaI, e);
    expect(bytesToHex(again.zI)).toBe(bytesToHex(r2.zI));

    const changedSeed = round2ResponseWithSeed(SHARE, new Uint8Array(32).fill(0xa5), rhoI, lambdaI, e);
    expect(bytesToHex(changedSeed.zI)).not.toBe(bytesToHex(r2.zI));
    const changedRho = round2ResponseWithSeed(SHARE, seed, new Uint8Array(32).fill(0x44), lambdaI, e);
    expect(bytesToHex(changedRho.zI)).not.toBe(bytesToHex(r2.zI));
    const changedLambda = round2ResponseWithSeed(SHARE, seed, rhoI, new Uint8Array(32).fill(0x44), e);
    expect(bytesToHex(changedLambda.zI)).not.toBe(bytesToHex(r2.zI));
    const changedE = round2ResponseWithSeed(SHARE, seed, rhoI, lambdaI, new Uint8Array(32).fill(0x44));
    expect(bytesToHex(changedE.zI)).not.toBe(bytesToHex(r2.zI));
  });

  it('rejects a zero secret share for round2Response too', () => {
    expect(() =>
      round2ResponseWithSeed(
        new Uint8Array(32),
        new Uint8Array(32).fill(1),
        new Uint8Array(32).fill(2),
        new Uint8Array(32).fill(3),
        new Uint8Array(32).fill(4),
      ),
    ).toThrow(/ERR_ZERO_SECRET_KEY/);
  });
});

describe('buildRound1Input / buildRound2Input (marshaling only)', () => {
  it('buildRound1Input rejects a blindedQuery that is not exactly 64 bytes', () => {
    expect(() => buildRound1Input(SHARE, new Uint8Array(63), new Uint8Array(32))).toThrow(/64/);
  });

  it('buildRound1Input rejects a seed that is not exactly 32 bytes', () => {
    expect(() => buildRound1Input(SHARE, GENERATOR_XY, new Uint8Array(31))).toThrow(/32/);
  });

  it('buildRound2Input rejects a wrong-length public input', () => {
    const ok32 = new Uint8Array(32);
    expect(() => buildRound2Input(SHARE, new Uint8Array(31), ok32, ok32, ok32)).toThrow(/seed/);
    expect(() => buildRound2Input(SHARE, ok32, new Uint8Array(31), ok32, ok32)).toThrow(/rhoI/);
    expect(() => buildRound2Input(SHARE, ok32, ok32, new Uint8Array(31), ok32)).toThrow(/lambdaI/);
    expect(() => buildRound2Input(SHARE, ok32, ok32, ok32, new Uint8Array(31))).toThrow(/e /);
  });
});

describe('wasmCommitteeCrypto.round1 / round2Response (public async interface)', () => {
  it('round1 takes the given seed as-is (no internal randomness) and matches round1WithSeed', async () => {
    const seed = new Uint8Array(32).fill(0x5a);
    const viaInterface = await wasmCommitteeCrypto.round1(SHARE, GENERATOR_XY, seed);
    const direct = round1WithSeed(SHARE, GENERATOR_XY, seed);
    expect(bytesToHex(viaInterface.rI)).toBe(bytesToHex(direct.rI));
    expect(bytesToHex(viaInterface.dG)).toBe(bytesToHex(direct.dG));
  });

  it('round2Response matches round2ResponseWithSeed', async () => {
    const seed = new Uint8Array(32).fill(0x5a);
    const rhoI = new Uint8Array(32).fill(0x11);
    const lambdaI = new Uint8Array(32).fill(0x22);
    const e = new Uint8Array(32).fill(0x33);
    const viaInterface = await wasmCommitteeCrypto.round2Response(SHARE, seed, rhoI, lambdaI, e);
    const direct = round2ResponseWithSeed(SHARE, seed, rhoI, lambdaI, e);
    expect(bytesToHex(viaInterface.zI)).toBe(bytesToHex(direct.zI));
  });
});
