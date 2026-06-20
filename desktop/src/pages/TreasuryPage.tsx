import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAgent } from "../context/AgentContext";
import AgentPanel from "../components/AgentPanel";
import "./Page.css";

interface TreasuryEntry {
  id: string;
  department: string;
  amount: string;
  currency: string;
  description: string;
  timestamp: number;
  ipfsHash: string;
}

export default function TreasuryPage() {
  const [entries, setEntries] = useState<TreasuryEntry[]>([]);
  const [selected, setSelected] = useState<TreasuryEntry | null>(null);
  const [loading, setLoading] = useState(true);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<TreasuryEntry[]>("fetch_treasury")
      .then(setEntries)
      .catch(() => setEntries([]))
      .finally(() => setLoading(false));
  }, []);

  function selectEntry(e: TreasuryEntry) {
    setSelected(e);
    setActiveItem(
      e.id,
      `Treasury transaction: ${e.description}\nDepartment: ${e.department}\nAmount: ${e.amount} ${e.currency}\nIPFS: ${e.ipfsHash}`
    );
  }

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">Treasury Ledger</h1>
        {loading && <p className="loading">Loading...</p>}
        {!loading && entries.length === 0 && <p className="empty">No transactions yet.</p>}
        <ul className="item-list">
          {entries.map((e) => (
            <li
              key={e.id}
              className={`item-row ${selected?.id === e.id ? "selected" : ""}`}
              onClick={() => selectEntry(e)}
            >
              <span className="item-title">{e.description}</span>
              <span className="item-meta">{e.department}</span>
              <span className="amount-chip">
                {e.amount} {e.currency}
              </span>
            </li>
          ))}
        </ul>
      </div>

      {selected && (
        <div className="detail-panel">
          <h2 className="detail-title">{selected.description}</h2>
          <p className="detail-meta">
            {selected.department} · {selected.amount} {selected.currency}
          </p>
          <a className="ipfs-link" href={`https://ipfs.io/ipfs/${selected.ipfsHash}`} target="_blank" rel="noreferrer">
            Audit record on IPFS
          </a>
          <AgentPanel itemTitle="transaction" />
        </div>
      )}
    </div>
  );
}
