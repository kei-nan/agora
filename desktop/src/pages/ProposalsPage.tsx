import { useEffect, useState } from "react";
import { invoke } from "../lib/invoke";
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
  tier: "ordinary" | "constitutional";
}

const ZERO_HASH = "0x" + "0".repeat(64);

export default function ProposalsPage() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [selected, setSelected] = useState<Proposal | null>(null);
  const [loading, setLoading] = useState(true);
  const [ipfsContent, setIpfsContent] = useState<string | null>(null);
  const [ipfsLoading, setIpfsLoading] = useState(false);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<Proposal[]>("fetch_proposals")
      .then(setProposals)
      .catch(() => setProposals([]))
      .finally(() => setLoading(false));
  }, []);

  function selectProposal(p: Proposal) {
    setSelected(p);
    setIpfsContent(null);
    setActiveItem(p.id, `Proposal: ${p.title}\n\nSummary: ${p.summary}\nStatus: ${p.status}\nIPFS: ${p.ipfsHash}`);
    if (p.ipfsHash && p.ipfsHash !== ZERO_HASH) {
      setIpfsLoading(true);
      invoke<string>("fetch_ipfs_content", { hashHex: p.ipfsHash })
        .then((text) => {
          setIpfsContent(text);
          setActiveItem(
            p.id,
            `Proposal: ${p.title}\nStatus: ${p.status}\nProposed by: ${p.proposer}\n\n${text}`
          );
        })
        .catch(() => setIpfsContent(null))
        .finally(() => setIpfsLoading(false));
    }
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
              {p.tier === "constitutional" && (
                <span className="tier-chip tier-constitutional">const.</span>
              )}
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
          <div className="detail-vote-row">
            <span className="detail-vote aye">{selected.votesFor} for</span>
            <span className="detail-vote nay">{selected.votesAgainst} against</span>
          </div>
          <p className="mobile-vote-note">
            🗳️ Voting happens in the Agora mobile app, using your phone's secure key. This desktop
            view is read-only.
          </p>
          {ipfsLoading && <p className="loading">Fetching proposal text from IPFS…</p>}
          {ipfsContent ? (
            <pre className="ipfs-content">{ipfsContent}</pre>
          ) : (
            !ipfsLoading && <p className="detail-summary">{selected.summary}</p>
          )}
          <a className="ipfs-link" href={`https://ipfs.io/ipfs/${selected.ipfsHash}`} target="_blank" rel="noreferrer">
            Full text on IPFS
          </a>
          <AgentPanel itemTitle="proposal" />
        </div>
      )}
    </div>
  );
}
