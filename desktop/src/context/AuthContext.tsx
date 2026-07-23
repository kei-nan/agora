import { createContext, useCallback, useContext, useRef, useState, ReactNode } from "react";
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
  requestQr: () => Promise<void>;
  logout: () => void;
}

const AuthContext = createContext<AuthState>({
  session: null,
  qrChallenge: null,
  isGeneratingQr: false,
  qrError: null,
  requestQr: async () => {},
  logout: () => {},
});

export function AuthProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [qrChallenge, setQrChallenge] = useState<string | null>(null);
  const [isGeneratingQr, setIsGeneratingQr] = useState(false);
  const [qrError, setQrError] = useState<string | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancelPoll = useCallback(() => {
    if (pollRef.current !== null) { clearInterval(pollRef.current); pollRef.current = null; }
    if (timeoutRef.current !== null) { clearTimeout(timeoutRef.current); timeoutRef.current = null; }
  }, []);

  const requestQr = useCallback(async () => {
    cancelPoll(); // clear any existing poll before starting a new one
    setIsGeneratingQr(true);
    setQrError(null);
    try {
      const challenge = await invoke<string>("auth_generate_challenge");
      setQrChallenge(challenge);
      setIsGeneratingQr(false);

      // Poll for session completion — mobile app deep-links back with signed token
      pollRef.current = setInterval(async () => {
        try {
          const sess = await invoke<Session>("auth_poll_session", { challenge });
          if (sess) {
            setSession(sess);
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
  }, [cancelPoll]);

  const logout = useCallback(() => {
    setSession(null);
    setQrChallenge(null);
    setQrError(null);
  }, []);

  return (
    <AuthContext.Provider value={{ session, qrChallenge, isGeneratingQr, qrError, requestQr, logout }}>
      {children}
    </AuthContext.Provider>
  );
}

export const useAuth = () => useContext(AuthContext);
