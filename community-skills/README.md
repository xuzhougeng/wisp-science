# Community Skills directory

This is Wisp Science's free, public Skill discovery directory. Entries may point
to a package in this repository or a third-party public GitHub repository.
Inclusion means the entry was reviewed for listing; it does not mean official
maintenance, security certification, scientific correctness, or runtime testing.
No account, payment, or rating system is required.

Read the [official Skill authoring guide](../docs/skill-authoring.md), including
its responsibilities structure, actual YAML vocabulary, dependency declarations,
permissions, and upgrade-preservation guidance. Review
[the complete example package](examples/research-handoff/SKILL.md).

To propose a listing or update, open a PR editing [index.json](index.json).
Use the existing entry as the template; all its fields are required except
`verified_wisp`, which should be `null` until a named version was actually tested.
Use plain, specific descriptions and declare required versus optional runtime,
model, tool, platform, and network dependencies. Include a valid license and an
accessible author/maintainer feedback link. Do not copy third-party code without
its license. Keep package descriptions and the index aligned. A GitHub source
must identify `owner/repository`, the exact branch/tag/commit, and the directory
containing `SKILL.md`; an empty package path means the repository root.

Review checklist:

- Source, license, maintainer and package path can be inspected publicly.
- Responsibilities include triggers, inputs, outputs, and exclusions.
- Frontmatter parses under the linked Wisp parser; references resolve inside
  the complete package. No symlinks, submodules, or Git LFS pointers are required.
- Claimed versions, actual verification, known limitations, and dependency setup
  are described separately. Parser success alone is not runtime validation.
- Tests use fixtures and temporary directories without GitHub credentials or
  real external infrastructure: `cargo test -p wisp-skills`.
- Maintainers use the normal review process; no automatic publication from a
  user's install operation.

The application ships a snapshot of this index and offers an explicit refresh.
Each install resolves the listing's ref to a concrete commit and asks the user
to review the package and global installation scope. Removing a listing or
changing a branch never removes or upgrades already installed files. This first
version refuses same-name replacement. Repository size limits and future work
are documented in the authoring guide.
