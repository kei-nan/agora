/**
 * Builds our own Document Signer Certificate (DSC) Merkle tree — the thing
 * that becomes `certificate_registry_root` (registered via
 * `pallet_identity::add_allowed_merkle_root`) and, per certificate, a leaf +
 * inclusion proof mobile fetches at proving time. See
 * mobile/src/chain/certificateTree.ts's doc comment for the full "why our
 * own tree instead of a vendor-hosted registry" rationale (carries over
 * unchanged from the Rarimo era, per HANDOFF.md logs #63/#65/#66) and the
 * exact tree spec (depth-16, index-addressed, Poseidon2, per ZKPassport's
 * `common/src/lib.nr`).
 *
 * REPLACES the previous Rarimo-era version of this file (`@iden3/js-merkletree`,
 * depth-80 Poseidon1 SMT). Nothing about the old tree design carries over —
 * see certificateTree.ts's doc comment for why.
 *
 * Leaf/tree math (hashing, Merkle combine, inclusion verification) all
 * comes from `mobile/src/chain/certificateTree.ts` — this file does NOT
 * reimplement that logic a second time. What this file adds on top:
 * (1) parsing real DER/PEM X.509 certificates into the fields a
 * `CertificateTreeLeaf` needs (country, public key bytes, expiry,
 * fingerprint), using `@zkpassport/utils`' own first-party ASN.1 helpers
 * (the same ones ZKPassport's own `src/ts/test-helper.ts#convertPemToPackagedCertificateV1`
 * uses — this is a from-scratch reimplementation against those same public
 * exports, not a copy of that file, since it isn't published), and
 * (2) actually assembling a depth-16 indexed tree from N leaves (sorting,
 * building levels bottom-up, extracting a proof per leaf) — logic
 * `certificateTree.ts` deliberately doesn't need on a phone, which only
 * ever verifies one already-built proof, never constructs a tree.
 *
 * A real API gotcha this surfaced, worth recording so it isn't
 * rediscovered: `@zkpassport/utils/registry`'s own `getCertificateLeafHash`
 * reads the certificate type (CSCA=1 vs DSC=2) from its *options* argument,
 * not the certificate object, and its `buildMerkleTreeFromCerts` helper
 * never forwards a type at all (always defaults to CSCA) — see
 * `mobile/src/chain/certificateTree.test.ts`'s doc comment for the full
 * story. This file sidesteps both by using only the low-level ASN.1
 * extraction exports (`getCertificateIssuerCountry`, `getRSAInfo`, etc.)
 * plus this project's own `calculateCertificateLeafHash`/tree-building,
 * rather than either of those two higher-level entry points.
 *
 * ---
 *
 * Sourcing DSC certificates — the part of this that isn't just code — is
 * unchanged from the Rarimo era; see git history for the original writeup
 * (HANDOFF.md log #63): DSCs are normally distributed via ICAO's PKD (a
 * paid/state-membership service this project has no access to), so
 * `pallet_identity::AllowedMerkleRoots` is deliberately additive and
 * governance-gated for incremental, legislature-approved onboarding rather
 * than a one-shot bulk import.
 *
 * ---
 *
 * Usage:
 *   npm install
 *   npm run build-tree -- --certs-dir ./certs --out ./tree.json
 *
 * `--certs-dir` should contain one DSC certificate per file, PEM or DER
 * (.pem/.crt/.cer/.der — content sniffed, not extension-trusted). Output
 * JSON: `{ root: "0x...", certificates: [{ leafHash: "0x...", index, tags,
 * certType, country, publicKey: "0x...", expiry, fingerprint: "0x...",
 * siblings: ["0x...", ...16 entries] }] }`, all field-element values
 * 32-byte big-endian hex, matching `Fr::from_be_bytes_mod_order`
 * (runtime/src/verifier.rs) and `fieldElementToBytes32BE`
 * (mobile/src/chain/certificateTree.ts).
 */
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { AsnParser } from '@peculiar/asn1-schema';
import { Certificate as X509Certificate } from '@peculiar/asn1-x509';
import {
  getCertificateIssuerCountry,
  countryCodeAlpha2ToAlpha3,
  getRSAInfo,
  getECDSAInfo,
  getKeySize,
  OIDS_TO_PUBKEY_TYPE,
} from '@zkpassport/utils';
import { poseidon2Hash } from '@zkpassport/poseidon2';
import {
  CERT_TYPE_DSC,
  TREE_DEPTH,
  calculateCertificateLeafHash,
  computeCertificateFingerprint,
  computeMerkleRoot,
  computeZeroes,
  fieldElementToBytes32BE,
  verifyInclusion,
  type CertificateTreeLeaf,
} from '../../mobile/src/chain/certificateTree';

