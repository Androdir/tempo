import { FormEvent, useCallback, useEffect, useState } from "react";
import {
  addWatchedFolder,
  getOutputEvents,
  getWatchedFolders,
  onOutputsUpdated,
  removeWatchedFolder,
  scanOutputsNow,
  updateWatchedFolder,
} from "../api";
import type { OutputEvent, OutputEventType, WatchedFolder } from "../types";

export const OUTPUT_META: Record<string, { label: string; icon: string; color: string }> = {
  video_export: { label: "Video export", icon: "🎬", color: "#0d9488" },
  editing_project_changed: { label: "Editing project", icon: "🎞️", color: "#0d9488" },
  code_change: { label: "Code change", icon: "💻", color: "#16a34a" },
  document_created: { label: "Document", icon: "📄", color: "#2563eb" },
  study_material: { label: "Study material", icon: "📚", color: "#2563eb" },
  download: { label: "Download", icon: "⬇️", color: "#64748b" },
  content_asset: { label: "Content asset", icon: "🖼️", color: "#9333ea" },
  other: { label: "Output", icon: "📦", color: "#64748b" },
};

export function outputMeta(t: string) {
  return OUTPUT_META[t] ?? OUTPUT_META.other;
}

const CORE_TYPES: OutputEventType[] = [
  "video_export",
  "code_change",
  "document_created",
  "download",
  "study_material",
  "other",
];

