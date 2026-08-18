/**
 * Persists the citizen's own registration/reverification pipeline progress
 * (passport scan -> proof assembly -> OPRF committee round-trip -> chain
 * submission) across app restarts, so `RegisterScreen.tsx` (or whatever UI
 * eventually drives it) can resume mid-pipeline instead of starting over
 * every time the app is killed. Same on-disk approach as `keystoreWallet.ts`
 * — a JSON file under `RNFS.DocumentDirectoryPath`, a `version` field, and an
 * "unrecognized shape -> treat as absent" defensive read — see that module's
 * doc comment for why that location/library combination was already chosen
 * for this app.
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

const REGISTRATION_STATE_FILE_PATH = `${RNFS.DocumentDirectoryPath}/agora-registration-status.json`;

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
   * Liveness (blink/turn) is required either way — this stage is never
   * reached at all if the liveness challenges themselves failed.
   */
  | { stage: 'LivenessVerified'; faceMatched: boolean; matchSkippedReason?: string }
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

/** Removes `address`'s entry entirely (used once the chain confirms `Active`, see `registrationReconciler.ts`). Serialized — see `serialize`. */
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
