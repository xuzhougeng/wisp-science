use super::*;

/// Text/code/CSV UI previews only pull a head — never 32 MiB into the WebView.
const TEXT_PREVIEW_MAX_BYTES: u64 = 1024 * 1024;

/// Even after a byte cap, pathological 1-byte lines would thrash the gutter.
const TEXT_PREVIEW_MAX_LINES: usize = 8_000;

pub(crate) fn table_view(table: &TableData, locale: Locale) -> impl IntoView {
    let total = table.rows.len();
    let truncated = total > 500;
    let copy = table_data_to_tsv(table);
    let headers: Vec<String> = table.headers.iter().map(|h| md_inline_to_html(h)).collect();
    let rows: Vec<Vec<String>> = table
        .rows
        .iter()
        .take(500)
        .map(|r| r.iter().map(|c| md_inline_to_html(c)).collect())
        .collect();
    view! {
        <div class="tbl-card">
            <div class="tbl-head">
                {truncated.then(|| view! {
                    <span class="tbl-note">{tf(locale, "table.rows_note", &[("total", &total.to_string())])}</span>
                })}
                <button type="button" class="tbl-copy" on:click=move |_| copy_text(copy.clone())>
                    {move || crate::i18n::t(locale, "table.copy")}
                </button>
            </div>
            <div class="tbl-wrap">
                <table class="tbl">
                    <thead><tr>{headers.into_iter().map(|h| view! { <th inner_html=h></th> }).collect_view()}</tr></thead>
                    <tbody>
                        {rows.into_iter().map(|r| view! {
                            <tr>{r.into_iter().map(|c| view! { <td inner_html=c></td> }).collect_view()}</tr>
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
        </div>
    }
}

pub(crate) fn artifact_group_label(key: &str, locale: Locale) -> String {
    if let Some(kind) = key.strip_prefix('@') {
        let i18n = match kind {
            "table" => "artifact.group.table",
            "latex" => "artifact.group.latex",
            "csv" => "artifact.group.csv",
            "fasta" => "artifact.group.fasta",
            "msa" => "artifact.group.msa",
            "text" | "markdown" | "code" | "notebook" => "artifact.group.text",
            _ => return kind.to_string(),
        };
        t(locale, i18n).into()
    } else if key == "." {
        t(locale, "artifact.group.root").into()
    } else {
        key.to_string()
    }
}

pub(crate) fn artifact_meta(a: &Artifact, locale: Locale) -> String {
    match &a.data {
        PreviewData::Table(t) => tf(
            locale,
            "artifact.meta.table",
            &[
                ("rows", &t.rows.len().to_string()),
                ("cols", &t.headers.len().to_string()),
            ],
        ),
        PreviewData::File { path, kind } => {
            let path = a.location.as_deref().unwrap_or(path);
            let visible_path = a.location.as_deref().unwrap_or(path);
            if kind == "fasta" {
                t(locale, "artifact.kind.fasta").into()
            } else if kind == "msa" {
                t(locale, "artifact.kind.msa").into()
            } else if let Some(parent) = visible_path.rsplit(['/', '\\']).nth(1) {
                if parent.is_empty() {
                    tf(locale, "artifact.meta.file", &[("kind", kind)])
                } else {
                    format!("{parent}/")
                }
            } else {
                tf(locale, "artifact.meta.file", &[("kind", kind)])
            }
        }
        PreviewData::Latex { .. } => t(locale, "artifact.latex").into(),
        PreviewData::Fasta(s) => tf(
            locale,
            "artifact.meta.fasta",
            &[("seqs", &fasta_seq_count(s.as_ref()).max(1).to_string())],
        ),
    }
}

#[component]
pub(crate) fn HeavyPreview(dom_id: String, kind: String, payload: String) -> impl IntoView {
    let id_for_effect = dom_id.clone();
    let kind_for_effect = kind.clone();
    let payload_for_effect = payload.clone();
    create_effect(move |_| {
        let dom_id = id_for_effect.clone();
        let kind = kind_for_effect.clone();
        let payload = payload_for_effect.clone();
        spawn_local(async move {
            let _ = mount_preview(&kind, &dom_id, &payload).await;
        });
    });
    view! { <div class="rp-heavy" id=dom_id></div> }
}

pub(crate) fn parse_csv_text(text: &str) -> Option<TableData> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let headers = parse_csv_line(lines[0]);
    let rows: Vec<Vec<String>> = lines[1..].iter().map(|l| parse_csv_line(l)).collect();
    Some(TableData { headers, rows })
}

pub(crate) fn artifact_id_path(path: &str) -> Option<&str> {
    path.strip_prefix("artifact:").filter(|id| !id.is_empty())
}

pub(crate) fn artifact_version_id_path(path: &str) -> Option<&str> {
    path.strip_prefix("artifact-version:")
        .filter(|id| !id.is_empty())
}

/// The remote-preview path spelling: `remote:ssh:<alias>:<path>`. Returns the
/// execution-context id (`ssh:<alias>`) and the path on that host. SSH aliases
/// never contain `:` (see `remote_file_download_uri`), so the split after the
/// alias is unambiguous even though remote paths may contain colons.
pub(crate) fn remote_file_path(path: &str) -> Option<(&str, &str)> {
    let ctx_and_path = path.strip_prefix("remote:")?;
    let after_kind = ctx_and_path.strip_prefix("ssh:")?;
    let alias_end = after_kind.find(':')?;
    let remote_path = &after_kind[alias_end + 1..];
    (alias_end > 0 && !remote_path.is_empty())
        .then(|| (&ctx_and_path[.."ssh:".len() + alias_end], remote_path))
}

#[cfg(test)]
mod remote_file_path_tests {
    use super::remote_file_path;

    #[test]
    fn splits_context_id_from_remote_path() {
        assert_eq!(
            remote_file_path("remote:ssh:gpu-server:/home/research/report.html"),
            Some(("ssh:gpu-server", "/home/research/report.html"))
        );
        assert_eq!(
            remote_file_path("remote:ssh:gpu:~/analysis.ipynb"),
            Some(("ssh:gpu", "~/analysis.ipynb"))
        );
        // Colons inside the remote path stay with the path.
        assert_eq!(
            remote_file_path("remote:ssh:gpu:/data/a:b.py"),
            Some(("ssh:gpu", "/data/a:b.py"))
        );
    }

    #[test]
    fn rejects_other_spellings() {
        assert_eq!(remote_file_path("reviews/notes.md"), None);
        assert_eq!(remote_file_path("artifact:abc"), None);
        assert_eq!(remote_file_path("remote:ssh:gpu:"), None);
        assert_eq!(remote_file_path("remote:ssh::/x"), None);
        assert_eq!(remote_file_path("remote:local:/x"), None);
    }
}

/// Read a workspace file, a remote (SSH) file, an artifact, or a pinned
/// artifact version — the four path spellings a preview can be handed — into
/// its `FileContent`. All previews route through here so every kind (html,
/// notebook, code, image, …) works for every spelling.
///
/// `max_bytes` is a soft head budget for text-ish files (backend truncates
/// instead of rejecting). Binary paths still hard-fail above the higher
/// full-file ceiling used by PDF/office.
pub(crate) async fn load_file_content(
    path: &str,
    loc: Locale,
    max_bytes: Option<u64>,
) -> Result<FileContent, String> {
    let result = if let Some((context_id, remote_path)) = remote_file_path(path) {
        let mut args = serde_json::json!({ "contextId": context_id, "path": remote_path });
        if let Some(n) = max_bytes {
            args["maxBytes"] = serde_json::json!(n);
        }
        invoke_checked("read_remote_file", to_value(&args).unwrap()).await
    } else if let Some(version_id) = artifact_version_id_path(path) {
        invoke_checked(
            "read_artifact_version",
            to_value(&serde_json::json!({ "versionId": version_id })).unwrap(),
        )
        .await
    } else if let Some(id) = artifact_id_path(path) {
        invoke_checked(
            "read_artifact",
            to_value(&serde_json::json!({ "id": id })).unwrap(),
        )
        .await
    } else {
        invoke_checked(
            "read_file",
            to_value(&tauri_args::read_file(path, max_bytes)).unwrap(),
        )
        .await
    };
    match result {
        Ok(v) => serde_wasm_bindgen::from_value::<FileContent>(v)
            .map_err(|_| tf(loc, "err.file_not_found", &[("path", path)])),
        Err(err_value) => Err(localize_backend(loc, &js_error_text(err_value))),
    }
}

fn preview_truncation_note(fc: &FileContent, shown_len: usize, locale: Locale) -> Option<String> {
    let total = fc.total_bytes.unwrap_or(shown_len as u64);
    let from_backend = fc.truncated;
    let from_lines = fc.text.as_ref().is_some_and(|t| shown_len < t.len());
    if !from_backend && !from_lines {
        return None;
    }
    Some(tf(
        locale,
        "preview.text_truncated",
        &[
            ("shown", &format_bytes(shown_len as u64)),
            ("total", &format_bytes(total.max(shown_len as u64))),
        ],
    ))
}

/// Cap rendered lines so a 1 MiB file of one-char lines cannot paint 1M gutters.
fn clip_preview_text(text: &str) -> (String, usize) {
    let mut lines = 0usize;
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            lines += 1;
            if lines >= TEXT_PREVIEW_MAX_LINES {
                let end = i + ch.len_utf8();
                return (text[..end].to_string(), end);
            }
        }
    }
    (text.to_string(), text.len())
}

