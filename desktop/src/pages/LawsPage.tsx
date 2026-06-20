import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAgent } from "../context/AgentContext";
import AgentPanel from "../components/AgentPanel";
import "./Page.css";

interface Law {
  id: string;
  title: string;
  tier: "constitutional" | "ordinary";
  version: number;
  enactedAt: number;
  ipfsHash: string;
  summary: string;
}

export default function LawsPage() {
  const [laws, setLaws] = useState<Law[]>([]);
  const [selected, setSelected] = useState<Law | null>(null);
  const [loading, setLoading] = useState(true);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<Law[]>("fetch_laws")
      .then(setLaws)
      .catch(() => setLaws([]))
      .finally(() => setLoading(false));
  }, []);

  function selectLaw(law: Law) {
    setSelected(law);
    setActiveItem(
      law.id,
      `Law: ${law.title}\nTier: ${law.tier}\nVersion: ${law.version}\nSummary: ${law.summary}\nIPFS: ${law.ipfsHash}`
    );
  }

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">Laws</h1>
        {loading && <p className="loading">Loading...</p>}
        {!loading && laws.length === 0 && <p className="empty">No laws enacted yet.</p>}
        <ul className="item-list">
          {laws.map((law) => (
            <li
              key={law.id}
              className={`item-row ${selected?.id === law.id ? "selected" : ""}`}
              onClick={() => selectLaw(law)}
            >
              <span className={`tier-chip tier-${law.tier}`}>{law.tier}</span>
              <span className="item-title">{law.title}</span>
              <span className="item-meta">v{law.version}</span>
            </li>
          ))}
        </ul>
      </div>

      {selected && (
        <div className="detail-panel">
          <h2 className="detail-title">{selected.title}</h2>
          <p className="detail-meta">
            {selected.tier} law · version {selected.version}
          </p>
          <p className="detail-summary">{selected.summary}</p>
          <a className="ipfs-link" href={`https://ipfs.io/ipfs/${selected.ipfsHash}`} target="_blank" rel="noreferrer">
            Full text on IPFS
          </a>
          <AgentPanel itemTitle="law" />
        </div>
      )}
    </div>
  );
}
