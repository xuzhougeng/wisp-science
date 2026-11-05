import { render_share_png } from './share-renderer.js';

// The host supplies sanitized HTML and the frozen selection over JSON. Never
// mount that whole document, execute its scripts or fetch referenced resources.
export async function renderNativeShare(payload) {
  document.documentElement.lang = payload.locale;
  document.documentElement.style.colorScheme = payload.dark ? 'dark' : 'light';
  for (const [name, value] of Object.entries(payload.variables))
    document.documentElement.style.setProperty(name, value);
  const parsed = new DOMParser().parseFromString(payload.html, 'text/html');
  const articles = [...parsed.querySelectorAll('.thread > .msg')];
  if (articles.length !== payload.rows.length) throw new Error('Share document does not match the selected messages');
  if (payload.format === 'png') {
    // External images cannot be loaded by this local renderer. Keep their alt
    // text in PNG; HTML preserves the existing exported image references.
    for (const image of parsed.querySelectorAll('img')) {
      const alt = parsed.createElement('span'); alt.textContent = image.alt; image.replaceWith(alt);
    }
    await document.fonts.load('15px Inter');
    await document.fonts.load('15px "Source Serif 4"');
    await document.fonts.load('12px "JetBrains Mono"');
    return render_share_png(JSON.stringify({ title: payload.title, subtitle: payload.stamp, footer: payload.footer,
      width: payload.width, messages: payload.rows.map((row, i) => ({
        kind: row.role === 'reasoning' ? 'thinking' : row.role,
        label: payload.labels[row.role], text: row.text,
        html: row.role === 'assistant' ? articles[i].querySelector('.body.md')?.innerHTML : undefined,
      })) }));
  }
  parsed.documentElement.lang = payload.locale;
  parsed.documentElement.style.cssText = document.documentElement.style.cssText;
  parsed.querySelector('.share-head h1').textContent = payload.title;
  const date = parsed.createElement('p'); date.className = 'share-date'; date.textContent = payload.stamp;
  parsed.querySelector('.share-head').append(date);
  parsed.querySelector('.share-foot').textContent = payload.footer;
  articles.forEach((article, i) => { article.querySelector('.role').textContent = payload.labels[payload.rows[i].role]; });
  return '<!doctype html>\n' + parsed.documentElement.outerHTML;
}

// A fresh native WebView instance processes exactly one export.
if (window.chrome?.webview) {
  let started = false;
  window.chrome.webview.addEventListener('message', async ({ data }) => {
    if (started) return;
    started = true;
    try { window.chrome.webview.postMessage({ type: 'done', output: await renderNativeShare(data) }); }
    catch (error) { window.chrome.webview.postMessage({ type: 'error', error: String(error?.message || error) }); }
  });
  window.chrome.webview.postMessage({ type: 'ready' });
}
