/**
 * One-time generator for a synthetic ICAO-shaped passport SOD used by the dev-only
 * "test passport" path in RegisterScreen.tsx (see docs/... discussion: testing the
 * registration pipeline without a physical passport).
 *
 * This is NOT a real passport and proves nothing about a real identity — it exists
 * purely to exercise `sodParser.ts`'s buildCircuitInputs() against a genuinely valid
 * CMS SignedData structure (real RSA keys, a real self-signed test certificate chain,
 * a real signature over a real LDSSecurityObject/DG1 hash), since that parser walks
 * actual ASN.1 shapes and cannot be satisfied by hand-typed placeholder bytes.
 *
 * Run once with `node scripts/generate-test-passport-fixture.js` from mobile/; writes
 * src/chain/__fixtures__/testPassport.ts. Re-run only if sodParser.ts's expected SOD
 * shape ever changes.
 */
const forge = require('node-forge');
const fs = require('fs');
const path = require('path');

const { asn1, pki, md, util } = forge;

function bin(str) {
  return Buffer.from(str, 'binary');
}

function der(asn1Node) {
  return bin(asn1.toDer(asn1Node).getBytes());
}

function oid(oidStr) {
  return asn1.create(asn1.Class.UNIVERSAL, asn1.Type.OID, false, asn1.oidToDer(oidStr).getBytes());
}

function seq(children) {
  return asn1.create(asn1.Class.UNIVERSAL, asn1.Type.SEQUENCE, true, children);
}

function set(children) {
  return asn1.create(asn1.Class.UNIVERSAL, asn1.Type.SET, true, children);
}

function octetString(bytes) {
  return asn1.create(asn1.Class.UNIVERSAL, asn1.Type.OCTETSTRING, false, bytes.toString('binary'));
}

function integer(n) {
  return asn1.create(asn1.Class.UNIVERSAL, asn1.Type.INTEGER, false, asn1.integerToDer(n).getBytes());
}

function nullNode() {
  return asn1.create(asn1.Class.UNIVERSAL, asn1.Type.NULL, false, '');
}

/** [n] IMPLICIT <tag>, constructed (used for SET/SEQUENCE-shaped context tags). */
function implicitConstructed(n, children) {
  return asn1.create(asn1.Class.CONTEXT_SPECIFIC, n, true, children);
}

/** [n] EXPLICIT <node> — wraps node unchanged inside a constructed context tag. */
function explicit(n, node) {
  return asn1.create(asn1.Class.CONTEXT_SPECIFIC, n, true, [node]);
}

const SHA256_OID = '2.16.840.1.101.3.4.2.1';
const RSA_ENCRYPTION_OID = '1.2.840.113549.1.1.1';
const SHA256_WITH_RSA_OID = '1.2.840.113549.1.1.11';
const CONTENT_TYPE_OID = '1.2.840.113549.1.9.3';
const MESSAGE_DIGEST_OID = '1.2.840.113549.1.9.4';
const LDS_SECURITY_OBJECT_OID = '2.23.136.1.1.1';
const SIGNED_DATA_OID = '1.2.840.113549.1.7.2';

function algorithmIdentifier(oidStr, withNull) {
  const children = [oid(oidStr)];
  if (withNull) children.push(nullNode());
  return seq(children);
}

// ---------------------------------------------------------------------------
// 1. Fake CSC (country signing certificate, self-signed root) + DSC (document
//    signer, signed by the CSC) — real RSA-2048 keys, real X.509 certs. Only
//    the DSC cert actually needs to end up in the SOD; the CSC is generated
//    for realism (a real passport's DSC really is CSC-signed) but unused by
//    sodParser.ts, which never needs the CSC (that's `unresolved.cscCertificate`).
// ---------------------------------------------------------------------------

function makeCert(subjectCN, issuerCN, publicKey, signingKey) {
  const cert = pki.createCertificate();
  cert.publicKey = publicKey;
  cert.serialNumber = '01';
  cert.validity.notBefore = new Date('2024-01-01T00:00:00Z');
  cert.validity.notAfter = new Date('2034-01-01T00:00:00Z');
  const subjectAttrs = [{ name: 'commonName', value: subjectCN }, { name: 'countryName', value: 'XT' }];
  const issuerAttrs = [{ name: 'commonName', value: issuerCN }, { name: 'countryName', value: 'XT' }];
  cert.setSubject(subjectAttrs);
  cert.setIssuer(issuerAttrs);
  cert.sign(signingKey, md.sha256.create());
  return cert;
}

console.log('Generating CSC keypair (RSA-2048)...');
const cscKeys = pki.rsa.generateKeyPair({ bits: 2048, e: 0x10001 });
const cscCert = makeCert('Test CSC (NOT REAL)', 'Test CSC (NOT REAL)', cscKeys.publicKey, cscKeys.privateKey);

console.log('Generating DSC keypair (RSA-2048)...');
const dscKeys = pki.rsa.generateKeyPair({ bits: 2048, e: 0x10001 });
const dscCert = makeCert('Test DSC (NOT REAL)', 'Test CSC (NOT REAL)', dscKeys.publicKey, cscKeys.privateKey);
const dscCertAsn1 = pki.certificateToAsn1(dscCert);

// ---------------------------------------------------------------------------
// 2. Synthetic DG1 — a TD3 MRZ. Not parsed as ASN.1 by sodParser.ts (it's
//    hashed as an opaque blob), so this only needs to fit DG1_LENGTH (95
//    bytes); the ICAO-style [APPLICATION 1] wrapper is just for realism.
// ---------------------------------------------------------------------------

