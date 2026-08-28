import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "../lib/invoke";
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

interface DepartmentBudget {
  departmentId: number;
  budget: string;
  spent: string;
  remaining: string;
}

const ZERO_HASH = "0x" + "0".repeat(64);

export default function TreasuryPage() {
  const { t } = useTranslation("treasury");
  const [entries, setEntries] = useState<TreasuryEntry[]>([]);
  const [budgets, setBudgets] = useState<DepartmentBudget[]>([]);
  const [selected, setSelected] = useState<TreasuryEntry | null>(null);
  const [loading, setLoading] = useState(true);
  const [ipfsContent, setIpfsContent] = useState<string | null>(null);
  const [ipfsLoading, setIpfsLoading] = useState(false);
  const { setActiveItem } = useAgent();

  useEffect(() => {
    Promise.all([
      invoke<TreasuryEntry[]>("fetch_treasury").catch(() => []),
      invoke<DepartmentBudget[]>("fetch_department_budgets").catch(() => []),
    ]).then(([tx, depts]) => {
      setEntries(tx);
      setBudgets(depts);
    }).finally(() => setLoading(false));
  }, []);

  function selectEntry(e: TreasuryEntry) {
    setSelected(e);
    setIpfsContent(null);
    setActiveItem(
      e.id,
      `Treasury expenditure: ${e.description}\nDepartment: ${e.department}\nAmount: ${e.amount} ${e.currency}\nIPFS: ${e.ipfsHash}`
    );
    if (e.ipfsHash && e.ipfsHash !== ZERO_HASH) {
      setIpfsLoading(true);
      invoke<string>("fetch_ipfs_content", { hashHex: e.ipfsHash })
        .then((text) => {
          setIpfsContent(text);
          setActiveItem(
            e.id,
            `Treasury: ${e.description}\nDept: ${e.department}\nAmount: ${e.amount} ${e.currency}\n\n${text}`
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

        {budgets.length > 0 && (
          <div className="budget-summary">
            <h3 className="budget-heading">{t("departmentBudgets")}</h3>
            <table className="budget-table">
              <thead>
                <tr>
                  <th>{t("tableDept")}</th>
                  <th>{t("tableBudget")}</th>
                  <th>{t("tableSpent")}</th>
                  <th>{t("tableRemaining")}</th>
                </tr>
              </thead>
              <tbody>
                {budgets.map((b) => (
                  <tr key={b.departmentId}>
                    <td>{b.departmentId}</td>
                    <td>{b.budget}</td>
                    <td>{b.spent}</td>
                    <td className="remaining">{b.remaining}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <h3 className="budget-heading">{t("expenditures")}</h3>
        {loading && <p className="loading">{t("loading")}</p>}
        {!loading && entries.length === 0 && <p className="empty">{t("empty")}</p>}
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
            {t("detailMeta", { department: selected.department, amount: selected.amount, currency: selected.currency })}
          </p>
          {ipfsLoading && <p className="loading">{t("fetchingAuditRecord")}</p>}
          {ipfsContent ? (
            <pre className="ipfs-content">{ipfsContent}</pre>
          ) : (
            !ipfsLoading && selected.ipfsHash && selected.ipfsHash !== ZERO_HASH && (
              <p className="detail-summary">{t("auditMetadataAvailable")}</p>
            )
          )}
          {selected.ipfsHash && selected.ipfsHash !== ZERO_HASH && (
            <a
              className="ipfs-link"
              href={`https://ipfs.io/ipfs/${selected.ipfsHash}`}
              target="_blank"
              rel="noreferrer"
            >
              {t("auditRecordLink")}
            </a>
          )}
          <AgentPanel itemTitle="transaction" />
        </div>
      )}
    </div>
  );
}
