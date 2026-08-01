/**
 * Assembles ZKPassport circuit inputs from a passport's raw DG1 / DG15 / SOD bytes (as
 * read by ../native/nfcPassportReader.ts).
 *
 * # What changed when Rarimo was dropped, and what deliberately did not
 *
 * This file began as a TypeScript port of rarimo/passport-zk-circuits'
 * `test/process_passport.js`. When the project replatformed onto ZKPassport (see
 * `docs/project/changelog/065-068.md` entry 65), the question was whether it needed a
 * rewrite. It did not, and the reason is worth stating: **the extraction was never
 * Rarimo-specific.** Everything that walks the SOD — the ASN.1 tree helpers, the
 * encapsulated-content and signed-attributes extraction, the RSA/ECDSA public key and
 * signature extraction, DG15 Active Authentication parsing, the datagroup-hash offset
 * search — reads the ICAO CMS SignedData structure the *passport* defines, which is the
 * same structure whichever circuit vendor consumes it afterwards. All of it is unchanged,
 * including the three disclosed DEPARTUREs from the reference script (search "DEPARTURE").
 *
 * What was genuinely Rarimo-specific was the *encoding* of the result, and that is
 * entirely replaced:
 *
 *  - `padBits` (SHA Merkle-Damgard padding flattened into a bit array of '0'/'1'
 *    strings) -> `padToFixedLength`. ZKPassport's Noir circuits take fixed-length **byte**
 *    arrays and do their own padding in-circuit.
 *  - `bigintToArrayString` (little-endian 64/66-bit limbs as decimal strings, circom's
 *    `bigIntFunc.circom` convention) -> `bigintToBeBytes`. `noir-bignum` takes plain
 *    big-endian byte arrays.
 *  - `getSigType` (Rarimo's hand-maintained SIGNATURE_TYPE integer table) ->
 *    `classifySignatureAlgorithm`, which returns a ZKPassport circuit *path*, because
 *    ZKPassport ships a directory tree of small circuits rather than one monolithic
 *    `registerIdentity_*` per parameter combination.
 *  - `extractTbsCertificate` is new. ZKPassport's `sig-check` circuits take the Document
 *    Signer Certificate's raw TBSCertificate DER and authenticate the DSC public key
 *    against it in-circuit; Rarimo's circuit trusted that key unauthenticated.
 *
 * # What this module still does NOT produce, and why
 *
 * These are real, unstarted work rather than oversights, and `ParsedPassport.unresolved`
 * names each one so a caller cannot quietly forget it:
 *
 *  - **The CSC (country signing) certificate and its Barrett-reduction parameters**, which
 *    `sig-check/dsc` needs to check that the DSC was genuinely signed by a trusted country
 *    root. That comes from the certificate registry, not from the passport chip.
 *    `certificateTree.ts` and `scripts/certificate-registry/` already build that registry
 *    (changelog entry 66), but nothing wires the two together yet.
 *  - **The commitment-chain salts** each subproof consumes (`salt_in`/`salt_out`). These
 *    must be freshly random per proof and generated on-device — deriving them from
 *    passport bytes, as the Rarimo reference script did for `skIdentity`, would make them
 *    not secret at all.
 *  - **`service_scope` / `service_subscope`**, which are chain-level constants, not
 *    passport data.
 *
 * ZKPassport also has no Active Authentication circuit today, so DG15 is parsed
 * (`ParsedPassport.activeAuthentication`) but unused — kept because the parsing is correct
 * and re-deriving it later would be wasted work.
 */
import { Buffer } from 'buffer';
import { sha256, sha384, sha512 } from '@noble/hashes/sha2';
import { sha1 } from '@noble/hashes/legacy';
import { Hex, Base64, decoded, type Asn1Node } from './asn1';

// ---------------------------------------------------------------------------
// Raw byte / hex helpers
// ---------------------------------------------------------------------------

function bytesToHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString('hex');
}

function hexToBytes(hex: string): Uint8Array {
  return new Uint8Array(Buffer.from(hex.replace(/\s+/g, ''), 'hex'));
}

/**
 * Decodes an ASN.1 DER blob (as raw bytes) into asn1.js's simplified tree.
 * asn1.js's own `decoded()` only accepts hex/base64 strings, so this always
 * feeds it hex — avoids the ambiguity of asn1.js guessing base64 vs. hex
 * from string content (`Hex`/`Base64.unarmor` are still exported from
 * ./asn1 for anyone who has an armored string instead of raw bytes).
 */
export function decodeAsn1(bytes: Uint8Array): Asn1Node {
  return decoded(bytesToHex(bytes));
}

/**
 * DEPARTURE from the original: `process_passport.js` reads an OCTET_STRING's
 * raw value via asn1.js's `node.content()`, which — only for OCTET_STRING —
 * first *tries to decode the bytes as printable UTF-8 text* and returns that
 * instead of hex if it succeeds (see asn1.js's `parseOctetString`, the
 * `parseStringUTF` + `checkPrintable` try/catch). For DER-encoded binary
 * data (hash digests, signature bytes) this essentially never triggers, but
 * "essentially never" isn't a property worth inheriting silently in
 * identity-registration code. This helper gets the same bytes a different,
 * unambiguous way: `node.dump` is always a raw hex dump of the full
 * tag+length+value TLV (asn1.js's `toHexString()`, no text-preview branch —
 * used as-is elsewhere in this file, e.g. `signedAttrsHex` below), so this
 * just strips the DER tag+length header off `dump` per standard BER/DER
 * length-encoding rules and returns the value bytes' hex.
 */
