/**
 * On-device Noir/UltraHonk proving for ZKPassport passport registration, plus the
 * content-addressed circuit-artifact fetch/cache it needs.
 *
 * Replaces the Rarimo-era Groth16 flow. That module wrapped
 * `@iden3/react-native-circom-witnesscalc` (`calculateWitness`) and
 * `@iden3/react-native-rapidsnark` (`groth16Prove`) — circom/snarkjs tooling with no
 * bearing on Noir. Both imports are gone. See `docs/project/changelog/065-068.md`
 * entries 65/66.
 *
 * # What's real here
 *
 * `hashToCid` and `fetchZkAsset` are unchanged and were never vendor-specific: they map a
 * 32-byte content hash to a CIDv0 and stream the asset to disk via RNFS, never buffering
 * it in the JS heap. They now fetch Noir ACIR artifacts rather than a 515MB circom proving
 * key, which is a much smaller job, but the mechanism is the same one the desktop app uses
 * (`hash_to_cid` in `desktop/src-tauri/src/commands/chain.rs`).
 *
 * The pipeline orchestration below is real and unit-tested against a fake prover. What it
 * orchestrates is not: **no Noir prover native module exists in this project yet.**
 *
 * # Why there is a `NoirProver` seam instead of a direct native import
 *
 * `@zkpassport/sdk` does not fill this slot — it is relying-party orchestration software
 * (show a QR code, the user scans it with ZKPassport's own app, get results by callback),
 * not an on-device proving SDK for someone else's app (changelog entry 66).
 * `zkpassport/noir_rs` is the right low-level building block and is actively maintained,
 * but ships no JNI/Swift/RN bridging of its own; wiring it up is the same order of custom
 * native work the old rapidsnark integration was. Until that exists, this module defines
 * the contract it must satisfy and fails loudly when nothing is registered, rather than
 * pretending to prove.
 *
 * # ZKPassport's pipeline, and why the proving modes differ per stage
 *
 * ZKPassport is a modular multi-circuit pipeline, not one monolithic circuit:
 *
 *  1. `sig-check/dsc`          — the country signing cert signed this document signer cert
 *  2. `sig-check/id-data`      — that document signer cert signed this passport
 *  3. `data-check/integrity`   — the datagroup hashes are internally consistent
 *  4. one or more `disclose`/`compare`/`inclusion-check`/`exclusion-check` circuits
 *  5. `main/outer/count_N`     — recursively verifies all of the above into one proof
 *
 * Stages 1-4 are proved for *in-circuit* verification, so they need Barretenberg's
 * recursion-friendly configuration (`noir-recursive`: poseidon2 transcript). The outer
 * proof of stage 5 is the one that lands on-chain, so it needs the keccak/EVM
 * configuration (`evm`) the runtime verifier is built around. Getting this backwards
 * produces proofs that fail with no useful error, so the two are separate types below
 * rather than a boolean.
 *
 * `N = 3 + (number of disclosure subproofs)`, which is why {@link outerCircuitFor} derives
 * it rather than taking it as a parameter.
 */
import { hexToU8a } from '@polkadot/util';
import bs58 from 'bs58';
import RNFS from 'react-native-fs';
import { encodeUltraHonkProof, splitPublicInputs } from './proofEncoding';

/** Default public IPFS gateway — matches the desktop app's `fetch_ipfs_content` default (desktop/src-tauri/src/commands/chain.rs). */
export const DEFAULT_IPFS_GATEWAY = 'https://ipfs.io/ipfs';

/** Local cache directory for downloaded ZK assets (circuit bytecode, verification keys), keyed by CID. */
const ZK_ASSET_CACHE_DIR = `${RNFS.CachesDirectoryPath}/zk-assets`;

/**
 * Converts a 32-byte SHA-256 digest into a CIDv0 string: a base58-encoded multihash,
 * `[0x12 (sha2-256), 0x20 (32-byte length), ...digest]`. Direct TypeScript port of
 * `hash_to_cid` in desktop/src-tauri/src/commands/chain.rs — same algorithm, same default
 * gateway convention, so a hash computed anywhere in this project resolves to the same
 * IPFS location from mobile and desktop alike.
 */