fn text_preview_banner(note: Option<String>) -> View {
    match note {
        Some(message) => view! { <div class="preview-trunc-note">{message}</div> }.into_view(),
        None => view! { <></> }.into_view(),
    }
}

#[component]
pub(crate) fn CsvFilePreview(path: String) -> impl IntoView {
    let locale = use_locale();
    let table = create_rw_signal::<Option<TableData>>(None);
    let note = create_rw_signal::<Option<String>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            table.set(None);
            note.set(None);
            err.set(None);
            let fc = match load_file_content(&path, loc, Some(TEXT_PREVIEW_MAX_BYTES)).await {
                Ok(fc) => fc,
                Err(e) => {
                    err.set(Some(e));
                    return;
                }
            };
            let shown = fc.text.as_ref().map(|t| t.len()).unwrap_or(0);
            note.set(preview_truncation_note(&fc, shown, loc));
            match fc.text.as_deref().and_then(parse_csv_text) {
                Some(t) => table.set(Some(t)),
                None => err.set(Some(tf(loc, "err.file_not_found", &[("path", &path)]))),
            }
        });
    });
    move || match (table.get(), err.get()) {
        (Some(t), _) => view! {
            {text_preview_banner(note.get())}
            {table_view(&t, locale.get()).into_view()}
        }
        .into_view(),
        (_, Some(e)) => view! { <div class="rp-error">{e}</div> }.into_view(),
        _ => view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }.into_view(),
    }
}

/// Text/source preview: line-numbered and syntax-highlighted via `RpCodeView`.
/// The old plain-text mount dropped the file's newlines (`textContent` on a
/// non-`pre` div), which is what made R/shell scripts render as one paragraph.
#[component]
pub(crate) fn CodeFilePreview(path: String, lang: String) -> impl IntoView {
    let locale = use_locale();
    let body = create_rw_signal::<Option<String>>(None);
    let note = create_rw_signal::<Option<String>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    let is_json = lang == "json";
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            body.set(None);
            note.set(None);
            err.set(None);
            let fc = match load_file_content(&path, loc, Some(TEXT_PREVIEW_MAX_BYTES)).await {
                Ok(fc) => fc,
                Err(e) => {
                    err.set(Some(e));
                    return;
                }
            };
            // No text means the backend judged it binary; an empty code view
            // would just look like an empty file.
            let Some(text) = fc.text.as_deref() else {
                err.set(Some(t(loc, "preview.unsupported_file")));
                return;
            };
            let rendered = if is_json {
                pretty_json(text)
            } else {
                text.to_string()
            };
            let (clipped, shown) = clip_preview_text(&rendered);
            note.set(preview_truncation_note(&fc, shown, loc));
            body.set(Some(clipped));
        });
    });
    move || match (body.get(), err.get()) {
        (Some(text), _) => view! {
            {text_preview_banner(note.get())}
            <RpCodeView lang=lang.clone() body=text />
        }
        .into_view(),
        (_, Some(e)) => view! { <div class="rp-error">{e}</div> }.into_view(),
        _ => view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }.into_view(),
    }
}

/// `.ipynb` preview: Markdown cells rendered, code cells highlighted in the
/// kernel's language, and saved static outputs under each cell. Reuses the chat
/// Notebook pane's styling so both read the same.
fn notebook_output_view(out: &NbOutput, dom_id: String, locale: Locale) -> View {
    match out {
        NbOutput::Text { text, error } => view! {
            <pre class=if *error { "nb-out-error" } else { "" }>{text.clone()}</pre>
        }
        .into_view(),
        NbOutput::Image { mime, b64 } => view! {
            <img
                class="rp-img"
                src=format!("data:{mime};base64,{b64}")
                alt=""
                loading="lazy"
                decoding="async"
            />
        }
        .into_view(),
        NbOutput::Html(html) => {
            let payload = serde_json::json!({ "text": html }).to_string();
            view! {
                <HeavyPreview dom_id=dom_id kind="notebook-html".to_string() payload=payload />
            }
            .into_view()
        }
        NbOutput::Svg(svg) => {
            let payload = serde_json::json!({
                "text": svg,
                "error": t(locale, "preview.unsupported_file"),
            })
            .to_string();
            view! {
                <HeavyPreview dom_id=dom_id kind="notebook-svg".to_string() payload=payload />
            }
            .into_view()
        }
        NbOutput::Latex(tex) => {
            let payload = serde_json::json!({ "tex": tex, "display": true }).to_string();
            view! {
                <div class="nb-out-latex">
                    <HeavyPreview dom_id=dom_id kind="latex".to_string() payload=payload />
                </div>
            }
            .into_view()
        }
        NbOutput::Omitted { mime, bytes } => {
            let size = format_bytes(*bytes as u64);
            let message = tf(
                locale,
                "preview.output_omitted",
                &[("kind", mime), ("size", &size)],
            );
            view! { <div class="nb-out-omitted">{message}</div> }.into_view()
        }
    }
}

#[component]
pub(crate) fn NotebookFilePreview(path: String) -> impl IntoView {
    let locale = use_locale();
    let nb = create_rw_signal::<Option<Notebook>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    let hid = unique_dom_id("nb");
    create_effect(move |_| {
        let path = path.clone();
        let loc = locale.get();
        spawn_local(async move {
            nb.set(None);
            err.set(None);
            // Notebooks are JSON documents; allow a larger budget than plain
            // text/code previews, still truncating so multi-hundred-MB dumps
            // cannot freeze the UI.
            match load_file_content(&path, loc, Some(8 * 1024 * 1024)).await {
                // A .ipynb that doesn't parse is corrupt or not really a notebook;
                // say so rather than drawing an empty cell list.
                Ok(fc) => match parse_notebook(fc.text.as_deref().unwrap_or("")) {
                    Some(parsed) => nb.set(Some(parsed)),
                    None if fc.truncated => err.set(Some(tf(
                        loc,
                        "preview.text_truncated",
                        &[
                            (
                                "shown",
                                &format_bytes(
                                    fc.text.as_ref().map(|t| t.len() as u64).unwrap_or(0),
                                ),
                            ),
                            ("total", &format_bytes(fc.total_bytes.unwrap_or(0))),
                        ],
                    ))),
                    None => err.set(Some(t(loc, "preview.unsupported_file"))),
                },
                Err(e) => err.set(Some(e)),
            }
        });
    });
    let hid_effect = hid.clone();
    // One pass over the whole list: highlights the fenced code and math inside
    // rendered Markdown cells. Code cells highlight themselves via RpCodeView.
    create_effect(move |_| {
        let _ = nb.get();
        schedule_highlight(hid_effect.clone());
    });
    move || match (nb.get(), err.get()) {
        (Some(parsed), _) => {
            let lang = parsed.lang.clone();
            view! {
                <div class="notebook-cells" id=hid.clone()>
                    {parsed.cells.iter().enumerate().map(|(i, cell)| {
                        let outputs = cell.outputs.iter().enumerate().map(|(output_i, out)| {
                            notebook_output_view(
                                out,
                                format!("{hid}-output-{i}-{output_i}"),
                                locale.get_untracked(),
                            )
                        }).collect_view();
                        let body = if cell.markdown {
                            view! { <div class="md" inner_html=md_to_html(&cell.source)></div> }.into_view()
                        } else {
                            view! {
                                <div class="notebook-source">
                                    <RpCodeView lang=lang.clone() body=cell.source.clone() />
                                </div>
                            }.into_view()
                        };
                        view! {
                            <div class="notebook-cell">
                                <div class="notebook-cell-head">
                                    <span class="notebook-index">{i + 1}</span>
                                    <span class="notebook-language">
                                        {if cell.markdown { "markdown".to_string() } else { lang.clone() }}
                                    </span>
                                </div>
                                {body}
                                {(!cell.outputs.is_empty()).then(|| view! {
                                    <div class="notebook-output">{outputs}</div>
                                })}
                            </div>
                        }
                    }).collect_view()}
                </div>
            }
            .into_view()
        }
        (_, Some(e)) => view! { <div class="rp-error">{e}</div> }.into_view(),
        _ => view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }.into_view(),
    }
}

