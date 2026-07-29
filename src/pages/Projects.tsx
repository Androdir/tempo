import { useCallback, useEffect, useState } from "react";
import {
  createProject,
  deleteProject,
  getCategoryDefinitions,
  getProjects,
  testProjectMatch,
  updateProject,
} from "../api";
import { CategoryBadge } from "../components/ui";
import type { Category, CategoryDefinition, Project, ProjectMatchTest } from "../types";

const BLANK = {
  id: 0,
  name: "",
  category: "business" as Category,
  priority: 50,
  keywords: "",
  apps: "",
  domains: "",
  excludedApps: "",
  excludedDomains: "",
  excludedKeywords: "",
};

function toList(value: string): string[] {
  return value.split(/[\n,]/).map((item) => item.trim()).filter(Boolean);
}

function fromList(values: string[]): string {
  return values.join(", ");
}

function explainSignal(signal: string): string {
  const [kind, ...rest] = signal.split(":");
  const value = rest.join(":").trim();
  if (kind === "app/domain") return `Related app or domain: ${value}`;
  if (kind === "title keyword") return `Title contains: “${value}”`;
  if (kind === "content keyword") return `Captured content contains: “${value}”`;
  if (kind.startsWith("excluded")) return `Blocked by ${kind.replace("excluded ", "")} “${value}”`;
  return signal;
}

