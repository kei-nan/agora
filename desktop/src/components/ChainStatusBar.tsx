import { useChain } from "../context/ChainContext";
import "./ChainStatusBar.css";

const STATUS_LABELS: Record<string, string> = {
  connecting: "Connecting to chain...",
  syncing: "Syncing...",
  ready: "Live",
  error: "Disconnected",
};

export default function ChainStatusBar() {
  const { status, bestBlock, finalizedBlock, error } = useChain();

  return (
    <div className={`chain-status-bar status-${status}`} title={status === "error" ? error ?? undefined : undefined}>
      <span className="status-dot" />
      <span className="status-label">{STATUS_LABELS[status]}</span>
      {status === "ready" && bestBlock != null && (
        <>
          <span className="status-sep">·</span>
          <span className="status-blocks">
            Block #{bestBlock.toLocaleString()}
            {finalizedBlock != null && ` (finalized: #${finalizedBlock.toLocaleString()})`}
          </span>
        </>
      )}
    </div>
  );
}
