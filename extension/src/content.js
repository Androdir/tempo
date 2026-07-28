// Tempo content script.
//
// Two jobs:
//   1. Emit a "tick" every 10s, ONLY while this tab is the visible, OS-focused
//      tab (document.hasFocus() is false whenever another desktop app is in
//      front — that's how we honour "when Chrome is the active window").
//   2. On request, extract *visible readable text* — never input values,
//      passwords, hidden nodes, nav menus, or ads.
//
// The script is passive: it extracts nothing unless the background asks.

(() => {
  if (window.top !== window) return; // top frame only

  const tempoBrowser = globalThis.browser || globalThis.chrome;
  const SAMPLE_MS = 10000;

  // --- tick loop -----------------------------------------------------------
  setInterval(() => {
    if (document.visibilityState === "visible" && document.hasFocus()) {
      try {
        tempoBrowser.runtime.sendMessage({ action: "tick" });
      } catch {
        /* background asleep / extension reloading — ignore */
      }
    }
  }, SAMPLE_MS);

  // --- extraction request --------------------------------------------------
  tempoBrowser.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg && msg.action === "extract") {
      try {
        sendResponse(extractReadable(msg.maxLength || 8000));
      } catch (e) {
        sendResponse({ error: String(e) });
      }
    }
    // synchronous response
  });

  // --- the extractor -------------------------------------------------------

  const SKIP_TAGS = new Set([
    "SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "SVG", "CANVAS", "IFRAME",
    "INPUT", "TEXTAREA", "SELECT", "OPTION", "OBJECT", "EMBED", "AUDIO", "VIDEO",
  ]);
  const NAV_TAGS = new Set(["NAV", "FOOTER", "ASIDE"]);
  const NAV_ROLES = new Set(["navigation", "banner", "complementary", "search", "contentinfo"]);
  // Class/id hints for nav, ads, chrome, cookie banners, etc.
  const JUNK = /(^|[-_\s])(ads?|advert|sponsor|promo|cookie|consent|banner|newsletter|navbar|nav|menu|sidebar|footer|header|breadcrumb|pagination|comment|share|social|toolbar)([-_\s]|$)/i;

  function isVisible(el) {
    if (!el || !el.getBoundingClientRect) return false;
    const cs = window.getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden" || Number(cs.opacity) === 0) {
      return false;
    }
    if (el.offsetParent === null && cs.position !== "fixed" && cs.position !== "sticky") {
      return false;
    }
    return true;
  }

  function isSkippable(el) {
    let node = el;
    let depth = 0;
    while (node && node !== document.documentElement && depth < 14) {
      const tag = node.tagName;
      if (tag && (SKIP_TAGS.has(tag) || NAV_TAGS.has(tag))) return true;
      if (node.getAttribute) {
        if (node.isContentEditable) return true; // don't capture what's being typed
        if (node.getAttribute("aria-hidden") === "true") return true;
        const role = node.getAttribute("role");
        if (role && NAV_ROLES.has(role)) return true;
        if (node.closest && node.tagName === "FORM") return true;
        const cls = typeof node.className === "string" ? node.className : node.getAttribute("class") || "";
        const id = node.id || "";
        if (JUNK.test(cls) || JUNK.test(id)) return true;
      }
      node = node.parentElement;
      depth++;
    }
    return false;
  }

  function redact(text) {
    return text
      // card / long account numbers
      .replace(/\b(?:\d[ -]?){13,19}\b/g, "[redacted-number]")
      // SSN-like
      .replace(/\b\d{3}-\d{2}-\d{4}\b/g, "[redacted]");
  }

  function extractReadable(maxLength) {
    const root =
      document.querySelector("article") ||
      document.querySelector("main") ||
      document.querySelector('[role="main"]') ||
      document.body;

    // Headings (visible, not in nav/ads).
    const headings = [];
    for (const h of document.querySelectorAll("h1, h2, h3")) {
      if (headings.length >= 8) break;
      if (isSkippable(h) || !isVisible(h)) continue;
      const t = clean(h.innerText);
      if (t && t.length <= 200) headings.push(t);
    }

    // Body text via a text-node walk so div-based content (chat apps, SPAs)
    // is captured too — not just <p>.
    const blocks = [];
    const seen = new Set();
    let total = 0;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        const t = node.nodeValue ? node.nodeValue.trim() : "";
        if (t.length < 2) return NodeFilter.FILTER_REJECT;
        const el = node.parentElement;
        if (!el || isSkippable(el) || !isVisible(el)) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    });

    while (walker.nextNode()) {
      if (total >= maxLength) break;
      const t = clean(walker.currentNode.nodeValue);
      if (!t || t.length < 2) continue;
      const key = t.slice(0, 80).toLowerCase();
      if (seen.has(key)) continue; // drop repeated menu/label text
      seen.add(key);
      blocks.push(t);
      total += t.length + 1;
    }

    let text = redact(blocks.join("\n"));
    if (text.length > maxLength) text = text.slice(0, maxLength);

    return {
      title: document.title || "",
      url: location.href,
      headings,
      text,
      flags: {
        hasArticle: !!document.querySelector("article"),
        hasVideo: !!document.querySelector("video"),
      },
    };
  }

  function clean(s) {
    return (s || "").replace(/\s+/g, " ").trim();
  }
})();
