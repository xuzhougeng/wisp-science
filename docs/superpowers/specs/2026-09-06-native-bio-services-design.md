# Native Rust biological data services

Date: 2026-09-06
Status: All 23 bio-tools domains implemented in Rust (247 tools), including previously license-gated KEGG, CADD, PanglaoDB and Sanger Cell Model Passports; vendored Python tree, copied schemas, launcher and packaging resources removed

## Current implementation slice

`crates/wisp-bio/` implements the seven PubMed operations: `search_articles`,
`get_article_metadata`, `convert_article_ids`, `find_related_articles`,
`lookup_article_by_citation`, `get_full_text_article`, and `get_copyright_status`.
Desktop, CLI and the ACP bridge register them natively and exclude the
corresponding legacy registrations. Native descriptors are included in the
settings inventory even if the Python bundle is missing. Delegation can discover
the native PubMed operations without managed Python.

The independently authored result contracts are explicit: search returns `pmids`,
`total`, `returned`, `retstart`, `next_retstart`, `has_more` and the upstream
retrieval ceiling; metadata returns `records` containing PMID/source URL, the
NCBI citation `summary` and an `abstract` string or null, plus `missing_pmids`.
An absent EFetch record is an error, rather than evidence that no abstract exists.
The previously advertised `title` search sort is not included: the official
PubMed ESearch reference documents relevance, publication date, author and
journal ordering. Date filters use the documented YYYY[/MM[/DD]] notation.
Identifier conversion uses the PMC ID Converter (same-type batches of at most
200 PMID / PMCID / DOI values; missing and unconverted IDs listed; embargo
`live` / `release_date` preserved). Related records use NCBI ELink link names
`pubmed_pubmed`, `pubmed_pmc`, `pubmed_gene`, `pubmed_protein`, and
`pubmed_nucleotide`, preserving upstream ranking on a bounded page. Citation
lookup uses ECitMatch `journal|year|volume|first_page|author|key` strings and
reports matched versus unmatched citations. Full text uses Europe PMC
`{PMCID}/fullTextXML` and distinguishes not-found, not-OA, and XML-unavailable
rather than returning success-shaped empty evidence.

`get_copyright_status` does not call the retired PMC OA Web Service. Mapping
from that legacy shape:

- `copyright.statement` / `year` / `holder` are omitted; they were OA-service
  fields. A stated license is Europe PMC core `license` when present
  (`license.name`).
- Legacy `license.is_open_access` is `is_open_access` from Europe PMC
  `isOpenAccess`. That flag is access, not a reuse grant.
- `reuse_permission` is `unknown` unless a license string is present, in which
  case it is `license_stated`. An OA boolean never becomes a reuse grant.
- Metadata availability, accessible full text (`inEPMC` → `full_text_accessible`),
  and reuse are separate fields. Converter `live` / `release_date` are embargo
  attributes.

The remaining operations also use native Rust. The vendored bio-tools tree,
launcher and catalog reads have now been removed from desktop, CLI, ACP bridge
and packaging. The current contracts and upstream access changes are documented
in [development.md](../../development.md). The problem statement and staged plan
below describe the migration's historical starting point.

Offline regression checks cover request encoding, secrets, upstream errors,
response limits, pagination, malformed XML, nested abstract markup, missing
records, domain filters and delegated authorization. Before release, manually
smoke a PubMed search, metadata, identifier conversion, related-article, citation, full-text and copyright query in the desktop, CLI and ACP bridge;
disable the PubMed connector and verify its tools disappear, then restore it.
Check a custom MCP connection still operates and inspect the native tool results
for source identifiers and pagination/missing-record information.

## Problem and scope

The bundled biological tools currently ship an externally vendored Python
implementation, copied connector schemas and descriptions, and upstream-specific
operational assumptions. The desktop, CLI and ACP MCP bridge all start that
implementation. A replacement must address the implementation and metadata
provenance as well as remove Python from the biological data retrieval path.

