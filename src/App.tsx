import { useEffect, useState } from "react";
import { DockerHealth, dockerHealth } from "./api";
import Dashboard from "./pages/Dashboard";
import Databases from "./pages/Databases";
import Destinations from "./pages/Destinations";
import History from "./pages/History";
import Schedules from "./pages/Schedules";
import Restore from "./pages/Restore";
import DevBackup from "./pages/DevBackup";
import Placeholder from "./pages/Placeholder";

type NavKey =
  | "dashboard"
  | "databases"
  | "quick"
  | "destinations"
  | "schedules"
  | "history"
  | "restore"
  | "settings";

const NAV: { key: NavKey; label: string }[] = [
  { key: "dashboard", label: "Dashboard" },
  { key: "databases", label: "Databases" },
  { key: "quick", label: "Quick Backup" },
  { key: "destinations", label: "Destinations" },
  { key: "schedules", label: "Schedules" },
  { key: "history", label: "History" },
  { key: "restore", label: "Restore" },
  { key: "settings", label: "Settings" },
];

export default function App() {
  const [nav, setNav] = useState<NavKey>("dashboard");
  const [docker, setDocker] = useState<DockerHealth | null>(null);

  useEffect(() => {
    dockerHealth()
      .then(setDocker)
      .catch(() => setDocker({ installed: false, running: false, version: null }));
  }, []);

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">DB</div>
          <div>
            <div className="brand-title">Database Backup</div>
            <div className="brand-sub">Manager</div>
          </div>
        </div>
        <nav>
          {NAV.map((n) => (
            <button
              key={n.key}
              className={nav === n.key ? "nav active" : "nav"}
              onClick={() => setNav(n.key)}
            >
              {n.label}
            </button>
          ))}
        </nav>
        <div className="docker-status">
          <span className={`dot ${docker?.running ? "ok" : docker?.installed ? "warn" : "bad"}`} />
          {docker == null
            ? "Checking Docker…"
            : docker.running
            ? `Docker ${docker.version ?? ""}`
            : docker.installed
            ? "Docker not running"
            : "Docker not installed"}
        </div>
      </aside>

      <main className="content">
        {nav === "dashboard" && <Dashboard />}
        {nav === "databases" && <Databases />}
        {nav === "quick" && <DevBackup />}
        {nav === "destinations" && <Destinations />}
        {nav === "history" && <History />}
        {nav === "schedules" && <Schedules />}
        {nav === "restore" && <Restore />}
        {nav === "settings" && <Placeholder title="Settings" phase="Phase 3+" />}
      </main>
    </div>
  );
}