#[component]
pub(crate) fn WorkspaceFilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    match kind.as_str() {
        "csv" => view! { <CsvFilePreview path=path /> }.into_view(),
        // Artifact/version tabs aren't real paths, so the extension can't be read
        // back off them — the kind is the only language signal here.
        "json" => view! { <CodeFilePreview path=path lang="json".to_string() /> }.into_view(),
        "code" | "text" => {
            let lang = code_lang(&path).unwrap_or("plaintext").to_string();
            view! { <CodeFilePreview path=path lang=lang /> }.into_view()
        }
        "notebook" => view! { <NotebookFilePreview path=path /> }.into_view(),
        // Image + PDF share the zoom viewport: the wheel zooms, and PDF pages are
        // stepped with the toolbar buttons / arrow keys / Page Up-Down.
        "image" | "pdf" => {
            view! { <ZoomableFilePreview dom_id=dom_id path=path kind=kind /> }.into_view()
        }
        _ => view! { <FilePreview dom_id=dom_id path=path kind=kind /> }.into_view(),
    }
}

/// A comment on a selected image region: the rect in fractional coordinates
/// of the image box (all values in 0..=1, measured from the top-left corner)
/// plus the note text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImagePin {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) w: f64,
    pub(crate) h: f64,
    pub(crate) note: String,
}

thread_local! {
    // ponytail: pins live in this session-scoped map only and are never
    // persisted — add real storage when annotations must survive a restart.
    static IMAGE_PINS: RefCell<HashMap<String, Vec<ImagePin>>> = RefCell::new(HashMap::new());
}

/// Percent-positioned marker style at the region's center: fractions of the
/// content box, so markers track the image through zoom and pan exactly like
/// the crop rectangle.
pub(crate) fn pin_marker_style(pin: &ImagePin) -> String {
    format!(
        "left:{:.4}%;top:{:.4}%",
        (pin.x + pin.w / 2.0) * 100.0,
        (pin.y + pin.h / 2.0) * 100.0
    )
}

/// The "send for AI revision" composer text: the localized head line naming
/// the file, then one numbered line per pin with its fractional region rect.
pub(crate) fn pin_review_message(head: &str, pins: &[ImagePin]) -> String {
    let mut out = String::from(head);
    for (index, pin) in pins.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. (x={:.3}, y={:.3}, w={:.3}, h={:.3}) {}",
            index + 1,
            pin.x,
            pin.y,
            pin.w,
            pin.h,
            pin.note
        ));
    }
    out
}

/// Hand the assembled pin message to the chat shell, which lands it in the
/// composer through the same quote path as "Ask AI in the conversation".
fn dispatch_pins_ask_ai(path: &str, text: &str) {
    let Ok(detail) = to_value(&PinsAskAi {
        path: path.to_string(),
        text: text.to_string(),
    }) else {
        return;
    };
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("wisp:pins-ask-ai", &init)
    else {
        return;
    };
    if let Some(window) = web_sys::window() {
        let _ = window.dispatch_event(&event);
    }
}

#[cfg(test)]
mod image_pin_tests {
    use super::{pin_marker_style, pin_review_message, ImagePin};

    #[test]
    fn pin_review_message_numbers_fractional_regions() {
        let pins = vec![
            ImagePin {
                x: 0.5,
                y: 0.25,
                w: 0.1,
                h: 0.05,
                note: "对比度太低".into(),
            },
            ImagePin {
                x: 0.0,
                y: 0.75,
                w: 0.25,
                h: 0.25,
                note: "crop this corner".into(),
            },
        ];
        assert_eq!(
            pin_review_message("Revision pins on image `results/plot.png`:", &pins),
            "Revision pins on image `results/plot.png`:\n\
             1. (x=0.500, y=0.250, w=0.100, h=0.050) 对比度太低\n\
             2. (x=0.000, y=0.750, w=0.250, h=0.250) crop this corner"
        );
        assert_eq!(pin_review_message("head", &[]), "head");
    }

    #[test]
    fn pin_marker_style_centers_the_region() {
        let pin = ImagePin {
            x: 0.4,
            y: 0.2,
            w: 0.2,
            h: 0.1,
            note: String::new(),
        };
        assert_eq!(pin_marker_style(&pin), "left:50.0000%;top:25.0000%");
        let corner = ImagePin {
            x: 0.0,
            y: 1.0,
            w: 0.0,
            h: 0.0,
            note: String::new(),
        };
        assert_eq!(pin_marker_style(&corner), "left:0.0000%;top:100.0000%");
    }
}

