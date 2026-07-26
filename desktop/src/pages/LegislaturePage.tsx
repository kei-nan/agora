import { useEffect, useState } from "react";
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
        <h1 className="page-title">Legislature</h1>
        <p className="page-subtitle">
          {data.members.length} member{data.members.length !== 1 ? "s" : ""}
          {data.motions.length > 0 && ` · ${pendingMotions.length} pending motion${pendingMotions.length !== 1 ? "s" : ""}`}
        </p>
        {loading && <p className="loading">Loading…</p>}
        {!loading && data.motions.length === 0 && (
          <p className="empty">No motions on record.</p>
        )}
        {pendingMotions.length > 0 && (
          <>
            <h2 className="section-heading">Pending</h2>
            <ul className="item-list">
              {pendingMotions.map((m) => (
                <li
                  key={m.id}
                  className={`item-row ${selected?.id === m.id ? "selected" : ""}`}
                  onClick={() => selectMotion(m)}
                >
                  <span className="item-title">Motion #{m.id}</span>
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
            <h2 className="section-heading">Executed</h2>
            <ul className="item-list">
              {executedMotions.map((m) => (
                <li
                  key={m.id}
                  className={`item-row muted ${selected?.id === m.id ? "selected" : ""}`}
                  onClick={() => selectMotion(m)}
                >
                  <span className="item-title">Motion #{m.id}</span>
                  <span className="tier-chip tier-ordinary">done</span>
                </li>
              ))}
            </ul>
          </>
        )}
        {data.members.length > 0 && (
          <>
            <h2 className="section-heading">Members ({data.members.length})</h2>
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
          <h2 className="detail-title">Motion #{selected.id}</h2>
          <p className="detail-meta">
            {selected.executed ? "Executed" : `Closes at block ${selected.endBlock}`}
          </p>
          <div className="detail-vote-row">
            <span className="detail-vote aye">
              {selected.ayes} Aye{selected.ayes !== 1 ? "s" : ""}
            </span>
            <span className="detail-vote nay">
              {selected.nays} Nay{selected.nays !== 1 ? "s" : ""}
            </span>
          </div>
          <dl className="detail-fields">
            <dt>Proposer</dt>
            <dd className="mono">{shortAddr(selected.proposer)}</dd>
            <dt>Call hash</dt>
            <dd className="mono">{selected.callHash}</dd>
          </dl>
          <AgentPanel itemTitle="legislature motion" />
        </div>
      ) : (
        <div className="detail-panel detail-empty">
          <p>Select a motion to view details.</p>
          {data.members.length === 0 && !loading && (
            <p className="hint">Connect the chain node to populate legislature data.</p>
          )}
        </div>
      )}
    </div>
  );
}
