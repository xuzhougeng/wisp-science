CREATE TABLE IF NOT EXISTS plugin_installations (
    plugin_id       TEXT NOT NULL,
    version         TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    author          TEXT NOT NULL DEFAULT '',
    license         TEXT NOT NULL DEFAULT '',
    source_uri      TEXT NOT NULL,
    install_root    TEXT NOT NULL,
    archive_sha256  TEXT NOT NULL,
    manifest_json   TEXT NOT NULL,
    trust_state     TEXT NOT NULL DEFAULT 'unverified',
    installed_at    INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY(plugin_id, version),
    UNIQUE(install_root)
);

CREATE TABLE IF NOT EXISTS project_plugins (
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    plugin_id    TEXT NOT NULL,
    version      TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
    grants_json  TEXT NOT NULL DEFAULT '{}',
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY(project_id, plugin_id),
    FOREIGN KEY(plugin_id, version)
        REFERENCES plugin_installations(plugin_id, version)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS ix_project_plugins_enabled
    ON project_plugins(project_id, enabled, plugin_id);
