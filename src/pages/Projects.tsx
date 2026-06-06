import { useCallback, useEffect, useState } from "react";
import { createProject, deleteProject, getProjects, updateProject } from "../api";
import { CATEGORY_LIST, CATEGORY_META } from "../categories";
import { CategoryBadge } from "../components/ui";
import type { Category, Project } from "../types";

const BLANK = {
  id: 0,
  name: "",
  category: "study" as Category,
  priority: 50,
  keywords: "",
  apps: "",
  domains: "",
};

function toList(s: string): string[] {
  return s.split(/[\n,]/).map((x) => x.trim()).filter(Boolean);
}
function fromList(a: string[]): string {
  return a.join(", ");
}

export default function Projects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [form, setForm] = useState(BLANK);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setProjects(await getProjects());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  function edit(p: Project) {
    setForm({
      id: p.id,
      name: p.name,
      category: p.category,
      priority: p.priority,
      keywords: fromList(p.keywords),
      apps: fromList(p.apps),
      domains: fromList(p.domains),
    });
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  async function save() {
    const payload = {
      name: form.name.trim(),
      category: form.category,
      priority: Number(form.priority) || 0,
      keywords: toList(form.keywords),
      apps: toList(form.apps),
      domains: toList(form.domains),
    };
    if (!payload.name) {
      setError("Project name is required");
      return;
    }
    try {
      if (form.id > 0) await updateProject({ id: form.id, ...payload });
      else await createProject(payload);
      setForm(BLANK);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(id: number) {
    if (!confirm("Delete this project?")) return;
    try {
      await deleteProject(id);
      if (form.id === id) setForm(BLANK);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Projects &amp; Goals</h1>
          <div className="page-subtitle">
            Define intents — activity is matched to them by app/domain and keywords.
          </div>
        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      <div className="card card-pad">
        <h2 className="card-title">{form.id > 0 ? "Edit project" : "New project"}</h2>
        <div className="project-form">
          <label className="pf-field">
            <span>Name</span>
            <input
              className="search"
              style={{ width: "100%" }}
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="Exam studying"
            />
          </label>
          <label className="pf-field">
            <span>Category</span>
            <select
              className="select"
              value={form.category}
              onChange={(e) => setForm({ ...form, category: e.target.value as Category })}
            >
              {CATEGORY_LIST.map((c) => (
                <option key={c} value={c}>{CATEGORY_META[c].label}</option>
              ))}
            </select>
          </label>
          <label className="pf-field">
            <span>Priority (0–100)</span>
            <input
              className="search"
              type="number"
              min={0}
              max={100}
              value={form.priority}
              onChange={(e) => setForm({ ...form, priority: Number(e.target.value) })}
            />
          </label>
          <label className="pf-field pf-wide">
            <span>Keywords <em>(comma or newline separated)</em></span>
            <textarea
              className="pf-textarea"
              value={form.keywords}
              onChange={(e) => setForm({ ...form, keywords: e.target.value })}
              placeholder="Hungarian Algorithm, Stable Marriage, LCS, suffix trees"
            />
          </label>
          <label className="pf-field">
            <span>Related apps</span>
            <textarea
              className="pf-textarea"
              value={form.apps}
              onChange={(e) => setForm({ ...form, apps: e.target.value })}
              placeholder="RemNote, PDF reader"
            />
          </label>
          <label className="pf-field">
            <span>Related domains</span>
            <textarea
              className="pf-textarea"
              value={form.domains}
              onChange={(e) => setForm({ ...form, domains: e.target.value })}
              placeholder="chatgpt.com, remnote.com"
            />
          </label>
        </div>
        <div className="head-actions" style={{ marginTop: 12 }}>
          <button className="btn btn-primary" onClick={save}>
            {form.id > 0 ? "Save changes" : "Create project"}
          </button>
          {form.id > 0 && (
            <button className="btn" onClick={() => setForm(BLANK)}>Cancel</button>
          )}
        </div>
      </div>

      <div className="project-grid section-gap">
        {projects.map((p) => (
          <div className="card card-pad project-card" key={p.id}>
            <div className="project-card-head">
              <div>
                <div className="project-card-name">{p.name}</div>
                <CategoryBadge category={p.category} />
              </div>
              <div className="project-prio" title="Priority">★ {p.priority}</div>
            </div>
            <ChipList label="Keywords" items={p.keywords} />
            <ChipList label="Apps" items={p.apps} />
            <ChipList label="Domains" items={p.domains} />
            <div className="project-card-actions">
              <button className="btn" onClick={() => edit(p)}>Edit</button>
              <button className="btn btn-danger" onClick={() => remove(p.id)}>Delete</button>
            </div>
          </div>
        ))}
        {projects.length === 0 && (
          <div className="card card-pad">
            <div className="empty">
              <div className="empty-glyph">🎯</div>
              <h3>No projects yet</h3>
              <p>Create one above to start matching activity to your goals.</p>
            </div>
          </div>
        )}
      </div>
    </>
  );
}

function ChipList({ label, items }: { label: string; items: string[] }) {
  if (!items.length) return null;
  return (
    <div className="project-chips">
      <span className="project-chips-label">{label}</span>
      <div className="kw-chips">
        {items.map((x) => (
          <span className="kw-chip" key={x}>{x}</span>
        ))}
      </div>
    </div>
  );
}
