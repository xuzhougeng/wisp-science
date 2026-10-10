import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(dir, "markdown.js"), "utf8");
const root = {};
vm.runInNewContext(source, { self: root, globalThis: root });

// Round-trip through JSON: the tree is built in another realm.
const blocks = (text) => JSON.parse(JSON.stringify(root.mdBlocks(text)));
const inline = (text) => JSON.parse(JSON.stringify(root.mdInline(text)));
const p = (...children) => ({ tag: "p", children });

test("headings, paragraphs and rules", () => {
  assert.deepEqual(blocks("# Title\n\nFirst line\nsecond line\n\n---\n\n### Sub ###"), [
    { tag: "h1", children: ["Title"] },
    p("First line\nsecond line"),
    { tag: "hr", children: [] },
    { tag: "h3", children: ["Sub"] }
  ]);
  assert.deepEqual(blocks("#hashtag is text"), [p("#hashtag is text")]);
});

test("emphasis, code and strike", () => {
  assert.deepEqual(inline("a **bold** and *it* and `x*y` and ~~old~~"), [
    "a ",
    { tag: "strong", children: ["bold"] },
    " and ",
    { tag: "em", children: ["it"] },
    " and ",
    { tag: "code", children: ["x*y"] },
    " and ",
    { tag: "del", children: ["old"] }
  ]);
  assert.deepEqual(inline("***both*** and **outer *inner***"), [
    { tag: "strong", children: [{ tag: "em", children: ["both"] }] },
    " and ",
    { tag: "strong", children: ["outer ", { tag: "em", children: ["inner"] }] }
  ]);
});

test("identifiers, arithmetic and unclosed markers stay literal", () => {
  assert.deepEqual(inline("read_count_matrix and 2 * 3 * 4"), ["read_count_matrix and 2 * 3 * 4"]);
  assert.deepEqual(inline("still **streaming"), ["still **streaming"]);
  assert.deepEqual(inline("escaped \\*star\\*"), ["escaped *star*"]);
});

test("only http(s) and mailto become links", () => {
  assert.deepEqual(inline("see [the paper](https://example.com/a) or https://example.org/b."), [
    "see ",
    { tag: "a", href: "https://example.com/a", children: ["the paper"] },
    " or ",
    { tag: "a", href: "https://example.org/b", children: ["https://example.org/b"] },
    "."
  ]);
  assert.deepEqual(inline("详见https://example.org/geo。其余略"), [
    "详见",
    { tag: "a", href: "https://example.org/geo", children: ["https://example.org/geo"] },
    "。其余略"
  ]);
  assert.deepEqual(inline("[x](javascript:alert(1))"), ["[x](javascript:alert(1))"]);
  assert.deepEqual(inline("[x](data:text/html,hi)"), ["[x](data:text/html,hi)"]);
});

test("HTML in the answer is text, never a node", () => {
  const tree = blocks('<img src=x onerror="alert(1)">\n\n<script>alert(1)</script>');
  assert.deepEqual(tree, [p('<img src=x onerror="alert(1)">'), p("<script>alert(1)</script>")]);
});

test("lists nest by indentation and keep loose items together", () => {
  assert.deepEqual(blocks("- one\n  - inner\n- two\n\n- three"), [
    {
      tag: "ul",
      children: [
        { tag: "li", children: ["one", { tag: "ul", children: [{ tag: "li", children: ["inner"] }] }] },
        { tag: "li", children: ["two"] },
        { tag: "li", children: ["three"] }
      ]
    }
  ]);
  assert.deepEqual(blocks("3. third\n4. fourth\n\nafter"), [
    { tag: "ol", start: 3, children: [{ tag: "li", children: ["third"] }, { tag: "li", children: ["fourth"] }] },
    p("after")
  ]);
});

test("a code block inside a list item belongs to the item", () => {
  assert.deepEqual(blocks("1. Run:\n   ```bash\n   ls -la\n   ```\n2. Done"), [
    {
      tag: "ol",
      children: [
        { tag: "li", children: ["Run:", { tag: "pre", children: [{ tag: "code", children: ["ls -la"] }] }] },
        { tag: "li", children: ["Done"] }
      ]
    }
  ]);
});

test("fenced code is verbatim, also while the fence is still open", () => {
  assert.deepEqual(blocks("```python\nx = a * b  # **not bold**\n```\nafter"), [
    { tag: "pre", children: [{ tag: "code", children: ["x = a * b  # **not bold**"] }] },
    p("after")
  ]);
  assert.deepEqual(blocks("```\nstill streaming"), [
    { tag: "pre", children: [{ tag: "code", children: ["still streaming"] }] }
  ]);
});

test("tables and block quotes", () => {
  assert.deepEqual(blocks("| Gene | n |\n|---|:-:|\n| **TP53** | 12 |\n| a \\| b | 3 |\n\n> quoted\n> *text*"), [
    {
      tag: "table",
      children: [
        {
          tag: "thead",
          children: [{ tag: "tr", children: [{ tag: "th", children: ["Gene"] }, { tag: "th", children: ["n"] }] }]
        },
        {
          tag: "tbody",
          children: [
            {
              tag: "tr",
              children: [
                { tag: "td", children: [{ tag: "strong", children: ["TP53"] }] },
                { tag: "td", children: ["12"] }
              ]
            },
            { tag: "tr", children: [{ tag: "td", children: ["a | b"] }, { tag: "td", children: ["3"] }] }
          ]
        }
      ]
    },
    { tag: "blockquote", children: [p("quoted\n", { tag: "em", children: ["text"] })] }
  ]);
  // A pipe in prose followed by a rule is not a table.
  assert.deepEqual(blocks("a | b\n---"), [p("a | b"), { tag: "hr", children: [] }]);
});

test("every tag the parser can emit is on the allow list", () => {
  const allowed = new Set([
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "blockquote", "pre", "code", "ul", "ol", "li",
    "table", "thead", "tbody", "tr", "th", "td", "strong", "em", "del", "a"
  ]);
  const seen = new Set();
  const walk = (nodes) => nodes.forEach((node) => {
    if (typeof node === "string") return;
    seen.add(node.tag);
    walk(node.children);
  });
  walk(blocks([
    "# h", "###### h6", "text **b** *i* ~~d~~ `c` [l](https://e.com)", "---", "> q", "```\ncode\n```",
    "- a\n  1. b", "| a |\n|---|\n| b |", "<div onclick=x>raw</div>"
  ].join("\n\n")));
  for (const tag of seen) assert.ok(allowed.has(tag), tag);
  assert.ok(seen.has("table") && seen.has("ol") && seen.has("a"));
});
