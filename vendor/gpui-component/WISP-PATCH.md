# Paragraph gaps in text views

This directory contains the published `gpui-component 0.5.1` crate, under its
original Apache-2.0 license. It is excluded from the root workspace and
selected through the `[patch.crates-io]` section of `apps/gpui/Cargo.toml`.

## Provenance

- Registry: <https://crates.io/crates/gpui-component/0.5.1>
- Published `.crate` SHA-256:
  `d021d46b4088d3d93a57ccdf443da85695a77272108caca2f6fe5369f584966a`
- Everything other than the local change below is unchanged from the
  published crate. The registry's `.cargo-ok` marker is omitted.

## Local change

Only `src/text/node.rs` is changed, at the places marked `WISP PATCH`:

- `Node::render_block`, `Node::Root`: a text view that does not scroll draws
  its root with `is_last: true` and passed that to every top-level block, so
  no paragraph, list or code block got `TextViewStyle::paragraph_gap` after
  it and an answer's paragraphs ran together. Each block is now last only if
  it is the root's last.
- `Node::render_table`: the gap after a table is `paragraph_gap`, not a fixed
  `1rem`.
- Inline strong text (`TextMark::bold`, also from HTML `<b>`/`<strong>`) is
  `FontWeight::MEDIUM`, not `BOLD`.
- `Node::Heading`: every level is `FontWeight::MEDIUM` (was Bold for h1,
  Semibold for h2–h5, Medium for h6). Sizes are left to the library's
  `TextViewStyle::heading_font_size` hook, which Wisp sets from its side
  (`surfaces::text_style`).

These follow Wisp's weight rule (no Semibold or Bold); a test in
`apps/gpui/src/workspace/surfaces.rs` rejects `FontWeight::BOLD` or
`SEMIBOLD` in `src/text/node.rs` outside comments.

## Removal

When a published `gpui-component` passes each root child its own `is_last`,
remove this directory, the patch section in `apps/gpui/Cargo.toml` and the
root workspace exclusion. Keep `surfaces::text_style` and its test.
