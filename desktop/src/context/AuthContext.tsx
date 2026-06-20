import { createContext, useCallback, useContext, useState, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

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

  const requestQr = useCallback(async () => {
    setIsGeneratingQr(true);
    setQrError(null);
    try {
      const challenge = await invoke<string>("auth_generate_challenge");
      setQrChallenge(challenge);
      setIsGeneratingQr(false);

      // Poll for session completion — mobile app deep-links back with signed token
      const poll = setInterval(async () => {
        try {
          const sess = await invoke<Session>("auth_poll_session", { challenge });
          if (sess) {
            setSession(sess);
            setQrChallenge(null);
            clearInterval(poll);
          }
        } catch {
          // "pending" error is expected until mobile completes auth
        }
      }, 2000);

      // Stop polling after 5 minutes
      setTimeout(() => {
        clearInterval(poll);
        setQrChallenge(null);
      }, 5 * 60 * 1000);
    } catch (err) {
      setQrError(String(err));
      setIsGeneratingQr(false);
    }
  }, []);

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