export default function Projects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [categories, setCategories] = useState<CategoryDefinition[]>([]);
  const [form, setForm] = useState({ ...BLANK });
  const [error, setError] = useState<string | null>(null);
  const [testIdentifier, setTestIdentifier] = useState("");
  const [testTitle, setTestTitle] = useState("");
  const [testContent, setTestContent] = useState("");
  const [testResult, setTestResult] = useState<ProjectMatchTest | null>(null);
  const [testing, setTesting] = useState(false);

  const load = useCallback(async () => {
    try {
      const [projectRows, categoryRows] = await Promise.all([getProjects(), getCategoryDefinitions()]);
      setProjects(projectRows);
      setCategories(categoryRows);
      setError(null);
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  function projectFromForm(): Project {
    return {
      id: form.id,
      name: form.name.trim() || "Unsaved project",
      category: form.category,
      priority: Number(form.priority) || 0,
      keywords: toList(form.keywords),
      apps: toList(form.apps),
      domains: toList(form.domains),
      excludedApps: toList(form.excludedApps),
      excludedDomains: toList(form.excludedDomains),
      excludedKeywords: toList(form.excludedKeywords),
    };
  }

  function edit(project: Project) {
    setForm({
      id: project.id,
      name: project.name,
      category: project.category,
      priority: project.priority,
      keywords: fromList(project.keywords),
      apps: fromList(project.apps),
      domains: fromList(project.domains),
      excludedApps: fromList(project.excludedApps),
      excludedDomains: fromList(project.excludedDomains),
      excludedKeywords: fromList(project.excludedKeywords),
    });
    setTestResult(null);
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  async function save() {
    const payload = projectFromForm();
    if (!form.name.trim()) {
      setError("Project name is required");
      return;
    }
    try {
      if (form.id > 0) await updateProject({ ...payload, name: form.name.trim() });
      else {
        const { id: _id, ...draft } = payload;
        await createProject({ ...draft, name: form.name.trim() });
      }
      setForm({ ...BLANK });
      setTestResult(null);
      await load();
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function runTest() {
    if (!testIdentifier.trim() && !testTitle.trim() && !testContent.trim()) {
      setError("Enter an example app/domain, title, or captured-content excerpt to test");
      return;
    }
    setTesting(true);
    try {
      setTestResult(await testProjectMatch(
        projectFromForm(),
        testIdentifier.trim(),
        testTitle.trim(),
        testContent.trim(),
      ));
      setError(null);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setTesting(false);
    }
  }

  async function remove(id: number) {
    if (!confirm("Delete this project?")) return;
    try {
      await deleteProject(id);
      if (form.id === id) setForm({ ...BLANK });
      await load();
    } catch (cause) {
      setError(String(cause));
    }
  }

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Projects</h1>
          <div className="page-subtitle">
            Define intent, preview exactly how matching behaves, and block false associations.
          </div>
        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      <div className="card card-pad">
        <h2 className="card-title">{form.id > 0 ? "Edit project" : "New project"}</h2>
        <p className="card-hint project-confidence-guide">
          A related app/domain alone is a 50% candidate. One matching keyword raises it to 85%, above Tempo's 60% assignment threshold. Exclusions always win.
        </p>
        <div className="project-form">
          <label className="pf-field">
            <span>Name</span>
            <input
              className="search"
              style={{ width: "100%" }}
              value={form.name}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
              placeholder="Content Creation"
            />
          </label>
          <label className="pf-field">
            <span>Category</span>
            <select
              className="select"
              value={form.category}
              onChange={(event) => setForm({ ...form, category: event.target.value as Category })}
            >
              {categories.map((category) => (
                <option key={category.id} value={category.id}>{category.label}</option>
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
              onChange={(event) => setForm({ ...form, priority: Number(event.target.value) })}
            />
          </label>
          <label className="pf-field pf-wide">
            <span>Keywords <em>(comma or newline separated)</em></span>
            <textarea
              className="pf-textarea"
              value={form.keywords}
              onChange={(event) => setForm({ ...form, keywords: event.target.value })}
              placeholder="DaVinci Resolve, editing, hooks, captions, YouTube Shorts"
            />
          </label>
          <label className="pf-field">
            <span>Related apps</span>
            <textarea
              className="pf-textarea"
              value={form.apps}
              onChange={(event) => setForm({ ...form, apps: event.target.value })}
              placeholder="DaVinci Resolve, CapCut, OBS Studio"
            />
          </label>
          <label className="pf-field">
            <span>Related domains</span>
            <textarea
              className="pf-textarea"
              value={form.domains}
              onChange={(event) => setForm({ ...form, domains: event.target.value })}
              placeholder="youtube.com, instagram.com, tiktok.com"
            />
          </label>
        </div>

        <details className="project-exclusions" open={Boolean(form.excludedApps || form.excludedDomains || form.excludedKeywords)}>
          <summary>
            <span>Never match rules</span>
            <small>Use these to block known false positives. They always override positive evidence.</small>
          </summary>
          <div className="project-form project-exclusion-grid">
            <label className="pf-field">
              <span>Excluded apps</span>
              <textarea
                className="pf-textarea"
                value={form.excludedApps}
                onChange={(event) => setForm({ ...form, excludedApps: event.target.value })}
                placeholder="Telegram Desktop"
              />
            </label>
            <label className="pf-field">
              <span>Excluded domains</span>
              <textarea
                className="pf-textarea"
                value={form.excludedDomains}
                onChange={(event) => setForm({ ...form, excludedDomains: event.target.value })}
                placeholder="reddit.com"
              />
            </label>
            <label className="pf-field">
              <span>Excluded words or phrases</span>
              <textarea
                className="pf-textarea"
                value={form.excludedKeywords}
                onChange={(event) => setForm({ ...form, excludedKeywords: event.target.value })}
                placeholder="personal, unrelated client"
              />
            </label>
          </div>
        </details>

        <div className="project-match-tester">
          <div className="project-match-tester-head">
            <div>
              <h3>Test this matcher</h3>
              <p>Use a real example before saving. Nothing is recorded.</p>
            </div>
            <button className="btn" onClick={runTest} disabled={testing}>
              {testing ? "Testing…" : "Test match"}
            </button>
          </div>
          <div className="project-test-inputs">
            <input
              className="search"
              value={testIdentifier}
              onChange={(event) => setTestIdentifier(event.target.value)}
              placeholder="App or domain, e.g. Telegram Desktop"
              aria-label="Test app or domain"
            />
            <input
              className="search"
              value={testTitle}
              onChange={(event) => setTestTitle(event.target.value)}
              placeholder="Window or page title"
              aria-label="Test window or page title"
            />
            <input
              className="search"
              value={testContent}
              onChange={(event) => setTestContent(event.target.value)}
              placeholder="Optional captured-content excerpt"
              aria-label="Test captured content"
            />
          </div>
          {testResult && (
            <div className={`project-test-result ${testResult.status}`} role="status">
              <div className="project-test-result-head">
                <strong>{testResult.explanation}</strong>
                <span>{testResult.confidence}%</span>
              </div>
              {testResult.signals.length > 0 && (
                <ul>
                  {testResult.signals.map((signal) => <li key={signal}>{explainSignal(signal)}</li>)}
                </ul>
              )}
            </div>
          )}
        </div>

        <div className="head-actions" style={{ marginTop: 12 }}>
          <button className="btn btn-primary" onClick={save}>
            {form.id > 0 ? "Save changes" : "Create project"}
          </button>
          {form.id > 0 && (
            <button className="btn" onClick={() => { setForm({ ...BLANK }); setTestResult(null); }}>Cancel</button>
          )}
        </div>
      </div>

      <div className="project-grid section-gap">
        {projects.map((project) => (
          <div className="card card-pad project-card" key={project.id}>
            <div className="project-card-head">
              <div>
                <div className="project-card-name">{project.name}</div>
                <CategoryBadge category={project.category} />
              </div>
              <div className="project-prio" title="Priority">★ {project.priority}</div>
            </div>
            <ChipList label="Keywords" items={project.keywords} />
            <ChipList label="Apps" items={project.apps} />
            <ChipList label="Domains" items={project.domains} />
            <ChipList label="Never apps" items={project.excludedApps} excluded />
            <ChipList label="Never domains" items={project.excludedDomains} excluded />
            <ChipList label="Never words" items={project.excludedKeywords} excluded />
            <div className="project-card-actions">
              <button className="btn" onClick={() => edit(project)}>Edit &amp; test</button>
              <button className="btn btn-danger" onClick={() => remove(project.id)}>Delete</button>
            </div>
          </div>
        ))}
        {projects.length === 0 && (
          <div className="card card-pad">
            <div className="empty">
              <div className="empty-glyph">🎯</div>
              <h3>No projects yet</h3>
              <p>Create one above, then test it with a real app/title example before relying on it.</p>
            </div>
          </div>
        )}
      </div>
    </>
  );
}

function ChipList({ label, items, excluded = false }: { label: string; items: string[]; excluded?: boolean }) {
  if (!items.length) return null;
  return (
    <div className={`project-chips${excluded ? " exclusions" : ""}`}>
      <span className="project-chips-label">{label}</span>
      <div className="kw-chips">
        {items.map((item) => <span className="kw-chip" key={item}>{item}</span>)}
      </div>
    </div>
  );
}