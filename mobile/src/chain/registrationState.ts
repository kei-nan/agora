/**
 * Persists the citizen's own registration/reverification pipeline progress
 * (passport scan -> proof assembly -> OPRF committee round-trip -> chain
 * submission) across app restarts, so `RegisterScreen.tsx` (or whatever UI
 * eventually drives it) can resume mid-pipeline instead of starting over
 * every time the app is killed. Same on-disk *library* as `keystoreWallet.ts`
 * (`react-native-fs`) and the same "JSON file, `version` field, unrecognized
 * shape -> treat as absent" defensive-read shape — see that module's doc
 * comment for why that library was already chosen for this app — but a
 * deliberately different *directory*. Unlike the signing key material in
 * `keystoreWallet.ts`, this file can reveal that its owner is mid-registration,
 * or that a face-match/liveness check specifically failed for them
 * (`LivenessVerified`'s `faceMatched`/`matchSkippedReason`) — sensitive on its
 * own to anyone with casual access to the device (e.g. an abusive partner),
 * even before registration finishes. So this lives under
 * `RNFS.CachesDirectoryPath`, not `DocumentDirectoryPath`: OS-evictable,
 * cache-tier storage rather than treated as durable user data — the same
 * tier `FaceCaptureModule.kt`'s `sweepCaptureFiles` already uses for the raw
 * liveness/face-match capture photos themselves (`reactContext.cacheDir`).
 * Losing this file early just means the pipeline-progress UI falls back to
 * re-deriving what it can from chain state (see `registrationReconciler.ts`)
 * instead of resuming exactly where it left off — never data loss for
 * anything the chain itself would consider authoritative. Users can also
 * clear it explicitly and immediately via `clearRegistrationStatus` from
 * `RegistrationStatusScreen.tsx`'s "Clear saved status" action, rather than
 * waiting on the OS to evict it.
 *
 * Deliberately has NO `@polkadot/api` import and no notion of "chain state":
 * this is a dumb, address-scoped key-value store. `registrationReconciler.ts`
 * is the layer that knows how to reconcile a persisted record here against
 * live chain storage (and, when the chain has moved on — the citizen is now
 * an on-chain-confirmed `Active`/`ReverificationDue`/`Suspended` citizen —
 * supersedes it entirely). That's also why `RegistrationStatus`'s three
 * chain-derived variants (`Active`, `ReverificationDue`, `Suspended`) are
 * excluded from `PersistableStatus`: this module only ever persists pipeline
 * progress that has no on-chain record yet, never a fact the chain itself
 * already answers authoritatively. `NotStarted` is excludable in spirit for
 * the same reason but stays in the persistable union for typing convenience;
 * the important invariant is the one enforced procedurally below —
 * `writeRegistrationStatus` is simply never called with it anywhere in this
 * codebase, since "no file / no entry" already means exactly that (see
 * `readRegistrationStatus`'s doc comment).
 *
 * Keyed by SS58 address (not a single global record) because the keystore
 * wallet's address is stable per-install but this file may outlive a single
 * signing key across debugging/reinstall scenarios, and a future multi-
 * account build could plausibly track more than one citizen's pipeline
 * state at once — no reason to force a single-address assumption into the
 * on-disk shape when scoping by address costs nothing.
 */
import RNFS from 'react-native-fs';

/** Bumped if the on-disk shape ever changes; unrecognized versions are treated as "no file on disk". */
const REGISTRATION_STATE_FILE_VERSION = 1;

const REGISTRATION_STATE_FILE_PATH = `${RNFS.CachesDirectoryPath}/agora-registration-status.json`;

/**
 * `committeeSlots` (on the four OPRF-tracking stages below) holds the
 * committee slot indices a registration query is outstanding against. Every
 * registration query goes to all 5 OPRF committees symmetrically — not
 * routed to a single one via a hash (see `docs/project/changelog/073.md`) —
 * so in practice this always holds all `NUM_OPRF_COMMITTEES` (5, see
 * `identity.ts`) slot indices, `[0, 1, 2, 3, 4]`. The type stays `number[]`
 * (not a fixed-length tuple) since nothing about the reconciliation logic
 * actually depends on the count being exactly 5.
 */
export type RegistrationStatus =
  | { stage: 'NotStarted' }
  | { stage: 'PassportScanned' }
  /**
   * The face-match/liveness gate (`RegisterScreen.tsx`, `../native/faceMatch.ts`)
   * passed. `faceMatched` is `false` whenever the match itself was `skipped`
   * (e.g. an undecodable DG2 photo — see `FaceMatchModule.kt`) as well as
   * whenever it genuinely didn't match; `matchSkippedReason` disambiguates.
   * Liveness — blink/turn, or the QR-code alternate challenge (see
   * `../screens/qrLivenessChallenge.ts`) — is required either way; this
   * stage is never reached at all if the liveness challenge itself failed.
   *
   * `deviceIntegrityCaptured`/`deviceIntegrityReason` record whether the
   * best-effort Play Integrity attestation (`../chain/deviceIntegrity.ts`)
   * was obtained for this attempt — never gates registration, purely a
   * defense-in-depth signal, see that module's doc comment for the full
   * accounting of what it does and doesn't prove. The raw token itself is
   * deliberately NOT persisted here (nothing downstream consumes it yet, so
   * there's no reason to keep a second copy of security-sensitive material
   * in this already cache-tier-but-sensitive file — see this module's own
   * doc comment above).
   */
  | {
      stage: 'LivenessVerified';
      faceMatched: boolean;
      matchSkippedReason?: string;
      deviceIntegrityCaptured?: boolean;
      deviceIntegrityReason?: string;
    }
  | { stage: 'ProofMaterialAssembled' }
  | {
      stage: 'OprfQuerySubmitted';
      queryId: string;
      committeeSlots: number[];
      submittedAtBlock: number;
      slaExpiresAtBlock: number;
    }
  | {
      stage: 'AwaitingCommitteeRound1';
      queryId: string;
      committeeSlots: number[];
      submittedAtBlock: number;
      slaExpiresAtBlock: number;
    }
  | {
      stage: 'AwaitingCommitteeRound2';
      queryId: string;
      committeeSlots: number[];
      submittedAtBlock: number;
      slaExpiresAtBlock: number;
    }
  | { stage: 'ProofCombining'; queryId: string; committeeSlots: number[] }
  | { stage: 'ProofReady' }
  | { stage: 'ChainSubmissionPending' }
  | { stage: 'Failed'; failedStage: string; reason: string; retryable: boolean; expiresAtBlock?: number }
  | { stage: 'Active' }
  | { stage: 'ReverificationDue' }
  | { stage: 'Suspended'; until: number | null };

