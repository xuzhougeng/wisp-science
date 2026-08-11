use super::*;

// --- Artifact detection (Markdown tables + fenced CSV) -----------------------

/// Segment assistant text into plain-text and rendered Markdown-table chunks.
pub(crate) enum Seg {
    Text,
    Table(TableData),
}

pub(crate) fn split_segments(text: &str) -> Vec<Seg> {
    let lines: Vec<&str> = text.lines().collect();
    let mut segs: Vec<Seg> = vec![];
    let mut buf: Vec<&str> = vec![];
    let mut i = 0;
    while i < lines.len() {
        if is_table_row(lines[i]) && i + 1 < lines.len() && is_separator(lines[i + 1]) {
            if !buf.is_empty() {
                segs.push(Seg::Text);
                buf.clear();
            }
            let headers = split_row(lines[i]);
            let mut rows = vec![];
            let mut j = i + 2;
            while j < lines.len() && is_table_row(lines[j]) {
                rows.push(split_row(lines[j]));
                j += 1;
            }
            segs.push(Seg::Table(TableData { headers, rows }));
            i = j;
        } else {
            buf.push(lines[i]);
            i += 1;
        }
    }
    if !buf.is_empty() {
        segs.push(Seg::Text);
    }
    segs
}

/// Locale-free artifact material extracted from one chat item. Ids, localized
/// names, per-kind numbering, and cross-item file dedup are assigned later in
/// `assemble_artifacts`, so this half is cacheable per item and streaming only
/// re-extracts the tail message instead of rescanning the whole transcript.
pub(crate) enum ProtoArtifact {
    Table(Rc<TableData>),
    Csv(Rc<TableData>),
    Fasta(Rc<str>),
    Latex(String),
    File { path: String, kind: &'static str },
}

pub(crate) fn file_proto(word: &str) -> Option<ProtoArtifact> {
    let p = normalize_path(
        word.trim()
            .trim_matches('`')
            .trim_matches('"')
            .trim_matches('\''),
    );
    if p.is_empty() {
        return None;
    }
    let kind = file_kind(&p)?;
    // Source files preview fine once opened, but this scan runs over every word
    // of tool output — auto-promoting each .py/.R path it mentions (tracebacks,
    // import lists, pip logs) would bury the pane. Code belongs in Notebook.
    if kind == "code" {
        return None;
    }
    Some(ProtoArtifact::File { path: p, kind })
}

pub(crate) struct ArtifactScan {
    tbl_n: usize,
    csv_n: usize,
    tex_n: usize,
}

pub(crate) fn extract_markdown_protos(out: &mut Vec<ProtoArtifact>, s: &str) {
    for seg in split_segments(s) {
        if let Seg::Table(t) = seg {
            out.push(ProtoArtifact::Table(Rc::new(t)));
        }
    }
    for (lang, body) in fenced_blocks(s) {
        if lang == "csv" || lang == "tsv" {
            let lines: Vec<&str> = body.lines().collect();
            if let Some(header) = lines.first() {
                let headers = parse_csv_line(header);
                let rows = lines[1..].iter().map(|line| parse_csv_line(line)).collect();
                out.push(ProtoArtifact::Csv(Rc::new(TableData { headers, rows })));
            }
        } else if lang == "fasta" || lang == "fa" {
            out.push(ProtoArtifact::Fasta(Rc::from(body)));
        }
    }
    let lines: Vec<&str> = s.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().starts_with("```") {
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                i += 1;
            }
            i = i.saturating_add(1);
            continue;
        }
        if lines[i].trim().starts_with("$") {
            let mut body = vec![];
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim().ends_with("$") {
                body.push(lines[j]);
                j += 1;
            }
            if j < lines.len() {
                body.push(lines[j].trim().trim_end_matches("$"));
            }
            out.push(ProtoArtifact::Latex(body.join("\n")));
            i = j + 1;
            continue;
        }
        i += 1;
    }
}