#[component]
fn ZoomableFilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    let locale = use_locale();
    let zoom = create_rw_signal(100u16);
    let is_dragging = create_rw_signal(false);
    let drag_start = Rc::new(Cell::new(None::<(i32, i32, i32, i32)>));
    let viewport_id = unique_dom_id("preview-viewport");
    // Region-crop (images only): drag a rectangle, then comment on it or
    // attach it to the chat. Confirmed comments become numbered region notes
    // ("pins") assembled into one revision-request message.
    let is_image = kind == "image";
    let pins = create_rw_signal(
        is_image
            .then(|| IMAGE_PINS.with(|m| m.borrow().get(&path).cloned().unwrap_or_default()))
            .unwrap_or_default(),
    );
    // Note text for the region popup input (window.prompt is a no-op under wry).
    let pin_note = create_rw_signal(String::new());
    let pin_input_id = unique_dom_id("pin-note");
    if is_image {
        // Write-through so pins survive a preview remount within the session.
        let store_path = path.clone();
        create_effect(move |_| {
            let value = pins.get();
            IMAGE_PINS.with(|m| {
                m.borrow_mut().insert(store_path.clone(), value);
            });
        });
    }
    let crop_mode = create_rw_signal(false);
    let crop_busy = create_rw_signal(false);
    // A finalized region: the rubber-band rect is frozen and the action popup
    // (comment input + attach buttons) is showing.
    let crop_ready = create_rw_signal(false);
    // Live rubber-band rect as fractions (0..1) of the crop layer, rendered
    // with percent positioning: (left, top, right, bottom). Content-anchored
    // coordinates keep the rect, the crop, and the action popup glued to the
    // image through zoom/scroll — and inside the modal, whose filling
    // entrance animation makes it the containing block for position:fixed
    // descendants in WebKit, which skewed the old client-pixel math.
    let crop_rect = create_rw_signal(None::<(f64, f64, f64, f64)>);
    // Pointer position as fractions of the crop layer. Resolved via
    // `target.closest` because Leptos delegates pointer events, so
    // `current_target` is the delegation root rather than the layer.
    fn layer_point(ev: &web_sys::PointerEvent) -> Option<(web_sys::Element, f64, f64)> {
        let layer = ev
            .target()?
            .dyn_into::<web_sys::Element>()
            .ok()?
            .closest(".file-preview-crop-layer")
            .ok()
            .flatten()?;
        let rect = layer.get_bounding_client_rect();
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return None;
        }
        let x = ((ev.client_x() as f64 - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let y = ((ev.client_y() as f64 - rect.top()) / rect.height()).clamp(0.0, 1.0);
        Some((layer, x, y))
    }
    // Takes the layer's on-screen size so the stray-click guard stays in
    // physical pixels regardless of zoom. Finalizing only freezes the rect and
    // raises the popup — the upload happens later, and only if an attach
    // action is chosen, so a comment-only region never uploads anything.
    let finish_input_id = pin_input_id.clone();
    let finish_crop = Callback::new(move |(layer_w, layer_h): (f64, f64)| {
        if crop_busy.get_untracked() || crop_ready.get_untracked() {
            return;
        }
        let Some((l, t, r, b)) = crop_rect.get_untracked() else {
            return;
        };
        let (w, h) = ((l - r).abs(), (t - b).abs());
        // Ignore stray clicks; require a real region.
        if w * layer_w < 8.0 || h * layer_h < 8.0 {
            crop_rect.set(None);
            return;
        }
        crop_ready.set(true);
        let focus_id = finish_input_id.clone();
        spawn_local(async move {
            focus_element(&focus_id);
        });
    });
    // Confirming the note turns the frozen region into a numbered pin and
    // keeps crop mode armed for the next region.
    let commit_pin = move || {
        let note = pin_note.get_untracked().trim().to_string();
        if note.is_empty() {
            return;
        }
        let Some((l, t, r, b)) = crop_rect.get_untracked() else {
            return;
        };
        let (x, y) = (l.min(r), t.min(b));
        let (w, h) = ((l - r).abs(), (t - b).abs());
        pins.update(|list| list.push(ImagePin { x, y, w, h, note }));
        pin_note.set(String::new());
        crop_rect.set(None);
        crop_ready.set(false);
    };
    let crop_host_id = dom_id.clone();
    let attach_region = move |jump: bool| {
        if crop_busy.get_untracked() {
            return;
        }
        let Some((l, t, r, b)) = crop_rect.get_untracked() else {
            return;
        };
        let (left, top) = (l.min(r), t.min(b));
        let (w, h) = ((l - r).abs(), (t - b).abs());
        let host_id = crop_host_id.clone();
        crop_busy.set(true);
        spawn_local(async move {
            let path = crop_region_to_upload(&host_id, left, top, w, h)
                .await
                .as_string()
                .unwrap_or_default();
            crop_busy.set(false);
            crop_rect.set(None);
            crop_ready.set(false);
            if !path.is_empty() {
                pin_note.set(String::new());
                crop_mode.set(false);
                attach_cropped_region(&path, jump);
            }
        });
    };
    let pin_send_path = path.clone();
    let locale_for_send = locale;
    let send_pins = move |_| {
        let list = pins.get_untracked();
        if list.is_empty() {
            return;
        }
        let head = tf(
            locale_for_send.get_untracked(),
            "preview.pin_message_head",
            &[("path", &pin_send_path)],
        );
        pin_note.set(String::new());
        crop_rect.set(None);
        crop_ready.set(false);
        crop_mode.set(false);
        dispatch_pins_ask_ai(&pin_send_path, &pin_review_message(&head, &list));
    };
    // Esc cancels the pending selection first, then crop mode itself.
    // Capture phase so this wins over the app-level Escape stack, which
    // would otherwise close the surrounding artifact modal.
    if is_image {
        window_capture_escape(move || {
            if crop_busy.get_untracked() {
                return false;
            }
            // Order: pending region (and its note) first, then crop mode.
            if crop_rect.get_untracked().is_some() || crop_ready.get_untracked() {
                crop_ready.set(false);
                crop_rect.set(None);
                pin_note.set(String::new());
            } else if crop_mode.get_untracked() {
                crop_mode.set(false);
            } else {
                return false;
            }
            true
        });
    }
    let adjust_zoom = move |delta: i16| {
        zoom.update(|value| {
            *value = ((*value as i16) + delta).clamp(25, 400) as u16;
        });
    };
    let viewport_for_event = Rc::new({
        let viewport_id = viewport_id.clone();
        move || {
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(&viewport_id))
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        }
    });
    let stop_drag = {
        let viewport_for_event = viewport_for_event.clone();
        let drag_start = drag_start.clone();
        move |pointer_id: i32| {
            if let Some(viewport) = viewport_for_event() {
                let _ = viewport.release_pointer_capture(pointer_id);
            }
            drag_start.set(None);
            is_dragging.set(false);
        }
    };
    let viewport_for_pointerdown = viewport_for_event.clone();
    let viewport_for_pointermove = viewport_for_event.clone();
    let stop_drag_up = stop_drag.clone();
    let stop_drag_cancel = stop_drag.clone();
    let drag_start_down = drag_start.clone();
    let drag_start_move = drag_start.clone();
    let drag_start_lost = drag_start.clone();
    let popup_input_id = pin_input_id.clone();
    view! {
        <div class="file-preview-zoom">
            <div class="file-preview-zoom-bar">
                <button type="button" aria-label=move || t(locale.get(), "preview.zoom_out")
                    disabled=move || { zoom.get() <= 25 }
                    on:click=move |_| adjust_zoom(-25)>"−"</button>
                <button type="button" aria-label=move || t(locale.get(), "preview.zoom_reset")
                    on:click=move |_| zoom.set(100)>{move || format!("{}%", zoom.get())}</button>
                <button type="button" aria-label=move || t(locale.get(), "preview.zoom_in")
                    disabled=move || { zoom.get() >= 400 }
                    on:click=move |_| adjust_zoom(25)>"+"</button>
                {is_image.then(|| view! {
                    <button type="button" class="file-preview-crop-btn"
                        class:active=move || crop_mode.get()
                        disabled=move || crop_busy.get()
                        aria-pressed=move || crop_mode.get().to_string()
                        title=move || t(locale.get(), "preview.select_region")
                        aria-label=move || t(locale.get(), "preview.select_region")
                        on:click=move |_| {
                            crop_rect.set(None);
                            crop_ready.set(false);
                            pin_note.set(String::new());
                            crop_mode.update(|m| *m = !*m);
                        }>
                        {compose_icon("crop")}
                    </button>
                })}
                {move || (is_image && !pins.get().is_empty()).then(|| view! {
                    <button type="button" class="file-preview-crop-btn file-preview-pin-send"
                        title=move || t(locale.get(), "preview.pin_send")
                        on:click=send_pins.clone()>
                        {compose_icon("chat")}
                        <span>{move || format!(
                            "{} ({})",
                            t(locale.get(), "preview.pin_send"),
                            pins.get().len(),
                        )}</span>
                    </button>
                })}
            </div>
            <div id=viewport_id class="file-preview-zoom-viewport"
                class:is-dragging=move || { is_dragging.get() }
                class:is-cropping=move || { crop_mode.get() }
                on:pointerdown=move |ev: web_sys::PointerEvent| {
                    if ev.button() != 0 || crop_mode.get_untracked() {
                        return;
                    }
                    // PDF glyphs remain drag-selectable for quoting. Starting
                    // on the surrounding page/whitespace pans the preview.
                    if ev
                        .target()
                        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                        .and_then(|target| target.closest(".rp-pdf-textlayer span").ok().flatten())
                        .is_some()
                    {
                        return;
                    }
                    let Some(viewport) = viewport_for_pointerdown() else {
                        return;
                    };
                    // Zoom percentage is not a reliable proxy for pannability:
                    // a tall image or PDF page can overflow the modal at 100%.
                    // Only capture the drag when there is actual scrollable
                    // content in at least one direction.
                    if viewport.scroll_width() <= viewport.client_width()
                        && viewport.scroll_height() <= viewport.client_height()
                    {
                        return;
                    }
                    ev.prevent_default();
                    let _ = viewport.set_pointer_capture(ev.pointer_id());
                    drag_start_down.set(Some((
                        ev.client_x(),
                        ev.client_y(),
                        viewport.scroll_left(),
                        viewport.scroll_top(),
                    )));
                    is_dragging.set(true);
                }
                on:pointermove=move |ev: web_sys::PointerEvent| {
                    let Some((start_x, start_y, scroll_left, scroll_top)) = drag_start_move.get() else {
                        return;
                    };
                    let Some(viewport) = viewport_for_pointermove() else {
                        return;
                    };
                    ev.prevent_default();
                    viewport.set_scroll_left(scroll_left - (ev.client_x() - start_x));
                    viewport.set_scroll_top(scroll_top - (ev.client_y() - start_y));
                }
                on:pointerup=move |ev: web_sys::PointerEvent| stop_drag_up(ev.pointer_id())
                on:pointercancel=move |ev: web_sys::PointerEvent| stop_drag_cancel(ev.pointer_id())
                on:lostpointercapture=move |_| {
                    drag_start_lost.set(None);
                    is_dragging.set(false);
                }
                on:wheel=move |ev: web_sys::WheelEvent| {
                    ev.prevent_default();
                    if ev.delta_y() < 0.0 {
                        adjust_zoom(25);
                    } else if ev.delta_y() > 0.0 {
                        adjust_zoom(-25);
                    }
                }>
                <div class="file-preview-zoom-content" data-zoom-kind=kind.clone()
                    style=move || format!("--preview-zoom:{}", zoom.get() as f32 / 100.0)>
                    <FilePreview dom_id=dom_id path=path kind=kind />
                // Region-crop overlay: lives inside the zoomed content so the
                // fraction-based rubber-band tracks the image through zoom and
                // scroll. Captures the drag so it never pans; releasing opens
                // the note/attach popup for the region.
                {move || crop_mode.get().then(|| {
                    let attach_region = attach_region.clone();
                    let popup_input_id = popup_input_id.clone();
                    view! {
                    <div class="file-preview-crop-layer"
                        on:pointerdown=move |ev: web_sys::PointerEvent| {
                            if ev.button() != 0
                                || crop_busy.get_untracked()
                                || crop_ready.get_untracked()
                            {
                                return;
                            }
                            ev.prevent_default();
                            let Some((target, x, y)) = layer_point(&ev) else {
                                return;
                            };
                            let _ = target.set_pointer_capture(ev.pointer_id());
                            crop_rect.set(Some((x, y, x, y)));
                        }
                        on:pointermove=move |ev: web_sys::PointerEvent| {
                            if crop_busy.get_untracked() || crop_ready.get_untracked() {
                                return;
                            }
                            let Some((_, x, y)) = layer_point(&ev) else {
                                return;
                            };
                            crop_rect.update(|r| {
                                if let Some((l, t, _, _)) = *r {
                                    *r = Some((l, t, x, y));
                                }
                            });
                        }
                        on:pointerup=move |ev: web_sys::PointerEvent| {
                            let Some((layer, _, _)) = layer_point(&ev) else {
                                return;
                            };
                            let rect = layer.get_bounding_client_rect();
                            finish_crop.call((rect.width(), rect.height()));
                        }
                        on:pointercancel=move |_| crop_rect.set(None)>
                        {move || crop_rect.get().map(|(left, top, right, bottom)| {
                            let selected = crop_ready.get();
                            let style = format!(
                                "left:{}%;top:{}%;width:{}%;height:{}%",
                                left.min(right) * 100.0,
                                top.min(bottom) * 100.0,
                                (left - right).abs() * 100.0,
                                (top - bottom).abs() * 100.0,
                            );
                            view! {
                                <div class="file-preview-crop-rect" class:selected=selected style=style>
                                    {selected.then(|| view! {
                                        <span class="file-preview-crop-label">
                                            {move || t(locale.get(), "preview.region_selected")}
                                        </span>
                                    })}
                                </div>
                            }
                        })}
                        // Region popup: a note input (comment pins) on top of
                        // the attach actions. Confirming the note records a
                        // numbered pin; attaching uploads the crop only then.
                        {move || {
                            let attach_region = attach_region.clone();
                            let input_id = popup_input_id.clone();
                            crop_ready.get()
                                .then(|| crop_rect.get())
                                .flatten()
                                .map(|(left, top, right, bottom)| {
                            let x = (left + right) / 2.0 * 100.0;
                            let y = top.min(bottom) * 100.0;
                            // Clamped inside the layer: the scroll viewport
                            // clips anything outside the content box.
                            let style = format!(
                                "left:clamp(150px,{x}%,calc(100% - 150px));top:max(52px,{y}%)",
                            );
                            let attach_add = attach_region.clone();
                            let attach_jump = attach_region;
                            view! {
                                <div class="selection-popup file-preview-crop-actions file-preview-crop-annotate" style=style
                                    on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                                    on:pointerup=|ev: web_sys::PointerEvent| ev.stop_propagation()>
                                    <div class="file-preview-pin-editor-row">
                                        <input id=input_id type="text"
                                            placeholder=move || t(locale.get(), "preview.pin_placeholder")
                                            prop:value=move || pin_note.get()
                                            on:input=move |ev| pin_note.set(event_target_value(&ev))
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                if ev.key() == "Enter" {
                                                    commit_pin();
                                                }
                                            } />
                                        <button type="button" class="selection-popup-btn"
                                            title=move || t(locale.get(), "preview.pin_add")
                                            aria-label=move || t(locale.get(), "preview.pin_add")
                                            on:click=move |_| commit_pin()>
                                            {compose_icon("pin")}
                                        </button>
                                    </div>
                                    <div class="file-preview-crop-actions-row">
                                        <button type="button" class="selection-popup-btn"
                                            disabled=move || crop_busy.get()
                                            on:click=move |_| attach_add(false)>
                                            {compose_icon("plus")}
                                            <span>{move || t(locale.get(), "selection.add_to_chat")}</span>
                                        </button>
                                        <button type="button" class="selection-popup-btn"
                                            disabled=move || crop_busy.get()
                                            on:click=move |_| attach_jump(true)>
                                            {compose_icon("chat")}
                                            <span>{move || t(locale.get(), "selection.add_to_chat_and_jump")}</span>
                                        </button>
                                    </div>
                                </div>
                            }
                        })}}
                    </div>
                    }
                })}
                // Pin overlay: numbered markers stay visible whenever the
                // image has region notes. The layer itself never takes the
                // pointer; markers sit above the crop layer so they remain
                // clickable for deletion while crop mode is armed.
                // Fraction-based positioning rides the same content box as the
                // crop rect, so pins track the image through zoom and pan.
                {move || (is_image && !pins.get().is_empty()).then(|| view! {
                    <div class="file-preview-pin-layer">
                        {move || pins.get().into_iter().enumerate().map(|(index, pin)| {
                            let style = pin_marker_style(&pin);
                            let note = pin.note.clone();
                            view! {
                                <button type="button" class="file-preview-pin-marker" style=style
                                    title=move || if crop_mode.get() {
                                        tf(locale.get(), "preview.pin_remove", &[("note", &note)])
                                    } else {
                                        note.clone()
                                    }
                                    on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                                    on:click=move |_| {
                                        if crop_mode.get_untracked() {
                                            pins.update(|list| {
                                                if index < list.len() {
                                                    list.remove(index);
                                                }
                                            });
                                        }
                                    }>
                                    {(index + 1).to_string()}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })}
                </div>
            </div>
        </div>
    }
}

#[component]
pub(crate) fn FilePreview(dom_id: String, path: String, kind: String) -> impl IntoView {
    let locale = use_locale();
    let id_for_effect = dom_id.clone();
    let path_for_effect = path.clone();
    create_effect(move |_| {
        let path = path_for_effect.clone();
        let kind = kind.clone();
        let dom_id = id_for_effect.clone();
        let loc = locale.get();
        spawn_local(async move {
            let doc = web_sys::window().and_then(|w| w.document());
            let el = doc.as_ref().and_then(|d| d.get_element_by_id(&dom_id));
            // PDF and OOXML previews fetch raw bytes inside api.js. Keeping the
            // Tauri Response on the JS side avoids Rust bytes -> Base64 -> WASM
            // string -> decoded ArrayBuffer copies in the WebView.
            if matches!(kind.as_str(), "pdf" | "docx" | "xlsx" | "pptx") {
                let payload = match kind.as_str() {
                    // 100 MB: journal PDFs with embedded figures routinely pass
                    // 32 MB; pdf.js renders page-at-a-time so memory is bounded.
                    "pdf" => serde_json::json!({
                        "path": path,
                        "maxBytes": 100 * 1024 * 1024,
                        "loading": t(loc, "loading"),
                        "error": t(loc, "preview.pdf_error"),
                        "pageLabel": t(loc, "preview.pdf_page"),
                        "prevPage": t(loc, "preview.pdf_prev_page"),
                        "nextPage": t(loc, "preview.pdf_next_page"),
                    }),
                    "docx" => serde_json::json!({
                        "path": path,
                        "maxBytes": 32 * 1024 * 1024,
                        "loading": t(loc, "loading"),
                        "error": t(loc, "preview.docx_error"),
                    }),
                    "xlsx" => serde_json::json!({
                        "path": path,
                        "maxBytes": 32 * 1024 * 1024,
                        "loading": t(loc, "loading"),
                        "error": t(loc, "preview.xlsx_error"),
                        "formulaLabel": t(loc, "preview.xlsx_formula"),
                        "truncated": t(loc, "preview.xlsx_truncated"),
                    }),
                    _ => serde_json::json!({
                        "path": path,
                        "maxBytes": 32 * 1024 * 1024,
                        "loading": t(loc, "loading"),
                        "error": t(loc, "preview.pptx_error"),
                    }),
                };
                let _ = mount_preview(&kind, &dom_id, &payload.to_string()).await;
                return;
            }
            // Images need a full under-budget payload; text-ish kinds only pull a
            // head so multi-GB logs never enter the WebView (#large-text-preview).
            let budget = if matches!(kind.as_str(), "image" | "document") {
                Some(32 * 1024 * 1024)
            } else {
                Some(TEXT_PREVIEW_MAX_BYTES)
            };
            let fc = match load_file_content(&path, loc, budget).await {
                Ok(fc) => fc,
                Err(message) => {
                    if let Some(el) = el {
                        el.set_class_name("rp-heavy rp-error");
                        el.set_text_content(Some(&message));
                    }
                    return;
                }
            };
            if kind != "image" && fc.text.is_none() {
                if let Some(el) = el {
                    el.set_class_name("rp-heavy rp-error");
                    el.set_text_content(Some(&t(loc, "preview.unsupported_file")));
                }
                return;
            }
            if matches!(kind.as_str(), "markdown" | "document") {
                if let Some(el) = el {
                    let raw = fc.text.as_deref().unwrap_or("");
                    let (clipped, shown) = clip_preview_text(raw);
                    let mut html = String::new();
                    if let Some(note) = preview_truncation_note(&fc, shown, loc) {
                        html.push_str(&format!(
                            "<div class=\"preview-trunc-note\">{}</div>",
                            html_escape(&note)
                        ));
                    }
                    html.push_str(&md_document_to_html(&clipped));
                    el.set_class_name("rp-heavy md");
                    el.set_inner_html(&html);
                    schedule_highlight(dom_id.clone());
                }
                return;
            }
            let (mount_kind, payload) = match kind.as_str() {
                "image" => (
                    "image",
                    serde_json::json!({ "b64": fc.base64, "mime": fc.mime }).to_string(),
                ),
                "html" => {
                    // A remote file's path would resolve as a local file:// base
                    // href; better no base at all than the wrong machine's.
                    let base = remote_file_path(&path)
                        .is_none()
                        .then_some(fc.path.as_str());
                    (
                        "html",
                        serde_json::json!({ "text": fc.text, "path": base }).to_string(),
                    )
                }
                "structure" => (
                    "structure",
                    serde_json::json!({ "text": fc.text, "format": "pdb" }).to_string(),
                ),
                "molecule" | "smiles" => (
                    "molecule",
                    serde_json::json!({ "text": fc.text, "smiles": fc.text }).to_string(),
                ),
                "fasta" => ("fasta", serde_json::json!({ "text": fc.text }).to_string()),
                "msa" => ("msa", serde_json::json!({ "text": fc.text }).to_string()),
                _ => ("text", serde_json::json!({ "text": fc.text }).to_string()),
            };
            let _ = mount_preview(mount_kind, &dom_id, &payload).await;
        });
    });
    view! { <div class="rp-heavy" id=dom_id>{move || t(locale.get(), "loading")}</div> }
}

pub(crate) fn artifact_preview(a: &Artifact, dom_id: String, locale: Locale) -> impl IntoView {
    match &a.data {
        PreviewData::Table(t) => table_view(t.as_ref(), locale).into_view(),
        PreviewData::Latex { tex, display } => {
            let payload = serde_json::json!({ "tex": tex, "display": display }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="latex".to_string() payload=payload /> }
                .into_view()
        }
        PreviewData::Fasta(text) => {
            let payload = serde_json::json!({ "text": text.as_ref() }).to_string();
            view! { <HeavyPreview dom_id=dom_id kind="fasta".to_string() payload=payload /> }
                .into_view()
        }
        PreviewData::File { path, kind } => view! {
            <p class="rp-path hint">{a.location.clone().unwrap_or_else(|| path.clone())}</p>
            <div class="rp-file-preview" data-file-path=path.clone()>
                <WorkspaceFilePreview dom_id=dom_id path=path.clone() kind=kind.clone() />
            </div>
        }
        .into_view(),
    }
}

/// Right-pane code view with a line-number gutter (Claude Science style).
/// The gutter is a plain <pre> (no <code>) so highlight.js skips it.
#[component]
pub(crate) fn RpCodeView(lang: String, body: String) -> impl IntoView {
    let lang_class = if lang.is_empty() {
        "plaintext".to_string()
    } else {
        lang.clone()
    };
    let hid = unique_dom_id("rpcode");
    let hid_for_effect = hid.clone();
    let body_track = body.clone();
    create_effect(move |_| {
        let _ = &body_track;
        schedule_highlight(hid_for_effect.clone());
    });
    // split('\n') matches how <pre> renders a trailing newline, keeping the
    // gutter aligned with the body line-for-line.
    let n = body.split('\n').count().max(1);
    let gutter = (1..=n)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    view! {
        <div class="rp-code" id=hid.clone()>
            <pre class="rp-code-gutter">{gutter}</pre>
            <pre class="rp-code-body"><code class=format!("language-{lang_class}")>{body.clone()}</code></pre>
        </div>
    }
}

/// Editable center source view for R/Python scripts: the same gutter +
/// highlighted body as `RpCodeView`, with a transparent textarea overlaid so
/// the user can type directly. The highlighted `<code>` node is driven
/// imperatively (`set_highlighted_code`) because highlight.js rewrites its
/// children, which the reactive renderer must not also own.
///
/// Drafts live in the caller's map keyed by path, so unsaved edits survive a
/// preview remount (an agent `FileChanged` bumps the revision). Files the
/// preview could not load in full fall back to the read-only view — saving a
/// clipped head would destroy the tail.
#[component]
pub(crate) fn RpCodeEditor(
    dom_id: String,
    path: String,
    lang: String,
    drafts: RwSignal<HashMap<String, String>>,
) -> impl IntoView {
    let locale = use_locale();
    let disk = create_rw_signal::<Option<String>>(None);
    let clipped_note = create_rw_signal::<Option<String>>(None);
    let err = create_rw_signal::<Option<String>>(None);
    let saving = create_rw_signal(false);
    let save_error = create_rw_signal::<Option<String>>(None);
    let code_id = unique_dom_id("rpedit");

    let load_path = path.clone();
    create_effect(move |_| {
        let path = load_path.clone();
        let loc = locale.get();
        spawn_local(async move {
            let fc = match load_file_content(&path, loc, Some(TEXT_PREVIEW_MAX_BYTES)).await {
                Ok(fc) => fc,
                Err(e) => {
                    err.set(Some(e));
                    return;
                }
            };
            let Some(text) = fc.text.as_deref() else {
                err.set(Some(t(loc, "preview.unsupported_file")));
                return;
            };
            let (rendered, shown) = clip_preview_text(text);
            if fc.truncated || rendered.len() < text.len() {
                clipped_note.set(preview_truncation_note(&fc, shown, loc));
                disk.set(Some(rendered));
            } else {
                clipped_note.set(None);
                disk.set(Some(text.to_string()));
            }
            err.set(None);
        });
    });

    let draft_path = path.clone();
    let draft = create_memo(move |_| {
        drafts
            .with(|drafts| drafts.get(&draft_path).cloned())
            .or_else(|| disk.get())
            .unwrap_or_default()
    });
    let dirty = create_memo(move |_| disk.get().is_some_and(|saved| saved != draft.get()));

    // The highlighted mirror under the textarea. The trailing newline keeps
    // the mirror's height covering the caret's empty last line.
    let mirror_id = code_id.clone();
    create_effect(move |_| {
        if clipped_note.get().is_some() || err.get().is_some() || disk.get().is_none() {
            return;
        }
        set_highlighted_code(mirror_id.clone(), format!("{}\n", draft.get()));
    });

    let save_path = path.clone();
    let save_now = Callback::new(move |_: ()| {
        if !dirty.get_untracked() || saving.get_untracked() {
            return;
        }
        let path = save_path.clone();
        let content = draft.get_untracked();
        saving.set(true);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "path": path,
                "content": content,
            }))
            .unwrap();
            match invoke_checked("save_file", args).await {
                Ok(_) => {
                    disk.set(Some(content));
                    drafts.update(|drafts| {
                        drafts.remove(&path);
                    });
                    save_error.set(None);
                }
                Err(error) => save_error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(error),
                ))),
            }
            saving.set(false);
        });
    });

    let input_path = path.clone();
    let lang_class = if lang.is_empty() {
        "plaintext".to_string()
    } else {
        lang.clone()
    };
    let fallback_lang = lang.clone();

    move || {
        if let Some(e) = err.get() {
            return view! { <div class="rp-error">{e}</div> }.into_view();
        }
        let Some(loaded) = disk.get() else {
            return view! { <div class="rp-heavy">{move || t(locale.get(), "loading")}</div> }
                .into_view();
        };
        if clipped_note.get().is_some() {
            return view! {
                {text_preview_banner(clipped_note.get())}
                <RpCodeView lang=fallback_lang.clone() body=loaded />
            }
            .into_view();
        }
        let _ = loaded;
        let gutter = move || {
            let n = draft.with(|text| text.split('\n').count()).max(1);
            (1..=n)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let input_path = input_path.clone();
        let keydown_save = save_now;
        view! {
            <div class="rp-code-editor" id=dom_id.clone()>
                <div class="rp-code rp-code-editable">
                    <pre class="rp-code-gutter">{gutter}</pre>
                    <div class="rp-code-edit-stack">
                        <pre class="rp-code-body" aria-hidden="true"><code
                            class=format!("language-{lang_class}") id=code_id.clone()></code></pre>
                        <textarea class="rp-code-input" wrap="off" spellcheck="false"
                            autocomplete="off" autocorrect="off" autocapitalize="off"
                            aria-label=move || t(locale.get(), "editor.source")
                            prop:value=move || draft.get()
                            on:input=move |event| {
                                let value = event_target_value(&event);
                                drafts.update(|drafts| {
                                    drafts.insert(input_path.clone(), value);
                                });
                            }
                            on:keydown=move |event: web_sys::KeyboardEvent| {
                                if event.key() == "s" && (event.ctrl_key() || event.meta_key()) {
                                    event.prevent_default();
                                    keydown_save.call(());
                                }
                            }></textarea>
                    </div>
                </div>
                {move || (dirty.get() || save_error.get().is_some()).then(|| view! {
                    <div class="rp-code-save">
                        {move || save_error.get().map(|error| view! {
                            <span class="rp-code-save-error">{error}</span>
                        })}
                        <button type="button" class="center-file-btn primary" data-editor-save=""
                            disabled=move || saving.get()
                            title=move || t(locale.get(), "editor.save_hint")
                            on:click=move |_| save_now.call(())>
                            {move || t(locale.get(), if saving.get() { "editor.saving" } else { "editor.save" })}
                        </button>
                    </div>
                })}
            </div>
        }
        .into_view()
    }
}

