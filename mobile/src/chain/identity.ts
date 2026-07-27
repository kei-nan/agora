/**
 * Identity pallet integration (crate `pallet-identity-zk`, runtime module
 * name `Identity` at pallet index 8 — see runtime/src/lib.rs — so the
 * @polkadot/api section is `api.query.identity` / `api.tx.identity`).
 *
 * Passport NFC scan + on-device Rarimo ZK proof generation is NOT
 * implemented yet (RegisterScreen.tsx still has the scan/liveness/proving
 * steps stubbed out with TODOs). Consequently getSigningKeypair() below
 * derives a DEV-ONLY keypair from a fixed, publicly-known mnemonic instead of
 * a hardware-backed key — this is not fit for anything but local development
 * against a --dev chain, matching how the rest of this codebase marks
 * dev/passthrough pieces (e.g. PassthroughMACIVerifier in
 * runtime/src/configs/mod.rs). Real key custody must live in iOS Secure
 * Enclave / Android Keystore per CLAUDE.md's Identity System section.
 */
import { Keyring } from '@polkadot/keyring';
import { KeyringPair } from '@polkadot/keyring/types';
import { blake2AsHex, cryptoWaitReady } from '@polkadot/util-crypto';
import { getApi } from './api';

/**
 * Rarimo registerIdentity ZK proof, ready to submit to register_citizen.
 * See pallets/pallet-identity/src/lib.rs for the exact 5-signal layout this
 * must contain (dg15PubKeyHash, passportHash, dg1Commitment, pkIdentityHash,
 * slaveMerkleRoot).
 */
export interface ZkRegistration {
  zkProof: Uint8Array;
  /** Exactly 5 32-byte public signals, in Rarimo registerIdentity order. */
  publicInputs: Uint8Array[];
}

// DEV-ONLY — this is the well-known Substrate test mnemonic. Every install of
// this build derives the SAME keypair from it. It exists purely so the
// already-wired chain-calling code (voting.ts, constitution.ts, courts.ts,
// governance.ts) has a real KeyringPair with a working .sign() to exercise
// end-to-end before hardware-backed key custody exists. Never use this for
// anything beyond a local --dev chain.
const DEV_ONLY_MNEMONIC =
  'bottom drive obey lake curtain smoke basket hold race lonely fit walk';

let _pairPromise: Promise<KeyringPair> | null = null;

function devKeyringPair(): Promise<KeyringPair> {
  if (!_pairPromise) {
    _pairPromise = (async () => {
      await cryptoWaitReady();
      const keyring = new Keyring({ type: 'sr25519', ss58Format: 42 });
      return keyring.addFromUri(DEV_ONLY_MNEMONIC, { name: 'agora-dev' });
    })();
  }
  return _pairPromise;
}

/**
 * Returns a real, working KeyringPair (DEV-ONLY — see module doc) plus a
 * nullifier hash.
 *
 * The returned `nullifierHash` is a PLACEHOLDER, not a real Rarimo nullifier.
 * The real nullifier is Poseidon2(national_id || country_code), computed
 * on-device from the passport NFC scan (public_inputs[2] / dg1Commitment in
 * pallet-identity's register_citizen) — that flow doesn't exist yet (see
 * RegisterScreen.tsx TODOs). The value below is just
 * blake2(keypair.publicKey), so it's deterministic per dev keypair for
 * testing flows like AuthScreen's QR POST body, but it will NOT match
 * whatever CitizenNullifier actually stores on-chain for this address, and
 * must not be treated as a real identity proof.
 */
export async function getSigningKeypair(): Promise<{ keypair: KeyringPair; nullifierHash: string }> {
  const keypair = await devKeyringPair();
  const nullifierHash = blake2AsHex(keypair.publicKey, 256);
  return { keypair, nullifierHash };
}

/** Real query against pallet-identity's CitizenNullifier storage map. */
export async function isCitizen(address: string): Promise<boolean> {
  const api = await getApi();
  const nullifier = await api.query.identity.citizenNullifier(address);
  return (nullifier as any).isSome;
}

/** Submits a register_citizen extrinsic with a Rarimo ZK proof + public signals. */
export async function registerCitizen(reg: ZkRegistration): Promise<void> {
  const api = await getApi();
  const { keypair } = await getSigningKeypair();
  return new Promise((resolve, reject) => {
    api.tx.identity
      .registerCitizen(reg.zkProof, reg.publicInputs)
      .signAndSend(keypair, ({ status, dispatchError }) => {
        if (dispatchError) {
          reject(new Error(dispatchError.toString()));
        } else if (status.isFinalized) {
          resolve();
        }
      })
      .catch(reject);
  });
}
