import {
  createContext, useCallback, useContext, useEffect, useRef, useState, ReactNode,
} from "react";
import { invoke } from "../lib/invoke";

interface Session {
  nullifierHash: string;
  expiresAt: number;
  /** Bearer token for privileged (read+submit) commands. The backend (`SessionStore` in
   * src-tauri/src/commands/auth.rs) is the actual authority on whether this is still valid —
   * every privileged command re-checks it server-side, so this being present in frontend state
   * is not itself proof of anything. */
  token: string;
}

interface AuthState {
  session: Session | null;
  qrChallenge: string | null;
  qrExpiresAt: number | null;
  isGeneratingQr: boolean;
  qrError: string | null;
  callbackPort: number | null;
  requestQr: () => Promise<void>;
  logout: () => void;
}

const AuthContext = createContext<AuthState>({
  session: null,
  qrChallenge: null,
  qrExpiresAt: null,
  isGeneratingQr: false,
  qrError: null,
  callbackPort: null,
  requestQr: async () => {},
  logout: () => {},
});

const QR_TIMEOUT_MS = 5 * 60 * 1000;

function randomPort(): number {
  return 12000 + Math.floor(Math.random() * 1000);
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [qrChallenge, setQrChallenge] = useState<string | null>(null);
  const [qrExpiresAt, setQrExpiresAt] = useState<number | null>(null);
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
      setQrExpiresAt(Date.now() + QR_TIMEOUT_MS);
      setIsGeneratingQr(false);

      // Poll for session completion — mobile app POSTs back to local callback server.
      // The backend only ever marks a challenge complete after it has independently looked up
      // the nullifier's registered on-chain pubkey AND verified the phone's signature against
      // it (commands/auth.rs's `handle_auth_callback`) — an unregistered or unauthenticated
      // callback is rejected there and the challenge is simply left pending. So by the time
      // `auth_poll_session` returns a session here, it's already backed by a real, verified
      // bearer token. The `auth_verify_nullifier` check below is a second, independent
      // registration check (defense-in-depth, not a substitute for the backend's signature
      // verification).
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
            setQrExpiresAt(null);
            cancelPoll();
          }
        } catch {
          // "pending" error is expected until mobile completes auth
        }
      }, 2000);

      // Stop polling after 5 minutes
      timeoutRef.current = setTimeout(() => {
        cancelPoll();
        setQrError("This code expired after 5 minutes — generate a new one.");
        setQrChallenge(null);
        setQrExpiresAt(null);
      }, QR_TIMEOUT_MS);
    } catch (err) {
      setQrError(String(err));
      setIsGeneratingQr(false);
    }
  }, [cancelPoll, callbackPort]);

  const logout = useCallback(() => {
    setSession(null);
    setQrChallenge(null);
    setQrExpiresAt(null);
    setQrError(null);
  }, []);

  // Actually enforce expiresAt client-side too: the backend's SessionStore is the real
  // authority (it rejects an expired token on every privileged command regardless of what the
  // UI shows), but without this the frontend would keep displaying a "logged in" session
  // indefinitely after expiry until the user happened to trigger a privileged call and got a
  // rejection. Schedule a timer for the exact expiry moment rather than polling.
  useEffect(() => {
    if (!session) return;
    const msRemaining = session.expiresAt * 1000 - Date.now();
    if (msRemaining <= 0) {
      setSession(null);
      return;
    }
    const timer = setTimeout(() => setSession(null), msRemaining);
    return () => clearTimeout(timer);
  }, [session]);

  return (
    <AuthContext.Provider
      value={{ session, qrChallenge, qrExpiresAt, isGeneratingQr, qrError, callbackPort, requestQr, logout }}
    >
      {children}
    </AuthContext.Provider>
  );
}

export const useAuth = () => useContext(AuthContext);
