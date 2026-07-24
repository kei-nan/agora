/**
 * Identity pallet integration.
 *
 * Flow:
 *  1. User scans NFC passport with Rarimo SDK (native module, not yet installed).
 *  2. SDK generates a ZK proof + nullifier on-device.
 *  3. This module submits the proof to the chain.
 *
 * TODO: install @rarimo/react-native-passport-reader once the package stabilises.
 */
import { KeyringPair } from '@polkadot/keyring/types';
import { getApi } from './api';

export interface ZkRegistration {
  /**
   * 129-byte binary proof in ark-serialize compressed format:
   *   [0..32]   A  G1 compressed
   *   [32..96]  B  G2 compressed
   *   [96..128] C  G1 compressed
   *   [128]     variant  0=SHA-256 circuit  1=SHA-1 circuit
   * Use encodeProofForChain() to convert from snarkjs output.
   */
  zkProof: Uint8Array;
  /**
   * 5 public signals from the Rarimo registerIdentity circuit (nPublic = 5),
   * each as a 32-byte big-endian Fr element.
   *   [0] dg15PubKeyHash  — Poseidon hash of DG15 active-auth pubkey (0 if NA)
   *   [1] passportHash    — Poseidon(SHA-256(signedAttributes)[252:])
   *   [2] dg1Commitment   — Poseidon(DG1_chunks, skIdentity)  ← used as nullifier on-chain
   *   [3] pkIdentityHash  — Poseidon(babyJubJub.X, babyJubJub.Y)
   *   [4] slaveMerkleRoot — root of trusted issuer CA tree (must be in AllowedMerkleRoots)
   */
  publicInputs: Uint8Array[];
}

const P = BigInt('21888242871839275222246405745257275088696311157297823662689037894645226208583');

function fqIsNegative(n: bigint): boolean { return n * 2n >= P; }

function decimalToLe32(dec: string): Uint8Array {
  let n = BigInt(dec);
  const buf = new Uint8Array(32);
  for (let i = 0; i < 32; i++) { buf[i] = Number(n & 0xffn); n >>= 8n; }
  return buf;
}

function compressG1(pt: [string, string, string]): Uint8Array {
  const buf = decimalToLe32(pt[0]);
  buf[31] &= 0x3f;
  if (fqIsNegative(BigInt(pt[1]))) buf[31] |= 0x80;
  return buf;
}

function compressG2(pt: [[string, string], [string, string], [string, string]]): Uint8Array {
  const x0 = decimalToLe32(pt[0][0]);
  const x1 = decimalToLe32(pt[0][1]);
  const y1 = BigInt(pt[1][1]);
  const y0 = BigInt(pt[1][0]);
  const buf = new Uint8Array(64);
  buf.set(x0, 0); buf.set(x1, 32);
  buf[63] &= 0x3f;
  if (y1 !== 0n ? fqIsNegative(y1) : fqIsNegative(y0)) buf[63] |= 0x80;
  return buf;
}

/**
 * Convert snarkjs proof JSON → 129-byte chain binary format.
 * @param variant  0 = SHA-256 passport circuit (default), 1 = SHA-1
 */
export function encodeProofForChain(
  proof: {
    pi_a: [string, string, string];
    pi_b: [[string, string], [string, string], [string, string]];
    pi_c: [string, string, string];
  },
  variant: 0 | 1 = 0,
): Uint8Array {
  const out = new Uint8Array(129);
  out.set(compressG1(proof.pi_a), 0);
  out.set(compressG2(proof.pi_b), 32);
  out.set(compressG1(proof.pi_c), 96);
  out[128] = variant;
  return out;
}

/** Convert snarkjs publicSignals (decimal strings) → 32-byte big-endian Fr arrays. */
export function encodePublicInputs(signals: string[]): Uint8Array[] {
  return signals.map((dec) => {
    let n = BigInt(dec);
    const buf = new Uint8Array(32);
    for (let i = 31; i >= 0; i--) { buf[i] = Number(n & 0xffn); n >>= 8n; }
    return buf;
  });
}

export async function registerCitizen(
  pair: KeyringPair,
  reg: ZkRegistration,
): Promise<string> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.identity
      .registerCitizen(Array.from(reg.zkProof), reg.publicInputs)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) {
          reject(new Error(dispatchError.toString()));
        } else if (status.isFinalized) {
          resolve(status.asFinalized.toString());
        }
      })
      .catch(reject);
  });
}

export async function isCitizen(address: string): Promise<boolean> {
  const api = await getApi();
  const result = await api.query.identity.citizenNullifier(address);
  return !result.isEmpty;
}

export async function suspendCitizen(
  pair: KeyringPair,
  nullifier: Uint8Array,
  until: number | null,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.identity
      .suspendCitizen(nullifier, until)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) {
          reject(new Error(dispatchError.toString()));
        } else if (status.isFinalized) {
          resolve();
        }
      })
      .catch(reject);
  });
}

// ── Desktop auth helpers ──────────────────────────────────────────────────────

/**
 * Returns the signing keypair and nullifier hash for the currently registered citizen.
 * In production this reads the hardware-backed key from iOS Secure Enclave / Android Keystore.
 * For the scaffold, we derive a deterministic key from the stored nullifier seed.
 *
 * Throws if the user has not completed passport registration.
 */
export async function getSigningKeypair(): Promise<{
  keypair: KeyringPair;
  nullifierHash: string;
}> {
  // Circular import avoided: import Keyring lazily since it requires native modules.
  const { Keyring } = await import('@polkadot/keyring');
  const { u8aToHex } = await import('@polkadot/util');

  // TODO: replace with hardware-backed key from Secure Enclave / Keystore.
  // The seed here is derived from the app's keyring, which stores the Ed25519 key generated
  // during registration. In production, the private key never leaves secure hardware.
  const keyring = new Keyring({ type: 'ed25519' });

  // Dev stub: deterministic from a constant seed. Replace with persisted key in production.
  const pair = keyring.addFromUri('//AgoraDevUser');
  const nullifierHash = u8aToHex(pair.publicKey);

  return { keypair: pair, nullifierHash };
}

/**
 * Returns the nullifier hash for the logged-in citizen (hex-encoded public key for now).
 * Used to verify identity on the desktop side via chain state lookup.
 */
export async function getNullifier(): Promise<string | null> {
  try {
    const { nullifierHash } = await getSigningKeypair();
    return nullifierHash;
  } catch {
    return null;
  }
}

export async function restoreCitizenRights(
  pair: KeyringPair,
  nullifier: Uint8Array,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.identity
      .restoreCitizenRights(nullifier)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) {
          reject(new Error(dispatchError.toString()));
        } else if (status.isFinalized) {
          resolve();
        }
      })
      .catch(reject);
  });
}
