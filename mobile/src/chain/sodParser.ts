/**
 * Assembles `RegisterIdentityBuilder` circuit inputs from a passport's raw
 * DG1 / DG15 / SOD bytes (as read by ../native/nfcPassportReader.ts).
 *
 * This is a TypeScript port of rarimo/passport-zk-circuits' `test/
 * process_passport.js` (MIT licensed) — the reference implementation for
 * turning a scanned passport into this exact circuit's input schema (see
 * that repo's README, "Register identity circuit inputs"). Ported (not
 * copied verbatim, unlike ./asn1.js) because the original is Node-specific
 * (`require('crypto')`, `fs`) and mixes input-building with file-writing;
 * every function below mirrors its namesake there field-for-field, with
 * three deliberate, disclosed departures from the original's behavior —
 * search "DEPARTURE" below for each one and why.
 *
 * What this module does NOT produce, and why — both are real, unresolved
 * blockers, not oversights (see HANDOFF.md item 8):
 *
 *  - `skIdentity`: the reference script derives this from the passport's
 *    own public SOD bytes (`hash(encapsulatedContent)`, truncated) — fine
 *    for generating deterministic *test* fixtures, but wrong for production:
 *    a secret derived entirely from data anyone can read off the chip isn't
 *    secret at all. The real identity secret must be generated locally
 *    on-device (e.g. a random field element) and persisted the same way
 *    ../native's signing key eventually will be — not attempted here.
 *  - `slaveMerkleRoot` / `slaveMerkleInclusionBranches`: these prove the
 *    passport's signing certificate is ICAO-trusted, against Rarimo's own
 *    `CertificatesSMT` registry (a live Sparse Merkle Tree on their zkRollup
 *    — see docs.rarimo.com/zk-passport/contracts). No documented client API
 *    for reading a proof from it was found; getting one requires the same
 *    kind of source-level research HANDOFF log #57 did for NFC reading, not
 *    yet done. `buildCircuitInputs` below takes these as caller-supplied
 *    parameters for exactly this reason.
 *
 * Also not produced: which prebuilt circuit variant (proving key / `.wcd`
 * graph / VK) this passport needs. `circuitVariant` below identifies it
 * (mirrors the original's `old_naming_convention` string), but whether a
 * matching prebuilt asset actually exists anywhere is a separate, unverified
 * question — passport-zk-circuits only ships prebuilt bundles for the
 * variant combinations Rarimo has actually encountered.
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
// SHA padding (mirrors process_passport.js's `padding`)
// ---------------------------------------------------------------------------

/**
 * Pads `bytes` per the SHA Merkle-Damgard scheme (append 0x80, zero-pad,
 * append the bit-length as a big-endian integer) so the circuit can hash it
 * with a fixed-size, block-count-parameterized SHA template instead of
 * parsing/measuring the message itself (see passport-zk-circuits' README,
 * "Padded data hashing").
 *
 * DEPARTURE from the original: `process_passport.js` computes this by
 * building a padded *hex string*, round-tripping it through `BigInt(...).
 * toString(2)`, then patching the front back up with zeros to fix the
 * leading zero bits `BigInt`'s binary conversion silently drops. That patch
 * is necessary *because* of the BigInt round-trip, not because of anything
 * about SHA padding itself. This builds the bit array directly from the
 * padded bytes (each byte -> 8 MSB-first '0'/'1' chars), which can't lose
 * leading zeros in the first place — same output, no patch-up needed. A
 * unit test below checks a known SHA-256 padding vector byte-for-byte
 * against this implementation.
 */
export function padBits(bytes: Uint8Array, blockSizeBits: 512 | 1024): string[] {
  const blockSizeBytes = blockSizeBits / 8;
  const lengthSizeBytes = blockSizeBits === 512 ? 8 : 16;

  const totalLenWith1AndLength = bytes.length + 1 + lengthSizeBytes;
  const paddingLen = (blockSizeBytes - (totalLenWith1AndLength % blockSizeBytes)) % blockSizeBytes;
  const totalLen = bytes.length + 1 + paddingLen + lengthSizeBytes;

  const padded = new Uint8Array(totalLen);
  padded.set(bytes, 0);
  padded[bytes.length] = 0x80;
  // Remaining padding bytes are already zero (Uint8Array default-initializes).

  const bitLen = BigInt(bytes.length) * 8n;
  let tmp = bitLen;
  for (let i = 0; i < lengthSizeBytes; i++) {
    padded[totalLen - 1 - i] = Number(tmp & 0xffn);
    tmp >>= 8n;
  }

  const bits: string[] = new Array(totalLen * 8);
  for (let i = 0; i < totalLen; i++) {
    const byte = padded[i];
    for (let b = 0; b < 8; b++) {
      bits[i * 8 + b] = ((byte >> (7 - b)) & 1).toString();
    }
  }
  return bits;
}

