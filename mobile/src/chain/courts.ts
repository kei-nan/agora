/**
 * Courts pallet integration.
 *
 * Level 0 rulings are issued by an AI judge (hash stored on IPFS on-chain).
 * Citizens may appeal to a random jury (Level 1) or a constitutional panel (Level 2).
 *
 * TODO: wire jury-selection RNG to an on-chain VRF once available.
 */
import { KeyringPair } from '@polkadot/keyring/types';
import { getApi } from './api';
import { submitExtrinsic } from './submitExtrinsic';

export type CaseSubject =
  | { General: null }
  | { LawChallenge: { law_id: number } }
  | { TreasuryDispute: { department_id: number } }
  | { CitizenConduct: { nullifier: Uint8Array; suspension_blocks: number | null } };

/** Mirrors `pallet_courts::Verdict` (`pallets/pallet-courts/src/lib.rs`) — a fieldless enum. */
export type Verdict = 'Upheld' | 'Overturned';

/** Mirrors `pallet_courts::CaseStatus` (`pallets/pallet-courts/src/lib.rs`) — a fieldless enum. */
export type CaseStatus =
  | 'Filed'
  | 'AIRulingIssued'
  | 'InJuryAppeal'
  | 'JurySeated'
  | 'FinalRuling'
  | 'Enforced';

/**
 * Decodes a `@polkadot/api` `CaseSubject` enum codec value back into this file's
 * `CaseSubject` TS union. Field names inside each variant (`law_id`, `department_id`,
 * `nullifier`, `suspension_blocks`) are read verbatim, not camelCased — Substrate/scale-info
 * only camelCases call/storage *method* names for `@polkadot/api` convenience, not struct
 * field names, which is why `fileCase`'s write-side `CaseSubject` values above already use
 * the exact snake_case Rust field names (`{ LawChallenge: { law_id: 3 } }`); this is the same
 * convention applied in the read direction.
 */
function decodeCaseSubject(raw: any): CaseSubject {
  switch (raw.type as string) {
    case 'General':
      return { General: null };
    case 'LawChallenge':
      return { LawChallenge: { law_id: raw.value.law_id.toNumber() } };
    case 'TreasuryDispute':
      return { TreasuryDispute: { department_id: raw.value.department_id.toNumber() } };
    case 'CitizenConduct': {
      const inner = raw.value;
      return {
        CitizenConduct: {
          nullifier: inner.nullifier.toU8a(),
          suspension_blocks: inner.suspension_blocks.isSome
            ? inner.suspension_blocks.unwrap().toNumber()
            : null,
        },
      };
    }
    default:
      throw new Error(`decodeCaseSubject: unrecognized CaseSubject variant '${raw.type}'`);
  }
}

export interface CaseSummary {
  caseId: number;
  /** SS58 address of the filer — `T::AutoChallengeAccount` for system-initiated cases. */
  filer: string;
  status: CaseStatus;
  /** Hex-encoded IPFS CID of the AI ruling's reasoning document, once issued; else null. */
  rulingIpfsHash: string | null;
  subject: CaseSubject;
}

/**
 * Lists every case on chain, oldest first. `pallet_courts::Cases` is keyed `0..NextCaseId`
 * contiguously — both `file_case` and `auto_file_case` (`pallets/pallet-courts/src/lib.rs`)
 * read `NextCaseId`, insert at exactly that id, then `put(id + 1)`; neither path ever skips
 * or reuses an id — so iterating `Cases.entries()` and sorting by id (rather than looping
 * `0..NextCaseId` and querying one at a time) is equivalent and cheaper.
 */
export async function fetchAllCases(): Promise<CaseSummary[]> {
  const api = await getApi();
  const entries = await api.query.courts.cases.entries();
  const cases: CaseSummary[] = [];
  for (const [key, value] of entries) {
    if ((value as any).isNone) continue;
    const caseId = (key.args[0] as any).toNumber();
    const [filer, status, rulingHashOpt, subject] = (value as any).unwrap();
    cases.push({
      caseId,
      filer: filer.toString(),
      status: status.type,
      rulingIpfsHash: (rulingHashOpt as any).isSome ? (rulingHashOpt as any).unwrap().toHex() : null,
      subject: decodeCaseSubject(subject),
    });
  }
  return cases.sort((a, b) => a.caseId - b.caseId);
}

export interface CaseDetail extends CaseSummary {
  /** Final verdict, once `Rulings[case_id]` is populated (status FinalRuling/Enforced); else null. */
  ruling: Verdict | null;
  /** SS58 addresses of the selected jurors, empty if no jury has been seated yet. */
  juryPool: string[];
  juryTally: { upheld: number; overturned: number };
  /** Block the AI ruling was issued at, or null if the case hasn't reached AIRulingIssued yet. */
  aiRulingBlock: number | null;
  /** `aiRulingBlock + AppealWindowBlocks`, or null when `aiRulingBlock` is null. */
  appealDeadlineBlock: number | null;
}

