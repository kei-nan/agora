import { useEffect, useState } from "react";
import { invoke } from "../lib/invoke";
import { useAgent } from "../context/AgentContext";
import AgentPanel from "../components/AgentPanel";
import "./Page.css";

interface AssetDisclosure {
  account: string;
  ipfsHash: string;
  disclosedAt: number;
  updateDueAt: number;
}

interface ConflictEntry {
  account: string;
  entityId: number;
  conflictType: string;
  registeredAt: number;
}

interface WhistleblowerReport {
  id: number;
  contentHash: string;
  submittedAt: number;
  status: string;
}

interface AntiCorruptionData {
  assetDisclosures: AssetDisclosure[];
  conflicts: ConflictEntry[];
  reports: WhistleblowerReport[];
  investigatorCount: number;
}

type Selection =
  | { kind: "disclosure"; item: AssetDisclosure }
  | { kind: "conflict"; item: ConflictEntry }
  | { kind: "report"; item: WhistleblowerReport };

function shortAddr(hex: string): string {
  if (!hex || hex.length <= 14) return hex;
  return `${hex.slice(0, 8)}…${hex.slice(-6)}`;
}

const CONFLICT_LABEL: Record<string, string> = {
  financial_interest: "Financial interest",
  family_relation: "Family relation",
  former_employer: "Former employer",
  business_partner: "Business partner",
};

const REPORT_STATUS_LABEL: Record<string, string> = {
  pending: "Pending",
  flagged: "Flagged",
  under_investigation: "Under investigation",
  cleared: "Cleared",
  referred_to_courts: "Referred to courts",
};

const REPORT_STATUS_CLASS: Record<string, string> = {
  pending: "status-pending",
  flagged: "status-pending",
  under_investigation: "status-pending",
  cleared: "status-passed",
  referred_to_courts: "status-overturned",
};