/// File kinds that open in the full ArtifactModal viewer on click (image/pdf
/// full-size, csv as a dataset table) rather than rendering inline in the pane.
pub(crate) fn opens_in_modal(kind: &str) -> bool {
    matches!(kind, "image" | "pdf" | "csv")
}

/// Map each preview-path spelling onto the backend command that can save it:
/// `artifact:` ids and pinned `artifact-version:` ids have no workspace path,
/// so they go to their dedicated commands; remote previews go out as the
/// ssh:// spelling `download_file` already understands; everything else is a
/// workspace path for `download_file`.
pub(crate) fn download_invocation(path: &str) -> Option<(&'static str, serde_json::Value)> {
    if let Some(id) = artifact_id_path(path) {
        return Some(("download_artifact", serde_json::json!({ "id": id })));
    }
    if let Some(id) = artifact_version_id_path(path) {
        return Some((
            "download_artifact_version",
            serde_json::json!({ "versionId": id }),
        ));
    }
    let path = match remote_file_path(path) {
        Some((context_id, remote_path)) => {
            crate::context_menu::remote_file_download_uri(context_id, remote_path)?
        }
        None => path.to_string(),
    };
    Some(("download_file", serde_json::json!({ "path": path })))
}

/// Fire the native save dialog to download the file behind a preview path
/// (backend copies it).
pub(crate) fn download_artifact(path: String) {
    let Some((command, args)) = download_invocation(&path) else {
        return;
    };
    spawn_local(async move {
        if let Err(error) = invoke_checked(command, to_value(&args).unwrap()).await {
            show_toast(&localize_backend(
                Locale::detect_browser(),
                &js_error_text(error),
            ));
        }
    });
}