function octetStringValueHex(node: Asn1Node): string {
  const dump = node.dump;
  // Tag: single byte for every tag this codebase's SOD parsing ever hits
  // (OCTET_STRING is a low-tag-number universal tag, always one byte: 0x04).
  let pos = 2; // 1 tag byte = 2 hex chars
  const firstLenByte = parseInt(dump.slice(pos, pos + 2), 16);
  pos += 2;
  let valueLen: number;
  if ((firstLenByte & 0x80) === 0) {
    // Short form: the byte itself is the length.
    valueLen = firstLenByte;
  } else {
    // Long form: low 7 bits = number of subsequent big-endian length bytes.
    const numLenBytes = firstLenByte & 0x7f;
    const lenHex = dump.slice(pos, pos + numLenBytes * 2);
    valueLen = parseInt(lenHex, 16);
    pos += numLenBytes * 2;
  }
  const valueHex = dump.slice(pos, pos + valueLen * 2);
  if (valueHex.length !== valueLen * 2) {
    throw new Error(
      `octetStringValueHex: expected ${valueLen} value bytes, dump only has ${valueHex.length / 2} left ` +
        `(node.dump may be truncated or this wasn't an OCTET_STRING)`,
    );
  }
  return valueHex;
}

// ---------------------------------------------------------------------------
// Hashing (mirrors process_passport.js's computeHash)
// ---------------------------------------------------------------------------

/** Digest byte-length -> algorithm, matching the circuit's DG_HASH_TYPE / hash-type params. */
export type HashOutLen = 20 | 28 | 32 | 48 | 64;

export function computeHash(outLen: HashOutLen, input: Uint8Array): Uint8Array {
  switch (outLen) {
    case 20:
      return sha1(input);
    case 32:
      return sha256(input);
    case 48:
      return sha384(input);
    case 64:
      return sha512(input);
    case 28:
      throw new Error('computeHash: SHA-224 not wired up (no passport encountered needing it yet — see @noble/hashes/sha2\'s sha224 if one turns up)');
    default:
      throw new Error(`computeHash: unsupported digest length ${outLen} bytes`);
  }
}

// ---------------------------------------------------------------------------
// Fixed-length byte packing (ZKPassport's Noir circuits take bytes, not bits)
// ---------------------------------------------------------------------------

/**
 * REPLACES the Rarimo-era `padBits`. That function produced SHA Merkle-Damgard
 * padding flattened into an array of '0'/'1' *strings* — the circom convention, where
 * the circuit hashes a pre-padded, pre-measured bit array with a fixed block count.
 *
 * ZKPassport's Noir circuits do neither. They take raw fixed-length **byte** arrays
 * (`DG1Data = [u8; 95]`, `EContentData = [u8; 700]`, `SignedAttrsData = [u8; 256]` —
 * `src/noir/lib/utils/src/types.nr`) and recover the real length in-circuit by parsing
 * the ASN.1 header (`unsafe_get_asn1_element_length`), then do their own SHA padding
 * (`sha256_and_check_data_to_sign(signed_attributes, signed_attributes_size)`).
 *
 * So the mobile side's job is much simpler than it was: right-pad with zeros to the
 * circuit's fixed array size. Nothing else.
 */

/** `DG1_LENGTH` — `src/noir/lib/utils/src/constants.nr`. Sized for the longer ID-card MRZ; passports use 93 of the 95 bytes. */
export const DG1_LENGTH = 95;

/** `ECONTENT_LENGTH` — `src/noir/lib/utils/src/constants.nr`. */
export const ECONTENT_LENGTH = 700;

/** `SIGNED_ATTRS_LENGTH` — `src/noir/lib/utils/src/constants.nr`. */
export const SIGNED_ATTRS_LENGTH = 256;

/**
 * Right-pads `bytes` with zeros to exactly `length`, the way a Noir `[u8; length]`
 * parameter needs. Throws rather than truncating if the data is already too long — a
 * silently truncated e-content would produce a proof that simply never verifies, with
 * nothing to point at.
 */
export function padToFixedLength(bytes: Uint8Array, length: number, what: string): Uint8Array {
  if (bytes.length > length) {
    throw new RangeError(
      `padToFixedLength: ${what} is ${bytes.length} bytes, longer than the circuit's ${length}-byte array`,
    );
  }
  const out = new Uint8Array(length);
  out.set(bytes, 0);
  return out;
}

// ---------------------------------------------------------------------------
// Big-integer -> fixed-width big-endian bytes
// ---------------------------------------------------------------------------

/**
 * REPLACES the Rarimo-era `bigintToArrayString`, which split a big integer into
 * little-endian 64/66-bit limbs as decimal strings (circom's `bigIntFunc.circom`
 * representation).
 *
 * ZKPassport uses `noir-bignum`, whose RSA/ECDSA circuits take plain **big-endian byte
 * arrays** of a fixed width (`csc_pubkey: [u8; 384]`, `sod_signature: [u8; 128]`,
 * `dsc_pubkey_x: [u8; 64]`). No limb splitting, no decimal strings.
 */
export function bigintToBeBytes(x: bigint, byteLength: number): Uint8Array {
  if (x < 0n) {
    throw new RangeError(`bigintToBeBytes: value must be non-negative, got ${x}`);
  }
  const out = new Uint8Array(byteLength);
  let rem = x;
  for (let i = byteLength - 1; i >= 0; i--) {
    out[i] = Number(rem & 0xffn);
    rem >>= 8n;
  }
  if (rem !== 0n) {
    throw new RangeError(`bigintToBeBytes: value does not fit in ${byteLength} bytes`);
  }
  return out;
}

/**
 * Same, from a hex string. Left-pads to `byteLength`, which matters: `BigInt(...)`
 * round-trips in this file's ancestry have lost leading zero bytes before (see
 * `certificateTree.ts`'s notes on the same hazard), and a pubkey missing its leading
 * zero byte is a pubkey the circuit will reject.
 */
