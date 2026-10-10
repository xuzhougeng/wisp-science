// Markdown subset for side panel answers: headings, paragraphs, lists, tables,
// block quotes, code, emphasis, and http(s)/mailto links.
//
// The answer is model output shaped by an untrusted page, so it is parsed into
// a tree of known tags and built node by node. Nothing is ever assigned to
// innerHTML, and HTML in the answer stays text.

var MD_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
var MD_FENCE = /^\s{0,3}(`{3,}|~{3,})([^`]*)$/;
var MD_HEADING = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/;
var MD_RULE = /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/;
var MD_QUOTE = /^\s{0,3}>\s?/;
var MD_TABLE_RULE = /^\s*\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*)*\|?\s*$/;
// Groups: 1 code, 2 bold+italic, 3 and 4 bold, 5 strike, 6 and 7 italic,
// 8 and 9 link text and target, 10 bare URL, 11 escaped character.
// A bare URL is ASCII only, so CJK text or punctuation right after it stays out.
var MD_INLINE = new RegExp(
  [
    "`([^`\\n]+)`",
    "\\*\\*\\*(?!\\s)([\\s\\S]+?)(?<!\\s)\\*\\*\\*",
    "\\*\\*(?!\\s)([\\s\\S]+?)(?<!\\s)\\*\\*(?!\\*)",
    "(?<!\\w)__(?!\\s)([\\s\\S]+?)(?<!\\s)__(?!\\w)",
    "~~(?!\\s)([\\s\\S]+?)(?<!\\s)~~",
    "\\*(?![\\s*])([^*]+?)(?<![\\s*])\\*",
    "(?<!\\w)_(?![\\s_])([^_]+?)(?<![\\s_])_(?!\\w)",
    "\\[([^\\]\\n]+)\\]\\(\\s*<?([^)\\s>]+)>?(?:\\s+\"[^\"]*\")?\\s*\\)",
    "(https?:\\/\\/[^\\s<>\\u0080-\\uffff]*[^\\s<>.,;:!?)\\]}'\"*_~`\\u0080-\\uffff])",
    "\\\\([\\\\`*_{}\\[\\]()#+\\-.!~|>])"
  ].join("|"),
  "g"
);
var MD_SAFE_LINK = /^(https?:|mailto:)/i;

function mdInline(text) {
  var out = [];
  // Neighbouring pieces of plain text become one text node.
  var add = function (node) {
    if (typeof node === "string" && typeof out[out.length - 1] === "string") out[out.length - 1] += node;
    else out.push(node);
  };
  var scan = new RegExp(MD_INLINE.source, "g");
  var last = 0;
  var m;
  while ((m = scan.exec(text))) {
    if (m.index > last) add(text.slice(last, m.index));
    last = scan.lastIndex;
    if (m[1] !== undefined) add({ tag: "code", children: [m[1]] });
    else if (m[2] !== undefined) add({ tag: "strong", children: [{ tag: "em", children: mdInline(m[2]) }] });
    else if (m[3] !== undefined || m[4] !== undefined) add({ tag: "strong", children: mdInline(m[3] || m[4]) });
    else if (m[5] !== undefined) add({ tag: "del", children: mdInline(m[5]) });
    else if (m[6] !== undefined || m[7] !== undefined) add({ tag: "em", children: mdInline(m[6] || m[7]) });
    else if (m[8] !== undefined) {
      // Any other scheme (javascript:, data:, a relative path) stays as typed.
      if (MD_SAFE_LINK.test(m[9])) add({ tag: "a", href: m[9], children: [m[8]] });
      else add(m[0]);
    } else if (m[10] !== undefined) add({ tag: "a", href: m[10], children: [m[10]] });
    else add(m[11]);
  }
  if (last < text.length) add(text.slice(last));
  return out;
}

function mdLeading(line) {
  return line.length - line.trimStart().length;
}

function mdIsTable(lines, i) {
  var rule = lines[i + 1];
  return lines[i].indexOf("|") !== -1 && rule !== undefined && rule.indexOf("|") !== -1 && MD_TABLE_RULE.test(rule);
}

function mdStartsBlock(lines, i) {
  var line = lines[i];
  return MD_FENCE.test(line) || MD_HEADING.test(line) || MD_RULE.test(line) || MD_QUOTE.test(line) ||
    MD_ITEM.test(line) || mdIsTable(lines, i);
}

function mdRow(line, tag) {
  var cells = line.trim().replace(/^\|/, "").replace(/\|$/, "").split(/(?<!\\)\|/);
  return {
    tag: "tr",
    children: cells.map(function (cell) {
      return { tag: tag, children: mdInline(cell.trim().replace(/\\\|/g, "|")) };
    })
  };
}