The current inventory is 23 domains and 247 declared tools, all implemented
natively. The old serving gate excluded 14 tools (KEGG, CADD, PanglaoDB, Sanger
Cell Model Passports) from the Python path; those operations are now native with
upstream academic/non-commercial notices in their descriptions. This inventory is
a functional migration baseline, not a specification to translate line by line.
An implemented Rust tool must have independently written descriptions, parameter
definitions, result formatting and tests.

Deliver domain-sized changes. Do not expose unfinished tools or replace working
operations with generic HTTP passthroughs merely to reach an inventory count.

This change concerns `mcp-servers/bio-tools/`. Other vendored assets, bundled
skills and Python/R scientific computing runtimes have separate provenance and
remain outside this migration's scope.

## Implementation provenance

- Use database operators' published API documentation as implementation sources.
  Record links and review dates for each provider alongside its implementation.
- Existing tool identifiers and observed user requirements are compatibility
  inputs. Do not translate vendored functions, copy their docstrings, reuse their
  schema JSON or import their test fixtures into the replacement.
- Write synthetic response fixtures that exercise the documented upstream wire
  formats, with no live credentials or copied publications in test data.
- Keep existing attribution while corresponding material is still shipped. At
  removal, review attribution entries individually; do not remove notices for
  unrelated assets.
- This is not a claim of a formal clean-room process: the existing implementation
  has already been inspected. Changing programming language is not itself a
  determination of copyright or API/data licensing status.

The current attribution file references a nonexistent
`mcp-servers/bio-tools/src/lib/` path and a missing bio-tools `NOTICE` file. These
are provenance gaps to resolve during removal, not evidence of a blanket license
for the entire bundle.

## Runtime architecture

Add `crates/wisp-bio/` as the shared biological service implementation. There is
a concrete dependency boundary: the CLI, desktop agent and ACP bridge need the
same catalog and dispatch behavior, without Tauri, Python or an MCP subprocess.

```text
Desktop registry ----\
CLI registry --------> wisp-bio -> reqwest -> documented database APIs
ACP MCP bridge -----/

Custom MCP connections -> existing wisp-mcp client -> external MCP servers
```

Use the existing `wisp_tools::Tool` contract for native registration. Retrieval
tools set `defer_schema()` so the existing `search_mcp_tools` / `use_mcp_tool`
gateway keeps its current behavior. Set `read_only()` per operation; an upstream
job submission or upload needs explicit classification instead of inheriting a
blanket read-only flag.

The ACP bridge retains its existing MCP protocol and exposes the same native
tool implementations. It does not start a second Rust server process. A separate
standalone bio MCP binary is not required for this migration.

Use a small native catalog tying together each implemented tool's identifier,
domain, description and input schema. The settings tree and runtime dispatch
must consume the same implemented inventory. Avoid a second handwritten tool
list that can advertise unavailable operations.

Reuse workspace `reqwest`, `tokio`, `serde`, `serde_json`, `anyhow` and the existing
tool contract. Add an XML parser only for a provider that requires XML. Do not
build an endpoint DSL, a generic plugin system, a new job system or a cache
framework in the first domain replacement.

## HTTP and data behavior

- Each provider owns its documented endpoint paths, argument validation,
  pagination and response parsing. Tools accept scientific identifiers and
  queries, not arbitrary credential-bearing URLs.
- Share clients and upstream pacing within the host process. Do not create a
  fresh rate limiter per tool or per conversation. Separate bridge processes
  and other applications may still share an upstream IP quota; handle 429
  without claiming process-local pacing is a global quota guarantee.
- Bound connect time, total tool duration, retries, response bytes and returned
  records. Retry only suitable transient failures; honor bounded Retry-After.
  Cancellation must stop further pages and retries.