export function hexToBeBytes(hex: string, byteLength: number): Uint8Array {
  return bigintToBeBytes(BigInt('0x' + hex), byteLength);
}


// ---------------------------------------------------------------------------
// Public key / signature shapes extracted from the SOD
// ---------------------------------------------------------------------------

export interface RsaPubkey {
  kind: 'rsa';
  /** Modulus, hex (no 0x prefix). */
  n: string;
  /** Public exponent, hex. */
  exp: string;
}

export interface EcdsaPubkey {
  kind: 'ecdsa';
  x: string;
  y: string;
  /** Curve identifier: hex field-prime for the curves getSigType recognizes by prime, or a curve name string. */
  param: string;
}

export type Pubkey = RsaPubkey | EcdsaPubkey;

export interface RsaSignature {
  kind: 'rsa';
  /** Raw signature bytes, hex. */
  n: string;
  /** PSS salt length as a decimal string if RSASSA-PSS params were found; the number `0` if this is plain RSA (falsy, matching getSigType's truthy check). */
  salt: string | 0;
}

export interface EcdsaSignature {
  kind: 'ecdsa';
  r: string;
  s: string;
}

export type Signature = RsaSignature | EcdsaSignature;

// ---------------------------------------------------------------------------
// ASN.1 tree navigation (mirrors process_passport.js's extract_*/get_* helpers)
//
// These walk the SOD's CMS SignedData structure by shape, not by OID
// lookup, exactly as the reference implementation does — ported faithfully,
// including which child index means what, since this is what the circuit's
// shift/param computation below was built and tested against. Each is
// commented with the CMS/LDSSecurityObject field it's actually locating.
// ---------------------------------------------------------------------------

/** DFS for the first OCTET_STRING node — locates SignedData.encapContentInfo.eContent (the LDSSecurityObject bytes). */
function getFirstOctetString(node: Asn1Node): Asn1Node | null {
  if (node.name === 'OCTET_STRING') return node;
  if (node.sub) {
    for (const child of node.sub) {
      const found = getFirstOctetString(child);
      if (found) return found;
    }
  }
  return null;
}

/** DFS for the last OCTET_STRING node + its parent — locates SignerInfo.signature (the raw signature bytes are the last OCTET_STRING encountered in the SOD). */
function findParentOfLastOctetString(
  node: Asn1Node,
  parent: Asn1Node | null = null,
): [Asn1Node, Asn1Node | null] | null {
  let result: [Asn1Node, Asn1Node | null] | null = null;
  if (node.name === 'OCTET_STRING') {
    result = [node, parent];
  }
  if (node.sub) {
    for (const child of node.sub) {
      const childResult = findParentOfLastOctetString(child, node);
      if (childResult) result = childResult;
    }
  }
  return result;
}

/**
 * DFS for a `[0]` context-tagged node whose last child looks like
 * `Attribute ::= SEQUENCE { type OBJECT_IDENTIFIER, values SET OF x }`
 * with exactly one OCTET_STRING value — locates the `messageDigest`
 * authenticated attribute inside SignerInfo.signedAttrs. Matched by shape
 * only (no OID check), same as the original — a crude but working
 * heuristic this file inherits rather than re-derives.
 */
function getZero(node: Asn1Node): Asn1Node | null {
  if (node.name === '[0]' && node.sub && node.sub.length > 0) {
    const last = node.sub[node.sub.length - 1];
    if (last.name === 'SEQUENCE' && last.content === '(2 elem)' && last.sub) {
      const [oid, set] = last.sub;
      if (oid?.name === 'OBJECT_IDENTIFIER' && set?.name === 'SET' && set.sub?.length === 1 && set.sub[0].name === 'OCTET_STRING') {
        return node;
      }
    }
  }
  if (node.sub) {
    for (const child of node.sub) {
      const found = getZero(child);
      if (found) return found;
    }
  }
  return null;
}

/** DFS for the BIT_STRING holding an uncompressed EC point (SEC1 `04 || x || y`) inside a SubjectPublicKeyInfo. */
export function getEcdsaKeyLocation(node: Asn1Node): Asn1Node | null {
  if (node.sub && node.sub.length >= 2) {
    const second = node.sub[1];
    if (second.name === 'BIT_STRING' && second.content?.startsWith('00000100')) {
      return node;
    }
  }
  if (node.sub) {
    for (const child of node.sub) {
      const found = getEcdsaKeyLocation(child);
      if (found) return found;
    }
  }
  return null;
}

/** DFS for the BIT_STRING wrapping an RSA `SEQUENCE { modulus INTEGER, exponent INTEGER }`. */
export function getRsaKeyLocation(node: Asn1Node): Asn1Node | null {
  if (node.name === 'BIT_STRING' && node.sub) {
    for (const child of node.sub) {
      if (child.name === 'SEQUENCE' && child.sub?.length === 2 && child.sub[0].name === 'INTEGER' && child.sub[1].name === 'INTEGER') {
        return node;
      }
    }
  }
  if (node.sub) {
    for (const child of node.sub) {
      const found = getRsaKeyLocation(child);
      if (found) return found;
    }
  }
  return null;
}

function extractEncapsulatedContent(sod: Asn1Node): { ecHex: string; dgHashType: number } {
  const ec = getFirstOctetString(sod);
  if (!ec || !ec.sub) throw new Error('extractEncapsulatedContent: no encapsulated content OCTET_STRING found in SOD');
  // ec.sub[0] = LDSSecurityObject SEQUENCE; .sub[2] = dataGroupHashValues;
  // .sub[0] = first DataGroupHash SEQUENCE; .sub[1] = its digest OCTET_STRING.
  const dgHashType = ec.sub[0]?.sub?.[2]?.sub?.[0]?.sub?.[1]?.length;
  if (dgHashType === undefined) throw new Error('extractEncapsulatedContent: LDSSecurityObject shape mismatch — could not find a dataGroupHashValue');
  return { ecHex: octetStringValueHex(ec), dgHashType };
}

