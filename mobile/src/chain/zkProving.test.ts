/**
 * Tests the ZKPassport proving-pipeline orchestration against a fake `NoirProver`.
 *
 * No real prover exists (no Noir native module is built yet — see `zkProving.ts`'s module
 * doc), so what is testable is the orchestration itself: that the pipeline proves the
 * subproofs in the order the outer circuit consumes them, uses the recursion-friendly
 * target for subproofs and the EVM target for the outer proof, derives the right
 * `count_N` variant, and hands back an envelope `runtime/src/verifier.rs` will parse.
 *
 * Those are exactly the things that are easy to get quietly wrong and produce proofs that
 * fail with no useful diagnostic.
 */
import { decodeBackingNullifierProof } from './backingNullifierEncoding';
import { decodeUltraHonkProof } from './proofEncoding';
import {
  BackingNullifierProveRequest,
  BASE_SUBPROOF_CIRCUITS,
  NoirProver,
  NoirProverUnavailableError,
  outerCircuitFor,
  proveBackingNullifier,
  proveDelegatePersona,
  proveMigration,
  proveRegistration,
  proveReverification,
  ProvingTarget,
  setNoirProver,
  SubproofRequest,
} from './zkProving';

jest.mock('react-native-fs', () => ({
  CachesDirectoryPath: '/tmp/caches',
  exists: jest.fn(async () => true),
  mkdir: jest.fn(async () => undefined),
  moveFile: jest.fn(async () => undefined),
  unlink: jest.fn(async () => undefined),
  downloadFile: jest.fn(() => ({ promise: Promise.resolve({ statusCode: 200 }) })),
}));

/** A 32-byte content hash, hex, distinct per circuit so the fetch path is identifiable. */
function assetHash(seed: number): string {
  return seed.toString(16).padStart(2, '0').repeat(32);
}

function request(circuit: string, seed: number): SubproofRequest {
  return { circuit, bytecodeHash: assetHash(seed), inputs: { seed } };
}

interface ProveCall {
  bytecodePath: string;
  target: ProvingTarget;
}

/**
 * A prover that records what it was asked to do and returns well-formed dummy artifacts.
 * Proof lengths are multiples of 32 so the envelope encoder accepts them, as real bb
 * output would be.
 */
function fakeProver(): { prover: NoirProver; proveCalls: ProveCall[]; executeCalls: string[] } {
  const proveCalls: ProveCall[] = [];
  const executeCalls: string[] = [];
  const prover: NoirProver = {
    async execute(bytecodePath, _inputs) {
      executeCalls.push(bytecodePath);
      return new Uint8Array([1, 2, 3]);
    },
    async prove(bytecodePath, _witness, target) {
      proveCalls.push({ bytecodePath, target });
      const publicInputs = new Uint8Array(32 * 9); // count_4's 9 public inputs
      publicInputs[32 * 7 + 31] = 1; // non-zero scoped_nullifier
      return {
        proof: new Uint8Array(32 * 142).fill(0xcd),
        publicInputs,
        verificationKey: new Uint8Array(1888),
      };
    },
  };
  return { prover, proveCalls, executeCalls };
}

const baseRequests = [
  request('sig-check/dsc/tbs_1000/rsa/pkcs/2048/sha256', 1),
  request('sig-check/id-data/tbs_1000/rsa/pkcs/2048/sha256', 2),
  request('data-check/integrity/sa_sha256/dg_sha256', 3),
];

afterEach(() => setNoirProver(null));

describe('outerCircuitFor', () => {
  it('counts the three base subproofs alongside the disclosures', () => {
    expect(outerCircuitFor(1)).toEqual({ path: 'main/outer/count_4', outerCount: 4 });
    expect(outerCircuitFor(2)).toEqual({ path: 'main/outer/count_5', outerCount: 5 });
    expect(outerCircuitFor(10)).toEqual({ path: 'main/outer/count_13', outerCount: 13 });
  });

  it('rejects a disclosure count ZKPassport has no outer circuit for', () => {
    expect(() => outerCircuitFor(0)).toThrow(RangeError);
    expect(() => outerCircuitFor(11)).toThrow(RangeError);
  });
});

