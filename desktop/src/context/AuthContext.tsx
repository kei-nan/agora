import {
  createContext, useCallback, useContext, useEffect, useRef, useState, ReactNode,
} from "react";
import { invoke } from "../lib/invoke";

interface Session {
  nullifierHash: string;
  expiresAt: number;
}

interface AuthState {
  session: Session | null;
  qrChallenge: string | null;
  isGeneratingQr: boolean;
  qrError: string | null;
  callbackPort: number | null;
  requestQr: () => Promise<void>;
  logout: () => void;
}

const AuthContext = createContext<AuthState>({
  session: null,
  qrChallenge: null,
  isGeneratingQr: false,
  qrError: null,
  callbackPort: null,
  requestQr: async () => {},
  logout: () => {},
});

function randomPort(): number {
  return 12000 + Math.floor(Math.random() * 1000);
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [qrChallenge, setQrChallenge] = useState<string | null>(null);
  const [isGeneratingQr, setIsGeneratingQr] = useState(false);
  const [qrError, setQrError] = useState<string | null>(null);
  const [callbackPort, setCallbackPort] = useState<number | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Start the local HTTP callback server once on mount.
  // The mobile app will POST the signed auth token to http://127.0.0.1:<port>/auth
  useEffect(() => {
    const port = randomPort();
    setCallbackPort(port);
    invoke("auth_start_callback_server", { port }).catch((e) =>
      console.warn("[auth] callback server not started:", e)
    );
  }, []);

  const cancelPoll = useCallback(() => {
    if (pollRef.current !== null) { clearInterval(pollRef.current); pollRef.current = null; }
    if (timeoutRef.current !== null) { clearTimeout(timeoutRef.current); timeoutRef.current = null; }
  }, []);

  const requestQr = useCallback(async () => {
    if (callbackPort === null) return;
    cancelPoll();
    setIsGeneratingQr(true);
    setQrError(null);
    try {
      const challenge = await invoke<string>("auth_generate_challenge", { port: callbackPort });
      setQrChallenge(challenge);
      setIsGeneratingQr(false);

      // Poll for session completion — mobile app POSTs back to local callback server
      pollRef.current = setInterval(async () => {
        try {
          const sess = await invoke<Session>("auth_poll_session", { challenge });
          if (sess) {
            // Verify the nullifier is actually registered on-chain before accepting.
            const isRegistered = await invoke<boolean>("auth_verify_nullifier", {
              nullifierHex: sess.nullifierHash,
            }).catch(() => false);
            if (isRegistered) {
              setSession(sess);
            } else {
              // Nullifier not on chain — reject and show error
              setQrError("Identity not registered on-chain. Complete passport registration first.");
            }
            setQrChallenge(null);
            cancelPoll();
          }
        } catch {
          // "pending" error is expected until mobile completes auth
        }
      }, 2000);

      // Stop polling after 5 minutes
      timeoutRef.current = setTimeout(() => {
        cancelPoll();
        setQrChallenge(null);
      }, 5 * 60 * 1000);
    } catch (err) {
      setQrError(String(err));
      setIsGeneratingQr(false);
    }
  }, [cancelPoll, callbackPort]);

  const logout = useCallback(() => {
    setSession(null);
    setQrChallenge(null);
    setQrError(null);
  }, []);

  return (
    <AuthContext.Provider value={{ session, qrChallenge, isGeneratingQr, qrError, callbackPort, requestQr, logout }}>
      {children}
    </AuthContext.Provider>
  );
}

export const useAuth = () => useContext(AuthContext);
