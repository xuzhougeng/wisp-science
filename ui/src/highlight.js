// Lazy highlight.js for markdown + tool code blocks.
let loading;

function ensure() {
  if (window.hljs) return Promise.resolve(window.hljs);
  if (!loading) {
    loading = new Promise((resolve, reject) => {
      const css = document.createElement("link");
      css.rel = "stylesheet";
      css.href = "/vendor/highlight-github.min.css";
      document.head.appendChild(css);
      const s = document.createElement("script");
      s.src = "/vendor/highlight.min.js";
      s.onload = () => resolve(window.hljs);
      s.onerror = reject;
      document.head.appendChild(s);
    });
  }
  return loading;
}

/** @param {ParentNode} root */
async function highlight_root(root) {
  const hljs = await ensure();
  // Re-rendered blocks are fresh DOM nodes without the marker, so content
  // changes still re-highlight; untouched siblings are skipped.
  root.querySelectorAll("pre code:not([data-hl])").forEach((node) => {
    hljs.highlightElement(node);
    node.dataset.hl = "1";
  });
}

/** @param {string} id */
export async function highlight_by_id(id) {
  const root = document.getElementById(id);
  if (root) await highlight_root(root);
}