/// Extraction half of the artifact scan: pure function of one item's content.
pub(crate) fn extract_protos(it: &ChatItem) -> Vec<ProtoArtifact> {
    let mut out = vec![];
    match it {
        // Uploaded files live only in the user turn ("Uploaded files: a, b").
        ChatItem::User(s) => {
            for word in s.split(|c: char| c.is_whitespace() || c == ',' || c == '"' || c == '\'') {
                if let Some(p) = file_proto(word) {
                    out.push(p);
                }
            }
        }
        ChatItem::Assistant { text: s, .. } => extract_markdown_protos(&mut out, s),
        ChatItem::Tool { name, output, .. } => {
            if name == "attempt_completion" && !output.is_empty() {
                extract_markdown_protos(&mut out, output);
            }
        }
        ChatItem::FileChanged(path) => {
            if let Some(proto) = file_proto(path) {
                out.push(proto);
            }
        }
        _ => {}
    }
    out
}

/// Numbering half: assign ids, localized names, and cross-item file dedup.
/// O(artifact count) per run — cheap next to re-scanning message text.
pub(crate) fn assemble_artifacts(
    per_item: &[Rc<Vec<ProtoArtifact>>],
    locale: Locale,
) -> Vec<Artifact> {
    let mut out: Vec<Artifact> = vec![];
    let mut scan = ArtifactScan {
        tbl_n: 0,
        csv_n: 0,
        tex_n: 0,
    };
    for (source_item, protos) in per_item.iter().enumerate() {
        for p in protos.iter() {
            match p {
                ProtoArtifact::Table(t) => {
                    scan.tbl_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: tf(locale, "artifact.table", &[("n", &scan.tbl_n.to_string())]),
                        kind: "table",
                        data: PreviewData::Table(t.clone()),
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::Csv(t) => {
                    scan.csv_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: format!("data-{}.csv", scan.csv_n),
                        kind: "csv",
                        data: PreviewData::Table(t.clone()),
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::Fasta(body) => {
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: format!("alignment-{}.fasta", scan.csv_n),
                        kind: "fasta",
                        data: PreviewData::Fasta(body.clone()),
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::Latex(tex) => {
                    scan.tex_n += 1;
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name: tf(
                            locale,
                            "artifact.equation",
                            &[("n", &scan.tex_n.to_string())],
                        ),
                        kind: "latex",
                        data: PreviewData::Latex {
                            tex: tex.clone(),
                            display: true,
                        },
                        source_item,
                        superseded: false,
                    });
                }
                ProtoArtifact::File { path, kind } => {
                    if out.iter().any(|a| {
                        a.source_item == source_item
                            && matches!(&a.data, PreviewData::File { path: p, .. } if p == path)
                    }) {
                        continue;
                    }
                    for existing in out.iter_mut().filter(
                        |a| matches!(&a.data, PreviewData::File { path: p, .. } if p == path),
                    ) {
                        existing.superseded = true;
                    }
                    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
                    let id = next_artifact_id(out.len());
                    out.push(Artifact {
                        id,
                        name,
                        kind,
                        data: PreviewData::File {
                            path: path.clone(),
                            kind: kind.to_string(),
                        },
                        source_item,
                        superseded: false,
                    });
                }
            }
        }
    }
    out
}

/// Fold one round's token usage into the current turn's single usage row and
/// float that row to the tail, so the reply ends with one cumulative line and
/// the row never splits the coalesced tool-steps panel mid-turn.
pub(crate) fn upsert_turn_usage(
    items: &mut Vec<ChatItem>,
    input: u64,
    output: u64,
    reasoning: u64,
    cached: u64,
    ctx_tokens: usize,
    max_context: usize,
    context_usage: ContextUsage,
) {
    let turn_start = items
        .iter()
        .rposition(|i| matches!(i, ChatItem::User(_)))
        .map(|i| i + 1)
        .unwrap_or(0);
    let (mut in_sum, mut out_sum, mut r_sum, mut c_sum) = (input, output, reasoning, cached);
    if let Some(i) = items[turn_start..]
        .iter()
        .position(|i| matches!(i, ChatItem::Usage { .. }))
    {
        if let ChatItem::Usage {
            input: a,
            output: b,
            reasoning: c,
            cached: d,
            ..
        } = items.remove(turn_start + i)
        {
            in_sum += a;
            out_sum += b;
            r_sum += c;
            c_sum += d;
        }
    }
    let idx = trailing_queue_start(items);
    items.insert(
        idx,
        ChatItem::Usage {
            input: in_sum,
            output: out_sum,
            reasoning: r_sum,
            cached: c_sum,
            ctx_tokens,
            max_context,
            context_usage,
        },
    );
}

pub(crate) fn latest_context_usage(items: &[ChatItem]) -> Option<ContextUsageSnapshot> {
    items.iter().rev().find_map(|item| {
        let ChatItem::Usage {
            ctx_tokens,
            max_context,
            context_usage,
            ..
        } = item
        else {
            return None;
        };
        (*ctx_tokens > 0 || *max_context > 0).then_some(ContextUsageSnapshot {
            used: *ctx_tokens,
            max: *max_context,
            // Pre-feature usage rows only persisted totals. Attribute the whole
            // window to Conversation so the panel never pretends the native
            // agent only has an opaque "Agent-managed" bucket.
            breakdown: Some(if context_usage.total() > 0 {
                *context_usage
            } else {
                ContextUsage {
                    conversation: *ctx_tokens,
                    ..ContextUsage::default()
                }
            }),
            estimated: true,
        })
    })
}

#[cfg(test)]
mod usage_row_tests {
    use super::*;

    #[test]
    fn accumulates_rounds_and_floats_to_tail() {
        let mut items = vec![
            ChatItem::User("q".into()),
            ChatItem::Assistant {
                text: "a".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        upsert_turn_usage(
            &mut items,
            100,
            10,
            5,
            40,
            1_000,
            8_000,
            ContextUsage::default(),
        );
        items.push(ChatItem::Tool {
            name: "python".into(),
            ok: Some(true),
            input: String::new(),
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
            call_key: None,
            details: None,
        });
        upsert_turn_usage(
            &mut items,
            200,
            20,
            0,
            60,
            1_500,
            8_000,
            ContextUsage::default(),
        );
        // Single cumulative row at the tail, prior turns untouched.
        assert!(matches!(
            items.last(),
            Some(ChatItem::Usage {
                input: 300,
                output: 30,
                reasoning: 5,
                cached: 100,
                ctx_tokens: 1_500,
                max_context: 8_000,
                ..
            })
        ));
        assert_eq!(
            items
                .iter()
                .filter(|i| matches!(i, ChatItem::Usage { .. }))
                .count(),
            1
        );
        let context = latest_context_usage(&items).expect("latest context snapshot");
        assert_eq!((context.used, context.max), (1_500, 8_000));
        assert_eq!(
            context.breakdown,
            Some(ContextUsage {
                conversation: 1_500,
                ..ContextUsage::default()
            })
        );
        assert!(context.estimated);
        // A new user turn starts a fresh row.
        items.push(ChatItem::User("q2".into()));
        upsert_turn_usage(
            &mut items,
            50,
            5,
            0,
            0,
            2_000,
            8_000,
            ContextUsage::default(),
        );
        assert!(matches!(
            items.last(),
            Some(ChatItem::Usage {
                input: 50,
                output: 5,
                reasoning: 0,
                cached: 0,
                ctx_tokens: 2_000,
                max_context: 8_000,
                ..
            })
        ));
    }
}

/// Promote `attempt_completion` output into the assistant bubble (web-dist renders
/// completion as the final markdown response, not a collapsed tool row).
///
/// Only the placeholder at the turn tail may be filled in place: reaching back
/// across tool rows to the pre-tool placeholder puts the answer before the
/// tools, where `is_commentary_at` folds it into the collapsed steps panel and
/// the final reply never shows until the session is reloaded.
pub(crate) fn promote_assistant_text(items: &mut Vec<ChatItem>, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let idx = process_item_insert_index(items);
    if let Some(ChatItem::Assistant { text: s, .. }) =
        idx.checked_sub(1).and_then(|i| items.get_mut(i))
    {
        if s.is_empty() {
            s.push_str(text);
            return;
        }
    }
    // Carry the turn's model label over from the stranded empty placeholder.
    let model = items.iter().rev().find_map(|item| match item {
        ChatItem::Assistant { text, model, .. } if text.is_empty() => model.clone(),
        _ => None,
    });
    items.insert(
        idx,
        ChatItem::Assistant {
            text: text.to_string(),
            model,
            resources: Vec::new(),
        },
    );
}

#[cfg(test)]
mod promote_assistant_text_tests {
    use super::{is_commentary_at, promote_assistant_text};
    use crate::dto::{ChatItem, ContextUsage};

    fn tool(name: &str) -> ChatItem {
        ChatItem::Tool {
            name: name.into(),
            ok: Some(true),
            input: String::new(),
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
            call_key: None,
            details: None,
        }
    }

    #[test]
    fn fills_the_tail_placeholder_in_place() {
        let mut items = vec![
            ChatItem::User("q".into()),
            ChatItem::Assistant {
                text: String::new(),
                model: Some("gpt".into()),
                resources: Vec::new(),
            },
        ];
        promote_assistant_text(&mut items, "answer");
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[1], ChatItem::Assistant { text, .. } if text == "answer"));
    }

    #[test]
    fn answer_lands_after_tool_rows_not_in_the_pre_tool_placeholder() {
        // Live layout: start_user_turn's empty placeholder precedes the tool
        // rows; filling it there renders the final reply as collapsed
        // commentary until the session is reloaded.
        let mut items = vec![
            ChatItem::User("q".into()),
            ChatItem::Assistant {
                text: String::new(),
                model: Some("gpt".into()),
                resources: Vec::new(),
            },
            tool("web_scan"),
            tool("attempt_completion"),
            ChatItem::Usage {
                input: 10,
                output: 1,
                reasoning: 0,
                cached: 0,
                ctx_tokens: 0,
                max_context: 0,
                context_usage: ContextUsage::default(),
            },
        ];
        promote_assistant_text(&mut items, "final answer");
        assert!(matches!(&items[1], ChatItem::Assistant { text, .. } if text.is_empty()));
        let ChatItem::Assistant { text, model, .. } = &items[4] else {
            panic!("expected promoted answer before the trailing usage row");
        };
        assert_eq!(text, "final answer");
        assert_eq!(model.as_deref(), Some("gpt"));
        assert!(!is_commentary_at(&items, 4));
        assert!(matches!(items[5], ChatItem::Usage { .. }));
    }
}

/// Identity hash of the artifact list as seen by assistant markdown (chip
/// index, id, and label). Mixed into assistant row keys so chips re-render
/// when the artifact list changes, and nothing re-renders when it doesn't.
pub(crate) fn artifacts_fingerprint(arts: &[Artifact]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for a in arts {
        (&a.id, &a.name).hash(&mut h);
    }
    h.finish()
}

pub(crate) type ProtoCache = HashMap<(usize, u64), Rc<Vec<ProtoArtifact>>>;

/// Collect tables, data blocks, equations, and file-path artifacts from the transcript.
/// Extraction is cached per (index, content hash): a streaming flush only
/// re-extracts the message it touched, then renumbering runs over the cached
/// protos — instead of rescanning every message on every signal write.
pub(crate) fn collect_artifacts(
    items: &[ChatItem],
    locale: Locale,
    cache: &mut ProtoCache,
) -> Vec<Artifact> {
    let mut next: ProtoCache = HashMap::with_capacity(items.len());
    let mut per_item: Vec<Rc<Vec<ProtoArtifact>>> = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        let key = (i, it.fingerprint());
        let protos = cache
            .remove(&key)
            .unwrap_or_else(|| Rc::new(extract_protos(it)));
        next.insert(key, protos.clone());
        per_item.push(protos);
    }
    *cache = next;
    let mut artifacts = assemble_artifacts(&per_item, locale);
    for artifact in &mut artifacts {
        if matches!(
            items.get(artifact.source_item),
            Some(ChatItem::FileChanged(_))
        ) {
            let next_final_assistant = items
                .iter()
                .enumerate()
                .skip(artifact.source_item + 1)
                .find_map(|(index, item)| match item {
                    ChatItem::Assistant { text, .. }
                        if !text.trim().is_empty() && !is_commentary_at(items, index) =>
                    {
                        Some(Some(index))
                    }
                    ChatItem::User(_) => Some(None),
                    _ => None,
                })
                .flatten();
            if let Some(next_final_assistant) = next_final_assistant {
                artifact.source_item = next_final_assistant;
            }
        }
    }
    artifacts
}

