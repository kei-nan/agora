// Real chain-state reads over the embedded smoldot light client, replacing the Rust
// `reqwest`-backed implementations of the same commands in `desktop/src-tauri/src/commands/chain.rs`
// for the desktop UI's actual call path (see `desktop/src/lib/invoke.ts`). Byte layouts and
// output shapes are a deliberate mirror of that file's doc comments — kept in lockstep so the
// two are easy to diff against each other if a pallet's storage layout changes.
import type { ApiPromise } from "@polkadot/api";
import { xxhashAsHex } from "@polkadot/util-crypto";
import { getApi } from "./client";
import { decodeCompact, extractU32KeySuffix, formatAgr, hexToBytes, u128LE, u32LE } from "./scale";

function storagePrefix(pallet: string, item: string): string {
  return "0x" + xxhashAsHex(pallet, 128).slice(2) + xxhashAsHex(item, 128).slice(2);
}

async function getKeysPaged(api: ApiPromise, prefixHex: string): Promise<string[]> {
  const all: string[] = [];
  let startKey = "";
  for (;;) {
    const page = await api.rpc.state.getKeysPaged(prefixHex, 1000, startKey);
    const keys = page.map((k) => k.toHex());
    all.push(...keys);
    if (keys.length < 1000) break;
    startKey = keys[keys.length - 1];
  }
  return all;
}

type OptionLike = { isSome: boolean; unwrap(): { toHex(): string } };

/** Returns values aligned 1:1 with `keys`; `null` where the key has no value. */
async function queryStorageAt(api: ApiPromise, keys: string[]): Promise<(string | null)[]> {
  if (keys.length === 0) return [];
  const values = (await api.rpc.state.queryStorageAt(keys)) as unknown as OptionLike[];
  return values.map((v) => (v && v.isSome ? v.unwrap().toHex() : null));
}

async function getStorage(api: ApiPromise, keyHex: string): Promise<string | null> {
  const v = (await api.rpc.state.getStorage(keyHex)) as unknown as OptionLike;
  return v.isSome ? v.unwrap().toHex() : null;
}

// ── chain_status ──────────────────────────────────────────────────────────────────────────

export async function chainStatus(): Promise<{ best: number; finalized: number }> {
  const api = await getApi();
  const header = await api.rpc.chain.getHeader();
  const finalizedHash = await api.rpc.chain.getFinalizedHead();
  const finalizedHeader = await api.rpc.chain.getHeader(finalizedHash);
  return { best: header.number.toNumber(), finalized: finalizedHeader.number.toNumber() };
}

// ── fetch_proposals ───────────────────────────────────────────────────────────────────────

export interface Proposal {
  id: string;
  title: string;
  status: string;
  proposer: string;
  votesFor: number;
  votesAgainst: number;
  endsAt: number;
  ipfsHash: string;
  summary: string;
  tier: string;
}

export async function fetchProposals(): Promise<Proposal[]> {
  const api = await getApi();

  const refPrefix = storagePrefix("Voting", "Referenda");
  const refKeys = await getKeysPaged(api, refPrefix);

  const tallyPrefix = storagePrefix("Voting", "ReferendumTally");
  const tallyKeys = await getKeysPaged(api, tallyPrefix);
  const tallyValues = await queryStorageAt(api, tallyKeys);

  const tallies = new Map<number, [number, number]>();
  tallyKeys.forEach((keyHex, i) => {
    const valHex = tallyValues[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 8) return;
    const yes = u32LE(bytes, 0);
    const no = u32LE(bytes, 4);
    const rid = extractU32KeySuffix(hexToBytes(keyHex));
    tallies.set(rid, [yes, no]);
  });

  if (refKeys.length === 0) return [];
  const refValues = await queryStorageAt(api, refKeys);

  const proposals: Proposal[] = [];
  refKeys.forEach((keyHex, i) => {
    const valHex = refValues[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 42) return;
    const topicHash = bytes.slice(4, 36);
    const endBlock = u32LE(bytes, 36);
    const state = bytes[40] === 0 ? "active" : bytes[40] === 1 ? "passed" : "rejected";
    const tier = bytes[41] === 1 ? "constitutional" : "ordinary";

    const referendumId = extractU32KeySuffix(hexToBytes(keyHex));
    const [yes, no] = tallies.get(referendumId) ?? [0, 0];

    proposals.push({
      id: `ref-${referendumId}`,
      title: `Referendum #${referendumId}`,
      status: state,
      proposer: "",
      votesFor: yes,
      votesAgainst: no,
      endsAt: endBlock,
      ipfsHash: "0x" + Array.from(topicHash).map((b) => b.toString(16).padStart(2, "0")).join(""),
      summary: `${tier} · closes block ${endBlock} · ${yes} for / ${no} against`,
      tier,
    });
  });
  return proposals;
}

