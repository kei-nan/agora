import { createContext, useContext, useEffect, useRef, useState, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

type ChainStatus = "connecting" | "syncing" | "ready" | "error";

interface ChainState {
  status: ChainStatus;
  bestBlock: number | null;
  finalizedBlock: number | null;
  error: string | null;
}

const ChainContext = createContext<ChainState>({
  status: "connecting",
  bestBlock: null,
  finalizedBlock: null,
  error: null,
});

export function ChainProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<ChainState>({
    status: "connecting",
    bestBlock: null,
    finalizedBlock: null,
    error: null,
  });
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    invoke<{ best: number; finalized: number }>("chain_status")
      .then(({ best, finalized }) =>
        setState({ status: "ready", bestBlock: best, finalizedBlock: finalized, error: null })
      )
      .catch((err) => setState({ status: "error", bestBlock: null, finalizedBlock: null, error: String(err) }));

    pollRef.current = setInterval(() => {
      invoke<{ best: number; finalized: number }>("chain_status")
        .then(({ best, finalized }) =>
          setState({ status: "ready", bestBlock: best, finalizedBlock: finalized, error: null })
        )
        .catch(() => {});
    }, 6000);

    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  return <ChainContext.Provider value={state}>{children}</ChainContext.Provider>;
}

export const useChain = () => useContext(ChainContext);
