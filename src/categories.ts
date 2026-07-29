import type { Bucket, Category, CategoryDefinition } from "./types";

export type CategoryMeta = Omit<CategoryDefinition, "id" | "builtIn">;

export const DEFAULT_CATEGORY_DEFINITIONS: CategoryDefinition[] = [
  { id: "productive", label: "Productive", color: "#16a34a", bucket: "productive", blurb: "Deep, focused work", builtIn: true },
  { id: "study", label: "Study", color: "#2563eb", bucket: "productive", blurb: "Learning & research", builtIn: true },
  { id: "business", label: "Business", color: "#0d9488", bucket: "productive", blurb: "Admin, email, ops", builtIn: true },
  { id: "neutral", label: "Neutral", color: "#64748b", bucket: "neutral", blurb: "Necessary or ambiguous; no automatic penalty", builtIn: true },
  { id: "distraction", label: "Distraction", color: "#dc2626", bucket: "distracting", blurb: "Off-task time", builtIn: true },
  { id: "recovery", label: "Recovery", color: "#9333ea", bucket: "neutral", blurb: "Intentional rest", builtIn: true },
];

// Default categories are still available immediately; pages can register the
// DB-backed definitions after loading.
export let CATEGORY_LIST: Category[] = DEFAULT_CATEGORY_DEFINITIONS.map((c) => c.id);

export interface SpecialCategoryMeta {
  label: string;
  color: string;
  bucket: Bucket;
  blurb: string;
}

// Single source of truth for category -> color + which high-level bucket it
// rolls up into. Keep this in sync with `bucket_for()` in the Rust backend.
export let CATEGORY_META: Record<string, SpecialCategoryMeta> = {
  ...Object.fromEntries(
    DEFAULT_CATEGORY_DEFINITIONS.map((c) => [
      c.id,
      { label: c.label, color: c.color, bucket: c.bucket, blurb: c.blurb },
    ]),
  ),
  uncategorized: { label: "Uncategorized", color: "#cbd5e1", bucket: "neutral", blurb: "Not tagged yet" },
  ignore: { label: "Ignored", color: "#b0b7c3", bucket: "neutral", blurb: "Excluded by you" },
};

export function registerCategoryDefinitions(defs: CategoryDefinition[]) {
  CATEGORY_LIST = defs.map((c) => c.id);
  CATEGORY_META = {
    ...Object.fromEntries(
      defs.map((c) => [
        c.id,
        { label: c.label, color: c.color, bucket: c.bucket, blurb: c.blurb },
      ]),
    ),
    uncategorized: CATEGORY_META.uncategorized,
    ignore: CATEGORY_META.ignore,
  };
}

export const BUCKET_LIST: Bucket[] = ["productive", "neutral", "distracting"];

export const BUCKET_META: Record<Bucket, { label: string; color: string; description: string }> = {
  productive: {
    label: "Productive",
    color: "#16a34a",
    description: "Supports a goal and earns productive credit.",
  },
  neutral: {
    label: "Neutral",
    color: "#64748b",
    description: "Necessary, ambiguous, or recovery time. Tracked, but it earns no productive credit and triggers no distraction warning.",
  },
  distracting: {
    label: "Distracting",
    color: "#dc2626",
    description: "Pulls you away from your declared goals and can trigger a nudge.",
  },
};

export function categoryMeta(c: string | null | undefined): CategoryMeta {
  return CATEGORY_META[c ?? "uncategorized"] ?? CATEGORY_META.uncategorized;
}

export const CONTENT_TYPE_META: Record<string, { label: string; icon: string }> = {
  article: { label: "Article", icon: "📄" },
  video: { label: "Video", icon: "▶️" },
  chat: { label: "Chat", icon: "💬" },
  docs_editor: { label: "Docs / Editor", icon: "📝" },
  social_feed: { label: "Social feed", icon: "📲" },
  search_results: { label: "Search", icon: "🔍" },
  other: { label: "Page", icon: "🌐" },
};

export function contentTypeMeta(t: string | null | undefined) {
  return CONTENT_TYPE_META[t ?? "other"] ?? CONTENT_TYPE_META.other;
}

export const CAPTURE_MODE_META: Record<string, { label: string; color: string; hint: string }> = {
  text: { label: "Readable text", color: "#16a34a", hint: "Page content can be captured" },
  meta: { label: "Title / URL only", color: "#64748b", hint: "No page content captured" },
  never: { label: "Never capture", color: "#dc2626", hint: "Blocked — never captured" },
};

export function captureModeMeta(m: string | null | undefined) {
  return CAPTURE_MODE_META[m ?? "meta"] ?? CAPTURE_MODE_META.meta;
}
