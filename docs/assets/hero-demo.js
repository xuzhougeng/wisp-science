// Hero demo: a scripted Wisp session that plays in the homepage mock as soon
// as the page opens. The prompt types into the composer, the agent's steps
// run one after another with their timings, artifacts land in the right
// panel and the answer streams in; then the next conversation starts. The
// sidebar switches conversations and every artifact opens a preview. Clicking
// anything stops the automatic rotation so the visitor can explore. With
// reduced motion each conversation shows its finished state; without this
// script the static mock in index.html stays as it is.
(() => {
  const visual = document.querySelector('[data-hero-demo]');
  const mock = visual?.querySelector('.hero-mock');
  if (!mock) return;
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');

  const UI = {
    zh: {
      region: 'Wisp Science 演示：一次科研对话的运行过程',
      convos: '示例对话',
      newChat: '新对话',
      settings: '设置',
      artifacts: 'Artifacts',
      empty: '运行中生成的表格、代码和图会出现在这里。',
      placeholder: '向 Wisp 提问…',
      running: '运行中',
      done: '已完成',
      outputs: (n) => `${n} 个产物`,
      pause: '暂停',
      play: '继续',
      close: '关闭预览',
      stream: '事件流',
      think: '思考',
      verify: '核对',
      artifact: '产物',
      rows: (n) => `另有 ${n} 行`,
    },
    en: {
      region: 'Wisp Science demo: a research conversation as it runs',
      convos: 'Example conversations',
      newChat: 'New chat',
      settings: 'Settings',
      artifacts: 'Artifacts',
      empty: 'Tables, code and figures appear here as the run produces them.',
      placeholder: 'Ask Wisp…',
      running: 'Running',
      done: 'Done',
      outputs: (n) => `${n} artifacts`,
      pause: 'Pause',
      play: 'Resume',
      close: 'Close preview',
      stream: 'Event stream',
      think: 'think',
      verify: 'verify',
      artifact: 'artifact',
      rows: (n) => `${n} more rows`,
    },
  };

  const ICON = {
    plus: '<path d="M12 5v14M5 12h14"/>',
    send: '<path d="M12 19V5M5 12l7-7 7 7"/>',
    pause: '<path d="M9 5v14M15 5v14"/>',
    play: '<path d="M7 5l12 7-12 7z"/>',
    close: '<path d="M6 6l12 12M18 6L6 18"/>',
    table: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 10h18M9 10v10"/>',
    code: '<path d="M8 8l-4 4 4 4M16 8l4 4-4 4M13.5 5l-3 14"/>',
    figure: '<path d="M4 4v16h16"/><path d="M8 15l3-4 3 2 4-6"/>',
    equation: '<path d="M18 5H7l6 7-6 7h11"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  };
  const icon = (name) => `<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">${ICON[name]}</svg>`;

  // Small inline figures, drawn the way the run's plotting code would.
  const flowFigure = () => {
    const steps = ['sgRNA library', 'Lentiviral transduction', 'Selection / sorting', 'NGS of guides', 'Hit calling (MAGeCK)'];
    const boxes = steps.map((label, i) => {
      const y = 6 + i * 46;
      const arrow = i < steps.length - 1
        ? `<path class="fig-arrow" d="M120 ${y + 30}v12M115 ${y + 37}l5 5 5-5"/>` : '';
      return `<rect class="fig-box${i === steps.length - 1 ? ' is-key' : ''}" x="20" y="${y}" width="200" height="30" rx="8"/>`
        + `<text x="120" y="${y + 19.5}" text-anchor="middle">${label}</text>${arrow}`;
    }).join('');
    return `<svg class="mock-figure" viewBox="0 0 240 236" role="img" aria-label="CRISPR screen workflow">${boxes}</svg>`;
  };
  const violin = (cx, top, bottom, profile) => {
    const step = (bottom - top) / (profile.length - 1);
    const right = profile.map((w, i) => `${(cx + w * 22).toFixed(1)} ${(top + i * step).toFixed(1)}`);
    const left = profile.map((w, i) => `${(cx - w * 22).toFixed(1)} ${(top + i * step).toFixed(1)}`).reverse();
    return `M${right.join('L')}L${left.join('L')}Z`;
  };
  const violinFigure = () => {
    const panels = [
      { name: 'n_genes', profile: [0.02, 0.05, 0.12, 0.35, 0.8, 1, 0.7, 0.3, 0.08, 0.02], cut: 0.17, label: '2,500' },
      { name: 'total_counts', profile: [0.02, 0.04, 0.1, 0.3, 0.75, 1, 0.65, 0.25, 0.06, 0.02], cut: null },
      { name: 'pct_mt', profile: [0.01, 0.03, 0.06, 0.12, 0.3, 0.7, 1, 0.8, 0.35, 0.05], cut: 0.42, label: '5%' },
    ];
    const body = panels.map((p, i) => {
      const cx = 44 + i * 76;
      const cut = p.cut == null ? '' : `<path class="fig-cut" d="M${cx - 30} ${30 + p.cut * 150}h60"/>`
        + `<text class="fig-cut-label" x="${cx - 30}" y="${26 + p.cut * 150}">${p.label}</text>`;
      return `<path class="fig-violin" d="${violin(cx, 30, 180, p.profile)}"/>${cut}`
        + `<text x="${cx}" y="200" text-anchor="middle">${p.name}</text>`;
    }).join('');
    return `<svg class="mock-figure" viewBox="0 0 240 210" role="img" aria-label="PBMC 3k QC violins">${body}</svg>`;
  };
  const mmEquation = () => '<div class="mock-eq"><i>v</i> =<span class="mock-frac"><span><i>V</i><sub>max</sub>&thinsp;[S]</span>'
    + '<span><i>K</i><sub>m</sub> + [S]</span></span></div>';

  // Text fields are { zh, en }; tool names and code stay as they are.
  const SCENARIOS = [
    {
      id: 'crispr',
      nav: { zh: 'CRISPR 筛选', en: 'CRISPR screen' },
      prompt: {
        zh: '检索 PubMed 上 CRISPR 筛选的最新方法，并画一张流程图。',
        en: 'Search PubMed for recent CRISPR screen methods and draft a flowchart.',
      },
      steps: [
        { kind: 'think', ms: 900, text: { zh: '拆分任务：检索 → 筛选 → 汇总 → 绘图', en: 'Plan: search → screen → summarize → draw' } },
        { kind: 'tool', tag: 'pubmed.search', ms: 1700, text: { zh: '“CRISPR screen” 2024–2026 · 48 条结果', en: '“CRISPR screen” 2024–2026 · 48 hits' } },
        { kind: 'tool', tag: 'python', ms: 1600, text: { zh: 'screen_papers.py · 按方法与样本量保留 12 篇', en: 'screen_papers.py · kept 12 by method and sample size' } },
        { kind: 'artifact', ref: 'table' },
        { kind: 'tool', tag: 'python', ms: 1500, text: { zh: 'figure_flow.py · 用 matplotlib 绘制流程图', en: 'figure_flow.py · flowchart in matplotlib' } },
        { kind: 'artifact', ref: 'code' },
        { kind: 'artifact', ref: 'figure' },
        { kind: 'verify', ms: 900, text: { zh: '12 条引用均可回溯到原文', en: 'All 12 citations trace back to their papers' } },
      ],
      answer: {
        zh: '已检索 12 篇候选文献，并整理为带来源的报告。Perturb-seq 与碱基编辑筛选是近两年的主要方向。绘图代码与流程图已保存到项目中。',
        en: 'Found 12 candidate papers and summarized them with sources. Perturb-seq and base-editor screens lead the last two years. The plotting code and flowchart are saved to the project.',
      },
      artifacts: {
        table: {
          icon: 'table', title: 'Table 1',
          meta: { zh: '12 行 · 文献摘要', en: '12 rows · literature summary' },
          table: {
            head: { zh: ['方法', '编辑方式', '读出'], en: ['Method', 'Editing', 'Readout'] },
            rows: [
              ['Perturb-seq', 'CRISPRi', 'scRNA-seq'],
              ['CROP-seq', 'Cas9 KO', 'scRNA-seq'],
              ['Base-editor screen', 'ABE / CBE', 'Variant effect'],
              ['Optical pooled screen', 'Cas9 KO', 'Imaging'],
            ],
            more: 8,
          },
        },
        code: {
          icon: 'code', title: 'figure_flow.py',
          meta: { zh: 'matplotlib · 可复现', en: 'matplotlib · reproducible' },
          code: `import matplotlib.pyplot as plt

steps = ["sgRNA library", "Lentiviral transduction",
         "Selection / sorting", "NGS of guides",
         "Hit calling (MAGeCK)"]
fig, ax = plt.subplots(figsize=(3, 4))
for i, label in enumerate(steps):
    ax.annotate(label, (0.5, 1 - i / 4), ha="center",
                bbox=dict(boxstyle="round", fc="#e6f4f1"))
ax.axis("off")
fig.savefig("figures/figure_flow.svg")`,
        },
        figure: {
          icon: 'figure', title: 'Figure 1',
          meta: { zh: 'SVG · 筛选流程图', en: 'SVG · screen workflow' },
          figure: flowFigure,
        },
      },
    },
    {
      id: 'enzyme',
      nav: { zh: '酶动力学', en: 'Enzyme kinetics' },
      prompt: {
        zh: '拟合 96 孔板中各突变体的 Michaelis–Menten 动力学，比较 kcat/Km。',
        en: 'Fit Michaelis–Menten kinetics for each mutant on the 96-well plate and compare kcat/Km.',
      },
      steps: [
        { kind: 'tool', tag: 'read_file', ms: 800, text: { zh: 'plate_reads.csv · 4 个突变体 × 8 个底物浓度', en: 'plate_reads.csv · 4 mutants × 8 substrate levels' } },
        { kind: 'think', ms: 1000, text: { zh: '初速度对 [S] 做非线性最小二乘拟合', en: 'Nonlinear least squares of initial rate against [S]' } },
        { kind: 'artifact', ref: 'equation' },
        { kind: 'tool', tag: 'python', ms: 1900, text: { zh: 'kinetics_fit.py · scipy curve_fit 逐个拟合', en: 'kinetics_fit.py · scipy curve_fit per mutant' } },
        { kind: 'artifact', ref: 'code' },
        { kind: 'verify', ms: 1100, text: { zh: '残差无系统偏差，R² ≥ 0.98', en: 'Residuals show no trend, R² ≥ 0.98' } },
        { kind: 'artifact', ref: 'table' },
      ],
      answer: {
        zh: 'S2 突变体的 kcat/Km 是野生型的 3.1 倍，提升主要来自 Km 下降。所有拟合 R² ≥ 0.98，方程、参数表与代码已保存。',
        en: 'Mutant S2 reaches 3.1× the wild-type kcat/Km, mostly from a lower Km. Every fit has R² ≥ 0.98; the equation, parameter table and code are saved.',
      },
      artifacts: {
        equation: {
          icon: 'equation', title: 'Equation 1',
          meta: { zh: 'LaTeX · KaTeX 预览', en: 'LaTeX · KaTeX preview' },
          equation: mmEquation,
          note: { zh: 'Michaelis–Menten 方程，kcat = Vmax / [E]。', en: 'Michaelis–Menten, with kcat = Vmax / [E].' },
        },
        code: {
          icon: 'code', title: 'kinetics_fit.py',
          meta: { zh: 'scipy · 可复现', en: 'scipy · reproducible' },
          code: `import pandas as pd
from scipy.optimize import curve_fit

def mm(s, vmax, km):
    return vmax * s / (km + s)

reads = pd.read_csv("data/plate_reads.csv")
for mutant, d in reads.groupby("mutant"):
    (vmax, km), _ = curve_fit(mm, d.substrate_uM, d.rate)
    kcat = vmax / d.enzyme_uM.iloc[0]
    print(mutant, round(kcat, 1), round(km), kcat / km)`,
        },
        table: {
          icon: 'table', title: 'Table 2',
          meta: { zh: '4 个突变体 · kcat/Km', en: '4 mutants · kcat/Km' },
          table: {
            head: { zh: ['突变体', 'kcat (s⁻¹)', 'Km (µM)', '相对 kcat/Km'], en: ['Mutant', 'kcat (s⁻¹)', 'Km (µM)', 'Rel. kcat/Km'] },
            rows: [
              ['WT', '12.4', '85', '1.0×'],
              ['S2', '13.1', '29', '3.1×'],
              ['S7', '18.9', '71', '1.8×'],
              ['L4', '6.2', '70', '0.6×'],
            ],
          },
        },
      },
    },
    {
      id: 'pbmc',
      nav: { zh: 'PBMC 3k 质控', en: 'PBMC 3k QC' },
      prompt: {
        zh: '检查 PBMC 3k 的数据质量，并给出过滤阈值。',
        en: 'Check PBMC 3k data quality and suggest filtering thresholds.',
      },
      steps: [
        { kind: 'tool', tag: 'read_file', ms: 900, text: { zh: '10x 矩阵 · 2,700 个细胞 × 32,738 个基因', en: '10x matrix · 2,700 cells × 32,738 genes' } },
        { kind: 'tool', tag: 'python', ms: 1700, text: { zh: 'qc_metrics.py · scanpy 计算基因数与线粒体比例', en: 'qc_metrics.py · scanpy gene counts and mito fraction' } },
        { kind: 'artifact', ref: 'code' },
        { kind: 'artifact', ref: 'figure' },
        { kind: 'think', ms: 1000, text: { zh: '按分布拐点设定阈值', en: 'Set cut-offs at the distribution elbows' } },
        { kind: 'verify', ms: 1000, text: { zh: '过滤后保留 2,638 个细胞（97.7%）', en: '2,638 cells kept after filtering (97.7%)' } },
        { kind: 'artifact', ref: 'table' },
      ],
      answer: {
        zh: '建议保留表达 200–2,500 个基因、线粒体比例 < 5% 的细胞，共 2,638 个。小提琴图和阈值表已保存，可以直接进入归一化。',
        en: 'Keep cells with 200–2,500 genes and under 5% mitochondrial reads: 2,638 cells. The violin plot and threshold table are saved, ready for normalization.',
      },
      artifacts: {
        code: {
          icon: 'code', title: 'qc_metrics.py',
          meta: { zh: 'scanpy · 可复现', en: 'scanpy · reproducible' },
          code: `import scanpy as sc

adata = sc.read_10x_mtx("data/filtered_gene_bc_matrices/hg19")
adata.var["mt"] = adata.var_names.str.startswith("MT-")
sc.pp.calculate_qc_metrics(adata, qc_vars=["mt"], inplace=True)
sc.pl.violin(adata, ["n_genes_by_counts", "total_counts",
             "pct_counts_mt"], save="_qc.png")
keep = (adata.obs.n_genes_by_counts.between(200, 2500)
        & (adata.obs.pct_counts_mt < 5))
print(keep.sum())  # 2638`,
        },
        figure: {
          icon: 'figure', title: 'Figure 2',
          meta: { zh: 'PNG · QC 小提琴图', en: 'PNG · QC violins' },
          figure: violinFigure,
        },
        table: {
          icon: 'table', title: 'Table 3',
          meta: { zh: '过滤阈值 · 2,638 个细胞', en: 'Thresholds · 2,638 cells' },
          table: {
            head: { zh: ['指标', '阈值'], en: ['Metric', 'Threshold'] },
            rows: [
              ['n_genes_by_counts', '200 – 2,500'],
              ['pct_counts_mt', '< 5%'],
              ['genes (min cells)', '≥ 3 → 13,714'],
              ['cells kept', '2,638 / 2,700'],
            ],
          },
        },
      },
    },
  ];

  const START_MS = 1800;
  const TYPE_MS = 32;
  const HOLD_MS = 6500;
  const TICK_MS = 40;

  const lang = () => (document.documentElement.dataset.lang === 'en' ? 'en' : 'zh');
  const say = (key, ...args) => {
    const value = UI[lang()][key];
    return typeof value === 'function' ? value(...args) : value;
  };
  const text = (field) => (typeof field === 'string' ? field : field[lang()]);
  const secs = (ms) => `${(ms / 1000).toFixed(1)} s`;
  const esc = (value) => value.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  const el = (tag, className, html) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (html != null) node.innerHTML = html;
    return node;
  };

  // ---- Frame -------------------------------------------------------------
  mock.replaceChildren();
  mock.classList.add('is-live');
  visual.classList.add('is-live');
  visual.removeAttribute('aria-hidden');
  visual.setAttribute('role', 'region');

  const sidebar = el('aside', 'mock-sidebar');
  const logo = el('div', 'mock-logo', '<img src="assets/logo.svg" alt="">wisp-science');
  const newChat = el('button', 'mock-new');
  newChat.type = 'button';
  const convos = el('nav', 'mock-convos');
  const navButtons = SCENARIOS.map((scenario) => {
    const button = el('button', 'mock-nav-item');
    button.type = 'button';
    button.dataset.id = scenario.id;
    convos.append(button);
    return button;
  });
  const settings = el('div', 'mock-nav-item is-static');
  sidebar.append(logo, newChat, convos, settings);

  const chat = el('section', 'mock-chat');
  const bar = el('header', 'mock-chat-bar');
  const title = el('span', 'mock-chat-title');
  const status = el('span', 'mock-status');
  const toggle = el('button', 'mock-toggle');
  toggle.type = 'button';
  bar.append(title, status, toggle);
  const stream = el('ol', 'mock-stream');
  const composer = el('div', 'mock-composer');
  const input = el('span', 'mock-input');
  const send = el('span', 'mock-send', icon('send'));
  composer.append(input, send);
  chat.append(bar, stream, composer);

  const shelf = el('aside', 'mock-artifacts');
  const shelfTitle = el('h4');
  const shelfList = el('ul', 'mock-artifact-list');
  const shelfEmpty = el('p', 'mock-empty');
  shelf.append(shelfTitle, shelfList, shelfEmpty);

  const preview = el('div', 'mock-preview');
  preview.hidden = true;
  mock.append(sidebar, chat, shelf, preview);

  // ---- Clock -------------------------------------------------------------
  // Steps wait on a clock that only advances while the demo is on screen, the
  // tab is visible and the visitor has not paused, so nothing is missed.
  const CANCELLED = Symbol('cancelled');
  let clock = 0;
  let waiters = [];
  let onScreen = false;
  let paused = false;
  const live = () => onScreen && !paused && !document.hidden;
  const wait = (ms) => new Promise((resolve, reject) => waiters.push({ at: clock + ms, resolve, reject }));
  const cancelWaits = () => {
    const old = waiters;
    waiters = [];
    old.forEach((w) => w.reject(CANCELLED));
  };
  const listeners = new Set();
  setInterval(() => {
    if (!live()) return;
    clock += TICK_MS;
    const due = waiters.filter((w) => w.at <= clock);
    if (due.length) {
      waiters = waiters.filter((w) => w.at > clock);
      due.forEach((w) => w.resolve());
    }
    listeners.forEach((fn) => fn());
  }, TICK_MS);

  // ---- State -------------------------------------------------------------
  let current = 0;
  let runId = 0;
  let autoCycle = true;
  let finished = false;
  let produced = [];
  let firstRun = true;
  let stick = true;
  let openRef = null;
  let opener = null;

  stream.addEventListener('scroll', () => {
    stick = stream.scrollHeight - stream.scrollTop - stream.clientHeight < 24;
  });
  const follow = () => {
    if (stick) stream.scrollTop = stream.scrollHeight;
  };

  const renderChrome = () => {
    const scenario = SCENARIOS[current];
    visual.setAttribute('aria-label', say('region'));
    convos.setAttribute('aria-label', say('convos'));
    stream.setAttribute('aria-label', say('stream'));
    newChat.innerHTML = `${icon('plus')}<span>${say('newChat')}</span>`;
    settings.textContent = say('settings');
    navButtons.forEach((button, i) => {
      button.textContent = text(SCENARIOS[i].nav);
      button.setAttribute('aria-pressed', String(i === current));
    });
    title.textContent = text(scenario.nav);
    shelfTitle.textContent = say('artifacts');
    shelfEmpty.textContent = say('empty');
    toggle.hidden = reduced.matches;
    toggle.innerHTML = `${icon(paused ? 'play' : 'pause')}<span>${say(paused ? 'play' : 'pause')}</span>`;
    toggle.setAttribute('aria-pressed', String(paused));
  };

  const renderStatus = (elapsed) => {
    status.dataset.state = finished ? 'done' : 'running';
    status.textContent = finished
      ? `${say('done')} · ${secs(elapsed)} · ${say('outputs', produced.length)}`
      : `${say('running')} · ${secs(elapsed)}`;
  };

  const setInput = (value) => {
    input.textContent = value || say('placeholder');
    input.classList.toggle('is-empty', !value);
    composer.classList.toggle('is-typing', Boolean(value));
  };

  const artifactLabel = (art) => `${icon(art.icon)}<span><strong>${art.title}</strong><small>${text(art.meta)}</small></span>`;

  const addArtifact = (ref, animate) => {
    const art = SCENARIOS[current].artifacts[ref];
    produced.push(ref);
    const item = el('li');
    const button = el('button', 'mock-chip', artifactLabel(art));
    button.type = 'button';
    button.dataset.artifact = ref;
    if (animate) item.classList.add('mock-in');
    item.append(button);
    shelfList.append(item);
    shelfEmpty.hidden = true;
  };

  const stepRow = (step, at, animate) => {
    const row = el('li', 'mock-step');
    row.dataset.kind = step.kind;
    if (animate) row.classList.add('mock-in');
    const time = el('span', 'mock-step-time', secs(at));
    const dot = el('span', 'mock-step-dot');
    const body = el('span', 'mock-step-body');
    if (step.kind === 'artifact') {
      const art = SCENARIOS[current].artifacts[step.ref];
      const link = el('button', 'mock-step-link', art.title);
      link.type = 'button';
      link.dataset.artifact = step.ref;
      body.append(el('b', 'mock-step-tag', say('artifact')), link, document.createTextNode(` · ${text(art.meta)}`));
      row.dataset.artifact = step.ref;
    } else {
      const tag = step.tag || say(step.kind);
      body.append(el('b', 'mock-step-tag'), document.createTextNode(text(step.text)));
      body.firstChild.textContent = tag;
    }
    const dur = el('span', 'mock-step-dur');
    row.append(time, dot, body, dur);
    stream.append(row);
    return row;
  };

  const finishRow = (row, ms) => {
    row.dataset.state = 'done';
    row.querySelector('.mock-step-dur').innerHTML = ms ? `${icon('check')}${secs(ms)}` : icon('check');
  };

  const bubble = (who, value, animate) => {
    const item = el('li', `mock-msg ${who}`);
    if (animate) item.classList.add('mock-in');
    const box = el('div', `mock-bubble ${who}`);
    box.textContent = value;
    item.append(box);
    stream.append(item);
    return box;
  };

  const reset = () => {
    cancelWaits();
    closePreview(false);
    runId += 1;
    finished = false;
    produced = [];
    stick = true;
    stream.replaceChildren();
    shelfList.replaceChildren();
    shelfEmpty.hidden = false;
    send.classList.remove('is-sent');
    setInput('');
    renderChrome();
  };

  // Finished state, used for reduced motion and after a language switch.
  const renderFinal = () => {
    reset();
    const scenario = SCENARIOS[current];
    bubble('user', text(scenario.prompt), false);
    let at = 400;
    scenario.steps.forEach((step) => {
      const row = stepRow(step, at, false);
      finishRow(row, step.ms);
      if (step.kind === 'artifact') addArtifact(step.ref, false);
      at += (step.ms || 300) + 200;
    });
    bubble('assistant', text(scenario.answer), false);
    finished = true;
    renderStatus(at);
    stream.scrollTop = 0;
  };

  const play = async () => {
    reset();
    const id = runId;
    const scenario = SCENARIOS[current];
    const guard = () => {
      if (id !== runId) throw CANCELLED;
    };
    let started = 0;
    const tick = () => renderStatus(clock - started);
    status.dataset.state = 'idle';
    status.textContent = '';
    try {
      if (firstRun) await wait(START_MS);
      firstRun = false;
      guard();
      const prompt = text(scenario.prompt);
      for (let i = 1; i <= prompt.length; i += 1) {
        setInput(prompt.slice(0, i));
        await wait(TYPE_MS);
        guard();
      }
      await wait(350);
      guard();
      send.classList.add('is-sent');
      setInput('');
      bubble('user', prompt, true);
      follow();
      started = clock;
      listeners.add(tick);
      await wait(400);
      guard();
      send.classList.remove('is-sent');
      for (const step of scenario.steps) {
        const row = stepRow(step, clock - started, true);
        follow();
        if (step.kind === 'artifact') {
          addArtifact(step.ref, true);
          finishRow(row, 0);
          await wait(450);
        } else {
          row.dataset.state = 'running';
          await wait(step.ms);
          guard();
          finishRow(row, step.ms);
          await wait(220);
        }
        guard();
      }
      const answer = text(scenario.answer);
      const box = bubble('assistant', '', true);
      box.classList.add('is-streaming');
      const chunk = lang() === 'en' ? 4 : 2;
      for (let i = chunk; i < answer.length + chunk; i += chunk) {
        box.textContent = answer.slice(0, i);
        follow();
        await wait(TICK_MS);
        guard();
      }
      box.classList.remove('is-streaming');
      finished = true;
      listeners.delete(tick);
      renderStatus(clock - started);
      if (!autoCycle) return;
      await wait(HOLD_MS);
      guard();
      if (autoCycle && openRef === null) show((current + 1) % SCENARIOS.length);
    } catch (error) {
      listeners.delete(tick);
      if (error !== CANCELLED) throw error;
    }
  };

  const show = (index) => {
    current = index;
    if (reduced.matches) renderFinal();
    else play();
  };

  // ---- Preview -------------------------------------------------------------
  const renderPreviewBody = (art) => {
    if (art.table) {
      const { head, rows, more } = art.table;
      const th = text(head).map((cell) => `<th>${esc(cell)}</th>`).join('');
      const tr = rows.map((row) => `<tr>${row.map((cell) => `<td>${esc(cell)}</td>`).join('')}</tr>`).join('');
      const tail = more ? `<p class="mock-preview-note">${say('rows', more)}</p>` : '';
      return `<table class="mock-table"><thead><tr>${th}</tr></thead><tbody>${tr}</tbody></table>${tail}`;
    }
    if (art.code) {
      const lines = art.code.split('\n').map((line) => {
        const safe = esc(line);
        const hash = safe.indexOf('#');
        return hash >= 0
          ? `${safe.slice(0, hash)}<span class="mock-code-comment">${safe.slice(hash)}</span>`
          : safe;
      });
      return `<pre class="mock-code"><code>${lines.join('\n')}</code></pre>`;
    }
    if (art.figure) return art.figure();
    if (art.equation) return `${art.equation()}<p class="mock-preview-note">${text(art.note)}</p>`;
    return '';
  };

  const onPreviewKey = (event) => {
    if (event.key !== 'Escape' || preview.hidden) return;
    event.preventDefault();
    closePreview(true);
  };

  const openPreview = (ref, from) => {
    const art = SCENARIOS[current].artifacts[ref];
    if (!art) return;
    openRef = ref;
    opener = from || null;
    preview.innerHTML = `<header class="mock-preview-head">${icon(art.icon)}`
      + `<span><strong>${art.title}</strong><small>${text(art.meta)}</small></span>`
      + `<button type="button" class="mock-preview-close" aria-label="${say('close')}">${icon('close')}</button></header>`
      + `<div class="mock-preview-body">${renderPreviewBody(art)}</div>`;
    preview.hidden = false;
    mock.dataset.preview = ref;
    mock.querySelectorAll('[data-artifact]').forEach((node) => {
      node.classList.toggle('is-linked', node.dataset.artifact === ref);
    });
    const row = stream.querySelector(`.mock-step[data-artifact="${ref}"]`);
    if (row) stream.scrollTop = Math.max(0, row.offsetTop - stream.clientHeight / 2);
    window.addEventListener('keydown', onPreviewKey);
    preview.querySelector('.mock-preview-close').focus({ preventScroll: true });
  };

  function closePreview(restoreFocus) {
    if (preview.hidden) return;
    preview.hidden = true;
    preview.replaceChildren();
    openRef = null;
    delete mock.dataset.preview;
    mock.querySelectorAll('.is-linked').forEach((node) => node.classList.remove('is-linked'));
    window.removeEventListener('keydown', onPreviewKey);
    if (restoreFocus && opener?.isConnected) opener.focus({ preventScroll: true });
    opener = null;
  }

  // ---- Interaction ---------------------------------------------------------
  const takeOver = () => {
    autoCycle = false;
  };
  mock.addEventListener('click', (event) => {
    const target = event.target.closest('button');
    if (!target || !mock.contains(target)) return;
    if (target === toggle) {
      paused = !paused;
      takeOver();
      renderChrome();
      return;
    }
    takeOver();
    if (target === newChat || target.classList.contains('mock-nav-item')) {
      paused = false;
      show(target === newChat ? current : SCENARIOS.findIndex((s) => s.id === target.dataset.id));
      return;
    }
    if (target.classList.contains('mock-preview-close')) {
      closePreview(true);
      return;
    }
    if (target.dataset.artifact) {
      if (openRef === target.dataset.artifact) closePreview(true);
      else openPreview(target.dataset.artifact, target);
    }
  });

  // i18n.js rewrites data-lang on the root (also on load, with the same
  // value); on a real switch, replay the conversation in the new language or
  // redraw its finished state.
  let shownLang = lang();
  new MutationObserver(() => {
    if (lang() === shownLang) return;
    shownLang = lang();
    if (reduced.matches || finished) renderFinal();
    else play();
  }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-lang'] });

  reduced.addEventListener?.('change', () => show(current));

  if ('IntersectionObserver' in window) {
    new IntersectionObserver(([entry]) => {
      onScreen = entry.isIntersecting;
    }, { threshold: 0.25 }).observe(visual);
  } else {
    onScreen = true;
  }

  show(0);
})();