export function hashToCid(digest: Uint8Array): string {
  if (digest.length !== 32) {
    throw new Error(`hashToCid: expected a 32-byte digest, got ${digest.length} bytes`);
  }
  const multihash = new Uint8Array(34);
  multihash[0] = 0x12; // sha2-256 function code
  multihash[1] = 0x20; // digest length = 32
  multihash.set(digest, 2);
  return bs58.encode(multihash);
}

/**
 * Fetches a content-addressed ZK asset (circuit bytecode, verification key, etc.) by its
 * 32-byte SHA-256 hash (hex, with or without a `0x` prefix), caching it on disk by CID so
 * repeat calls don't re-download. Returns the local filesystem path — callers pass this
 * straight to the prover.
 *
 * Downloads stream directly to disk via RNFS's `downloadFile` rather than an in-memory
 * fetch. That mattered enormously for the Rarimo-era 515MB proving key and matters less
 * for Noir ACIR artifacts, but a phone is still a phone and the pattern costs nothing.
 */
export async function fetchZkAsset(
  hashHex: string,
  gatewayUrl: string = DEFAULT_IPFS_GATEWAY,
): Promise<string> {
  const digest = hexToU8a(hashHex.startsWith('0x') ? hashHex : `0x${hashHex}`);
  const cid = hashToCid(digest);
  const cachePath = `${ZK_ASSET_CACHE_DIR}/${cid}`;

  if (await RNFS.exists(cachePath)) {
    return cachePath;
  }

  if (!(await RNFS.exists(ZK_ASSET_CACHE_DIR))) {
    await RNFS.mkdir(ZK_ASSET_CACHE_DIR);
  }

  // Download to a temp path first so a crash/interruption mid-download can never leave a
  // corrupt file sitting at `cachePath` that a later call would treat as a cache hit.
  const tmpPath = `${cachePath}.part`;
  const { promise } = RNFS.downloadFile({
    fromUrl: `${gatewayUrl}/${cid}`,
    toFile: tmpPath,
  });
  const result = await promise;
  if (result.statusCode < 200 || result.statusCode >= 300) {
    await RNFS.unlink(tmpPath).catch(() => undefined);
    throw new Error(`fetchZkAsset: gateway returned status ${result.statusCode} for ${cid}`);
  }
  await RNFS.moveFile(tmpPath, cachePath);
  return cachePath;
}

// ---------------------------------------------------------------------------
// The Noir prover seam
// ---------------------------------------------------------------------------

/**
 * Barretenberg proving configuration.
 *
 * `noir-recursive` (poseidon2 transcript) for a proof that will be verified *inside*
 * another circuit; `evm` (keccak transcript) for the outer proof that goes on-chain.
 * These are `bb`'s own `-t` target names, kept verbatim so the mapping to the toolchain
 * is not something anyone has to guess.
 */
export type ProvingTarget = 'noir-recursive' | 'evm';

/** What a proving run returns, mirroring the files `bb prove` writes. */
export interface NoirProof {
  /** The raw proof bytes — a flat array of 32-byte big-endian words. */
  proof: Uint8Array;
  /** The circuit's public inputs, concatenated. Split with `splitPublicInputs`. */
  publicInputs: Uint8Array;
  /** The circuit's verification key. Needed to feed a subproof into the outer circuit. */
  verificationKey: Uint8Array;
}

/**
 * The native-module contract a Noir prover must satisfy. Nothing in this project
 * implements it yet — see the module doc comment for why, and what the candidates are
 * (`zkpassport/noir_rs` plus `Swoir` / `noir_android` / `madztheo/noir-react-native-starter`
 * bridging).
 *
 * Deliberately narrow: witness generation and proving, nothing else. Circuit artifacts
 * arrive as filesystem paths (from {@link fetchZkAsset}) rather than buffers so the
 * implementation can memory-map them.
 */
