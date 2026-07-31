import { useEffect, useRef, useState } from "react";
import Sidebar, { navSectionForPage, type Page } from "./components/Sidebar";
import AccountabilityLayer from "./components/AccountabilityLayer";
import Dashboard from "./pages/Dashboard";
import Goals from "./pages/Goals";
import Focus from "./pages/Focus";
import DailyScore from "./pages/DailyScore";
import Timeline from "./pages/Timeline";
import TimeBreakdown from "./pages/TimeBreakdown";
import ActivityLog from "./pages/ActivityLog";
import BrowserActivity from "./pages/BrowserActivity";
import OutputEvents from "./pages/OutputEvents";
import Projects from "./pages/Projects";
import Categories from "./pages/Categories";
import DailyReview from "./pages/DailyReview";
import WeeklyReview from "./pages/WeeklyReview";
import Streaks from "./pages/Streaks";
import PrivacySettings from "./pages/PrivacySettings";
import SetupGuide from "./pages/SetupGuide";

type Theme = "light" | "dark";

function initialTheme(): Theme {
  if (typeof window === "undefined") return "light";
  const saved = window.localStorage.getItem("tempo_theme");
  if (saved === "light" || saved === "dark") return saved;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function SectionTabs({ page, onNavigate }: { page: Page; onNavigate: (page: Page) => void }) {
  const section = navSectionForPage(page);
  if (section.pages.length <= 1) return null;

  return (
    <nav className="section-tabs" aria-label={`${section.label} views`}>
      {section.pages.map((item) => (
        <button
          key={item.id}
          className={`section-tab${page === item.id ? " active" : ""}`}
          onClick={() => onNavigate(item.id)}
          aria-current={page === item.id ? "page" : undefined}
        >
          {item.label}
        </button>
      ))}
    </nav>
  );
}

export default function App() {
  const [page, setPage] = useState<Page>("dashboard");
  const [theme, setTheme] = useState<Theme>(initialTheme);
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const mainRef = useRef<HTMLElement | null>(null);
  const currentSection = navSectionForPage(page);
  const currentPageLabel = currentSection.pages.find((item) => item.id === page)?.label ?? currentSection.label;

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    window.localStorage.setItem("tempo_theme", theme);
  }, [theme]);

  useEffect(() => {
    if (mainRef.current) mainRef.current.scrollTop = 0;
    setMobileNavOpen(false);
  }, [page]);

  useEffect(() => {
    if (!mobileNavOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMobileNavOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [mobileNavOpen]);

  return (
    <div className={`app-shell${mobileNavOpen ? " nav-open" : ""}`}>
      <header className="mobile-app-bar">
        <button
          className="mobile-menu-button"
          type="button"
          aria-label="Open navigation"
          aria-expanded={mobileNavOpen}
          aria-controls="tempo-navigation"
          onClick={() => setMobileNavOpen(true)}
        >
          <span aria-hidden="true">☰</span>
        </button>
        <div className="mobile-app-title">
          <strong>Tempo</strong>
          <span>{currentPageLabel}</span>
        </div>
      </header>
      <Sidebar
        page={page}
        theme={theme}
        open={mobileNavOpen}
        onNavigate={setPage}
        onClose={() => setMobileNavOpen(false)}
        onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
      />
      {mobileNavOpen && (
        <button
          className="mobile-nav-backdrop"
          type="button"
          aria-label="Close navigation"
          onClick={() => setMobileNavOpen(false)}
        />
      )}
      <main className="main" ref={mainRef}>
        <SectionTabs page={page} onNavigate={setPage} />
        {page === "dashboard" && <Dashboard onNavigate={setPage} />}
        {page === "goals" && <Goals />}
        {page === "focus" && <Focus />}
        {page === "score" && <DailyScore />}
        {page === "breakdown" && <TimeBreakdown />}
        {page === "timeline" && <Timeline />}
        {page === "activity" && <ActivityLog />}
        {page === "browser" && <BrowserActivity />}
        {page === "outputs" && <OutputEvents />}
        {page === "projects" && <Projects />}
        {page === "categories" && <Categories />}
        {page === "review" && <DailyReview onNavigate={setPage} />}
        {page === "weekly" && <WeeklyReview onNavigate={setPage} />}
        {page === "streaks" && <Streaks />}
        {page === "privacy" && <PrivacySettings />}
        {page === "guide" && <SetupGuide onNavigate={setPage} />}
      </main>
      <AccountabilityLayer />
    </div>
  );
}