// ---------------------------------------------------------------------------
// Big-integer <-> circuit-limb conversion (mirrors bigintToArrayString)
// ---------------------------------------------------------------------------

/** Splits `x` into `k` little-endian limbs of `n` bits each, as decimal strings — the `bigIntFunc.circom` chunked representation. */
export function bigintToArrayString(n: number, k: number, x: bigint): string[] {
  const mod = 1n << BigInt(n);
  const result: string[] = [];
  let rem = x;
  for (let i = 0; i < k; i++) {
    result.push((rem % mod).toString(10));
    rem /= mod;
  }
  return result;
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
// Signature type classification (mirrors getSigType)
// ---------------------------------------------------------------------------

// SIGNATURE_TYPE, per passport-zk-circuits:
//   1: RSA 2048 + SHA2-256, e=65537        2: RSA 4096 + SHA2-256, e=65537
//   3: RSA 2048 + SHA1, e=65537
//  10: RSASSA-PSS 2048 MGF1(SHA256) e=3  salt=32     11: e=65537 salt=32
//  12: RSASSA-PSS 2048 MGF1(SHA256) e=65537 salt=64  13: MGF1(SHA384) salt=48
//  14: RSASSA-PSS 3072 MGF1(SHA256) e=65537 salt=32
//  20: ECDSA secp256r1     21: ECDSA brainpoolP256r1
//  22: ECDSA brainpoolP320r1   23: ECDSA secp192r1
export function getSigType(pk: Pubkey, sig: Signature, hashType: number): number {
  if (pk.kind === 'rsa' && sig.kind === 'rsa') {
    if (sig.salt) {
      const salt = String(sig.salt);
      const hash = String(hashType);
      if (pk.n.length === 512 && pk.exp === '3' && salt === '32' && hash === '32') return 10;
      if (pk.n.length === 512 && pk.exp === '10001' && salt === '32' && hash === '32') return 11;
      if (pk.n.length === 512 && pk.exp === '10001' && salt === '64' && hash === '32') return 12;
      if (pk.n.length === 512 && pk.exp === '10001' && salt === '48' && hash === '48') return 13;
      if (pk.n.length === 768 && pk.exp === '10001' && salt === '32' && hash === '32') return 14;
    }
    if (sig.salt === 0) {
      if (pk.n.length === 512 && pk.exp === '10001' && hashType === 32) return 1;
      if (pk.n.length === 1024 && pk.exp === '10001' && hashType === 32) return 2;
      if (pk.n.length === 512 && pk.exp === '10001' && hashType === 20) return 3;
    }
  }
  if (pk.kind === 'ecdsa' && sig.kind === 'ecdsa') {
    switch (pk.param) {
      case '7D5A0975FC2C3057EEF67530417AFFE7FB8055C126DC5C6CE94A4B44F330B5D9':
        return 21; // brainpoolP256r1
      case 'FFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFC':
        return 20; // secp256r1
      case 'FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFFFFFFFFFFFFFFFFFFFE':
        return 24; // secp224r1
      case '7BC382C63D8C150C3C72080ACE05AFA0C2BEA28E4FB22787139165EFBA91F90F8AA5814A503AD4EB04A8C7DD22CE2826':
        return 25; // brainpoolP384r1
      case '7830A3318B603B89E2327145AC234CC594CBDD8D3DF91610A83441CAEA9863BC2DED5D5AA8253AA10A2EF1C98B9AC8B57F1117A72BF2C7B9E7C1AC4D77FC94CA':
        return 26; // brainpoolP512r1
      case 'secp521r1':
        return 27;
      default:
        return 0;
    }
  }
  return 0;
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
// RSA/ECDSA limb chunking (mirrors getChunkedParams)
// ---------------------------------------------------------------------------

interface ChunkedParams {
  ecFieldSize: number;
  chunkNumber: number;
  pkChunked: string[];
  sigChunked: string[];
}

const reHexOnly = /^[0-9A-Fa-f]+$/;

function getChunkedParams(pk: Pubkey, sig: Signature): ChunkedParams {
  const ecFieldSize =
    pk.kind === 'ecdsa'
      ? reHexOnly.test(pk.param)
        ? pk.param.length * 4
        : (pk.param.match(/\d+/)?.[0] && parseInt(pk.param.match(/\d+/)![0], 10)) || 0
      : 0;
  const rawChunkNumber = pk.kind === 'ecdsa' ? Math.ceil(pk.x.length / 16) : Math.ceil(pk.n.length / 16);
  const chunkSize = ecFieldSize > 512 ? 66 : 64;
  const chunkNumber = ecFieldSize !== 0 ? rawChunkNumber * 2 : rawChunkNumber;

  const pkChunked =
    pk.kind === 'ecdsa'
      ? [...bigintToArrayString(chunkSize, rawChunkNumber, BigInt('0x' + pk.x)), ...bigintToArrayString(chunkSize, rawChunkNumber, BigInt('0x' + pk.y))]
      : bigintToArrayString(chunkSize, rawChunkNumber, BigInt('0x' + pk.n));

  const sigChunked =
    sig.kind === 'ecdsa'
      ? [...bigintToArrayString(chunkSize, rawChunkNumber, BigInt('0x' + sig.r)), ...bigintToArrayString(chunkSize, rawChunkNumber, BigInt('0x' + sig.s))]
      : bigintToArrayString(chunkSize, rawChunkNumber, BigInt('0x' + sig.n));

  return { ecFieldSize, chunkNumber, pkChunked, sigChunked };
}

// ---------------------------------------------------------------------------
// Top-level assembly
// ---------------------------------------------------------------------------

/** Identifies exactly which `RegisterIdentityBuilder` instantiation this passport needs — the params `writeToCircom` compiles a concrete circuit with, and what a prebuilt-asset bundle (proving key / `.wcd` graph / VK) must be built for to match. */
export interface CircuitVariant {
  sigAlgo: number;
  /** Bits (not bytes) — matches writeToCircom's dg_hash_type*8 convention. */
  dgHashTypeBits: number;
  docType: 1 | 3;
  ecBlocks: number;
  ecShiftBits: number;
  dg1ShiftBits: number;
  dg15SigAlgo: number;
  dg15ShiftBits: number;
  dg15Blocks: number;
  aaShiftBits: number;
  /** Human-readable variant id, mirrors the original's `old_naming_convention`, e.g. `registerIdentity_11_256_3_2_336_216_NA`. */
  name: string;
}

/** The `RegisterIdentityBuilder` circuit's inputs, minus `skIdentity` / `slaveMerkleRoot` / `slaveMerkleInclusionBranches` — see this module's doc comment for why those are the caller's responsibility. */
export interface RegisterIdentityInputs {
  dg1: string[];
  dg15: string[];
  encapsulatedContent: string[];
  signedAttributes: string[];
  pubkey: string[];
  signature: string[];
}

export interface ParsedPassport {
  variant: CircuitVariant;
  inputs: RegisterIdentityInputs;
}

/**
 * Builds `RegisterIdentityBuilder` circuit inputs from a passport's raw
 * DG1 / DG15 / SOD bytes. `dg15` may be an empty `Uint8Array` for passports
 * without Active Authentication — this produces an `_NA` variant with empty
 * `dg15`/AA fields, exactly like the `registerIdentity_*_NA` variants
 * upstream ships (this is also the case log #61 fixed a real
 * `circom-witnesscalc` bug for — see HANDOFF.md item 8).
 */
export function buildCircuitInputs(dg1: Uint8Array, dg15: Uint8Array, sod: Uint8Array): ParsedPassport {
  const sodTree = decodeAsn1(sod);

  const { ecHex, dgHashType } = extractEncapsulatedContent(sodTree);
  const ecBytes = hexToBytes(ecHex);
  const { saHex, hashType } = extractSignedAttributes(sodTree);
  const saBytes = hexToBytes(saHex);

  const dgHashBlockBits = dgHashType <= 32 ? 512 : 1024;
  const hashBlockBits = hashType <= 32 ? 512 : 1024;

  const signature = extractSignature(sodTree);
  const pubkey = signature.kind === 'rsa' ? extractRsaPubkey(sodTree) : extractEcdsaPubkey(sodTree);
  if (pubkey.kind !== signature.kind) {
    throw new Error(`buildCircuitInputs: pubkey algorithm (${pubkey.kind}) doesn't match signature algorithm (${signature.kind})`);
  }

  const sigType = getSigType(pubkey, signature, hashType);
  if (sigType === 0) {
    throw new Error('buildCircuitInputs: unrecognized signature/pubkey/hash combination — see getSigType\'s SIGNATURE_TYPE table');
  }

  const dg1ShiftBits = hexShift(ecHex, computeHash(dgHashType as HashOutLen, dg1), 'DG1') * 8;
  const ecShiftBits = hexShift(saHex, computeHash(hashType as HashOutLen, ecBytes), 'encapsulated content') * 8;
  // Offset of DG15's own hash within the encapsulated content — distinct
  // from `aaShift` below (the offset of the AA public key *within DG15
  // itself*); the original conflates neither, and an earlier draft of this
  // port mistakenly did — kept as two separate values on purpose.
  const dg15ShiftBits = dg15.length !== 0 ? hexShift(ecHex, computeHash(dgHashType as HashOutLen, dg15), 'DG15') * 8 : 0;

  const dg15Info = extractFromDg15(dg15);

  const chunked = getChunkedParams(pubkey, signature);

  const docType: 1 | 3 = dg1.length === 93 ? 3 : 1;
  const ecBlocks = hashType <= 32 ? Math.ceil((ecBytes.length + 8) / 64) : Math.ceil((ecBytes.length + 8) / 128);
  const dg15Blocks = dg15.length !== 0 ? (dgHashType <= 32 ? Math.ceil((dg15.length + 8) / 64) : Math.ceil((dg15.length + 8) / 128)) : 0;

  const aaShiftBits = dg15Info ? dg15Info.aaShift * 8 : 0;

  // The *display name* built here is a separate, disclosed quirk from the
  // real circuit params above: process_passport.js's own naming-string
  // template re-multiplies the already-bit-converted `ec_shift`/`dg1_shift`
  // by another `*8` (i.e. the name embeds shift-in-bits-treated-as-bytes-
  // then-re-converted, not the actual bit shift), while its `writeToCircom`
  // call — the thing that actually compiles a circuit — uses the correctly
  // single-multiplied values, matching `ecShiftBits`/`dg1ShiftBits` above.
  // Confirmed empirically against the unmodified reference script (see
  // sodParser.test.ts) and independently already flagged in HANDOFF.md log
  // #60 ("the release name's shift digits are not guaranteed to be
  // literally identical to the constructor's raw shift arguments"). This
  // reproduces the same doubling *only* in the name string, since matching
  // Rarimo's actual published release-asset filenames (which use this same
  // buggy convention, per log #60) is the whole point of computing a name —
  // never use the digits inside `variant.name` as real shift values; use
  // the `CircuitVariant` fields for that.
  //
  // The DG15-present branch below is NOT verified against a real DG15
  // fixture (this module's test only covers an _NA/no-DG15 passport) — its
  // `writeToCircom` call passes `aa_shift` raw (byte units, despite the
  // reference's own comment there claiming "AA shift in bits"), a further
  // inconsistency this port hasn't independently confirmed one way or the
  // other. Treat `dg15SigAlgo`/`dg15ShiftBits`/`aaShiftBits` and this name's
  // DG15 segment as unverified until checked against a real AA-bearing
  // passport's reference output the same way the NA path was here.
  const variantName = `registerIdentity_${sigType}_${dgHashType * 8}_${docType}_${ecBlocks}_${ecShiftBits * 8}_${dg1ShiftBits * 8}_${
    dg15.length === 0 ? 'NA' : `${dg15Info!.aaSigType}_${dg15ShiftBits * 8}_${dg15Blocks}_${aaShiftBits}`
  }`;

  const variant: CircuitVariant = {
    sigAlgo: sigType,
    dgHashTypeBits: dgHashType * 8,
    docType,
    ecBlocks,
    ecShiftBits,
    dg1ShiftBits,
    dg15SigAlgo: dg15Info?.aaSigType ?? 0,
    dg15ShiftBits,
    dg15Blocks,
    aaShiftBits,
    name: variantName,
  };

  const inputs: RegisterIdentityInputs = {
    dg1: padBits(dg1, dgHashBlockBits as 512 | 1024),
    dg15: dg15.length !== 0 ? padBits(dg15, dgHashBlockBits as 512 | 1024) : [],
    encapsulatedContent: padBits(ecBytes, hashBlockBits as 512 | 1024),
    signedAttributes: padBits(saBytes, hashBlockBits as 512 | 1024),
    pubkey: chunked.pkChunked,
    signature: chunked.sigChunked,
  };

  return { variant, inputs };
}

export { Hex, Base64 };