/// Stable file identity for the current workspace. Agent and tool messages may
/// refer to the same file using either an absolute path or a workspace-relative
/// path; the Artifacts panel should still treat those references as one file.
fn artifact_file_identity(path: &str, project_root: &str) -> String {
    fn slash_path(path: &str) -> String {
        let mut normalized = path.trim().replace('\\', "/");
        while normalized.contains("//") {
            normalized = normalized.replace("//", "/");
        }
        while normalized.starts_with("./") {
            normalized.drain(..2);
        }
        normalized.trim_end_matches('/').to_string()
    }

    let path = slash_path(path);
    let root = slash_path(project_root);
    let windows_workspace = root.as_bytes().get(1) == Some(&b':');
    let comparable_path = if windows_workspace {
        path.to_ascii_lowercase()
    } else {
        path.clone()
    };
    let comparable_root = if windows_workspace {
        root.to_ascii_lowercase()
    } else {
        root.clone()
    };

    let relative = comparable_path
        .strip_prefix(&(comparable_root + "/"))
        .unwrap_or(&comparable_path);
    relative.to_string()
}

/// Current artifact projection for the right-hand panel. The full scan is kept
/// for transcript cards and provenance, while this view shows only the latest
/// live reference for each physical workspace file.
///
/// `registered` are the rows `list_artifacts` returns for the session — files
/// that harvest, delegation, the MCP bridge and uploads wrote straight to the
/// database. Nothing mentions them in chat text, so the transcript scan above
/// cannot see them; they are merged in here, deduplicated against it by file
/// identity. They deliberately skip the `missing_paths` filter: that guard
/// exists for paths *scraped* out of prose, and it would also drop every
/// registered remote (`ssh://`) reference, which has no local file to stat.
pub(crate) fn current_artifacts(
    artifacts: &[Artifact],
    registered: &[ArtifactInfo],
    project_root: &str,
    missing_paths: &HashSet<String>,
) -> Vec<Artifact> {
    let mut seen_files = HashSet::new();
    let mut current = Vec::with_capacity(artifacts.len());
    for artifact in artifacts.iter().rev() {
        if artifact.superseded {
            continue;
        }
        match &artifact.data {
            PreviewData::File { path, .. } => {
                if missing_paths.contains(path) {
                    continue;
                }
                let identity = artifact_file_identity(path, project_root);
                if !seen_files.insert(identity) {
                    continue;
                }
            }
            _ => {}
        }
        current.push(artifact.clone());
    }
    current.reverse();
    for info in registered {
        if !seen_files.insert(artifact_file_identity(&info.path, project_root)) {
            continue;
        }
        let kind = file_kind(&info.path)
            .or_else(|| file_kind(&info.name))
            .unwrap_or("file");
        current.push(Artifact {
            id: info.id.clone(),
            name: info.name.clone(),
            kind,
            data: PreviewData::File {
                path: info.path.clone(),
                kind: kind.to_string(),
            },
            // No transcript message produced these, so they never render as an
            // inline card under one (`artifacts_for_item` matches on this).
            source_item: usize::MAX,
            superseded: false,
        });
    }
    current
}

