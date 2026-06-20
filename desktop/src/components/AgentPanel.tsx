import { FormEvent, useRef, useState, useEffect } from "react";
import { useAgent } from "../context/AgentContext";
import "./AgentPanel.css";

interface Props {
  itemTitle: string;
}

export default function AgentPanel({ itemTitle }: Props) {
  const { messages, isThinking, isAvailable, ask, clear } = useAgent();
  const [input, setInput] = useState("");
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, isThinking]);

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const q = input.trim();
    if (!q || isThinking) return;
    setInput("");
    ask(q);
  }

  return (
    <div className="agent-panel">
      <div className="agent-header">
        <span className="agent-title">Ask AI about this {itemTitle}</span>
        {messages.length > 0 && (
          <button className="agent-clear" onClick={clear}>
            Clear
          </button>
        )}
        {!isAvailable && <span className="agent-offline-badge">Offline</span>}
      </div>

      <div className="agent-messages">
        {messages.length === 0 && (
          <p className="agent-empty">
            Ask anything — "What does this change?", "Is this constitutional?", "Who proposed this?"
          </p>
        )}
        {messages.map((msg, i) => (
          <div key={i} className={`agent-message agent-message-${msg.role}`}>
            <span className="message-role">{msg.role === "user" ? "You" : "AI"}</span>
            <p className="message-content">{msg.content}</p>
          </div>
        ))}
        {isThinking && (
          <div className="agent-message agent-message-assistant agent-thinking">
            <span className="message-role">AI</span>
            <span className="thinking-dots">···</span>
          </div>
        )}
        <div ref={bottomRef} />
      </div>

      <form className="agent-input-row" onSubmit={handleSubmit}>
        <input
          className="agent-input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder={isAvailable ? "Ask a question..." : "AI unavailable offline"}
          disabled={!isAvailable || isThinking}
        />
        <button className="agent-send" type="submit" disabled={!isAvailable || isThinking || !input.trim()}>
          Ask
        </button>
      </form>
    </div>
  );
}
