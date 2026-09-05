# Workspace scrolling

The desktop viewport and its three shell columns do not scroll. Only inner
conversation, session-list, artifact-list, file and agent content areas scroll;
project/session headers and right-panel tabs remain visible. Shell clipping uses
`overflow: clip` because `overflow: hidden` still permits programmatic scrolling
from focus and `scrollIntoView`; the body is fixed to the viewport so document
scrolling cannot move all three headers together.

Conversation jumps scroll the conversation container explicitly. Conversation
reading positions remain per-session; switching conversations resets the mounted
right-panel lists to the top so a previous session's deep offset is not reused.

Manual check on Windows WebView2 and macOS: open two long conversations with
many artifacts, scroll each column to the bottom, switch sessions repeatedly,
and use conversation outline jumps. All three headers must stay visible and
Artifacts/Agents/Files must remain clickable. Repeat with the artifact preview
hidden, a small window, and a large file preview.
