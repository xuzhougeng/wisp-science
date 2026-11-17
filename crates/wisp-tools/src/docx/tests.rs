use super::review::{inspect_revisions, resolve_revisions};
use super::*;
use std::path::PathBuf;
use std::sync::Mutex;
use wisp_dto::{DocxChangeKind, DocxReviewAction, DocxReviewSelection};

const DATE: &str = "2026-11-05T00:00:00Z";

fn review_fixture() -> Vec<u8> {
    fixture(
        r#"<w:p><w:del w:id="1" w:author="Wisp" w:date="2026-11-05T00:00:00Z"><w:r><w:delText>old</w:delText></w:r></w:del><w:ins w:id="2" w:author="Wisp" w:date="2026-11-05T00:00:00Z"><w:r><w:t>new</w:t></w:r></w:ins></w:p><w:p><w:del w:id="3" w:author="Wisp"><w:r><w:delText>delete</w:delText></w:r></w:del></w:p><w:p><w:ins w:id="4" w:author="Human"><w:r><w:t>insert</w:t></w:r></w:ins></w:p><w:sdt><w:sdtContent><w:p><w:del w:id="5"><w:r><w:delText>control</w:delText></w:r></w:del></w:p></w:sdtContent></w:sdt>"#,
    )
}

#[test]
fn review_batches_keep_rewrite_pairs_and_other_kinds_intact() {
    let source = review_fixture();
    let view = inspect_revisions(&source).unwrap();
    assert_eq!(
        view.changes.iter().map(|c| c.kind).collect::<Vec<_>>(),
        [
            DocxChangeKind::Rewrite,
            DocxChangeKind::Deletion,
            DocxChangeKind::Insertion
        ]
    );
    assert_eq!(view.unsupported_count, 1);
    let output = resolve_revisions(
        &source,
        &view.source_sha256,
        &DocxReviewSelection::Kind(DocxChangeKind::Deletion),
        DocxReviewAction::Accept,
    )
    .unwrap();
    let remaining = inspect_revisions(&output).unwrap();
    assert_eq!(remaining.changes.len(), 2);
    assert_eq!(remaining.changes[0].kind, DocxChangeKind::Rewrite);
    assert_eq!(remaining.changes[0].old, "old");
    assert_eq!(remaining.changes[0].new, "new");
    assert_eq!(remaining.unsupported_count, 1);
    let output = resolve_revisions(
        &source,
        &view.source_sha256,
        &DocxReviewSelection::Kind(DocxChangeKind::Rewrite),
        DocxReviewAction::Accept,
    )
    .unwrap();
    assert_eq!(
        inspect_revisions(&output)
            .unwrap()
            .changes
            .iter()
            .map(|c| c.kind)
            .collect::<Vec<_>>(),
        [DocxChangeKind::Deletion, DocxChangeKind::Insertion]
    );
    let xml = Package::open(&output).unwrap().xml;
    assert!(!xml.contains(">old<"));
    assert!(xml.contains(">new<"));
    for (name, bytes) in entries(&source) {
        if name != MAIN {
            assert_eq!(entries(&output)[&name], bytes);
        }
    }
}

