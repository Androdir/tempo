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
  | "privacy"
  | "guide";

export type NavSection = {
  id: string;
  label: string;
  description: string;
  icon: string;
  defaultPage: Page;
  pages: { id: Page; label: string }[];
};

export const NAV_SECTIONS: NavSection[] = [
  {
    id: "today",
    label: "Today",
    description: "Your day at a glance",
    icon: "🏠",
    defaultPage: "dashboard",
    pages: [{ id: "dashboard", label: "Today" }],
  },
  {
    id: "plan",
    label: "Plan",
    description: "Goals and focus sessions",
    icon: "🎯",
    defaultPage: "goals",
    pages: [
      { id: "goals", label: "Daily Goals" },
      { id: "focus", label: "Focus Mode" },
    ],
  },
  {
    id: "activity",
    label: "Activity",
    description: "Time and work evidence",
    icon: "📈",
    defaultPage: "timeline",
    pages: [
      { id: "timeline", label: "Timeline" },
      { id: "activity", label: "Classifications" },
      { id: "browser", label: "Websites" },
      { id: "outputs", label: "Outputs" },
    ],
  },
  {
    id: "insights",
    label: "Insights",
    description: "Scores, reviews and streaks",
    icon: "🔥",
    defaultPage: "score",
    pages: [
      { id: "score", label: "Daily Score" },
      { id: "review", label: "Daily Review" },
      { id: "weekly", label: "Weekly Review" },
      { id: "streaks", label: "Streaks" },
    ],
  },
  {
    id: "projects",
    label: "Projects",
    description: "Matching and categories",
    icon: "🗂️",
    defaultPage: "projects",
    pages: [
      { id: "projects", label: "Projects" },
      { id: "categories", label: "Categories" },
    ],
  },
  {
    id: "settings",
    label: "Settings",
    description: "Setup, devices and data",
    icon: "⚙️",
    defaultPage: "privacy",
    pages: [
      { id: "privacy", label: "Settings" },
      { id: "guide", label: "Setup Guide" },
    ],
  },
];

export function navSectionForPage(page: Page): NavSection {
  return NAV_SECTIONS.find((section) => section.pages.some((item) => item.id === page))
    ?? NAV_SECTIONS[0];
}

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
        {NAV_SECTIONS.map((section) => {
          const active = section.pages.some((item) => item.id === page);
          return (
            <button
              key={section.id}
              className={`nav-item${active ? " active" : ""}`}
              onClick={() => onNavigate(active ? page : section.defaultPage)}
              aria-label={section.label}
              aria-current={active ? "page" : undefined}
            >
              <span className="nav-icon">{section.icon}</span>
              <span className="nav-copy">
                <span className="nav-label">{section.label}</span>
                <span className="nav-description">{section.description}</span>
              </span>
            </button>
          );
        })}
      </nav>

      <div className="sidebar-spacer" />

      <button
        className={`setup-guide-link${page === "guide" ? " active" : ""}`}
        onClick={() => onNavigate("guide")}
        type="button"
      >
        <span aria-hidden="true">?</span>
        <span>
          <b>Setup &amp; guide</b>
          <small>Get started or troubleshoot</small>
        </span>
      </button>

      <button className="theme-toggle" onClick={onToggleTheme} type="button">
        <span className="theme-toggle-icon">{theme === "dark" ? "☀️" : "🌙"}</span>
        <span>{theme === "dark" ? "Light mode" : "Dark mode"}</span>
      </button>

      <div className="privacy-card">
        <div className="privacy-title">🔒 Private by design</div>
        <ul>
          <li>No keystrokes recorded</li>
          <li>No audio captured</li>
          <li>Local-only by default</li>
          <li>Optional sync is self-hosted</li>
        </ul>
      </div>
    </aside>
  );
}
