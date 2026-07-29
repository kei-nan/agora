/**
 * Builds our own Document Signer Certificate (DSC) Merkle tree — the thing
 * that becomes `slaveMerkleRoot` (registered via
 * `pallet_identity::add_allowed_merkle_root`) and, per certificate, a
 * `slaveMerkleInclusionBranches` proof mobile fetches at proving time. See
 * mobile/src/chain/certificateTree.ts's doc comment for the full "why our
 * own tree instead of Rarimo's hosted CertificatesSMT" rationale and the
 * exact tree spec (depth-80 Poseidon SMT, iden3-style).
 *
 * Uses `@iden3/js-merkletree` (iden3's own production SMT implementation —
 * the same tree design used across Polygon ID, verified byte-for-byte
 * against the circuit's SMTHash1/SMTHash2 formulas — see this repo's own
 * `leafKey`/`NodeMiddle.getKey` source) rather than a hand-rolled tree
 * engine. It genuinely does branch per-bit along a shared key prefix (a
 * real `NodeMiddle` can have one Empty side, hashed as literal 0, while
 * the two keys routed there haven't diverged yet) — an earlier hand-rolled
 * version of this tool assumed that was wrong and tried to "path-compress"
 * away those levels, which produced a *different*, incompatible root. The
 * actual fix needed was on the *reading* side, not the tree structure:
 * `mobile/src/chain/certificateTree.ts`'s `verifyInclusion` must find the
 * real/padding boundary as "one past the deepest nonzero sibling," not
 * "first zero seen" — a real per-bit tree can have a legitimate,
 * non-padding zero sibling partway down. Multi-certificate trees exposed
 * this; a single-certificate tree (the realistic near-term case — see
 * below) happened to work under either (wrong or right) assumption, which
 * is why it's worth recording rather than re-discovering.
 *
 * ---
 *
 * Sourcing DSC certificates — the part of this that isn't just code:
 *
 * The tree's leaves are Document Signer Certificates (DSCs) — the
 * certificate that directly signs a passport's SOD — NOT CSCA root
 * certificates. ICAO's publicly downloadable "Master List" is CSCA roots
 * only; complete DSC lists are normally distributed through ICAO's PKD,
 * which is a paid/state-membership service, not an open download. This
 * project has no PKD membership and isn't a state, so there is no "just
 * download everything" path to a complete tree.
 *
 * `pallet_identity::AllowedMerkleRoots` is already governance-gated
 * (`AdminOrigin`/legislature) for exactly this kind of situation: the
 * intended model is INCREMENTAL, governance-approved onboarding, not a
 * one-shot bulk import —
 *
 *   1. Start from whatever DSC/CSCA data can legitimately be sourced in
 *      the open (national PKI publication pages many countries maintain,
 *      open mirrors, citizens submitting their own passport's DSC cert for
 *      review — a DSC is not secret, it's embedded in every passport
 *      signed with it).
 *   2. Before adding a DSC to the tree, verify it chains up to a
 *      recognized CSCA (CSCA roots ARE freely available via the ICAO
 *      Master List, so this step doesn't have the same access problem).
 *   3. Add it via a legislature-approved call to `add_allowed_merkle_root`
 *      with the new tree root this tool computes — this file only builds
 *      the tree offline; submitting the root on-chain is a separate,
 *      deliberate governance action, not something this tool does itself.
 *   4. Rebuild and re-register the root each time the certificate set
 *      changes. Existing citizens already registered under an old root are
 *      unaffected (`AllowedMerkleRoots` is additive — old roots can be left
 *      valid, or retired separately via `remove_allowed_merkle_root`).
 *
 * This tool takes a directory of certificates as a deliberate on-ramp for
 * that process, not a claim that the input directory is complete or
 * authoritative — verifying what goes into that directory is a governance
 * and PKI-chain-validation problem, not something this script can decide.
 *
 * ---
 *
 * Usage:
 *   npm install
 *   npm run build-tree -- --certs-dir ./certs --out ./tree.json
 *
 * `--certs-dir` should contain one DSC certificate per file, PEM or DER
 * (.pem/.crt/.cer/.der — content sniffed, not extension-trusted). Output
 * JSON: `{ root: "0x...", certificates: [{ pubkeyHash: "0x...", siblings:
 * ["0x...", ...80 entries] }] }`, all values 32-byte big-endian hex,
 * matching `Fr::from_be_bytes_mod_order` (runtime/src/verifier.rs) and
 * `fieldElementToBytes32BE` (mobile/src/chain/certificateTree.ts).
 */
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { Merkletree, InMemoryDB, str2Bytes, circomSiblingsFromSiblings } from '@iden3/js-merkletree';
import { computeDscPubkeyHash, extractPubkeyFromCertificate, fieldElementToBytes32BE, verifyInclusion } from '../../mobile/src/chain/certificateTree';