#[cfg(test)]
mod artifact_scan_tests {
    use super::*;

    fn fresh(items: &[ChatItem], locale: Locale) -> Vec<Artifact> {
        collect_artifacts(items, locale, &mut ProtoCache::new())
    }

    #[test]
    fn cached_table_payload_is_shared_across_artifact_projections() {
        let item = ChatItem::Assistant {
            text: "| sample | value |\n| --- | --- |\n| a | 1 |".into(),
            model: None,
            resources: Vec::new(),
        };
        let protos = Rc::new(extract_protos(&item));
        let proto_table = match &protos[0] {
            ProtoArtifact::Table(table) => table,
            _ => panic!("expected a table proto"),
        };
        let artifacts = assemble_artifacts(&[Rc::clone(&protos)], Locale::En);
        let artifact_table = match &artifacts[0].data {
            PreviewData::Table(table) => table,
            _ => panic!("expected a table artifact"),
        };
        assert!(Rc::ptr_eq(proto_table, artifact_table));

        let current = current_artifacts(&artifacts, &[], "", &HashSet::new());
        let current_table = match &current[0].data {
            PreviewData::Table(table) => table,
            _ => panic!("expected a current table artifact"),
        };
        assert!(Rc::ptr_eq(artifact_table, current_table));
    }

