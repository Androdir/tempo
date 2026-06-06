import { useCallback, useEffect, useState } from "react";
import { generateDailyReview, getDailyReview, setDailyNote } from "../api";
import { LockinPlanCard } from "../components/LockinPlan";
import { formatLongDate } from "../format";
import type { DailyAiReview } from "../types";

export default function DailyReview() {
  const [review, setReview] = useState<DailyAiReview | null>(null);
  const [notes, setNotes] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [generating, setGenerating] = useState(false);

  const load = useCallback(async () => {
    try {
      const r = await getDailyReview();
      setReview(r);
      setNotes(r.notes);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function generate() {
    setGenerating(true);
    try {
      setReview(await generateDailyReview());
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
    }
  }

  async function saveNote() {
    try {
      await setDailyNote(notes);
    } catch (e) {
      setError(String(e));
    }
  }

  if (error && !review) {
    return (
      <>
        <Head />
        <div className="error-box">{error}</div>
      </>
    );
  }
  if (!review) {
    return (
      <>
        <Head />
        <div className="loading">Loading your review…</div>
      </>
    );
  }

  const r = review;

  return (
    <>
      <Head date={r.date} />
      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      <div className="card card-pad review-hero">
        <div className="review-hero-top">
          <span className={`src-chip ${r.source === "llm" ? "llm" : "rule"}`}>
            {r.source === "llm" ? `🤖 Local LLM${r.model ? ` · ${r.model}` : ""}` : "rule-based"}
          </span>
          <button className="btn btn-primary" onClick={generate} disabled={generating}>
            {generating ? "Thinking…" : r.generatedAt ? "Regenerate" : "Generate with AI"}
          </button>
        </div>
        <p className="review-verdict">{r.verdict}</p>
        {r.roast && <p className="review-roast">🔥 {r.roast}</p>}
      </div>

      <div className="two-col section-gap">
        <div className="card card-pad">
          <h2 className="card-title">Wins</h2>
          <ul className="review-list">
            {r.wins.map((w, i) => (
              <li key={i}>
                <span className="marker" style={{ color: "var(--good)" }}>✓</span>
                <span>{w}</span>
              </li>
            ))}
          </ul>
        </div>
        <div className="card card-pad">
          <h2 className="card-title">Problems</h2>
          <ul className="review-list">
            {r.problems.map((p, i) => (
              <li key={i}>
                <span className="marker" style={{ color: "var(--bad)" }}>✗</span>
                <span>{p}</span>
              </li>
            ))}
          </ul>
        </div>
      </div>

      <div className="card card-pad section-gap tomorrow-card">
        <h2 className="card-title">🎯 One goal for tomorrow</h2>
        <p className="tomorrow-text">{r.tomorrow}</p>
      </div>

      <div className="card card-pad section-gap">
        <h2 className="card-title">Your notes</h2>
        <p className="card-hint">Add context — saved locally and fed to the LLM when you generate.</p>
        <textarea
          className="pf-textarea"
          style={{ width: "100%" }}
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          onBlur={saveNote}
          placeholder="e.g. slow after lunch, gym at 6, shipped the editor feature, slept badly"
        />
      </div>

      {r.source === "fallback" && (
        <p className="muted-num" style={{ marginTop: 12 }}>
          This is a rule-based review. Enable the local LLM in <b>Privacy &amp; Settings → Local AI
          classification</b> and start Ollama for a sharper, funnier one.
        </p>
      )}

      <LockinPlanCard />
    </>
  );
}

function Head({ date }: { date?: string }) {
  return (
    <div className="page-head">
      <div>
        <h1 className="page-title">Daily Review</h1>
        <div className="page-subtitle">
          {date ? formatLongDate(date) : "Your blunt daily review — locally generated"}
        </div>
      </div>
    </div>
  );
}
