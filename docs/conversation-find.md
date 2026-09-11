# Find in a conversation

Press **Ctrl+F** on Windows/Linux or **Command+F** on macOS while viewing the
conversation to search its displayed message bodies. The count and orange
active highlight use the same ordered list of matches; yellow highlights show
the other matches. Sidebar labels, session titles, composer text and hidden
content are excluded. Matching is literal and case-insensitive, including text
split by inline Markdown formatting.

**Enter** / **Shift+Enter** and the next/previous arrows move through matches,
wrapping at either end. Navigation pauses automatic following of new output,
so a result reached from the bottom remains visible. **Back to latest** resumes
following. **Escape** closes the find bar after any overlaid dialog or menu;
closing find preserves the reading position. Changing sessions closes find.

Search covers the currently displayed transcript window. Load older messages
to search that window; the count refreshes when displayed content changes.
Collapsed content is searchable after expanding it. Editors and terminals keep
their own find shortcuts when focused.
Older WebViews without the CSS Highlight API retain their native find UI.