function toHex(x: bigint): string {
  return '0x' + Buffer.from(fieldElementToBytes32BE(x)).toString('hex');
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

/**
 * Parses a DER-encoded X.509 certificate into a `CertificateTreeLeaf` —
 * everything `calculateCertificateLeafHash` needs — using
 * `@zkpassport/utils`' own first-party ASN.1 extraction helpers (the same
 * building blocks `src/ts/test-helper.ts#convertPemToPackagedCertificateV1`
 * in zkpassport/circuits uses; not copied from there since that file isn't
 * published, reimplemented here against the same public exports).
 * `tags` defaults to `[0n, 0n, 0n]` (untagged) — see
 * `tagsArrayToBitsFlag`/`@zkpassport/utils/registry` if a future onboarding
 * process needs real tags (e.g. jurisdiction/environment flags).
 */
export function certificateDerToLeaf(der: Uint8Array): CertificateTreeLeaf {
  const x509 = AsnParser.parse(Buffer.from(der), X509Certificate);

  const countryAlpha2 = getCertificateIssuerCountry(x509);
  if (!countryAlpha2 || countryAlpha2.length !== 2) {
    throw new Error(`certificateDerToLeaf: invalid or missing issuer country code: ${countryAlpha2}`);
  }
  const country = countryCodeAlpha2ToAlpha3(countryAlpha2);

  const notAfter = Math.floor(x509.tbsCertificate.validity.notAfter.getTime().getTime() / 1000);

  const publicKeyOID = x509.tbsCertificate.subjectPublicKeyInfo.algorithm.algorithm;
  const publicKeyType = (OIDS_TO_PUBKEY_TYPE as Record<string, string>)[publicKeyOID] ?? publicKeyOID;

  let publicKey: Uint8Array;
  if (publicKeyType === 'rsaEncryption' || publicKeyType === 'rsassa-pss') {
    const rsa = getRSAInfo(x509.tbsCertificate.subjectPublicKeyInfo);
    const hex = rsa.modulus.toString(16);
    const padded = hex.length % 2 === 0 ? hex : '0' + hex;
    publicKey = new Uint8Array(Buffer.from(padded, 'hex'));
  } else if (publicKeyType === 'ecPublicKey') {
    const ec = getECDSAInfo(x509.tbsCertificate.subjectPublicKeyInfo);
    const half = ec.publicKey.length / 2;
    // Strip the uncompressed-point 0x04 prefix, matching test-helper.ts's convention.
    publicKey = new Uint8Array([...ec.publicKey.slice(1, half + 1), ...ec.publicKey.slice(half + 1)]);
  } else {
    throw new Error(`certificateDerToLeaf: unsupported public key type: ${publicKeyType}`);
  }

  return {
    tags: [0n, 0n, 0n],
    certType: CERT_TYPE_DSC,
    country,
    publicKey,
    expiry: notAfter,
    fingerprint: computeCertificateFingerprint(der),
  };
}

export interface CertificateTreeOutput {
  root: string;
  certificates: Array<{
    leafHash: string;
    index: number;
    country: string;
    publicKey: string;
    expiry: number;
    fingerprint: string;
    siblings: string[];
  }>;
}

/**
 * Builds a depth-16, index-addressed, Poseidon2 tree from a list of leaf
 * hashes — sorted ascending, then assembled bottom-up exactly the way
 * `@zkpassport/utils`' own tree class does (confirmed via
 * `mobile/src/chain/certificateTree.test.ts`'s cross-checked vectors).
 * Returns per-leaf sibling paths (indexed by the leaf's position in the
 * SORTED order, not insertion order) plus the root.
 */
function buildIndexedTree(leafHashes: bigint[], depth: number): { root: bigint; siblingsByIndex: bigint[][] } {
  const maxLeaves = 2 ** depth;
  if (leafHashes.length > maxLeaves) {
    throw new Error(`buildIndexedTree: cannot fit ${leafHashes.length} leaves in a depth-${depth} tree (max ${maxLeaves})`);
  }
  // zeroes[i] = the "empty subtree" hash at level i (0 at the leaf level,
  // Poseidon2(zeroes[i-1], zeroes[i-1]) above that) — see
  // certificateTree.ts's computeZeroes doc comment.
  const zeroes = computeZeroes(depth);
  const sorted = [...leafHashes].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));

  if (sorted.length === 0) {
    return { root: poseidon2Hash([zeroes[depth - 1], zeroes[depth - 1]]), siblingsByIndex: [] };
  }

  const nodes: bigint[][] = [sorted];
  for (let level = 0; level < depth; level++) {
    const current = nodes[level];
    const next: bigint[] = [];
    for (let n = 0; n < Math.ceil(current.length / 2); n++) {
      const left = current[n * 2] ?? zeroes[level];
      const right = current[n * 2 + 1] ?? zeroes[level];
      next.push(poseidon2Hash([left, right]));
    }
    nodes.push(next);
  }
  const root = nodes[depth][0];

  const siblingsByIndex: bigint[][] = sorted.map((_, leafIndex) => {
    const siblings: bigint[] = [];
    let i = leafIndex;
    for (let level = 0; level < depth; level++) {
      const siblingIndex = i % 2 === 0 ? i + 1 : i - 1;
      siblings.push(nodes[level][siblingIndex] ?? zeroes[level]);
      i = Math.floor(i / 2);
    }
    return siblings;
  });

  return { root, siblingsByIndex };
}