- Report requested/returned identifiers, missing records and whether more data
  exists. Preserve upstream totals where supplied; unknown is not zero and a
  capped page is not a complete dataset. Respect source-specific retrieval
  ceilings and pagination ordering.
- Detect upstream errors even when HTTP status is 200. Return failures through
  `ToolResult::fail` and the MCP bridge's error result rather than success-shaped
  error text or empty scientific evidence.
- Include provider identity, upstream identifiers and source links in results.
  Querying does not automatically persist `Paper`, `Artifact` or `DataAsset`
  records; use existing explicit save/register flows for that work.
- Keep large scientific payloads as references and metadata by default. Full
  text, alignments and structure payloads require explicit, bounded operations.
- Distinguish metadata availability, accessible full text and reuse permissions;
  an open-access boolean alone must not become a blanket reuse assertion.

## Credentials and host integration

Desktop credentials continue to come from `models::service_env()` backed by the
existing keyring. Pass only the needed values into native provider construction;
do not mutate global environment variables or introduce a SQLite secret store.
CLI credentials can come from its environment. Do not log request URLs containing
keys, upstream request bodies echoing secrets, or raw credential structures.

Preserve domain slugs used by disabled-connector settings and capability grants.
Preserve old tool names where their argument and result contracts can be honored
with independently authored code. If a contract changes, provide a documented
mapping and update dependent skills/tests; retaining a name while silently
changing its meaning is not compatibility.

Keep the stable `mcp_bio` connector identity where existing grants and persisted
references depend on it. A native implementation should not accidentally bypass
tool denial, planning mode, connector disabling or delegated capability limits.

The three runtime entry points must switch together for each migrated operation:

| Entry point | Existing integration to replace |
| --- | --- |
| Desktop agent | `src-tauri/src/lib.rs::wire_runtimes_and_mcp` |
| CLI | `crates/wisp-cli/src/main.rs` bundled MCP registration |
| ACP bridge | `src-tauri/src/mcp_bridge.rs::register_bundled_bio_tools` and bio route dispatch |
| Settings/catalog | `src-tauri/src/lib.rs::bio_domains`, `list_mcp_servers` |
| Connector settings | `src-tauri/src/connector_commands.rs::list_connectors` |
| Delegation resources | `src-tauri/src/delegation_resources.rs::bundled_connector_resources`; remove the Python prerequisite for migrated domains |
| Delegated authorization | `src-tauri/src/delegation_runtime.rs::granted_connector_identity`; retain catalog-based identity checks |
| Startup diagnostics | `src-tauri/src/app_commands.rs` bundle existence check |

`WISP_MCP_COMMAND` remains the custom stdio override. During migration, define
`WISP_MCP_PKG` explicitly: a migrated domain selects native tools, an unmigrated
domain retains its temporary legacy path, and `mcp_bio` selects the available
aggregate. On final removal, retain documented domain selection or issue a clear
configuration error; never silently ignore an old selection.

## Incremental migration and removal

1. Replace one useful domain end to end, beginning with PubMed literature
   retrieval. Add the shared crate only with working operations and offline
   tests. Wire native tools through all three hosts and verify settings/grants.
2. Register each migrated operation exactly once. Exclude it from legacy MCP
   registration before adding its native equivalent. A native failure must not
   silently fall back to the vendored implementation.
3. Replace subsequent domains in bounded changes. Track each operation's status,
   official documentation, contract compatibility and tests. Remove obsolete
   provider files once no remaining legacy domain imports them.
4. Partial migration still ships legacy code. It must not be described as a
   completed vendored-code removal or as resolving the whole provenance issue.
5. After coverage is complete, delete the vendored directory and its launcher;
   remove the Tauri resource mapping, legacy path helpers, Python launch code,
   directory-derived catalog and obsolete startup diagnostics.
6. Review Python requirements against actual kernel imports before removing
   MCP-only dependencies. Keep Python/R runtime setup needed for scientific
   computing and keep external MCP client support.
