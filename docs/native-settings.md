# Desktop settings boundary

The separate SwiftUI and WinUI settings screens and packaging helpers are
retired. The GPUI preview currently reads explicit provider environment values;
see [GPUI setup and migration](gpui-desktop.md). Its settings UI and migration
of the existing desktop database/keyring configuration are pending.

The existing Tauri settings host, `wisp-dto` settings contracts, and keyring
storage remain for current desktop and remote consumers. Do not delete these
Rust services merely because their modules begin with `native_`.
Secrets must remain in the existing keyring path or process environment, never
in SQLite. The GPUI preview does not import or overwrite production settings.
