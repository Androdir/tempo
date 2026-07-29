import { useCallback, useEffect, useState } from "react";
import { getBrowserActivity, getCategoryDefinitions } from "../api";
import { categoryMeta, contentTypeMeta } from "../categories";
import ActivityDetailsDrawer from "../components/ActivityDetailsDrawer";
import { AppGlyph, BarRow, CategoryBadge } from "../components/ui";
import { formatDuration } from "../format";
import type { BrowserActivityView } from "../types";

export default function BrowserActivity() {
  const [data, setData] = useState<BrowserActivityView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [openId, setOpenId] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      const [activity] = await Promise.all([getBrowserActivity(), getCategoryDefinitions()]);
      setData(activity);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
    const t = window.setInterval(load, 15000);
    return () => window.clearInterval(t);
  }, [load]);

  if (error && !data) {
    return (
      <>
        <Head />
        <div className="error-box">{error}</div>
      </>
    );
  }
  if (!data) {
    return (
      <>
        <Head />
        <div className="loading">Loading browser activity…</div>
      </>
    );
  }

  const maxDomain = data.perDomain[0]?.seconds ?? 0;
  const empty = data.perDomain.length === 0 && data.recentPages.length === 0;

  return (
    <>
      <Head />
      {empty ? (
        <div className="card card-pad">
          <div className="empty">
            <div className="empty-glyph">🌐</div>
            <h3>No browser activity yet</h3>
            <p>
              Install the Tempo browser extension (see <code>extension/README.md</code>) and
              browse while Chrome is focused. Visits will appear here automatically.
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="card card-pad">
            <h2 className="card-title">Time per website</h2>
            <p className="card-hint">Domains visited today. Tag them on the Categories page.</p>
            <div className="bars">
              {data.perDomain.map((w) => {
                const meta = categoryMeta(w.category);
                return (
                  <BarRow
                    key={w.domain}
                    color={meta.color}
                    value={w.seconds}
                    max={maxDomain}
                    left={
                      <>
                        <AppGlyph name={w.domain} size={24} />
                        <span className="label">{w.domain}</span>
                      </>
                    }
                  />
                );
              })}
            </div>
          </div>

          <div className="card section-gap">
            <div className="card-pad" style={{ paddingBottom: 6 }}>
              <h2 className="card-title">Recent pages</h2>
              <p className="card-hint">Click a row to open full details.</p>
            </div>
            <table className="app-table">
              <thead>
                <tr>
                  <th>Page</th>
                  <th>Type</th>
                  <th>Category</th>
                  <th className="right">Duration</th>
                </tr>
              </thead>
              <tbody>
                {data.recentPages.map((p) => (
                  <tr key={p.id} className="clickable" onClick={() => setOpenId(p.id)}>
                    <td>
                      <div className="app-cell">
                        <AppGlyph name={p.domain} />
                        <div style={{ minWidth: 0 }}>
                          <div className="ellip">{p.pageTitle || p.domain}</div>
                          <div className="muted-num ellip" style={{ fontSize: 11.5 }}>
                            {p.domain}
                            {p.hasRaw ? " · raw stored" : ""}
                          </div>
                        </div>
                      </div>
                    </td>
                    <td>
                      {contentTypeMeta(p.contentType).icon}{" "}
                      <span className="muted-num">{contentTypeMeta(p.contentType).label}</span>
                    </td>
                    <td>
                      <CategoryBadge category={p.category} />
                    </td>
                    <td className="right muted-num">{formatDuration(p.durationSeconds)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}

      <ActivityDetailsDrawer id={openId} onClose={() => setOpenId(null)} onCorrected={load} />
    </>
  );
}

function Head() {
  return (
    <div className="page-head">
      <div>
        <h1 className="page-title">Websites</h1>
        <div className="page-subtitle">Website-only time and recent pages captured by the Tempo extension</div>
      </div>
    </div>
  );
}