/** Reads every piece of on-chain state for one case. Returns null if `caseId` doesn't exist. */
export async function fetchCaseDetail(caseId: number): Promise<CaseDetail | null> {
  const api = await getApi();
  const caseOpt = await api.query.courts.cases(caseId);
  if ((caseOpt as any).isNone) return null;
  const [filer, status, rulingHashOpt, subject] = (caseOpt as any).unwrap();

  const [rulingOpt, juryPoolOpt, tallyRaw, aiRulingBlockOpt] = await Promise.all([
    api.query.courts.rulings(caseId),
    api.query.courts.juryPool(caseId),
    api.query.courts.juryTally(caseId),
    api.query.courts.aiRulingBlock(caseId),
  ]);
  const appealWindowBlocks = (api.consts.courts.appealWindowBlocks as any).toNumber();

  const aiRulingBlock = (aiRulingBlockOpt as any).isSome
    ? (aiRulingBlockOpt as any).unwrap().toNumber()
    : null;
  const [upheld, overturned] = tallyRaw as any; // JuryTally is ValueQuery — defaults to (0, 0).

  return {
    caseId,
    filer: filer.toString(),
    status: status.type,
    rulingIpfsHash: (rulingHashOpt as any).isSome ? (rulingHashOpt as any).unwrap().toHex() : null,
    subject: decodeCaseSubject(subject),
    ruling: (rulingOpt as any).isSome ? ((rulingOpt as any).unwrap().type as Verdict) : null,
    juryPool: (juryPoolOpt as any).isSome
      ? Array.from((juryPoolOpt as any).unwrap() as any).map((juror: any) => juror.toString())
      : [],
    juryTally: { upheld: upheld.toNumber(), overturned: overturned.toNumber() },
    aiRulingBlock,
    appealDeadlineBlock: aiRulingBlock === null ? null : aiRulingBlock + appealWindowBlocks,
  };
}

/**
 * Whether `jurorAddress` has already cast a vote on `caseId` — pallet-courts' `JuryVotes`
 * (`(case_id, AccountId) -> Verdict`, `OptionQuery`, a single-key `StorageMap` whose key
 * happens to be a tuple, not a genuine double map — same calling convention `governance.ts`
 * already uses for `Delegations`/`ReferendumHasVoted`: pass the key parts as one array arg).
 */
export async function hasJurorVoted(caseId: number, jurorAddress: string): Promise<boolean> {
  const api = await getApi();
  const vote = await api.query.courts.juryVotes([caseId, jurorAddress]);
  return (vote as any).isSome;
}

/** Reads `OracleAccount` — the designated AI oracle, or null if governance hasn't set one yet. */
export async function getOracleAccount(): Promise<string | null> {
  const api = await getApi();
  const account = await api.query.courts.oracleAccount();
  return (account as any).isSome ? (account as any).unwrap().toString() : null;
}

/**
 * Pure client-side port of `pallet_courts::Pallet::is_filer_or_oracle`
 * (`pallets/pallet-courts/src/lib.rs`) — true if `callerAddress` is the case's own filer or
 * the designated oracle account.
 *
 * Deliberately omits the Rust helper's third branch — `system_case && CitizenChecker::
 * is_active_citizen(who)`, which additionally allows ANY active citizen to act on a
 * system-initiated case (`filer == T::AutoChallengeAccount`, e.g. an auto-filed law
 * challenge). `AutoChallengeAccount` is a `Config::Get` constant baked into the runtime
 * binary, not on-chain storage or a runtime API this mobile app can read — there is no
 * reasonable way to query it from here. Effect: for a system-filed case, this function
 * under-approximates what the chain would actually accept (it may report `false` for a
 * caller the chain would in fact authorize); it never over-approximates (an address this
 * returns `true` for really is authorized on-chain). This is the "citizen appealing a
 * system-auto-filed case" edge case the task that produced this function explicitly allowed
 * omitting.
 */
export function isFilerOrOracle(
  caseDetail: CaseDetail,
  callerAddress: string,
  oracleAddress: string | null,
): boolean {
  return callerAddress === caseDetail.filer || (oracleAddress !== null && callerAddress === oracleAddress);
}

/**
 * Pure client-side port of `pallet_courts::Pallet::is_ruled_against_party`
 * (`pallets/pallet-courts/src/lib.rs`) — true only for a `CitizenConduct` case whose
 * `nullifier` matches `callerCitizenNullifier` (the caller's own registered identity
 * nullifier, e.g. from `identity.ts`'s `api.query.identity.citizenNullifier(address)`); false
 * for every other subject and false when there's no match.
 */
export function isRuledAgainstParty(
  caseDetail: CaseDetail,
  callerCitizenNullifier: Uint8Array | null,
): boolean {
  if (callerCitizenNullifier === null || !('CitizenConduct' in caseDetail.subject)) return false;
  const caseNullifier = caseDetail.subject.CitizenConduct.nullifier;
  if (caseNullifier.length !== callerCitizenNullifier.length) return false;
  return caseNullifier.every((byte, i) => byte === callerCitizenNullifier[i]);
}

export async function fileCase(
  pair: KeyringPair,
  subject: CaseSubject,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.courts.fileCase(subject), pair);
}

export async function appealRuling(
  pair: KeyringPair,
  caseId: number,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.courts.appealRuling(caseId), pair);
}

/**
 * `cast_jury_vote(origin, case_id, verdict)` takes a `Verdict` enum
 * (`Upheld`/`Overturned`), not a bare boolean — a previous version of this wrapper
 * took `verdict: boolean` and passed it straight through, which either fails to
 * encode or silently coerces to the wrong variant, potentially recording a jury
 * vote as the opposite of what the caller intended.
 */
export async function castJuryVote(
  pair: KeyringPair,
  caseId: number,
  verdict: Verdict,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.courts.castJuryVote(caseId, verdict), pair);
}