    /// #307 made .R/.py previewable, which must not also turn every source path
    /// this scan walks past (tracebacks, import lists) into an artifact chip.
    #[test]
    fn source_paths_do_not_become_artifacts() {
        assert!(file_proto("scripts/deseq2.R").is_none());
        assert!(file_proto("/mock/root/train.py").is_none());
        assert!(file_proto("pixi.toml").is_none());
        // Data and documents still do.
        assert!(matches!(
            file_proto("out/report.csv"),
            Some(ProtoArtifact::File { kind: "csv", .. })
        ));
        assert!(matches!(
            file_proto("notes.md"),
            Some(ProtoArtifact::File {
                kind: "markdown",
                ..
            })
        ));
    }

    /// Streaming reuses cached extractions for untouched messages; the result
    /// must be identical to a from-scratch scan (ids, names, order, dedup).
    #[test]
    fn cached_scan_matches_fresh_scan() {
        let mut cache = ProtoCache::new();
        let mut items = vec![
            ChatItem::User("check data.csv and data.csv".into()),
            ChatItem::Assistant {
                text: "| a | b |\n|---|---|\n| 1 | 2 |\n\n```csv\nx,y\n1,2\n```".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let a1 = collect_artifacts(&items, Locale::En, &mut cache);
        assert!(a1 == fresh(&items, Locale::En));
        // csv file (deduped once) + table + fenced csv
        assert_eq!(a1.len(), 3);

        // Simulate a streaming flush: the tail message grows a code fence.
        if let ChatItem::Assistant { text, .. } = &mut items[1] {
            text.push_str("\n```py\nprint(1)\n```");
        }
        items.push(ChatItem::Tool {
            name: "write".into(),
            ok: Some(true),
            input: String::new(),
            output: "wrote out/result.csv".into(),
            started_at_ms: None,
            duration_ms: Some(1),
            call_key: None,
            details: None,
        });
        items.push(ChatItem::FileChanged("out/result.csv".into()));
        let a2 = collect_artifacts(&items, Locale::En, &mut cache);
        assert!(a2 == fresh(&items, Locale::En));
        assert_eq!(a2.len(), 4); // code moves to Notebook; the structured write is an artifact
    }

    #[test]
    fn overwritten_file_belongs_to_its_latest_message() {
        let items = vec![
            ChatItem::FileChanged("result.csv".into()),
            ChatItem::Assistant {
                text: "Created the result.".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::FileChanged("result.csv".into()),
            ChatItem::Assistant {
                text: "Updated the result.".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let artifacts = fresh(&items, Locale::En);
        assert_eq!(artifacts.len(), 2);
        assert!(artifacts[0].superseded);
        assert_eq!(artifacts[0].source_item, 1);
        assert!(!artifacts[1].superseded);
        assert_eq!(artifacts[1].source_item, 3);
    }

    #[test]
    fn structured_file_write_belongs_to_the_following_reply() {
        let items = vec![
            ChatItem::Tool {
                name: "write".into(),
                ok: Some(true),
                input: String::new(),
                output: "write completed".into(),
                started_at_ms: None,
                duration_ms: None,
                call_key: None,
                details: None,
            },
            ChatItem::FileChanged("result.csv".into()),
            ChatItem::Assistant {
                text: "Done.".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let artifacts = fresh(&items, Locale::En);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].source_item, 2);
    }

    #[test]
    fn structured_file_writes_skip_tool_call_commentary_for_the_final_reply() {
        let items = vec![
            ChatItem::Tool {
                name: "python".into(),
                ok: Some(true),
                input: "save comparison images".into(),
                output: "five images written".into(),
                started_at_ms: None,
                duration_ms: None,
                call_key: None,
                details: None,
            },
            ChatItem::FileChanged("evidence/one.png".into()),
            ChatItem::FileChanged("evidence/two.png".into()),
            ChatItem::FileChanged("evidence/three.png".into()),
            ChatItem::FileChanged("evidence/four.png".into()),
            ChatItem::FileChanged("evidence/five.png".into()),
            ChatItem::Assistant {
                text: "I will inspect the comparison images first.".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::Tool {
                name: "view_image".into(),
                ok: Some(true),
                input: "evidence/one.png".into(),
                output: "image is legible".into(),
                started_at_ms: None,
                duration_ms: None,
                call_key: None,
                details: None,
            },
            ChatItem::Assistant {
                text: "The five comparison images are ready.".into(),
                model: None,
                resources: Vec::new(),
            },
        ];

        assert!(is_commentary_at(&items, 6));
        let artifacts = fresh(&items, Locale::En);
        assert_eq!(artifacts.len(), 5);
        assert!(artifacts.iter().all(|artifact| artifact.source_item == 8));
    }

    #[test]
    fn paths_mentioned_by_tools_or_assistant_are_not_generated() {
        for name in ["shell", "read", "search", "grep"] {
            let items = vec![
                ChatItem::Tool {
                    name: name.into(),
                    ok: Some(true),
                    input: "inspect old-input.csv".into(),
                    output: "old.csv\nplots/old.png\nnotes/report.md".into(),
                    started_at_ms: None,
                    duration_ms: None,
                    call_key: None,
                    details: None,
                },
                ChatItem::Assistant {
                    text: "I inspected `old.csv`; no new file was created.".into(),
                    model: None,
                    resources: Vec::new(),
                },
            ];
            assert!(
                fresh(&items, Locale::En).is_empty(),
                "{name} output must not imply a file write"
            );
        }
    }

    #[test]
    fn file_write_does_not_cross_the_next_user_turn() {
        let items = vec![
            ChatItem::FileChanged("result.csv".into()),
            ChatItem::User("summarize a different PDF".into()),
            ChatItem::Assistant {
                text: "This answer is unrelated.".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let artifacts = fresh(&items, Locale::En);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].source_item, 0);
    }

    #[test]
    fn artifact_panel_deduplicates_message_versions_and_path_forms() {
        let items = vec![
            ChatItem::FileChanged("sample_statistics.png".into()),
            ChatItem::Assistant {
                text: "Created `sample_statistics.png`".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::FileChanged(r"E:\cross-species-root\sample_statistics.png".into()),
            ChatItem::Assistant {
                text: r"Updated `E:\cross-species-root\sample_statistics.png`".into(),
                model: None,
                resources: Vec::new(),
            },
        ];
        let all = fresh(&items, Locale::En);
        assert_eq!(all.len(), 2);

        let current = current_artifacts(&all, &[], r"E:\cross-species-root", &HashSet::new());
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].name, "sample_statistics.png");
        assert!(matches!(
            &current[0].data,
            PreviewData::File { path, .. }
                if path == r"E:\cross-species-root\sample_statistics.png"
        ));
    }

    fn registered(id: &str, name: &str, path: &str) -> ArtifactInfo {
        ArtifactInfo {
            id: id.into(),
            name: name.into(),
            kind: "table".into(),
            path: path.into(),
            ts: 0,
            project_id: None,
            project_name: None,
            session_id: None,
            session_title: None,
            size_bytes: None,
            origin: None,
        }
    }

    /// Harvested run outputs and uploads only exist as database rows. They have
    /// to reach the panel, but an upload is *also* named in the user turn, and
    /// the two spellings of its path (relative vs. absolute) must not produce
    /// two tiles.
    #[test]
    fn registered_artifacts_join_the_panel_without_duplicating_the_transcript() {
        let items = vec![ChatItem::User("Uploaded files: uploads/counts.csv".into())];
        let all = fresh(&items, Locale::En);
        assert_eq!(all.len(), 1);

        let current = current_artifacts(
            &all,
            &[
                registered("db1", "counts.csv", "/work/proj/uploads/counts.csv"),
                registered("db2", "deseq2.tsv", "/work/proj/results/deseq2.tsv"),
            ],
            "/work/proj",
            &HashSet::new(),
        );
        let names: Vec<&str> = current.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["counts.csv", "deseq2.tsv"]);
        assert_eq!(current[1].kind, "csv");
    }

    /// A remote reference has no local file, so the missing-file filter that
    /// guards path-scraped artifacts must not reach it.
    #[test]
    fn registered_remote_reference_survives_the_missing_file_filter() {
        let current = current_artifacts(
            &[],
            &[registered("db3", "out.bam", "ssh://cpu1/scratch/out.bam")],
            "/work/proj",
            &HashSet::from(["ssh://cpu1/scratch/out.bam".to_string()]),
        );
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].kind, "file");
    }
}
