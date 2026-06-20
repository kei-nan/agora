import { useEffect } from "react";
import { QRCodeSVG } from "qrcode.react";
import { useAuth } from "../context/AuthContext";
import "./Page.css";
import "./AuthPage.css";

export default function AuthPage() {
  const { session, qrChallenge, isGeneratingQr, qrError, requestQr, logout } = useAuth();

  // Auto-generate QR when the page opens and no session/challenge exists yet
  useEffect(() => {
    if (!session && !qrChallenge && !isGeneratingQr) {
      requestQr();
    }
  }, []);

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
            Identity: {session.nullifierHash.slice(0, 8)}…{session.nullifierHash.slice(-6)}
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