describe('proveRegistration', () => {
  it('fails loudly when no prover is registered', async () => {
    setNoirProver(null);
    await expect(
      proveRegistration(baseRequests, [request('disclose/bytes', 4)], {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(NoirProverUnavailableError);
  });

  it('proves subproofs recursively and the outer proof for the EVM', async () => {
    const { prover, proveCalls } = fakeProver();
    setNoirProver(prover);

    await proveRegistration(baseRequests, [request('disclose/bytes', 4)], {
      bytecodeHash: assetHash(9),
      inputs: {},
    });

    // 3 base + 1 disclosure + 1 outer.
    expect(proveCalls).toHaveLength(5);
    expect(proveCalls.slice(0, 4).map((c) => c.target)).toEqual([
      'noir-recursive',
      'noir-recursive',
      'noir-recursive',
      'noir-recursive',
    ]);
    // Only the outer proof is verified on-chain, so only it uses the keccak/EVM target.
    expect(proveCalls[4].target).toBe('evm');
  });

  it('produces an envelope the runtime verifier will parse', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    const result = await proveRegistration(baseRequests, [request('disclose/bytes', 4)], {
      bytecodeHash: assetHash(9),
      inputs: {},
    });

    expect(result.outerCount).toBe(4);
    const decoded = decodeUltraHonkProof(result.zkProof);
    expect(decoded.header).toEqual({ outerCount: 4, variant: 'zk', proofLength: 32 * 142 });
    expect(result.publicInputs).toHaveLength(9);
    expect(result.publicInputs.every((value) => value.length === 32)).toBe(true);
  });

  it('picks count_5 when two attributes are disclosed', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    const result = await proveRegistration(
      baseRequests,
      [request('disclose/bytes', 4), request('compare/age', 5)],
      { bytecodeHash: assetHash(9), inputs: {} },
    );
    expect(result.outerCount).toBe(5);
    expect(decodeUltraHonkProof(result.zkProof).header.outerCount).toBe(5);
  });

  it('rejects base subproofs given in the wrong order', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    const swapped = [baseRequests[1], baseRequests[0], baseRequests[2]];
    await expect(
      proveRegistration(swapped, [request('disclose/bytes', 4)], {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(/order matters/);
  });

  it('rejects a missing base subproof', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    await expect(
      proveRegistration(baseRequests.slice(0, 2), [request('disclose/bytes', 4)], {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(RangeError);
  });

  it('reports progress once per subproof plus once for the outer proof', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    const stages: string[] = [];
    await proveRegistration(
      baseRequests,
      [request('disclose/bytes', 4)],
      { bytecodeHash: assetHash(9), inputs: {} },
      { onProgress: (stage) => stages.push(stage) },
    );

    expect(stages).toEqual([
      'sig-check/dsc/tbs_1000/rsa/pkcs/2048/sha256',
      'sig-check/id-data/tbs_1000/rsa/pkcs/2048/sha256',
      'data-check/integrity/sa_sha256/dg_sha256',
      'disclose/bytes',
      'main/outer/count_4',
    ]);
  });
});

/**
 * `proveReverification`/`proveMigration` (HANDOFF log #76): `reverify_citizen`/
 * `migrate_oprf_scheme` now take the same outer `zk_proof`/`public_inputs` shape
 * `register_citizen` does, so these two are thin, differently-named wrappers over the exact
 * same orchestration `proveRegistration` already implements and the tests above already
 * cover in detail (subproof ordering, proving targets, envelope shape, count_N derivation).
 * These tests exist to pin that the wrappers really do delegate rather than silently
 * diverge, not to re-prove the underlying mechanics a second time.
 */
describe('proveReverification', () => {
  it('produces the identical outer-proof shape proveRegistration does', async () => {
    const { prover, proveCalls } = fakeProver();
    setNoirProver(prover);

    const result = await proveReverification(baseRequests, [request('disclosure/anchor', 4)], {
      bytecodeHash: assetHash(9),
      inputs: {},
    });

    expect(proveCalls).toHaveLength(5);
    expect(proveCalls[4].target).toBe('evm');
    expect(result.outerCount).toBe(4);
    expect(decodeUltraHonkProof(result.zkProof).header.outerCount).toBe(4);
    expect(result.publicInputs).toHaveLength(9);
  });

  it('fails loudly when no prover is registered, same as proveRegistration', async () => {
    setNoirProver(null);
    await expect(
      proveReverification(baseRequests, [request('disclosure/anchor', 4)], {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(NoirProverUnavailableError);
  });

  it('still enforces base-subproof ordering', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    const swapped = [baseRequests[1], baseRequests[0], baseRequests[2]];
    await expect(
      proveReverification(swapped, [request('disclosure/anchor', 4)], {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(/order matters/);
  });
});

describe('proveMigration', () => {
  it('produces the identical outer-proof shape proveRegistration does, over a migrate-disclosure subproof', async () => {
    const { prover, proveCalls } = fakeProver();
    setNoirProver(prover);

    const result = await proveMigration(
      baseRequests,
      [request('migrate-disclosure/bytes', 5)],
      { bytecodeHash: assetHash(9), inputs: {} },
    );

    expect(proveCalls).toHaveLength(5);
    expect(proveCalls.slice(0, 4).map((c) => c.target)).toEqual([
      'noir-recursive',
      'noir-recursive',
      'noir-recursive',
      'noir-recursive',
    ]);
    expect(proveCalls[4].target).toBe('evm');
    expect(result.outerCount).toBe(4);
    expect(decodeUltraHonkProof(result.zkProof).header.outerCount).toBe(4);
    expect(result.publicInputs).toHaveLength(9);
  });

  it('fails loudly when no prover is registered, same as proveRegistration', async () => {
    setNoirProver(null);
    await expect(
      proveMigration(baseRequests, [request('migrate-disclosure/bytes', 5)], {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(NoirProverUnavailableError);
  });

  it('reports progress the same way proveRegistration does', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    const stages: string[] = [];
    await proveMigration(
      baseRequests,
      [request('migrate-disclosure/bytes', 5)],
      { bytecodeHash: assetHash(9), inputs: {} },
      { onProgress: (stage) => stages.push(stage) },
    );

    expect(stages).toEqual([
      'sig-check/dsc/tbs_1000/rsa/pkcs/2048/sha256',
      'sig-check/id-data/tbs_1000/rsa/pkcs/2048/sha256',
      'data-check/integrity/sa_sha256/dg_sha256',
      'migrate-disclosure/bytes',
      'main/outer/count_4',
    ]);
  });
});

/**
 * `proveDelegatePersona` (commit 2e07f68's `delegate-persona` circuit): another thin wrapper
 * over `proveRegistration`'s exact mechanics, this time over exactly one `delegate-persona`
 * subproof — see `proveReverification`'s test block above for the precedent this mirrors.
 */
describe('proveDelegatePersona', () => {
  it('produces the identical outer-proof shape proveRegistration does, over a single delegate-persona subproof', async () => {
    const { prover, proveCalls } = fakeProver();
    setNoirProver(prover);

    const result = await proveDelegatePersona(
      baseRequests,
      request('delegate-persona/bytes', 6),
      { bytecodeHash: assetHash(9), inputs: {} },
    );

    expect(proveCalls).toHaveLength(5); // 3 base + 1 delegate-persona + 1 outer
    expect(proveCalls[4].target).toBe('evm');
    expect(result.outerCount).toBe(4);
    expect(decodeUltraHonkProof(result.zkProof).header.outerCount).toBe(4);
    expect(result.publicInputs).toHaveLength(9);
  });

  it('fails loudly when no prover is registered, same as proveRegistration', async () => {
    setNoirProver(null);
    await expect(
      proveDelegatePersona(baseRequests, request('delegate-persona/bytes', 6), {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(NoirProverUnavailableError);
  });

  it('still enforces base-subproof ordering', async () => {
    const { prover } = fakeProver();
    setNoirProver(prover);

    const swapped = [baseRequests[1], baseRequests[0], baseRequests[2]];
    await expect(
      proveDelegatePersona(swapped, request('delegate-persona/bytes', 6), {
        bytecodeHash: assetHash(9),
        inputs: {},
      }),
    ).rejects.toThrow(/order matters/);
  });
});

/**
 * `proveBackingNullifier` (`circuits/oprf-identity-anchor/backing-nullifier`): unlike every
 * function above, this circuit is standalone — no base subproofs, no outer circuit, a single
 * `execute`/`prove` round trip against the `evm` target directly (see that circuit's module
 * docs: verified the same way the outer ZKPassport proof itself is, a genuine standalone
 * pairing check, not folded into anything).
 */
describe('proveBackingNullifier', () => {
  function fakeBackingProver(): { prover: NoirProver; proveCalls: ProveCall[] } {
    const proveCalls: ProveCall[] = [];
    const prover: NoirProver = {
      async execute() {
        return new Uint8Array([9, 9, 9]);
      },
      async prove(bytecodePath, _witness, target) {
        proveCalls.push({ bytecodePath, target });
        const publicInputs = new Uint8Array(32 * 4); // root, delegate_persona_id, max_backings_per_citizen, backing_nullifier
        publicInputs[32 * 3 + 31] = 0xab; // a non-zero backing_nullifier
        return {
          proof: new Uint8Array(32 * 61).fill(0xef), // this circuit's real bb-reported size, per e31257a
          publicInputs,
          verificationKey: new Uint8Array(1888),
        };
      },
    };
    return { prover, proveCalls };
  }

  function backingRequest(): BackingNullifierProveRequest {
    return {
      root: 1n,
      delegatePersonaId: 2n,
      maxBackingsPerCitizen: 50n,
      backingRootSecret: 3n,
      slotIndex: 0n,
      leafIndex: 4n,
      merkleSiblings: Array.from({ length: 32 }, (_, i) => BigInt(i)),
      bytecodeHash: assetHash(11),
    };
  }

  it('fails loudly when no prover is registered', async () => {
    setNoirProver(null);
    await expect(proveBackingNullifier(backingRequest())).rejects.toThrow(NoirProverUnavailableError);
  });

  it('proves directly against the evm target — no recursive subproofs', async () => {
    const { prover, proveCalls } = fakeBackingProver();
    setNoirProver(prover);

    await proveBackingNullifier(backingRequest());

    expect(proveCalls).toHaveLength(1);
    expect(proveCalls[0].target).toBe('evm');
  });

  it('produces an envelope the backing-nullifier runtime verifier will parse, with 4 public inputs', async () => {
    const { prover } = fakeBackingProver();
    setNoirProver(prover);

    const result = await proveBackingNullifier(backingRequest());

    const decoded = decodeBackingNullifierProof(result.zkProof);
    expect(decoded.header).toEqual({ variant: 'zk', proofLength: 32 * 61 });
    expect(result.publicInputs).toHaveLength(4);
    expect(result.publicInputs.every((value) => value.length === 32)).toBe(true);
  });

  it('rejects a request with the wrong number of merkle siblings', async () => {
    const { prover } = fakeBackingProver();
    setNoirProver(prover);

    await expect(
      proveBackingNullifier({ ...backingRequest(), merkleSiblings: [1n, 2n] }),
    ).rejects.toThrow(/merkle siblings/);
  });

  it('reports progress once, for the single proving step', async () => {
    const { prover } = fakeBackingProver();
    setNoirProver(prover);

    const stages: string[] = [];
    await proveBackingNullifier(backingRequest(), { onProgress: (stage) => stages.push(stage) });

    expect(stages).toEqual(['backing-nullifier']);
  });
});

describe('BASE_SUBPROOF_CIRCUITS', () => {
  it('is the order the outer circuit takes them in', () => {
    // main/outer/count_N's signature: csc_to_dsc_proof, dsc_to_id_data_proof,
    // integrity_check_proof, disclosure_proofs.
    expect(BASE_SUBPROOF_CIRCUITS).toEqual([
      'sig-check/dsc',
      'sig-check/id-data',
      'data-check/integrity',
    ]);
  });
});