/** Exported for tests; `main()` below is the CLI entry point. */
export function buildCertificateTree(certDers: Uint8Array[]): CertificateTreeOutput {
  const leaves = certDers.map(certificateDerToLeaf);
  const leafHashes = leaves.map(calculateCertificateLeafHash);

  const { root, siblingsByIndex } = buildIndexedTree(leafHashes, TREE_DEPTH);
  if (leaves.length === 0) {
    return { root: toHex(root), certificates: [] };
  }

  // Sorted order determines index assignment — reproduce it to line leaves back up with their proofs.
  const order = leaves
    .map((leaf, originalIndex) => ({ leaf, hash: leafHashes[originalIndex] }))
    .sort((a, b) => (a.hash < b.hash ? -1 : a.hash > b.hash ? 1 : 0));

  const certificates = order.map(({ leaf, hash }, index) => {
    const siblings = siblingsByIndex[index];
    // Self-check with the same verifier mobile uses, before trusting this output — a bug here would otherwise only surface on-device, mid-registration, for a real citizen.
    if (!verifyInclusion(root, leaf, index, siblings)) {
      throw new Error(`buildCertificateTree: generated proof for leaf ${toHex(hash)} does not verify against the tree's own root — do not publish this output`);
    }
    return {
      leafHash: toHex(hash),
      index,
      country: leaf.country,
      publicKey: '0x' + Buffer.from(leaf.publicKey).toString('hex'),
      expiry: leaf.expiry,
      fingerprint: toHex(leaf.fingerprint),
      siblings: siblings.map(toHex),
    };
  });

  return { root: toHex(root), certificates };
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

function main(): void {
  const { certsDir, out } = parseArgs(process.argv.slice(2));
  const files = readdirSync(certsDir).filter((f) => !f.startsWith('.'));
  if (files.length === 0) {
    throw new Error(`no certificate files found in ${certsDir}`);
  }
  const certDers = files.map((f) => certFileToDer(readFileSync(join(certsDir, f))));
  const result = buildCertificateTree(certDers);
  writeFileSync(out, JSON.stringify(result, null, 2));
  console.log(`wrote ${result.certificates.length} certificate(s), root ${result.root}, to ${out}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}