/**
 * The subset of `RegistrationStatus` this module ever writes to disk.
 * `Active`/`ReverificationDue`/`Suspended` are facts the chain itself
 * answers authoritatively (see `registrationReconciler.ts`) and are never
 * persisted here — persisting them would just be a second, staler copy of
 * something a live query already gives for free.
 */
export type PersistableStatus = Exclude<
  RegistrationStatus,
  { stage: 'Active' } | { stage: 'ReverificationDue' } | { stage: 'Suspended' }
>;

interface RegistrationStateFileEntry {
  status: PersistableStatus;
  updatedAtMs: number;
}

interface RegistrationStateFile {
  version: number;
  byAddress: Record<string, RegistrationStateFileEntry>;
}

function isRegistrationStateFile(value: unknown): value is RegistrationStateFile {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as any).byAddress === 'object' &&
    (value as any).byAddress !== null
  );
}

async function readFile(): Promise<RegistrationStateFile> {
  const exists = await RNFS.exists(REGISTRATION_STATE_FILE_PATH);
  if (!exists) return { version: REGISTRATION_STATE_FILE_VERSION, byAddress: {} };

  const raw = await RNFS.readFile(REGISTRATION_STATE_FILE_PATH, 'utf8');
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return { version: REGISTRATION_STATE_FILE_VERSION, byAddress: {} }; // Corrupt file — treated the same as "no file" below.
  }
  if (!isRegistrationStateFile(parsed) || parsed.version !== REGISTRATION_STATE_FILE_VERSION) {
    return { version: REGISTRATION_STATE_FILE_VERSION, byAddress: {} };
  }
  return parsed;
}

async function writeFile(file: RegistrationStateFile): Promise<void> {
  await RNFS.writeFile(REGISTRATION_STATE_FILE_PATH, JSON.stringify(file), 'utf8');
}

/**
 * Serializes every write (including clears) through a single in-module
 * promise chain. Unlike `keystoreWallet.ts` (which writes its file exactly
 * once, on first-ever use), this module is read-modify-write against a
 * *shared* file across many call sites over the app's lifetime — the OPRF
 * polling loop, screen-level "mark passport scanned" calls, the reconciler,
 * etc. Without this, two concurrent writers could both read the same
 * pre-write file, then each write back their own modified copy, with
 * whichever write lands second silently discarding the first one's update.
 * Chaining onto `_writeChain` (rather than e.g. a mutex library) forces every
 * `fn` to run strictly after the previous one has settled, so each write
 * always starts from the previous write's result.
 */
let _writeChain: Promise<void> = Promise.resolve();
function serialize<T>(fn: () => Promise<T>): Promise<T> {
  const result = _writeChain.then(fn, fn);
  _writeChain = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

/**
 * Returns the persisted pipeline record for `address`, or `null` if there is
 * none (no file on disk, file corrupt/wrong-version, or no entry for this
 * address). Callers treat `null` as `NotStarted` themselves — this module
 * never writes a `NotStarted` entry, since "absent" already means exactly
 * that, and writing it explicitly would just be a redundant entry to keep in
 * sync.
 */
export async function readRegistrationStatus(address: string): Promise<PersistableStatus | null> {
  const file = await readFile();
  const entry = file.byAddress[address];
  return entry ? entry.status : null;
}

/** Read-modify-write of the whole file, updating only this address's entry. Writes are serialized — see `serialize`. */
export async function writeRegistrationStatus(address: string, status: PersistableStatus): Promise<void> {
  return serialize(async () => {
    const file = await readFile();
    file.byAddress[address] = { status, updatedAtMs: Date.now() };
    await writeFile(file);
  });
}

/**
 * Removes `address`'s entry entirely. Two call sites: automatically once the
 * chain confirms `Active` (see `registrationReconciler.ts`), and on-demand
 * from the user's own explicit "Clear saved status" action
 * (`RegistrationStatusScreen.tsx`) so they don't have to wait on
 * `RegistrationStatus`'s cache-tier storage (see this module's doc comment)
 * to eventually be evicted by the OS. Serialized — see `serialize`.
 */
export async function clearRegistrationStatus(address: string): Promise<void> {
  return serialize(async () => {
    const file = await readFile();
    if (!(address in file.byAddress)) return; // Nothing to do — avoid a needless write.
    delete file.byAddress[address];
    await writeFile(file);
  });
}

/** Test-only escape hatch — deletes the whole on-disk file. Production code never needs this within a process lifetime. */
export async function _resetAllRegistrationStateForTests(): Promise<void> {
  return serialize(async () => {
    const exists = await RNFS.exists(REGISTRATION_STATE_FILE_PATH);
    if (exists) await RNFS.unlink(REGISTRATION_STATE_FILE_PATH);
  });
}
