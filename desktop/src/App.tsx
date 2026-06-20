import { Routes, Route, Navigate } from "react-router-dom";
import { ChainProvider } from "./context/ChainContext";
import { AuthProvider } from "./context/AuthContext";
import { AgentProvider } from "./context/AgentContext";
import Sidebar from "./components/Sidebar";
import ChainStatusBar from "./components/ChainStatusBar";
import ProposalsPage from "./pages/ProposalsPage";
import LawsPage from "./pages/LawsPage";
import TreasuryPage from "./pages/TreasuryPage";
import CourtsPage from "./pages/CourtsPage";
import AuthPage from "./pages/AuthPage";
import "./styles/app.css";

export default function App() {
  return (
    <ChainProvider>
      <AuthProvider>
        <AgentProvider>
          <div className="app-layout">
            <Sidebar />
            <div className="main-area">
              <ChainStatusBar />
              <div className="page-content">
                <Routes>
                  <Route path="/" element={<Navigate to="/proposals" replace />} />
                  <Route path="/proposals" element={<ProposalsPage />} />
                  <Route path="/laws" element={<LawsPage />} />
                  <Route path="/treasury" element={<TreasuryPage />} />
                  <Route path="/courts" element={<CourtsPage />} />
                  <Route path="/auth" element={<AuthPage />} />
                </Routes>
              </div>
            </div>
          </div>
        </AgentProvider>
      </AuthProvider>
    </ChainProvider>
  );
}
