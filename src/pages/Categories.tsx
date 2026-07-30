import { useCallback, useEffect, useMemo, useState } from "react";
import {
  deleteCategoryDefinition,
  deleteCategoryRule,
  getCategoryDefinitions,
  getClassificationPolicies,
  getTrackedApps,
  getTrackedDomains,
  setCategoryRule,
  setClassificationPolicies,
  setDomainRule,
  upsertCategoryDefinition,
} from "../api";
import {
  BUCKET_LIST,
  BUCKET_META,
  captureModeMeta,
} from "../categories";
import { AppGlyph } from "../components/ui";
import { formatDuration } from "../format";
import type { Bucket, Category, CategoryDefinition, ClassificationPolicy, TrackedApp, TrackedDomain } from "../types";

type Tab = "all" | "apps" | "websites";

export default function Categories() {
  const [tab, setTab] = useState<Tab>("all");
  const [apps, setApps] = useState<TrackedApp[]>([]);
  const [domains, setDomains] = useState<TrackedDomain[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [categoryDefs, setCategoryDefs] = useState<CategoryDefinition[]>([]);
  const [policies, setPolicies] = useState<ClassificationPolicy[]>([]);
  const [savingPolicies, setSavingPolicies] = useState(false);
  const [draft, setDraft] = useState<CategoryDefinition>({
    id: "",
    label: "",
    color: "#64748b",
    bucket: "neutral",
    blurb: "",
    builtIn: false,
  });

  const load = useCallback(async () => {
    try {
      const [a, d, defs, savedPolicies] = await Promise.all([getTrackedApps(), getTrackedDomains(), getCategoryDefinitions(), getClassificationPolicies()]);
      setApps(a);
      setDomains(d);
      setCategoryDefs(defs);
      setPolicies(savedPolicies);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  function editCategory(c: CategoryDefinition) {
    setDraft({ ...c });
  }

  async function saveCategory() {
    const id = draft.id.trim().toLowerCase().replace(/\s+/g, "-");
    if (!id || !draft.label.trim()) {
      setError("Category id and label are required");
      return;
    }
    try {
      await upsertCategoryDefinition({
        ...draft,
        id,
        label: draft.label.trim(),
        color: draft.color || "#64748b",
        blurb: draft.blurb.trim(),
      });
      setDraft({ id: "", label: "", color: "#64748b", bucket: "neutral", blurb: "", builtIn: false });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeCategory(c: CategoryDefinition) {
    if (!confirm(`Delete category "${c.label}"? Existing rules and projects will move to a remaining fallback category.`)) return;
    try {
      await deleteCategoryDefinition(c.id);
      if (draft.id === c.id) {
        setDraft({ id: "", label: "", color: "#64748b", bucket: "neutral", blurb: "", builtIn: false });
      }
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  function updatePolicy(id: string, patch: Partial<ClassificationPolicy>) {
    setPolicies((current) => current.map((policy) => policy.id === id ? { ...policy, ...patch } : policy));
  }

  function addPolicy() {
    const suffix = Date.now().toString(36);
    setPolicies((current) => [...current, {
      id: `custom-${suffix}`,
      name: "New policy",
      category: "distraction",
      kinds: [],
      terms: [],
      enabled: true,
      builtIn: false,
      priority: 50,
    }]);
  }

  async function savePolicies() {
    try {
      setSavingPolicies(true);
      await setClassificationPolicies(policies);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingPolicies(false);
    }
  }
  useEffect(() => {
    load();
  }, [load]);

  async function changeApp(a: TrackedApp, value: string) {
    const category = value === "" ? null : (value as Category);
    setApps((prev) => prev.map((x) => (x.appName === a.appName ? { ...x, category } : x)));
    try {
      if (category) await setCategoryRule(a.appName, category, a.aiReview);
      else await deleteCategoryRule(a.appName);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  async function toggleAppAi(a: TrackedApp, aiReview: boolean) {
    if (!a.category) return; // need a category rule before flagging an app
    setApps((prev) => prev.map((x) => (x.appName === a.appName ? { ...x, aiReview } : x)));
    try {
      await setCategoryRule(a.appName, a.category, aiReview);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  async function changeDomain(d: TrackedDomain, value: string) {
    const category = value === "" ? null : (value as Category);
    setDomains((prev) => prev.map((x) => (x.domain === d.domain ? { ...x, category } : x)));
    try {
      // Preserve the existing capture mode + AI-review flag.
      await setDomainRule(d.domain, category, d.captureMode, d.aiReview);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  async function toggleDomainAi(d: TrackedDomain, aiReview: boolean) {
    setDomains((prev) => prev.map((x) => (x.domain === d.domain ? { ...x, aiReview } : x)));
    try {
      await setDomainRule(d.domain, d.category, d.captureMode, aiReview);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  const q = filter.trim().toLowerCase();
  const visibleApps = useMemo(
    () => (q ? apps.filter((a) => a.appName.toLowerCase().includes(q)) : apps),
    [apps, q]
  );
  const visibleDomains = useMemo(
    () => (q ? domains.filter((d) => d.domain.toLowerCase().includes(q)) : domains),
    [domains, q]
  );

  return (
    <div className="categories-page">
      <div className="page-head">
        <div>
          <h1 className="page-title">Apps &amp; websites</h1>
          <div className="page-subtitle">Assign a category once; Tempo reuses it for future activity.</div>
        </div>
        <div className="head-actions">
          <div className="segmented" aria-label="Show apps, websites, or both">
            <button className={tab === "all" ? "on" : ""} onClick={() => setTab("all")}>All</button>
            <button className={tab === "apps" ? "on" : ""} onClick={() => setTab("apps")}>Apps</button>
            <button className={tab === "websites" ? "on" : ""} onClick={() => setTab("websites")}>Websites</button>
          </div>
          <input
            className="search"
            placeholder={`Filter ${tab === "all" ? "apps & websites" : tab}…`}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
      </div>

      {error && <div className="error-box section-gap">{error}</div>}

      <div className="card section-gap">
        {loading ? (
          <div className="loading">Loading…</div>
        ) : (
          <>
            {(tab === "all" || tab === "apps") && (
              <section className="category-source-section">
                {tab === "all" && (
                  <div className="category-source-head">
                    <div>
                      <h2 className="card-title">Desktop apps</h2>
                      <p className="card-hint">{visibleApps.length} shown · {apps.length} tracked</p>
                    </div>
                  </div>
                )}
                <CategoryTable
                  categories={categoryDefs}
                  empty={apps.length === 0}
                  emptyTitle="No apps tracked yet"
                  emptyHint="Once the tracker has seen a few apps, they’ll appear here."
                  rows={visibleApps.map((a) => ({
                    key: a.appName,
                    name: a.appName,
                    seconds: a.totalSeconds,
                    category: a.category,
                    extra: null,
                    onChange: (v: string) => changeApp(a, v),
                    aiReview: a.aiReview,
                    aiDisabled: !a.category,
                    onToggleAi: (v: boolean) => toggleAppAi(a, v),
                  }))}
                />
              </section>
            )}

            {(tab === "all" || tab === "websites") && (
              <section className={`category-source-section ${tab === "all" ? "with-divider" : ""}`}>
                {tab === "all" && (
                  <div className="category-source-head">
                    <div>
                      <h2 className="card-title">Websites</h2>
                      <p className="card-hint">{visibleDomains.length} shown · {domains.length} tracked</p>
                    </div>
                  </div>
                )}
                <CategoryTable
                  categories={categoryDefs}
                  empty={domains.length === 0}
                  emptyTitle="No websites tracked yet"
                  emptyHint="Connect the browser extension, browse normally, and visited sites will appear here."
                  rows={visibleDomains.map((d) => ({
                    key: d.domain,
                    name: d.domain,
                    seconds: d.totalSeconds,
                    category: d.category,
                    extra: (
                      <span className="badge" style={{ color: captureModeMeta(d.captureMode).color }}>
                        {captureModeMeta(d.captureMode).label}
                      </span>
                    ),
                    onChange: (v: string) => changeDomain(d, v),
                    aiReview: d.aiReview,
                    onToggleAi: (v: boolean) => toggleDomainAi(d, v),
                  }))}
                />
              </section>
            )}
          </>
        )}
      </div>

      <details className="card card-pad section-gap category-advanced classification-policies">
        <summary>Classification policies</summary>
        <p className="card-hint">
          Tempo first identifies what an activity is, then applies your policy. For example, the built-in Game policy makes recognised games distractions without labelling each title. Manual app/site rules and strong project matches remain exceptions. Saving a policy clears stale local-AI results and re-evaluates past activity when you open that day; your manual corrections remain in place.
        </p>
        <div className="policy-list">
          {policies.map((policy) => (
            <div className="policy-row" key={policy.id}>
              <label className="ai-toggle policy-enabled" title="Use this policy">
                <input type="checkbox" checked={policy.enabled} onChange={(e) => updatePolicy(policy.id, { enabled: e.target.checked })} />
                <span>{policy.enabled ? "On" : "Off"}</span>
              </label>
              <input className="input" value={policy.name} aria-label="Policy name" onChange={(e) => updatePolicy(policy.id, { name: e.target.value })} />
              <select className="select" value={policy.category} aria-label={`${policy.name} category`} onChange={(e) => updatePolicy(policy.id, { category: e.target.value })}>
                {categoryDefs.map((category) => <option key={category.id} value={category.id}>{category.label}</option>)}
              </select>
              <input
                className="input policy-kinds"
                defaultValue={policy.kinds.join(", ")}
                aria-label={`${policy.name} activity types`}
                placeholder="Activity types: game, chat…"
                onBlur={(e) => updatePolicy(policy.id, { kinds: e.target.value.split(",").map((kind) => kind.trim()).filter(Boolean) })}
              />
              <textarea
                className="input policy-terms"
                defaultValue={policy.terms.join(", ")}
                aria-label={`${policy.name} matching terms`}
                placeholder="gameplay, Steam, Minecraft…"
                onBlur={(e) => updatePolicy(policy.id, { terms: e.target.value.split(",").map((term) => term.trim()).filter(Boolean) })}
              />
              <div className="policy-actions">
                {policy.builtIn ? <span className="badge">Built in</span> : (
                  <button className="btn ghost small" onClick={() => setPolicies((current) => current.filter((item) => item.id !== policy.id))}>Delete</button>
                )}
              </div>
            </div>
          ))}
        </div>
        <div className="policy-footer">
          <button className="btn ghost" onClick={addPolicy}>+ Add policy</button>
          <button className="btn primary" disabled={savingPolicies} onClick={savePolicies}>{savingPolicies ? "Saving…" : "Save policies"}</button>
        </div>
      </details>
      <details className="card card-pad section-gap category-advanced">
        <summary>Customize categories</summary>
        <p className="card-hint">Optional: keep the defaults, rename them, or add your own.</p>
        <CategoryManager
          defs={categoryDefs}
          draft={draft}
          onDraft={setDraft}
          onEdit={editCategory}
          onSave={saveCategory}
          onDelete={removeCategory}
        />
        <BucketLegend defs={categoryDefs} />
      </details>
    </div>
  );
}

interface Row {
  key: string;
  name: string;
  seconds: number;
  category: Category | null;
  extra: React.ReactNode;
  onChange: (value: string) => void;
  aiReview: boolean;
  aiDisabled?: boolean;
  onToggleAi: (value: boolean) => void;
}

function CategoryTable({
  rows,
  categories,
  empty,
  emptyTitle,
  emptyHint,
}: {
  rows: Row[];
  categories: CategoryDefinition[];
  empty: boolean;
  emptyTitle: string;
  emptyHint: string;
}) {
  if (empty) {
    return (
      <div className="empty">
        <div className="empty-glyph">🏷️</div>
        <h3>{emptyTitle}</h3>
        <p>{emptyHint}</p>
      </div>
    );
  }
  if (rows.length === 0) {
    return (
      <div className="empty">
        <div className="empty-glyph">🔍</div>
        <h3>No matches</h3>
        <p>Try a different search term.</p>
      </div>
    );
  }
  return (
    <table className="app-table">
      <thead>
        <tr>
          <th>Name</th>
          <th className="right">Tracked</th>
          <th style={{ width: 190 }}>Category</th>
          <th style={{ width: 160 }}>Local AI check</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key}>
            <td>
              <div className="app-cell">
                <AppGlyph name={r.name} />
                {r.name}
                {r.extra}
              </div>
            </td>
            <td className="right muted-num">{formatDuration(r.seconds)}</td>
            <td>
              <select
                className="select"
                value={r.category ?? ""}
                onChange={(e) => r.onChange(e.target.value)}
                style={{ width: "100%" }}
              >
                <option value="">Uncategorized</option>
                {categories.map((c) => (
                  <option key={c.id} value={c.id}>{c.label}</option>
                ))}
              </select>
            </td>
            <td>
              <label
                className="ai-toggle"
                title={
                  r.aiDisabled
                    ? "Set a category first"
                    : "Always ask your local Ollama model to double-check this rule, even when Tempo is already confident"
                }
              >
                <input
                  type="checkbox"
                  checked={r.aiReview}
                  disabled={r.aiDisabled}
                  onChange={(e) => r.onToggleAi(e.target.checked)}
                />
                <span>Always double-check</span>
              </label>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function CategoryManager({
  defs,
  draft,
  onDraft,
  onEdit,
  onSave,
  onDelete,
}: {
  defs: CategoryDefinition[];
  draft: CategoryDefinition;
  onDraft: (c: CategoryDefinition) => void;
  onEdit: (c: CategoryDefinition) => void;
  onSave: () => void;
  onDelete: (c: CategoryDefinition) => void;
}) {
  return (
    <div className="card card-pad">
      <h2 className="card-title">Edit categories</h2>
      <p className="card-hint">Keep the defaults, rename them, or add your own categories.</p>
      <div className="category-editor">
        <input
          className="search"
          placeholder="category-id"
          value={draft.id}
          disabled={draft.builtIn}
          onChange={(e) => onDraft({ ...draft, id: e.target.value })}
        />
        <input
          className="search"
          placeholder="Label"
          value={draft.label}
          onChange={(e) => onDraft({ ...draft, label: e.target.value })}
        />
        <input
          className="color-input"
          type="color"
          value={draft.color}
          onChange={(e) => onDraft({ ...draft, color: e.target.value })}
          aria-label="Category color"
        />
        <select className="select" value={draft.bucket} onChange={(e) => onDraft({ ...draft, bucket: e.target.value as Bucket })}>
          {BUCKET_LIST.map((b) => (
            <option key={b} value={b}>{BUCKET_META[b].label}</option>
          ))}
        </select>
        <input
          className="search"
          placeholder="Short description"
          value={draft.blurb}
          onChange={(e) => onDraft({ ...draft, blurb: e.target.value })}
        />
        <button className="btn btn-primary" onClick={onSave}>
          {draft.id ? "Save" : "Add"}
        </button>
      </div>
      <div className="category-pills">
        {defs.map((c) => (
          <span className="category-pill" key={c.id}>
            <span className="dot" style={{ background: c.color }} />
            <button className="link-inline" onClick={() => onEdit(c)}>{c.label}</button>
            <span className="muted-num">{BUCKET_META[c.bucket].label}</span>
            <button
              className="icon-btn"
              onClick={() => onDelete(c)}
              disabled={defs.length <= 1}
              title={defs.length <= 1 ? "At least one category must remain" : "Delete category"}
              aria-label={`Delete ${c.label}`}
            >
              ×
            </button>
          </span>
        ))}
      </div>
    </div>
  );
}

function BucketLegend({ defs }: { defs: CategoryDefinition[] }) {
  return (
    <div className="card card-pad">
      <h2 className="card-title">How categories roll up</h2>
      <p className="card-hint">Categories group into three dashboard buckets.</p>
      <div className="bucket-legend">
        {BUCKET_LIST.map((bucket) => (
          <div className="bucket-col" key={bucket}>
            <div className="bucket-head">
              <span className="legend-dot" style={{ background: BUCKET_META[bucket].color }} />
              {BUCKET_META[bucket].label}
            </div>
            <p className="bucket-description">{BUCKET_META[bucket].description}</p>
            <div className="bucket-cats">
              {defs.filter((c) => c.bucket === bucket).map((c) => (
                <div className="bucket-cat" key={c.id}>
                  <span className="dot" style={{ background: c.color }} />
                  <span>{c.label}</span>
                  <span className="bucket-cat-blurb">{c.blurb}</span>
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