function extractSignedAttributes(sod: Asn1Node): { saHex: string; hashType: number } {
  const sa = getZero(sod);
  if (!sa || !sa.sub) throw new Error('extractSignedAttributes: no signedAttrs [0] SET found in SOD');
  const lastAttr = sa.sub[sa.sub.length - 1];
  const hashType = lastAttr?.sub?.[lastAttr.sub.length - 1]?.sub?.[0]?.length;
  if (hashType === undefined) throw new Error('extractSignedAttributes: messageDigest attribute shape mismatch');
  // CMS signedAttrs is `[0] IMPLICIT SET OF Attribute` in the wire encoding,
  // but must be re-tagged as an EXPLICIT SET (DER tag 0x31) before hashing
  // for signature verification, per RFC 5652 §5.4. Both are single-byte
  // tags, so swapping just the first hex-encoded byte is exact.
  const saHex = '31' + sa.dump.slice(2);
  return { saHex, hashType };
}

function extractSignature(sod: Asn1Node): Signature {
  const found = findParentOfLastOctetString(sod);
  if (!found) throw new Error('extractSignature: no signature OCTET_STRING found in SOD');
  const [octet, parent] = found;

  if (octet.sub) {
    // The signature OCTET_STRING's bytes were themselves ASN.1-decodable —
    // true for ECDSA (`Ecdsa-Sig-Value ::= SEQUENCE { r INTEGER, s INTEGER }`),
    // never for RSA (opaque padded bytes).
    const r = octet.sub[0]?.sub?.[0]?.content;
    const s = octet.sub[0]?.sub?.[1]?.content;
    if (r === null || r === undefined || s === null || s === undefined) {
      throw new Error('extractSignature: ECDSA signature OCTET_STRING did not decode to SEQUENCE{r,s}');
    }
    return { kind: 'ecdsa', r: BigInt(r).toString(16).toLowerCase(), s: BigInt(s).toString(16).toLowerCase() };
  }

  // DEPARTURE from the original: it navigates
  // `parent.sub.slice(-2,-1)[0].sub.slice(-1)[0].sub?.slice(-1)[0].sub[0].content`
  // (SignerInfo.signatureAlgorithm's RSASSA-PSS params, if present) to find
  // the PSS salt length, with an optional-chain on only one hop of a
  // four-hop path — for a plain (non-PSS) RSA signature, where that params
  // structure is absent or shallow, this throws a TypeError instead of
  // falling back to "no salt", which the surrounding `cond ? val : 0`
  // ternary can't catch since the throw happens while evaluating `cond`
  // itself. Optional-chaining every hop preserves the exact same salt value
  // whenever the full PSS-params path IS present (the case this was
  // actually built and tested against), and just stops crashing when it
  // isn't — a plain-RSA-signed passport is exactly the case this needs to
  // not crash on.
  const saltNode = parent?.sub?.slice(-2, -1)[0]?.sub?.slice(-1)[0]?.sub?.slice(-1)[0]?.sub?.[0];
  const salt: string | 0 = saltNode?.content ? saltNode.content : 0;
  return { kind: 'rsa', n: octetStringValueHex(octet), salt };
}

/** Also usable on a standalone X.509 certificate's decoded ASN.1 tree, not just a SOD — matched by shape (any EC SubjectPublicKeyInfo), not by position. */
export function extractEcdsaPubkey(sod: Asn1Node): EcdsaPubkey {
  const loc = getEcdsaKeyLocation(sod);
  if (!loc || !loc.sub) throw new Error('extractEcdsaPubkey: no EC SubjectPublicKeyInfo found in SOD');
  const pubkey = loc.sub[1].content!.slice(8); // strip the BIT_STRING's leading "unused bits" + point-format-tag prefix
  const x = BigInt('0b' + pubkey.slice(0, pubkey.length / 2)).toString(16);
  const y = BigInt('0b' + pubkey.slice(pubkey.length / 2)).toString(16);
  const algParams = loc.sub[0]?.sub?.[1];
  const param = algParams?.sub ? algParams.sub[2]?.sub?.[0]?.content ?? '' : algParams?.content?.split('\n')[1] ?? '';
  return { kind: 'ecdsa', x, y, param };
}

/** Also usable on a standalone X.509 certificate's decoded ASN.1 tree, not just a SOD — matched by shape (any RSA SubjectPublicKeyInfo), not by position. */
export function extractRsaPubkey(sod: Asn1Node): RsaPubkey {
  const loc = getRsaKeyLocation(sod);
  if (!loc || !loc.sub || !loc.sub[0]?.sub) throw new Error('extractRsaPubkey: no RSA SubjectPublicKeyInfo found in SOD');
  const n = BigInt(loc.sub[0].sub[0].content!).toString(16);
  const exp = BigInt(loc.sub[0].sub[1].content!).toString(16);
  return { kind: 'rsa', n, exp };
}

// ---------------------------------------------------------------------------
// DG15 (Active Authentication public key) parsing
// ---------------------------------------------------------------------------

export interface Dg15Info {
  pubkey: Pubkey;
  /** Bit offset of the AA public key within the padded DG15 bit stream. */
  aaShift: number;
  /** SIGNATURE_TYPE for the AA key (0 if unrecognized). */
  aaSigType: number;
}

/**
 * DG15 ::= [APPLICATION 15] SubjectPublicKeyInfo (raw file bytes, as
 * ../native/nfcPassportReader.ts returns them — outer application tag
 * included, matching what `decoded()` expects here).
 */
