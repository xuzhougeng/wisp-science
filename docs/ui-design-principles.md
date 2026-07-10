# UI design principles

## Icons

- Interactive icons must be SVGs from the shared UI icon renderer or the existing SVG mask set.
- Do not use Unicode characters, emoji, or text glyphs as icons. Their shape, alignment, and availability vary across Windows, macOS, browsers, and fallback fonts.
- Text remains appropriate for labels, status values, scientific notation, and keyboard hints such as `↑↓` or `⌘K`.
- Icon-only controls must retain an accessible `title` or `aria-label`.
