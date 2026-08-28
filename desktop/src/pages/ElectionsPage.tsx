import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "../lib/invoke";
import { useAgent } from "../context/AgentContext";
import AgentPanel from "../components/AgentPanel";
import "./Page.css";

interface Delegate {
  account: string;
  displayName: string;
  backingCount: number;
  status: "pending" | "active" | "on_break";
  consecutiveTerms: number;
  profileIpfsHash: string;
}

interface ElectionsData {
  delegates: Delegate[];
}

function shortAddr(hex: string): string {
  if (hex.length <= 14) return hex;
  return `${hex.slice(0, 8)}…${hex.slice(-6)}`;
}

export default function ElectionsPage() {
  const { t } = useTranslation("elections");
  const STATUS_LABEL: Record<string, string> = {
    pending: t("statusLabel.pending"),
    active: t("statusLabel.active"),
    on_break: t("statusLabel.on_break"),
  };
  const [data, setData] = useState<ElectionsData>({ delegates: [] });
  const [selectedDelegate, setSelectedDelegate] = useState<Delegate | null>(null);
  const [loading, setLoading] = useState(true);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<ElectionsData>("fetch_elections_data")
      .then(setData)
      .catch(() => setData({ delegates: [] }))
      .finally(() => setLoading(false));
  }, []);

  function selectDelegate(d: Delegate) {
    setSelectedDelegate(d);
    setActiveItem(
      `delegate-${d.account}`,
      `Delegate: ${d.displayName || shortAddr(d.account)}\nAccount: ${d.account}\nStatus: ${d.status}\nBacking count: ${d.backingCount}\nConsecutive terms: ${d.consecutiveTerms}`
    );
  }

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">{t("title")}</h1>
        <p className="page-subtitle">
          {t("subtitle", { count: data.delegates.length })}
        </p>
        {loading && <p className="loading">{t("loading")}</p>}

        {!loading && data.delegates.length === 0 && (
          <p className="empty">{t("empty")}</p>
        )}

        {data.delegates.length > 0 && (
          <>
            <h2 className="section-heading">{t("delegatesHeading")}</h2>
            <ul className="item-list">
              {data.delegates.map((d) => (
                <li
                  key={d.account}
                  className={`item-row ${selectedDelegate?.account === d.account ? "selected" : ""} ${d.status === "on_break" ? "muted" : ""}`}
                  onClick={() => selectDelegate(d)}
                >
                  <span className={`status-dot status-${d.status}`} />
                  <span className="item-title">
                    {d.displayName || shortAddr(d.account)}
                  </span>
                  <span className="backing-badge">{d.backingCount}</span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>

      {selectedDelegate ? (
        <div className="detail-panel">
          <h2 className="detail-title">
            {selectedDelegate.displayName || shortAddr(selectedDelegate.account)}
          </h2>
          <p className="detail-meta">
            <span className={`tier-chip tier-${selectedDelegate.status === "active" ? "constitutional" : "ordinary"}`}>
              {STATUS_LABEL[selectedDelegate.status] ?? selectedDelegate.status}
            </span>
          </p>
          <dl className="detail-fields">
            <dt>{t("accountLabel")}</dt>
            <dd className="mono">{shortAddr(selectedDelegate.account)}</dd>
            <dt>{t("backingLabel")}</dt>
            <dd>{t("citizenCount", { count: selectedDelegate.backingCount })}</dd>
            <dt>{t("consecutiveTermsLabel")}</dt>
            <dd>{selectedDelegate.consecutiveTerms}</dd>
            {selectedDelegate.profileIpfsHash && selectedDelegate.profileIpfsHash !== "0x" + "0".repeat(64) && (
              <>
                <dt>{t("profileLabel")}</dt>
                <dd>
                  <a
                    className="ipfs-link"
                    href={`https://ipfs.io/ipfs/${selectedDelegate.profileIpfsHash}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {t("viewOnIpfs")}
                  </a>
                </dd>
              </>
            )}
          </dl>
          <AgentPanel itemTitle="delegate" />
        </div>
      ) : (
        <div className="detail-panel detail-empty">
          <p>{t("selectPrompt")}</p>
        </div>
      )}
    </div>
  );
}
