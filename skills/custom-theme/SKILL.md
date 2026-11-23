---
name: custom-theme
description: Author a Wisp custom theme CSS file that uses documented tokens and import constraints. Use when the user wants a custom look, hide the paragraph lead bar, restyle chat Markdown, override colors, or import a stylesheet under Settings → Appearance.
---

# Custom Wisp theme CSS

Wisp injects one user stylesheet after the built-in theme. Prefer CSS variables
on `:root`. Do not invent Settings toggles for one-off look changes.

## Workflow

1. Ask what should change (lead bar, table size, accent, chat density, fonts).
2. Write a `.css` file in the project, usually `theme.css`.
3. Apply it:
   - Desktop: `configure` with `action=set` and `values.custom_css` set to the
     full stylesheet text. Empty string clears it.
   - Or tell the user to import the file under **Settings → Appearance**.
4. Confirm the change is visible. If a rule did nothing, the selector is wrong;
   do not add `!important` unless a built-in rule requires it.

## Constraints

The host sanitizes CSS before inject. These are stripped:

- `@import` and `@namespace`
- every `url(...)`
- `javascript:`, `expression(`, `behavior:`, `-moz-binding`
- `</style` and `<script`
- NULs; payload longer than 64 KB is truncated

Do not rely on remote fonts, images, or other stylesheets. Name an installed
font with `configure` `ui_font_family` / `code_font_family` instead.

Scope light/dark overrides with `:root[data-theme="light"]` and
`:root[data-theme="dark"]`. System theme follows the OS, so also cover
`:root[data-theme="system"]` when the user cares about both.

## Tokens

Override these on `:root`. Leave unused tokens alone.

| Token | Default role |
| --- | --- |
| `--bg-app`, `--bg-elev`, `--bg-sunken`, `--bg-input`, `--bg-panel` | Surfaces |
| `--text`, `--text-muted`, `--text-faint` | Ink |
| `--border`, `--border-strong` | Hairlines |
| `--clay`, `--clay-strong`, `--clay-soft`, `--on-clay` | Accent (teal) |
| `--ui-font-size`, `--code-font-size` | Set by Appearance sliders; do not hard-code px on chat body |
| `--md-table-font-size` | Markdown tables; default `calc(var(--ui-font-size, 14px) - 1px)` |
| `--md-lead-bar-width`, `--md-lead-bar-pad` | Bold-at-start emphasis bar; `3px` / `0.55em` |
| `--md-prose-measure` | Maximum line width of answer prose (paragraphs, lists, headings); tables, code and figures still use the full column. Default `calc(50 * (var(--ui-font-size, 14px) + 1px))`, about 750px; `none` removes the cap |
| `--toggle-thumb` | Settings switch thumb; light in both themes |
| `--font-user-ui`, `--font-user-mono` | Optional family names; empty restores the stack |
| `--radius`, `--radius-sm`, `--radius-xs` | Corners |

Palette attributes (`data-light-palette`, `data-dark-palette`) already map onto
the semantic tokens. Prefer `--clay` / `--bg-app` over palette internals
(`--lp-*`, `--dp-*`) unless the user asked to restyle one palette only.

## Common requests

Hide the lead bar (the vertical mark before a paragraph that starts with bold):

```css
:root {
  --md-lead-bar-width: 0;
  --md-lead-bar-pad: 0;
}
```

Keep tables aligned with UI size, or pin a size:

```css
:root { --md-table-font-size: calc(var(--ui-font-size, 14px) - 1px); }
```

```css
:root { --md-table-font-size: 14px; }
```

Quiet chat chrome:

```css
.msg.assistant .role { display: none; }
.msg.assistant .body.md { letter-spacing: 0; }
```

Do not restyle window controls, settings forms, or tool-approval cards unless
the user asked. Keep contrast readable on both themes.

## Apply via configure

```json
{
  "action": "set",
  "values": {
    "custom_css": ":root { --md-lead-bar-width: 0; --md-lead-bar-pad: 0; }\n"
  }
}
```

After writing `theme.css`, `read` it and pass the exact file contents. Do not
summarize or wrap the CSS in Markdown fences inside `custom_css`.