const mrzLine1 = 'P<TESTXXTESTPASSPORT<<HOLDER<<<<<<<<<<<<<<<<';
const mrzLine2 = 'L898902C36TEST8001019M3401017<<<<<<<<<<<<<<02';
const mrz = (mrzLine1 + mrzLine2).padEnd(88, '<').slice(0, 88);
const mrzBytes = Buffer.from(mrz, 'ascii');
const dg1 = Buffer.concat([Buffer.from([0x61, 0x5b, 0x5f, 0x1f, 0x58]), mrzBytes]);
if (dg1.length > 95) throw new Error(`DG1 fixture is ${dg1.length} bytes, over the 95-byte budget`);

// ---------------------------------------------------------------------------
// 3. LDSSecurityObject (the SOD's encapsulated content) — real SHA-256 of the
//    real dg1 bytes above, so hexShift() in sodParser.ts finds a real match.
// ---------------------------------------------------------------------------

const dg1Hash = forge.md.sha256.create();
dg1Hash.update(dg1.toString('binary'));
const dg1HashBytes = bin(dg1Hash.digest().getBytes());

const ldsSecurityObject = seq([
  integer(0),
  algorithmIdentifier(SHA256_OID, true),
  seq([seq([integer(1), octetString(dg1HashBytes)])]),
]);
const eContentBytes = der(ldsSecurityObject);

const eContentInfo = seq([
  oid(LDS_SECURITY_OBJECT_OID),
  explicit(0, octetString(eContentBytes)),
]);

// ---------------------------------------------------------------------------
// 4. signedAttrs — deliberately just the messageDigest attribute (see
//    sodParser.ts's getZero(): it only inspects the LAST child of the `[0]`
//    node, so a single-attribute SET trivially satisfies that shape check).
// ---------------------------------------------------------------------------

const eContentDigest = forge.md.sha256.create();
eContentDigest.update(eContentBytes.toString('binary'));
const eContentDigestBytes = bin(eContentDigest.digest().getBytes());

const messageDigestAttr = seq([oid(MESSAGE_DIGEST_OID), set([octetString(eContentDigestBytes)])]);
const signedAttrsImplicit = implicitConstructed(0, [messageDigestAttr]);
const signedAttrsDer = der(signedAttrsImplicit);

// Re-tag [0] IMPLICIT SET (0xA0) as a universal SET (0x31) before hashing/signing —
// this is exactly RFC 5652 section 5.4, and exactly what sodParser.ts's
// extractSignedAttributes() does on the way back out (`'31' + sa.dump.slice(2)`).
const signedAttrsForSigning = Buffer.concat([Buffer.from([0x31]), signedAttrsDer.subarray(1)]);

const sigMd = forge.md.sha256.create();
sigMd.update(signedAttrsForSigning.toString('binary'));
const signatureBytes = bin(dscKeys.privateKey.sign(sigMd));

// ---------------------------------------------------------------------------
// 5. SignerInfo / SignedData / ContentInfo
// ---------------------------------------------------------------------------

const signerInfo = seq([
  integer(1),
  // sid: issuerAndSerialNumber — not read by sodParser.ts at all (it locates
  // fields by shape, not by sid), so this only needs to be a valid SEQUENCE.
  seq([seq([]), integer(1)]),
  algorithmIdentifier(SHA256_OID, true),
  signedAttrsImplicit,
  algorithmIdentifier(SHA256_WITH_RSA_OID, true),
  octetString(signatureBytes),
]);

const signedData = seq([
  integer(1),
  set([algorithmIdentifier(SHA256_OID, true)]),
  eContentInfo,
  implicitConstructed(0, [dscCertAsn1]),
  set([signerInfo]),
]);

const contentInfo = seq([oid(SIGNED_DATA_OID), explicit(0, signedData)]);
const sod = der(contentInfo);

console.log(`Generated fixture: DG1 ${dg1.length}B, SOD ${sod.length}B`);

// ---------------------------------------------------------------------------
// 6. Write the fixture as base64 in a small TS module.
// ---------------------------------------------------------------------------

const outDir = path.join(__dirname, '..', 'src', 'chain', '__fixtures__');
fs.mkdirSync(outDir, { recursive: true });
const outFile = path.join(outDir, 'testPassport.ts');

const contents = `/**
 * Synthetic ICAO-shaped passport data — NOT a real passport, generated by
 * scripts/generate-test-passport-fixture.js. A real self-signed test RSA-2048
 * certificate genuinely signs a genuine LDSSecurityObject over a genuine
 * SHA-256 hash of the DG1 bytes below, so sodParser.ts's buildCircuitInputs()
 * (which walks real ASN.1/CMS shapes) succeeds against it exactly as it would
 * against a real passport's SOD — used only to exercise the registration
 * pipeline above the native NFC layer without a physical passport in hand.
 * No Active Authentication (DG15 is empty) and no real certificate-chain
 * trust: this proves nothing about a real identity.
 */
export const TEST_PASSPORT_DG1_BASE64 = '${dg1.toString('base64')}';
export const TEST_PASSPORT_DG15_BASE64 = '';
export const TEST_PASSPORT_SOD_BASE64 = '${sod.toString('base64')}';
`;

fs.writeFileSync(outFile, contents);
console.log(`Wrote ${outFile}`);