// ── fetch_laws ────────────────────────────────────────────────────────────────────────────

export interface Law {
  id: string;
  title: string;
  tier: string;
  version: number;
  enactedAt: number;
  ipfsHash: string;
  summary: string;
}

export async function fetchLaws(): Promise<Law[]> {
  const api = await getApi();
  const prefix = storagePrefix("Constitution", "Laws");
  const keys = await getKeysPaged(api, prefix);
  if (keys.length === 0) return [];
  const values = await queryStorageAt(api, keys);

  const laws: Law[] = [];
  keys.forEach((keyHex, i) => {
    const valHex = values[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 38) return;
    const tier = bytes[0] === 0 ? "ordinary" : "constitutional";
    const status = bytes[1] === 0 ? "active" : bytes[1] === 1 ? "paused" : "repealed";
    const version = u32LE(bytes, 2);
    const ipfsHash = "0x" + Array.from(bytes.slice(6, 38)).map((b) => b.toString(16).padStart(2, "0")).join("");
    const keyBytes = hexToBytes(keyHex);
    const lawId = keyBytes.length >= 36 ? u32LE(keyBytes, keyBytes.length - 4) : i;
    laws.push({
      id: `law-${lawId}`,
      title: `Law #${lawId}`,
      tier,
      version,
      enactedAt: 0,
      ipfsHash,
      summary: `Status: ${status} · v${version}. Fetch full text from IPFS.`,
    });
  });
  return laws;
}

// ── fetch_treasury / fetch_department_budgets ────────────────────────────────────────────

export interface TreasuryEntry {
  id: string;
  department: string;
  amount: string;
  currency: string;
  description: string;
  timestamp: number;
  ipfsHash: string;
}

export async function fetchTreasury(): Promise<TreasuryEntry[]> {
  const api = await getApi();
  const prefix = storagePrefix("TreasuryLedger", "ExpenditureLog");
  const keys = await getKeysPaged(api, prefix);
  if (keys.length === 0) return [];
  const values = await queryStorageAt(api, keys);

  const entries: TreasuryEntry[] = [];
  values.forEach((valHex, i) => {
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 52) return;
    const deptId = u32LE(bytes, 0);
    const amount = u128LE(bytes, 4);
    const ipfsHash = "0x" + Array.from(bytes.slice(20, 52)).map((b) => b.toString(16).padStart(2, "0")).join("");
    entries.push({
      id: `tx-${i}`,
      department: `Department ${deptId}`,
      amount: formatAgr(amount),
      currency: "AGR",
      description: `Dept ${deptId} expenditure #${i}`,
      timestamp: 0,
      ipfsHash,
    });
  });
  return entries;
}

export interface DepartmentBudget {
  departmentId: number;
  budget: string;
  spent: string;
  remaining: string;
}