export interface NoirProver {
  /**
   * Solves the circuit's witness from its inputs.
   * @param bytecodePath path to the compiled ACIR artifact (nargo's `<circuit>.json`)
   * @param inputs the circuit's named inputs (nargo's `Prover.toml` fields, as JSON)
   */
  execute(bytecodePath: string, inputs: Record<string, unknown>): Promise<Uint8Array>;
  /** Proves a solved witness under the given target configuration. */
  prove(bytecodePath: string, witness: Uint8Array, target: ProvingTarget): Promise<NoirProof>;
}

/** Thrown when proving is attempted with no prover registered. */
export class NoirProverUnavailableError extends Error {
  constructor() {
    super(
      'No Noir prover is registered. On-device ZKPassport proving needs a native module ' +
        '(zkpassport/noir_rs plus Swoir/noir_android bridging) that this project has not built yet; ' +
        'call setNoirProver() with an implementation, or with a fake in tests.',
    );
    this.name = 'NoirProverUnavailableError';
  }
}

let activeProver: NoirProver | null = null;

/** Registers the prover implementation. Pass `null` to unregister (used by tests). */
export function setNoirProver(prover: NoirProver | null): void {
  activeProver = prover;
}

/** Returns the registered prover, or throws {@link NoirProverUnavailableError}. */
export function getNoirProver(): NoirProver {
  if (activeProver === null) {
    throw new NoirProverUnavailableError();
  }
  return activeProver;
}

// ---------------------------------------------------------------------------
// The ZKPassport proving pipeline
// ---------------------------------------------------------------------------

/**
 * The three subproofs every ZKPassport registration needs, regardless of what is being
 * disclosed. Order matters: it is the order `main/outer/count_N` takes them in
 * (`csc_to_dsc_proof`, `dsc_to_id_data_proof`, `integrity_check_proof`), and the
 * commitment chain runs through them in exactly this sequence.
 */
export const BASE_SUBPROOF_CIRCUITS = [
  'sig-check/dsc',
  'sig-check/id-data',
  'data-check/integrity',
] as const;

/** One circuit to prove: which circuit, where to get it, and what to feed it. */
export interface SubproofRequest {
  /** Circuit path, e.g. `sig-check/id-data/tbs_1000/rsa/pkcs/2048/sha256`. Identification only. */
  circuit: string;
  /** 32-byte SHA-256 content hash (hex) of the compiled ACIR artifact, for {@link fetchZkAsset}. */
  bytecodeHash: string;
  /** The circuit's named inputs. */
  inputs: Record<string, unknown>;
}

/** A proved subproof, in the shape the outer circuit consumes. */
export interface SubproofResult {
  circuit: string;
  proof: Uint8Array;
  /** Split into 32-byte field elements — the outer circuit indexes these positionally. */
  publicInputs: Uint8Array[];
  verificationKey: Uint8Array;
}

/** Options shared by every step of the pipeline. */
export interface ProvingOptions {
  gatewayUrl?: string;
  /** Called after each subproof, for progress UI. Proving a passport takes a while. */
  onProgress?: (stage: string, completed: number, total: number) => void;
}

/**
 * Proves one subproof, for in-circuit verification by the outer circuit.
 *
 * Always uses the `noir-recursive` target: the outer circuit verifies these with
 * `verify_proof_with_type(..., PROOF_TYPE_HONK_ZK)`, which expects the poseidon2
 * transcript. An `evm`-target subproof would be rejected by the outer circuit with no
 * useful diagnostic, so the target is not a caller-supplied option here.
 */
export async function proveSubcircuit(
  request: SubproofRequest,
  options: ProvingOptions = {},
): Promise<SubproofResult> {
  const prover = getNoirProver();
  const bytecodePath = await fetchZkAsset(request.bytecodeHash, options.gatewayUrl);
  const witness = await prover.execute(bytecodePath, request.inputs);
  const proved = await prover.prove(bytecodePath, witness, 'noir-recursive');
  return {
    circuit: request.circuit,
    proof: proved.proof,
    publicInputs: splitPublicInputs(proved.publicInputs),
    verificationKey: proved.verificationKey,
  };
}

/**
 * The `main/outer/count_N` circuit path for a given number of disclosure subproofs.
 *
 * `N` counts *all* wrapped subproofs — the 3 base ones plus the disclosures — which is
 * why this derives it rather than trusting a caller's arithmetic. ZKPassport ships
 * `count_4` through `count_13`, i.e. 1 to 10 disclosure subproofs.
 */
