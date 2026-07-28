/**
 * On-device Groth16 ZK proving for Rarimo passport registration, plus the
 * IPFS-backed proving-key fetch/cache this needs (the Full-circuit
 * `circuit_final.zkey` is ~515MB — see HANDOFF.md item 8 for why Full mode
 * and IPFS distribution were chosen over a smaller "Light" circuit or a
 * bundled/server-hosted key).
 *
 * What's real here: `fetchZkAsset` (content-hash -> CIDv0 -> cached
 * download-to-file via RNFS, never loading the whole asset into JS memory)
 * and thin wrappers around the two real native modules
 * (`@iden3/react-native-circom-witnesscalc`'s `calculateWitness` and
 * `@iden3/react-native-rapidsnark`'s `groth16Prove`). Their actual shipped
 * TypeScript declarations were read directly out of node_modules to write
 * these wrappers (not assumed from README prose, which undersells a couple
 * of details — e.g. `calculateWitness` returns a base64 *string*, and
 * `groth16Prove` returns `{proof, pub_signals}` as JSON-encoded *strings*,
 * not parsed objects/arrays).
 *
 * What's NOT here, on purpose, per HANDOFF.md item 8 ("Real Rarimo passport
 * ZK flow (mobile)"):
 *  - NFC passport chip reading (BAC key derivation from the MRZ + APDU
 *    exchange). No confirmed off-the-shelf RN library exists for this;
 *    RegisterScreen.tsx still stubs it. Not attempted here.
 *  - The witness-calculator file-format mismatch: `passport-zk-circuits`'
 *    release bundles ship the classic circom C++ witness generator
 *    (`.cpp`/`.dat`), but `@iden3/react-native-circom-witnesscalc` expects
 *    the newer graph format (`.wcd`) from a different iden3 tool
 *    (`circom-witnesscalc`). No `.wcd` graph for any passport-zk-circuits
 *    variant is known to exist yet — one would need to be compiled from the
 *    circuit's `.circom` source using `circom-witnesscalc`'s own compiler
 *    tooling, or obtained some other way not yet determined. `computeWitness`
 *    below takes a `graphPath` argument for exactly this file and does not
 *    attempt to fabricate or derive it.
 *  - The snarkjs-JSON -> `verifier.rs` compact byte-layout (129-byte
 *    A/B/C-points-plus-variant-byte) conversion for the proof this module
 *    produces. `generateProof` below returns the raw parsed snarkjs shapes;
 *    turning that into the bytes `pallet-identity::register_citizen` expects
 *    is separate, not-yet-done work per HANDOFF item 8.
 *
 * None of this has been runtime-tested — no device/emulator is available in
 * this environment, and neither the 515MB proving key nor a `.wcd` graph
 * file is obtainable yet. Only `tsc --noEmit` has been checked.
 */
import { Buffer } from 'buffer';
import { hexToU8a } from '@polkadot/util';
import bs58 from 'bs58';
import RNFS from 'react-native-fs';
import { calculateWitness } from '@iden3/react-native-circom-witnesscalc';
import { groth16Prove } from '@iden3/react-native-rapidsnark';

/** Default public IPFS gateway — matches the desktop app's `fetch_ipfs_content` default (desktop/src-tauri/src/commands/chain.rs). */
export const DEFAULT_IPFS_GATEWAY = 'https://ipfs.io/ipfs';

/** Local cache directory for downloaded ZK assets (proving keys, witness graphs), keyed by CID. */
const ZK_ASSET_CACHE_DIR = `${RNFS.CachesDirectoryPath}/zk-assets`;

/**
 * Converts a 32-byte SHA-256 digest into a CIDv0 string: a base58-encoded
 * multihash, `[0x12 (sha2-256), 0x20 (32-byte length), ...digest]`. Direct
 * TypeScript port of `hash_to_cid` in
 * desktop/src-tauri/src/commands/chain.rs — same algorithm, same default
 * gateway convention, so a hash computed anywhere in this project resolves
 * to the same IPFS location from mobile and desktop alike.
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
 * Fetches a content-addressed ZK asset (proving key, witness graph, etc.)
 * by its 32-byte SHA-256 hash (hex, with or without a `0x` prefix),
 * caching it on disk by CID so repeat calls don't re-download. Returns the
 * local filesystem path to the cached file — callers pass this straight to
 * `computeWitness/generateProof`.
 *
 * Downloads stream directly to disk via RNFS's `downloadFile` rather than
 * an in-memory fetch, since the Full-circuit proving key is ~515MB and must
 * never be buffered whole in the JS heap.
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

  // Download to a temp path first so a crash/interruption mid-download can
  // never leave a corrupt file sitting at `cachePath` that a later call
  // would treat as a valid cache hit.
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

/**
 * Computes a circuit witness via `@iden3/react-native-circom-witnesscalc`.
 *
 * `graphPath` must point at a `.wcd` witness-calculator graph file (the
 * *newer* iden3 `circom-witnesscalc` format) already resolved on disk — e.g.
 * the path returned by `fetchZkAsset`. As of this writing no such file is
 * known to exist for any `passport-zk-circuits` variant: their release
 * bundles ship the classic circom C++ generator (`.cpp`/`.dat`) instead, and
 * nothing in this codebase converts between the two. See the module doc
 * comment and HANDOFF.md item 8 — this function is real and callable, but
 * unusable end-to-end until a `.wcd` graph for the target circuit is
 * obtained by some means not yet determined.
 */
export async function computeWitness(inputsJson: string, graphPath: string): Promise<Uint8Array> {
  const graphBase64 = await RNFS.readFile(graphPath, 'base64');
  // The native module returns the computed witness as a base64 string
  // (confirmed from its shipped index.d.ts + README, not assumed).
  const witnessBase64 = await calculateWitness(inputsJson, graphBase64);
  return new Uint8Array(Buffer.from(witnessBase64, 'base64'));
}

/** Groth16 proof + public signals, as produced by `generateProof` below. */
export interface Groth16ProveResult {
  /** Parsed snarkjs proof object (pi_a/pi_b/pi_c/protocol/curve). Not yet converted to `verifier.rs`'s compact byte layout — see module doc comment. */
  proof: unknown;
  /** Parsed public signals, as decimal-string field elements (snarkjs convention). */
  pub_signals: string[];
}

/**
 * Generates a Groth16 proof via `@iden3/react-native-rapidsnark`.
 *
 * `witnessBase64` is the base64-encoded witness bytes (e.g.
 * `Buffer.from(await computeWitness(...)).toString('base64')`, or a witness
 * read straight off disk with `RNFS.readFile(path, 'base64')`).
 *
 * The native module's actual return shape is `{proof: string, pub_signals:
 * string}` — both JSON-encoded strings, per its shipped `index.d.ts`, not
 * the already-parsed shapes some usage examples imply. This wrapper parses
 * both before returning so callers get real objects/arrays; no other
 * transformation is applied.
 */
export async function generateProof(
  zkeyPath: string,
  witnessBase64: string,
): Promise<Groth16ProveResult> {
  const { proof, pub_signals } = await groth16Prove(zkeyPath, witnessBase64);
  return {
    proof: JSON.parse(proof),
    pub_signals: JSON.parse(pub_signals),
  };
}