const TREE_DEPTH = 80;

function toHex(bytes: Uint8Array): string {
  return '0x' + Buffer.from(bytes).toString('hex');
}

/** Strips PEM armor if present; passes DER through unchanged. Sniffed by content, not file extension. */
function certFileToDer(raw: Buffer): Uint8Array {
  const text = raw.toString('utf8');
  if (text.includes('-----BEGIN CERTIFICATE-----')) {
    const base64 = text
      .split(/-----BEGIN CERTIFICATE-----|-----END CERTIFICATE-----/)[1]
      .replace(/\s+/g, '');
    return new Uint8Array(Buffer.from(base64, 'base64'));
  }
  return new Uint8Array(raw);
}

export interface CertificateTreeOutput {
  root: string;
  certificates: Array<{ pubkeyHash: string; siblings: string[] }>;
}

/** Builds the tree from a list of DER-encoded certificates and produces the on-disk output shape. Exported for tests; `main()` below is the CLI entry point. */
export async function buildCertificateTree(certDers: Uint8Array[]): Promise<CertificateTreeOutput> {
  const db = new InMemoryDB(str2Bytes('certificate-registry'));
  const tree = new Merkletree(db, true, TREE_DEPTH);

  const pubkeyHashes: bigint[] = [];
  for (const der of certDers) {
    const pubkey = extractPubkeyFromCertificate(der);
    const pubkeyHash = computeDscPubkeyHash(pubkey);
    await tree.add(pubkeyHash, pubkeyHash);
    pubkeyHashes.push(pubkeyHash);
  }

  const root = (await tree.root()).bigInt();
  const certificates = [];
  for (const pubkeyHash of pubkeyHashes) {
    const { proof, value } = await tree.generateProof(pubkeyHash);
    if (!proof.existence || value !== pubkeyHash) {
      throw new Error(`buildCertificateTree: internal inconsistency — just-inserted certificate ${pubkeyHash} not found on lookup`);
    }
    // allSiblings() returns only the real (unpadded) depth reached — pad to
    // the circuit's fixed 80 levels the same way the circuit's own
    // convention requires (see mobile/src/chain/certificateTree.ts's doc
    // comment: zero beyond the leaf's real depth, never combined).
    const siblings = circomSiblingsFromSiblings(proof.allSiblings(), TREE_DEPTH).map((s) => s.bigInt());
    if (siblings.length !== TREE_DEPTH) {
      throw new Error(`buildCertificateTree: expected ${TREE_DEPTH} siblings, got ${siblings.length}`);
    }
    // Self-check with the same verifier mobile uses, before trusting this output — a bug here would otherwise only surface on-device, mid-registration, for a real citizen.
    if (!verifyInclusion(root, pubkeyHash, pubkeyHash, siblings)) {
      throw new Error(`buildCertificateTree: generated proof for ${pubkeyHash} does not verify against the tree's own root — do not publish this output`);
    }
    certificates.push({ pubkeyHash: toHex(fieldElementToBytes32BE(pubkeyHash)), siblings: siblings.map((s) => toHex(fieldElementToBytes32BE(s))) });
  }

  return { root: toHex(fieldElementToBytes32BE(root)), certificates };
}

function parseArgs(argv: string[]): { certsDir: string; out: string } {
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(flag);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const certsDir = get('--certs-dir');
  const out = get('--out');
  if (!certsDir || !out) {
    throw new Error('usage: build-tree --certs-dir <dir> --out <file.json>');
  }
  return { certsDir, out };
}

async function main(): Promise<void> {
  const { certsDir, out } = parseArgs(process.argv.slice(2));
  const files = readdirSync(certsDir).filter((f) => !f.startsWith('.'));
  if (files.length === 0) {
    throw new Error(`no certificate files found in ${certsDir}`);
  }
  const certDers = files.map((f) => certFileToDer(readFileSync(join(certsDir, f))));
  const result = await buildCertificateTree(certDers);
  await import('node:fs').then((fs) => fs.writeFileSync(out, JSON.stringify(result, null, 2)));
  console.log(`wrote ${result.certificates.length} certificate(s), root ${result.root}, to ${out}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((err) => {
    console.error(err);
    process.exitCode = 1;
  });
}