export function outerCircuitFor(disclosureCount: number): { path: string; outerCount: number } {
  if (!Number.isInteger(disclosureCount) || disclosureCount < 1 || disclosureCount > 10) {
    throw new RangeError(
      `outerCircuitFor: ZKPassport's outer circuits wrap 1..10 disclosure subproofs, got ${disclosureCount}`,
    );
  }
  const outerCount = BASE_SUBPROOF_CIRCUITS.length + disclosureCount;
  return { path: `main/outer/count_${outerCount}`, outerCount };
}

/** Everything the outer circuit needs beyond the subproofs themselves. */
export interface OuterProofInputs {
  /** 32-byte SHA-256 content hash (hex) of the compiled `main/outer/count_N` artifact. */
  bytecodeHash: string;
  /**
   * The outer circuit's own named inputs — `certificate_registry_root`,
   * `circuit_registry_root`, `current_date`, `service_scope`, `service_subscope`,
   * `param_commitments`, `nullifier_type`, `scoped_nullifier`, `oprf_pk_hash`, plus each
   * subproof's Merkle path into the circuit registry. The subproof
   * proof/vkey/public-input fields are filled in by {@link proveRegistration}.
   */
  inputs: Record<string, unknown>;
}

/**
 * The finished artifact: exactly the `zk_proof`/`public_inputs` pair
 * `pallet_identity_zk::register_citizen` takes.
 *
 * HANDOFF log #76 restructured `reverify_citizen`/`migrate_oprf_scheme` to take this same
 * shape (previously they took only a bare, unauthenticated proof-bytes blob) — so this type,
 * despite the name, is also what {@link proveReverification}/{@link proveMigration} below
 * return. Kept as one name rather than three near-identical interfaces since the shape truly
 * is identical; `mobile/src/chain/identity.ts` defines its own `OuterProofPayload` with the
 * same fields for callers that don't want a dependency on this module.
 */
export interface RegistrationProof {
  /** `zk_proof` — the envelope `runtime/src/verifier.rs` parses. */
  zkProof: Uint8Array;
  /** `public_inputs` — 32-byte big-endian field elements in the circuit's own order. */
  publicInputs: Uint8Array[];
  /** Which outer circuit produced it, for diagnostics. */
  outerCount: number;
}

/**
 * Runs the whole pipeline: every subproof, then the outer proof, then the envelope.
 *
 * @param baseSubproofs the three {@link BASE_SUBPROOF_CIRCUITS} requests, in that order
 * @param disclosureSubproofs one or more disclosure-circuit requests
 * @param outer the outer circuit's artifact hash and its own inputs
 *
 * The outer proof is the only one proved with the `evm` target, because it is the only
 * one that gets verified on-chain.
 */
export async function proveRegistration(
  baseSubproofs: readonly SubproofRequest[],
  disclosureSubproofs: readonly SubproofRequest[],
  outer: OuterProofInputs,
  options: ProvingOptions = {},
): Promise<RegistrationProof> {
  if (baseSubproofs.length !== BASE_SUBPROOF_CIRCUITS.length) {
    throw new RangeError(
      `proveRegistration: expected ${BASE_SUBPROOF_CIRCUITS.length} base subproofs ` +
        `(${BASE_SUBPROOF_CIRCUITS.join(', ')}), got ${baseSubproofs.length}`,
    );
  }
  baseSubproofs.forEach((request, index) => {
    const expected = BASE_SUBPROOF_CIRCUITS[index];
    if (!request.circuit.startsWith(expected)) {
      throw new Error(
        `proveRegistration: base subproof ${index} is "${request.circuit}", expected a "${expected}" circuit — ` +
          'the outer circuit takes these positionally, so order matters',
      );
    }
  });

  const { outerCount } = outerCircuitFor(disclosureSubproofs.length);

  const prover = getNoirProver();
  const requests = [...baseSubproofs, ...disclosureSubproofs];
  const results: SubproofResult[] = [];
  for (const request of requests) {
    results.push(await proveSubcircuit(request, options));
    options.onProgress?.(request.circuit, results.length, requests.length + 1);
  }

  const outerBytecodePath = await fetchZkAsset(outer.bytecodeHash, options.gatewayUrl);
  const outerWitness = await prover.execute(outerBytecodePath, {
    ...outer.inputs,
    csc_to_dsc_proof: toOuterSubproofInput(results[0]),
    dsc_to_id_data_proof: toOuterSubproofInput(results[1]),
    integrity_check_proof: toOuterSubproofInput(results[2]),
    disclosure_proofs: results.slice(3).map(toOuterSubproofInput),
  });
  const outerProof = await prover.prove(outerBytecodePath, outerWitness, 'evm');
  options.onProgress?.(`main/outer/count_${outerCount}`, requests.length + 1, requests.length + 1);

  return {
    zkProof: encodeUltraHonkProof(outerProof.proof, { outerCount, variant: 'zk' }),
    publicInputs: splitPublicInputs(outerProof.publicInputs),
    outerCount,
  };
}

