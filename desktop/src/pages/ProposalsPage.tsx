import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAgent } from "../context/AgentContext";
import AgentPanel from "../components/AgentPanel";
import "./Page.css";

interface Proposal {
  id: string;
  title: string;
  status: "active" | "passed" | "rejected" | "pending";
  proposer: string;
  votesFor: number;
  votesAgainst: number;
  endsAt: number;
  ipfsHash: string;
  summary: string;
}

export default function ProposalsPage() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [selected, setSelected] = useState<Proposal | null>(null);
  const [loading, setLoading] = useState(true);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<Proposal[]>("fetch_proposals")
      .then(setProposals)
      .catch(() => setProposals([]))
      .finally(() => setLoading(false));
  }, []);

  function selectProposal(p: Proposal) {
    setSelected(p);
    setActiveItem(p.id, `Proposal: ${p.title}\n\nSummary: ${p.summary}\nStatus: ${p.status}\nIPFS: ${p.ipfsHash}`);
  }

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">Proposals</h1>
        {loading && <p className="loading">Loading...</p>}
        {!loading && proposals.length === 0 && <p className="empty">No proposals found.</p>}
        <ul className="item-list">
          {proposals.map((p) => (
            <li
              key={p.id}
              className={`item-row ${selected?.id === p.id ? "selected" : ""}`}
              onClick={() => selectProposal(p)}
            >
              <span className={`status-chip status-${p.status}`}>{p.status}</span>
              <span className="item-title">{p.title}</span>
              <span className="item-meta">
                {p.votesFor} for · {p.votesAgainst} against
              </span>
            </li>
          ))}
        </ul>
      </div>

      {selected && (
        <div className="detail-panel">
          <h2 className="detail-title">{selected.title}</h2>
          <p className="detail-meta">Proposed by {selected.proposer}</p>
          <p className="detail-summary">{selected.summary}</p>
          <a className="ipfs-link" href={`https://ipfs.io/ipfs/${selected.ipfsHash}`} target="_blank" rel="noreferrer">
            Full text on IPFS
          </a>
          <AgentPanel itemTitle="proposal" />
        </div>
      )}
    </div>
  );
}