7. Update developer/user documentation, capability text and attribution. Do not
   describe the 87 Python packages as 87 databases. Do not rewrite Git history
   or past release artifacts as part of a source-tree migration.

## Domain coverage baseline

Counts are current callable tools, not a requirement for the new API to have
exactly the same number of separately advertised schemas.

| Domain | Callable tools | Data sources |
| --- | ---: | --- |
| pubmed | 7 | NCBI PubMed/PMC, Europe PMC |
| literature | 9 | OpenAlex, arXiv |
| biorxiv | 7 | bioRxiv, medRxiv |
| biomart | 8 | BioMart |
| genes-ontologies | 10 | MyGene, OLS, QuickGO, Reactome, UniProt, KEGG REST |
| genomes | 11 | Ensembl, UCSC |
| variants | 18 | gnomAD, ClinVar, dbSNP, CADD |
| human-genetics | 14 | GWAS Catalog, eQTL Catalogue, PheWeb |
| clinical-genomics | 20 | ClinGen, CIViC, Open Targets |
| expression | 15 | GTEx, PanglaoDB |
| cellguide | 5 | CELLxGENE CellGuide |
| regulation | 16 | ENCODE, JASPAR, UniBind |
| protein-annotation | 13 | InterPro/Pfam, Human Protein Atlas, STRING |
| rna | 9 | Rfam |
| structures-interactions | 16 | PDB, AlphaFold DB, EMDB, Complex Portal, IntAct |
| omics-archives | 17 | GEO, ArrayExpress, MetaboLights, MGnify, PRIDE |
| cancer-models | 11 | cBioPortal, Sanger Cell Model Passports |
| clinical-trials | 6 | ClinicalTrials.gov |
| drug-regulatory | 7 | openFDA |
| chembl | 6 | ChEMBL |
| chemistry | 12 | PubChem, ChEBI, Rhea, BindingDB |
| zinc | 5 | ZINC22 |
| research-resources | 5 | Antibody Registry, Grants.gov |
| **Total** | **247** | |

## Verification and completion

Use pure parser/request-builder tests and synthetic fixtures. Where HTTP behavior
requires an integration check, use an in-process fake upstream, never a real
database, key, network dependency or external compute resource. Cover malformed
responses, empty results, partial pages, duplicate/missing IDs, upstream errors,
credential redaction, response limits and cancellation where implemented.

Host checks must demonstrate native retrieval without Python, disabled-domain
and delegated-grant enforcement, no duplicate tool names, deferred discovery,
planning-mode classification and error propagation through the ACP bridge.

Run the repository's narrow checks first, then formatting and workspace tests;
Tauri/UI changes also require the wasm check and Playwright suite. MCP integration
changes require the existing `wisp-mcp` smoke example. Source provenance review
and optional live provider smoke checks are separate from automated tests.

Completion means all retained capabilities are implemented and verified, the
three hosts use the native implementation, no shipped runtime/catalog file loads
vendored bio-tools, and packaging/docs/attributions reflect that final state.

## Initial primary sources

Reviewed 2026-09-06:

- [NCBI E-utilities overview and usage requirements](https://www.ncbi.nlm.nih.gov/books/NBK25497/).
- [NCBI E-utilities parameters and retrieval limits](https://www.ncbi.nlm.nih.gov/books/NBK25499/).
- [PMC ID Converter API](https://pmc.ncbi.nlm.nih.gov/tools/id-converter-api/).
- [Europe PMC REST API](https://europepmc.org/RestfulWebService): core metadata and
  Open Access full-text XML retrieval.
- [PMC OA service retirement](https://pmc.ncbi.nlm.nih.gov/tools/oa-service/): the
  official page now states that the old OA Web Service is no longer available.
  Do not recreate that retired dependency for copyright/license metadata.

Before implementing each additional provider, retrieve and record its current
official API reference. Do not use the vendored source as substitute documentation.