/**
 * Produces the outer ZKPassport proof for periodic re-verification (`reverify_citizen`,
 * call index 6).
 *
 * HANDOFF log #76 found reverification needs no new circuit at all: it rides the identical
 * `disclosure` subproof shape registration already uses (recompute the anchor from the
 * fresh proof, confirm it still matches the one on file), so there is no separate proving
 * pipeline here — this is `proveRegistration`'s exact mechanics under a name that matches
 * the call site it feeds. `disclosureSubproofs` should be `disclosure` circuit requests,
 * exactly as for registration.
 */
export async function proveReverification(
  baseSubproofs: readonly SubproofRequest[],
  disclosureSubproofs: readonly SubproofRequest[],
  outer: OuterProofInputs,
  options: ProvingOptions = {},
): Promise<RegistrationProof> {
  return proveRegistration(baseSubproofs, disclosureSubproofs, outer, options);
}

/**
 * Produces the outer ZKPassport proof for OPRF scheme migration (`migrate_oprf_scheme`,
 * call index 7).
 *
 * Unlike reverification, log #76 found migration genuinely needed a new circuit —
 * `circuits/oprf-identity-anchor/migrate-disclosure` — because the standalone `migrate`
 * circuit shared the same unauthenticated-`comm_in` flaw `disclosure` was built to close for
 * `anchor`. `migrate-disclosure` folds both the old and new anchor/scheme-version/committee-
 * hash sets into one 15-element `param_commitment` (tag `201`) plus an expiry check
 * `migrate` never had, but it still exposes the same fixed 8-field outer-circuit disclosure-
 * subproof interface `disclosure` does (empirically confirmed in changelog entry 77) — so the
 * outer-proof assembly below is mechanically identical to registration's; only the *content*
 * of `disclosureSubproofs` differs. Pass `migrate-disclosure` circuit requests here instead of
 * `disclosure` ones; everything else (base subproofs, outer witness assembly, envelope
 * encoding) is unchanged.
 */
export async function proveMigration(
  baseSubproofs: readonly SubproofRequest[],
  disclosureSubproofs: readonly SubproofRequest[],
  outer: OuterProofInputs,
  options: ProvingOptions = {},
): Promise<RegistrationProof> {
  return proveRegistration(baseSubproofs, disclosureSubproofs, outer, options);
}

/**
 * Shapes a subproof for the outer circuit's `CSCtoDSCProof`/`DSCtoIDDataProof`/
 * `IntegrityCheckProof`/`DisclosureProof` struct fields.
 *
 * `key_hash`, `tree_index` and `tree_hash_path` are deliberately absent: they are the
 * subproof's position in the on-chain circuit-registry Merkle tree, which is chain
 * configuration rather than proving output, and belong in `OuterProofInputs.inputs`.
 * Filling them in with placeholders here would hide a missing input behind a proof that
 * simply fails.
 */
function toOuterSubproofInput(result: SubproofResult): Record<string, unknown> {
  return {
    proof: result.proof,
    vkey: result.verificationKey,
    public_inputs: result.publicInputs,
  };
}