export async function fetchDepartmentBudgets(): Promise<DepartmentBudget[]> {
  const api = await getApi();
  const budgetPrefix = storagePrefix("TreasuryLedger", "DepartmentBudgets");
  const spentPrefix = storagePrefix("TreasuryLedger", "DepartmentSpent");

  const budgetKeys = await getKeysPaged(api, budgetPrefix);
  const spentKeys = await getKeysPaged(api, spentPrefix);
  const budgetVals = await queryStorageAt(api, budgetKeys);
  const spentVals = await queryStorageAt(api, spentKeys);

  const spentMap = new Map<number, bigint>();
  spentKeys.forEach((keyHex, i) => {
    const valHex = spentVals[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 16) return;
    const amt = u128LE(bytes, 0);
    const deptId = extractU32KeySuffix(hexToBytes(keyHex));
    spentMap.set(deptId, amt);
  });

  const budgets: DepartmentBudget[] = [];
  budgetKeys.forEach((keyHex, i) => {
    const valHex = budgetVals[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 16) return;
    const budgetAmt = u128LE(bytes, 0);
    const deptId = extractU32KeySuffix(hexToBytes(keyHex));
    const spentAmt = spentMap.get(deptId) ?? 0n;
    const remaining = budgetAmt > spentAmt ? budgetAmt - spentAmt : 0n;
    budgets.push({
      departmentId: deptId,
      budget: formatAgr(budgetAmt),
      spent: formatAgr(spentAmt),
      remaining: formatAgr(remaining),
    });
  });
  return budgets;
}

// ── fetch_rulings ─────────────────────────────────────────────────────────────────────────

export interface Ruling {
  id: string;
  caseTitle: string;
  level: number;
  outcome: string;
  summary: string;
  ipfsHash: string;
  timestamp: number;
}

