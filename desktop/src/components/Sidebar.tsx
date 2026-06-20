import { NavLink, useNavigate } from "react-router-dom";
import { useAuth } from "../context/AuthContext";
import "./Sidebar.css";

const NAV_ITEMS = [
  { to: "/proposals", label: "Proposals", icon: "📋" },
  { to: "/laws", label: "Laws", icon: "⚖️" },
  { to: "/treasury", label: "Treasury", icon: "🏛️" },
  { to: "/courts", label: "Courts", icon: "🔨" },
];

export default function Sidebar() {
  const { session } = useAuth();
  const navigate = useNavigate();

  return (
    <nav className="sidebar">
      <div className="sidebar-logo">
        <span className="logo-mark">⛓</span>
        <span className="logo-text">Agora</span>
      </div>

      <ul className="sidebar-nav">
        {NAV_ITEMS.map(({ to, label, icon }) => (
          <li key={to}>
            <NavLink to={to} className={({ isActive }) => (isActive ? "nav-item active" : "nav-item")}>
              <span className="nav-icon">{icon}</span>
              <span>{label}</span>
            </NavLink>
          </li>
        ))}
      </ul>

      <div className="sidebar-footer">
        {session ? (
          <div className="session-badge">
            <span className="session-dot" />
            <span className="session-label">Authenticated</span>
          </div>
        ) : (
          <button className="auth-btn" onClick={() => navigate("/auth")}>
            Sign in with phone
          </button>
        )}
      </div>
    </nav>
  );
}
