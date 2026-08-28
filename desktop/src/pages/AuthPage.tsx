import { useEffect, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { useAuth } from "../context/AuthContext";
import "./Page.css";
import "./AuthPage.css";

function formatRemaining(ms: number): string {
  const totalSeconds = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

export default function AuthPage() {
  const { session, qrChallenge, qrExpiresAt, isGeneratingQr, qrError, requestQr, logout } = useAuth();
  const [remainingMs, setRemainingMs] = useState<number | null>(null);

  // Auto-generate QR when the page opens and no session/challenge exists yet
  useEffect(() => {
    if (!session && !qrChallenge && !isGeneratingQr) {
      requestQr();
    }
  }, []);

  // Live countdown while a QR code is displayed and awaiting mobile confirmation
  useEffect(() => {
    if (!qrChallenge || !qrExpiresAt) {
      setRemainingMs(null);
      return;
    }
    setRemainingMs(qrExpiresAt - Date.now());
    const intervalId = setInterval(() => {
      setRemainingMs(qrExpiresAt - Date.now());
    }, 1000);
    return () => clearInterval(intervalId);
  }, [qrChallenge, qrExpiresAt]);

  if (session) {
    return (
      <div className="auth-page">
        <div className="auth-card">
          <div className="auth-success-icon">✓</div>
          <h2>Authenticated</h2>
          <p className="auth-meta">
            Session valid until {new Date(session.expiresAt * 1000).toLocaleTimeString()}
          </p>
          <p className="auth-nullifier">
            Session ID: {session.nullifierHash.slice(0, 6)}…{session.nullifierHash.slice(-4)}
          </p>
          <button className="auth-logout-btn" onClick={logout}>
            Sign out
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="auth-page">
      <div className="auth-card">
        <h2>Sign in with your phone</h2>
        <p className="auth-instructions">
          Open the Agora mobile app and scan this QR code. Your identity stays on your phone.
        </p>

        {qrChallenge ? (
          <div className="qr-wrapper">
            <QRCodeSVG value={qrChallenge} size={240} level="M" />
            <p className="qr-hint">Waiting for mobile confirmation...</p>
            {remainingMs !== null && (
              <p className="qr-countdown">Code expires in {formatRemaining(remainingMs)}</p>
            )}
          </div>
        ) : (
          <>
            <button className="auth-btn-large" onClick={requestQr} disabled={isGeneratingQr}>
              {isGeneratingQr ? "Generating..." : "Generate QR code"}
            </button>
            {qrError && <p className="auth-error">{qrError}</p>}
          </>
        )}
      </div>
    </div>
  );
}