export async function fetchRulings(): Promise<Ruling[]> {
  const api = await getApi();

  const casesPrefix = storagePrefix("Courts", "Cases");
  const caseKeys = await getKeysPaged(api, casesPrefix);
  const caseValues = await queryStorageAt(api, caseKeys);

  const caseIpfs = new Map<number, string>();
  caseKeys.forEach((keyHex, i) => {
    const valHex = caseValues[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 34) return;
    const ipfs = bytes[33] === 1 && bytes.length >= 66
      ? "0x" + Array.from(bytes.slice(34, 66)).map((b) => b.toString(16).padStart(2, "0")).join("")
      : "";
    const caseId = extractU32KeySuffix(hexToBytes(keyHex));
    caseIpfs.set(caseId, ipfs);
  });

  const rulingPrefix = storagePrefix("Courts", "Rulings");
  const rulingKeys = await getKeysPaged(api, rulingPrefix);
  if (rulingKeys.length === 0) return [];
  const rulingValues = await queryStorageAt(api, rulingKeys);

  const rulings: Ruling[] = [];
  rulingKeys.forEach((keyHex, i) => {
    const valHex = rulingValues[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    const outcome = bytes[0] === 0 ? "upheld" : bytes[0] === 1 ? "overturned" : "unknown";
    const caseId = extractU32KeySuffix(hexToBytes(keyHex));
    const ipfsHash = caseIpfs.get(caseId) ?? "";
    rulings.push({
      id: `ruling-${caseId}`,
      caseTitle: `Case #${caseId}`,
      level: 0,
      outcome,
      summary: `Verdict: ${outcome}. Fetch full ruling text from IPFS.`,
      ipfsHash,
      timestamp: 0,
    });
  });
  return rulings;
}

// ── fetch_legislature_data ────────────────────────────────────────────────────────────────

export interface LegislatureMotion {
  id: number;
  callHash: string;
  proposer: string;
  ayes: number;
  nays: number;
  endBlock: number;
  executed: boolean;
}

export interface LegislatureData {
  members: string[];
  motions: LegislatureMotion[];
}

export async function fetchLegislatureData(): Promise<LegislatureData> {
  const api = await getApi();

  const membersKey = storagePrefix("Legislature", "Members");
  const membersHex = await getStorage(api, membersKey);
  const members: string[] = [];
  if (membersHex) {
    const bytes = hexToBytes(membersHex);
    const [count, offset] = decodeCompact(bytes);
    let pos = offset;
    for (let i = 0; i < count; i++) {
      if (pos + 32 <= bytes.length) {
        members.push("0x" + Array.from(bytes.slice(pos, pos + 32)).map((b) => b.toString(16).padStart(2, "0")).join(""));
        pos += 32;
      }
    }
  }

  const motionsPrefix = storagePrefix("Legislature", "Motions");
  const motionKeys = await getKeysPaged(api, motionsPrefix);
  const motions: LegislatureMotion[] = [];
  if (motionKeys.length > 0) {
    const values = await queryStorageAt(api, motionKeys);
    motionKeys.forEach((keyHex, i) => {
      const valHex = values[i];
      if (!valHex) return;
      const bytes = hexToBytes(valHex);
      if (bytes.length < 77) return;
      const callHash = "0x" + Array.from(bytes.slice(0, 32)).map((b) => b.toString(16).padStart(2, "0")).join("");
      const proposer = "0x" + Array.from(bytes.slice(32, 64)).map((b) => b.toString(16).padStart(2, "0")).join("");
      const ayes = u32LE(bytes, 64);
      const nays = u32LE(bytes, 68);
      const endBlock = u32LE(bytes, 72);
      const executed = bytes[76] !== 0;
      const id = extractU32KeySuffix(hexToBytes(keyHex));
      motions.push({ id, callHash, proposer, ayes, nays, endBlock, executed });
    });
  }

  return { members, motions };
}

// ── fetch_elections_data ──────────────────────────────────────────────────────────────────

export interface Delegate {
  account: string;
  displayName: string;
  backingCount: number;
  status: string;
  consecutiveTerms: number;
  profileIpfsHash: string;
}

export interface ElectionsData {
  delegates: Delegate[];
}

export async function fetchElectionsData(): Promise<ElectionsData> {
  const api = await getApi();

  const backingPrefix = storagePrefix("PalletElections", "BackingCount");
  const backingKeys = await getKeysPaged(api, backingPrefix);
  const backingValues = await queryStorageAt(api, backingKeys);
  const backingMap = new Map<string, number>();
  backingKeys.forEach((keyHex, i) => {
    const valHex = backingValues[i];
    if (!valHex) return;
    const bytes = hexToBytes(valHex);
    if (bytes.length < 4) return;
    const count = u32LE(bytes, 0);
    const keyBytes = hexToBytes(keyHex);
    if (keyBytes.length >= 32) {
      const account = "0x" + Array.from(keyBytes.slice(keyBytes.length - 32)).map((b) => b.toString(16).padStart(2, "0")).join("");
      backingMap.set(account, count);
    }
  });

  const delegatesPrefix = storagePrefix("PalletElections", "Delegates");
  const delegateKeys = await getKeysPaged(api, delegatesPrefix);
  const delegates: Delegate[] = [];
  if (delegateKeys.length > 0) {
    const values = await queryStorageAt(api, delegateKeys);
    delegateKeys.forEach((keyHex, i) => {
      const valHex = values[i];
      if (!valHex) return;
      const bytes = hexToBytes(valHex);
      let pos = 0;
      const [nameLen, consumed] = decodeCompact(bytes.slice(pos));
      pos += consumed;
      let displayName = "";
      if (pos + nameLen <= bytes.length) {
        displayName = new TextDecoder().decode(bytes.slice(pos, pos + nameLen));
        pos += nameLen;
      }
      let profileIpfsHash = "";
      if (pos + 32 <= bytes.length) {
        profileIpfsHash = "0x" + Array.from(bytes.slice(pos, pos + 32)).map((b) => b.toString(16).padStart(2, "0")).join("");
        pos += 32;
      }
      let status = "pending";
      if (pos < bytes.length) {
        status = bytes[pos] === 1 ? "active" : bytes[pos] === 2 ? "on_break" : "pending";
        pos += 1;
      }
      let consecutiveTerms = 0;
      if (pos + 4 <= bytes.length) {
        consecutiveTerms = u32LE(bytes, pos);
      }
      const keyBytes = hexToBytes(keyHex);
      const account = keyBytes.length >= 32
        ? "0x" + Array.from(keyBytes.slice(keyBytes.length - 32)).map((b) => b.toString(16).padStart(2, "0")).join("")
        : "";
      const backingCount = backingMap.get(account) ?? 0;
      delegates.push({ account, displayName, backingCount, status, consecutiveTerms, profileIpfsHash });
    });
  }
  delegates.sort((a, b) => b.backingCount - a.backingCount);

  return { delegates };
}

// ── fetch_anticorruption_data ─────────────────────────────────────────────────────────────

export interface AssetDisclosure {
  account: string;
  ipfsHash: string;
  disclosedAt: number;
  updateDueAt: number;
}

export interface ConflictEntry {
  account: string;
  entityId: number;
  conflictType: string;
  registeredAt: number;
}

export interface WhistleblowerReport {
  id: number;
  contentHash: string;
  submittedAt: number;
  status: string;
}

export interface AntiCorruptionData {
  assetDisclosures: AssetDisclosure[];
  conflicts: ConflictEntry[];
  reports: WhistleblowerReport[];
  investigatorCount: number;
}

export async function fetchAnticorruptionData(): Promise<AntiCorruptionData> {
  const api = await getApi();

  const disclosuresPrefix = storagePrefix("PalletAntiCorruption", "AssetDisclosures");
  const disclosureKeys = await getKeysPaged(api, disclosuresPrefix);
  const assetDisclosures: AssetDisclosure[] = [];
  if (disclosureKeys.length > 0) {
    const values = await queryStorageAt(api, disclosureKeys);
    disclosureKeys.forEach((keyHex, i) => {
      const valHex = values[i];
      if (!valHex) return;
      const bytes = hexToBytes(valHex);
      if (bytes.length < 40) return;
      const ipfsHash = "0x" + Array.from(bytes.slice(0, 32)).map((b) => b.toString(16).padStart(2, "0")).join("");
      const disclosedAt = u32LE(bytes, 32);
      const updateDueAt = u32LE(bytes, 36);
      const keyBytes = hexToBytes(keyHex);
      const account = keyBytes.length >= 32
        ? "0x" + Array.from(keyBytes.slice(keyBytes.length - 32)).map((b) => b.toString(16).padStart(2, "0")).join("")
        : "";
      assetDisclosures.push({ account, ipfsHash, disclosedAt, updateDueAt });
    });
  }

  const conflictsPrefix = storagePrefix("PalletAntiCorruption", "ConflictRegistry");
  const conflictKeys = await getKeysPaged(api, conflictsPrefix);
  const conflicts: ConflictEntry[] = [];
  if (conflictKeys.length > 0) {
    const values = await queryStorageAt(api, conflictKeys);
    conflictKeys.forEach((keyHex, i) => {
      const valHex = values[i];
      if (!valHex) return;
      const bytes = hexToBytes(valHex);
      if (bytes.length < 5) return;
      const conflictType = ["financial_interest", "family_relation", "former_employer", "business_partner"][bytes[0]] ?? "unknown";
      const registeredAt = u32LE(bytes, 1);
      const keyBytes = hexToBytes(keyHex);
      let account = "";
      let entityId = 0;
      if (keyBytes.length >= 36) {
        const s = keyBytes.length;
        account = "0x" + Array.from(keyBytes.slice(s - 36, s - 4)).map((b) => b.toString(16).padStart(2, "0")).join("");
        entityId = u32LE(keyBytes, s - 4);
      }
      conflicts.push({ account, entityId, conflictType, registeredAt });
    });
  }

  const reportsPrefix = storagePrefix("PalletAntiCorruption", "WhistleblowerReports");
  const reportKeys = await getKeysPaged(api, reportsPrefix);
  const reports: WhistleblowerReport[] = [];
  if (reportKeys.length > 0) {
    const values = await queryStorageAt(api, reportKeys);
    reportKeys.forEach((keyHex, i) => {
      const valHex = values[i];
      if (!valHex) return;
      const bytes = hexToBytes(valHex);
      if (bytes.length < 37) return;
      const contentHash = "0x" + Array.from(bytes.slice(0, 32)).map((b) => b.toString(16).padStart(2, "0")).join("");
      const submittedAt = u32LE(bytes, 32);
      const status = ["pending", "flagged", "under_investigation", "cleared", "referred_to_courts"][bytes[36]] ?? "unknown";
      const id = extractU32KeySuffix(hexToBytes(keyHex));
      reports.push({ id, contentHash, submittedAt, status });
    });
  }
  reports.sort((a, b) => b.id - a.id);

  const investigatorsKey = storagePrefix("PalletAntiCorruption", "Investigators");
  const investigatorsHex = await getStorage(api, investigatorsKey);
  const investigatorCount = investigatorsHex ? decodeCompact(hexToBytes(investigatorsHex))[0] : 0;

  return { assetDisclosures, conflicts, reports, investigatorCount };
}
