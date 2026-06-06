import type { Bucket, Category } from "./types";

// The six categories the user can assign, in display order.
export const CATEGORY_LIST: Category[] = [
  "productive",
  "study",
  "business",
  "neutral",
  "distraction",
  "recovery",
];

export interface CategoryMeta {
  label: string;
  color: string;
  bucket: Bucket;
  blurb: string;
}

// Single source of truth for category -> color + which high-level bucket it
// rolls up into. Keep this in sync with `bucket_for()` in the Rust backend.
export const CATEGORY_META: Record<string, CategoryMeta> = {
  productive: { label: "Productive", color: "#16a34a", bucket: "productive", blurb: "Deep, focused work" },
  study: { label: "Study", color: "#2563eb", bucket: "productive", blurb: "Learning & research" },
  business: { label: "Business", color: "#0d9488", bucket: "productive", blurb: "Admin, email, ops" },
  neutral: { label: "Neutral", color: "#64748b", bucket: "neutral", blurb: "Necessary but neutral" },
  distraction: { label: "Distraction", color: "#dc2626", bucket: "distracting", blurb: "Off-task time" },
  recovery: { label: "Recovery", color: "#9333ea", bucket: "neutral", blurb: "Intentional rest" },
  uncategorized: { label: "Uncategorized", color: "#cbd5e1", bucket: "neutral", blurb: "Not tagged yet" },
  ignore: { label: "Ignored", color: "#b0b7c3", bucket: "neutral", blurb: "Excluded by you" },
};

export const BUCKET_LIST: Bucket[] = ["productive", "neutral", "distracting"];

export const BUCKET_META: Record<Bucket, { label: string; color: string }> = {
  productive: { label: "Productive", color: "#16a34a" },
  neutral: { label: "Neutral", color: "#64748b" },
  distracting: { label: "Distracting", color: "#dc2626" },
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
