import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "../lib/invoke";
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
  const { t } = useTranslation("laws");
  const [laws, setLaws] = useState<Law[]>([]);
  const [selected, setSelected] = useState<Law | null>(null);
  const [loading, setLoading] = useState(true);
  const [ipfsContent, setIpfsContent] = useState<string | null>(null);
  const [ipfsLoading, setIpfsLoading] = useState(false);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    invoke<Law[]>("fetch_laws")
      .then(setLaws)
      .catch(() => setLaws([]))
      .finally(() => setLoading(false));
  }, []);

  function selectLaw(law: Law) {
    setSelected(law);
    setIpfsContent(null);
    setActiveItem(
      law.id,
      `Law: ${law.title}\nTier: ${law.tier}\nVersion: ${law.version}\nSummary: ${law.summary}\nIPFS: ${law.ipfsHash}`
    );
    if (law.ipfsHash && law.ipfsHash !== "0x" + "0".repeat(64)) {
      setIpfsLoading(true);
      invoke<string>("fetch_ipfs_content", { hashHex: law.ipfsHash })
        .then((text) => {
          setIpfsContent(text);
          setActiveItem(
            law.id,
            `Law: ${law.title}\nTier: ${law.tier}\nVersion: ${law.version}\n\n${text}`
          );
        })
        .catch(() => setIpfsContent(null))
        .finally(() => setIpfsLoading(false));
    }
  }

  return (
    <div className="page-layout">
      <div className="list-panel">
        <h1 className="page-title">{t("title")}</h1>
        {loading && <p className="loading">{t("loading")}</p>}
        {!loading && laws.length === 0 && <p className="empty">{t("empty")}</p>}
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
            {t("detailMeta", { tier: selected.tier, version: selected.version })}
          </p>
          {ipfsLoading && <p className="loading">{t("fetchingContent")}</p>}
          {ipfsContent ? (
            <pre className="ipfs-content">{ipfsContent}</pre>
          ) : (
            !ipfsLoading && <p className="detail-summary">{selected.summary}</p>
          )}
          <a
            className="ipfs-link"
            href={`https://ipfs.io/ipfs/${selected.ipfsHash}`}
            target="_blank"
            rel="noreferrer"
          >
            {t("viewRawOnIpfs")}
          </a>
          <AgentPanel itemTitle="law" />
        </div>
      )}
    </div>
  );
}