function extractFromDg15(dg15: Uint8Array): Dg15Info | null {
  if (dg15.length === 0) return null;
  const dg15Decoded = decodeAsn1(dg15);
  const spki = dg15Decoded.sub?.[0];
  if (!spki || !spki.sub) throw new Error('extractFromDg15: DG15 did not decode to [APPLICATION 15] SubjectPublicKeyInfo');

  const isEcdsa = spki.sub[1].content?.slice(0, 8) === '00000100';

  if (isEcdsa) {
    const pkBit = spki.sub[1].content!.slice(8);
    const x = pkBit.slice(0, pkBit.length / 2);
    const y = pkBit.slice(pkBit.length / 2);
    const p = BigInt(spki.sub[0]?.sub?.[1]?.sub?.[4]?.content ?? '0').toString(16).toUpperCase();
    const curveSigType: Record<string, number> = {
      A9FB57DBA1EEA9BC3E660A909D838D718C397AA3B561A6F7901E0E82974856A7: 21, // brainpoolP256r1
      FFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF: 20, // secp256r1
      D35E472036BC4FB7E13C785ED201E065F98FCFA6F6F40DEF4F92B9EC7893EC28FCD412B1F1B32E27: 22, // brainpoolP320r1
      FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFFFFFFFFFFFF: 23, // secp192r1
    };
    const aaSigType = curveSigType[p] ?? 0;
    const aaShift = dg15Decoded.dump.split(BigInt('0x' + x).toString(16).toUpperCase())[0].length / 2;
    return { pubkey: { kind: 'ecdsa', x, y, param: p }, aaShift, aaSigType };
  }

  const pkLocation = spki.sub[1]?.sub?.[0];
  if (!pkLocation?.sub) throw new Error('extractFromDg15: RSA SubjectPublicKeyInfo shape mismatch');
  const n = pkLocation.sub[0].content!;
  const exp = pkLocation.sub[1].content!;
  const aaShift = dg15Decoded.dump.split(BigInt(n).toString(16).toUpperCase())[0].length / 2;
  return { pubkey: { kind: 'rsa', n: BigInt(n).toString(16), exp: BigInt(exp).toString(16) }, aaShift, aaSigType: 1 };
}

// ---------------------------------------------------------------------------
// ZKPassport circuit selection (REPLACES Rarimo's getSigType / SIGNATURE_TYPE)
// ---------------------------------------------------------------------------
//
// Rarimo compiled one monolithic `registerIdentity_<sigType>_...` circuit per parameter
// combination, and `getSigType` mapped a passport onto a small integer in its own
// hand-maintained SIGNATURE_TYPE table. ZKPassport instead ships a directory tree of
// small circuits, selected by a path — see `src/noir/bin/` in the circuits repo. These
// helpers produce those paths. The names below are the real directory names, checked
// against the repo at `d3a75ac`, not invented.

/** Hash algorithms ZKPassport's circuits are built for. */
export type HashAlgorithm = 'sha1' | 'sha224' | 'sha256' | 'sha384' | 'sha512';

/** TBSCertificate size buckets ZKPassport compiles `sig-check` circuits for. */
export type TbsBucket = 700 | 1000 | 1200 | 1600;

const TBS_BUCKETS: TbsBucket[] = [700, 1000, 1200, 1600];

/** Maps a digest length in bytes onto its algorithm name. */
export function hashAlgorithmFor(digestLength: number, what: string): HashAlgorithm {
  switch (digestLength) {
    case 20:
      return 'sha1';
    case 28:
      return 'sha224';
    case 32:
      return 'sha256';
    case 48:
      return 'sha384';
    case 64:
      return 'sha512';
    default:
      throw new Error(`hashAlgorithmFor: no ZKPassport circuit for a ${digestLength}-byte ${what} digest`);
  }
}

/**
 * Smallest `tbs_N` bucket that fits a DER TBSCertificate of `length` bytes. Picking the
 * smallest is not just tidiness: it is the cheapest circuit that can hold the data, and
 * an oversized bucket means proving a much larger circuit for no benefit.
 */
export function tbsBucketFor(length: number): TbsBucket {
  const bucket = TBS_BUCKETS.find((candidate) => length <= candidate);
  if (bucket === undefined) {
    throw new Error(
      `tbsBucketFor: TBSCertificate is ${length} bytes; ZKPassport's largest bucket is tbs_${TBS_BUCKETS[TBS_BUCKETS.length - 1]}`,
    );
  }
  return bucket;
}

/** The signature-algorithm half of a `sig-check` circuit path. */
export interface SignatureAlgorithm {
  kind: 'rsa' | 'ecdsa';
  /** Path fragment under `sig-check/*\/tbs_N/`, e.g. `rsa/pkcs/2048/sha256` or `ecdsa/nist/p256/sha256`. */
  path: string;
  /**
   * Width, in bytes, of one big-endian value for this algorithm: the RSA modulus, or one
   * ECDSA coordinate. An ECDSA signature is two of these; an RSA signature is one.
   */
  byteWidth: number;
}

/** RSA modulus sizes ZKPassport compiles circuits for, in bits. */
const RSA_KEY_BITS = [1024, 2048, 3072, 4096];

/**
 * Named curves, keyed by the curve's `a` parameter as this parser extracts it
 * (`EcdsaPubkey.param`). Values are ZKPassport's own directory names.
 *
 * Carried over from the Rarimo-era `getSigType` table, which recognised these same
 * parameters — the curve identification was never vendor-specific, only the integer it
 * mapped to was.
 */
