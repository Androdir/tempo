import type { ReactNode } from "react";
import { categoryMeta } from "../categories";
import { glyphColor, hexAlpha, initials } from "../color";
import { formatDuration } from "../format";

/** Colored rounded square with an app's initials. */
export function AppGlyph({ name, size = 26 }: { name: string; size?: number }) {
  return (
    <span
      className="app-glyph"
      style={{ background: glyphColor(name), width: size, height: size }}
    >
      {initials(name)}
    </span>
  );
}

/** Small pill showing a category with its color. `null` => "Uncategorized". */
export function CategoryBadge({ category }: { category: string | null }) {
  const m = categoryMeta(category);
  return (
    <span
      className="badge"
      style={{
        background: hexAlpha(m.color, 0.12),
        color: m.color,
        borderColor: hexAlpha(m.color, 0.25),
      }}
    >
      <span className="dot" style={{ background: m.color }} />
      {m.label}
    </span>
  );
}

/** A single metric tile. */
export function StatCard({
  label,
  value,
  foot,
  chip,
}: {
  label: ReactNode;
  value: string;
  foot?: ReactNode;
  chip?: string;
}) {
  return (
    <div className="stat">
      <div className="stat-label">
        {chip && <span className="stat-chip" style={{ background: chip }} />}
        {label}
      </div>
      <div className="stat-value">{value}</div>
      {foot && <div className="stat-foot">{foot}</div>}
    </div>
  );
}

/** Horizontal labelled bar used in the per-app and per-category lists. */
export function BarRow({
  left,
  color,
  value,
  max,
}: {
  left: ReactNode;
  color: string;
  value: number;
  max: number;
}) {
  const pct = max > 0 ? Math.max(3, Math.round((value / max) * 100)) : 0;
  return (
    <div className="bar-row">
      <div className="bar-name">{left}</div>
      <div className="bar-track">
        <div className="bar-fill" style={{ width: `${pct}%`, background: color }} />
      </div>
      <div className="bar-value">{formatDuration(value)}</div>
    </div>
  );
}

/** Project match chip with a confidence level. */
export function ProjectTag({ name, confidence }: { name: string; confidence: number }) {
  const color = confidence >= 75 ? "#16a34a" : confidence >= 50 ? "#d97706" : "#94a3b8";
  return (
    <span className="project-tag" title={`Match confidence ${confidence}%`}>
      <span className="project-dot" style={{ background: color }} />
      <span className="project-name">{name}</span>
      <span className="project-conf" style={{ color }}>{confidence}%</span>
    </span>
  );
}

/** A single stacked proportion bar (used for the productive/neutral/distracting split). */
export function StackedBar({
  segments,
  total,
}: {
  segments: { color: string; value: number }[];
  total: number;
}) {
  return (
    <div className="stacked">
      {total > 0 &&
        segments.map(
          (s, i) =>
            s.value > 0 && (
              <div
                key={i}
                className="stacked-seg"
                style={{ width: `${(s.value / total) * 100}%`, background: s.color }}
              />
            )
        )}
    </div>
  );
}
