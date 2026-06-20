import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAgent } from "../context/AgentContext";
import AgentPanel from "../components/AgentPanel";
import "./Page.css";

interface Ruling {
  id: string;
  caseTitle: string;
  level: 0 | 1 | 2;
  outcome: "upheld" | "overturned" | "pending";
  summary: string;
  ipfsHash: string;
  timestamp: number;
}

const LEVEL_LABELS = ["AI Judge", "Jury (7)", "Constitutional Jury (21)"];

export default function CourtsPage() {
  const [rulings, setRulings] = useState<Ruling[]>([]);
  const [selected, setSelected] = useState<Ruling | null>(null);
  const [loading, setLoading] = useState(true);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<Ruling[]>("fetch_rulings")
      .then(setRulings)
      .catch(() => setRulings([]))
      .finally(() => setLoading(false));
  }, []);

  function selectRuling(r: Ruling) {
    setSelected(r);
    setActiveItem(
      r.id,
      `Court ruling: ${r.caseTitle}\nLevel: ${LEVEL_LABELS[r.level]}\nOutcome: ${r.outcome}\nSummary: ${r.summary}\nIPFS: ${r.ipfsHash}`
    );
  }

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">Court Rulings</h1>
        {loading && <p className="loading">Loading...</p>}
        {!loading && rulings.length === 0 && <p className="empty">No rulings yet.</p>}
        <ul className="item-list">
          {rulings.map((r) => (
            <li
              key={r.id}
              className={`item-row ${selected?.id === r.id ? "selected" : ""}`}
              onClick={() => selectRuling(r)}
            >
              <span className={`status-chip status-${r.outcome}`}>{r.outcome}</span>
              <span className="item-title">{r.caseTitle}</span>
              <span className="item-meta">{LEVEL_LABELS[r.level]}</span>
            </li>
          ))}
        </ul>
      </div>

      {selected && (
        <div className="detail-panel">
          <h2 className="detail-title">{selected.caseTitle}</h2>
          <p className="detail-meta">
            {LEVEL_LABELS[selected.level]} · {selected.outcome}
          </p>
          <p className="detail-summary">{selected.summary}</p>
          <a className="ipfs-link" href={`https://ipfs.io/ipfs/${selected.ipfsHash}`} target="_blank" rel="noreferrer">
            Full ruling on IPFS
          </a>
          <AgentPanel itemTitle="ruling" />
        </div>
      )}
    </div>
  );
}