const CURVE_BY_PARAM: Record<string, { path: string; byteWidth: number }> = {
  FFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFC: { path: 'nist/p256', byteWidth: 32 },
  FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFFFFFFFFFFFFFFFFFFFE: { path: 'nist/p224', byteWidth: 28 },
  '7D5A0975FC2C3057EEF67530417AFFE7FB8055C126DC5C6CE94A4B44F330B5D9': { path: 'brainpool/256r1', byteWidth: 32 },
  '7BC382C63D8C150C3C72080ACE05AFA0C2BEA28E4FB22787139165EFBA91F90F8AA5814A503AD4EB04A8C7DD22CE2826': {
    path: 'brainpool/384r1',
    byteWidth: 48,
  },
  '7830A3318B603B89E2327145AC234CC594CBDD8D3DF91610A83441CAEA9863BC2DED5D5AA8253AA10A2EF1C98B9AC8B57F1117A72BF2C7B9E7C1AC4D77FC94CA': {
    path: 'brainpool/512r1',
    byteWidth: 64,
  },
};

/**
 * Picks the ZKPassport `sig-check` circuit path for a passport's signature algorithm.
 *
 * REPLACES `getSigType`. The classification inputs are the same (key size, exponent, PSS
 * salt presence, curve parameter); only the output changed, from Rarimo's SIGNATURE_TYPE
 * integer to a circuit path.
 */
export function classifySignatureAlgorithm(
  pubkey: Pubkey,
  signature: Signature,
  hash: HashAlgorithm,
): SignatureAlgorithm {
  if (pubkey.kind === 'rsa' && signature.kind === 'rsa') {
    // `RsaPubkey.n` is a hex string, so 4 bits per character.
    const bits = pubkey.n.length * 4;
    if (!RSA_KEY_BITS.includes(bits)) {
      throw new Error(`classifySignatureAlgorithm: no ZKPassport circuit for a ${bits}-bit RSA key`);
    }
    // A non-zero PSS salt length is what distinguishes RSASSA-PSS from PKCS#1 v1.5 here;
    // `extractSignature` already reads it off the signature's algorithm parameters.
    const padding = signature.salt ? 'pss' : 'pkcs';
    return { kind: 'rsa', path: `rsa/${padding}/${bits}/${hash}`, byteWidth: bits / 8 };
  }

  if (pubkey.kind === 'ecdsa' && signature.kind === 'ecdsa') {
    const curve = CURVE_BY_PARAM[pubkey.param.toUpperCase()];
    if (curve === undefined) {
      throw new Error(
        `classifySignatureAlgorithm: unrecognized ECDSA curve parameter ${pubkey.param} — add it to CURVE_BY_PARAM if ZKPassport ships a circuit for it`,
      );
    }
    return { kind: 'ecdsa', path: `ecdsa/${curve.path}/${hash}`, byteWidth: curve.byteWidth };
  }

  throw new Error(
    `classifySignatureAlgorithm: pubkey (${pubkey.kind}) and signature (${signature.kind}) algorithms disagree`,
  );
}

// ---------------------------------------------------------------------------
// DSC certificate extraction (NEW — ZKPassport needs the raw TBSCertificate)
// ---------------------------------------------------------------------------

/**
 * Pulls the Document Signer Certificate's `TBSCertificate` DER out of the SOD's CMS
 * `certificates` field.
 *
 * NEW relative to the Rarimo parser, which never needed it: ZKPassport's `sig-check`
 * circuits take `tbs_certificate: [u8; N]` and verify in-circuit both that the CSC signed
 * it (`sig-check/dsc`) and that the DSC public key used for the SOD signature really is
 * the one inside it (`verify_ecdsa_pubkey_in_tbs` / `verify_rsa_pubkey_in_tbs` in
 * `sig-check/id-data`). Without it the DSC public key would be unauthenticated.
 *
 * The TBSCertificate is the first element of the `Certificate` SEQUENCE (RFC 5280
 * `Certificate ::= SEQUENCE { tbsCertificate TBSCertificate, signatureAlgorithm .., signatureValue .. }`),
 * and is passed to the circuit as its full tag-length-value DER, since the circuit
 * re-parses its ASN.1 header to recover the real length.
 */
export function extractTbsCertificate(sod: Asn1Node): Uint8Array {
  const certificate = findCertificate(sod);
  if (certificate === null) {
    throw new Error(
      'extractTbsCertificate: no X.509 certificate found in the SOD — this SOD omits the Document Signer Certificate, so ZKPassport cannot authenticate its public key',
    );
  }
  const tbs = certificate.sub?.[0];
  if (tbs === undefined) {
    throw new Error('extractTbsCertificate: certificate SEQUENCE has no TBSCertificate element');
  }
  return hexToBytes(tbs.dump);
}

/**
 * Finds the X.509 `Certificate` SEQUENCE inside the SOD tree, by shape rather than by
 * position — the same strategy the rest of this file uses for the SOD's other elements,
 * and for the same reason: real SODs vary in exactly which optional CMS fields are
 * present, so index-based paths are brittle.
 *
 * A `Certificate` is recognised as a SEQUENCE of exactly 3 children whose first child is
 * itself a SEQUENCE containing an explicit `[0]` version tag or an INTEGER serial number,
 * followed by an AlgorithmIdentifier SEQUENCE and a BIT STRING signature.
 */
function findCertificate(node: Asn1Node): Asn1Node | null {
  const children = node.sub ?? [];
  if (node.name === 'SEQUENCE' && children.length === 3) {
    const [tbs, algorithm, signatureValue] = children;
    if (
      tbs.name === 'SEQUENCE' &&
      (tbs.sub?.length ?? 0) >= 6 &&
      algorithm.name === 'SEQUENCE' &&
      signatureValue.name === 'BIT_STRING'
    ) {
      return node;
    }
  }
  for (const child of children) {
    const found = findCertificate(child);
    if (found !== null) return found;
  }
  return null;
}


