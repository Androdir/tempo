// Local, non-AI summarisation + content-type detection (ES module used by the
// background service worker).

const STOPWORDS = new Set(
  ("the a an and or but if then else of to in on at for with without from by as is are was were be been being this that these those it its it's you your we our they them he she his her my me i do does did not no yes can will just so than too very more most some any all how what when where which who whom why about into over under out up down off again once here there then also like get got make made use used using new now one two three see read watch click page home menu sign log search").split(
    /\s+/
  )
);

export function summarize(extracted) {
  const text = (extracted.text || "").slice(0, 30000);
  const headings = (extracted.headings || []).slice(0, 4);

  const blocks = text
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
  const firstParas = blocks.filter((b) => b.length >= 60).slice(0, 2);

  const freq = new Map();
  const words = text.toLowerCase().match(/[a-z][a-z0-9'+-]{2,}/g) || [];
  for (const w of words) {
    if (STOPWORDS.has(w)) continue;
    freq.set(w, (freq.get(w) || 0) + 1);
  }
  const keywords = [...freq.entries()]
    .filter(([, n]) => n >= 1)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 12)
    .map(([w]) => w);

  let summary = "";
  if (headings.length) summary += headings.join(" · ");
  if (firstParas.length) summary += (summary ? " — " : "") + firstParas.join(" ");
  summary = summary.slice(0, 600).trim();

  return { summary, keywords };
}

export function detectContentType(url, flags) {
  let host = "";
  let path = "";
  try {
    const u = new URL(url);
    host = u.hostname.replace(/^www\./, "");
    path = u.pathname;
  } catch {
    return "other";
  }

  const has = (s) => host.includes(s);

  if ((has("youtube.com") && (path.startsWith("/watch") || path.startsWith("/shorts"))) || has("vimeo.com")) {
    return "video";
  }
  if (has("chatgpt.com") || has("chat.openai.com") || has("claude.ai") || has("gemini.google.com") || has("perplexity.ai")) {
    return "chat";
  }
  if (has("instagram.com") || has("tiktok.com") || has("twitter.com") || host === "x.com" || has("facebook.com") || has("reddit.com")) {
    return "social_feed";
  }
  if ((has("google.") && path.startsWith("/search")) || (has("bing.com") && path.startsWith("/search")) || has("duckduckgo.com")) {
    return "search_results";
  }
  if (has("github.com") || has("gitlab.com") || has("docs.google.com") || has("notion.so") || has("remnote.com") || has("overleaf.com") || has("stackoverflow.com")) {
    return "docs_editor";
  }
  if (flags && flags.hasVideo) return "video";
  if (flags && flags.hasArticle) return "article";
  return "other";
}