#[cfg(test)]
mod download_invocation_tests {
    use super::download_invocation;

    #[test]
    fn routes_each_preview_path_spelling_to_its_command() {
        assert_eq!(
            download_invocation("artifact:art-1"),
            Some(("download_artifact", serde_json::json!({ "id": "art-1" })))
        );
        // Pinned versions (branch/exploration previews) must download the
        // exact displayed bytes, never a workspace path that may not exist.
        assert_eq!(
            download_invocation("artifact-version:ver-1"),
            Some((
                "download_artifact_version",
                serde_json::json!({ "versionId": "ver-1" })
            ))
        );
        assert_eq!(
            download_invocation("remote:ssh:gpu:/data/run.csv"),
            Some((
                "download_file",
                serde_json::json!({ "path": "ssh://gpu/data/run.csv" })
            ))
        );
        assert_eq!(
            download_invocation("results/report.md"),
            Some((
                "download_file",
                serde_json::json!({ "path": "results/report.md" })
            ))
        );
    }
}

pub(crate) fn reveal_in_file_manager(path: String) {
    // Remote files have no local location to reveal in the OS file manager.
    if remote_file_path(&path).is_some() {
        return;
    }
    spawn_local(async move {
        let arg = to_value(&serde_json::json!({ "path": path })).unwrap();
        let _ = invoke_checked("reveal_in_file_manager", arg).await;
    });
}

