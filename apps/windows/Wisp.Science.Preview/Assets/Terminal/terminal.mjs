import { Terminal } from './xterm.mjs';
import { FitAddon } from './xterm-addon-fit.mjs';
let version = -1, enabled = false, timer;
let terminal, fit;
const send = message => window.chrome.webview.postMessage({ ...message, version });
function reset() {
    const options = terminal ? { ...terminal.options } : { scrollback: 5000, screenReaderMode: true, disableStdin: true };
    terminal?.dispose();
    terminal = new Terminal(options);
    fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(document.getElementById('terminal'));
    terminal.onData(data => { if (enabled) send({ type: 'input', data }); });
    terminal.onBinary(data => { if (enabled) send({ type: 'binary', data: btoa(data) }); });
    terminal.onResize(({ rows, cols }) => send({ type: 'resize', rows, cols }));
    fit.fit();
}
reset();
window.chrome.webview.addEventListener('message', ({ data }) => {
    if (data.type === 'state') {
        if (version !== data.version) { version = data.version; reset(); }
        enabled = data.enabled;
        terminal.options.disableStdin = !enabled;
        terminal.options.fontSize = data.fontSize;
        terminal.options.fontFamily = data.fontFamily;
        terminal.options.theme = { background: data.background, foreground: data.foreground, cursor: data.foreground };
        document.body.style.background = data.background;
        fit.fit();
        // A terminal switch needs its dimensions even if the viewport did not change.
        send({ type: 'resize', rows: terminal.rows, cols: terminal.cols });
    } else if (data.type === 'output' && data.version === version) {
        if (data.reset) reset();
        terminal.write(Uint8Array.from(atob(data.base64), c => c.charCodeAt(0)));
    }
});
new ResizeObserver(() => { clearTimeout(timer); timer = setTimeout(() => fit.fit(), 80); }).observe(document.body);
window.addEventListener('pagehide', () => { clearTimeout(timer); terminal.dispose(); });
send({ type: 'ready' });
