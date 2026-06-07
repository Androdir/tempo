export type Page =
  | "dashboard"
  | "goals"
  | "focus"
  | "score"
  | "timeline"
  | "activity"
  | "browser"
  | "outputs"
  | "projects"
  | "categories"
  | "review"
  | "weekly"
  | "streaks"
  | "privacy";

const NAV: { id: Page; label: string; icon: string }[] = [
  { id: "dashboard", label: "Dashboard", icon: "📊" },
  { id: "goals", label: "Daily Goals", icon: "✅" },
  { id: "focus", label: "Focus Mode", icon: "🧘" },
  { id: "score", label: "Daily Score", icon: "🔥" },
  { id: "timeline", label: "Timeline", icon: "📈" },
  { id: "activity", label: "Activity Log", icon: "🗂️" },
  { id: "browser", label: "Browser Activity", icon: "🌐" },
  { id: "outputs", label: "Output Events", icon: "📦" },
  { id: "projects", label: "Projects & Goals", icon: "🎯" },
  { id: "categories", label: "Categories", icon: "🏷️" },
  { id: "review", label: "Daily Review", icon: "📋" },
  { id: "weekly", label: "Weekly Review", icon: "📅" },
  { id: "streaks", label: "Streaks", icon: "🗓️" },
  { id: "privacy", label: "Privacy & Settings", icon: "🔒" },
];

export default function Sidebar({
  page,
  theme,
  onNavigate,
  onToggleTheme,
}: {
  page: Page;
  theme: "light" | "dark";
  onNavigate: (p: Page) => void;
  onToggleTheme: () => void;
}) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">◷</div>
        <div>
          <div className="brand-name">Tempo</div>
          <div className="brand-sub">Local productivity</div>
        </div>
      </div>

      <nav className="nav">
        {NAV.map((item) => (
          <button
            key={item.id}
            className={`nav-item${page === item.id ? " active" : ""}`}
            onClick={() => onNavigate(item.id)}
          >
            <span className="nav-icon">{item.icon}</span>
            {item.label}
          </button>
        ))}
      </nav>

      <div className="sidebar-spacer" />

      <button className="theme-toggle" onClick={onToggleTheme} type="button">
        <span className="theme-toggle-icon">{theme === "dark" ? "☀️" : "🌙"}</span>
        <span>{theme === "dark" ? "Light mode" : "Dark mode"}</span>
      </button>

      <div className="privacy-card">
        <div className="privacy-title">🔒 Private by design</div>
        <ul>
          <li>No keystrokes recorded</li>
          <li>No audio captured</li>
          <li>No data leaves this device</li>
          <li>Stored locally in SQLite</li>
        </ul>
      </div>
    </aside>
  );
}