pub(crate) fn keyboard_event_targets_text_entry(ev: &web_sys::KeyboardEvent) -> bool {
    let mut el = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    while let Some(node) = el {
        if node.dyn_ref::<web_sys::HtmlInputElement>().is_some()
            || node.dyn_ref::<web_sys::HtmlTextAreaElement>().is_some()
            || node.tag_name().eq_ignore_ascii_case("select")
            || node.has_attribute("contenteditable")
        {
            return true;
        }
        el = node.parent_element();
    }
    false
}

/// Click-to-expand modal for a produced artifact: shows the full-size
/// image/PDF (or a CSV as a dataset table) plus tabbed provenance
/// (Code/Log/Inputs/Environment) fetched from `get_artifact_provenance`.
/// Provenance is best-effort — a `None` result (or any empty field within it)
/// renders an empty state; the figure never depends on provenance being present.
#[component]
pub(crate) fn ArtifactModal(
    path: String,
    name: String,
    kind: String,
    session: Option<String>,
    can_prev: bool,
    can_next: bool,
    on_prev: Callback<()>,
    on_next: Callback<()>,
    on_close: Callback<()>,
    on_open_center: Callback<ModalArtifact>,
    on_open_path: Callback<(String, String)>, // open an input file (path, kind)
    on_rerun: Callback<String>,               // drop a rerun request into the composer (#455)
    library_items: ReadSignal<Vec<LibraryItemSummary>>,
    on_library_changed: Callback<()>,
) -> impl IntoView {
    let locale = use_locale();
    let prov = create_rw_signal(None::<ArtifactProvenance>);
    let loaded = create_rw_signal(false);
    let tab = create_rw_signal("code");
    let editing_code = create_rw_signal(false);
    let code_draft = create_rw_signal(String::new());
    let dom_id = unique_dom_id("amodal");
    {
        let path = path.clone();
        let session = session.clone();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "sessionId": session, "path": path })).unwrap();
            let v = invoke("get_artifact_provenance", arg).await;
            prov.set(
                serde_wasm_bindgen::from_value::<Option<ArtifactProvenance>>(v)
                    .ok()
                    .flatten(),
            );
            loaded.set(true);
        });
    }
    let path_head = path.clone();
    let path_dl = path.clone();
    let rerun_name = name.clone();
    let center_artifact = (path.clone(), name.clone(), kind.clone());
    let star_path = path.clone();
    let star_session = session.clone();
    let starred = create_memo(move |_| {
        star_session.as_deref().is_some_and(|session| {
            library_items.with(|items| {
                items
                    .iter()
                    .any(|item| item.matches_figure(session, &star_path))
            })
        })
    });
    let click_path = path.clone();
    let click_name = name.clone();
    let click_session = session.clone();
    let is_html = kind == "html";
    let is_zoomable = matches!(kind.as_str(), "image" | "pdf");
    let is_docx = kind == "docx";
    let is_office = matches!(kind.as_str(), "xlsx" | "pptx");
    let can_star = kind == "image";
    view! {
        <div class="overlay" on:click=move |_| on_close.call(())>
            <div class="modal artifact-modal" class:html-preview=is_html on:click=|ev| ev.stop_propagation()>
                <div class="am-head">
                    <span class="am-name">{name.clone()}</span>
                    {(can_prev || can_next).then(|| view! {
                        <div class="am-nav">
                            <button type="button" class="icon-btn am-nav-btn"
                                disabled=!can_prev
                                aria-label=move || t(locale.get(), "artifact.prev_image")
                                title=move || format!("{} (←)", t(locale.get(), "artifact.prev_image"))
                                on:click=move |_| on_prev.call(())>{compose_icon("chevron-left")}</button>
                            <button type="button" class="icon-btn am-nav-btn"
                                disabled=!can_next
                                aria-label=move || t(locale.get(), "artifact.next_image")
                                title=move || format!("{} (→)", t(locale.get(), "artifact.next_image"))
                                on:click=move |_| on_next.call(())>{compose_icon("chevron-right")}</button>
                        </div>
                    })}
                    <div class="spacer"></div>
                    {can_star.then(|| view! {
                        <button type="button" class="icon-btn" class:starred=move || starred.get()
                            disabled=click_session.is_none()
                            title=move || t(locale.get(), if starred.get() { "library.remove" } else { "library.add" })
                            aria-label=move || t(locale.get(), if starred.get() { "library.remove" } else { "library.add" })
                            aria-pressed=move || starred.get().to_string()
                            on:click=move |_| {
                                let Some(session_id) = click_session.clone() else { return; };
                                let existing = library_items.with_untracked(|items| {
                                    items
                                        .iter()
                                        .find(|item| {
                                            item.matches_figure(&session_id, &click_path)
                                        })
                                        .cloned()
                                });
                                let path = click_path.clone();
                                let name = click_name.clone();
                                spawn_local(async move {
                                    let (command, args) = match existing {
                                        Some(item) => (
                                            "delete_library_item",
                                            serde_json::json!({ "id": item.id }),
                                        ),
                                        None => (
                                            "star_library_figure",
                                            serde_json::json!({
                                                "sessionId": session_id,
                                                "path": path,
                                                "name": name,
                                            }),
                                        ),
                                    };
                                    if invoke_checked(command, to_value(&args).unwrap()).await.is_ok() {
                                        on_library_changed.call(());
                                    }
                                });
                            }>
                            {move || compose_icon(if starred.get() { "star-filled" } else { "star" })}
                        </button>
                    })}
                    <button type="button" class="icon-btn"
                        aria-label=move || t(locale.get(), "center.open_file")
                        title=move || t(locale.get(), "center.open_file")
                        on:click=move |_| on_open_center.call(center_artifact.clone())>{compose_icon("expand")}</button>
                    <button class="icon-btn" title=move || t(locale.get(), "artifact.download")
                        on:click=move |_| download_artifact(path_dl.clone())>{compose_icon("download")}</button>
                    <button class="icon-btn" title=move || t(locale.get(), "right.close")
                        on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
                </div>
                <div class="am-figure" class:zoomable-preview=is_zoomable
                    class:docx-preview=is_docx class:office-preview=is_office
                    data-file-path=path_head.clone()>
                    <WorkspaceFilePreview dom_id=dom_id path=path_head.clone() kind=kind.clone() />
                </div>
                <div class="am-tabs">
                    {["code","log","inputs","env"].iter().map(|k| {
                        let k = *k;
                        let label_key = format!("artifact.tab.{k}");
                        view! {
                            <button class="am-tab" class:active=move || tab.get()==k
                                on:click=move |_| tab.set(k)>
                                {move || t(locale.get(), &label_key)}</button>
                        }
                    }).collect_view()}
                    {move || {
                        (tab.get() == "code")
                            .then(|| prov.get())
                            .flatten()
                            .filter(|p| !p.code.is_empty())
                            .map(|p| {
                                let code = p.code;
                                let draft_seed = code.clone();
                                view! {
                                    <button type="button" class="icon-btn" style="margin-left: auto"
                                        title=move || t(locale.get(), "tool.copy_code")
                                        aria-label=move || t(locale.get(), "tool.copy_code")
                                        on:click=move |_| copy_text(code.clone())>
                                        {compose_icon("copy")}
                                    </button>
                                    <button type="button" class="icon-btn" class:active=move || editing_code.get()
                                        title=move || t(locale.get(), "artifact.edit_code")
                                        aria-label=move || t(locale.get(), "artifact.edit_code")
                                        on:click=move |_| {
                                            if !editing_code.get_untracked() {
                                                code_draft.set(draft_seed.clone());
                                            }
                                            editing_code.update(|v| *v = !*v);
                                        }>
                                        {compose_icon("edit")}
                                    </button>
                                }
                            })
                    }}
                </div>
                <div class="am-panel">
                    {move || {
                        let loc = locale.get();
                        if !loaded.get() { return view! { <div class="rp-heavy">{t(loc,"loading")}</div> }.into_view(); }
                        let Some(p) = prov.get() else {
                            return view! { <div class="am-empty">{t(loc,"artifact.none")}</div> }.into_view();
                        };
                        match tab.get() {
                            "code" if editing_code.get() => {
                                let lang = p.language.clone();
                                let send_name = rerun_name.clone();
                                view! {
                                    <textarea class="am-edit-area" prop:value=move || code_draft.get()
                                        on:input=move |ev| code_draft.set(event_target_value(&ev))></textarea>
                                    <div class="am-edit-actions">
                                        <button type="button" class="btn-ghost"
                                            on:click=move |_| editing_code.set(false)>
                                            {t(loc, "library.cancel")}
                                        </button>
                                        <button type="button" class="btn-primary"
                                            on:click=move |_| {
                                                let code = code_draft.get_untracked();
                                                if code.trim().is_empty() {
                                                    return;
                                                }
                                                let message = format!(
                                                    "{}\n```{}\n{}\n```",
                                                    tf(loc, "artifact.rerun_prompt", &[("name", &send_name)]),
                                                    lang,
                                                    code.trim_end(),
                                                );
                                                on_rerun.call(message);
                                            }>
                                            {t(loc, "artifact.rerun_send")}
                                        </button>
                                    </div>
                                }.into_view()
                            }
                            "code" => view! { <RpCodeView lang=p.language.clone() body=p.code.clone() /> }.into_view(),
                            "log" => view! { <pre class="am-log">{p.output.clone()}</pre> }.into_view(),
                            "inputs" => view! {
                                <div class="am-inputs">
                                    {p.inputs.iter().map(|i| {
                                        let ip = i.path.clone();
                                        let linkable = i.produced_here;
                                        let open = on_open_path;
                                        view! {
                                            <button class="am-input" class:linkable=linkable
                                                on:click=move |_| if linkable {
                                                    let kind = file_kind(&ip).unwrap_or("text").to_string();
                                                    open.call((ip.clone(), kind));
                                                }>
                                                {i.path.clone()}</button>
                                        }
                                    }).collect_view()}
                                </div>
                            }.into_view(),
                            _ => match p.env.clone() {
                                None => view! { <div class="am-empty">{t(loc,"artifact.env.none")}</div> }.into_view(),
                                Some(env) => view! {
                                    <table class="am-env">
                                        {env.packages.iter().map(|pk| view! {
                                            <tr><td>{pk.name.clone()}</td><td>{pk.version.clone()}</td></tr>
                                        }).collect_view()}
                                    </table>
                                }.into_view(),
                            },
                        }
                    }}
                </div>
            </div>
        </div>
    }
}
