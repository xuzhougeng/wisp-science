// Lazy highlight.js + KaTeX post-processing for rendered markdown.
let loading;
let katexLoading;

function ensure() {
  if (window.hljs) return Promise.resolve(window.hljs);
  if (!loading) {
    loading = new Promise((resolve, reject) => {
      const css = document.createElement("link");
      css.rel = "stylesheet";
      css.href = "/vendor-runtime/highlight-github.min.css";
      document.head.appendChild(css);
      const s = document.createElement("script");
      s.src = "/vendor-runtime/highlight.min.js";
      s.onload = () => resolve(window.hljs);
      s.onerror = reject;
      document.head.appendChild(s);
    });
  }
  return loading;
}

function ensureKatex() {
  if (!katexLoading) {
    const css = document.createElement("link");
    css.rel = "stylesheet";
    css.href = "/vendor-runtime/katex-DwwF5kvc.css";
    document.head.appendChild(css);
    katexLoading = import("/vendor-runtime/katex-Dn761jRB.js").then((m) => m.k);
  }
  return katexLoading;
}

/** @param {ParentNode} root */
export async function highlight_root(root) {
  // Re-rendered blocks are fresh DOM nodes without the marker, so content
  // changes still re-render; untouched siblings are skipped.
  const math = root.querySelectorAll(".math:not([data-math])");
  if (math.length) {
    const katex = await ensureKatex();
    math.forEach((node) => {
      const tex = node.textContent;
      node.dataset.math = "1";
      try {
        katex.render(tex, node, {
          displayMode: node.classList.contains("math-display"),
          throwOnError: false,
        });
      } catch {
        node.textContent = tex;
      }
    });
  }
  const code = root.querySelectorAll("pre code:not([data-hl])");
  if (!code.length) return;
  const hljs = await ensure();
  code.forEach((node) => {
    hljs.highlightElement(node);
    node.dataset.hl = "1";
  });
}

/** @param {string} id */
export async function highlight_by_id(id) {
  const root = document.getElementById(id);
  if (root) await highlight_root(root);
}

/** Wait until a just-mounted node is queryable, or give up after a few frames. */
async function waitForId(id) {
  let node = document.getElementById(id);
  for (let i = 0; !node && i < 8; i++) {
    await new Promise((resolve) => requestAnimationFrame(resolve));
    node = document.getElementById(id);
  }
  return node;
}

/** Replace one editable code node's text and re-highlight it. The editor owns
 * this node's children (not the reactive renderer), so swapping textContent
 * and clearing the markers is safe on every keystroke.
 * @param {string} id @param {string} text */
export async function highlight_set_code(id, text) {
  const node = await waitForId(id);
  if (!node) return;
  node.textContent = text;
  delete node.dataset.hl;
  delete node.dataset.highlighted;
  const hljs = await ensure();
  hljs.highlightElement(node);
  node.dataset.hl = "1";
}
