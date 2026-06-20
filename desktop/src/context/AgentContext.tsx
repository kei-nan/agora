import { createContext, useCallback, useContext, useState, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface Message {
  role: "user" | "assistant";
  content: string;
}

interface AgentState {
  messages: Message[];
  isThinking: boolean;
  isAvailable: boolean;
  activeItemId: string | null;
  setActiveItem: (id: string | null, context: string | null) => void;
  ask: (question: string) => Promise<void>;
  clear: () => void;
}

const AgentContext = createContext<AgentState>({
  messages: [],
  isThinking: false,
  isAvailable: false,
  activeItemId: null,
  setActiveItem: () => {},
  ask: async () => {},
  clear: () => {},
});

export function AgentProvider({ children }: { children: ReactNode }) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isThinking, setIsThinking] = useState(false);
  const [isAvailable, setIsAvailable] = useState(true);
  const [activeItemId, setActiveItemId] = useState<string | null>(null);
  const [itemContext, setItemContext] = useState<string | null>(null);

  const setActiveItem = useCallback((id: string | null, context: string | null) => {
    setActiveItemId(id);
    setItemContext(context);
    setMessages([]);
  }, []);

  const ask = useCallback(
    async (question: string) => {
      const userMsg: Message = { role: "user", content: question };
      setMessages((prev) => [...prev, userMsg]);
      setIsThinking(true);
      try {
        const reply = await invoke<string>("agent_ask", {
          question,
          itemContext: itemContext ?? "",
          history: messages,
        });
        setMessages((prev) => [...prev, { role: "assistant", content: reply }]);
        setIsAvailable(true);
      } catch (err) {
        const errMsg = String(err);
        const isOffline = errMsg.includes("network") || errMsg.includes("connect") || errMsg.includes("timeout");
        setIsAvailable(!isOffline);
        setMessages((prev) => [
          ...prev,
          {
            role: "assistant",
            content: isOffline
              ? "AI assistant is unavailable offline. Browse the full text of this item above."
              : `Error: ${errMsg}`,
          },
        ]);
      } finally {
        setIsThinking(false);
      }
    },
    [itemContext, messages]
  );

  const clear = useCallback(() => {
    setMessages([]);
  }, []);

  return (
    <AgentContext.Provider value={{ messages, isThinking, isAvailable, activeItemId, setActiveItem, ask, clear }}>
      {children}
    </AgentContext.Provider>
  );
}

export const useAgent = () => useContext(AgentContext);