function todayIso(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function timeOf(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

export function formatBytes(n: number): string {
  if (!n || n <= 0) return "—";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${u[i]}`;
}

export default function OutputEvents() {
  const [day, setDay] = useState(todayIso());
  const [events, setEvents] = useState<OutputEvent[] | null>(null);
  const [folders, setFolders] = useState<WatchedFolder[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [showAdd, setShowAdd] = useState(false);

  const load = useCallback(async () => {
    try {
      const [e, f] = await Promise.all([getOutputEvents(day), getWatchedFolders()]);
      setEvents(e);
      setFolders(f);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }, [day]);

  useEffect(() => {
    load();
    let un: (() => void) | undefined;
    onOutputsUpdated(load).then((u) => (un = u));
    return () => un?.();
  }, [load]);

  async function scan() {
    setScanning(true);
    try {
      await scanOutputsNow();
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  }

  async function toggleFolder(f: WatchedFolder) {
    try {
      await updateWatchedFolder({ ...f, enabled: !f.enabled });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(id: number) {
    try {
      await removeWatchedFolder(id);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Output Events</h1>
          <div className="page-subtitle">
            Actual things you shipped — detected from watched folders (file metadata only, never contents).
          </div>
        </div>
        <div className="head-actions tl-controls">
          <input type="date" className="tl-date" value={day} max={todayIso()} onChange={(e) => setDay(e.target.value || todayIso())} />
          <button className="btn" onClick={scan} disabled={scanning}>
            {scanning ? "Scanning…" : "Scan now"}
          </button>
        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      {/* Watched folders */}
      <div className="card card-pad">
        <div className="missions-head">
          <h2 className="card-title">📁 Watched folders</h2>
          <button className="link-btn" onClick={() => setShowAdd((v) => !v)}>
            {showAdd ? "Close" : "Add folder →"}
          </button>
        </div>
        <p className="card-hint">
          Only the folders you add here are watched — never system folders. The watcher reads file
          name, size and timestamps only; it never opens file contents.
        </p>

        {folders.length === 0 ? (
          <p className="empty-hint" style={{ margin: "6px 0 0" }}>
            No folders watched yet. Add your video export folder, a coding repo, or your downloads.
          </p>
        ) : (
          <ul className="folder-list">
            {folders.map((f) => {
              const m = outputMeta(f.outputType);
              return (
                <li key={f.id} className={`folder-row ${f.enabled ? "" : "off"}`}>
                  <span className="folder-icon">{m.icon}</span>
                  <div className="folder-main">
                    <div className="folder-label">
                      {f.label}
                      <span className="out-badge" style={{ color: m.color, borderColor: m.color }}>{m.label}</span>
                      {f.project && <span className="goal-chip">{f.project}</span>}
                    </div>
                    <div className="folder-path muted-num">{f.path}</div>
                    <div className="folder-meta muted-num">
                      {f.extensions.length ? f.extensions.map((e) => `.${e}`).join(" ") : "any type"}
                      {f.minSizeBytes > 0 ? ` · ≥ ${formatBytes(f.minSizeBytes)}` : ""}
                      {` · ${f.debounceSeconds}s settle`}
                    </div>
                  </div>
                  <button
                    className={`pill-toggle ${f.enabled ? "on" : ""}`}
                    onClick={() => toggleFolder(f)}
                    title={f.enabled ? "Watching — click to pause" : "Paused — click to watch"}
                  >
                    {f.enabled ? "Watching" : "Paused"}
                  </button>
                  <button className="goal-del" onClick={() => remove(f.id)} aria-label="Remove folder">✕</button>
                </li>
              );
            })}
          </ul>
        )}

        {showAdd && <AddFolderForm onAdded={() => { setShowAdd(false); load(); }} onError={setError} />}
      </div>

      {/* Events */}
      <div className="card section-gap">
        {!events ? (
          <div className="loading">Loading outputs…</div>
        ) : events.length === 0 ? (
          <div className="empty">
            <div className="empty-glyph">📦</div>
            <h3>No outputs detected for this day</h3>
            <p>Once you watch a folder, exported videos, code changes and saved documents show up here.</p>
          </div>
        ) : (
          <table className="app-table">
            <thead>
              <tr>
                <th>Output</th>
                <th style={{ width: 150 }}>Type</th>
                <th style={{ width: 160 }}>Linked activity</th>
                <th className="right" style={{ width: 90 }}>Size</th>
                <th className="right" style={{ width: 64 }}>Time</th>
              </tr>
            </thead>
            <tbody>
              {events.map((e) => {
                const m = outputMeta(e.eventType);
                return (
                  <tr key={e.id}>
                    <td>
                      <div className="app-cell">
                        <span className="folder-icon">{m.icon}</span>
                        <div style={{ minWidth: 0 }}>
                          <div className="ellip">{e.fileName}</div>
                          <div className="muted-num ellip" style={{ fontSize: 11.5 }}>{e.folderPath}</div>
                        </div>
                      </div>
                    </td>
                    <td>
                      <span className="out-badge" style={{ color: m.color, borderColor: m.color }}>{m.label}</span>
                      {e.project && <div className="muted-num" style={{ fontSize: 11.5, marginTop: 3 }}>{e.project}</div>}
                    </td>
                    <td className="muted-num">{e.linkedLabel ?? "—"}</td>
                    <td className="right muted-num">{formatBytes(e.fileSize)}</td>
                    <td className="right muted-num">{timeOf(e.modifiedAt ?? e.timestamp)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      <p className="card-hint" style={{ marginTop: 10 }}>
        Detected outputs feed your Daily Score (a real video export is stronger proof than time spent),
        the Timeline, and your streaks.
      </p>
    </>
  );
}

function AddFolderForm({
  onAdded,
  onError,
}: {
  onAdded: () => void;
  onError: (e: string) => void;
}) {
  const [path, setPath] = useState("");
  const [label, setLabel] = useState("");
  const [project, setProject] = useState("");
  const [outputType, setOutputType] = useState<OutputEventType>("video_export");
  const [exts, setExts] = useState("");
  const [minMb, setMinMb] = useState("0");
  const [debounce, setDebounce] = useState("5");
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!path.trim() || busy) return;
    setBusy(true);
    try {
      await addWatchedFolder({
        path: path.trim(),
        label: label.trim(),
        project: project.trim() || null,
        outputType,
        enabled: true,
        extensions: exts
          .split(/[\s,]+/)
          .map((x) => x.trim().replace(/^\./, "").toLowerCase())
          .filter(Boolean),
        minSizeBytes: Math.max(0, Math.round((parseFloat(minMb) || 0) * 1_000_000)),
        debounceSeconds: Math.max(0, parseInt(debounce, 10) || 0),
      });
      onAdded();
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="folder-form" onSubmit={submit}>
      <input
        className="pf-input"
        placeholder="Folder path  e.g. C:\Users\you\Videos\Exports"
        value={path}
        onChange={(e) => setPath(e.target.value)}
      />
      <div className="folder-form-row">
        <input className="pf-input" placeholder="Label (optional)" value={label} onChange={(e) => setLabel(e.target.value)} />
        <select className="pf-select" value={outputType} onChange={(e) => setOutputType(e.target.value as OutputEventType)}>
          {CORE_TYPES.map((t) => (
            <option key={t} value={t}>{outputMeta(t).label}</option>
          ))}
        </select>
        <input className="pf-input" placeholder="Project (optional)" value={project} onChange={(e) => setProject(e.target.value)} />
      </div>
      <div className="folder-form-row">
        <input className="pf-input" placeholder="Extensions  e.g. mp4, mov (blank = any)" value={exts} onChange={(e) => setExts(e.target.value)} />
        <label className="folder-num">
          Min size
          <input className="pf-input" type="number" min="0" step="0.1" value={minMb} onChange={(e) => setMinMb(e.target.value)} />
          MB
        </label>
        <label className="folder-num">
          Settle
          <input className="pf-input" type="number" min="0" value={debounce} onChange={(e) => setDebounce(e.target.value)} />
          s
        </label>
      </div>
      <div className="folder-form-actions">
        <button className="btn btn-primary" type="submit" disabled={busy || !path.trim()}>
          {busy ? "Adding…" : "Watch this folder"}
        </button>
        <span className="card-hint" style={{ margin: 0 }}>Tip: paste the full absolute path.</span>
      </div>
    </form>
  );
}