#[test]
fn review_individual_accept_and_reject_preserve_text_and_namespaces() {
    let source = fixture(r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>original</w:t></w:r></w:p>"#);
    let (suggested, _) = change(&source, "p1", "original", "replacement").unwrap();
    let view = inspect_revisions(&suggested).unwrap();
    let selection = DocxReviewSelection::Change(view.changes[0].id.clone());
    for (action, text) in [
        (DocxReviewAction::Accept, "replacement"),
        (DocxReviewAction::Reject, "original"),
    ] {
        let output =
            resolve_revisions(&suggested, &view.source_sha256, &selection, action).unwrap();
        let package = Package::open(&output).unwrap();
        let doc = parse(&package.xml).unwrap();
        assert_eq!(paragraph_text(doc.root_element()), text);
        assert_eq!(inspect_revisions(&output).unwrap().changes.len(), 0);
        assert!(!doc.descendants().any(|n| is_word(n, "delText")));
        assert!(doc.descendants().any(|n| is_word(n, "b")));
    }
    // Namespace bindings declared only on a revision must survive unwrapping.
    let source = fixture(
        r#"<w:p><w:del xmlns:x = "http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:id="1"><x:r xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:delText xml:space="preserve"> &amp; text </x:delText></x:r></w:del></w:p>"#,
    );
    let view = inspect_revisions(&source).unwrap();
    let output = resolve_revisions(
        &source,
        &view.source_sha256,
        &DocxReviewSelection::Change(view.changes[0].id.clone()),
        DocxReviewAction::Reject,
    )
    .unwrap();
    assert_eq!(
        paragraph_text(
            parse(&Package::open(&output).unwrap().xml)
                .unwrap()
                .root_element()
        ),
        " & text "
    );
}

#[test]
fn review_resolves_multiple_matches_and_standalone_accept_reject_semantics() {
    let body: String = (1..=2).map(|id| format!(
        r#"<w:p><w:r><w:t>keep</w:t></w:r><w:del w:id="{id}"><w:r><w:delText>delete{id}</w:delText></w:r></w:del></w:p>"#
    )).collect();
    let source = fixture(&body);
    for (action, expected) in [
        (DocxReviewAction::Accept, "keepkeep"),
        (DocxReviewAction::Reject, "keepdelete1keepdelete2"),
    ] {
        let output = resolve_revisions(
            &source,
            &sha256(&source),
            &DocxReviewSelection::Kind(DocxChangeKind::Deletion),
            action,
        )
        .unwrap();
        assert!(inspect_revisions(&output).unwrap().changes.is_empty());
        assert_eq!(
            paragraph_text(
                parse(&Package::open(&output).unwrap().xml)
                    .unwrap()
                    .root_element()
            ),
            expected
        );
    }
    let source = fixture(
        r#"<w:p><w:r><w:t>keep</w:t></w:r><w:ins w:id="1"><w:r><w:t>added</w:t></w:r></w:ins></w:p>"#,
    );
    let view = inspect_revisions(&source).unwrap();
    for (action, expected) in [
        (DocxReviewAction::Accept, "keepadded"),
        (DocxReviewAction::Reject, "keep"),
    ] {
        let output = resolve_revisions(
            &source,
            &view.source_sha256,
            &DocxReviewSelection::Change(view.changes[0].id.clone()),
            action,
        )
        .unwrap();
        assert!(inspect_revisions(&output).unwrap().changes.is_empty());
        assert_eq!(
            paragraph_text(
                parse(&Package::open(&output).unwrap().xml)
                    .unwrap()
                    .root_element()
            ),
            expected
        );
    }
}

#[test]
fn review_refuses_stale_or_unknown_selections_and_keeps_distinct_authors_separate() {
    let source = review_fixture();
    assert!(resolve_revisions(
        &source,
        "stale",
        &DocxReviewSelection::Kind(DocxChangeKind::Deletion),
        DocxReviewAction::Accept
    )
    .is_err());
    assert!(resolve_revisions(
        &source,
        &sha256(&source),
        &DocxReviewSelection::Change("missing".into()),
        DocxReviewAction::Reject
    )
    .is_err());
    let source = fixture(
        r#"<w:p><w:del w:id="1" w:author="A" w:date="2026-11-05T00:00:00Z"><w:r><w:delText>a</w:delText></w:r></w:del><w:ins w:id="2" w:author="B" w:date="2026-11-05T00:00:00Z"><w:r><w:t>b</w:t></w:r></w:ins></w:p>"#,
    );
    assert_eq!(
        inspect_revisions(&source)
            .unwrap()
            .changes
            .iter()
            .map(|c| c.kind)
            .collect::<Vec<_>>(),
        [DocxChangeKind::Deletion, DocxChangeKind::Insertion]
    );
}

fn fixture(body: &str) -> Vec<u8> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body>{body}<w:sectPr/></w:body></w:document>"#
    );
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.as_slice()),
        ("_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_slice()),
        (MAIN, xml.as_bytes()),
        ("word/media/image1.png", b"opaque-image-bytes".as_slice()),
        ("word/styles.xml", br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#.as_slice()),
    ] {
        zip.start_file(name, SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn change(bytes: &[u8], paragraph: &str, old: &str, new: &str) -> Result<(Vec<u8>, Value)> {
    propose(
        bytes,
        Edit {
            paragraph_id: paragraph,
            old,
            new,
            source_sha256: &sha256(bytes),
        },
        DATE,
    )
}

// Semantic reader independent of the patch algorithm: rejecting all text
// revisions must reproduce the original, accepting them the requested text.
fn text_view(xml: &str, accept: bool) -> String {
    let doc = parse(xml).unwrap();
    doc.descendants()
        .filter(|n| {
            let text = is_word(*n, "t") || is_word(*n, "delText");
            let excluded = n
                .ancestors()
                .any(|a| is_word(a, if accept { "del" } else { "ins" }));
            text && !excluded
        })
        .filter_map(|n| n.text())
        .collect()
}

fn entries(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut entry = zip.by_index(i).unwrap();
            let name = entry.name().to_string();
            let mut value = Vec::new();
            entry.read_to_end(&mut value).unwrap();
            (name, value)
        })
        .collect()
}

#[test]
fn revisions_round_trip_across_runs_and_preserve_other_package_parts() {
    let source = fixture(
        r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>原来</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>的结论。</w:t></w:r></w:p><w:p><w:r><w:t>不变。</w:t></w:r></w:p>"#,
    );
    let (output, summary) = change(&source, "p1", "原来的结论", "新的证据 & 限制").unwrap();
    let original_parts = entries(&source);
    let output_parts = entries(&output);
    assert_eq!(
        original_parts.keys().collect::<Vec<_>>(),
        output_parts.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &original_parts {
        if name != MAIN {
            assert_eq!(bytes, &output_parts[name], "{name}");
        }
    }
    let xml = std::str::from_utf8(&output_parts[MAIN]).unwrap();
    assert_eq!(text_view(xml, false), "原来的结论。不变。");
    assert_eq!(text_view(xml, true), "新的证据 & 限制。不变。");
    assert!(xml.contains(r#"<w:pPr><w:jc w:val="center"/></w:pPr>"#));
    assert!(xml.contains("<w:p><w:r><w:t>不变。</w:t></w:r></w:p>"));
    let doc = parse(xml).unwrap();
    let deletion = doc.descendants().find(|n| is_word(*n, "del")).unwrap();
    assert_eq!(deletion.children().filter(|n| is_word(*n, "r")).count(), 2);
    assert!(deletion.descendants().any(|n| is_word(n, "b")));
    assert!(deletion.descendants().any(|n| is_word(n, "i")));
    let insertion = doc.descendants().find(|n| is_word(*n, "ins")).unwrap();
    assert!(insertion.descendants().any(|n| is_word(n, "b")));
    assert_eq!(insertion.attribute((W, "author")), Some("Wisp"));
    assert_eq!(insertion.attribute((W, "date")), Some(DATE));
    assert_ne!(
        insertion.attribute((W, "id")),
        deletion.attribute((W, "id"))
    );
    assert_eq!(summary["status"], "pending_review");
    assert_eq!(summary["output_sha256"], sha256(&output));
}

#[test]
fn minimal_unicode_insert_delete_and_replace_have_correct_original_and_final_views() {
    for (before, old, new, after, kinds) in [
        (
            "研究有效。",
            "有效",
            "可能有效",
            "研究可能有效。",
            vec!["ins"],
        ),
        ("结果🙂", "结果🙂", "结果🙂！", "结果🙂！", vec!["ins"]),
        ("非常有效", "非常有效", "有效", "有效", vec!["del"]),
        ("全删", "全删", "", "", vec!["del"]),
        ("a & b", "&", "<证据>", "a <证据> b", vec!["del", "ins"]),
    ] {
        let source = fixture(&format!(
            "<w:p><w:r><w:t>{}</w:t></w:r></w:p>",
            escape(before)
        ));
        let (output, summary) = change(&source, "p1", old, new).unwrap();
        let package = Package::open(&output).unwrap();
        assert_eq!(text_view(&package.xml, false), before);
        assert_eq!(text_view(&package.xml, true), after);
        let actual: Vec<_> = summary["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["kind"].as_str().unwrap())
            .collect();
        assert_eq!(actual, kinds);
    }
}

#[test]
fn multiple_text_nodes_run_boundaries_and_unusual_namespace_prefixes() {
    let source = fixture(
        r#"<w:p><x:r xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:t>A</x:t><x:t>🙂</x:t></x:r><w:r><w:t>尾</w:t></w:r></w:p>"#,
    );
    let (output, _) = change(&source, "p1", "🙂尾", "🙂新尾").unwrap();
    let package = Package::open(&output).unwrap();
    assert_eq!(text_view(&package.xml, false), "A🙂尾");
    assert_eq!(text_view(&package.xml, true), "A🙂新尾");
    assert!(package.xml.contains(r#"<x:r xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:t>A</x:t><x:t>🙂</x:t></x:r>"#));
}

#[test]
fn repeated_text_requires_a_unique_match_within_the_selected_paragraph() {
    let overlapping = fixture("<w:p><w:r><w:t>aaa</w:t></w:r></w:p>");
    assert!(change(&overlapping, "p1", "aa", "b").is_err());
    let source =
        fixture("<w:p><w:r><w:t>word word</w:t></w:r></w:p><w:p><w:r><w:t>word</w:t></w:r></w:p>");
    assert!(change(&source, "p1", "word", "new")
        .unwrap_err()
        .contains("exactly once"));
    assert!(change(&source, "p2", "word", "new").is_ok());
    for (paragraph, old, new) in [
        ("p0", "word", "new"),
        ("p99", "word", "new"),
        ("p2", "absent", "new"),
        ("p2", "word", "word"),
        ("p2", "", "new"),
        ("p2", "word", "two\nlines"),
        ("p2", "word", "bad\0text"),
    ] {
        assert!(change(&source, paragraph, old, new).is_err());
    }
}

#[test]
fn stale_snapshot_is_rejected_even_if_paragraph_text_is_unchanged() {
    let source = fixture("<w:p><w:r><w:t>text</w:t></w:r></w:p>");
    let other =
        fixture("<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:t>text</w:t></w:r></w:p>");
    let result = propose(
        &other,
        Edit {
            paragraph_id: "p1",
            old: "text",
            new: "new",
            source_sha256: &sha256(&source),
        },
        DATE,
    );
    assert!(result.unwrap_err().contains("changed"));
}

#[test]
fn complex_paragraphs_are_readable_but_cannot_be_changed() {
    for body in [
        r#"<w:p><w:hyperlink><w:r><w:t>text</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:ins w:id="9" w:author="Human"><w:r><w:t>text</w:t></w:r></w:ins></w:p>"#,
        r#"<w:p><w:r><w:drawing/><w:t>text</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:footnoteReference w:id="1"/><w:t>text</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:rPr><w:rPrChange w:id="1"/></w:rPr><w:t>text</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:sectPr/></w:pPr><w:r><w:t>text</w:t></w:r></w:p>"#,
        r#"<w:sdt><w:sdtContent><w:p><w:r><w:t>text</w:t></w:r></w:p></w:sdtContent></w:sdt>"#,
        r#"<w:tbl><w:tr><w:trPr><w:ins w:id="1" w:author="A"/></w:trPr><w:tc><w:p><w:r><w:t>text</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
    ] {
        let source = fixture(body);
        let view = inspect(&source, 0, 10).unwrap();
        assert_eq!(view["paragraphs"][0]["editable"], false, "{body}");
        assert_eq!(view["paragraphs"][0]["text"], "text");
        assert!(change(&source, "p1", "text", "new").is_err());
    }
}

// p1 body, p2-p3 row 1, p4 cell with a nested table (p5) and a trailing empty
// paragraph (p6), p7 a cell with a single paragraph, p8 body.
const TABLE: &str = r#"<w:p><w:r><w:t>Intro</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="0" w:type="auto"/></w:tcPr><w:p><w:r><w:t>Reagent</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>10 mM</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>NaCl</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/></w:tc><w:tc><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>remove me</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>Outro</w:t></w:r></w:p>"#;

fn paragraph_texts(bytes: &[u8]) -> Vec<String> {
    let xml = Package::open(bytes).unwrap().xml;
    let doc = parse(&xml).unwrap();
    check_cells(&doc).unwrap();
    doc.descendants()
        .filter(|n| is_word(*n, "p"))
        .map(paragraph_text)
        .collect()
}

#[test]
fn table_cell_paragraphs_are_editable_with_unchanged_ids_and_locations() {
    let source = fixture(TABLE);
    let view = inspect(&source, 0, 100).unwrap();
    let rows = view["paragraphs"].as_array().unwrap();
    assert_eq!(view["total_paragraphs"], 8);
    let expected = [
        ("p1", "Intro", "body"),
        ("p2", "Reagent", "table 1 row 1 cell 1"),
        ("p3", "10 mM", "table 1 row 1 cell 2"),
        ("p4", "NaCl", "table 1 row 2 cell 1"),
        (
            "p5",
            "nested",
            "table 1 row 2 cell 1 > table 1 row 1 cell 1",
        ),
        ("p6", "", "table 1 row 2 cell 1"),
        ("p7", "remove me", "table 1 row 2 cell 2"),
        ("p8", "Outro", "body"),
    ];
    for (row, (id, text, location)) in rows.iter().zip(expected) {
        assert_eq!(row["paragraph_id"], id);
        assert_eq!(row["text"], text);
        assert_eq!(row["location"], location, "{id}");
        assert_eq!(row["editable"], true, "{id}: {}", row["reason"]);
    }
    // Body paragraph IDs still count table paragraphs in document order.
    assert!(change(&source, "p8", "Outro", "Summary").is_ok());
    assert!(change(&source, "p2", "Outro", "Summary").is_err());
}

#[test]
fn table_cell_edits_write_tracked_changes_with_cell_formatting_and_review_them() {
    let source = fixture(TABLE);
    for (paragraph, depth, old, new, before, after) in [
        ("p3", 1, "10 mM", "20 mM", "10 mM", "20 mM"),
        ("p5", 2, "nested", "nested table", "nested", "nested table"),
    ] {
        let (output, _) = change(&source, paragraph, old, new).unwrap();
        let xml = Package::open(&output).unwrap().xml;
        assert_eq!(
            text_view(&xml, false),
            text_view(&Package::open(&source).unwrap().xml, false)
        );
        assert!(text_view(&xml, true).contains(after));
        let doc = parse(&xml).unwrap();
        let insertion = doc.descendants().find(|n| is_word(*n, "ins")).unwrap();
        assert_eq!(
            table_path(insertion.parent().unwrap()).unwrap().len(),
            depth
        );
        if paragraph == "p3" {
            assert!(insertion.descendants().any(|n| is_word(n, "b")));
        }
        // Reading the output keeps every ID and marks only the edited cell read-only.
        let reread = inspect(&output, 0, 100).unwrap();
        for row in reread["paragraphs"].as_array().unwrap() {
            assert_eq!(row["editable"], row["paragraph_id"] != paragraph);
        }
        let view = inspect_revisions(&output).unwrap();
        assert_eq!(view.unsupported_count, 0);
        assert_eq!(view.changes.len(), 1);
        assert_eq!(view.changes[0].paragraph_id, paragraph);
        let selection = DocxReviewSelection::Change(view.changes[0].id.clone());
        for (action, text) in [
            (DocxReviewAction::Accept, after),
            (DocxReviewAction::Reject, before),
        ] {
            let resolved =
                resolve_revisions(&output, &view.source_sha256, &selection, action).unwrap();
            assert!(inspect_revisions(&resolved).unwrap().changes.is_empty());
            let index = paragraph[1..].parse::<usize>().unwrap() - 1;
            assert_eq!(paragraph_texts(&resolved)[index], text);
            assert_eq!(paragraph_texts(&resolved).len(), 8);
        }
    }
}

#[test]
fn deleting_a_cells_only_paragraph_text_keeps_the_paragraph() {
    let source = fixture(TABLE);
    let (output, summary) = change(&source, "p7", "remove me", "").unwrap();
    assert_eq!(summary["revisions"][0]["kind"], "del");
    let view = inspect_revisions(&output).unwrap();
    assert_eq!(view.changes[0].kind, DocxChangeKind::Deletion);
    assert_eq!(view.changes[0].paragraph_id, "p7");
    for selection in [
        DocxReviewSelection::Change(view.changes[0].id.clone()),
        DocxReviewSelection::Kind(DocxChangeKind::Deletion),
    ] {
        let accepted = resolve_revisions(
            &output,
            &view.source_sha256,
            &selection,
            DocxReviewAction::Accept,
        )
        .unwrap();
        // The cell keeps its paragraph and paragraph properties, now empty.
        let texts = paragraph_texts(&accepted);
        assert_eq!(texts.len(), 8);
        assert_eq!(texts[6], "");
        let xml = Package::open(&accepted).unwrap().xml;
        assert!(xml.contains(r#"<w:tc><w:p><w:pPr><w:jc w:val="center"/></w:pPr></w:p></w:tc>"#));
        let reread = inspect(&accepted, 6, 1).unwrap();
        assert_eq!(reread["paragraphs"][0]["location"], "table 1 row 2 cell 2");
        assert_eq!(reread["paragraphs"][0]["editable"], true);
    }
    let rejected = resolve_revisions(
        &output,
        &view.source_sha256,
        &DocxReviewSelection::Change(view.changes[0].id.clone()),
        DocxReviewAction::Reject,
    )
    .unwrap();
    assert_eq!(paragraph_texts(&rejected)[6], "remove me");
    // The written-package guard refuses a cell without any paragraph.
    let empty = format!(
        r#"<w:document xmlns:w="{W}"><w:body><w:tbl><w:tr><w:tc><w:tcPr/></w:tc></w:tr></w:tbl></w:body></w:document>"#
    );
    assert!(check_cells(&parse(&empty).unwrap()).is_err());
}

#[test]
fn accept_all_resolves_body_and_table_revisions_by_kind() {
    let source = fixture(TABLE);
    let (step, _) = change(&source, "p3", "10 mM", "20 mM").unwrap();
    let (step, _) = change(&step, "p7", "remove me", "").unwrap();
    let (step, _) = change(&step, "p1", "Intro", "Introduction").unwrap();
    let view = inspect_revisions(&step).unwrap();
    assert_eq!(
        view.changes
            .iter()
            .map(|c| (c.paragraph_id.as_str(), c.kind))
            .collect::<Vec<_>>(),
        [
            ("p1", DocxChangeKind::Insertion),
            ("p3", DocxChangeKind::Rewrite),
            ("p7", DocxChangeKind::Deletion)
        ]
    );
    let rewrites = resolve_revisions(
        &step,
        &view.source_sha256,
        &DocxReviewSelection::Kind(DocxChangeKind::Rewrite),
        DocxReviewAction::Accept,
    )
    .unwrap();
    assert_eq!(paragraph_texts(&rewrites)[2], "20 mM");
    assert_eq!(
        inspect_revisions(&rewrites)
            .unwrap()
            .changes
            .iter()
            .map(|c| (c.paragraph_id.as_str(), c.kind))
            .collect::<Vec<_>>(),
        [
            ("p1", DocxChangeKind::Insertion),
            ("p7", DocxChangeKind::Deletion)
        ]
    );
    let deletions = resolve_revisions(
        &rewrites,
        &sha256(&rewrites),
        &DocxReviewSelection::Kind(DocxChangeKind::Deletion),
        DocxReviewAction::Accept,
    )
    .unwrap();
    let remaining = inspect_revisions(&deletions).unwrap();
    assert_eq!(remaining.changes.len(), 1);
    assert_eq!(remaining.changes[0].kind, DocxChangeKind::Insertion);
    let texts = paragraph_texts(&deletions);
    assert_eq!(texts.len(), 8);
    assert_eq!(texts[2], "20 mM");
    assert_eq!(texts[6], "");
}

#[test]
fn structural_table_revisions_and_other_containers_stay_refused() {
    let cell = r#"<w:p><w:r><w:t>text</w:t></w:r><w:del w:id="7" w:author="A"><w:r><w:delText>gone</w:delText></w:r></w:del></w:p>"#;
    for body in [
        format!(
            r#"<w:tbl><w:tr><w:trPr><w:ins w:id="1" w:author="A"/></w:trPr><w:tc>{cell}</w:tc></w:tr></w:tbl>"#
        ),
        format!(
            r#"<w:tbl><w:tr><w:trPr><w:del w:id="1" w:author="A"/></w:trPr><w:tc>{cell}</w:tc></w:tr></w:tbl>"#
        ),
        format!(
            r#"<w:tbl><w:tr><w:tc><w:tcPr><w:cellIns w:id="1" w:author="A"/></w:tcPr>{cell}</w:tc></w:tr></w:tbl>"#
        ),
        format!(
            r#"<w:tbl><w:tr><w:tc><w:tcPr><w:cellDel w:id="1" w:author="A"/></w:tcPr>{cell}</w:tc></w:tr></w:tbl>"#
        ),
        format!(
            r#"<w:tbl><w:tr><w:tc><w:tcPr><w:cellMerge w:id="1" w:author="A" w:vMerge="cont"/></w:tcPr>{cell}</w:tc></w:tr></w:tbl>"#
        ),
        format!(
            r#"<w:tbl><w:tr><w:tc><w:tcPr><w:tcPrChange w:id="1" w:author="A"><w:tcPr/></w:tcPrChange></w:tcPr>{cell}</w:tc></w:tr></w:tbl>"#
        ),
        format!(
            r#"<w:tbl><w:tblPr><w:tblPrChange w:id="1" w:author="A"><w:tblPr/></w:tblPrChange></w:tblPr><w:tr><w:tc>{cell}</w:tc></w:tr></w:tbl>"#
        ),
        // A structural change on the outer row also covers its nested tables.
        format!(
            r#"<w:tbl><w:tr><w:trPr><w:ins w:id="1" w:author="A"/></w:trPr><w:tc><w:tbl><w:tr><w:tc>{cell}</w:tc></w:tr></w:tbl><w:p/></w:tc></w:tr></w:tbl>"#
        ),
    ] {
        let source = fixture(&body);
        let view = inspect(&source, 0, 10).unwrap();
        assert_eq!(view["paragraphs"][0]["editable"], false, "{body}");
        assert!(view["paragraphs"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("structural"));
        assert!(change(&source, "p1", "text", "new").is_err());
        let review = inspect_revisions(&source).unwrap();
        assert!(review.changes.is_empty(), "{body}");
        assert!(review.unsupported_count >= 2, "{body}");
        assert!(resolve_revisions(
            &source,
            &review.source_sha256,
            &DocxReviewSelection::Kind(DocxChangeKind::Deletion),
            DocxReviewAction::Accept,
        )
        .is_err());
    }
    for body in [
        // Text boxes, including a table inside one, and content controls.
        r#"<w:p><w:r><w:pict><w:txbxContent><w:p><w:r><w:t>text</w:t></w:r></w:p></w:txbxContent></w:pict></w:r></w:p>"#,
        r#"<w:p><w:r><w:pict><w:txbxContent><w:tbl><w:tr><w:tc><w:p><w:r><w:t>text</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:txbxContent></w:pict></w:r></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:sdt><w:sdtContent><w:p><w:r><w:t>text</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p/></w:tc></w:tr></w:tbl>"#,
    ] {
        let source = fixture(body);
        let view = inspect(&source, 0, 10).unwrap();
        // The anchoring body paragraph also reports the box text; take the inner one.
        let row = view["paragraphs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["text"] == "text")
            .last()
            .unwrap();
        assert_eq!(row["editable"], false, "{body}");
        assert_eq!(row["location"], Value::Null, "{body}");
        let id = row["paragraph_id"].as_str().unwrap();
        assert!(change(&source, id, "text", "new").is_err());
    }
}

#[test]
fn cross_paragraph_fields_and_annotations_protect_their_interior() {
    for (start, end) in [
        (
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r>"#,
            r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        ),
        (
            r#"<w:bookmarkStart w:id="1" w:name="cite"/>"#,
            r#"<w:bookmarkEnd w:id="1"/>"#,
        ),
        (
            r#"<w:commentRangeStart w:id="1"/>"#,
            r#"<w:commentRangeEnd w:id="1"/>"#,
        ),
    ] {
        let source = fixture(&format!("<w:p>{start}</w:p><w:p><w:r><w:t>inside</w:t></w:r></w:p><w:p>{end}</w:p><w:p><w:r><w:t>outside</w:t></w:r></w:p>"));
        assert!(change(&source, "p2", "inside", "new").is_err());
        assert!(change(&source, "p4", "outside", "new").is_ok());
    }
}

#[test]
fn existing_revisions_elsewhere_are_preserved_and_new_ids_do_not_collide() {
    let source = fixture(
        r#"<w:p><w:ins w:id="42" w:author="Human"><w:r><w:t>prior</w:t></w:r></w:ins></w:p><w:p><w:r><w:t>text</w:t></w:r></w:p>"#,
    );
    let (output, summary) = change(&source, "p2", "text", "new").unwrap();
    assert!(Package::open(&output)
        .unwrap()
        .xml
        .contains(r#"<w:ins w:id="42" w:author="Human"><w:r><w:t>prior</w:t></w:r></w:ins>"#));
    assert!(summary["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["id"].as_u64().unwrap() > 42));
    assert_eq!(
        inspect(&output, 1, 1).unwrap()["paragraphs"][0]["editable"],
        false
    );
}

#[test]
fn read_pages_have_version_bound_ids_and_report_more() {
    let source =
        fixture("<w:p><w:r><w:t>first</w:t></w:r></w:p><w:p><w:r><w:t>second</w:t></w:r></w:p>");
    let view = inspect(&source, 0, 1).unwrap();
    assert_eq!(view["source_sha256"], sha256(&source));
    assert_eq!(view["paragraphs"][0]["paragraph_id"], "p1");
    assert_eq!(view["next_offset"], 1);
    let view = inspect(&source, 1, 1).unwrap();
    assert_eq!(view["paragraphs"][0]["paragraph_id"], "p2");
    assert_eq!(view["next_offset"], Value::Null);
}

#[test]
fn read_view_omits_move_sources_and_edits_do_not_drop_empty_runs() {
    let source = fixture(
        r#"<w:p><w:moveFrom w:id="3"><w:r><w:t>moved</w:t></w:r></w:moveFrom><w:moveTo w:id="4"><w:r><w:t>moved</w:t></w:r></w:moveTo></w:p>"#,
    );
    let view = inspect(&source, 0, 10).unwrap();
    assert_eq!(view["paragraphs"][0]["text"], "moved");
    assert_eq!(view["paragraphs"][0]["editable"], false);
    let source = fixture(
        r#"<w:p><w:r><w:t>first</w:t></w:r><w:r><w:rPr><w:b/></w:rPr></w:r><w:r><w:t>second</w:t></w:r></w:p>"#,
    );
    assert!(change(&source, "p1", "firstsecond", "replaced")
        .unwrap_err()
        .contains("empty formatting run"));
}

#[test]
fn rejects_malformed_archives_unsafe_paths_and_dtds() {
    assert!(Package::open(b"not a zip").is_err());
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("../escape", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"no").unwrap();
    assert!(Package::open(&zip.finish().unwrap().into_inner()).is_err());
    assert!(parse("<!DOCTYPE x [<!ENTITY y 'boom'>]><x>&y;</x>").is_err());
    let source = fixture(&format!(
        "<w:p><w:r><w:t>{}</w:t></w:r></w:p>",
        "x".repeat(MAX_XML as usize)
    ));
    assert!(Package::open(&source).is_err());
}

struct TestEnv {
    root: PathBuf,
    events: Mutex<Vec<ToolEvent>>,
    planning: bool,
    cancelled: bool,
}

impl TestEnv {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "wisp-docx-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self {
            root,
            events: Mutex::new(Vec::new()),
            planning: false,
            cancelled: false,
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[async_trait]
impl ToolEnv for TestEnv {
    fn project_root(&self) -> &Path {
        &self.root
    }
    fn restrict_read_paths_to_project(&self) -> bool {
        true
    }
    fn plan_mode(&self) -> bool {
        self.planning
    }
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    async fn confirm(&self, _: &str) -> bool {
        true
    }
    async fn emit(&self, event: ToolEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn args(source: &[u8]) -> Value {
    json!({"path":"source.docx", "output_path":"review.docx", "source_sha256":sha256(source),
        "paragraph_id":"p1", "old":"text", "new":"revised"})
}

#[tokio::test]
async fn registered_tools_create_a_review_copy_never_overwrite_and_emit_after_success() {
    let env = TestEnv::new();
    let source = fixture("<w:p><w:r><w:t>text</w:t></w:r></w:p>");
    std::fs::write(env.root.join("source.docx"), &source).unwrap();
    let registry = crate::Registry::builtins();
    let read = registry
        .run("read_docx", &json!({"path":"source.docx"}), &env)
        .await;
    assert!(read.success, "{}", read.content);
    let result = registry
        .run("suggest_docx_edit", &args(&source), &env)
        .await;
    assert!(result.success, "{}", result.content);
    assert_eq!(std::fs::read(env.root.join("source.docx")).unwrap(), source);
    let review = std::fs::read(env.root.join("review.docx")).unwrap();
    assert_eq!(
        text_view(&Package::open(&review).unwrap().xml, true),
        "revised"
    );
    assert!(
        !registry
            .run("suggest_docx_edit", &args(&source), &env)
            .await
            .success
    );
    let mut same = args(&source);
    same["output_path"] = json!("source.docx");
    assert!(!registry.run("suggest_docx_edit", &same, &env).await.success);
    assert_eq!(std::fs::read(env.root.join("review.docx")).unwrap(), review);
    assert_eq!(std::fs::read(env.root.join("source.docx")).unwrap(), source);
    assert_eq!(
        env.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, ToolEvent::FileChanged { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn plan_mode_cancellation_and_invalid_paths_do_not_create_output() {
    let mut env = TestEnv::new();
    let source = fixture("<w:p><w:r><w:t>text</w:t></w:r></w:p>");
    std::fs::write(env.root.join("source.docx"), &source).unwrap();
    let registry = crate::Registry::builtins();
    env.planning = true;
    assert!(
        registry
            .run("read_docx", &json!({"path":"source.docx"}), &env)
            .await
            .success
    );
    assert!(
        !registry
            .run("suggest_docx_edit", &args(&source), &env)
            .await
            .success
    );
    env.planning = false;
    env.cancelled = true;
    assert!(
        !registry
            .run("suggest_docx_edit", &args(&source), &env)
            .await
            .success
    );
    env.cancelled = false;
    for path in ["../outside.docx", "missing/review.docx", "review.txt"] {
        let mut value = args(&source);
        value["output_path"] = json!(path);
        assert!(
            !registry
                .run("suggest_docx_edit", &value, &env)
                .await
                .success
        );
    }
    assert!(
        !registry
            .run("read_docx", &json!({"path":"../outside.docx"}), &env)
            .await
            .success
    );
    assert!(
        !registry
            .run(
                "read_docx",
                &json!({"path":"source.docx", "offset":-1}),
                &env
            )
            .await
            .success
    );
    assert!(!env.root.join("review.docx").exists());
    assert!(!env
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, ToolEvent::FileChanged { .. })));
}

#[cfg(unix)]
#[tokio::test]
async fn output_symlinks_are_not_followed() {
    use std::os::unix::fs::symlink;
    let env = TestEnv::new();
    let source = fixture("<w:p><w:r><w:t>text</w:t></w:r></w:p>");
    std::fs::write(env.root.join("source.docx"), &source).unwrap();
    symlink(env.root.join("source.docx"), env.root.join("review.docx")).unwrap();
    assert!(!SuggestDocxEditTool.run(&args(&source), &env).await.success);
    assert_eq!(std::fs::read(env.root.join("source.docx")).unwrap(), source);
    std::fs::remove_file(env.root.join("review.docx")).unwrap();
    symlink(env.root.join("missing.docx"), env.root.join("review.docx")).unwrap();
    assert!(!SuggestDocxEditTool.run(&args(&source), &env).await.success);
    assert!(!env.root.join("missing.docx").exists());
}
