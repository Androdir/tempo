import { useCallback, useEffect, useState } from "react";
import {
  getDailyReview,
  showNativeNotification,
  onDailyReviewDue,
  onDistractionWarning,
  onFocusViolation,
  setDistractionIntentional,
  setDistractionSnooze,
} from "../api";
import type { DailyAiReview } from "../types";

type ToastKind = "distraction" | "focus";
interface Toast {
  id: number;
  kind: ToastKind;
  title: string;
  body: string;
  target?: string;
}

/** Fire a local toast from anywhere (used by the "preview warning" buttons). */
export function previewToast(kind: ToastKind, title: string, body: string, target?: string) {
  void showNativeNotification(`Tempo · ${title}`, body);
  window.dispatchEvent(new CustomEvent("tempo-toast", { detail: { kind, title, body, target } }));
}

/**
 * Always-mounted layer that turns backend accountability events into on-screen
 * toasts + local OS notifications, and shows the end-of-day review modal.
 * Everything here is local; no network involved.
 */
export default function AccountabilityLayer() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [review, setReview] = useState<DailyAiReview | null>(null);

  const push = useCallback(
    (kind: ToastKind, title: string, body: string, target?: string) => {
      const id = Date.now() + Math.random();
      setToasts((cur) => [...cur, { id, kind, title, body, target }].slice(-4));
      window.setTimeout(() => setToasts((cur) => cur.filter((t) => t.id !== id)), 12000);

    },
    [],
  );

  const dismiss = useCallback((id: number) => {
    setToasts((cur) => cur.filter((t) => t.id !== id));
  }, []);

  useEffect(() => {
    const onTest = (e: Event) => {
      const d = (e as CustomEvent).detail || {};
      push(d.kind ?? "distraction", d.title ?? "Distraction check", d.body ?? "", d.target);
    };
    window.addEventListener("tempo-toast", onTest);

    const subs = [
      onDistractionWarning((w) => push("distraction", "Distraction check", w.message, w.key)),
      onFocusViolation((v) => push("focus", "Focus mode", v.message)),
      onDailyReviewDue(async () => {
        try {
          setReview(await getDailyReview());
        } catch {
          /* ignore */
        }
      }),
    ];

    return () => {
      window.removeEventListener("tempo-toast", onTest);
      subs.forEach((p) => p.then((un) => un()));
    };
  }, [push]);

  return (
    <>
      <div className="toast-stack">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast-${t.kind}`} role="alert">
            <div className="toast-icon">{t.kind === "distraction" ? "⏳" : "🎯"}</div>
            <div className="toast-body">
              <div className="toast-title">{t.title}</div>
              <div className="toast-text">{t.body}</div>
              {t.kind === "distraction" && (
                <div className="toast-actions">
                  <button
                    className="toast-btn"
                    onClick={() => {
                      setDistractionSnooze(30);
                      dismiss(t.id);
                    }}
                  >
                    Snooze 30m
                  </button>
                  {t.target && (
                    <button
                      className="toast-btn"
                      onClick={() => {
                        setDistractionIntentional(t.target!, 60);
                        dismiss(t.id);
                      }}
                    >
                      It's intentional
                    </button>
                  )}
                </div>
              )}
            </div>
            <button className="toast-x" onClick={() => dismiss(t.id)}>
              ✕
            </button>
          </div>
        ))}
      </div>
      {review && <EodModal review={review} onClose={() => setReview(null)} />}
    </>
  );
}

function EodModal({ review, onClose }: { review: DailyAiReview; onClose: () => void }) {
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2 className="modal-title">🌙 End of day</h2>
          <button className="toast-x" onClick={onClose}>
            ✕
          </button>
        </div>
        <p className="review-verdict">{review.verdict}</p>
        {review.roast && <p className="review-roast">🔥 {review.roast}</p>}
        <div className="eod-cols">
          <div>
            <h3 className="card-title">Wins</h3>
            <ul className="review-list">
              {review.wins.map((w, i) => (
                <li key={i}>
                  <span className="marker" style={{ color: "var(--good)" }}>✓</span>
                  <span>{w}</span>
                </li>
              ))}
            </ul>
          </div>
          <div>
            <h3 className="card-title">Problems</h3>
            <ul className="review-list">
              {review.problems.map((p, i) => (
                <li key={i}>
                  <span className="marker" style={{ color: "var(--bad)" }}>✗</span>
                  <span>{p}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>
        <div className="eod-tomorrow">
          <b>🎯 Tomorrow:</b> {review.tomorrow}
        </div>
        <div className="modal-actions">
          <button className="btn btn-primary" onClick={onClose}>
            Got it
          </button>
        </div>
      </div>
    </div>
  );
}
