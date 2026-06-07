import { useEffect, useState } from "react";
import Sidebar, { type Page } from "./components/Sidebar";
import AccountabilityLayer from "./components/AccountabilityLayer";
import Dashboard from "./pages/Dashboard";
import Goals from "./pages/Goals";
import Focus from "./pages/Focus";
import DailyScore from "./pages/DailyScore";
import Timeline from "./pages/Timeline";
import ActivityLog from "./pages/ActivityLog";
import BrowserActivity from "./pages/BrowserActivity";
import OutputEvents from "./pages/OutputEvents";
import Projects from "./pages/Projects";
import Categories from "./pages/Categories";
import DailyReview from "./pages/DailyReview";
import WeeklyReview from "./pages/WeeklyReview";
import Streaks from "./pages/Streaks";
import PrivacySettings from "./pages/PrivacySettings";

type Theme = "light" | "dark";

function initialTheme(): Theme {
  if (typeof window === "undefined") return "light";
  const saved = window.localStorage.getItem("tempo_theme");
  if (saved === "light" || saved === "dark") return saved;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export default function App() {
  const [page, setPage] = useState<Page>("dashboard");
  const [theme, setTheme] = useState<Theme>(initialTheme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    window.localStorage.setItem("tempo_theme", theme);
  }, [theme]);

  return (
    <div className="app-shell">
      <Sidebar
        page={page}
        theme={theme}
        onNavigate={setPage}
        onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
      />
      <main className="main">
        {page === "dashboard" && <Dashboard onNavigate={setPage} />}
        {page === "goals" && <Goals />}
        {page === "focus" && <Focus />}
        {page === "score" && <DailyScore />}
        {page === "timeline" && <Timeline />}
        {page === "activity" && <ActivityLog />}
        {page === "browser" && <BrowserActivity />}
        {page === "outputs" && <OutputEvents />}
        {page === "projects" && <Projects />}
        {page === "categories" && <Categories />}
        {page === "review" && <DailyReview />}
        {page === "weekly" && <WeeklyReview />}
        {page === "streaks" && <Streaks />}
        {page === "privacy" && <PrivacySettings />}
      </main>
      <AccountabilityLayer />
    </div>
  );
}