export default function AntiCorruptionPage() {
  const [data, setData] = useState<AntiCorruptionData>({
    assetDisclosures: [],
    conflicts: [],
    reports: [],
    investigatorCount: 0,
  });
  const [selection, setSelection] = useState<Selection | null>(null);
  const [loading, setLoading] = useState(true);
  const [ipfsContent, setIpfsContent] = useState<string | null>(null);
  const [ipfsLoading, setIpfsLoading] = useState(false);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<AntiCorruptionData>("fetch_anticorruption_data")
      .then(setData)
      .catch(() =>
        setData({ assetDisclosures: [], conflicts: [], reports: [], investigatorCount: 0 })
      )
      .finally(() => setLoading(false));
  }, []);

  function selectDisclosure(d: AssetDisclosure) {
    setSelection({ kind: "disclosure", item: d });
    setIpfsContent(null);
    const itemId = `disclosure-${d.account}`;
    setActiveItem(
      itemId,
      `Asset disclosure\nOfficial: ${d.account}\nDisclosed at block ${d.disclosedAt}\nRenewal due at block ${d.updateDueAt}`
    );
    if (d.ipfsHash && d.ipfsHash !== "0x" + "0".repeat(64)) {
      setIpfsLoading(true);
      invoke<string>("fetch_ipfs_content", { hashHex: d.ipfsHash })
        .then((text) => {
          setIpfsContent(text);
          setActiveItem(
            itemId,
            `Asset disclosure\nOfficial: ${d.account}\nDisclosed at block ${d.disclosedAt}\n\n${text}`
          );
        })
        .catch(() => setIpfsContent(null))
        .finally(() => setIpfsLoading(false));
    }
  }

  function selectConflict(c: ConflictEntry) {
    setSelection({ kind: "conflict", item: c });
    setIpfsContent(null);
    setActiveItem(
      `conflict-${c.account}-${c.entityId}`,
      `Conflict of interest\nOfficial: ${c.account}\nEntity id: ${c.entityId}\nType: ${CONFLICT_LABEL[c.conflictType] ?? c.conflictType}\nRegistered at block ${c.registeredAt}`
    );
  }

  function selectReport(r: WhistleblowerReport) {
    setSelection({ kind: "report", item: r });
    setIpfsContent(null);
    setActiveItem(
      `report-${r.id}`,
      `Whistleblower report #${r.id}\nStatus: ${REPORT_STATUS_LABEL[r.status] ?? r.status}\nSubmitted at block ${r.submittedAt}\nContent is encrypted to the appointed investigator and is not readable from the public chain.`
    );
  }

  const totalItems = data.assetDisclosures.length + data.conflicts.length + data.reports.length;

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">Anti-Corruption</h1>
        <p className="page-subtitle">
          {data.assetDisclosures.length} disclosure{data.assetDisclosures.length !== 1 ? "s" : ""} ·{" "}
          {data.conflicts.length} conflict{data.conflicts.length !== 1 ? "s" : ""} ·{" "}
          {data.reports.length} report{data.reports.length !== 1 ? "s" : ""} ·{" "}
          {data.investigatorCount} investigator{data.investigatorCount !== 1 ? "s" : ""}
        </p>
        {loading && <p className="loading">Loading…</p>}
        {!loading && totalItems === 0 && (
          <p className="empty">No disclosures, conflicts, or reports on chain yet.</p>
        )}

        {data.reports.length > 0 && (
          <>
            <h2 className="section-heading">Whistleblower Reports</h2>
            <ul className="item-list">
              {data.reports.map((r) => (
                <li
                  key={r.id}
                  className={`item-row ${
                    selection?.kind === "report" && selection.item.id === r.id ? "selected" : ""
                  }`}
                  onClick={() => selectReport(r)}
                >
                  <span className={`status-chip ${REPORT_STATUS_CLASS[r.status] ?? "status-pending"}`}>
                    {REPORT_STATUS_LABEL[r.status] ?? r.status}
                  </span>
                  <span className="item-title">Report #{r.id}</span>
                  <span className="item-meta">block {r.submittedAt}</span>
                </li>
              ))}
            </ul>
          </>
        )}

        {data.assetDisclosures.length > 0 && (
          <>
            <h2 className="section-heading">Asset Disclosures</h2>
            <ul className="item-list">
              {data.assetDisclosures.map((d) => (
                <li
                  key={d.account}
                  className={`item-row ${
                    selection?.kind === "disclosure" && selection.item.account === d.account
                      ? "selected"
                      : ""
                  }`}
                  onClick={() => selectDisclosure(d)}
                >
                  <span className="item-title mono">{shortAddr(d.account)}</span>
                  <span className="item-meta">renews #{d.updateDueAt}</span>
                </li>
              ))}
            </ul>
          </>
        )}

        {data.conflicts.length > 0 && (
          <>
            <h2 className="section-heading">Conflict-of-Interest Registry</h2>
            <ul className="item-list">
              {data.conflicts.map((c) => (
                <li
                  key={`${c.account}-${c.entityId}`}
                  className={`item-row ${
                    selection?.kind === "conflict" &&
                    selection.item.account === c.account &&
                    selection.item.entityId === c.entityId
                      ? "selected"
                      : ""
                  }`}
                  onClick={() => selectConflict(c)}
                >
                  <span className="item-title mono">{shortAddr(c.account)}</span>
                  <span className="item-meta">entity {c.entityId}</span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>

      {selection?.kind === "disclosure" && (
        <div className="detail-panel">
          <h2 className="detail-title">Asset Disclosure</h2>
          <p className="detail-meta">{selection.item.account}</p>
          <dl className="detail-fields">
            <dt>Disclosed at</dt>
            <dd>block #{selection.item.disclosedAt}</dd>
            <dt>Renewal due</dt>
            <dd>block #{selection.item.updateDueAt}</dd>
          </dl>
          {ipfsLoading && <p className="loading">Fetching declaration from IPFS…</p>}
          {ipfsContent && <pre className="ipfs-content">{ipfsContent}</pre>}
          <a
            className="ipfs-link"
            href={`https://ipfs.io/ipfs/${selection.item.ipfsHash}`}
            target="_blank"
            rel="noreferrer"
          >
            View raw on IPFS
          </a>
          <AgentPanel itemTitle="asset disclosure" />
        </div>
      )}

      {selection?.kind === "conflict" && (
        <div className="detail-panel">
          <h2 className="detail-title">Conflict of Interest</h2>
          <p className="detail-meta">{selection.item.account}</p>
          <dl className="detail-fields">
            <dt>Entity id</dt>
            <dd>{selection.item.entityId}</dd>
            <dt>Type</dt>
            <dd>{CONFLICT_LABEL[selection.item.conflictType] ?? selection.item.conflictType}</dd>
            <dt>Registered at</dt>
            <dd>block #{selection.item.registeredAt}</dd>
          </dl>
          <AgentPanel itemTitle="conflict of interest entry" />
        </div>
      )}

      {selection?.kind === "report" && (
        <div className="detail-panel">
          <h2 className="detail-title">Whistleblower Report #{selection.item.id}</h2>
          <p className="detail-meta">
            <span
              className={`status-chip ${REPORT_STATUS_CLASS[selection.item.status] ?? "status-pending"}`}
            >
              {REPORT_STATUS_LABEL[selection.item.status] ?? selection.item.status}
            </span>
          </p>
          <dl className="detail-fields">
            <dt>Submitted at</dt>
            <dd>block #{selection.item.submittedAt}</dd>
            <dt>Content hash</dt>
            <dd className="mono">{shortAddr(selection.item.contentHash)}</dd>
          </dl>
          <p className="hint">
            Report content is encrypted to the appointed investigator and is not readable from
            the public chain. This is by design — the identity and content of the whistleblower
            report stay private until an investigator advances the case.
          </p>
          <AgentPanel itemTitle="whistleblower report status" />
        </div>
      )}

      {!selection && (
        <div className="detail-panel detail-empty">
          <p>Select a disclosure, conflict entry, or report to view details.</p>
        </div>
      )}
    </div>
  );
}
