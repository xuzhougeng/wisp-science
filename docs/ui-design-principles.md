# UI design principles

## Icons

- Interactive icons must be SVGs from the shared UI icon renderer or the existing SVG mask set.
- Do not use Unicode characters, emoji, or text glyphs as icons. Their shape, alignment, and availability vary across Windows, macOS, browsers, and fallback fonts.
- Text remains appropriate for labels, status values, scientific notation, and keyboard hints such as `↑↓` or `⌘K`.
- Icon-only controls must retain an accessible `title` or `aria-label`.

## Composer attachments and references

- Files, images, skills, artifacts, and conversation references must remain visually distinguishable before and after send.
- Image attachments use a real thumbnail when the project file is readable. Other files use a document card with a filename and type label.
- Persisted transcript markers such as `Uploaded files:` and `Selected skills:` are transport metadata. The chat UI renders them as cards instead of exposing the raw marker text.
- Long attachment names truncate inside the card; the full value remains available through the control's title.
- Remove controls live inside the related card and retain an accessible label.