// ---------------------------------------------------------------------------
// Shift computation (mirrors getDg1Shift / getDg15Shift / getEcShift)
// ---------------------------------------------------------------------------

/**
 * DEPARTURE from the original: `getDg1Shift`/`getDg15Shift`/`getEcShift` use
 * `.split(needle)[0].length` to locate a hash's byte offset, which — if the
 * needle isn't found at all — silently returns the *whole haystack's*
 * length (a wrong, garbage shift) rather than failing loudly. Wrong shift
 * params compile into the wrong circuit variant, which would fail proof
 * generation or, worse, verify against the wrong bytes. This uses
 * `indexOf` and throws when it comes back `-1` instead.
 */
function hexShift(haystackHex: string, needle: Uint8Array, what: string): number {
  const idx = haystackHex.toLowerCase().indexOf(bytesToHex(needle).toLowerCase());
  if (idx < 0) throw new Error(`hexShift: ${what} hash not found — wrong hash type, or the passport data doesn't match this SOD`);
  return idx / 2;
}

// ---------------------------------------------------------------------------
// Top-level assembly — ZKPassport circuit inputs
// ---------------------------------------------------------------------------

/**
 * Identifies which ZKPassport circuits this passport needs. These are real directory
 * paths under the circuits repo's `src/noir/bin/`, so a name here maps 1:1 onto a
 * compiled artifact (and, eventually, onto a leaf of the on-chain `circuit_registry_root`
 * tree the outer circuit checks each subproof's verification key against).
 */
export interface CircuitVariant {
  /**
   * Human-readable id for the whole variant — the `id-data` circuit path, which is the
   * most specific of the three and uniquely determines the other two.
   * e.g. `sig-check/id-data/tbs_1000/rsa/pkcs/2048/sha256`.
   */
  name: string;
  /** `sig-check/id-data/tbs_{bucket}/...` — DSC-signs-passport check. */
  idDataCircuit: string;
  /** `sig-check/dsc/tbs_{bucket}/...` — CSC-signs-DSC check. Same algorithm shape, different TBS. */
  dscCircuit: string;
  /** `data-check/integrity/sa_{hash}/dg_{hash}` — datagroup-hash integrity check. */
  integrityCircuit: string;
  /** The `tbs_N` size bucket the DSC's TBSCertificate falls into. */
  tbsBucket: TbsBucket;
  signature: SignatureAlgorithm;
  /** Hash over `signedAttributes` (the SOD signature's message digest). */
  signedAttrsHash: HashAlgorithm;
  /** Hash over each datagroup, listed inside the encapsulated content. */
  dataGroupHash: HashAlgorithm;
}

/**
 * The `sig-check/id-data` circuit's inputs that are derivable from DG1/DG15/SOD alone.
 *
 * Byte arrays are already padded to the circuit's fixed sizes; `*Size` fields record the
 * real length for cross-checking (the circuit re-derives them from the ASN.1 itself, so
 * they are not passed in — they are here so a caller can assert the padding is right).
 */
export interface IdDataInputs {
  /** `dg1: DG1Data` — `[u8; 95]`. */
  dg1: Uint8Array;
  dg1Size: number;
  /** `signed_attributes: SignedAttrsData` — `[u8; 256]`. */
  signedAttributes: Uint8Array;
  signedAttributesSize: number;
  /** `e_content: EContentData` — `[u8; 700]`. */
  eContent: Uint8Array;
  eContentSize: number;
  /** `sod_signature` — raw big-endian bytes, width set by the key size. */
  sodSignature: Uint8Array;
  /** `tbs_certificate` — the DSC's TBSCertificate DER, padded to `tbsBucket`. */
  tbsCertificate: Uint8Array;
  tbsCertificateSize: number;
  /** RSA: `dsc_pubkey` (the modulus). ECDSA: `dsc_pubkey_x`/`dsc_pubkey_y`. */
  dscPubkey: DscPubkeyBytes;
}

/** The DSC public key, in whichever shape its algorithm's circuit takes. */
export type DscPubkeyBytes =
  | { kind: 'rsa'; modulus: Uint8Array; exponent: number }
  | { kind: 'ecdsa'; x: Uint8Array; y: Uint8Array };

/**
 * The `data-check/integrity` circuit's positional inputs: where each hash sits inside the
 * next structure up. Same values the Rarimo circuit called `dg1Shift`/`ecShift`, in bytes
 * rather than bits — Noir indexes byte arrays, circom indexed bit arrays.
 */
export interface IntegrityInputs {
  /** Offset of `SHA(dg1)` within the encapsulated content, in bytes. */
  dg1HashOffset: number;
  /** Offset of `SHA(eContent)` within the signed attributes, in bytes. */
  eContentHashOffset: number;
  /** Offset of `SHA(dg15)` within the encapsulated content, in bytes. `null` when the passport has no Active Authentication. */
  dg15HashOffset: number | null;
}

/**
 * Inputs ZKPassport's pipeline needs that **cannot** come from the passport chip, listed
 * explicitly so a caller cannot forget one. Assembling these is separate, unstarted work
 * (see the module doc comment).
 */
export interface UnresolvedInputs {
  /** `sig-check/dsc` needs the CSC (country signing) certificate that signed this DSC, plus its `redc` params. Comes from the certificate registry, not the chip. */
  cscCertificate: null;
  /** The DSC's inclusion proof in the certificate registry tree — `certificateTree.ts` builds these, but wiring is not done. */
  certificateTreeProof: null;
  /** Per-subproof commitment-chain salts. Must be freshly random per proof, generated on-device. */
  commitmentSalts: null;
  /** `service_scope` / `service_subscope` — chain-level constants, not passport data. */
  serviceScope: null;
}