function mdSameList(item, indent, ordered) {
  return !!item && item[1].length >= indent && item[1].length <= indent + 1 && /\d/.test(item[2]) === ordered;
}

// An item owns the lines indented under it; they are parsed as blocks again,
// which is what nests lists and puts code blocks inside items.
function mdList(lines, start) {
  var first = MD_ITEM.exec(lines[start]);
  var indent = first[1].length;
  var ordered = /\d/.test(first[2]);
  var items = [];
  var i = start;
  while (i < lines.length) {
    var item = MD_ITEM.exec(lines[i]);
    if (!mdSameList(item, indent, ordered)) break;
    var content = item[1].length + item[2].length + 1;
    var body = [item[3]];
    i += 1;
    while (i < lines.length) {
      var line = lines[i];
      var next = lines[i + 1];
      var owned = line.trim()
        ? mdLeading(line) >= indent + 2
        : next !== undefined && !!next.trim() && mdLeading(next) >= indent + 2;
      if (!owned) break;
      body.push(line.slice(Math.min(mdLeading(line), content)));
      i += 1;
    }
    var blocks = mdBlocks(body.join("\n"));
    items.push({
      tag: "li",
      children: blocks.length && blocks[0].tag === "p" ? blocks[0].children.concat(blocks.slice(1)) : blocks
    });
    // A blank line between two items of the same list does not end it.
    var after = i;
    while (after < lines.length && !lines[after].trim()) after += 1;
    if (after > i && after < lines.length && mdSameList(MD_ITEM.exec(lines[after]), indent, ordered)) i = after;
  }
  var node = { tag: ordered ? "ol" : "ul", children: items };
  var number = parseInt(first[2], 10);
  if (ordered && number !== 1) node.start = number;
  return { node: node, next: i };
}

function mdBlocks(text) {
  var lines = String(text || "").replace(/\r\n?/g, "\n").split("\n");
  var out = [];
  var i = 0;
  while (i < lines.length) {
    var line = lines[i];
    var m;
    if (!line.trim()) {
      i += 1;
    } else if ((m = MD_FENCE.exec(line))) {
      var code = [];
      i += 1;
      // A fence still open at the end is an answer that is still streaming.
      while (i < lines.length) {
        var close = /^\s{0,3}(`{3,}|~{3,})\s*$/.exec(lines[i]);
        if (close && close[1][0] === m[1][0] && close[1].length >= m[1].length) break;
        code.push(lines[i]);
        i += 1;
      }
      i += 1;
      out.push({ tag: "pre", children: [{ tag: "code", children: [code.join("\n")] }] });
    } else if ((m = MD_HEADING.exec(line))) {
      out.push({ tag: "h" + m[1].length, children: mdInline(m[2]) });
      i += 1;
    } else if (MD_RULE.test(line)) {
      out.push({ tag: "hr", children: [] });
      i += 1;
    } else if (MD_QUOTE.test(line)) {
      var quote = [];
      while (i < lines.length && MD_QUOTE.test(lines[i])) {
        quote.push(lines[i].replace(MD_QUOTE, ""));
        i += 1;
      }
      out.push({ tag: "blockquote", children: mdBlocks(quote.join("\n")) });
    } else if (mdIsTable(lines, i)) {
      var head = mdRow(line, "th");
      var rows = [];
      i += 2;
      while (i < lines.length && lines[i].trim() && lines[i].indexOf("|") !== -1) {
        rows.push(mdRow(lines[i], "td"));
        i += 1;
      }
      out.push({
        tag: "table",
        children: [{ tag: "thead", children: [head] }, { tag: "tbody", children: rows }]
      });
    } else if (MD_ITEM.test(line)) {
      var list = mdList(lines, i);
      out.push(list.node);
      i = list.next;
    } else {
      var para = [line.trim()];
      i += 1;
      while (i < lines.length && lines[i].trim() && !mdStartsBlock(lines, i)) {
        para.push(lines[i].trim());
        i += 1;
      }
      out.push({ tag: "p", children: mdInline(para.join("\n")) });
    }
  }
  return out;
}

function renderMarkdown(target, text) {
  var doc = target.ownerDocument;
  var build = function (node) {
    if (typeof node === "string") return doc.createTextNode(node);
    var el = doc.createElement(node.tag);
    if (node.tag === "a") {
      el.href = node.href;
      el.target = "_blank";
      el.rel = "noopener noreferrer";
    }
    if (node.start) el.start = node.start;
    node.children.forEach(function (child) { el.appendChild(build(child)); });
    return el;
  };
  target.replaceChildren.apply(target, mdBlocks(text).map(build));
}

if (typeof self !== "undefined") {
  self.mdInline = mdInline;
  self.mdBlocks = mdBlocks;
  self.renderMarkdown = renderMarkdown;
}
