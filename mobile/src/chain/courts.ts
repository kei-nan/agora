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
  | { CitizenConduct: { nullifier: Uint8Array; suspension_blocks: number | null } }
  | { TierConflict: { law_id: number } };

/**
 * `pallet_courts::file_case`'s ZK citizenship proof, required for anonymized filings
 * (`LawChallenge`/`TreasuryDispute`/`TierConflict` — see `CaseFiler`'s doc comment on the
 * Rust side for why those case types file under a nullifier instead of a plain `AccountId`)
 * and rejected for `CitizenConduct`/`General`. Mirrors `identity.ts`'s `OuterProofPayload`
 * convention, minus `outerCount`: unlike `identity.ts`'s callers, nothing here needs to
 * validate `publicInputs`'s shape client-side before submission.
 */
export interface CaseFilingProof {
  zkProof: Uint8Array;
  publicInputs: Uint8Array[];
}

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
    case 'TierConflict':
      return { TierConflict: { law_id: raw.value.law_id.toNumber() } };
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

/**
 * Mirrors `pallet_courts::CaseFiler<AccountId>` (`pallets/pallet-courts/src/lib.rs`) — who
 * filed a case. `LawChallenge`/`TreasuryDispute`/`TierConflict` cases are filed under the
 * filer's ZKPassport `scoped_nullifier` rather than their `AccountId` (see that type's Rust
 * doc comment for why: the filer risks retaliation from the institutional power they're
 * challenging); `CitizenConduct`/`General` cases still file under the plain signing account,
 * since the accused has a legitimate interest in knowing their accuser.
 */
export type CaseFiler =
  | { kind: 'account'; address: string }
  | { kind: 'nullifier'; value: Uint8Array };

function decodeCaseFiler(raw: any): CaseFiler {
  switch (raw.type as string) {
    case 'Account':
      return { kind: 'account', address: raw.value.toString() };
    case 'Nullifier':
      return { kind: 'nullifier', value: raw.value.toU8a() };
    default:
      throw new Error(`decodeCaseFiler: unrecognized CaseFiler variant '${raw.type}'`);
  }
}

/** Byte-for-byte equality check for two 32-byte identity nullifiers. */
function nullifiersEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  return a.every((byte, i) => byte === b[i]);
}

export interface CaseSummary {
  caseId: number;
  /**
   * Who filed the case — an `AccountId` for `CitizenConduct`/`General`, or an anonymizing
   * ZK nullifier for `LawChallenge`/`TreasuryDispute`/`TierConflict`. See `CaseFiler`.
   * `T::AutoChallengeAccount` (as `{ kind: 'account' }`) for system-initiated cases.
   */
  filer: CaseFiler;
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
      filer: decodeCaseFiler(filer),
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
    filer: decodeCaseFiler(filer),
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

/**
 * Reads `OracleMembers` — the Oracle Council roster (`BoundedVec<AccountId, MaxOracleMembers>`,
 * `ValueQuery`, so always present, possibly empty — never an `Option`). Replaces the earlier
 * single `OracleAccount` storage item removed by the Oracle Council migration
 * (`pallets/pallet-courts/src/lib.rs`).
 */
export async function getOracleMembers(): Promise<string[]> {
  const api = await getApi();
  const members = await api.query.courts.oracleMembers();
  return Array.from(members as unknown as Iterable<{ toString(): string }>).map((member) =>
    member.toString(),
  );
}

/**
 * Pure client-side port of `pallet_courts::Pallet::is_filer_or_oracle`
 * (`pallets/pallet-courts/src/lib.rs`) — true if the caller is the case's own filer or
 * a current member of the Oracle Council (`OracleMembers`).
 *
 * `caseDetail.filer` is now a `CaseFiler` (see that type's doc comment), not a plain
 * address: for a `{ kind: 'account' }` filer this compares `callerAddress` the same way
 * the old string comparison did; for a `{ kind: 'nullifier' }` filer (an anonymized
 * `LawChallenge`/`TreasuryDispute`/`TierConflict` filing) it instead compares
 * `callerCitizenNullifier` — the caller's own registered identity nullifier, e.g. from
 * `identity.ts`'s `api.query.identity.citizenNullifier(address)` — against the stored
 * nullifier byte-for-byte via `nullifiersEqual`, mirroring how `isRuledAgainstParty` already
 * identifies a caller by nullifier rather than address.
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
  callerCitizenNullifier: Uint8Array | null,
  oracleMembers: string[],
): boolean {
  const filer = caseDetail.filer;
  const isFiler =
    filer.kind === 'account'
      ? callerAddress === filer.address
      : callerCitizenNullifier !== null && nullifiersEqual(filer.value, callerCitizenNullifier);
  return isFiler || oracleMembers.includes(callerAddress);
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
  return nullifiersEqual(caseDetail.subject.CitizenConduct.nullifier, callerCitizenNullifier);
}

/**
 * `pallet_courts::file_case(subject, zk_proof, public_inputs)` requires both `zk_proof` and
 * `public_inputs` for `LawChallenge`/`TreasuryDispute`/`TierConflict` (else
 * `Error::MissingZkProof`) and rejects both for `CitizenConduct`/`General` (else
 * `Error::UnexpectedZkProof`) — see `CaseFilingProof`'s doc comment. `proof` defaults to
 * `null`, which is correct for `CitizenConduct`/`General` filings and is the only case this
 * mobile app can produce today — no Noir prover native module is registered in this project
 * yet (see `zkProving.ts`'s documented blocker), so nothing here can actually build a
 * `CaseFilingProof` for the anonymized case types.
 */
export async function fileCase(
  pair: KeyringPair,
  subject: CaseSubject,
  proof: CaseFilingProof | null = null,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(
    api.tx.courts.fileCase(subject, proof ? proof.zkProof : null, proof ? proof.publicInputs : null),
    pair,
  );
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
