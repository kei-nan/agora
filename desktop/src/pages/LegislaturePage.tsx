import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "../lib/invoke";
import { useAgent } from "../context/AgentContext";
import AgentPanel from "../components/AgentPanel";
import "./Page.css";

interface Motion {
  id: number;
  callHash: string;
  proposer: string;
  ayes: number;
  nays: number;
  endBlock: number;
  executed: boolean;
}

interface LegislatureData {
  members: string[];
  motions: Motion[];
}

function shortAddr(hex: string): string {
  if (hex.length <= 14) return hex;
  return `${hex.slice(0, 8)}…${hex.slice(-6)}`;
}

export default function LegislaturePage() {
  const { t } = useTranslation("legislature");
  const [data, setData] = useState<LegislatureData>({ members: [], motions: [] });
  const [selected, setSelected] = useState<Motion | null>(null);
  const [loading, setLoading] = useState(true);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<LegislatureData>("fetch_legislature_data")
      .then(setData)
      .catch(() => setData({ members: [], motions: [] }))
      .finally(() => setLoading(false));
  }, []);

  function selectMotion(motion: Motion) {
    setSelected(motion);
    setActiveItem(
      `motion-${motion.id}`,
      `Legislature Motion #${motion.id}\nProposer: ${motion.proposer}\nAyes: ${motion.ayes} · Nays: ${motion.nays}\nEnds at block: ${motion.endBlock}\nExecuted: ${motion.executed}\nCall hash: ${motion.callHash}`
    );
  }

  const pendingMotions = data.motions.filter((m) => !m.executed);
  const executedMotions = data.motions.filter((m) => m.executed);

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">{t("title")}</h1>
        <p className="page-subtitle">
          {t("memberCount", { count: data.members.length })}
          {data.motions.length > 0 && t("pendingMotionCount", { count: pendingMotions.length })}
        </p>
        {loading && <p className="loading">{t("loading")}</p>}
        {!loading && data.motions.length === 0 && (
          <p className="empty">{t("empty")}</p>
        )}
        {pendingMotions.length > 0 && (
          <>
            <h2 className="section-heading">{t("pendingHeading")}</h2>
            <ul className="item-list">
              {pendingMotions.map((m) => (
                <li
                  key={m.id}
                  className={`item-row ${selected?.id === m.id ? "selected" : ""}`}
                  onClick={() => selectMotion(m)}
                >
                  <span className="item-title">{t("motionTitle", { id: m.id })}</span>
                  <span className="vote-chips">
                    <span className="vote-chip aye">{m.ayes} ✓</span>
                    <span className="vote-chip nay">{m.nays} ✗</span>
                  </span>
                </li>
              ))}
            </ul>
          </>
        )}
        {executedMotions.length > 0 && (
          <>
            <h2 className="section-heading">{t("executedHeading")}</h2>
            <ul className="item-list">
              {executedMotions.map((m) => (
                <li
                  key={m.id}
                  className={`item-row muted ${selected?.id === m.id ? "selected" : ""}`}
                  onClick={() => selectMotion(m)}
                >
                  <span className="item-title">{t("motionTitle", { id: m.id })}</span>
                  <span className="tier-chip tier-ordinary">{t("done")}</span>
                </li>
              ))}
            </ul>
          </>
        )}
        {data.members.length > 0 && (
          <>
            <h2 className="section-heading">{t("membersHeading", { count: data.members.length })}</h2>
            <ul className="item-list member-list">
              {data.members.map((addr) => (
                <li key={addr} className="item-row member-row">
                  <span className="member-addr">{shortAddr(addr)}</span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>

      {selected ? (
        <div className="detail-panel">
          <h2 className="detail-title">{t("motionTitle", { id: selected.id })}</h2>
          <p className="detail-meta">
            {selected.executed ? t("executedStatus") : t("closesAtBlock", { block: selected.endBlock })}
          </p>
          <div className="detail-vote-row">
            <span className="detail-vote aye">
              {t("ayeCount", { count: selected.ayes })}
            </span>
            <span className="detail-vote nay">
              {t("nayCount", { count: selected.nays })}
            </span>
          </div>
          <dl className="detail-fields">
            <dt>{t("proposerLabel")}</dt>
            <dd className="mono">{shortAddr(selected.proposer)}</dd>
            <dt>{t("callHashLabel")}</dt>
            <dd className="mono">{selected.callHash}</dd>
          </dl>
          <AgentPanel itemTitle="legislature motion" />
        </div>
      ) : (
        <div className="detail-panel detail-empty">
          <p>{t("selectPrompt")}</p>
          {data.members.length === 0 && !loading && (
            <p className="hint">{t("connectHint")}</p>
          )}
        </div>
      )}
    </div>
  );
}