export interface ParsedPassport {
  variant: CircuitVariant;
  idData: IdDataInputs;
  integrity: IntegrityInputs;
  /** Active Authentication key material, when DG15 is present. ZKPassport has no AA circuit today, so this is parsed but unused. */
  activeAuthentication: Dg15Info | null;
  unresolved: UnresolvedInputs;
}

/**
 * Builds ZKPassport circuit inputs from a passport's raw DG1 / DG15 / SOD bytes.
 * `dg15` may be an empty `Uint8Array` for passports without Active Authentication.
 *
 * Everything above this function — the ASN.1 walk, the encapsulated-content and
 * signed-attributes extraction, the RSA/ECDSA public key and signature extraction, the
 * hash-offset search — is unchanged from the Rarimo-era parser, because it was never
 * Rarimo-specific: it reads the ICAO CMS SignedData structure the passport itself
 * defines. Only the encoding of the result changed (bits/limbs -> fixed-length bytes),
 * and with it the variant naming.
 */
export function buildCircuitInputs(dg1: Uint8Array, dg15: Uint8Array, sod: Uint8Array): ParsedPassport {
  const sodTree = decodeAsn1(sod);

  const { ecHex, dgHashType } = extractEncapsulatedContent(sodTree);
  const ecBytes = hexToBytes(ecHex);
  const { saHex, hashType } = extractSignedAttributes(sodTree);
  const saBytes = hexToBytes(saHex);

  const signature = extractSignature(sodTree);
  const pubkey = signature.kind === 'rsa' ? extractRsaPubkey(sodTree) : extractEcdsaPubkey(sodTree);
  if (pubkey.kind !== signature.kind) {
    throw new Error(`buildCircuitInputs: pubkey algorithm (${pubkey.kind}) doesn't match signature algorithm (${signature.kind})`);
  }

  const signedAttrsHash = hashAlgorithmFor(hashType, 'signedAttributes');
  const dataGroupHash = hashAlgorithmFor(dgHashType, 'datagroup');
  const sigAlgorithm = classifySignatureAlgorithm(pubkey, signature, signedAttrsHash);

  const tbsCertificateDer = extractTbsCertificate(sodTree);
  const tbsBucket = tbsBucketFor(tbsCertificateDer.length);

  const dg1HashOffset = hexShift(ecHex, computeHash(dgHashType as HashOutLen, dg1), 'DG1');
  const eContentHashOffset = hexShift(saHex, computeHash(hashType as HashOutLen, ecBytes), 'encapsulated content');
  const dg15HashOffset =
    dg15.length !== 0 ? hexShift(ecHex, computeHash(dgHashType as HashOutLen, dg15), 'DG15') : null;

  const idDataCircuit = `sig-check/id-data/tbs_${tbsBucket}/${sigAlgorithm.path}`;
  const variant: CircuitVariant = {
    name: idDataCircuit,
    idDataCircuit,
    dscCircuit: `sig-check/dsc/tbs_${tbsBucket}/${sigAlgorithm.path}`,
    integrityCircuit: `data-check/integrity/sa_${signedAttrsHash}/dg_${dataGroupHash}`,
    tbsBucket,
    signature: sigAlgorithm,
    signedAttrsHash,
    dataGroupHash,
  };

  const idData: IdDataInputs = {
    dg1: padToFixedLength(dg1, DG1_LENGTH, 'DG1'),
    dg1Size: dg1.length,
    signedAttributes: padToFixedLength(saBytes, SIGNED_ATTRS_LENGTH, 'signedAttributes'),
    signedAttributesSize: saBytes.length,
    eContent: padToFixedLength(ecBytes, ECONTENT_LENGTH, 'encapsulatedContent'),
    eContentSize: ecBytes.length,
    sodSignature: signatureToBytes(signature, sigAlgorithm),
    tbsCertificate: padToFixedLength(tbsCertificateDer, tbsBucket, 'TBSCertificate'),
    tbsCertificateSize: tbsCertificateDer.length,
    dscPubkey: pubkeyToBytes(pubkey, sigAlgorithm),
  };

  return {
    variant,
    idData,
    integrity: { dg1HashOffset, eContentHashOffset, dg15HashOffset },
    activeAuthentication: extractFromDg15(dg15),
    unresolved: {
      cscCertificate: null,
      certificateTreeProof: null,
      commitmentSalts: null,
      serviceScope: null,
    },
  };
}

/** Packs a parsed signature into the raw big-endian byte array its circuit takes. */
function signatureToBytes(signature: Signature, algorithm: SignatureAlgorithm): Uint8Array {
  if (signature.kind === 'rsa') {
    return hexToBeBytes(signature.n, algorithm.byteWidth);
  }
  // ECDSA: r || s, each padded to the curve's field width.
  const out = new Uint8Array(algorithm.byteWidth * 2);
  out.set(hexToBeBytes(signature.r, algorithm.byteWidth), 0);
  out.set(hexToBeBytes(signature.s, algorithm.byteWidth), algorithm.byteWidth);
  return out;
}

/** Packs a parsed public key into the raw big-endian byte array(s) its circuit takes. */
function pubkeyToBytes(pubkey: Pubkey, algorithm: SignatureAlgorithm): DscPubkeyBytes {
  if (pubkey.kind === 'rsa') {
    return {
      kind: 'rsa',
      modulus: hexToBeBytes(pubkey.n, algorithm.byteWidth),
      exponent: Number(BigInt('0x' + pubkey.exp)),
    };
  }
  return {
    kind: 'ecdsa',
    x: hexToBeBytes(pubkey.x, algorithm.byteWidth),
    y: hexToBeBytes(pubkey.y, algorithm.byteWidth),
  };
}

export { Hex, Base64 };
