use crate::agent_workflows::{workflow_studio as workflow_studio_view, AgentPanelState};
use crate::app_support::{
    allow_drop, apply_base_url_suggestions, blank_conn_secret_field, build_conn_json,
    close_details_ancestor, compose_icon, conn_form_from_row, context_capability_summary,
    drag_session_id, endpoint_has_stored_key, focus_element_soon, format_relative_time,
    import_custom_css_from_input, join_tags, js_error_text, model_form_entry, new_acp_form,
    new_conn_form, new_model_form, profile_to_form, provider_entries_are_pristine,
    quick_action_label, reviewer_backend_key, reviewer_backend_label,
    reviewer_missing_acp_profile_id, set_reviewer_backend, settings_section_label,
    settings_subpage_label, show_toast, skill_matches_filter, start_session_drag,
    DefaultAnalysisSelect, CRED_GROUPS,
};
use crate::bindings::{invoke, invoke_checked, is_mac, is_windows, open_external_url};
use crate::dto::*;
use crate::i18n::{localize_backend, set_document_lang, t, tf, use_locale, Locale};
use crate::text::{
    dom_value, endpoint_host, event_target_checked, event_target_input, event_target_value,
    format_bytes, join_api_url,
};
use crate::window_capture_escape;
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::{BTreeSet, HashMap, HashSet};
use wasm_bindgen::JsValue;

fn connector_parameter_type(schema: &serde_json::Value) -> String {
    match schema.get("type") {
        Some(serde_json::Value::String(kind)) => kind.clone(),
        Some(serde_json::Value::Array(kinds)) => kinds.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" | "),
        _ => String::new(),
    }
}

#[component]
fn ConnectorToolDocumentation(locale: RwSignal<Locale>, tool: ConnectorTool) -> impl IntoView {
    let description = if tool.description.trim().is_empty() {
        t(locale.get(), "conn.description_missing").to_string()
    } else { tool.description };
    view! {
        <div class="conn-tool-documentation" data-testid="connector-tool-documentation">
            <p class="conn-tool-description">{description}</p>
            {tool.input_schema.map(|schema| {
                let properties = schema.get("properties").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                let required = schema.get("required").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let examples = schema.get("examples").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                view! {
                    <h4>{move || t(locale.get(), "conn.parameters")}</h4>
                    {if properties.is_empty() {
                        view! { <p class="hint">{move || t(locale.get(), "conn.no_named_parameters")}</p> }.into_view()
                    } else {
                        view! {
                            <div class="conn-parameters-scroll">
                                <table class="conn-parameters">
                                    <thead><tr>
                                        <th>{move || t(locale.get(), "conn.parameter")}</th>
                                        <th>{move || t(locale.get(), "conn.parameter_type")}</th>
                                        <th>{move || t(locale.get(), "conn.parameter_description")}</th>
                                    </tr></thead>
                                    <tbody>{properties.into_iter().map(|(name, property)| {
                                        let is_required = required.iter().any(|value| value.as_str() == Some(&name));
                                        let kind = connector_parameter_type(&property);
                                        let description = property.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                                        let default = property.get("default").map(|value| value.to_string());
                                        view! { <tr>
                                            <td><code>{name}</code>{is_required.then(|| view! { <span class="conn-required">{move || t(locale.get(), "conn.required")}</span> })}</td>
                                            <td><code>{kind}</code></td>
                                            <td>{description}{default.map(|value| view! { <div class="hint">{move || t(locale.get(), "conn.default_value")} ": " <code>{value}</code></div> })}</td>
                                        </tr> }
                                    }).collect_view()}</tbody>
                                </table>
                            </div>
                        }.into_view()
                    }}
                    <details class="conn-schema"><summary>{move || t(locale.get(), "conn.input_schema")}</summary>
                        <pre>{serde_json::to_string_pretty(&schema).unwrap_or_default()}</pre>
                    </details>
                    {(!examples.is_empty()).then(|| view! {
                        <h4>{move || t(locale.get(), "conn.examples")}</h4>
                        <pre>{serde_json::to_string_pretty(&examples).unwrap_or_default()}</pre>
                    })}
                }
            })}
            {tool.output_schema.map(|schema| view! {
                <details class="conn-schema"><summary>{move || t(locale.get(), "conn.output_schema")}</summary>
                    <pre>{serde_json::to_string_pretty(&schema).unwrap_or_default()}</pre>
                </details>
            })}
        </div>
    }
}

/// Pending "确定删除?" confirmation. Both models and ACP agents route through
/// one overlay so the confirm gate lives in a single place. The signal is owned
/// by the app so the window-level Escape stack can close it before settings.
#[derive(Clone)]
pub(super) enum DeleteConfirm {
    Model {
        id: String,
        label: String,
    },
    Acp {
        id: String,
        label: String,
    },
    Plugin {
        id: String,
        version: String,
        label: String,
    },
    Skill {
        name: String,
        label: String,
    },
    /// Dropping a server: `detail` reports what would be abandoned there.
    Host {
        alias: String,
        label: String,
        detail: String,
    },
}

impl DeleteConfirm {
    fn label(&self) -> &str {
        match self {
            DeleteConfirm::Model { label, .. }
            | DeleteConfirm::Acp { label, .. }
            | DeleteConfirm::Plugin { label, .. }
            | DeleteConfirm::Skill { label, .. }
            | DeleteConfirm::Host { label, .. } => label,
        }
    }
}

fn trust_alias(context_id: &str) -> &str {
    context_id.strip_prefix("ssh:").unwrap_or(context_id)
}

/// i18n label for a `get_storage_usage` entry key.
fn storage_entry_label_key(key: &str) -> &'static str {
    match key {
        "database" => "settings.storage.database",
        "python" => "settings.storage.python",
        "plugins" => "settings.storage.plugins",
        "workspace" => "settings.storage.workspace",
        _ => "settings.storage.other",
    }
}

const USAGE_SESSION_PAGE_SIZE: usize = 20;
const USAGE_MODEL_COLORS: [&str; 8] = [
    "#7c3aed", "#0ea5e9", "#10b981", "#f59e0b", "#ef4444", "#ec4899", "#6366f1", "#64748b",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsageActivityMode {
    Daily,
    Weekly,
    Cumulative,
}

#[derive(Clone, Debug, PartialEq)]
struct UsageActivityCell {
    period: String,
    tokens: i64,
    level: u8,
    future: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct UsageModelSlice {
    label: String,
    tokens: i64,
    color: &'static str,
}

fn usage_level(value: i64, max: i64) -> u8 {
    if value <= 0 || max <= 0 {
        0
    } else {
        (((value as f64 / max as f64) * 4.0).ceil() as u8).clamp(1, 4)
    }
}

fn usage_activity_cells(days: &[TokenUsageDay], mode: UsageActivityMode) -> Vec<UsageActivityCell> {
    if mode == UsageActivityMode::Daily {
        let max = days
            .iter()
            .filter(|day| !day.future)
            .map(|day| day.tokens)
            .max()
            .unwrap_or(0);
        return days
            .iter()
            .map(|day| UsageActivityCell {
                period: day.date.clone(),
                tokens: day.tokens,
                level: if day.future {
                    0
                } else {
                    usage_level(day.tokens, max)
                },
                future: day.future,
            })
            .collect();
    }

    let weekly = days
        .chunks(7)
        .map(|week| {
            week.iter()
                .filter(|day| !day.future)
                .map(|day| day.tokens.max(0))
                .sum::<i64>()
        })
        .collect::<Vec<_>>();
    let amounts = if mode == UsageActivityMode::Cumulative {
        let mut running = 0i64;
        weekly
            .iter()
            .map(|tokens| {
                running = running.saturating_add(*tokens);
                running
            })
            .collect::<Vec<_>>()
    } else {
        weekly
    };
    let max = amounts.iter().copied().max().unwrap_or(0);
    days.chunks(7)
        .zip(amounts)
        .flat_map(|(week, tokens)| {
            let fill = if tokens <= 0 || max <= 0 {
                0
            } else {
                ((tokens as f64 / max as f64 * week.len() as f64).ceil() as usize)
                    .clamp(1, week.len())
            };
            let start = week.first().map(|day| day.date.as_str()).unwrap_or("");
            let end = week
                .iter()
                .rev()
                .find(|day| !day.future)
                .or_else(|| week.last())
                .map(|day| day.date.as_str())
                .unwrap_or("");
            let period = if mode == UsageActivityMode::Cumulative {
                end.to_string()
            } else {
                format!("{start} – {end}")
            };
            (0..week.len())
                .map(move |row| UsageActivityCell {
                    period: period.clone(),
                    tokens,
                    level: if row + fill >= week.len() {
                        usage_level(tokens, max).max(1)
                    } else {
                        0
                    },
                    future: false,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn usage_activity_months(days: &[TokenUsageDay]) -> Vec<(usize, u32)> {
    let mut previous = None;
    days.chunks(7)
        .enumerate()
        .filter_map(|(week, days)| {
            let month = days.first()?.date.get(5..7)?.parse::<u32>().ok()?;
            if previous == Some(month) {
                None
            } else {
                previous = Some(month);
                Some((week, month))
            }
        })
        .collect()
}

fn usage_month_label(month: u32, locale: Locale) -> String {
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    if locale == Locale::Zh {
        format!("{month}月")
    } else {
        EN.get(month.saturating_sub(1) as usize)
            .copied()
            .unwrap_or("")
            .to_string()
    }
}

fn usage_model_slices(
    rows: &[ModelTokenUsage],
    profiles: &[ModelProfile],
    unknown: &str,
    other: &str,
) -> Vec<UsageModelSlice> {
    let mut merged = HashMap::<String, i64>::new();
    for row in rows.iter().filter(|row| row.tokens > 0) {
        let label = profiles
            .iter()
            .find(|profile| profile.id == row.model)
            .map(|profile| {
                if profile.model.trim().is_empty() {
                    profile.label.as_str()
                } else {
                    profile.model.as_str()
                }
            })
            .unwrap_or_else(|| {
                if row.model == "unknown" {
                    unknown
                } else {
                    row.model.as_str()
                }
            })
            .to_string();
        *merged.entry(label).or_insert(0) += row.tokens;
    }
    let mut totals = merged.into_iter().collect::<Vec<_>>();
    totals.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    if totals.len() > 8 {
        let remainder = totals.drain(7..).map(|(_, tokens)| tokens).sum();
        totals.push((other.to_string(), remainder));
    }
    totals
        .into_iter()
        .enumerate()
        .map(|(index, (label, tokens))| UsageModelSlice {
            label,
            tokens,
            color: USAGE_MODEL_COLORS[index % USAGE_MODEL_COLORS.len()],
        })
        .collect()
}

fn usage_model_gradient(slices: &[UsageModelSlice]) -> String {
    let total = slices.iter().map(|slice| slice.tokens.max(0)).sum::<i64>();
    if total <= 0 {
        return "var(--bg-sunken)".into();
    }
    let mut start = 0.0;
    let segments = slices
        .iter()
        .map(|slice| {
            let end = start + slice.tokens.max(0) as f64 / total as f64 * 100.0;
            let segment = format!("{} {start:.3}% {end:.3}%", slice.color);
            start = end;
            segment
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("conic-gradient({segments})")
}

#[derive(Clone)]
struct UsageToolRankRow {
    kind: String,
    name: String,
    calls: i64,
    color: &'static str,
}

fn usage_tool_rank_rows(rows: &[ToolCallUsage], other: &str) -> Vec<UsageToolRankRow> {
    let mut totals = rows
        .iter()
        .filter(|row| row.calls > 0 && (row.kind == "skill" || row.kind == "mcp"))
        .map(|row| (row.kind.clone(), row.name.clone(), row.calls))
        .collect::<Vec<_>>();
    totals.sort_by(|left, right| {
        right
            .2
            .cmp(&left.2)
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.1.cmp(&right.1))
    });
    if totals.len() > 10 {
        let remainder = totals.drain(9..).map(|(_, _, calls)| calls).sum();
        totals.push(("other".into(), other.to_string(), remainder));
    }
    totals
        .into_iter()
        .enumerate()
        .map(|(index, (kind, name, calls))| UsageToolRankRow {
            kind,
            name,
            calls,
            color: USAGE_MODEL_COLORS[index % USAGE_MODEL_COLORS.len()],
        })
        .collect()
}

fn usage_summary_view(loc: Locale, totals: (i64, i64, i64, i64)) -> impl IntoView {
    let tokens = |value: i64| crate::fmt_tokens(value.max(0) as u64);
    view! {
        <div class="usage-summary">
            {[
                ("settings.usage.input", totals.0),
                ("settings.usage.output", totals.1),
                ("settings.usage.reasoning", totals.2),
                ("settings.usage.cached", totals.3),
            ].into_iter().map(|(key, value)| view! {
                <div class="usage-tile">
                    <span class="usage-tile-value">{tokens(value)}</span>
                    <span class="usage-tile-label">{t(loc, key)}</span>
                </div>
            }).collect_view()}
        </div>
    }
}

#[cfg(test)]
mod usage_dashboard_tests {
    use super::*;

    fn days(first_week: i64, second_week: i64) -> Vec<TokenUsageDay> {
        (0..14)
            .map(|index| TokenUsageDay {
                date: format!("2026-07-{:02}", index + 1),
                tokens: if index == 6 {
                    first_week
                } else if index == 13 {
                    second_week
                } else {
                    0
                },
                future: false,
            })
            .collect()
    }

    #[test]
    fn activity_modes_keep_daily_values_and_aggregate_weeks() {
        let days = days(10, 30);
        let daily = usage_activity_cells(&days, UsageActivityMode::Daily);
        assert_eq!(daily.len(), 14);
        assert_eq!(daily[6].tokens, 10);
        assert!(daily[13].level > daily[6].level);

        let weekly = usage_activity_cells(&days, UsageActivityMode::Weekly);
        assert_eq!(weekly[..7].iter().filter(|cell| cell.level > 0).count(), 3);
        assert_eq!(weekly[7..].iter().filter(|cell| cell.level > 0).count(), 7);

        let cumulative = usage_activity_cells(&days, UsageActivityMode::Cumulative);
        assert_eq!(cumulative[0].tokens, 10);
        assert_eq!(cumulative[7].tokens, 40);
    }

    #[test]
    fn model_pie_caps_a_long_tail_as_other() {
        let rows = (1..=9)
            .map(|index| ModelTokenUsage {
                model: format!("model-{index}"),
                tokens: index,
            })
            .collect::<Vec<_>>();
        let slices = usage_model_slices(&rows, &[], "Unknown", "Other");
        assert_eq!(slices.len(), 8);
        assert_eq!(slices.last().unwrap().label, "Other");
        assert_eq!(slices.last().unwrap().tokens, 3);
        assert!(usage_model_gradient(&slices).starts_with("conic-gradient("));
    }

    #[test]
    fn tool_rank_keeps_skill_and_mcp_and_caps_tail() {
        let mut rows = vec![
            ToolCallUsage {
                kind: "skill".into(),
                name: "bear-support".into(),
                calls: 5,
            },
            ToolCallUsage {
                kind: "mcp".into(),
                name: "pubmed_search".into(),
                calls: 3,
            },
            ToolCallUsage {
                kind: "shell".into(),
                name: "shell".into(),
                calls: 9,
            },
        ];
        rows.extend((1..=10).map(|index| ToolCallUsage {
            kind: "mcp".into(),
            name: format!("tool-{index}"),
            calls: 1,
        }));
        let ranked = usage_tool_rank_rows(&rows, "Other");
        assert_eq!(ranked.len(), 10);
        assert_eq!(ranked[0].name, "bear-support");
        assert_eq!(ranked[0].kind, "skill");
        assert_eq!(ranked.last().unwrap().name, "Other");
        assert_eq!(ranked.last().unwrap().calls, 3);
    }
}

fn valid_sha256(value: &str) -> bool {
    let value = value.trim();
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn settings_provider_value(provider: &str) -> &'static str {
    match provider.trim() {
        "anthropic" => "anthropic",
        "openai_responses" | "openai-responses" | "responses" => "openai_responses",
        _ => "openai",
    }
}

/// Every effort value any supported provider understands; shown when the
/// model is not in the curated table below.
pub(crate) const ALL_EFFORT_VALUES: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

/// Curated reasoning-effort support per model family, per vendor docs as of
/// 2026-08 (OpenAI reasoning guide, Anthropic effort docs, xAI reasoning
/// docs, DeepSeek/Moonshot/Alibaba API references). `None` = unknown model
/// (full list + "can't verify" hint); `Some(&[])` = the provider rejects the
/// parameter for this model, so only "default" makes sense.
/// ponytail: the baked model catalog (model_catalog.rs) already carries
/// per-model effort values from models.dev; swap this table for catalog
/// lookups in a follow-up. Keep longer patterns above their shorter
/// siblings ("claude-opus-4-5" before "claude-opus").
pub(crate) fn known_effort_values(_provider: &str, model: &str) -> Option<&'static [&'static str]> {
    // Users write model names loosely ("opus-4.8", "claude-opus-4-8"), so
    // match on a normalized form and don't require the vendor prefix.
    let m = model.to_ascii_lowercase().replace(['.', '_'], "-");
    if m.contains("gpt-5-pro") {
        Some(&["high"])
    } else if m.contains("codex-max") {
        Some(&["none", "low", "medium", "high", "xhigh"])
    } else if m.contains("gpt-5-1") {
        Some(&["none", "low", "medium", "high"])
    } else if m.contains("gpt-5-6") {
        Some(&["none", "low", "medium", "high", "xhigh", "max"])
    } else if m.contains("gpt-5-2")
        || m.contains("gpt-5-3")
        || m.contains("gpt-5-4")
        || m.contains("gpt-5-5")
    {
        Some(&["none", "low", "medium", "high", "xhigh"])
    } else if m.contains("gpt-5") {
        Some(&["minimal", "low", "medium", "high"])
    } else if m.starts_with("o1-mini") || m.starts_with("o1-preview") {
        Some(&[])
    } else if m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") {
        Some(&["low", "medium", "high"])
    } else if m.contains("opus-4-5") {
        Some(&["low", "medium", "high"])
    } else if m.contains("sonnet-4-5") || m.contains("haiku") {
        // These reject the effort parameter with a 400.
        Some(&[])
    } else if m.contains("opus-4-6") || m.contains("sonnet-4-6") || m.contains("mythos-preview") {
        Some(&["low", "medium", "high", "max"])
    } else if m.contains("opus")
        || m.contains("sonnet")
        || m.contains("fable")
        || m.contains("mythos")
    {
        Some(&["low", "medium", "high", "xhigh", "max"])
    } else if m.contains("grok-4-6") || m.contains("grok-4-20") {
        Some(&["low", "medium", "high", "xhigh"])
    } else if m.contains("grok-4") {
        Some(&["low", "medium", "high"])
    } else if m.contains("grok") {
        Some(&["low", "high"])
    } else if m.contains("deepseek-v4") {
        // medium/xhigh are silently down-mapped to high; don't offer them.
        Some(&["low", "high", "max"])
    } else if m.contains("kimi-k3") {
        Some(&["low", "high", "max"])
    } else if m.contains("kimi-k2") {
        // k2.x only toggles thinking on/off; no effort parameter.
        Some(&[])
    } else if m.contains("qwen3-8-max") {
        Some(&["low", "medium", "xhigh"])
    } else {
        None
    }
}

#[cfg(test)]
mod effort_values_tests {
    use super::known_effort_values;

    #[test]
    fn maps_families_and_leaves_unknown_open() {
        assert_eq!(
            known_effort_values("anthropic", "claude-sonnet-5"),
            Some(&["low", "medium", "high", "xhigh", "max"][..])
        );
        assert_eq!(
            known_effort_values("anthropic", "claude-opus-4-5"),
            Some(&["low", "medium", "high"][..])
        );
        assert_eq!(
            known_effort_values("anthropic", "claude-sonnet-4-6"),
            Some(&["low", "medium", "high", "max"][..])
        );
        assert_eq!(
            known_effort_values("anthropic", "claude-haiku-4-5"),
            Some(&[][..])
        );
        assert_eq!(
            known_effort_values("anthropic", "claude-sonnet-4-5"),
            Some(&[][..])
        );
        assert_eq!(
            known_effort_values("openai", "gpt-5.1-codex-max"),
            Some(&["none", "low", "medium", "high", "xhigh"][..])
        );
        assert_eq!(
            known_effort_values("openai_responses", "gpt-5.1"),
            Some(&["none", "low", "medium", "high"][..])
        );
        assert_eq!(
            known_effort_values("openai_responses", "gpt-5.6"),
            Some(&["none", "low", "medium", "high", "xhigh", "max"][..])
        );
        assert_eq!(
            known_effort_values("openai", "gpt-5-pro"),
            Some(&["high"][..])
        );
        assert_eq!(
            known_effort_values("openai", "o3-mini"),
            Some(&["low", "medium", "high"][..])
        );
        assert_eq!(known_effort_values("openai", "o1-mini"), Some(&[][..]));
        // Loose user spelling (no vendor prefix, dots) matches the same family.
        assert_eq!(
            known_effort_values("anthropic", "opus-4.8"),
            Some(&["low", "medium", "high", "xhigh", "max"][..])
        );
        assert_eq!(
            known_effort_values("openai", "grok-4.6"),
            Some(&["low", "medium", "high", "xhigh"][..])
        );
        assert_eq!(
            known_effort_values("openai", "grok-4"),
            Some(&["low", "medium", "high"][..])
        );
        assert_eq!(
            known_effort_values("openai", "deepseek-v4-pro"),
            Some(&["low", "high", "max"][..])
        );
        assert_eq!(
            known_effort_values("openai", "kimi-k3"),
            Some(&["low", "high", "max"][..])
        );
        assert_eq!(known_effort_values("openai", "kimi-k2.5"), Some(&[][..]));
        assert_eq!(
            known_effort_values("openai", "qwen3.8-max-preview"),
            Some(&["low", "medium", "xhigh"][..])
        );
        assert_eq!(known_effort_values("openai", "some-future-model"), None);
    }
}

/// Fill documented limits from the baked model catalog (models.dev, compiled
/// in by build.rs). Exact id match only — a family id never absorbs a longer
/// sibling (`kimi-k3` vs `k3-256k`). Unknown models keep whatever the form
/// already has; the backend clamps authoritatively on save regardless of what
/// this preview does.
fn apply_catalog_limits(
    model_form: RwSignal<Option<ModelForm>>,
    catalog_limits: RwSignal<Option<CatalogEntryDto>>,
) {
    // A stale entry must never survive a model edit.
    catalog_limits.set(None);
    let Some(current) = model_form.get() else {
        return;
    };
    let (provider, api_url, model) = (
        current.provider,
        join_api_url(&current.api_url, &current.endpoint_suffix),
        current.model,
    );
    if model.trim().is_empty()
        || is_image_generation_model(&model)
        || is_video_generation_model(&model)
    {
        return;
    }
    spawn_local(async move {
        let args = to_value(&serde_json::json!({
            "provider": provider,
            "apiUrl": api_url,
            "model": model,
        }))
        .unwrap();
        let dto = invoke_checked("model_catalog_lookup", args)
            .await
            .ok()
            .and_then(|v| serde_wasm_bindgen::from_value::<Option<CatalogEntryDto>>(v).ok())
            .flatten();
        // Don't clobber edits made while the lookup was in flight.
        if model_form
            .get()
            .map_or(true, |f| !f.model.trim().eq_ignore_ascii_case(model.trim()))
        {
            return;
        }
        if let Some(dto) = dto {
            model_form.update(|o| {
                if let Some(o) = o {
                    o.max_tokens = dto.max_tokens;
                    o.context_window = dto.context_window;
                }
            });
            catalog_limits.set(Some(dto));
        }
    });
}

/// One-click presets for popular OpenAI-compatible providers (#334):
/// (label, api_url, model). The user only has to paste an API key.
/// The "Coding" entries are the monthly coding-plan endpoints — those
/// subscription keys only work there, not on the pay-per-token URLs.
const MODEL_PRESETS: [(&str, &str, &str); 5] = [
    ("Kimi", "https://api.moonshot.cn/v1", "kimi-k3"),
    ("GLM", "https://open.bigmodel.cn/api/paas/v4", "glm-5"),
    ("DeepSeek", "https://api.deepseek.com", "deepseek-v4-flash"),
    (
        "Kimi Coding",
        "https://api.kimi.com/coding/v1",
        "kimi-coding",
    ),
    (
        "GLM Coding",
        "https://open.bigmodel.cn/api/coding/paas/v4",
        "glm-5.2",
    ),
];

fn appearance_palette_options(dark: bool) -> [(&'static str, &'static str); 5] {
    if dark {
        [
            ("charcoal", "Wisp Charcoal"),
            ("codex", "Codex"),
            ("github", "GitHub Dark"),
            ("catppuccin", "Catppuccin Mocha"),
            ("gruvbox", "Gruvbox"),
        ]
    } else {
        [
            ("paper", "Wisp Paper"),
            ("codex", "Codex"),
            ("github", "GitHub"),
            ("catppuccin", "Catppuccin Latte"),
            ("everforest", "Everforest"),
        ]
    }
}

fn appearance_palette_meta(
    dark: bool,
    palette: &str,
) -> (&'static str, &'static str, &'static str) {
    match (dark, palette) {
        (false, "codex") => ("#2563EB", "#F4F6F8", "#172033"),
        (false, "github") => ("#0969DA", "#F6F8FA", "#1F2328"),
        (false, "catppuccin") => ("#8839EF", "#EFF1F5", "#4C4F69"),
        (false, "everforest") => ("#3A8F6B", "#F4F0D9", "#2F383E"),
        (true, "codex") => ("#7C8CFF", "#202123", "#F3F4F6"),
        (true, "github") => ("#58A6FF", "#0D1117", "#F0F6FC"),
        (true, "catppuccin") => ("#CBA6F7", "#1E1E2E", "#CDD6F4"),
        (true, "gruvbox") => ("#D79921", "#282828", "#EBDBB2"),
        (true, _) => ("#2DA898", "#171614", "#EBE8E2"),
        _ => ("#0D9488", "#FAF9F6", "#141413"),
    }
}

#[derive(Clone, Copy)]
pub(super) struct SettingsViewState {
    pub(super) locale: RwSignal<Locale>,
    pub(super) theme_mode: RwSignal<String>,
    pub(super) light_palette: RwSignal<String>,
    pub(super) dark_palette: RwSignal<String>,
    pub(super) ui_font_size: RwSignal<u16>,
    pub(super) code_font_size: RwSignal<u16>,
    pub(super) ui_font_family: RwSignal<String>,
    pub(super) code_font_family: RwSignal<String>,
    pub(super) selection_popup_enabled: RwSignal<bool>,
    pub(super) send_with_modifier: RwSignal<bool>,
    pub(super) custom_css: RwSignal<String>,
    pub(super) update_check_enabled: RwSignal<bool>,
    pub(super) show_settings: RwSignal<bool>,
    pub(super) settings_section: RwSignal<String>,
    pub(super) open_conn_key: RwSignal<Option<String>>,
    pub(super) channels_open: RwSignal<Option<String>>,
    pub(super) connectors: RwSignal<Option<ConnectorsView>>,
    pub(super) model_form: RwSignal<Option<ModelForm>>,
    pub(super) model_catalog_limits: RwSignal<Option<CatalogEntryDto>>,
    pub(super) conn_form: RwSignal<Option<ConnForm>>,
    pub(super) memory_selected: RwSignal<Option<String>>,
    pub(super) specialist_form: RwSignal<Option<Specialist>>,
    pub(super) settings: RwSignal<Settings>,
    pub(super) bootstrap: RwSignal<Option<BootstrapStatus>>,
    pub(super) settings_message: RwSignal<Option<(bool, String)>>,
    pub(super) settings_busy: RwSignal<bool>,
    pub(super) model_form_open: Memo<bool>,
    pub(super) model_form_key: RwSignal<String>,
    pub(super) models: RwSignal<Vec<ModelProfile>>,
    pub(super) model_form_msg: RwSignal<Option<(bool, String)>>,
    pub(super) show_acp_agents: RwSignal<bool>,
    pub(super) acp_agents: RwSignal<Vec<AcpAgentProfile>>,
    pub(super) active_acp_agent_id: RwSignal<Option<String>>,
    pub(super) acp_form: RwSignal<Option<AcpAgentProfile>>,
    pub(super) acp_form_msg: RwSignal<Option<(bool, String)>>,
    pub(super) acp_infos: RwSignal<HashMap<String, AcpAgentInfo>>,
    pub(super) specialists: RwSignal<Vec<Specialist>>,
    pub(super) quick_actions: RwSignal<Vec<QuickAction>>,
    pub(super) workflow_templates: RwSignal<Vec<WorkflowTemplate>>,
    pub(super) workflow_studio: AgentPanelState,
    pub(super) selected_workflow_template: RwSignal<Option<String>>,
    pub(super) specialist_form_open: Memo<bool>,
    pub(super) memory_view: RwSignal<Option<MemoryView>>,
    pub(super) memory_editor: RwSignal<String>,
    pub(super) memory_msg: RwSignal<Option<(bool, String)>>,
    pub(super) skills_list: RwSignal<Vec<SkillRow>>,
    pub(super) skill_filter_tag: RwSignal<String>,
    pub(super) skills_search: RwSignal<String>,
    pub(super) skills_msg: RwSignal<Option<(bool, String)>>,
    pub(super) plugins_list: RwSignal<Vec<PluginRow>>,
    pub(super) plugins_msg: RwSignal<Option<(bool, String)>>,
    pub(super) plugin_install_open: RwSignal<bool>,
    pub(super) cred_status: RwSignal<HashMap<String, bool>>,
    pub(super) cred_inputs: RwSignal<HashMap<String, String>>,
    pub(super) custom_credentials: RwSignal<Vec<CustomCredentialStatus>>,
    pub(super) cred_msg: RwSignal<Option<(bool, String)>>,
    pub(super) approval_grants: RwSignal<Vec<ApprovalGrantRow>>,
    pub(super) conns_view: RwSignal<Option<ConnView>>,
    pub(super) conn_form_open: Memo<bool>,
    pub(super) conn_form_kind: Memo<String>,
    pub(super) conn_test_msg: RwSignal<Option<(bool, String)>>,
    pub(super) custom_conn_tools: RwSignal<HashMap<String, Vec<ConnectorTool>>>,
    pub(super) custom_conn_tools_loading: RwSignal<HashSet<String>>,
    pub(super) custom_conn_tool_errors: RwSignal<HashMap<String, String>>,
    pub(super) pet_status: RwSignal<PetStatus>,
    pub(super) ssh_hosts: RwSignal<Vec<SshHost>>,
    pub(super) execution_contexts: RwSignal<Vec<ExecutionContext>>,
    pub(super) default_execution_context: RwSignal<Option<String>>,
    pub(super) runtime_interpreter_form: RwSignal<Option<RuntimeInterpreterForm>>,
    pub(super) probing_context_id: RwSignal<Option<String>>,
    pub(super) delete_confirm: RwSignal<Option<DeleteConfirm>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnSecretKind {
    Env,
    Headers,
}

fn conn_secret_rows(form: &ConnForm, kind: ConnSecretKind) -> &[ConnSecretField] {
    match kind {
        ConnSecretKind::Env => &form.env,
        ConnSecretKind::Headers => &form.headers,
    }
}

fn conn_secret_rows_mut(form: &mut ConnForm, kind: ConnSecretKind) -> &mut Vec<ConnSecretField> {
    match kind {
        ConnSecretKind::Env => &mut form.env,
        ConnSecretKind::Headers => &mut form.headers,
    }
}

/// Keyed secret rows so typing a name/value does not remount the inputs.
#[component]
fn ConnSecretRows(
    conn_form: RwSignal<Option<ConnForm>>,
    kind: ConnSecretKind,
    #[prop(into)] disabled: MaybeSignal<bool>,
) -> impl IntoView {
    let locale = use_locale();
    view! {
        <For
            each=move || {
                conn_form.with(|form| {
                    form.as_ref()
                        .map(|form| {
                            conn_secret_rows(form, kind)
                                .iter()
                                .map(|row| row.row_id)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
            }
            key=|id| *id
            let:row_id
        >
            {
                let reveal = create_rw_signal(false);
                let value_ref = create_node_ref::<leptos::html::Input>();
                view! {
                    <div class="conn-secret-row" data-testid="conn-secret-row">
                        <input
                            data-testid="conn-secret-name"
                            autocomplete="off"
                            placeholder=move || match kind {
                                ConnSecretKind::Env => "NAME".to_string(),
                                ConnSecretKind::Headers => {
                                    if conn_form.with(|form| {
                                        form.as_ref().is_some_and(|form| form.auth == "oauth")
                                    }) {
                                        "X-Custom-Header".to_string()
                                    } else {
                                        "Authorization".to_string()
                                    }
                                }
                            }
                            prop:value=move || {
                                conn_form.with(|form| {
                                    form.as_ref()
                                        .and_then(|form| {
                                            conn_secret_rows(form, kind)
                                                .iter()
                                                .find(|row| row.row_id == row_id)
                                        })
                                        .map(|row| row.name.clone())
                                        .unwrap_or_default()
                                })
                            }
                            prop:disabled=move || disabled.get()
                            on:input=move |ev| {
                                let value = event_target_input(&ev).value();
                                conn_form.update(|form| if let Some(form) = form {
                                    if let Some(row) = conn_secret_rows_mut(form, kind)
                                        .iter_mut()
                                        .find(|row| row.row_id == row_id)
                                    {
                                        row.name = value;
                                    }
                                });
                            }
                        />
                        <div class="conn-secret-value">
                            <input
                                node_ref=value_ref
                                data-testid="conn-secret-value"
                                attr:type=move || if reveal.get() { "text" } else { "password" }
                                autocomplete="new-password"
                                placeholder=move || {
                                    let has_value = conn_form.with(|form| {
                                        form.as_ref()
                                            .and_then(|form| {
                                                conn_secret_rows(form, kind)
                                                    .iter()
                                                    .find(|row| row.row_id == row_id)
                                            })
                                            .map(|row| row.has_value)
                                            .unwrap_or(false)
                                    });
                                    if has_value {
                                        t(locale.get(), "conn.secret_keep").to_string()
                                    } else if kind == ConnSecretKind::Headers
                                        && conn_form.with(|form| {
                                            form.as_ref().is_some_and(|form| form.auth == "oauth")
                                        })
                                    {
                                        "value".to_string()
                                    } else if kind == ConnSecretKind::Headers {
                                        "Bearer token".to_string()
                                    } else {
                                        t(locale.get(), "conn.secret_value").to_string()
                                    }
                                }
                                prop:value=move || {
                                    conn_form.with(|form| {
                                        form.as_ref()
                                            .and_then(|form| {
                                                conn_secret_rows(form, kind)
                                                    .iter()
                                                    .find(|row| row.row_id == row_id)
                                            })
                                            .map(|row| row.value.clone())
                                            .unwrap_or_default()
                                    })
                                }
                                prop:disabled=move || disabled.get()
                                on:input=move |ev| {
                                    let value = event_target_input(&ev).value();
                                    conn_form.update(|form| if let Some(form) = form {
                                        if let Some(row) = conn_secret_rows_mut(form, kind)
                                            .iter_mut()
                                            .find(|row| row.row_id == row_id)
                                        {
                                            row.value = value;
                                        }
                                    });
                                }
                            />
                            <button
                                type="button"
                                class="conn-secret-reveal"
                                data-testid="conn-secret-reveal"
                                title=move || {
                                    t(
                                        locale.get(),
                                        if reveal.get() {
                                            "conn.secret_hide"
                                        } else {
                                            "conn.secret_show"
                                        },
                                    )
                                }
                                aria-label=move || {
                                    t(
                                        locale.get(),
                                        if reveal.get() {
                                            "conn.secret_hide"
                                        } else {
                                            "conn.secret_show"
                                        },
                                    )
                                }
                                aria-pressed=move || if reveal.get() { "true" } else { "false" }
                                prop:disabled=move || disabled.get()
                                on:click=move |_| {
                                    reveal.update(|on| *on = !*on);
                                    if let Some(input) = value_ref.get() {
                                        let _ = input.focus();
                                    }
                                }
                            >
                                {move || compose_icon(if reveal.get() { "eye-off" } else { "eye" })}
                            </button>
                        </div>
                        <button
                            type="button"
                            class="settings-list-remove"
                            data-testid="conn-secret-remove"
                            title=move || t(locale.get(), "conn.secret_remove")
                            aria-label=move || t(locale.get(), "conn.secret_remove")
                            prop:disabled=move || disabled.get()
                            on:click=move |_| conn_form.update(|form| if let Some(form) = form {
                                conn_secret_rows_mut(form, kind)
                                    .retain(|row| row.row_id != row_id);
                            })
                        >{compose_icon("close")}</button>
                    </div>
                }
            }
        </For>
    }
}

#[component]
pub(super) fn SettingsView(
    state: SettingsViewState,
    open_project: Callback<String>,
    go_settings_section: Callback<String>,
    close_settings_subpage: Callback<()>,
    check_updates: Callback<web_sys::MouseEvent>,
    save_settings: Callback<web_sys::MouseEvent>,
    save_model_form: Callback<web_sys::MouseEvent>,
    save_specialist_form: Callback<web_sys::MouseEvent>,
    test_reviewer_form: Callback<web_sys::MouseEvent>,
    validate_model_form: Callback<web_sys::MouseEvent>,
    start_specialist_chat: Callback<web_sys::MouseEvent>,
    refresh_conns: Callback<()>,
    refresh_skills: Callback<()>,
    reload_skills: Callback<()>,
    refresh_approval_grants: Callback<()>,
    load_memory_file: Callback<String>,
    load_custom_conn_tools: Callback<ConnRow>,
    save_skill_tags: Callback<(String, String)>,
    set_visible_skills_enabled: Callback<bool>,
    install_skill_from: Callback<String>,
    install_plugin_from: Callback<(String, Option<String>)>,
    install_plugin_url: Callback<(String, String)>,
    set_plugin_enabled: Callback<(String, String, bool)>,
    use_plugin: Callback<(String, String, String, Vec<String>, bool)>,
    remove_plugin: Callback<(String, String)>,
    remove_specialist: Callback<String>,
    open_add_host: Callback<()>,
    edit_ssh_host: Callback<String>,
    import_ssh_hosts: Callback<()>,
    import_wsl_contexts: Callback<()>,
    remove_ssh_host: Callback<String>,
    probe_compute_resource: Callback<String>,
    set_default_compute_resource: Callback<Option<String>>,
    open_terminal_session: Callback<TerminalSessionSummary>,
) -> impl IntoView {
    let expanded_connector_tools = create_rw_signal(HashSet::<(String, String)>::new());
    let SettingsViewState {
        locale,
        theme_mode,
        light_palette,
        dark_palette,
        ui_font_size,
        code_font_size,
        ui_font_family,
        code_font_family,
        selection_popup_enabled,
        send_with_modifier,
        custom_css,
        update_check_enabled,
        show_settings,
        settings_section,
        open_conn_key,
        channels_open,
        connectors,
        model_form,
        model_catalog_limits,
        conn_form,
        memory_selected,
        specialist_form,
        settings,
        bootstrap,
        settings_message,
        settings_busy,
        model_form_open,
        model_form_key,
        models,
        model_form_msg,
        show_acp_agents,
        acp_agents,
        active_acp_agent_id,
        acp_form,
        acp_form_msg,
        acp_infos,
        specialists,
        quick_actions,
        workflow_templates,
        workflow_studio,
        selected_workflow_template,
        specialist_form_open,
        memory_view,
        memory_editor,
        memory_msg,
        skills_list,
        skill_filter_tag,
        skills_search,
        skills_msg,
        plugins_list,
        plugins_msg,
        plugin_install_open,
        cred_status,
        cred_inputs,
        custom_credentials,
        cred_msg,
        approval_grants,
        conns_view,
        conn_form_open,
        conn_form_kind,
        conn_test_msg,
        custom_conn_tools,
        custom_conn_tools_loading,
        custom_conn_tool_errors,
        pet_status,
        ssh_hosts,
        execution_contexts,
        default_execution_context,
        runtime_interpreter_form,
        probing_context_id,
        delete_confirm,
    } = state;
    let acp_form_open = create_memo(move |_| acp_form.get().is_some());
    // Keep the edit/add branch stable while fields update. Reading the whole
    // form directly in the view gate remounts the inputs on every keystroke.
    let model_form_is_edit =
        create_memo(move |_| model_form.get().is_some_and(|form| form.id.is_some()));
    let memory_projects = create_rw_signal(Vec::<ProjectSummary>::new());
    let memory_project_menu_open = create_rw_signal(false);
    let global_memory_edit_id = create_rw_signal(None::<String>);
    let global_memory_editor = create_rw_signal(String::new());
    let global_memory_busy = create_rw_signal(false);
    let browser_filters = create_rw_signal(BrowserUrlFilters::default());
    let browser_filters_msg = create_rw_signal(None::<(bool, String)>);
    let browser_filters_busy = create_rw_signal(false);
    let browser_block_host = create_rw_signal(String::new());
    let browser_block_reason = create_rw_signal(String::new());
    let browser_prefer_host = create_rw_signal(String::new());
    let browser_prefer_reason = create_rw_signal(String::new());
    let browser_auto_launch = create_rw_signal(true);
    let browser_auto_close_tabs = create_rw_signal(false);
    create_effect(move |_| {
        if show_settings.get() && settings_section.get() == "browser" {
            spawn_local(async move {
                if let Ok(value) =
                    invoke_checked("get_browser_url_filters", JsValue::UNDEFINED).await
                {
                    if let Ok(filters) = serde_wasm_bindgen::from_value::<BrowserUrlFilters>(value)
                    {
                        browser_filters.set(filters);
                    }
                }
                if let Ok(value) =
                    invoke_checked("get_browser_auto_launch", JsValue::UNDEFINED).await
                {
                    if let Ok(enabled) = serde_wasm_bindgen::from_value::<bool>(value) {
                        browser_auto_launch.set(enabled);
                    }
                }
                if let Ok(value) =
                    invoke_checked("get_browser_auto_close_tabs", JsValue::UNDEFINED).await
                {
                    if let Ok(enabled) = serde_wasm_bindgen::from_value::<bool>(value) {
                        browser_auto_close_tabs.set(enabled);
                    }
                }
            });
        }
    });
    let save_browser_filters = Callback::new(move |next: BrowserUrlFilters| {
        browser_filters_busy.set(true);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "filters": next })).unwrap();
            match invoke_checked("set_browser_url_filters", arg).await {
                Ok(value) => {
                    if let Ok(filters) = serde_wasm_bindgen::from_value::<BrowserUrlFilters>(value)
                    {
                        browser_filters.set(filters);
                        browser_filters_msg.set(Some((
                            true,
                            t(locale.get_untracked(), "browser.filters.saved").into(),
                        )));
                    }
                }
                Err(err) => browser_filters_msg.set(Some((false, js_error_text(err)))),
            }
            browser_filters_busy.set(false);
        });
    });
    let save_browser_auto_launch = Callback::new(move |enabled: bool| {
        browser_auto_launch.set(enabled);
        browser_filters_busy.set(true);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "enabled": enabled })).unwrap();
            match invoke_checked("set_browser_auto_launch", arg).await {
                Ok(_) => browser_filters_msg.set(Some((
                    true,
                    t(locale.get_untracked(), "browser.auto_launch_saved").into(),
                ))),
                Err(err) => browser_filters_msg.set(Some((false, js_error_text(err)))),
            }
            browser_filters_busy.set(false);
        });
    });
    let save_browser_auto_close_tabs = Callback::new(move |enabled: bool| {
        browser_auto_close_tabs.set(enabled);
        browser_filters_busy.set(true);
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "enabled": enabled })).unwrap();
            match invoke_checked("set_browser_auto_close_tabs", arg).await {
                Ok(_) => browser_filters_msg.set(Some((
                    true,
                    t(locale.get_untracked(), "browser.auto_close_tabs_saved").into(),
                ))),
                Err(err) => browser_filters_msg.set(Some((false, js_error_text(err)))),
            }
            browser_filters_busy.set(false);
        });
    });
    create_effect(move |_| {
        if settings_section.get() != "memory" {
            memory_project_menu_open.set(false);
            global_memory_edit_id.set(None);
            global_memory_editor.set(String::new());
            return;
        }
        spawn_local(async move {
            let v = invoke("list_projects", JsValue::UNDEFINED).await;
            if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ProjectSummary>>(v) {
                memory_projects.set(list);
            }
        });
    });
    let reset_memory_browse = move || {
        memory_selected.set(None);
        memory_editor.set(String::new());
        memory_msg.set(None);
        memory_project_menu_open.set(false);
    };
    let load_memory_project = Callback::new(move |id: String| {
        let current = memory_view
            .get_untracked()
            .map(|view| view.project_id)
            .unwrap_or_default();
        if id == current {
            memory_project_menu_open.set(false);
            return;
        }
        reset_memory_browse();
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "projectId": id })).unwrap();
            match invoke_checked("get_memory_view", arg).await {
                Ok(v) => {
                    if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                        memory_view.set(Some(view));
                    } else {
                        memory_msg.set(Some((
                            false,
                            t(locale.get_untracked(), "memory.load_failed").into(),
                        )));
                    }
                }
                Err(err) => {
                    memory_msg.set(Some((false, js_error_text(err))));
                }
            }
        });
    });
    let joining = create_rw_signal(false);
    let join_code = create_rw_signal(String::new());
    let join_busy = create_rw_signal(false);
    let join_error = create_rw_signal(None::<String>);
    let plugin_checksum = create_rw_signal(String::new());
    let plugin_source = create_rw_signal(String::new());
    let plugin_url = create_rw_signal(String::new());
    let plugin_install_mode = create_rw_signal("local".to_string());
    let plugin_search = create_rw_signal(String::new());
    let oauth_authorizing = create_rw_signal(false);
    let conn_testing = create_rw_signal(false);
    let conn_test_seq = create_rw_signal(0u64);
    let custom_cred_name = create_rw_signal(String::new());
    let custom_cred_env = create_rw_signal(String::new());
    let custom_cred_value = create_rw_signal(String::new());
    let custom_cred_busy = create_rw_signal(false);
    // Model-list drag-reorder state (local — no need to hoist to the app shell).
    let drag_model = create_rw_signal(None::<String>);
    let drop_model = create_rw_signal(None::<String>);
    // Agent-created SSH trust edges (`configure_ssh_trust`), shown under the
    // hosts they involve so the user can see and revoke them. Reloaded each
    // time the Environments section opens.
    let ssh_trust_edges = create_rw_signal(Vec::<SshTrustEdge>::new());
    let trust_cleanup_error = create_rw_signal(None::<String>);
    let quick_action_form = create_rw_signal(None::<QuickAction>);
    let quick_action_busy = create_rw_signal(false);
    let quick_action_error = create_rw_signal(None::<String>);
    // Specialist skill whitelist picker: search query + filtered results, so a
    // large skill library never renders as an unbounded checkbox list.
    let specialist_skill_query = create_rw_signal(String::new());
    let specialist_filtered_skills = create_memo(move |_| {
        let query = specialist_skill_query.get();
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        skills_list
            .get()
            .into_iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
                    || s.scope.to_lowercase().contains(&query)
                    || s.tags.iter().any(|tag| tag.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>()
    });
    window_capture_escape(move || {
        if !show_settings.get_untracked() {
            return false;
        }
        if joining.get_untracked() {
            joining.set(false);
            return true;
        }
        if memory_project_menu_open.get_untracked() {
            memory_project_menu_open.set(false);
            return true;
        }
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            if let Ok(Some(details)) = document.query_selector("details.settings-add-menu[open]") {
                let _ = details.remove_attribute("open");
                return true;
            }
        }
        if quick_action_form.get_untracked().is_some() {
            quick_action_form.set(None);
            return true;
        }
        // Breadcrumb subpages (memory file, model/ACP/specialist/conn/channel
        // editors): one Escape returns to the section list, not the app.
        let has_subpage = memory_selected.get_untracked().is_some()
            || model_form.get_untracked().is_some()
            || acp_form.get_untracked().is_some()
            || specialist_form.get_untracked().is_some()
            || conn_form.get_untracked().is_some()
            || open_conn_key.get_untracked().is_some()
            || channels_open.get_untracked().is_some();
        if has_subpage {
            close_settings_subpage.call(());
            return true;
        }
        // Workflow Studio is a full-page settings surface; its own Escape
        // stack (connect → portfolio → back) is registered later and wins.
        false
    });
    create_effect(move |_| {
        if show_settings.get() && settings_section.get() == "environments" {
            spawn_local(async move {
                if let Ok(value) = invoke_checked("list_ssh_trust_edges", JsValue::UNDEFINED).await
                {
                    if let Ok(edges) = serde_wasm_bindgen::from_value::<Vec<SshTrustEdge>>(value) {
                        ssh_trust_edges.set(edges);
                    }
                }
            });
        }
    });
    // Storage / Usage panes fetch on every open; stale data stays visible
    // while the refresh (a blocking directory scan for storage) runs.
    let storage_usage = create_rw_signal(None::<StorageUsage>);
    let storage_project = create_rw_signal(String::new());
    let token_usage = create_rw_signal(None::<TokenUsageOverview>);
    let selected_usage_workspace = create_rw_signal(None::<ProjectTokenUsage>);
    let session_token_usage = create_rw_signal(None::<SessionTokenUsagePage>);
    let usage_session_page = create_rw_signal(0usize);
    let usage_activity_mode = create_rw_signal(UsageActivityMode::Daily);
    create_effect(move |_| {
        if show_settings.get() && settings_section.get() == "storage" {
            spawn_local(async move {
                if let Ok(value) = invoke_checked("get_storage_usage", JsValue::UNDEFINED).await {
                    if let Ok(usage) = serde_wasm_bindgen::from_value::<StorageUsage>(value) {
                        let selected = storage_project.get_untracked();
                        if !selected.is_empty()
                            && !usage.projects.iter().any(|project| project.id == selected)
                        {
                            storage_project.set(String::new());
                        }
                        storage_usage.set(Some(usage));
                    }
                }
            });
        }
    });
    create_effect(move |_| {
        if show_settings.get() && settings_section.get() == "usage" {
            spawn_local(async move {
                if let Ok(value) = invoke_checked("get_token_usage", JsValue::UNDEFINED).await {
                    if let Ok(overview) =
                        serde_wasm_bindgen::from_value::<TokenUsageOverview>(value)
                    {
                        token_usage.set(Some(overview));
                    }
                }
            });
        }
    });
    create_effect(move |_| {
        if !show_settings.get() || settings_section.get() != "usage" {
            return;
        }
        let Some(workspace) = selected_usage_workspace.get() else {
            return;
        };
        let page = usage_session_page.get();
        let project_id = workspace.project_id;
        session_token_usage.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "projectId": project_id,
                "offset": page.saturating_mul(USAGE_SESSION_PAGE_SIZE),
                "limit": USAGE_SESSION_PAGE_SIZE,
            }))
            .unwrap();
            let Ok(value) = invoke_checked("get_session_token_usage", args).await else {
                return;
            };
            let Ok(result) = serde_wasm_bindgen::from_value::<SessionTokenUsagePage>(value) else {
                return;
            };
            if selected_usage_workspace
                .get_untracked()
                .as_ref()
                .is_none_or(|workspace| workspace.project_id != project_id)
                || usage_session_page.get_untracked() != page
            {
                return;
            }
            let page_count = (result.total.max(0) as usize).div_ceil(USAGE_SESSION_PAGE_SIZE);
            if page > 0 && page >= page_count {
                usage_session_page.set(page_count.saturating_sub(1));
            } else {
                session_token_usage.set(Some(result));
            }
        });
    });
    create_effect(move |_| {
        if joining.get() {
            focus_element_soon("sync-device-code");
        }
    });
    let choose_sync_folder = move |_| {
        spawn_local(async move {
            let value = invoke("pick_directory", JsValue::UNDEFINED).await;
            if let Ok(path) = serde_wasm_bindgen::from_value::<String>(value) {
                settings.update(|current| current.sync_folder = path);
            }
        });
    };
    let choose_pet_directory = move |_| {
        spawn_local(async move {
            let value = invoke("pick_directory", JsValue::UNDEFINED).await;
            if let Ok(path) = serde_wasm_bindgen::from_value::<String>(value) {
                settings.update(|current| current.pet_directory = path);
            }
        });
    };
    let open_sync_guide = move |_| {
        let page = if locale.get_untracked() == Locale::Zh {
            "project-sync.zh-CN.md"
        } else {
            "project-sync.md"
        };
        crate::bindings::open_external_url(format!(
            "https://github.com/xuzhougeng/wisp-science/blob/main/docs/{page}"
        ));
    };
    let join_project = move |_| {
        let code = join_code.get();
        if code.trim().is_empty() || join_busy.get_untracked() {
            return;
        }
        join_busy.set(true);
        join_error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "code": code })).unwrap();
            match invoke_checked("join_synced_project", args).await {
                Ok(value) => {
                    if let Ok(Some(project)) =
                        serde_wasm_bindgen::from_value::<Option<ProjectSummary>>(value)
                    {
                        joining.set(false);
                        join_code.set(String::new());
                        show_settings.set(false);
                        open_project.call(project.id);
                    }
                }
                Err(error) => {
                    let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                    join_error.set(Some(message));
                }
            }
            join_busy.set(false);
        });
    };
    let persist_quick_action = Callback::new(move |action: QuickAction| {
        if quick_action_busy.get_untracked() {
            return;
        }
        quick_action_busy.set(true);
        quick_action_error.set(None);
        spawn_local(async move {
            let args = serde_json::json!({ "action": action });
            match invoke_checked("save_quick_action", to_value(&args).unwrap()).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<QuickAction>>(value) {
                    Ok(items) => {
                        quick_actions.set(items);
                        quick_action_form.set(None);
                    }
                    Err(error) => quick_action_error.set(Some(error.to_string())),
                },
                Err(error) => quick_action_error.set(Some(js_error_text(error))),
            }
            quick_action_busy.set(false);
        });
    });
    let save_quick_action_form = Callback::new(move |_: web_sys::MouseEvent| {
        let Some(action) = quick_action_form.get_untracked() else {
            return;
        };
        persist_quick_action.call(action);
    });
    let remove_quick_action = Callback::new(move |action_id: String| {
        if quick_action_busy.get_untracked() {
            return;
        }
        quick_action_busy.set(true);
        quick_action_error.set(None);
        spawn_local(async move {
            let args = serde_json::json!({ "actionId": action_id });
            match invoke_checked("remove_quick_action", to_value(&args).unwrap()).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<QuickAction>>(value) {
                    Ok(items) => quick_actions.set(items),
                    Err(error) => quick_action_error.set(Some(error.to_string())),
                },
                Err(error) => quick_action_error.set(Some(js_error_text(error))),
            }
            quick_action_busy.set(false);
        });
    });

    move || {
        show_settings.get().then(|| view! {
        <div class="settings-page"
            class:workflow-studio-mode=move || settings_section.get() == "workflows">
            <div class="settings-nav">
                <button type="button" class="settings-app-back settings-head-close"
                    on:click=move |_| show_settings.set(false)>
                    {compose_icon("chevron-left")}
                    <span>{move || t(locale.get(), "settings.back_to_app")}</span>
                </button>
                <div class="settings-nav-title">{move || t(locale.get(), "settings.title")}</div>
                <div class="settings-nav-group">
                    <span class="settings-nav-label">{move || t(locale.get(), "settings.nav.workspace")}</span>
                    <button class:active=move || settings_section.get()=="general"
                        on:click=move |_| go_settings_section.call("general".into())>
                        {move || t(locale.get(), "settings.nav.general")}</button>
                    <button class:active=move || settings_section.get()=="session"
                        data-testid="settings-nav-session"
                        on:click=move |_| go_settings_section.call("session".into())>
                        {move || t(locale.get(), "settings.nav.session")}</button>
                    <button class:active=move || settings_section.get()=="appearance"
                        on:click=move |_| go_settings_section.call("appearance".into())>
                        {move || t(locale.get(), "settings.nav.appearance")}</button>
                    <button class:active=move || settings_section.get()=="pet"
                        on:click=move |_| go_settings_section.call("pet".into())>
                        {move || t(locale.get(), "settings.nav.pet")}</button>
                    <button class:active=move || settings_section.get()=="credentials"
                        on:click=move |_| go_settings_section.call("credentials".into())>
                        {move || t(locale.get(), "settings.nav.credentials")}</button>
                    <button class:active=move || settings_section.get()=="permissions"
                        on:click=move |_| go_settings_section.call("permissions".into())>
                        {move || t(locale.get(), "settings.nav.permissions")}</button>
                    <button class:active=move || settings_section.get()=="environments"
                        on:click=move |_| go_settings_section.call("environments".into())>
                        {move || t(locale.get(), "settings.nav.environments")}</button>
                    <button class:active=move || settings_section.get()=="storage"
                        on:click=move |_| go_settings_section.call("storage".into())>
                        {move || t(locale.get(), "settings.nav.storage")}</button>
                    <button class:active=move || settings_section.get()=="usage"
                        on:click=move |_| go_settings_section.call("usage".into())>
                        {move || t(locale.get(), "settings.nav.usage")}</button>
                </div>
                <div class="settings-nav-group">
                    <span class="settings-nav-label">{move || t(locale.get(), "settings.nav.capabilities")}</span>
                    <button class:active=move || settings_section.get()=="models"
                        on:click=move |_| go_settings_section.call("models".into())>
                        {move || t(locale.get(), "settings.nav.models")}</button>
                    <button class:active=move || settings_section.get()=="quick-actions"
                        data-testid="settings-nav-quick-actions"
                        on:click=move |_| go_settings_section.call("quick-actions".into())>
                        {move || t(locale.get(), "settings.nav.quick_actions")}</button>
                    <button class:active=move || settings_section.get()=="workflows"
                        data-testid="settings-nav-workflows"
                        on:click=move |_| go_settings_section.call("workflows".into())>
                        {move || t(locale.get(), "settings.nav.workflows")}</button>
                    <button class:active=move || settings_section.get()=="specialists"
                        on:click=move |_| go_settings_section.call("specialists".into())>
                        {move || t(locale.get(), "settings.nav.specialists")}</button>
                    <button class:active=move || settings_section.get()=="memory"
                        on:click=move |_| go_settings_section.call("memory".into())>
                        {move || t(locale.get(), "settings.nav.memory")}</button>
                    <button class:active=move || settings_section.get()=="skills"
                        on:click=move |_| go_settings_section.call("skills".into())>
                        {move || t(locale.get(), "settings.nav.skills")}</button>
                    <button class:active=move || settings_section.get()=="plugins"
                        on:click=move |_| go_settings_section.call("plugins".into())>
                        {move || t(locale.get(), "settings.nav.plugins")}</button>
                    <button class:active=move || settings_section.get()=="browser"
                        data-testid="settings-nav-browser"
                        on:click=move |_| go_settings_section.call("browser".into())>
                        {move || t(locale.get(), "settings.nav.browser")}</button>
                    <button class:active=move || settings_section.get()=="network"
                        data-testid="settings-nav-network"
                        on:click=move |_| go_settings_section.call("network".into())>
                        {move || t(locale.get(), "settings.nav.network")}</button>
                    <button class:active=move || settings_section.get()=="connections"
                        on:click=move |_| go_settings_section.call("connections".into())>
                        {move || t(locale.get(), "settings.nav.connections")}</button>
                    <button class:active=move || settings_section.get()=="channels"
                        on:click=move |_| go_settings_section.call("channels".into())>
                        {move || t(locale.get(), "settings.nav.channels")}</button>
                </div>
            </div>
            <div class="settings-content">
                {move || {
                    let sec = settings_section.get();
                    let loc = locale.get();
                    let parent = settings_section_label(loc, &sec);
                    let open_conn_name = open_conn_key.get().and_then(|k| {
                        connectors.get().and_then(|v| {
                            v.connectors.into_iter().find(|c| c.key == k).map(|c| c.name)
                        })
                    });
                    let sub = settings_subpage_label(
                        loc,
                        &sec,
                        model_form.get().as_ref(),
                        conn_form.get().as_ref(),
                        open_conn_name.as_deref(),
                        memory_selected.get().as_deref(),
                        specialist_form.get().as_ref(),
                        acp_form.get().as_ref(),
                        channels_open.get().as_deref(),
                    );
                    view! {
                        <div class="settings-head">
                            <div class="settings-head-main">
                                {sub.is_some().then(|| view! {
                                    <button type="button" class="settings-head-back"
                                        title=move || t(locale.get(), "settings.back")
                                        on:click=move |_| close_settings_subpage.call(())>{compose_icon("chevron-left")}</button>
                                })}
                                {move || if let Some(child) = sub.clone() {
                                    view! {
                                        <div class="settings-breadcrumb">
                                            <button type="button" class="settings-crumb-link"
                                                on:click=move |_| close_settings_subpage.call(())>{parent.clone()}</button>
                                            <span class="settings-crumb-sep">"›"</span>
                                            <span class="settings-crumb-current">{child}</span>
                                        </div>
                                    }.into_view()
                                } else {
                                    view! { <h2>{parent.clone()}</h2> }.into_view()
                                }}
                            </div>
                        </div>
                    }
                }}
                {move || (settings_section.get() == "network").then(|| view! {
                    <crate::network_settings::NetworkSettingsView settings=settings />
                })}
                {move || (settings_section.get() == "general").then(|| view! {
                    <div class="settings-pane">
                        <div class="settings-form-grid">
                        <label class="span-2">{move || t(locale.get(), "settings.language")}
                            <select data-testid="settings-language"
                                on:change=move|ev| {
                                    let code = dom_value(&ev);
                                    let loc = Locale::from_code(&code);
                                    locale.set(loc);
                                    set_document_lang(loc);
                                    settings.update(|s| s.locale = code);
                                }
                                // Bind `selected` on the options instead of `value` on the
                                // select: the select's `value` property is applied before the
                                // option children exist, so it falls back to the first option
                                // and shows "English" while the locale is Chinese (#431).
                                >
                                <option value="en" prop:selected=move || locale.get() == Locale::En>{move || t(locale.get(), "settings.language.en")}</option>
                                <option value="zh" prop:selected=move || locale.get() == Locale::Zh>{move || t(locale.get(), "settings.language.zh")}</option>
                            </select>
                        </label>
                        <label class="span-2">{move || t(locale.get(), "settings.workspace_dir")}
                            <input class="settings-path-input" on:input=move|ev| settings.update(|s| {
                                    s.workspace_dir = event_target_input(&ev).value();
                                })
                                prop:value={move || settings.get().workspace_dir}
                                placeholder=move || bootstrap.get().map(|b| b.workspace).unwrap_or_default() />
                        </label>
                        <div class="span-2 appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "settings.resume_last_session")}</strong>
                                <span>{move || t(locale.get(), "settings.resume_last_session_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="resume-last-session-enabled"
                                    prop:checked=move || settings.get().resume_last_session
                                    on:change=move |ev| settings.update(|current| current.resume_last_session = event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        <label class="span-2">{move || t(locale.get(), "settings.send_shortcut")}
                            <select data-testid="send-shortcut"
                                prop:value=move || if send_with_modifier.get() { "modifier_enter" } else { "enter" }
                                on:change=move |ev| send_with_modifier.set(dom_value(&ev) == "modifier_enter")>
                                <option value="enter">{move || t(locale.get(), "settings.send_shortcut.enter")}</option>
                                <option value="modifier_enter">{move || tf(
                                    locale.get(),
                                    "settings.send_shortcut.modifier_enter",
                                    &[("modifier", if is_mac() { "Cmd" } else { "Ctrl" })],
                                )}</option>
                            </select>
                        </label>
                        <div class="span-2 appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "settings.notifications")}</strong>
                                <span>{move || t(locale.get(), "settings.notifications_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="notifications-enabled"
                                    prop:checked=move || settings.get().notifications_enabled
                                    on:change=move |ev| settings.update(|current| current.notifications_enabled = event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        <div class="span-2 appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "settings.selection_popup")}</strong>
                                <span>{move || t(locale.get(), "settings.selection_popup_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="selection-popup-enabled"
                                    prop:checked=move || selection_popup_enabled.get()
                                    on:change=move |ev| selection_popup_enabled.set(event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        <div class="span-2 appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "settings.update_check")}</strong>
                                <span>{move || t(locale.get(), "settings.update_check_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="update-check-enabled"
                                    prop:checked=move || update_check_enabled.get()
                                    on:change=move |ev| {
                                        let on = event_target_checked(&ev);
                                        update_check_enabled.set(on);
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({ "enabled": on })).unwrap();
                                            let _ = invoke("set_update_check_enabled", arg).await;
                                        });
                                    } />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        </div>
                        {move || settings_message.get().map(|(ok, text)| view! {
                            <div class="settings-status"
                                class:ok=move || ok
                                class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                                <span class="settings-version">{concat!("wisp-science v", env!("CARGO_PKG_VERSION"))}</span>
                                <button type="button" disabled=move || settings_busy.get() on:click=move |ev| check_updates.call(ev)>{move || t(locale.get(), "settings.check_updates")}</button>
                            <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                                <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "session").then(|| view! {
                    <div class="settings-pane" data-testid="session-settings-pane">
                        <div class="settings-form-grid">
                        <label class="span-2">{move || t(locale.get(), "settings.max_iter")}
                            <input data-testid="max-iter" type="number" min="0" step="1"
                                on:input=move |ev| settings.update(|s| {
                                    if let Ok(value) = event_target_input(&ev).value().parse() {
                                        s.max_iter = value;
                                    }
                                })
                                prop:value=move || settings.get().max_iter.to_string() />
                            <span class="settings-field-hint">{move || t(locale.get(), "settings.max_iter_hint")}</span>
                        </label>
                        <div class="span-2 appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "settings.auto_compact")}</strong>
                                <span>{move || t(locale.get(), "settings.auto_compact_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="auto-compact-enabled"
                                    prop:checked=move || settings.get().auto_compact
                                    on:change=move |ev| settings.update(|current| current.auto_compact = event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        <div class="span-2 appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "settings.auto_continue")}</strong>
                                <span>{move || t(locale.get(), "settings.auto_continue_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="auto-continue-enabled"
                                    prop:checked=move || settings.get().auto_continue
                                    on:change=move |ev| settings.update(|current| current.auto_continue = event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        <label class="span-2">{move || t(locale.get(), "settings.auto_continue_limit")}
                            <input data-testid="auto-continue-limit" type="number" min="1" step="1"
                                on:input=move |ev| settings.update(|current| {
                                    if let Ok(value) = event_target_input(&ev).value().parse() {
                                        current.auto_continue_limit = value;
                                    }
                                })
                                prop:value=move || settings.get().auto_continue_limit.to_string() />
                            <span class="settings-field-hint">{move || t(locale.get(), "settings.auto_continue_limit_hint")}</span>
                        </label>
                        <div class="span-2 appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "settings.follow_up_questions")}</strong>
                                <span>{move || t(locale.get(), "settings.follow_up_questions_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="follow-up-questions-enabled"
                                    prop:checked=move || settings.get().follow_up_questions
                                    on:change=move |ev| settings.update(|current| current.follow_up_questions = event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        </div>
                        {move || settings_message.get().map(|(ok, text)| view! {
                            <div class="settings-status"
                                class:ok=move || ok
                                class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                            <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "environments").then(|| view! {
                    <div class="settings-pane settings-pane-list environment-settings-pane">
                        <p class="settings-note">{move || t(locale.get(), "environments.hint")}</p>
                        <div class="environment-default-field">
                            <label>
                                {move || t(locale.get(), "environments.default_analysis")}
                                <DefaultAnalysisSelect
                                    locale=locale
                                    execution_contexts=execution_contexts
                                    default_execution_context=default_execution_context
                                    on_change=set_default_compute_resource
                                    test_id="default-analysis-environment".to_string()
                                />
                            </label>
                            <p class="settings-field-hint">{move || t(locale.get(), "environments.default_analysis_hint")}</p>
                        </div>
                        {move || trust_cleanup_error.get().map(|error| view! {
                            <div class="settings-status fail" role="alert">
                                {format!("{}: {error}", t(locale.get(), "environments.trust_cleanup_failed"))}
                            </div>
                        })}
                        <div class="settings-toolbar environment-settings-actions">
                            <button type="button" class="primary" on:click=move |_| open_add_host.call(())>
                                {compose_icon("plus")}
                                <span>{move || t(locale.get(), "hosts.add")}</span>
                            </button>
                            <span></span>
                            <button type="button" on:click=move |_| import_ssh_hosts.call(())>
                                {move || t(locale.get(), "hosts.import")}
                            </button>
                            {is_windows().then(|| view! {
                                <button type="button" on:click=move |_| import_wsl_contexts.call(())>
                                    {move || t(locale.get(), "contexts.import_wsl")}
                                </button>
                            })}
                        </div>
                        <div class="settings-list environment-settings-list">
                            {move || {
                                let contexts = execution_contexts.get();
                                let hosts = ssh_hosts.get();
                                let trust_edges = ssh_trust_edges.get();
                                if contexts.is_empty() {
                                    return view! { <div class="settings-list-empty">{t(locale.get(), "environments.empty")}</div> }.into_view();
                                }
                                contexts.into_iter().map(|context| {
                                    // An edge involves two hosts; list it under both so it
                                    // stays visible even after one endpoint is deleted.
                                    let context_trust_edges = trust_edges.iter()
                                        .filter(|edge| edge.source_context_id == context.id
                                            || edge.destination_context_id == context.id)
                                        .cloned()
                                        .collect::<Vec<_>>();
                                    let context_id = context.id.clone();
                                    let title = if context.kind == "local" {
                                        t(locale.get(), "compute.local").to_string()
                                    } else if context.label.trim().is_empty() {
                                        context.id.clone()
                                    } else {
                                        context.label.clone()
                                    };
                                    let connection = context.id.strip_prefix("ssh:")
                                        .and_then(|alias| hosts.iter().find(|host| host.alias == alias))
                                        .map(|host| match (&host.user, host.port) {
                                            (Some(user), Some(port)) => format!("{user}@{}:{port}", host.alias),
                                            (Some(user), None) => format!("{user}@{}", host.alias),
                                            (None, Some(port)) => format!("{}:{port}", host.alias),
                                            (None, None) => host.alias.clone(),
                                        })
                                        .unwrap_or_else(|| context.id.clone());
                                    let capability_summary = format!(" · {}", context_capability_summary(&context));
                                    let config_context = context.clone();
                                    let probe_id = context_id.clone();
                                    let probe_busy_id = context_id.clone();
                                    let probe_label_id = context_id.clone();
                                    let probe_status_id = context_id.clone();
                                    let is_ssh = context.kind == "ssh";
                                    let ssh_alias = context.id.strip_prefix("ssh:").map(str::to_string);
                                    let edit_alias = ssh_alias.clone();
                                    let remove_alias = ssh_alias;
                                    view! {
                                        <div class="settings-list-row environment-settings-row" data-context-id=context_id>
                                            <span class="environment-server-icon">
                                                {compose_icon("server")}
                                            </span>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{title}</span>
                                                <span class="settings-list-sub">
                                                    {connection}
                                                    {capability_summary}
                                                </span>
                                                {move || (probing_context_id.get().as_deref() == Some(probe_status_id.as_str())).then(|| view! {
                                                    <span class="environment-probe-feedback" role="status">
                                                        <span class="environment-probe-spinner" aria-hidden="true"></span>
                                                        {if is_ssh {
                                                            t(locale.get(), "contexts.probing_ssh")
                                                        } else {
                                                            t(locale.get(), "contexts.probing_local")
                                                        }}
                                                    </span>
                                                })}
                                            </div>
                                            <div class="settings-list-actions">
                                                {edit_alias.map(|alias| view! {
                                                    <button type="button" class="environment-edit"
                                                        title=move || t(locale.get(), "environments.edit")
                                                        aria-label=move || t(locale.get(), "environments.edit")
                                                        on:click=move |_| edit_ssh_host.call(alias.clone())>
                                                        {t(locale.get(), "environments.edit")}
                                                    </button>
                                                })}
                                                <button type="button" class="environment-runtime-config"
                                                    title=move || t(locale.get(), "contexts.configure_interpreters")
                                                    aria-label=move || t(locale.get(), "contexts.configure_interpreters")
                                                    on:click=move |_| runtime_interpreter_form.set(Some(
                                                        RuntimeInterpreterForm::from_context(&config_context)
                                                    ))>
                                                    {t(locale.get(), "runtime.configure")}
                                                </button>
                                                <button type="button" class="environment-probe"
                                                    disabled=move || probing_context_id.get().is_some()
                                                    aria-busy=move || if probing_context_id.get().as_deref() == Some(probe_busy_id.as_str()) { "true" } else { "false" }
                                                    on:click=move |_| probe_compute_resource.call(probe_id.clone())>
                                                    {move || if probing_context_id.get().as_deref() == Some(probe_label_id.as_str()) {
                                                        t(locale.get(), "contexts.probing")
                                                    } else {
                                                        t(locale.get(), "contexts.probe")
                                                    }}
                                                </button>
                                                <span class="environment-remove-slot">
                                                    {remove_alias.map(|alias| view! {
                                                        <button type="button" class="settings-list-remove"
                                                            title=move || t(locale.get(), "environments.remove")
                                                            aria-label=move || t(locale.get(), "environments.remove")
                                                            on:click=move |_| {
                                                                let alias = alias.clone();
                                                                spawn_local(async move {
                                                                    let args = to_value(&serde_json::json!({
                                                                        "contextId": format!("ssh:{alias}"),
                                                                    }))
                                                                    .unwrap();
                                                                    let report = invoke_checked("context_disposal_report", args)
                                                                        .await
                                                                        .ok()
                                                                        .and_then(|value| serde_wasm_bindgen::from_value::<ContextDisposalReport>(value).ok());
                                                                    match report {
                                                                        Some(report)
                                                                            if report.external_references > 0
                                                                                || report.staged_files > 0
                                                                                || report.active_runs > 0 =>
                                                                        {
                                                                            let detail = tf(
                                                                                locale.get_untracked(),
                                                                                "hosts.disposal_detail",
                                                                                &[
                                                                                    ("refs", &report.external_references.to_string()),
                                                                                    ("files", &report.staged_files.to_string()),
                                                                                    ("runs", &report.active_runs.to_string()),
                                                                                ],
                                                                            );
                                                                            delete_confirm.set(Some(DeleteConfirm::Host {
                                                                                alias: alias.clone(),
                                                                                label: alias.clone(),
                                                                                detail,
                                                                            }));
                                                                        }
                                                                        _ => remove_ssh_host.call(alias.clone()),
                                                                    }
                                                                });
                                                            }>
                                                            {compose_icon("close")}
                                                        </button>
                                                    })}
                                                </span>
                                            </div>
                                        </div>
                                        {(!context_trust_edges.is_empty()).then(|| view! {
                                            <div class="environment-trust-edges">
                                                {context_trust_edges.into_iter().map(|edge| {
                                                    let title = format!("{} → {}",
                                                        trust_alias(&edge.source_context_id),
                                                        trust_alias(&edge.destination_context_id));
                                                    let target = match edge.destination_port {
                                                        Some(port) => format!("{}:{port}", edge.destination_target),
                                                        None => edge.destination_target.clone(),
                                                    };
                                                    let date = js_sys::Date::new(&JsValue::from_f64(edge.verified_at as f64 * 1000.0))
                                                        .to_locale_date_string(
                                                            if locale.get() == Locale::Zh { "zh-CN" } else { "en-US" },
                                                            &JsValue::UNDEFINED,
                                                        )
                                                        .as_string()
                                                        .unwrap_or_default();
                                                    let kind_key = if edge.managed {
                                                        "environments.trust_managed"
                                                    } else {
                                                        "environments.trust_verified"
                                                    };
                                                    let sub = format!("{} · {target} · {date}", t(locale.get(), kind_key));
                                                    let source = edge.source_context_id.clone();
                                                    let destination = edge.destination_context_id.clone();
                                                    view! {
                                                        <div class="environment-trust-edge">
                                                            <span class="environment-trust-edge-title">{title}</span>
                                                            <span class="environment-trust-edge-sub">{sub}</span>
                                                            <button type="button" class="environment-trust-revoke"
                                                                title=move || t(locale.get(), "environments.trust_revoke")
                                                                aria-label=move || t(locale.get(), "environments.trust_revoke")
                                                                on:click=move |_| {
                                                                    let source = source.clone();
                                                                    let destination = destination.clone();
                                                                    spawn_local(async move {
                                                                        let args = to_value(&serde_json::json!({
                                                                            "sourceContextId": source,
                                                                            "destinationContextId": destination,
                                                                        })).unwrap();
                                                                        match invoke_checked("revoke_ssh_trust_edge", args).await {
                                                                            Ok(value) => {
                                                                                if let Ok(response) = serde_wasm_bindgen::from_value::<RevokeTrustResponse>(value) {
                                                                                    ssh_trust_edges.set(response.edges);
                                                                                    trust_cleanup_error.set(response.cleanup_error);
                                                                                }
                                                                            }
                                                                            Err(error) => trust_cleanup_error.set(Some(
                                                                                localize_backend(locale.get_untracked(), &js_error_text(error)),
                                                                            )),
                                                                        }
                                                                    });
                                                                }>
                                                                {move || t(locale.get(), "environments.trust_revoke")}
                                                            </button>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        })}
                                    }.into_view()
                                }).collect_view()
                            }}
                        </div>
                    </div>
                }.into_view())}
                {move || joining.get().then(|| view! {
                    <div class="overlay project-sync-join-overlay">
                        <div class="modal project-sync-join-modal" role="dialog"
                            aria-modal="true"
                            aria-label=move || t(locale.get(), "projects.sync.join_title")
                            aria-describedby="sync-join-hint">
                            <div class="ps-head">
                                <h2>{move || t(locale.get(), "projects.sync.join_title")}</h2>
                                <button type="button" class="ps-close"
                                    title=move || t(locale.get(), "projects.cancel")
                                    aria-label=move || t(locale.get(), "projects.cancel")
                                    on:click=move |_| joining.set(false)>{compose_icon("close")}</button>
                            </div>
                            <p id="sync-join-hint" class="project-sync-join-hint">
                                {move || t(locale.get(), "projects.sync.join_hint")}
                            </p>
                            <div class="project-sync-code-head">
                                <label for="sync-device-code">
                                    {move || t(locale.get(), "projects.sync.code_label")}
                                </label>
                                <button type="button" class="project-sync-guide" on:click=open_sync_guide>
                                    {compose_icon("doc")}
                                    <span>{move || t(locale.get(), "projects.sync.guide")}</span>
                                </button>
                            </div>
                            <textarea id="sync-device-code" data-testid="sync-device-code" rows="5"
                                autofocus=true autocomplete="off" spellcheck="false"
                                placeholder=move || t(locale.get(), "projects.sync.code_placeholder")
                                prop:value=move || join_code.get()
                                on:input=move |ev| join_code.set(event_target_value(&ev))></textarea>
                            {move || join_error.get().map(|message| view! {
                                <div class="settings-status fail" role="alert">{message}</div>
                            })}
                            <div class="row project-sync-join-actions">
                                <button type="button" disabled=move || join_busy.get()
                                    on:click=move |_| joining.set(false)>
                                    {move || t(locale.get(), "projects.cancel")}</button>
                                <button type="button" class="primary"
                                    disabled=move || join_busy.get() || join_code.get().trim().is_empty()
                                    on:click=join_project>{move || t(locale.get(), "projects.sync.join_action")}</button>
                            </div>
                        </div>
                    </div>
                })}
                {move || (settings_section.get() == "storage").then(|| view! {
                    <div class="settings-pane">
                        <div class="settings-form-grid">
                            <div class="span-2 appearance-config-row">
                                <div>
                                    <strong>{move || t(locale.get(), "settings.storage.data_dir")}</strong>
                                    <span>{move || t(locale.get(), "settings.storage.data_dir_hint")}</span>
                                </div>
                            </div>
                            {move || match storage_usage.get() {
                                None => view! {
                                    <div class="span-2 settings-field-hint">
                                        {move || t(locale.get(), "settings.storage.loading")}
                                    </div>
                                }.into_view(),
                                Some(usage) => {
                                    let selected = storage_project.get();
                                    let selected_project = usage
                                        .projects
                                        .iter()
                                        .find(|project| project.id == selected)
                                        .cloned();
                                    let project_only = selected_project.is_some();
                                    let (path, entries, total_bytes) = match selected_project {
                                        Some(project) => (
                                            project.path,
                                            vec![StorageEntry {
                                                key: "workspace".into(),
                                                bytes: project.bytes,
                                            }],
                                            project.bytes,
                                        ),
                                        None => (
                                            usage.data_dir.clone(),
                                            usage.entries.clone(),
                                            usage.total_bytes,
                                        ),
                                    };
                                    let total = total_bytes.max(1);
                                    let projects = usage.projects.clone();
                                    let all_bytes = format_bytes(usage.total_bytes);
                                    let loc = locale.get();
                                    view! {
                                        <div class="span-2 storage-block">
                                            {(!projects.is_empty()).then(|| view! {
                                                <section class="storage-project-section">
                                                    <div class="storage-project-heading">
                                                        <strong>{t(loc, "settings.storage.projects")}</strong>
                                                        <span>{t(loc, "settings.storage.select_project")}</span>
                                                    </div>
                                                    <div class="storage-project-list" data-testid="storage-project-list">
                                                        <button type="button" class="storage-project-row"
                                                            class:active=move || storage_project.get().is_empty()
                                                            aria-pressed=move || storage_project.get().is_empty().to_string()
                                                            on:click=move |_| storage_project.set(String::new())>
                                                            <span class="storage-project-main">
                                                                <span class="storage-project-name">
                                                                    {t(loc, "settings.storage.all_projects")}
                                                                </span>
                                                            </span>
                                                            <span class="storage-project-bytes">{all_bytes}</span>
                                                        </button>
                                                        {projects.into_iter().map(|project| {
                                                            let id = project.id;
                                                            let active_id = id.clone();
                                                            let pressed_id = id.clone();
                                                            let select_id = id.clone();
                                                            let path_title = project.path.clone();
                                                            view! {
                                                                <button type="button" class="storage-project-row"
                                                                    class:active=move || storage_project.get() == active_id
                                                                    data-project-id=id
                                                                    aria-pressed=move || (storage_project.get() == pressed_id).to_string()
                                                                    on:click=move |_| storage_project.set(select_id.clone())>
                                                                    <span class="storage-project-main">
                                                                        <span class="storage-project-name">{project.name}</span>
                                                                        <code class="storage-project-path" title=path_title>
                                                                            {project.path}
                                                                        </code>
                                                                    </span>
                                                                    <span class="storage-project-bytes">
                                                                        {format_bytes(project.bytes)}
                                                                    </span>
                                                                </button>
                                                            }
                                                        }).collect_view()}
                                                    </div>
                                                </section>
                                            })}
                                            <code class="storage-path">{path}</code>
                                            {project_only.then(|| view! {
                                                <span class="settings-field-hint">
                                                    {t(loc, "settings.storage.project_hint")}
                                                </span>
                                            })}
                                            <div class="storage-bar">
                                                {entries.iter().filter(|entry| entry.bytes > 0).map(|entry| {
                                                    let pct = entry.bytes as f64 / total as f64 * 100.0;
                                                    view! {
                                                        <span class=format!("storage-seg storage-seg-{}", entry.key)
                                                            style:width=format!("{pct:.2}%")
                                                            title=format!("{} {}", t(loc, storage_entry_label_key(&entry.key)), format_bytes(entry.bytes))>
                                                        </span>
                                                    }
                                                }).collect_view()}
                                            </div>
                                            <div class="storage-legend">
                                                {entries.iter().map(|entry| view! {
                                                    <div class="storage-legend-row">
                                                        <span class=format!("storage-dot storage-seg-{}", entry.key) aria-hidden="true"></span>
                                                        <span class="storage-legend-label">{t(loc, storage_entry_label_key(&entry.key))}</span>
                                                        <span class="storage-legend-bytes">{format_bytes(entry.bytes)}</span>
                                                    </div>
                                                }).collect_view()}
                                                <div class="storage-legend-row storage-legend-total">
                                                    <span class="storage-dot" aria-hidden="true"></span>
                                                    <span class="storage-legend-label">{t(loc, "settings.storage.total")}</span>
                                                    <span class="storage-legend-bytes">{format_bytes(total_bytes)}</span>
                                                </div>
                                            </div>
                                        </div>
                                    }.into_view()
                                }
                            }}
                        </div>
                    </div>
                })}
                {move || (settings_section.get() == "usage").then(|| view! {
                    <div class="settings-pane settings-usage-pane" data-testid="usage-pane">
                        {move || match token_usage.get() {
                            None => view! {
                                <div class="settings-field-hint">
                                    {move || t(locale.get(), "settings.usage.loading")}
                                </div>
                            }.into_view(),
                            Some(overview) if overview.workspaces.is_empty() => view! {
                                <div class="settings-field-hint">
                                    {move || t(locale.get(), "settings.usage.empty")}
                                </div>
                            }.into_view(),
                            Some(overview) => {
                                let loc = locale.get();
                                let tokens = |n: i64| crate::fmt_tokens(n.max(0) as u64);
                                if let Some(workspace) = selected_usage_workspace.get() {
                                    let totals = (
                                        workspace.input,
                                        workspace.output,
                                        workspace.reasoning,
                                        workspace.cached,
                                    );
                                    let workspace_name = if workspace.name.trim().is_empty() {
                                        workspace.project_id.clone()
                                    } else {
                                        workspace.name.clone()
                                    };
                                    view! {
                                        <div class="usage-detail-head">
                                            <button type="button" class="btn-ghost" data-testid="usage-back"
                                                on:click=move |_| {
                                                    usage_session_page.set(0);
                                                    session_token_usage.set(None);
                                                    selected_usage_workspace.set(None);
                                                }>
                                                <span aria-hidden="true">"←"</span>
                                                {t(loc, "settings.usage.all_workspaces")}
                                            </button>
                                            <div class="usage-detail-title">
                                                <strong>{workspace_name}</strong>
                                                {(!workspace.workspace_dir.is_empty()).then(|| view! {
                                                    <span title=workspace.workspace_dir.clone()>{workspace.workspace_dir.clone()}</span>
                                                })}
                                            </div>
                                        </div>
                                        {usage_summary_view(loc, totals)}
                                        {move || match session_token_usage.get() {
                                            None => view! {
                                                <div class="settings-field-hint">
                                                    {t(locale.get(), "settings.usage.loading")}
                                                </div>
                                            }.into_view(),
                                            Some(page) if page.items.is_empty() => view! {
                                                <div class="settings-field-hint">
                                                    {t(locale.get(), "settings.usage.sessions_empty")}
                                                </div>
                                            }.into_view(),
                                            Some(page) => {
                                                let loc = locale.get();
                                                let current_page = usage_session_page.get();
                                                let page_count = (page.total.max(0) as usize)
                                                    .div_ceil(USAGE_SESSION_PAGE_SIZE)
                                                    .max(1);
                                                view! {
                                                    <div class="usage-table" data-testid="usage-session-table">
                                                        <div class="usage-row usage-row-head">
                                                            <span>{t(loc, "settings.usage.session")}</span>
                                                            <span class="usage-num">{t(loc, "settings.usage.input")}</span>
                                                            <span class="usage-num">{t(loc, "settings.usage.output")}</span>
                                                            <span class="usage-num">{t(loc, "settings.usage.reasoning")}</span>
                                                            <span class="usage-num">{t(loc, "settings.usage.cached")}</span>
                                                        </div>
                                                        {page.items.into_iter().map(|row| view! {
                                                            <div class="usage-row" data-testid="usage-session-row" data-session-id=row.id>
                                                                <span class="usage-session">
                                                                    <span class="usage-session-title">{row.title}</span>
                                                                    <span class="usage-session-when">{format_relative_time(row.updated_at, loc)}</span>
                                                                </span>
                                                                <span class="usage-num">{tokens(row.input)}</span>
                                                                <span class="usage-num">{tokens(row.output)}</span>
                                                                <span class="usage-num">{tokens(row.reasoning)}</span>
                                                                <span class="usage-num">{tokens(row.cached)}</span>
                                                            </div>
                                                        }).collect_view()}
                                                    </div>
                                                    {(page_count > 1).then(|| view! {
                                                        <div class="usage-pagination" data-testid="usage-pagination">
                                                            <button type="button" class="btn-ghost"
                                                                disabled=(current_page == 0).then_some("")
                                                                on:click=move |_| usage_session_page.update(|page| {
                                                                    *page = page.saturating_sub(1);
                                                                })>
                                                                {t(loc, "settings.usage.previous")}
                                                            </button>
                                                            <span>{tf(loc, "settings.usage.page", &[
                                                                ("page", &(current_page + 1).to_string()),
                                                                ("pages", &page_count.to_string()),
                                                            ])}</span>
                                                            <button type="button" class="btn-ghost"
                                                                disabled=(current_page + 1 >= page_count).then_some("")
                                                                on:click=move |_| usage_session_page.update(|page| {
                                                                    *page = (*page + 1).min(page_count - 1);
                                                                })>
                                                                {t(loc, "settings.usage.next")}
                                                            </button>
                                                        </div>
                                                    })}
                                                }.into_view()
                                            }
                                        }}
                                    }.into_view()
                                } else {
                                    let totals = overview.workspaces.iter().fold(
                                        (0i64, 0i64, 0i64, 0i64),
                                        |acc, row| (
                                            acc.0 + row.input,
                                            acc.1 + row.output,
                                            acc.2 + row.reasoning,
                                            acc.3 + row.cached,
                                        ),
                                    );
                                    let activity_cells = usage_activity_cells(
                                        &overview.days,
                                        usage_activity_mode.get(),
                                    );
                                    let activity_months = usage_activity_months(&overview.days);
                                    let model_slices = usage_model_slices(
                                        &overview.models,
                                        &models.get(),
                                        &t(loc, "settings.usage.unknown_model"),
                                        &t(loc, "settings.usage.other_models"),
                                    );
                                    let model_total = model_slices
                                        .iter()
                                        .map(|slice| slice.tokens)
                                        .sum::<i64>();
                                    let model_gradient = usage_model_gradient(&model_slices);
                                    let tool_rows = usage_tool_rank_rows(
                                        &overview.tools,
                                        &t(loc, "settings.usage.other_tools"),
                                    );
                                    let tool_total =
                                        tool_rows.iter().map(|row| row.calls).sum::<i64>();
                                    let tool_max = tool_rows
                                        .iter()
                                        .map(|row| row.calls)
                                        .max()
                                        .unwrap_or(0)
                                        .max(1);
                                    view! {
                                        <p class="settings-field-hint">{t(loc, "settings.usage.hint")}</p>
                                        {usage_summary_view(loc, totals)}
                                        <section class="usage-card usage-activity-card" data-testid="usage-activity">
                                            <div class="usage-card-head">
                                                <div>
                                                    <h3>{t(loc, "settings.usage.activity")}</h3>
                                                    <p>{t(loc, "settings.usage.activity_hint")}</p>
                                                </div>
                                                <div class="usage-mode-tabs" role="group"
                                                    aria-label=t(loc, "settings.usage.activity")>
                                                    {[
                                                        (UsageActivityMode::Daily, "settings.usage.daily"),
                                                        (UsageActivityMode::Weekly, "settings.usage.weekly"),
                                                        (UsageActivityMode::Cumulative, "settings.usage.cumulative"),
                                                    ].into_iter().map(|(mode, key)| view! {
                                                        <button type="button"
                                                            class:active=move || usage_activity_mode.get() == mode
                                                            aria-pressed=move || if usage_activity_mode.get() == mode {
                                                                "true"
                                                            } else {
                                                                "false"
                                                            }
                                                            on:click=move |_| usage_activity_mode.set(mode)>
                                                            {t(loc, key)}
                                                        </button>
                                                    }).collect_view()}
                                                </div>
                                            </div>
                                            <div class="usage-activity-scroll">
                                                <div class="usage-activity-months" aria-hidden="true">
                                                    {activity_months.into_iter().map(|(week, month)| view! {
                                                        <span style=format!("grid-column:{}", week + 1)>
                                                            {usage_month_label(month, loc)}
                                                        </span>
                                                    }).collect_view()}
                                                </div>
                                                <div class="usage-activity-grid" aria-label=t(loc, "settings.usage.activity")>
                                                    {activity_cells.into_iter().map(|cell| {
                                                        let title = if cell.future {
                                                            String::new()
                                                        } else {
                                                            tf(loc, "settings.usage.activity_tooltip", &[
                                                                ("period", &cell.period),
                                                                ("tokens", &tokens(cell.tokens)),
                                                            ])
                                                        };
                                                        view! {
                                                            <span class=format!("usage-activity-cell level-{}", cell.level)
                                                                class:future=cell.future title=title aria-hidden="true"></span>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            </div>
                                        </section>
                                        <div class="usage-overview-grid">
                                            <div class="usage-left-stack">
                                            <section class="usage-card usage-model-card" data-testid="usage-model-share">
                                                <div class="usage-card-head">
                                                    <div>
                                                        <h3>{t(loc, "settings.usage.model_share")}</h3>
                                                        <p>{t(loc, "settings.usage.model_share_hint")}</p>
                                                    </div>
                                                </div>
                                                {if model_slices.is_empty() {
                                                    view! {
                                                        <p class="settings-field-hint">{t(loc, "settings.usage.models_empty")}</p>
                                                    }.into_view()
                                                } else {
                                                    let slices_for_legend = model_slices.clone();
                                                    view! {
                                                        <div class="usage-model-body">
                                                            <div class="usage-model-pie" style=format!("background:{model_gradient}")
                                                                role="img" aria-label=t(loc, "settings.usage.model_share")>
                                                                <span>
                                                                    <strong>{tokens(model_total)}</strong>
                                                                    <small>{t(loc, "settings.usage.tokens")}</small>
                                                                </span>
                                                            </div>
                                                            <div class="usage-model-legend">
                                                                {slices_for_legend.into_iter().map(|slice| {
                                                                    let pct = slice.tokens as f64 / model_total.max(1) as f64 * 100.0;
                                                                    view! {
                                                                        <div class="usage-model-row" data-testid="usage-model-row">
                                                                            <i style:background=slice.color></i>
                                                                            <span title=slice.label.clone()>{slice.label}</span>
                                                                            <strong>{format!("{pct:.1}%")}</strong>
                                                                            <small>{tokens(slice.tokens)}</small>
                                                                        </div>
                                                                    }
                                                                }).collect_view()}
                                                            </div>
                                                        </div>
                                                    }.into_view()
                                                }}
                                            </section>
                                            <section class="usage-card usage-tool-rank-card" data-testid="usage-tool-rank">
                                                <div class="usage-card-head">
                                                    <div>
                                                        <h3>{t(loc, "settings.usage.tool_rank")}</h3>
                                                        <p>{t(loc, "settings.usage.tool_rank_hint")}</p>
                                                    </div>
                                                </div>
                                                {if tool_rows.is_empty() {
                                                    view! {
                                                        <p class="settings-field-hint">{t(loc, "settings.usage.tools_empty")}</p>
                                                    }.into_view()
                                                } else {
                                                    view! {
                                                        <div class="usage-tool-rank-list">
                                                            {tool_rows.into_iter().enumerate().map(|(index, row)| {
                                                                let pct = row.calls as f64
                                                                    / tool_total.max(1) as f64
                                                                    * 100.0;
                                                                let width = row.calls as f64
                                                                    / tool_max as f64
                                                                    * 100.0;
                                                                let badge = match row.kind.as_str() {
                                                                    "skill" => t(loc, "settings.usage.tool_kind_skill"),
                                                                    "mcp" => t(loc, "settings.usage.tool_kind_mcp"),
                                                                    _ => t(loc, "settings.usage.other_tools"),
                                                                };
                                                                view! {
                                                                    <div class="usage-tool-rank-row" data-testid="usage-tool-rank-row">
                                                                        <span class="usage-tool-rank-index">{index + 1}</span>
                                                                        <span class=format!("usage-tool-rank-badge kind-{}", row.kind)>{badge}</span>
                                                                        <div class="usage-tool-rank-main">
                                                                            <div class="usage-tool-rank-meta">
                                                                                <span title=row.name.clone()>{row.name}</span>
                                                                                <strong>{format!("{pct:.1}%")}</strong>
                                                                                <small>{row.calls}</small>
                                                                            </div>
                                                                            <div class="usage-tool-rank-track" aria-hidden="true">
                                                                                <i style=format!(
                                                                                    "width:{width:.1}%;background:{}",
                                                                                    row.color
                                                                                )></i>
                                                                            </div>
                                                                        </div>
                                                                    </div>
                                                                }
                                                            }).collect_view()}
                                                        </div>
                                                    }.into_view()
                                                }}
                                            </section>
                                            </div>
                                            <section class="usage-card usage-workspaces-card">
                                                <div class="usage-card-head">
                                                    <div>
                                                        <h3>{t(loc, "settings.usage.workspaces")}</h3>
                                                        <p>{t(loc, "settings.usage.workspaces_hint")}</p>
                                                    </div>
                                                </div>
                                                <div class="usage-table">
                                                    <div class="usage-row usage-row-head">
                                                        <span>{t(loc, "settings.usage.workspace")}</span>
                                                        <span class="usage-num">{t(loc, "settings.usage.input")}</span>
                                                        <span class="usage-num">{t(loc, "settings.usage.output")}</span>
                                                        <span class="usage-num">{t(loc, "settings.usage.reasoning")}</span>
                                                        <span class="usage-num">{t(loc, "settings.usage.cached")}</span>
                                                    </div>
                                                    {overview.workspaces.into_iter().map(|row| {
                                                        let selected = row.clone();
                                                        let name = if row.name.trim().is_empty() {
                                                            row.project_id.clone()
                                                        } else {
                                                            row.name.clone()
                                                        };
                                                        let sessions = tf(loc, "settings.usage.sessions", &[
                                                            ("n", &row.session_count.max(0).to_string()),
                                                        ]);
                                                        view! {
                                                            <button type="button" class="usage-row usage-workspace-row"
                                                                data-testid="usage-workspace-row"
                                                                data-workspace-id=row.project_id
                                                                aria-label=tf(loc, "settings.usage.open_workspace", &[("name", &name)])
                                                                on:click=move |_| {
                                                                    usage_session_page.set(0);
                                                                    session_token_usage.set(None);
                                                                    selected_usage_workspace.set(Some(selected.clone()));
                                                                }>
                                                                <span class="usage-session">
                                                                    <span class="usage-session-title">{name}</span>
                                                                    <span class="usage-session-when">
                                                                        {sessions}" · "{format_relative_time(row.updated_at, loc)}
                                                                    </span>
                                                                    {(!row.workspace_dir.is_empty()).then(|| view! {
                                                                        <span class="usage-workspace-path" title=row.workspace_dir.clone()>{row.workspace_dir}</span>
                                                                    })}
                                                                </span>
                                                                <span class="usage-num">{tokens(row.input)}</span>
                                                                <span class="usage-num">{tokens(row.output)}</span>
                                                                <span class="usage-num">{tokens(row.reasoning)}</span>
                                                                <span class="usage-num">{tokens(row.cached)}</span>
                                                            </button>
                                                        }
                                                    }).collect_view()}
                                                </div>
                                            </section>
                                        </div>
                                    }.into_view()
                                }
                            }
                        }}
                    </div>
                })}
                {move || (settings_section.get() == "appearance").then(|| view! {
                    <div class="settings-pane settings-appearance-pane">
                        <section class="appearance-theme-section">
                            <h3>{move || t(locale.get(), "appearance.theme")}</h3>
                            <div class="theme-mode-grid" role="radiogroup"
                                aria-label=move || t(locale.get(), "appearance.theme")>
                                {[
                                    ("system", "appearance.system", "theme-preview-system"),
                                    ("light", "appearance.light", "theme-preview-light"),
                                    ("dark", "appearance.dark", "theme-preview-dark"),
                                ].into_iter().map(|(mode, label_key, preview_class)| view! {
                                    <button type="button"
                                        class="theme-mode-card"
                                        class:active=move || theme_mode.get() == mode
                                        aria-pressed=move || theme_mode.get() == mode
                                        data-testid=format!("theme-mode-{mode}")
                                        on:click=move |_| theme_mode.set(mode.into())>
                                        <span class=format!("theme-mode-preview {preview_class}") aria-hidden="true">
                                            <span class="theme-preview-window">
                                                <span class="theme-preview-sidebar"></span>
                                                <span class="theme-preview-content">
                                                    <i></i><i></i><i></i>
                                                </span>
                                            </span>
                                        </span>
                                        <span>{move || t(locale.get(), label_key)}</span>
                                    </button>
                                }).collect_view()}
                            </div>
                        </section>
                        <div class="appearance-diff-preview" aria-hidden="true">
                            <div class="appearance-diff-column is-removed">
                                <div><b>"1"</b><code><em>"const"</em> " themePreview: "<i>"ThemeConfig"</i>" = {"</code></div>
                                <div><b>"2"</b><code>"  surface: "<span>"\"sidebar\""</span>","</code></div>
                                <div><b>"3"</b><code>"  accent: "<span>"\"#2563eb\""</span>","</code></div>
                                <div><b>"4"</b><code>"  contrast: "<strong>"42"</strong>","</code></div>
                                <div><b>"5"</b><code>"};"</code></div>
                            </div>
                            <div class="appearance-diff-column is-added">
                                <div><b>"1"</b><code><em>"const"</em> " themePreview: "<i>"ThemeConfig"</i>" = {"</code></div>
                                <div><b>"2"</b><code>"  surface: "<span>"\"sidebar-elevated\""</span>","</code></div>
                                <div><b>"3"</b><code>"  accent: "<span>"\"#0ea5e9\""</span>","</code></div>
                                <div><b>"4"</b><code>"  contrast: "<strong>"68"</strong>","</code></div>
                                <div><b>"5"</b><code>"};"</code></div>
                            </div>
                        </div>
                        {move || {
                            let dark = theme_mode.get() == "dark";
                            let palette = if dark { dark_palette.get() } else { light_palette.get() };
                            let (accent, background, foreground) = appearance_palette_meta(dark, &palette);
                            let accent_ink = if dark && palette == "gruvbox" { "#1D2021" } else { "#FFFFFF" };
                            let background_ink = if dark { "#FFFFFF" } else { "#1F2328" };
                            let foreground_ink = if dark { "#1F2328" } else { "#FFFFFF" };
                            let options = appearance_palette_options(dark);
                            view! {
                                <section class="appearance-config-card">
                                    <div class="appearance-config-head">
                                        <strong>{t(locale.get(), if dark { "appearance.dark_theme" } else { "appearance.light_theme" })}</strong>
                                        <select data-testid="appearance-palette-select"
                                            aria-label=t(locale.get(), "appearance.palette")
                                            on:change=move |ev| {
                                                let value = dom_value(&ev);
                                                if dark { dark_palette.set(value); } else { light_palette.set(value); }
                                            }>
                                            {options.into_iter().map(|(value, name)| view! {
                                                <option value=value
                                                    prop:selected=move || if dark {
                                                        dark_palette.get() == value
                                                    } else {
                                                        light_palette.get() == value
                                                    }>{name}</option>
                                            }).collect_view()}
                                        </select>
                                    </div>
                                    <div class="appearance-config-row">
                                        <strong>{t(locale.get(), "appearance.accent")}</strong>
                                        <output class="appearance-color-value" style=format!("--appearance-color:{accent};--appearance-ink:{accent_ink}")><i></i>{accent}</output>
                                    </div>
                                    <div class="appearance-config-row">
                                        <strong>{t(locale.get(), "appearance.background")}</strong>
                                        <output class="appearance-color-value" style=format!("--appearance-color:{background};--appearance-ink:{background_ink}")><i></i>{background}</output>
                                    </div>
                                    <div class="appearance-config-row">
                                        <strong>{t(locale.get(), "appearance.foreground")}</strong>
                                        <output class="appearance-color-value" style=format!("--appearance-color:{foreground};--appearance-ink:{foreground_ink}")><i></i>{foreground}</output>
                                    </div>
                                    <div class="appearance-config-row">
                                        <div>
                                            <strong>{t(locale.get(), "appearance.ui_font_size")}</strong>
                                            <span>{t(locale.get(), "appearance.ui_font_size_hint")}</span>
                                        </div>
                                        <label class="font-size-control">
                                            <input type="range" min="0" max="30" step="1"
                                                aria-label=t(locale.get(), "appearance.ui_font_size")
                                                prop:value=move || ui_font_size.get().to_string()
                                                on:input=move |ev| ui_font_size.set(event_target_value(&ev).parse().unwrap_or(14)) />
                                            <output>{move || format!("{} px", ui_font_size.get())}</output>
                                        </label>
                                    </div>
                                    <div class="appearance-config-row">
                                        <div>
                                            <strong>{t(locale.get(), "appearance.ui_font_family")}</strong>
                                            <span>{t(locale.get(), "appearance.ui_font_family_hint")}</span>
                                        </div>
                                        <input type="text" class="appearance-font-input" data-testid="appearance-ui-font"
                                            aria-label=t(locale.get(), "appearance.ui_font_family")
                                            placeholder="Inter"
                                            prop:value=move || ui_font_family.get()
                                            on:input=move |ev| ui_font_family.set(event_target_value(&ev)) />
                                    </div>
                                    <div class="appearance-config-row">
                                        <div>
                                            <strong>{t(locale.get(), "appearance.code_font_size")}</strong>
                                            <span>{t(locale.get(), "appearance.code_font_size_hint")}</span>
                                        </div>
                                        <label class="font-size-control">
                                            <input type="range" min="0" max="30" step="1"
                                                aria-label=t(locale.get(), "appearance.code_font_size")
                                                prop:value=move || code_font_size.get().to_string()
                                                on:input=move |ev| code_font_size.set(event_target_value(&ev).parse().unwrap_or(12)) />
                                            <output>{move || format!("{} px", code_font_size.get())}</output>
                                        </label>
                                    </div>
                                    <div class="appearance-config-row">
                                        <div>
                                            <strong>{t(locale.get(), "appearance.code_font_family")}</strong>
                                            <span>{t(locale.get(), "appearance.code_font_family_hint")}</span>
                                        </div>
                                            <input type="text" class="appearance-font-input" data-testid="appearance-code-font"
                                            aria-label=t(locale.get(), "appearance.code_font_family")
                                            placeholder="JetBrains Mono"
                                            prop:value=move || code_font_family.get()
                                            on:input=move |ev| code_font_family.set(event_target_value(&ev)) />
                                    </div>
                                </section>
                            }
                        }}
                        <section class="appearance-config-card appearance-custom-css-card" data-testid="appearance-custom-css-card">
                            <div class="appearance-custom-css-head">
                                <div class="appearance-config-row">
                                    <strong>{move || t(locale.get(), "appearance.custom_css")}</strong>
                                    <div class="appearance-custom-css-actions">
                                        <label class="appearance-custom-css-import">
                                            <input
                                                type="file"
                                                accept=".css,text/css"
                                                data-testid="appearance-custom-css-file"
                                                on:change=move |ev| import_custom_css_from_input(&ev, custom_css)
                                            />
                                            {compose_icon("upload")}
                                            {move || t(locale.get(), "appearance.custom_css_import")}
                                        </label>
                                        <button type="button" data-testid="appearance-custom-css-clear"
                                            disabled=move || custom_css.get().is_empty()
                                            on:click=move |_| custom_css.set(String::new())>
                                            {move || t(locale.get(), "appearance.custom_css_clear")}
                                        </button>
                                    </div>
                                </div>
                                <div class="appearance-custom-css-meta">
                                    <span>{move || t(locale.get(), "appearance.custom_css_hint")}</span>
                                    <span data-testid="appearance-custom-css-status">
                                        {move || tf(locale.get(), "appearance.custom_css_bytes", &[("n", &custom_css.get().len().to_string())])}
                                    </span>
                                </div>
                            </div>
                            <textarea
                                data-testid="appearance-custom-css"
                                spellcheck="false"
                                aria-label=move || t(locale.get(), "appearance.custom_css")
                                prop:value=move || custom_css.get()
                                on:input=move |ev| custom_css.set(event_target_value(&ev))>
                            </textarea>
                        </section>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "models").then(|| {
                    if acp_form_open.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage acp-agents-pane" data-testid="acp-agents-settings">
                                <div class="conn-form model-form">
                                    <div class="settings-form-grid">
                                        <label class="span-2">{move || t(locale.get(), "models.acp_label")}
                                            <input data-testid="acp-agent-label"
                                                prop:value=move || acp_form.get().map(|f| f.label.clone()).unwrap_or_default()
                                                on:input=move |ev| acp_form.update(|o| if let Some(o)=o { o.label = event_target_value(&ev); }) /></label>
                                        <label class="span-2">{move || t(locale.get(), "models.acp_command")}
                                            <input data-testid="acp-agent-command"
                                                prop:value=move || acp_form.get().map(|f| f.command.clone()).unwrap_or_default()
                                                on:input=move |ev| acp_form.update(|o| if let Some(o)=o { o.command = event_target_value(&ev); }) /></label>
                                        <label class="span-2">{move || t(locale.get(), "models.acp_args")}
                                            <textarea data-testid="acp-agent-args" rows="5"
                                                prop:value=move || acp_form.get().map(|f| f.args.join("\n")).unwrap_or_default()
                                                on:input=move |ev| acp_form.update(|o| if let Some(o)=o {
                                                    o.args = event_target_value(&ev).split('\n').map(|arg| arg.to_string()).collect();
                                                })></textarea></label>
                                    </div>
                                    <span class="hint">{move || t(locale.get(), "models.acp_subpage_hint")}</span>
                                    {move || acp_form_msg.get().map(|(ok, text)| view! {
                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                    })}
                                    <div class="row settings-footer">
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| {
                                            acp_form.set(None);
                                            acp_form_msg.set(None);
                                        }>{move || t(locale.get(), "settings.cancel")}</button>
                                        <button type="button" class="primary" data-testid="save-acp-agent" disabled=move || settings_busy.get()
                                            on:click=move |_| {
                                                let Some(mut profile) = acp_form.get() else { return; };
                                                profile.label = profile.label.trim().to_string();
                                                profile.command = profile.command.trim().to_string();
                                                if profile.label.is_empty() || profile.command.is_empty() {
                                                    acp_form_msg.set(Some((false, t(locale.get(), "models.acp_required").to_string())));
                                                    return;
                                                }
                                                let saved = t(locale.get(), "models.acp_saved").to_string();
                                                spawn_local(async move {
                                                    settings_busy.set(true);
                                                    let arg = to_value(&serde_json::json!({ "profile": profile })).unwrap();
                                                    match invoke_checked("save_acp_agent", arg).await {
                                                        Ok(value) => match serde_wasm_bindgen::from_value::<Vec<AcpAgentProfile>>(value) {
                                                            Ok(list) => {
                                                                acp_agents.set(list);
                                                                acp_form.set(None);
                                                                acp_form_msg.set(Some((true, saved)));
                                                                show_acp_agents.set(true);
                                                            }
                                                            Err(error) => {
                                                                acp_form_msg.set(Some((false, error.to_string())));
                                                            }
                                                        },
                                                        Err(error) => {
                                                            acp_form_msg.set(Some((false, js_error_text(error))));
                                                        }
                                                    }
                                                    settings_busy.set(false);
                                                });
                                            }>{move || t(locale.get(), "models.acp_save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    } else if model_form_open.get() {
                        if model_form_is_edit.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                <div class="conn-form model-form">
                                    <div class="settings-form-grid">
                                        <label class="span-2">{move || t(locale.get(), "settings.api_url")}
                                            <input aria-describedby="model-api-url-hint"
                                                prop:value=move || model_form.get().map(|f| f.api_url.clone()).unwrap_or_default()
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.api_url = event_target_input(&ev).value(); }) /></label>
                                        <span id="model-api-url-hint" class="hint span-2" data-testid="model-api-url-hint">
                                            {move || t(locale.get(), "settings.tip")}
                                        </span>
                                        <label class="span-2">{move || t(locale.get(), "settings.api_key")}
                                            <input type="password" id="model-form-api-key" prop:value=move || model_form_key.get()
                                                placeholder=move || {
                                                    let Some(id) = model_form.get().and_then(|f| f.id) else { return String::new(); };
                                                    if models.get().iter().any(|m| m.id == id && m.has_api_key) {
                                                        t(locale.get(), "settings.stored_key").to_string()
                                                    } else {
                                                        String::new()
                                                    }
                                                }
                                                autocomplete="new-password"
                                                on:input=move |ev| model_form_key.set(event_target_input(&ev).value()) /></label>
                                        <label>{move || t(locale.get(), "settings.provider")}
                                            <select data-testid="settings-provider"
                                                on:change=move|ev| {
                                                    let p = dom_value(&ev);
                                                    model_form.update(|o| if let Some(o)=o {
                                                        o.provider = settings_provider_value(&p).into();
                                                    });
                                                    apply_catalog_limits(model_form, model_catalog_limits);
                                                }
                                                >
                                                <option value="openai"
                                                    prop:selected=move || model_form.get().is_some_and(|f| settings_provider_value(&f.provider) == "openai")>
                                                    {move || t(locale.get(), "settings.provider.openai")}
                                                </option>
                                                <option value="openai_responses"
                                                    prop:selected=move || model_form.get().is_some_and(|f| settings_provider_value(&f.provider) == "openai_responses")>
                                                    {move || t(locale.get(), "settings.provider.openai_responses")}
                                                </option>
                                                <option value="anthropic"
                                                    prop:selected=move || model_form.get().is_some_and(|f| settings_provider_value(&f.provider) == "anthropic")>
                                                    {move || t(locale.get(), "settings.provider.anthropic")}
                                                </option>
                                            </select>
                                        </label>
                                        <label>{move || t(locale.get(), "settings.model")}
                                            <input prop:value=move || model_form.get().map(|f| f.model.clone()).unwrap_or_default()
                                                placeholder=move || t(locale.get(), "settings.model_ph")
                                                on:input=move |ev| {
                                                    model_form.update(|o| if let Some(o)=o {
                                                        o.model = event_target_input(&ev).value();
                                                        if is_image_generation_model(&o.model) {
                                                            o.supports_vision = false;
                                                            o.use_for_vision = false;
                                                            o.use_for_image_generation = true;
                                                            o.use_for_video_generation = false;
                                                        } else if is_video_generation_model(&o.model) {
                                                            o.supports_vision = false;
                                                            o.use_for_vision = false;
                                                            o.use_for_image_generation = false;
                                                            o.use_for_video_generation = true;
                                                        }
                                                    });
                                                    apply_catalog_limits(model_form, model_catalog_limits);
                                                } /></label>
                                        <label>{move || t(locale.get(), "settings.endpoint_suffix")}
                                            <input data-testid="model-endpoint-suffix"
                                                prop:value=move || model_form.get().map(|f| f.endpoint_suffix.clone()).unwrap_or_default()
                                                placeholder=move || t(locale.get(), "settings.endpoint_suffix_ph")
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o {
                                                    o.endpoint_suffix = event_target_input(&ev).value();
                                                }) /></label>
                                        <label>{move || t(locale.get(), "settings.label")}
                                            <input prop:value=move || model_form.get().map(|f| f.label.clone()).unwrap_or_default()
                                                placeholder=move || t(locale.get(), "settings.label_ph")
                                                on:input=move |ev| model_form.update(|o| if let Some(o)=o { o.label = event_target_input(&ev).value(); }) /></label>
                                        {move || {
                                            let image = model_form.get().is_some_and(|f| is_image_generation_model(&f.model));
                                            // A model id is never both image and video, but keep the
                                            // branches mutually exclusive anyway.
                                            let video = !image && model_form.get().is_some_and(|f| is_video_generation_model(&f.model));
                                            if video {
                                                view! {
                                                    <label>{move || t(locale.get(), "settings.video_duration")}
                                                        <input type="number" min="1" max="15" step="1" data-testid="video-duration"
                                                            on:input=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                let secs = dom_value(&ev).parse::<u32>().unwrap_or(5).clamp(1, 15);
                                                                o.video_duration_secs = Some(secs);
                                                            })
                                                            prop:value=move || model_form.get()
                                                                .and_then(|f| f.video_duration_secs)
                                                                .map(|d| d.to_string())
                                                                .unwrap_or_else(|| "5".into()) />
                                                    </label>
                                                    <label>{move || t(locale.get(), "settings.video_aspect_ratio")}
                                                        <select data-testid="video-aspect-ratio"
                                                            on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                o.video_aspect_ratio = Some(dom_value(&ev));
                                                            })>
                                                            {VIDEO_ASPECT_RATIOS.iter().map(|value| {
                                                                let selected = model_form.get().is_some_and(|f| {
                                                                    f.video_aspect_ratio.as_deref() == Some(*value)
                                                                        || (f.video_aspect_ratio.is_none() && *value == "16:9")
                                                                });
                                                                view! { <option value=*value selected=selected>{*value}</option> }
                                                            }).collect_view()}
                                                        </select>
                                                    </label>
                                                    <label>{move || t(locale.get(), "settings.video_resolution")}
                                                        <select data-testid="video-resolution"
                                                            on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                o.video_resolution = Some(dom_value(&ev));
                                                            })>
                                                            {VIDEO_RESOLUTIONS.iter().map(|value| {
                                                                let selected = model_form.get().is_some_and(|f| {
                                                                    f.video_resolution.as_deref() == Some(*value)
                                                                        || (f.video_resolution.is_none() && *value == "720p")
                                                                });
                                                                view! { <option value=*value selected=selected>{*value}</option> }
                                                            }).collect_view()}
                                                        </select>
                                                    </label>
                                                    <span class="hint span-2">{move || t(locale.get(), "settings.video_defaults_hint")}</span>
                                                    <label class="settings-check span-2">
                                                        <input type="checkbox" data-testid="use-for-video-generation"
                                                            prop:checked=move || model_form.get().map(|f| f.use_for_video_generation).unwrap_or(false)
                                                            on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                o.use_for_video_generation = event_target_checked(&ev);
                                                                if o.use_for_video_generation {
                                                                    o.use_for_image_generation = false;
                                                                }
                                                            }) />
                                                        <span>{move || t(locale.get(), "settings.use_for_video_generation")}</span>
                                                    </label>
                                                    <span class="hint span-2">{move || t(locale.get(), "settings.video_generation_hint")}</span>
                                                }.into_view()
                                            } else if image {
                                                view! {
                                                    {move || {
                                                        let grok = model_form.get().is_some_and(|f| is_grok_imagine_model(&f.model));
                                                        if grok {
                                                            view! {
                                                                <label>{move || t(locale.get(), "settings.image_aspect_ratio")}
                                                                    <select data-testid="image-aspect-ratio"
                                                                        on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                            o.image_aspect_ratio = dom_value(&ev);
                                                                        })>
                                                                        {GROK_IMAGE_ASPECT_RATIOS.iter().map(|value| {
                                                                            let selected = model_form.get().is_some_and(|f| {
                                                                                f.image_aspect_ratio == *value
                                                                                    || (f.image_aspect_ratio.is_empty() && *value == "auto")
                                                                            });
                                                                            view! { <option value=*value selected=selected>{*value}</option> }
                                                                        }).collect_view()}
                                                                    </select>
                                                                </label>
                                                                <label>{move || t(locale.get(), "settings.image_resolution")}
                                                                    <select data-testid="image-resolution"
                                                                        on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                            o.image_resolution = dom_value(&ev);
                                                                        })>
                                                                        {GROK_IMAGE_RESOLUTIONS.iter().map(|value| {
                                                                            let selected = model_form.get().is_some_and(|f| {
                                                                                f.image_resolution == *value
                                                                                    || (f.image_resolution.is_empty() && *value == "1k")
                                                                            });
                                                                            view! { <option value=*value selected=selected>{*value}</option> }
                                                                        }).collect_view()}
                                                                    </select>
                                                                </label>
                                                                <label>{move || t(locale.get(), "settings.image_quality")}
                                                                    <select data-testid="image-quality"
                                                                        on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                            o.image_quality = dom_value(&ev);
                                                                        })>
                                                                        {GROK_IMAGE_QUALITIES.iter().map(|value| {
                                                                            let selected = model_form.get().is_some_and(|f| {
                                                                                f.image_quality == *value
                                                                                    || (f.image_quality.is_empty() && *value == "medium")
                                                                            });
                                                                            view! { <option value=*value selected=selected>{*value}</option> }
                                                                        }).collect_view()}
                                                                    </select>
                                                                </label>
                                                            }.into_view()
                                                        } else {
                                                            view! {
                                                                <label>{move || t(locale.get(), "settings.image_size")}
                                                                    <select data-testid="image-size"
                                                                        on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                            o.image_size = dom_value(&ev);
                                                                        })>
                                                                        {OPENAI_IMAGE_SIZES.iter().map(|value| {
                                                                            let selected = model_form.get().is_some_and(|f| {
                                                                                f.image_size == *value
                                                                                    || (f.image_size.is_empty() && *value == "auto")
                                                                            });
                                                                            view! { <option value=*value selected=selected>{*value}</option> }
                                                                        }).collect_view()}
                                                                    </select>
                                                                </label>
                                                                <label>{move || t(locale.get(), "settings.image_quality")}
                                                                    <select data-testid="image-quality"
                                                                        on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                            o.image_quality = dom_value(&ev);
                                                                        })>
                                                                        {OPENAI_IMAGE_QUALITIES.iter().map(|value| {
                                                                            let selected = model_form.get().is_some_and(|f| {
                                                                                f.image_quality == *value
                                                                                    || (f.image_quality.is_empty() && *value == "auto")
                                                                            });
                                                                            view! { <option value=*value selected=selected>{*value}</option> }
                                                                        }).collect_view()}
                                                                    </select>
                                                                </label>
                                                            }.into_view()
                                                        }
                                                    }}
                                                    <span class="hint span-2">{move || t(locale.get(), "settings.image_defaults_hint")}</span>
                                                    <label class="settings-check span-2">
                                                        <input type="checkbox" data-testid="use-for-image-generation"
                                                            prop:checked=move || model_form.get().map(|f| f.use_for_image_generation).unwrap_or(false)
                                                            on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                                o.use_for_image_generation = event_target_checked(&ev);
                                                            }) />
                                                        <span>{move || t(locale.get(), "settings.use_for_image_generation")}</span>
                                                    </label>
                                                    <span class="hint span-2">{move || t(locale.get(), "settings.image_generation_hint")}</span>
                                                }.into_view()
                                            } else {
                                                view! {
                                        <label>{move || t(locale.get(), "settings.max_tokens")}
                                            <input type="number" min="16" step="1"
                                                attr:max=move || model_catalog_limits.get().map(|d| d.max_tokens.to_string())
                                                on:input=move|ev| model_form.update(|o| if let Some(o)=o {
                                                    o.max_tokens = dom_value(&ev).parse().unwrap_or(0);
                                                })
                                                prop:value=move || model_form.get().map(|f| f.max_tokens.to_string()).unwrap_or_else(|| "8192".into()) />
                                        </label>
                                        <label>{move || t(locale.get(), "settings.context_window")}
                                            <input type="number" min="4096" step="1024"
                                                attr:max=move || model_catalog_limits.get().map(|d| d.context_window.to_string())
                                                on:input=move|ev| model_form.update(|o| if let Some(o)=o {
                                                    o.context_window = dom_value(&ev).parse().unwrap_or(0);
                                                })
                                                prop:value=move || model_form.get().map(|f| f.context_window.to_string()).unwrap_or_else(|| "128000".into()) />
                                        </label>
                                        {move || model_catalog_limits.get().map(|d| view! {
                                            <span class="hint" data-testid="model-catalog-limits-hint">
                                                {tf(locale.get(), "settings.catalog_limits_hint", &[
                                                    ("context", &d.context_window.to_string()),
                                                    ("output", &d.max_tokens.to_string()),
                                                ])}
                                            </span>
                                        })}
                                        <label>{move || t(locale.get(), "settings.reasoning_effort")}
                                            {move || {
                                                let form = model_form.get();
                                                let current = form.as_ref().map(|f| f.reasoning_effort.clone()).unwrap_or_default();
                                                let provider = form.as_ref().map(|f| f.provider.clone()).unwrap_or_default();
                                                let model = form.as_ref().map(|f| f.model.clone()).unwrap_or_default();
                                                let mut values: Vec<String> = known_effort_values(&provider, &model)
                                                    .unwrap_or(ALL_EFFORT_VALUES)
                                                    .iter()
                                                    .map(|v| v.to_string())
                                                    .collect();
                                                // Keep a saved value visible even when the curated
                                                // list for this model no longer includes it.
                                                if !current.is_empty() && !values.iter().any(|v| v == &current) {
                                                    values.push(current.clone());
                                                }
                                                let loc = locale.get();
                                                view! {
                                                    <select
                                                        on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                            let v = dom_value(&ev);
                                                            o.reasoning_effort = if v == "default" { String::new() } else { v };
                                                        })
                                                        >
                                                        <option value="default" selected=current.is_empty()>
                                                            {t(loc, "settings.reasoning_effort.default")}
                                                        </option>
                                                        {values.into_iter().map(|v| {
                                                            let sel = v == current;
                                                            view! { <option value=v.clone() selected=sel>{v}</option> }
                                                        }).collect_view()}
                                                    </select>
                                                }
                                            }}
                                        </label>
                                        // Hint lives OUTSIDE the <label> on purpose: its text mentions
                                        // "model", and nesting it would fold that into the <select>'s
                                        // accessible name, so getByLabel("Model") would match it (#e2e).
                                        <span class="hint effort-hint span-2">{move || {
                                            let form = model_form.get();
                                            let provider = form.as_ref().map(|f| f.provider.clone()).unwrap_or_default();
                                            let model = form.as_ref().map(|f| f.model.clone()).unwrap_or_default();
                                            let loc = locale.get();
                                            match known_effort_values(&provider, &model) {
                                                Some([]) => t(loc, "settings.reasoning_effort.unsupported_hint").to_string(),
                                                Some(list) => tf(loc, "settings.reasoning_effort.known_hint", &[("list", &list.join(" / "))]),
                                                None => t(loc, "settings.reasoning_effort.unknown_hint").to_string(),
                                            }
                                        }}</span>
                                        {move || {
                                            let form = model_form.get();
                                            let provider = form.as_ref().map(|f| settings_provider_value(&f.provider)).unwrap_or_default();
                                            matches!(provider, "openai" | "openai_responses").then(|| {
                                                let current = form.as_ref().map(|f| f.service_tier.clone()).unwrap_or_default();
                                                let loc = locale.get();
                                                let fast_selected = matches!(current.as_str(), "priority" | "fast");
                                                view! {
                                                    <div class="fast-setting span-2" class:enabled=fast_selected
                                                        data-testid="service-tier-toggle-row">
                                                        <span class="fast-setting-icon">{compose_icon("bolt")}</span>
                                                        <span class="fast-setting-copy">
                                                            <strong>{t(loc, "settings.service_tier")}</strong>
                                                            <small>{t(loc, if fast_selected {
                                                                "settings.service_tier.fast"
                                                            } else {
                                                                "settings.service_tier.default"
                                                            })}</small>
                                                        </span>
                                                        <label class="toggle fast-setting-toggle">
                                                            <input type="checkbox" data-testid="service-tier-toggle"
                                                                aria-label=t(loc, "settings.service_tier")
                                                                prop:checked=fast_selected
                                                                on:change=move |ev| model_form.update(|o| if let Some(o)=o {
                                                                    o.service_tier = if event_target_checked(&ev) {
                                                                        "priority".into()
                                                                    } else {
                                                                        String::new()
                                                                    };
                                                                }) />
                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                        </label>
                                                    </div>
                                                    <span class="hint span-2">{t(loc, "settings.service_tier.hint")}</span>
                                                }
                                            })
                                        }}
                                        <div class="span-2 settings-form-grid">
                                            <label class="settings-check">
                                                <input type="checkbox"
                                                    prop:checked=move || model_form.get().map(|f| f.supports_vision).unwrap_or(false)
                                                    on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                        o.supports_vision = event_target_checked(&ev);
                                                        if !o.supports_vision {
                                                            o.use_for_vision = false;
                                                        }
                                                    }) />
                                                <span>{move || t(locale.get(), "settings.supports_vision")}</span>
                                            </label>
                                            <label class="settings-check">
                                                <input type="checkbox"
                                                    prop:checked=move || model_form.get().map(|f| f.use_for_vision).unwrap_or(false)
                                                    on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                        o.use_for_vision = event_target_checked(&ev);
                                                        if o.use_for_vision {
                                                            o.supports_vision = true;
                                                        }
                                                    }) />
                                                <span>{move || t(locale.get(), "settings.use_for_vision")}</span>
                                            </label>
                                            <span class="hint span-2">{move || t(locale.get(), "settings.vision_hint")}</span>
                                            <label class="settings-check span-2">
                                                <input type="checkbox" data-testid="use-for-image-generation"
                                                    prop:checked=move || model_form.get().map(|f| f.use_for_image_generation).unwrap_or(false)
                                                    on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                        o.use_for_image_generation = event_target_checked(&ev);
                                                    }) />
                                                <span>{move || t(locale.get(), "settings.use_for_image_generation")}</span>
                                            </label>
                                            <span class="hint span-2">{move || t(locale.get(), "settings.image_generation_hint")}</span>
                                            <label class="settings-check span-2">
                                                <input type="checkbox" data-testid="use-for-video-generation"
                                                    prop:checked=move || model_form.get().map(|f| f.use_for_video_generation).unwrap_or(false)
                                                    on:change=move|ev| model_form.update(|o| if let Some(o)=o {
                                                        o.use_for_video_generation = event_target_checked(&ev);
                                                        if o.use_for_video_generation {
                                                            o.use_for_image_generation = false;
                                                        }
                                                    }) />
                                                <span>{move || t(locale.get(), "settings.use_for_video_generation")}</span>
                                            </label>
                                            <span class="hint span-2">{move || t(locale.get(), "settings.video_generation_hint")}</span>
                                        </div>
                                                }.into_view()
                                            }
                                        }}
                                    </div>
                                    {move || model_form_msg.get().map(|(ok, text)| view! {
                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                    })}
                                    <div class="row settings-footer">
                                            <button type="button" disabled=move || settings_busy.get() on:click=move |ev| validate_model_form.call(ev)>{move || t(locale.get(), "settings.validate")}</button>
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| close_settings_subpage.call(())>{move || t(locale.get(), "settings.cancel")}</button>
                                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_model_form.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                        } else {
                        view! {
                            <div class="settings-pane settings-pane-subpage" data-testid="provider-add-form">
                                <div class="conn-form model-form">
                                    <p class="hint" data-testid="provider-byok-hint">{move || t(locale.get(), "models.byok_hint")}</p>
                                    <div class="settings-form-grid">
                                        <label class="span-2">{move || t(locale.get(), "settings.api_url")}
                                            <input aria-describedby="model-api-url-hint" data-testid="provider-api-url"
                                                prop:value=move || model_form.get().map(|f| f.api_url.clone()).unwrap_or_default()
                                                on:input=move |ev| {
                                                    let url = event_target_input(&ev).value();
                                                    model_form.update(|o| if let Some(o)=o {
                                                        if provider_entries_are_pristine(o) {
                                                            apply_base_url_suggestions(o, &url);
                                                        } else {
                                                            o.api_url = url;
                                                        }
                                                    });
                                                } /></label>
                                        <span id="model-api-url-hint" class="hint span-2" data-testid="model-api-url-hint">
                                            {move || t(locale.get(), "settings.tip")}
                                        </span>
                                        <label class="span-2">{move || t(locale.get(), "settings.api_key")}
                                            <input type="password" id="model-form-api-key" data-testid="provider-api-key"
                                                prop:value=move || model_form_key.get()
                                                placeholder=move || {
                                                    let url = model_form.get().map(|f| f.api_url).unwrap_or_default();
                                                    if endpoint_has_stored_key(&models.get(), &url) {
                                                        tf(locale.get(), "models.reuse_key", &[("host", &endpoint_host(&url))])
                                                    } else {
                                                        String::new()
                                                    }
                                                }
                                                autocomplete="new-password"
                                                on:input=move |ev| model_form_key.set(event_target_input(&ev).value()) /></label>
                                        {move || {
                                            let url = model_form.get().map(|f| f.api_url).unwrap_or_default();
                                            endpoint_has_stored_key(&models.get(), &url).then(|| view! {
                                                <span class="hint span-2" data-testid="provider-separate-key-hint">
                                                    {t(locale.get(), "models.separate_key_hint")}
                                                </span>
                                            })
                                        }}
                                    </div>
                                    <div class="provider-models" data-testid="provider-models">
                                        <div class="provider-models-head">
                                            <strong>{move || t(locale.get(), "models.entries")}</strong>
                                            <span class="hint">{move || t(locale.get(), "models.entries_hint")}</span>
                                        </div>
                                        <For
                                            each=move || model_form.get().map(|f| f.entries).unwrap_or_default()
                                            key=|entry| entry.row_id
                                            let:entry
                                        >
                                            {
                                                let row_id = entry.row_id;
                                                view! {
                                                    <div class="provider-model-row" data-testid="provider-model-row">
                                                        <div class="provider-model-row-head">
                                                            <label class="provider-model-protocol">{move || t(locale.get(), "settings.provider")}
                                                                <select data-testid="provider-model-protocol"
                                                                    on:change=move |ev| {
                                                                        let value = dom_value(&ev);
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.provider = settings_provider_value(&value).into();
                                                                            }
                                                                        });
                                                                    }>
                                                                    <option value="openai"
                                                                        prop:selected=move || model_form.get()
                                                                            .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                            .is_some_and(|e| settings_provider_value(&e.provider) == "openai")>
                                                                        {move || t(locale.get(), "settings.provider.openai")}
                                                                    </option>
                                                                    <option value="openai_responses"
                                                                        prop:selected=move || model_form.get()
                                                                            .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                            .is_some_and(|e| settings_provider_value(&e.provider) == "openai_responses")>
                                                                        {move || t(locale.get(), "settings.provider.openai_responses")}
                                                                    </option>
                                                                    <option value="anthropic"
                                                                        prop:selected=move || model_form.get()
                                                                            .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                            .is_some_and(|e| settings_provider_value(&e.provider) == "anthropic")>
                                                                        {move || t(locale.get(), "settings.provider.anthropic")}
                                                                    </option>
                                                                </select>
                                                            </label>
                                                            <label class="provider-model-id">{move || t(locale.get(), "settings.model")}
                                                                <input data-testid="provider-model-id"
                                                                    prop:value=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.model)
                                                                        .unwrap_or_default()
                                                                    placeholder=move || t(locale.get(), "settings.model_ph")
                                                                    on:input=move |ev| {
                                                                        let value = event_target_input(&ev).value();
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.model = value;
                                                                                if is_image_generation_model(&e.model) {
                                                                                    e.supports_vision = false;
                                                                                    e.use_for_vision = false;
                                                                                    e.use_for_image_generation = true;
                                                                                    e.use_for_video_generation = false;
                                                                                } else if is_video_generation_model(&e.model) {
                                                                                    e.supports_vision = false;
                                                                                    e.use_for_vision = false;
                                                                                    e.use_for_image_generation = false;
                                                                                    e.use_for_video_generation = true;
                                                                                }
                                                                            }
                                                                        });
                                                                    } /></label>
                                                            <label class="provider-model-label">{move || t(locale.get(), "settings.label")}
                                                                <input data-testid="provider-model-label"
                                                                    prop:value=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.label)
                                                                        .unwrap_or_default()
                                                                    placeholder=move || t(locale.get(), "settings.label_ph")
                                                                    on:input=move |ev| {
                                                                        let value = event_target_input(&ev).value();
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.label = value;
                                                                            }
                                                                        });
                                                                    } /></label>
                                                            <label class="provider-model-endpoint">{move || t(locale.get(), "settings.endpoint_suffix")}
                                                                <input data-testid="provider-endpoint-suffix"
                                                                    prop:value=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.endpoint_suffix)
                                                                        .unwrap_or_default()
                                                                    placeholder=move || t(locale.get(), "settings.endpoint_suffix_ph")
                                                                    on:input=move |ev| {
                                                                        let value = event_target_input(&ev).value();
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.endpoint_suffix = value;
                                                                            }
                                                                        });
                                                                    } /></label>
                                                            <button type="button" class="settings-list-remove" data-testid="provider-remove-model"
                                                                title=move || t(locale.get(), "models.remove_entry")
                                                                disabled=move || model_form.get().is_some_and(|f| f.entries.len() < 2)
                                                                on:click=move |_| {
                                                                    model_form.update(|o| if let Some(o)=o {
                                                                        if o.entries.len() > 1 {
                                                                            o.entries.retain(|e| e.row_id != row_id);
                                                                        }
                                                                    });
                                                                }>{compose_icon("close")}</button>
                                                        </div>
                                                        <div class="provider-model-roles">
                                                            {move || {
                                                                let image = model_form.get()
                                                                    .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                    .is_some_and(|e| e.is_image_model());
                                                                let video = !image && model_form.get()
                                                                    .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                    .is_some_and(|e| e.is_video_model());
                                                                if video {
                                                                    view! {
                                                            <label class="settings-check">
                                                                <input type="checkbox" data-testid="provider-use-for-video"
                                                                    prop:checked=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.use_for_video_generation)
                                                                        .unwrap_or(false)
                                                                    on:change=move |ev| {
                                                                        let checked = event_target_checked(&ev);
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.use_for_video_generation = checked;
                                                                                if checked {
                                                                                    e.supports_vision = false;
                                                                                    e.use_for_vision = false;
                                                                                    e.use_for_image_generation = false;
                                                                                }
                                                                            }
                                                                        });
                                                                    } />
                                                                <span>{move || t(locale.get(), "settings.use_for_video_generation")}</span>
                                                            </label>
                                                                    }.into_view()
                                                                } else if image {
                                                                    view! {
                                                            <label class="settings-check">
                                                                <input type="checkbox" data-testid="provider-use-for-image"
                                                                    prop:checked=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.use_for_image_generation)
                                                                        .unwrap_or(false)
                                                                    on:change=move |ev| {
                                                                        let checked = event_target_checked(&ev);
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.use_for_image_generation = checked;
                                                                                if checked {
                                                                                    e.supports_vision = false;
                                                                                    e.use_for_vision = false;
                                                                                }
                                                                            }
                                                                        });
                                                                    } />
                                                                <span>{move || t(locale.get(), "settings.use_for_image_generation")}</span>
                                                            </label>
                                                                    }.into_view()
                                                                } else {
                                                                    view! {
                                                            <label class="settings-check">
                                                                <input type="checkbox"
                                                                    prop:checked=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.supports_vision)
                                                                        .unwrap_or(false)
                                                                    on:change=move |ev| {
                                                                        let checked = event_target_checked(&ev);
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.supports_vision = checked;
                                                                                if !checked {
                                                                                    e.use_for_vision = false;
                                                                                }
                                                                            }
                                                                        });
                                                                    } />
                                                                <span>{move || t(locale.get(), "settings.supports_vision")}</span>
                                                            </label>
                                                            <label class="settings-check">
                                                                <input type="checkbox"
                                                                    prop:checked=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.use_for_vision)
                                                                        .unwrap_or(false)
                                                                    on:change=move |ev| {
                                                                        let checked = event_target_checked(&ev);
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.use_for_vision = checked;
                                                                                if checked {
                                                                                    e.supports_vision = true;
                                                                                }
                                                                            }
                                                                        });
                                                                    } />
                                                                <span>{move || t(locale.get(), "settings.use_for_vision")}</span>
                                                            </label>
                                                            <label class="settings-check">
                                                                <input type="checkbox" data-testid="provider-use-for-image"
                                                                    prop:checked=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.use_for_image_generation)
                                                                        .unwrap_or(false)
                                                                    on:change=move |ev| {
                                                                        let checked = event_target_checked(&ev);
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.use_for_image_generation = checked;
                                                                                if checked {
                                                                                    e.supports_vision = false;
                                                                                    e.use_for_vision = false;
                                                                                }
                                                                            }
                                                                        });
                                                                    } />
                                                                <span>{move || t(locale.get(), "settings.use_for_image_generation")}</span>
                                                            </label>
                                                            <label class="settings-check">
                                                                <input type="checkbox" data-testid="provider-use-for-video"
                                                                    prop:checked=move || model_form.get()
                                                                        .and_then(|f| f.entries.into_iter().find(|e| e.row_id == row_id))
                                                                        .map(|e| e.use_for_video_generation)
                                                                        .unwrap_or(false)
                                                                    on:change=move |ev| {
                                                                        let checked = event_target_checked(&ev);
                                                                        model_form.update(|o| if let Some(o)=o {
                                                                            if let Some(e) = o.entries.iter_mut().find(|e| e.row_id == row_id) {
                                                                                e.use_for_video_generation = checked;
                                                                                if checked {
                                                                                    e.supports_vision = false;
                                                                                    e.use_for_vision = false;
                                                                                    e.use_for_image_generation = false;
                                                                                }
                                                                            }
                                                                        });
                                                                    } />
                                                                <span>{move || t(locale.get(), "settings.use_for_video_generation")}</span>
                                                            </label>
                                                                    }.into_view()
                                                                }
                                                            }}
                                                        </div>
                                                    </div>
                                                }
                                            }
                                        </For>
                                        <button type="button" class="settings-add-btn" data-testid="provider-add-model"
                                            on:click=move |_| {
                                                model_form.update(|o| if let Some(o)=o {
                                                    o.entries.push(model_form_entry("openai", "", "", false));
                                                });
                                            }>
                                            {compose_icon("plus")}
                                            {move || t(locale.get(), "models.add_entry")}
                                        </button>
                                    </div>
                                    {move || model_form_msg.get().map(|(ok, text)| view! {
                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                    })}
                                    <div class="row settings-footer">
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |ev| validate_model_form.call(ev)>{move || t(locale.get(), "settings.validate")}</button>
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| close_settings_subpage.call(())>{move || t(locale.get(), "settings.cancel")}</button>
                                        <button type="button" class="primary" data-testid="save-provider" disabled=move || settings_busy.get() on:click=move |ev| save_model_form.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                        }
                    } else {
                        view! {
                        <div class="settings-pane settings-pane-list model-settings-pane">
                            <crate::overlays::LocalEnvironmentPanel locale=locale bootstrap=bootstrap />
                            <div class="settings-toolbar settings-toolbar-end model-category-toolbar">
                                <div class="settings-category-tabs" role="tablist" aria-label="Model categories">
                                    <button type="button" role="tab" class="settings-category-tab"
                                        class:active=move || !show_acp_agents.get()
                                        aria-selected=move || (!show_acp_agents.get()).to_string()
                                        data-testid="models-category-http"
                                        on:click=move |_| show_acp_agents.set(false)>
                                        {move || {
                                            let n = models.get().len();
                                            format!("{} ({n})", t(locale.get(), "models.category.http"))
                                        }}
                                    </button>
                                    <button type="button" role="tab" class="settings-category-tab"
                                        class:active=move || show_acp_agents.get()
                                        aria-selected=move || show_acp_agents.get().to_string()
                                        data-testid="open-acp-agents-from-settings"
                                        on:click=move |_| show_acp_agents.set(true)>
                                        {move || {
                                            let n = acp_agents.get().len();
                                            format!("{} ({n})", t(locale.get(), "models.acp_open"))
                                        }}
                                    </button>
                                </div>
                                <div class="settings-toolbar-actions">
                                    {move || if show_acp_agents.get() {
                                        view! {
                                            <button type="button" class="settings-add-btn" data-testid="add-acp-agent-settings" on:click=move |_| {
                                                show_acp_agents.set(true);
                                                acp_form.set(Some(new_acp_form()));
                                                acp_form_msg.set(None);
                                            }>{move || t(locale.get(), "models.add_acp")}</button>
                                        }.into_view()
                                    } else {
                                        view! {
                                            <button type="button" class="settings-add-btn" data-testid="add-provider" on:click=move |_| {
                                                show_acp_agents.set(false);
                                                let form = new_model_form();
                                                let reuse = endpoint_has_stored_key(&models.get(), &form.api_url);
                                                model_form.set(Some(form));
                                                model_form_key.set(String::new());
                                                model_form_msg.set(None);
                                                if !reuse {
                                                    focus_element_soon("model-form-api-key");
                                                }
                                            }>{move || t(locale.get(), "models.add")}</button>
                                        }.into_view()
                                    }}
                                </div>
                            </div>
                            {move || if show_acp_agents.get() {
                                view! {
                                    <div class="acp-agents-pane" data-testid="acp-agents-settings">
                                        <p class="hint">{move || t(locale.get(), "models.acp_subpage_hint")}</p>
                                        {move || acp_form_msg.get().map(|(ok, text)| view! {
                                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                        })}
                                        <div class="settings-list" data-testid="acp-agents-list">
                                            <For each=move || acp_agents.get() key=|agent| agent.id.clone() let:agent>
                                                {
                                                    let edit = agent.clone();
                                                    let id_for_test = agent.id.clone();
                                                    let id_for_delete = agent.id.clone();
                                                    let label_for_delete = agent.label.clone();
                                                    let is_active = active_acp_agent_id.get().as_deref() == Some(agent.id.as_str());
                                                    view! {
                                                        <div class="settings-list-row settings-list-row-link"
                                                            data-testid="acp-agent-row"
                                                            class:settings-list-row-active=is_active
                                                            on:click=move |_| {
                                                                acp_form.set(Some(edit.clone()));
                                                                acp_form_msg.set(None);
                                                            }>
                                                            <div class="settings-list-main">
                                                                <span class="settings-list-title">
                                                                    {agent.label.clone()}
                                                                    {is_active.then(|| view! { <span class="settings-active-mark" title="active">" ✓"</span> })}
                                                                </span>
                                                                <span class="settings-list-sub">{agent.command.clone()}</span>
                                                            </div>
                                                            <div class="settings-list-actions">
                                                                {is_active.then(|| view! {
                                                                    <span class="settings-active-mark" title="active">"✓"</span>
                                                                })}
                                                                <button class="settings-list-use" type="button" data-testid="test-acp-agent"
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        let id = id_for_test.clone();
                                                                        spawn_local(async move {
                                                                            settings_busy.set(true);
                                                                            let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                                            match invoke_checked("test_acp_agent", args).await {
                                                                                Ok(value) => match serde_wasm_bindgen::from_value::<AcpAgentInfo>(value) {
                                                                                    Ok(info) => {
                                                                                        acp_infos.update(|infos| {
                                                                                            infos.insert(id, info);
                                                                                        });
                                                                                        acp_form_msg.set(None);
                                                                                    }
                                                                                    Err(error) => acp_form_msg.set(Some((false, error.to_string()))),
                                                                                },
                                                                                Err(error) => acp_form_msg.set(Some((false, js_error_text(error)))),
                                                                            }
                                                                            settings_busy.set(false);
                                                                        });
                                                                    }>{move || t(locale.get(), "models.acp_test")}</button>
                                                                <button class="settings-list-remove" type="button" title=move || t(locale.get(), "models.remove")
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        delete_confirm.set(Some(DeleteConfirm::Acp {
                                                                            id: id_for_delete.clone(),
                                                                            label: label_for_delete.clone(),
                                                                        }));
                                                                    }>{compose_icon("close")}</button>
                                                                <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                            </div>
                                                            {move || {
                                                                let id = agent.id.clone();
                                                                acp_infos.get().get(&id).cloned().map(|info| {
                                                                    // "Codex 1.1.2 · ACP v1": the agent's own version first, so the
                                                                    // protocol version is not mistaken for it (#200).
                                                                    let mut version_label = format!("ACP v{}", info.protocol_version);
                                                                    if let Some(implementation) = info.implementation.as_ref() {
                                                                        let name = implementation.get("title").and_then(serde_json::Value::as_str)
                                                                            .or_else(|| implementation.get("name").and_then(serde_json::Value::as_str));
                                                                        if let Some(name) = name {
                                                                            let version = implementation.get("version").and_then(serde_json::Value::as_str).unwrap_or("");
                                                                            version_label = if version.is_empty() {
                                                                                format!("{name} · {version_label}")
                                                                            } else {
                                                                                format!("{name} {version} · {version_label}")
                                                                            };
                                                                        }
                                                                    }
                                                                    let methods = info.auth_methods;
                                                                    view! {
                                                                        <div class="acp-agent-info" data-testid="acp-agent-info" on:click=|ev| ev.stop_propagation()>
                                                                            <span>{version_label}</span>
                                                                            {methods.into_iter().map(|method| {
                                                                                let id = id.clone();
                                                                                let method_id = method.id.clone();
                                                                                view! {
                                                                                    <button type="button" data-testid="authenticate-acp-agent" title=method.description.clone().unwrap_or_default()
                                                                                        disabled=move || settings_busy.get()
                                                                                        on:click=move |ev| {
                                                                                            ev.stop_propagation();
                                                                                            let id = id.clone();
                                                                                            let method_id = method_id.clone();
                                                                                            spawn_local(async move {
                                                                                                settings_busy.set(true);
                                                                                                let args = to_value(&serde_json::json!({ "id": id, "methodId": method_id })).unwrap();
                                                                                                match invoke_checked("authenticate_acp_agent", args).await {
                                                                                                    Ok(value) => match serde_wasm_bindgen::from_value::<Option<TerminalSessionSummary>>(value) {
                                                                                                        Ok(Some(session)) => {
                                                                                                            open_terminal_session.call(session);
                                                                                                            show_settings.set(false);
                                                                                                            show_toast(&t(locale.get_untracked(), "models.acp_auth_terminal_started"));
                                                                                                        }
                                                                                                        Ok(None) => acp_form_msg.set(Some((true, t(locale.get_untracked(), "models.acp_auth_ok").into()))),
                                                                                                        Err(error) => acp_form_msg.set(Some((false, error.to_string()))),
                                                                                                    },
                                                                                                    Err(error) => acp_form_msg.set(Some((false, js_error_text(error)))),
                                                                                                }
                                                                                                settings_busy.set(false);
                                                                                            });
                                                                                        }>{method.name.clone()}</button>
                                                                                }
                                                                            }).collect_view()}
                                                                        </div>
                                                                    }
                                                                })
                                                            }}
                                                        </div>
                                                    }
                                                }
                                            </For>
                                        </div>
                                        {move || acp_agents.get().is_empty().then(|| view! {
                                            <p class="model-empty-hint">{move || t(locale.get(), "models.empty_acp")}</p>
                                        })}
                                    </div>
                                }.into_view()
                            } else {
                                view! {
                                    <p class="hint" data-testid="acp-models-list-hint">{move || t(locale.get(), "models.acp_hint")}</p>
                                    <div class="model-preset-row" data-testid="model-presets">
                                        <span class="model-preset-label">{move || t(locale.get(), "models.quick_add")}</span>
                                        {MODEL_PRESETS.iter().map(|&(label, api_url, _model)| view! {
                                            <button type="button" class="model-preset-btn"
                                                on:click=move |_| {
                                                    show_acp_agents.set(false);
                                                    let mut form = ModelForm {
                                                        provider: "openai".into(),
                                                        max_tokens: 8192,
                                                        context_window: 128_000,
                                                        ..Default::default()
                                                    };
                                                    apply_base_url_suggestions(&mut form, api_url);
                                                    let reuse = endpoint_has_stored_key(&models.get(), &form.api_url);
                                                    model_form.set(Some(form));
                                                    model_form_key.set(String::new());
                                                    model_form_msg.set(None);
                                                    if !reuse {
                                                        focus_element_soon("model-form-api-key");
                                                    }
                                                }>{label}</button>
                                        }).collect_view()}
                                    </div>
                                    <div class="settings-list">
                                        <For each=move || models.get() key=|m| (m.id.clone(), m.active) let:m>
                                            {
                                                let pick_id = m.id.clone();
                                                let del_id = m.id.clone();
                                                let del_label = m.label.clone();
                                                let edit = m.clone();
                                                let is_active = m.active;
                                                let is_chat_model = m.is_chat_model();
                                                let can_delete = models.get().iter().any(|other| {
                                                    other.id != m.id && other.is_chat_model()
                                                });
                                                let show_sub = !m.model.is_empty() && m.model != m.label;
                                                let drag_id = m.id.clone();
                                                let drag_cls = m.id.clone();
                                                let enter_id = m.id.clone();
                                                let drop_id = m.id.clone();
                                                let over_cls = m.id.clone();
                                                view! {
                                                    <div class="settings-list-row settings-list-row-link"
                                                        class:settings-list-row-active=is_active
                                                        class:dragging=move || drag_model.get().as_deref() == Some(drag_cls.as_str())
                                                        class:model-drag-over=move || drop_model.get().as_deref() == Some(over_cls.as_str())
                                                        attr:draggable="true"
                                                        on:dragstart=move |ev: web_sys::DragEvent| {
                                                            start_session_drag(&ev, &drag_id);
                                                            drag_model.set(Some(drag_id.clone()));
                                                        }
                                                        on:dragend=move |_| {
                                                            drag_model.set(None);
                                                            drop_model.set(None);
                                                        }
                                                        on:dragover=move |ev: web_sys::DragEvent| allow_drop(&ev)
                                                        on:dragenter=move |ev: web_sys::DragEvent| {
                                                            allow_drop(&ev);
                                                            if drop_model.get().as_deref() != Some(enter_id.as_str()) {
                                                                drop_model.set(Some(enter_id.clone()));
                                                            }
                                                        }
                                                        on:drop=move |ev: web_sys::DragEvent| {
                                                            allow_drop(&ev);
                                                            let from = drag_session_id(&ev, drag_model.get());
                                                            drag_model.set(None);
                                                            drop_model.set(None);
                                                            let Some(from) = from.filter(|f| f != &drop_id) else { return };
                                                            let mut list = models.get_untracked();
                                                            let (Some(fi), Some(ti)) = (
                                                                list.iter().position(|x| x.id == from),
                                                                list.iter().position(|x| x.id == drop_id),
                                                            ) else { return };
                                                            let item = list.remove(fi);
                                                            // After removal the target shifts up by one when dragging
                                                            // downward; insert after it so the row lands where dropped.
                                                            let at = list.iter().position(|x| x.id == drop_id).unwrap()
                                                                + usize::from(fi < ti);
                                                            list.insert(at, item);
                                                            let ids: Vec<String> = list.iter().map(|x| x.id.clone()).collect();
                                                            models.set(list);
                                                            spawn_local(async move {
                                                                let arg = to_value(&serde_json::json!({ "ids": ids })).unwrap();
                                                                if let Ok(v) = invoke_checked("reorder_models", arg).await {
                                                                    if let Ok(l) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                        models.set(l);
                                                                    }
                                                                }
                                                            });
                                                        }
                                                        on:click=move |_| {
                                                            let form = profile_to_form(&edit);
                                                            show_acp_agents.set(false);
                                                            model_form.set(Some(form));
                                                            apply_catalog_limits(model_form, model_catalog_limits);
                                                            model_form_key.set(String::new());
                                                            model_form_msg.set(None);
                                                        }>
                                                        <span class="settings-list-grip" aria-hidden="true" title=move || t(locale.get(), "models.reorder")>"\u{283F}"</span>
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">
                                                                {m.label.clone()}
                                                                {m.use_for_vision.then(|| view! { <span class="settings-cap-badge" title="vision">"vision"</span> })}
                                                                {m.use_for_image_generation.then(|| view! {
                                                                    <span class="settings-cap-badge" title="image generation">"image gen"</span>
                                                                })}
                                                                {m.use_for_video_generation.then(|| view! {
                                                                    <span class="settings-cap-badge" title="video generation">"video gen"</span>
                                                                })}
                                                            </span>
                                                            {show_sub.then(|| view! {
                                                                <span class="settings-list-sub">{m.model.clone()}</span>
                                                            })}
                                                        </div>
                                                        <div class="settings-list-actions">
                                                            {is_active.then(|| view! {
                                                                <span class="settings-active-mark" title="active">"✓"</span>
                                                            })}
                                                            {(can_delete && !is_active).then(|| { let id = del_id.clone(); view! {
                                                                <button class="settings-list-remove" type="button" title=move || t(locale.get(), "models.remove")
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        delete_confirm.set(Some(DeleteConfirm::Model {
                                                                            id: id.clone(),
                                                                            label: del_label.clone(),
                                                                        }));
                                                                    }>{compose_icon("close")}</button>
                                                            }})}
                                                            {(!is_active && is_chat_model).then(|| { let id = pick_id.clone(); view! {
                                                                <button class="settings-list-use" type="button"
                                                                    on:click=move |ev| {
                                                                        ev.stop_propagation();
                                                                        let id = id.clone();
                                                                        spawn_local(async move {
                                                                            let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                                            if let Ok(v) = invoke_checked("set_active_model", arg).await {
                                                                                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(v) {
                                                                                    models.set(list);
                                                                                }
                                                                            }
                                                                        });
                                                                    }>{move || t(locale.get(), "models.use")}</button>
                                                            }})}
                                                            <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                        </div>
                                                    </div>
                                                }
                                            }
                                        </For>
                                    </div>
                                    {move || models.get().is_empty().then(|| view! {
                                        <p class="model-empty-hint">{move || t(locale.get(), "models.empty")}</p>
                                    })}
                                }.into_view()
                            }}
                            {move || settings_message.get().map(|(ok, text)| view! {
                                <div class="settings-status"
                                    class:ok=move || ok
                                    class:fail=move || !ok>{text}</div>
                            })}
                            <div class="row settings-footer">
                                <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                                <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                            </div>
                        </div>
                        }.into_view()
                    }
                })}
                {move || (settings_section.get() == "pet").then(|| view! {
                    <div class="settings-pane pet-settings-pane">
                        <div class="pet-settings-hero">
                            <div class="pet-settings-preview" class:empty=move || pet_status.get().asset.is_none()>
                                {move || pet_status.get().asset.map(|asset| {
                                    let style = format!("background-image:url('{}')", asset.spritesheet_data_url);
                                    view! { <span class="pet-settings-sprite" style=style aria-hidden="true"></span> }
                                })}
                            </div>
                            <div class="pet-settings-copy">
                                <h3>{move || pet_status.get().asset.map(|asset| asset.display_name).unwrap_or_else(|| t(locale.get(), "pet.not_configured").into())}</h3>
                                <p>{move || pet_status.get().asset.map(|asset| asset.description).filter(|text| !text.is_empty()).unwrap_or_else(|| t(locale.get(), "pet.description").into())}</p>
                                {move || pet_status.get().asset.map(|asset| view! {
                                    <div class="pet-settings-meta">
                                        <code>{asset.id}</code>
                                        <span>{format!("v{}", asset.sprite_version_number)}</span>
                                        <code title=pet_status.get().directory>{pet_status.get().directory}</code>
                                    </div>
                                })}
                            </div>
                        </div>
                        <div class="appearance-config-card pet-config-card">
                            <div class="appearance-config-row">
                                <div>
                                    <strong>{move || t(locale.get(), "pet.enabled")}</strong>
                                    <span>{move || t(locale.get(), "pet.enabled_hint")}</span>
                                </div>
                                <label class="toggle">
                                    <input type="checkbox" data-testid="pet-enabled"
                                        prop:checked=move || settings.get().pet_enabled
                                        on:change=move |ev| settings.update(|current| current.pet_enabled = event_target_checked(&ev)) />
                                    <span class="toggle-track" aria-hidden="true"></span>
                                </label>
                            </div>
                            <div class="pet-directory-row">
                                <label>{move || t(locale.get(), "pet.directory")}
                                    <div class="settings-path-row">
                                        <input class="settings-path-input" data-testid="pet-directory"
                                            prop:value=move || settings.get().pet_directory
                                            placeholder=move || t(locale.get(), "pet.directory_placeholder")
                                            on:input=move |ev| settings.update(|current| current.pet_directory = event_target_input(&ev).value()) />
                                        <button type="button" class="settings-add-btn" data-testid="pet-choose"
                                            on:click=choose_pet_directory>
                                            {move || t(locale.get(), "projects.choose_dir")}
                                        </button>
                                    </div>
                                    <span class="settings-field-hint">{move || t(locale.get(), "pet.directory_hint")}</span>
                                </label>
                            </div>
                        </div>
                        {move || pet_status.get().error.map(|error| view! {
                            <div class="settings-status fail">{error}</div>
                        })}
                        {move || settings_message.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                            <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "quick-actions").then(|| view! {
                    <div class="settings-pane settings-pane-list quick-actions-pane"
                        data-testid="quick-actions-settings">
                        <div class="quick-actions-hero">
                            <div>
                                <h3>{move || t(locale.get(), "quick_actions.title")}</h3>
                                <p>{move || t(locale.get(), "quick_actions.help")}</p>
                            </div>
                            <button type="button" class="settings-add-btn" data-testid="quick-action-new"
                                disabled=move || workflow_templates.get().is_empty()
                                on:click=move |_| {
                                    let workflow_template_id = workflow_templates
                                        .get_untracked()
                                        .first()
                                        .map(|template| template.id.clone())
                                        .unwrap_or_default();
                                    let sort_order = quick_actions
                                        .get_untracked()
                                        .iter()
                                        .map(|action| action.sort_order)
                                        .max()
                                        .unwrap_or(0)
                                        + 10;
                                    quick_action_form.set(Some(QuickAction {
                                        id: String::new(),
                                        name: String::new(),
                                        description: String::new(),
                                        icon: "sparkles".into(),
                                        context: "selection".into(),
                                        workflow_template_id,
                                        enabled: true,
                                        sort_order,
                                        builtin: false,
                                    }));
                                    quick_action_error.set(None);
                                }>
                                {move || format!("+ {}", t(locale.get(), "quick_actions.new"))}
                            </button>
                        </div>
                        {move || quick_action_form.get().map(|form| {
                            let builtin = form.builtin;
                            view! {
                                <div class="quick-action-form" data-testid="quick-action-form">
                                    <div class="settings-form-grid">
                                        <label>
                                            <span>{move || t(locale.get(), "quick_actions.name")}</span>
                                            <input type="text" data-testid="quick-action-name"
                                                prop:value=move || quick_action_form.get()
                                                    .map(|action| action.name)
                                                    .unwrap_or_default()
                                                on:input=move |event| quick_action_form.update(|action| {
                                                    if let Some(action) = action {
                                                        action.name = event_target_value(&event);
                                                    }
                                                }) />
                                        </label>
                                        <label>
                                            <span>{move || t(locale.get(), "quick_actions.workflow")}</span>
                                            <select data-testid="quick-action-workflow"
                                                disabled=builtin
                                                on:change=move |event| quick_action_form.update(|action| {
                                                    if let Some(action) = action {
                                                        action.workflow_template_id = dom_value(&event);
                                                    }
                                                })>
                                                <For each=move || workflow_templates.get()
                                                    key=|template| template.id.clone()
                                                    children=move |template| {
                                                        let id = template.id.clone();
                                                        view! {
                                                            <option value=template.id
                                                                prop:selected=move || quick_action_form.get()
                                                                    .is_some_and(|action| {
                                                                        action.workflow_template_id == id
                                                                    })>
                                                                {template.name}
                                                            </option>
                                                        }
                                                    }
                                                />
                                            </select>
                                        </label>
                                        <label class="span-2">
                                            <span>{move || t(locale.get(), "quick_actions.description")}</span>
                                            <input type="text" data-testid="quick-action-description"
                                                prop:value=move || quick_action_form.get()
                                                    .map(|action| action.description)
                                                    .unwrap_or_default()
                                                disabled=builtin
                                                on:input=move |event| quick_action_form.update(|action| {
                                                    if let Some(action) = action {
                                                        action.description = event_target_value(&event);
                                                    }
                                                }) />
                                        </label>
                                    </div>
                                    <label class="settings-check">
                                        <input type="checkbox"
                                            prop:checked=move || quick_action_form.get()
                                                .is_some_and(|action| action.enabled)
                                            on:change=move |event| quick_action_form.update(|action| {
                                                if let Some(action) = action {
                                                    action.enabled = event_target_checked(&event);
                                                }
                                            }) />
                                        <span>{move || t(locale.get(), "quick_actions.enabled")}</span>
                                    </label>
                                    <div class="row">
                                        <button type="button"
                                            on:click=move |_| quick_action_form.set(None)>
                                            {move || t(locale.get(), "settings.cancel")}
                                        </button>
                                        <button type="button" class="primary"
                                            data-testid="quick-action-save"
                                            disabled=move || {
                                                quick_action_busy.get()
                                                    || quick_action_form.get()
                                                        .is_none_or(|action| {
                                                            action.name.trim().is_empty()
                                                                || action.workflow_template_id.is_empty()
                                                        })
                                            }
                                            on:click=move |event| {
                                                save_quick_action_form.call(event);
                                            }>
                                            {move || t(locale.get(), "settings.save")}
                                        </button>
                                    </div>
                                </div>
                            }
                        })}
                        {move || quick_action_error.get().map(|error| view! {
                            <div class="settings-status fail" data-testid="quick-action-error">
                                {error}
                            </div>
                        })}
                        <div class="settings-list quick-action-list">
                            <For each=move || quick_actions.get() key=|action| action.id.clone()
                                children=move |action| {
                                    let workflow_name = workflow_templates
                                        .get_untracked()
                                        .into_iter()
                                        .find(|template| {
                                            template.id == action.workflow_template_id
                                        })
                                        .map(|template| template.name)
                                        .unwrap_or_else(|| action.workflow_template_id.clone());
                                    let workflow_id = action.workflow_template_id.clone();
                                    let edit_action = action.clone();
                                    let toggle_action = action.clone();
                                    let remove_id = action.id.clone();
                                    let is_builtin = action.builtin;
                                    view! {
                                        <div class="settings-list-row quick-action-row"
                                            data-testid="quick-action-row"
                                            data-action-id=action.id.clone()>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">
                                                    {compose_icon(&action.icon)}
                                                    <strong>{quick_action_label(locale.get(), &action)}</strong>
                                                    {is_builtin.then(|| view! {
                                                        <small>{move || t(locale.get(), "quick_actions.builtin")}</small>
                                                    })}
                                                </span>
                                                <span class="settings-list-sub">{action.description}</span>
                                                <code>{workflow_name}</code>
                                            </div>
                                            <div class="settings-list-actions">
                                                <label class="toggle" title=move || {
                                                    t(locale.get(), "quick_actions.enabled")
                                                }>
                                                    <input type="checkbox"
                                                        data-testid="quick-action-toggle"
                                                        prop:checked=action.enabled
                                                        disabled=move || quick_action_busy.get()
                                                        on:change=move |event| {
                                                            let mut next = toggle_action.clone();
                                                            next.enabled = event_target_checked(&event);
                                                            persist_quick_action.call(next);
                                                        } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                                <button type="button" class="settings-list-use"
                                                    data-testid="quick-action-open-workflow"
                                                    on:click=move |_| {
                                                        selected_workflow_template
                                                            .set(Some(workflow_id.clone()));
                                                        go_settings_section.call("workflows".into());
                                                    }>
                                                    {move || t(locale.get(), "quick_actions.open_workflow")}
                                                </button>
                                                <button type="button" class="settings-list-edit"
                                                    data-testid="quick-action-edit"
                                                    title=move || t(locale.get(), "quick_actions.edit")
                                                    on:click=move |_| {
                                                        quick_action_form.set(Some(edit_action.clone()));
                                                        quick_action_error.set(None);
                                                    }>
                                                    {compose_icon("edit")}
                                                </button>
                                                {(!is_builtin).then(|| view! {
                                                    <button type="button" class="settings-list-remove"
                                                        data-testid="quick-action-delete"
                                                        title=move || t(locale.get(), "quick_actions.delete")
                                                        on:click=move |_| {
                                                            remove_quick_action.call(remove_id.clone());
                                                        }>
                                                        {compose_icon("close")}
                                                    </button>
                                                })}
                                            </div>
                                        </div>
                                    }
                                }
                            />
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "workflows").then(|| view! {
                    <div class="settings-pane workflow-settings-pane">
                        {workflow_studio_view(
                            workflow_studio,
                            workflow_templates,
                            selected_workflow_template,
                            specialists,
                            models,
                            locale,
                            Callback::new(move |_: ()| {
                                go_settings_section.call("quick-actions".into());
                            }),
                        )}
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "specialists").then(|| {
                    if specialist_form_open.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                <div class="conn-form model-form">
                                    <div class="settings-form-grid">
                                        <label class="span-2">{move || t(locale.get(), "specialists.name")}
                                            <input prop:value=move || specialist_form.get().map(|f| f.name.clone()).unwrap_or_default()
                                                on:input=move |ev| specialist_form.update(|o| if let Some(o)=o { o.name = event_target_value(&ev); }) /></label>
                                        <label class="span-2">{move || t(locale.get(), "specialists.description")}
                                            <textarea prop:value=move || specialist_form.get().map(|f| f.description.clone()).unwrap_or_default()
                                                on:input=move |ev| specialist_form.update(|o| if let Some(o)=o { o.description = event_target_value(&ev); })></textarea></label>
                                        <label class="span-2">{move || t(locale.get(), "specialists.instructions")}
                                            <textarea rows="6"
                                                prop:disabled=move || specialist_form.get().map(|f| f.builtin).unwrap_or(false)
                                                prop:value=move || specialist_form.get().map(|f| f.instructions.clone()).unwrap_or_default()
                                                on:input=move |ev| specialist_form.update(|o| if let Some(o)=o { o.instructions = event_target_value(&ev); })></textarea></label>
                                        {move || specialist_form.get().filter(|f| f.builtin).map(|_| view! {
                                            <span class="hint span-2">{move || t(locale.get(), "specialists.builtin_locked")}</span>
                                        })}
                                        {move || specialist_form.get().filter(|f| !f.builtin).map(|_| view! {
                                            <span class="hint span-2">{move || t(locale.get(), "specialists.instructions.hint")}</span>
                                        })}
                                        <label class="span-2">{move || t(locale.get(), "specialists.model")}
                                            <select
                                                data-testid="reviewer-backend-select"
                                                on:change=move |ev| specialist_form.update(|o| if let Some(o)=o {
                                                    let value = dom_value(&ev);
                                                    if o.id == "reviewer" {
                                                        set_reviewer_backend(o, &value);
                                                    } else {
                                                        o.model_id = value;
                                                    }
                                                })>
                                                {move || if specialist_form.get().is_some_and(|f| f.id == "reviewer") {
                                                    view! {
                                                        <option value="http:"
                                                            prop:selected=move || specialist_form.get().is_some_and(|f| reviewer_backend_key(&f) == "http:")>
                                                            {t(locale.get(), "composer.reviewer.default_http")}
                                                        </option>
                                                        <option value="follow_session"
                                                            prop:selected=move || specialist_form.get().is_some_and(|f| reviewer_backend_key(&f) == "follow_session")>
                                                            {t(locale.get(), "composer.reviewer.follow_session")}
                                                        </option>
                                                    }.into_view()
                                                } else {
                                                    view! {
                                                        <option value=""
                                                            prop:selected=move || specialist_form.get().is_some_and(|f| f.model_id.is_empty())>
                                                            {t(locale.get(), "specialists.model.follow")}
                                                        </option>
                                                    }.into_view()
                                                }}
                                                {move || specialist_form.get()
                                                    .filter(|f| f.id == "reviewer")
                                                    .and_then(|reviewer| reviewer_missing_acp_profile_id(
                                                        &reviewer,
                                                        &acp_agents.get(),
                                                    ))
                                                    .map(|profile_id| {
                                                        let value = format!("acp:{profile_id}");
                                                        let label = format!(
                                                            "{} · {profile_id}",
                                                            t(locale.get(), "composer.reviewer.missing_acp"),
                                                        );
                                                        view! {
                                                            <option value=value prop:selected=true disabled=true
                                                                data-testid="reviewer-missing-acp-option">
                                                                {label}
                                                            </option>
                                                        }
                                                    })}
                                                {move || models.get().into_iter().filter(ModelProfile::is_chat_model).map(|m| {
                                                    let value = if specialist_form.get().is_some_and(|f| f.id == "reviewer") {
                                                        format!("http:{}", m.id)
                                                    } else {
                                                        m.id.clone()
                                                    };
                                                    let selected_value = value.clone();
                                                    view! {
                                                        <option value=value prop:selected=move || specialist_form.get().is_some_and(|f| {
                                                            if f.id == "reviewer" {
                                                                reviewer_backend_key(&f) == selected_value
                                                            } else {
                                                                f.model_id == selected_value
                                                            }
                                                        })>{m.label.clone()}</option>
                                                    }
                                                }).collect_view()}
                                                {move || specialist_form.get().is_some_and(|f| f.id == "reviewer").then(|| view! {
                                                    <optgroup label="ACP Agents">
                                                        {acp_agents.get().into_iter().map(|agent| {
                                                            let value = format!("acp:{}", agent.id);
                                                            let selected_value = value.clone();
                                                            view! {
                                                                <option value=value prop:selected=move || specialist_form.get().is_some_and(|f| {
                                                                    reviewer_backend_key(&f) == selected_value
                                                                })>{format!("{} · ACP", agent.label)}</option>
                                                            }
                                                        }).collect_view()}
                                                    </optgroup>
                                                })}
                                            </select>
                                        </label>
                                        {move || specialist_form.get().filter(|f| f.id == "reviewer").map(|reviewer| {
                                            let backend = reviewer_backend_label(
                                                &reviewer,
                                                &models.get(),
                                                &acp_agents.get(),
                                                &t(locale.get(), "composer.reviewer.follow_session"),
                                                &t(locale.get(), "composer.reviewer.missing_acp"),
                                            ).unwrap_or_else(|| t(locale.get(), "composer.reviewer.default_http"));
                                            view! {
                                                <span class="hint span-2" data-testid="reviewer-selected-backend">
                                                    {tf(locale.get(), "specialists.reviewer.selected_backend", &[("backend", &backend)])}
                                                </span>
                                                <span class="hint span-2">{move || t(locale.get(), "specialists.reviewer.test_hint")}</span>
                                            }
                                        })}
                                        <div class="span-2 settings-form-grid">
                                            <span class="span-2">{move || t(locale.get(), "specialists.skills")}</span>
                                            <label class="settings-check">
                                                <input type="checkbox"
                                                    prop:checked=move || specialist_form.get().map(|f| f.skills.is_none()).unwrap_or(true)
                                                    on:change=move |ev| specialist_form.update(|o| if let Some(o)=o {
                                                        o.skills = if event_target_checked(&ev) { None } else { Some(vec![]) };
                                                    }) />
                                                <span>{move || t(locale.get(), "specialists.inherit")}</span>
                                            </label>
                                            {move || specialist_form.get().filter(|f| f.skills.is_some()).map(|_| view! {
                                                <span class="hint span-2">{move || t(locale.get(), "specialists.skills.whitelist_hint")}</span>
                                            })}
                                            {move || specialist_form.get().is_some_and(|f| f.skills.is_some()).then(|| view! {
                                                <div class="span-2 dynamic-skill-picker specialist-skill-picker" data-testid="specialist-skill-picker">
                                                    {move || {
                                                        let selected = specialist_form.get()
                                                            .and_then(|f| f.skills)
                                                            .unwrap_or_default();
                                                        (!selected.is_empty()).then(|| view! {
                                                            <div class="dynamic-skill-selected" data-testid="specialist-selected-skills">
                                                                <For each=move || specialist_form.get()
                                                                        .and_then(|f| f.skills)
                                                                        .unwrap_or_default()
                                                                    key=|name| name.clone()
                                                                    children=move |name| {
                                                                        let remove_name = name.clone();
                                                                        view! {
                                                                            <button type="button" data-testid="specialist-selected-skill"
                                                                                aria-label=tf(
                                                                                    locale.get(),
                                                                                    "specialists.skills.remove",
                                                                                    &[("skill", &remove_name)],
                                                                                )
                                                                                on:click=move |_| specialist_form.update(|o| if let Some(o) = o {
                                                                                    if let Some(cur) = o.skills.as_mut() {
                                                                                        cur.retain(|n| n != &remove_name);
                                                                                    }
                                                                                })>
                                                                                <span>{name}</span>
                                                                                {compose_icon("close")}
                                                                            </button>
                                                                        }
                                                                    }
                                                                />
                                                            </div>
                                                        })
                                                    }}
                                                    <input type="search" class="dynamic-skill-search"
                                                        data-testid="specialist-skill-search"
                                                        autocomplete="off"
                                                        prop:value=move || specialist_skill_query.get()
                                                        prop:placeholder=move || t(locale.get(), "specialists.skills.search")
                                                        aria-label=move || t(locale.get(), "specialists.skills.search")
                                                        on:input=move |ev| specialist_skill_query.set(event_target_value(&ev)) />
                                                    <div class="dynamic-skill-results" data-testid="specialist-skill-results">
                                                        {move || {
                                                            if specialist_skill_query.get().trim().is_empty() {
                                                                view! {
                                                                    <span class="dynamic-skill-hint">{tf(
                                                                        locale.get(),
                                                                        "specialists.skills.search_hint",
                                                                        &[("count", &skills_list.get().len().to_string())],
                                                                    )}</span>
                                                                }.into_view()
                                                            } else if specialist_filtered_skills.get().is_empty() {
                                                                view! {
                                                                    <span class="dynamic-skill-hint">
                                                                        {t(locale.get(), "specialists.skills.no_results")}
                                                                    </span>
                                                                }.into_view()
                                                            } else {
                                                                ().into_view()
                                                            }
                                                        }}
                                                        <For each=move || specialist_filtered_skills.get()
                                                            key=|s| s.name.clone()
                                                            children=move |s| {
                                                                let name = s.name.clone();
                                                                let name_checked = name.clone();
                                                                view! {
                                                                    <label class="dynamic-skill-option" title=s.description.clone()
                                                                        data-testid="specialist-skill-option">
                                                                        <input type="checkbox"
                                                                            prop:checked=move || specialist_form.get()
                                                                                .and_then(|f| f.skills.clone())
                                                                                .unwrap_or_default()
                                                                                .contains(&name_checked)
                                                                            on:change=move |ev| {
                                                                                let on = event_target_checked(&ev);
                                                                                let name = name.clone();
                                                                                specialist_form.update(|o| if let Some(o) = o {
                                                                                    let mut cur = o.skills.clone().unwrap_or_default();
                                                                                    if on {
                                                                                        if !cur.contains(&name) { cur.push(name); }
                                                                                    } else {
                                                                                        cur.retain(|n| n != &name);
                                                                                    }
                                                                                    o.skills = Some(cur);
                                                                                });
                                                                            } />
                                                                        <span>{s.name}</span>
                                                                        <small>{s.scope}</small>
                                                                    </label>
                                                                }
                                                            }
                                                        />
                                                    </div>
                                                </div>
                                            })}
                                        </div>
                                    </div>
                                    {move || model_form_msg.get().map(|(ok, text)| view! {
                                        <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                    })}
                                    <div class="row settings-footer">
                                        {move || specialist_form.get().is_some_and(|f| f.id == "reviewer").then(|| view! {
                                            <button type="button" data-testid="test-reviewer-backend"
                                                disabled=move || settings_busy.get()
                                                on:click=move |ev| test_reviewer_form.call(ev)>
                                                {move || t(locale.get(), "specialists.reviewer.test")}
                                            </button>
                                        })}
                                        <button type="button" disabled=move || settings_busy.get() on:click=move |_| close_settings_subpage.call(())>{move || t(locale.get(), "settings.cancel")}</button>
                                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_specialist_form.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    } else {
                        view! {
                        <div class="settings-pane settings-pane-list">
                            <div class="settings-toolbar settings-toolbar-end">
                                <span class="settings-filter">{move || {
                                    let n = specialists.get().len();
                                    format!("{} ({n})", t(locale.get(), "settings.nav.specialists"))
                                }}</span>
                                <details class="settings-add-menu">
                                    <summary>{move || t(locale.get(), "specialists.add")}</summary>
                                    <button type="button" on:click=move |ev| {
                                        close_details_ancestor(&ev);
                                        model_form_msg.set(None);
                                        specialist_skill_query.set(String::new());
                                        specialist_form.set(Some(Specialist {
                                            id: String::new(),
                                            name: String::new(),
                                            icon: "review".into(),
                                            color: "clay".into(),
                                            description: String::new(),
                                            instructions: String::new(),
                                            model_id: String::new(),
                                            review_backend: None,
                                            skills: None,
                                            connectors: None,
                                            builtin: false,
                                        }));
                                    }>{move || t(locale.get(), "specialists.add.scratch")}</button>
                                        <button type="button" on:click=move |ev| start_specialist_chat.call(ev)>
                                        {move || t(locale.get(), "specialists.add.chat")}
                                    </button>
                                </details>
                            </div>
                            <div class="conn-group-label">{move || t(locale.get(), "specialists.builtin")}</div>
                            <div class="settings-list">
                                <For each=move || { specialists.get().into_iter().filter(|s| s.builtin).collect::<Vec<_>>() } key=|s| s.id.clone() let:s>
                                    {
                                        let edit = s.clone();
                                        view! {
                                            <div class="settings-list-row settings-list-row-link"
                                                on:click=move |_| {
                                                    model_form_msg.set(None);
                                                    specialist_skill_query.set(String::new());
                                                    specialist_form.set(Some(edit.clone()));
                                                }>
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title">{s.name.clone()}</span>
                                                    {(!s.description.is_empty()).then(|| view! {
                                                        <span class="settings-list-sub">{s.description.clone()}</span>
                                                    })}
                                                </div>
                                                <div class="settings-list-actions">
                                                    <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                </For>
                            </div>
                            <div class="conn-group-label">{move || t(locale.get(), "specialists.custom")}</div>
                            <div class="settings-list">
                                <For each=move || { specialists.get().into_iter().filter(|s| !s.builtin).collect::<Vec<_>>() } key=|s| s.id.clone() let:s>
                                    {
                                        let edit = s.clone();
                                        let del_id = s.id.clone();
                                        view! {
                                            <div class="settings-list-row settings-list-row-link"
                                                on:click=move |_| {
                                                    model_form_msg.set(None);
                                                    specialist_skill_query.set(String::new());
                                                    specialist_form.set(Some(edit.clone()));
                                                }>
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title">{s.name.clone()}</span>
                                                    {(!s.description.is_empty()).then(|| view! {
                                                        <span class="settings-list-sub">{s.description.clone()}</span>
                                                    })}
                                                </div>
                                                <div class="settings-list-actions">
                                                    {(!s.builtin).then(|| { let id = del_id.clone(); view! {
                                                        <button class="settings-list-remove" type="button" title=move || t(locale.get(), "specialists.remove")
                                                            on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                remove_specialist.call(id.clone());
                                                            }>{compose_icon("close")}</button>
                                                    }})}
                                                    <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                </For>
                            </div>
                        </div>
                        }.into_view()
                    }
                })}
                {move || (settings_section.get() == "memory").then(|| {
                    if memory_selected.get().is_some() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                {move || memory_selected.get().map(|name| {
                                    let name_del = name.clone();
                                    let name_save = name.clone();
                                    view! {
                                        <div class="memory-editor-inner memory-editor-page">
                                            <textarea class="memory-editor-text" prop:value=move || memory_editor.get()
                                                on:input=move |ev| memory_editor.set(event_target_value(&ev))></textarea>
                                            {move || memory_msg.get().map(|(ok, text)| view! {
                                                <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                                            })}
                                            <div class="row settings-footer">
                                                <button type="button" class="memory-delete-btn"
                                                    on:click=move |_| {
                                                        let n = name_del.clone();
                                                        let project_id = memory_view
                                                            .get_untracked()
                                                            .map(|view| view.project_id)
                                                            .unwrap_or_default();
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({
                                                                "name": n,
                                                                "projectId": project_id,
                                                            }))
                                                            .unwrap();
                                                            if let Ok(files) = invoke_checked("delete_memory_file", arg).await {
                                                                if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(files) {
                                                                    memory_view.update(|o| if let Some(o)=o { o.files = list; });
                                                                    close_settings_subpage.call(());
                                                                }
                                                            }
                                                        });
                                                    }>{move || t(locale.get(), "memory.delete")}</button>
                                                <button type="button" class="primary" on:click=move |_| {
                                                    let n = name_save.clone();
                                                    let content = memory_editor.get();
                                                    let project_id = memory_view
                                                        .get_untracked()
                                                        .map(|view| view.project_id)
                                                        .unwrap_or_default();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({
                                                            "name": n,
                                                            "content": content,
                                                            "projectId": project_id,
                                                        }))
                                                        .unwrap();
                                                        match invoke_checked("write_memory_file", arg).await {
                                                            Ok(v) => {
                                                                if let Ok(files) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(v) {
                                                                    memory_view.update(|o| if let Some(o)=o { o.files = files; });
                                                                }
                                                                memory_msg.set(Some((true, t(locale.get(), "memory.save").into())));
                                                            }
                                                            Err(e) => memory_msg.set(Some((false, js_error_text(e)))),
                                                        }
                                                    });
                                                }>{move || t(locale.get(), "memory.save")}</button>
                                            </div>
                                        </div>
                                    }
                                })}
                            </div>
                        }.into_view()
                    } else {
                        view! {
                        <div class="settings-pane settings-pane-list">
                            <div class="settings-toolbar settings-toolbar-end">
                                <div class="memory-project" data-testid="memory-project">
                                    <div class="memory-project-picker">
                                        <button
                                            type="button"
                                            class="settings-filter memory-project-trigger"
                                            data-testid="memory-project-select"
                                            aria-haspopup="listbox"
                                            aria-expanded=move || memory_project_menu_open.get().to_string()
                                            aria-label=move || t(locale.get(), "memory.choose_project")
                                            attr:data-project-id=move || {
                                                memory_view
                                                    .get()
                                                    .map(|view| view.project_id)
                                                    .unwrap_or_default()
                                            }
                                            class:active=move || memory_project_menu_open.get()
                                            on:click=move |_| {
                                                memory_project_menu_open.update(|open| *open = !*open);
                                            }>
                                            <span class="memory-project-name">{move || {
                                                let view = memory_view.get();
                                                let id = view.as_ref().map(|v| v.project_id.as_str()).unwrap_or("");
                                                let name = memory_projects
                                                    .get()
                                                    .into_iter()
                                                    .find(|p| p.id == id)
                                                    .map(|p| {
                                                        if p.name.trim().is_empty() {
                                                            p.id
                                                        } else {
                                                            p.name
                                                        }
                                                    })
                                                    .or_else(|| {
                                                        view.as_ref().map(|v| v.project_name.clone())
                                                            .filter(|name| !name.trim().is_empty())
                                                    })
                                                    .unwrap_or_else(|| {
                                                        t(locale.get(), "settings.nav.memory")
                                                    });
                                                let n = view.map(|v| v.files.len()).unwrap_or(0);
                                                format!("{name} ({n})")
                                            }}</span>
                                            <span class="caret" aria-hidden="true">"▾"</span>
                                        </button>
                                        {move || memory_project_menu_open.get().then(|| view! {
                                            <div class="memory-project-backdrop"
                                                data-testid="memory-project-backdrop"
                                                on:mousedown=move |ev: web_sys::MouseEvent| {
                                                    ev.prevent_default();
                                                    memory_project_menu_open.set(false);
                                                }></div>
                                            <div class="memory-project-menu" role="listbox"
                                                data-testid="memory-project-menu"
                                                aria-label=move || t(locale.get(), "memory.choose_project")
                                                on:mousedown=|ev: web_sys::MouseEvent| ev.stop_propagation()
                                                on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()>
                                                <div class="memory-project-menu-list">
                                                    <For
                                                        each=move || memory_projects.get()
                                                        key=|p| p.id.clone()
                                                        let:project>
                                                        {
                                                            let id = project.id.clone();
                                                            let id_aria = id.clone();
                                                            let id_active = id.clone();
                                                            let id_check = id.clone();
                                                            let id_pick = id.clone();
                                                            let name = if project.name.trim().is_empty() {
                                                                project.id.clone()
                                                            } else {
                                                                project.name.clone()
                                                            };
                                                            let desc = project.description.clone();
                                                            view! {
                                                                <button type="button"
                                                                    class="memory-project-option"
                                                                    role="option"
                                                                    data-testid=format!("memory-project-option-{id}")
                                                                    aria-selected=move || {
                                                                        memory_view
                                                                            .get()
                                                                            .map(|view| view.project_id == id_aria)
                                                                            .unwrap_or(false)
                                                                            .to_string()
                                                                    }
                                                                    class:active=move || {
                                                                        memory_view
                                                                            .get()
                                                                            .map(|view| view.project_id == id_active)
                                                                            .unwrap_or(false)
                                                                    }
                                                                    on:mousedown=move |ev: web_sys::MouseEvent| {
                                                                        ev.prevent_default();
                                                                        ev.stop_propagation();
                                                                        load_memory_project.call(id_pick.clone());
                                                                    }>
                                                                    <span class="memory-project-option-text">
                                                                        <span class="memory-project-option-name">{name}</span>
                                                                        {(!desc.trim().is_empty()).then(|| view! {
                                                                            <span class="memory-project-option-desc">{desc.clone()}</span>
                                                                        })}
                                                                    </span>
                                                                    {move || {
                                                                        let selected = memory_view
                                                                            .get()
                                                                            .map(|view| view.project_id == id_check)
                                                                            .unwrap_or(false);
                                                                        selected.then(|| view! {
                                                                            <span class="memory-project-option-check" aria-hidden="true">"✓"</span>
                                                                        })
                                                                    }}
                                                                </button>
                                                            }
                                                        }
                                                    </For>
                                                </div>
                                            </div>
                                        })}
                                    </div>
                                </div>
                                <div class="settings-toolbar-actions memory-toolbar-actions">
                                    <span class="memory-toggle-label">{move || t(locale.get(), "memory.enabled_label")}</span>
                                    <label class="toggle" title=move || t(locale.get(), "settings.nav.memory")>
                                        <input type="checkbox" prop:checked=move || memory_view.get().map(|v| v.enabled).unwrap_or(true)
                                            on:change=move |ev| {
                                                let on = event_target_checked(&ev);
                                                let project_id = memory_view
                                                    .get_untracked()
                                                    .map(|view| view.project_id)
                                                    .unwrap_or_default();
                                                spawn_local(async move {
                                                    let arg = to_value(&serde_json::json!({
                                                        "enabled": on,
                                                        "projectId": project_id,
                                                    }))
                                                    .unwrap();
                                                    if let Ok(v) = invoke_checked("set_memory_enabled", arg).await {
                                                        if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                                                            memory_view.set(Some(view));
                                                        }
                                                    }
                                                });
                                            } />
                                        <span class="toggle-track" aria-hidden="true"></span>
                                    </label>
                                    <button type="button" class="memory-clear-btn" on:click=move |_| {
                                        let project_id = memory_view
                                            .get_untracked()
                                            .map(|view| view.project_id)
                                            .unwrap_or_default();
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({
                                                "projectId": project_id,
                                            }))
                                            .unwrap();
                                            let v = invoke("clear_memory", arg).await;
                                            if let Ok(files) = serde_wasm_bindgen::from_value::<Vec<MemoryFile>>(v) {
                                                memory_view.update(|o| if let Some(o)=o { o.files = files; });
                                                reset_memory_browse();
                                            }
                                        });
                                    }>{move || t(locale.get(), "memory.clear_all")}</button>
                                    <button type="button" class="settings-add-btn" data-testid="memory-add-note"
                                        on:click=move |_| {
                                            if let Some(today) = memory_view.get().map(|v| v.today_file) {
                                                load_memory_file.call(today);
                                            }
                                        }>{move || t(locale.get(), "memory.add")}</button>
                                </div>
                            </div>
                            {move || memory_msg.get().map(|(ok, text)| view! {
                                <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                            })}
                            {move || {
                                let off = memory_view.get().map(|v| !v.enabled).unwrap_or(false);
                                off.then(|| view! {
                                <div class="memory-off-banner">
                                    <span>{move || t(locale.get(), "memory.off_banner")}</span>
                                    <button type="button" class="settings-add-btn" on:click=move |_| {
                                        let project_id = memory_view
                                            .get_untracked()
                                            .map(|view| view.project_id)
                                            .unwrap_or_default();
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({
                                                "enabled": true,
                                                "projectId": project_id,
                                            }))
                                            .unwrap();
                                            if let Ok(v) = invoke_checked("set_memory_enabled", arg).await {
                                                if let Ok(view) = serde_wasm_bindgen::from_value::<MemoryView>(v) {
                                                    memory_view.set(Some(view));
                                                }
                                            }
                                        });
                                    }>{move || t(locale.get(), "memory.turn_on")}</button>
                                </div>
                                })
                            }}
                            <div class="conn-group-label">{move || t(locale.get(), "memory.scope_hint")}</div>
                            <div class="settings-list" data-testid="memory-notes">
                                <For each=move || memory_view.get().map(|v| v.files).unwrap_or_default()
                                    key=|f| f.name.clone() let:f>
                                    {
                                        let pick = f.name.clone();
                                        view! {
                                            <div class="settings-list-row settings-list-row-link"
                                                on:click=move |_| load_memory_file.call(pick.clone())>
                                                <span class="memory-note-icon" aria-hidden="true">{compose_icon("doc")}</span>
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title">{f.name.clone()}</span>
                                                </div>
                                                <div class="settings-list-actions">
                                                    <span class="memory-note-size">{format_bytes(f.bytes)}</span>
                                                    <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                </For>
                                {move || {
                                    let empty = memory_view.get().map(|v| v.files.is_empty()).unwrap_or(true);
                                    empty.then(|| view! {
                                        <div class="memory-empty">
                                            <span class="memory-empty-icon" aria-hidden="true">{compose_icon("doc")}</span>
                                            <span>{move || t(locale.get(), "memory.empty")}</span>
                                        </div>
                                    })
                                }}
                            </div>
                            <div class="cred-group-heading memory-global-heading">
                                <span class="conn-group-label">{move || t(locale.get(), "memory.global_scope")}</span>
                                <button type="button" class="settings-add-btn memory-global-add-btn"
                                    data-testid="global-memory-add"
                                    disabled=move || global_memory_busy.get()
                                    on:click=move |_| {
                                        global_memory_edit_id.set(Some(String::new()));
                                        global_memory_editor.set(String::new());
                                        memory_msg.set(None);
                                    }>{move || t(locale.get(), "memory.global_add")}</button>
                            </div>
                            <p class="model-empty-hint memory-global-timing">
                                {move || t(locale.get(), "memory.global_timing_hint")}
                            </p>
                            {move || global_memory_edit_id.get().map(|id| {
                                let save_id = id.clone();
                                view! {
                                    <div class="global-memory-editor" data-testid="global-memory-editor">
                                        <textarea class="memory-editor-text"
                                            aria-label=move || t(locale.get(), "memory.proposal.content")
                                            prop:value=move || global_memory_editor.get()
                                            disabled=move || global_memory_busy.get()
                                            on:input=move |event| global_memory_editor.set(event_target_value(&event))></textarea>
                                        <div class="row">
                                            <button type="button"
                                                disabled=move || global_memory_busy.get()
                                                on:click=move |_| {
                                                    global_memory_edit_id.set(None);
                                                    global_memory_editor.set(String::new());
                                                }>{move || t(locale.get(), "settings.cancel")}</button>
                                            <button type="button" class="primary"
                                                disabled=move || global_memory_busy.get()
                                                    || global_memory_editor.get().trim().is_empty()
                                                on:click=move |_| {
                                                    let id = save_id.clone();
                                                    let content = global_memory_editor.get_untracked().trim().to_string();
                                                    if content.is_empty() {
                                                        memory_msg.set(Some((false, t(
                                                            locale.get_untracked(),
                                                            "memory.proposal.empty",
                                                        ).into())));
                                                        return;
                                                    }
                                                    global_memory_busy.set(true);
                                                    spawn_local(async move {
                                                        if id.is_empty() {
                                                            let arg = to_value(&serde_json::json!({
                                                                "content": content.clone(),
                                                            })).unwrap();
                                                            match invoke_checked("create_global_memory", arg).await {
                                                                Ok(value) => {
                                                                    match serde_wasm_bindgen::from_value::<GlobalMemory>(value) {
                                                                        Ok(memory) => {
                                                                            memory_view.update(|view| {
                                                                                if let Some(view) = view {
                                                                                    view.global_memories.insert(0, memory);
                                                                                }
                                                                            });
                                                                            global_memory_edit_id.set(None);
                                                                            global_memory_editor.set(String::new());
                                                                            memory_msg.set(Some((true, t(
                                                                                locale.get_untracked(),
                                                                                "memory.global_created",
                                                                            ).into())));
                                                                        }
                                                                        Err(error) => memory_msg.set(Some((false, error.to_string()))),
                                                                    }
                                                                }
                                                                Err(error) => memory_msg.set(Some((false, js_error_text(error)))),
                                                            }
                                                            global_memory_busy.set(false);
                                                            return;
                                                        }
                                                        let arg = to_value(&serde_json::json!({
                                                            "id": id.clone(),
                                                            "content": content.clone(),
                                                        })).unwrap();
                                                        match invoke_checked("update_global_memory", arg).await {
                                                            Ok(_) => {
                                                                memory_view.update(|view| {
                                                                    if let Some(view) = view {
                                                                        if let Some(memory) = view.global_memories
                                                                            .iter_mut()
                                                                            .find(|memory| memory.id == id)
                                                                        {
                                                                            memory.content = content;
                                                                        }
                                                                    }
                                                                });
                                                                global_memory_edit_id.set(None);
                                                                global_memory_editor.set(String::new());
                                                                memory_msg.set(Some((true, t(
                                                                    locale.get_untracked(),
                                                                    "memory.global_updated",
                                                                ).into())));
                                                            }
                                                            Err(error) => memory_msg.set(Some((false, js_error_text(error)))),
                                                        }
                                                        global_memory_busy.set(false);
                                                    });
                                                }>{move || t(locale.get(), "memory.global_save")}</button>
                                        </div>
                                    </div>
                                }
                            })}
                            <div class="settings-list" data-testid="global-memories">
                                <For each=move || memory_view.get().map(|v| v.global_memories).unwrap_or_default()
                                    key=|memory| (memory.id.clone(), memory.content.clone()) let:memory>
                                    {
                                        let id = memory.id.clone();
                                        let edit_id = memory.id.clone();
                                        let edit_content = memory.content.clone();
                                        view! {
                                            <div class="settings-list-row">
                                                <div class="settings-list-main">
                                                    <span class="settings-list-title global-memory-content">{memory.content}</span>
                                                </div>
                                                <div class="settings-list-actions">
                                                    <button type="button" class="settings-list-edit"
                                                        aria-label=move || t(locale.get(), "memory.global_edit")
                                                        title=move || t(locale.get(), "memory.global_edit")
                                                        on:click=move |_| {
                                                            global_memory_edit_id.set(Some(edit_id.clone()));
                                                            global_memory_editor.set(edit_content.clone());
                                                            memory_msg.set(None);
                                                        }>{compose_icon("edit")}</button>
                                                    <button type="button" class="settings-list-remove"
                                                        aria-label=move || t(locale.get(), "memory.global_delete")
                                                        title=move || t(locale.get(), "memory.global_delete")
                                                        on:click=move |_| {
                                                            let id = id.clone();
                                                            spawn_local(async move {
                                                                let arg = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                                match invoke_checked("delete_global_memory", arg).await {
                                                                    Ok(_) => {
                                                                        memory_view.update(|view| {
                                                                            if let Some(view) = view {
                                                                                view.global_memories.retain(|memory| memory.id != id);
                                                                            }
                                                                        });
                                                                        if global_memory_edit_id.get_untracked().as_deref() == Some(id.as_str()) {
                                                                            global_memory_edit_id.set(None);
                                                                            global_memory_editor.set(String::new());
                                                                        }
                                                                        memory_msg.set(Some((true, t(
                                                                            locale.get_untracked(),
                                                                            "memory.global_deleted",
                                                                        ).into())));
                                                                    }
                                                                    Err(error) => memory_msg.set(Some((false, js_error_text(error)))),
                                                                }
                                                            });
                                                        }>{compose_icon("close")}</button>
                                                </div>
                                            </div>
                                        }
                                    }
                                </For>
                                {move || {
                                    let empty = memory_view
                                        .get()
                                        .map(|view| view.global_memories.is_empty())
                                        .unwrap_or(true);
                                    empty.then(|| view! {
                                        <div class="memory-empty">
                                            <span class="memory-empty-icon" aria-hidden="true">{compose_icon("doc")}</span>
                                            <span>{move || t(locale.get(), "memory.global_empty")}</span>
                                        </div>
                                    })
                                }}
                            </div>
                        </div>
                        }.into_view()
                    }
                })}
                {move || (settings_section.get() == "plugins").then(|| view! {
                    <div class="settings-pane settings-pane-list">
                        {move || plugin_install_open.get().then(|| view! {
                            <div class="overlay" on:click=move |_| plugin_install_open.set(false)>
                                <section class="modal plugin-install-dialog" role="dialog"
                                    aria-modal="true" aria-labelledby="plugin-install-title"
                                    data-testid="plugin-settings" on:click=|event| event.stop_propagation()>
                                    <div class="plugin-install-dialog-head">
                                        <div>
                                            <h3 id="plugin-install-title">{move || t(locale.get(), "plugins.install_title")}</h3>
                                            <p class="hint">{move || t(locale.get(), "plugins.install_safety")}</p>
                                        </div>
                                        <button type="button" class="ps-close"
                                            title=move || t(locale.get(), "plugins.install_close")
                                            aria-label=move || t(locale.get(), "plugins.install_close")
                                            on:click=move |_| plugin_install_open.set(false)>
                                            {compose_icon("close")}
                                        </button>
                                    </div>
                                    <div class="plugin-install-modes" role="tablist">
                                        <button type="button" role="tab"
                                            aria-selected=move || (plugin_install_mode.get() == "local").to_string()
                                            class:active=move || plugin_install_mode.get() == "local"
                                            on:click=move |_| plugin_install_mode.set("local".into())>
                                            {move || t(locale.get(), "plugins.source_local")}
                                        </button>
                                        <button type="button" role="tab"
                                            aria-selected=move || (plugin_install_mode.get() == "remote").to_string()
                                            class:active=move || plugin_install_mode.get() == "remote"
                                            on:click=move |_| plugin_install_mode.set("remote".into())>
                                            {move || t(locale.get(), "plugins.source_remote")}
                                        </button>
                                    </div>
                                    {move || if plugin_install_mode.get() == "local" {
                                        view! {
                                            <div class="plugin-install-fields">
                                                <div class="settings-field-wide">
                                                    <span>{move || t(locale.get(), "plugins.zip_file")}</span>
                                                    <div class="plugin-local-source">
                                                        <input type="text" readonly
                                                            aria-label=move || t(locale.get(), "plugins.zip_file")
                                                            placeholder=move || t(locale.get(), "plugins.no_zip_selected")
                                                            prop:value=move || plugin_source.get() />
                                                        <button type="button" data-testid="choose-plugin-zip"
                                                            on:click=move |_| spawn_local(async move {
                                                                let picked = invoke("pick_plugin_source", JsValue::UNDEFINED).await;
                                                                if let Some(path) = picked.as_string() {
                                                                    plugin_source.set(path);
                                                                }
                                                            })>
                                                            {move || t(locale.get(), "plugins.choose_zip")}
                                                        </button>
                                                    </div>
                                                </div>
                                                <label class="settings-field-wide">
                                                    <span>{move || t(locale.get(), "plugins.sha256_optional")}</span>
                                                    <input type="text" autocomplete="off" spellcheck="false"
                                                        placeholder=move || t(locale.get(), "plugins.sha256_hint")
                                                        prop:value=move || plugin_checksum.get()
                                                        on:input=move |event| plugin_checksum.set(event_target_input(&event).value()) />
                                                </label>
                                                <button type="button" class="primary" data-testid="install-plugin"
                                                    disabled=move || {
                                                        let checksum = plugin_checksum.get();
                                                        plugin_source.get().is_empty()
                                                            || (!checksum.trim().is_empty() && !valid_sha256(&checksum))
                                                    }
                                                    on:click=move |_| {
                                                        let expected = plugin_checksum.get().trim().to_string();
                                                        install_plugin_from.call((
                                                            plugin_source.get(),
                                                            (!expected.is_empty()).then_some(expected),
                                                        ));
                                                    }>
                                                    {move || t(locale.get(), "plugins.install_action")}
                                                </button>
                                            </div>
                                        }.into_view()
                                    } else {
                                        view! {
                                            <div class="plugin-install-fields">
                                                <label class="settings-field-wide">
                                                    <span>{move || t(locale.get(), "plugins.url")}</span>
                                                    <input type="url" autocomplete="off" spellcheck="false"
                                                        placeholder="https://github.com/…/plugin.zip"
                                                        prop:value=move || plugin_url.get()
                                                        on:input=move |event| plugin_url.set(event_target_input(&event).value()) />
                                                </label>
                                                <label class="settings-field-wide">
                                                    <span>{move || t(locale.get(), "plugins.sha256_required")}</span>
                                                    <input type="text" autocomplete="off" spellcheck="false"
                                                        placeholder=move || t(locale.get(), "plugins.sha256_hint")
                                                        prop:value=move || plugin_checksum.get()
                                                        on:input=move |event| plugin_checksum.set(event_target_input(&event).value()) />
                                                </label>
                                                <button type="button" class="primary"
                                                    disabled=move || plugin_url.get().trim().is_empty() || !valid_sha256(&plugin_checksum.get())
                                                    on:click=move |_| install_plugin_url.call((
                                                        plugin_url.get().trim().to_string(),
                                                        plugin_checksum.get().trim().to_string(),
                                                    ))>
                                                    {move || t(locale.get(), "plugins.install_url")}
                                                </button>
                                            </div>
                                        }.into_view()
                                    }}
                                </section>
                            </div>
                        })}
                        <div class="settings-toolbar plugin-toolbar">
                            <span class="settings-filter">{move || {
                                let total = plugins_list.get().len();
                                let enabled = plugins_list.get().iter().filter(|plugin| plugin.enabled).count();
                                tf(locale.get(), "plugins.summary", &[
                                    ("enabled", &enabled.to_string()),
                                    ("total", &total.to_string()),
                                ])
                            }}</span>
                            <input class="settings-search" type="text" inputmode="search"
                                autocomplete="off" spellcheck="false"
                                placeholder=move || t(locale.get(), "plugins.search")
                                prop:value=move || plugin_search.get()
                                on:input=move |event| plugin_search.set(event_target_input(&event).value()) />
                            <button type="button" class="primary" on:click=move |_| {
                                plugin_checksum.set(String::new());
                                plugin_source.set(String::new());
                                plugin_url.set(String::new());
                                plugin_install_mode.set("local".into());
                                plugin_install_open.set(true);
                            }>
                                {move || t(locale.get(), "plugins.install_action")}
                            </button>
                        </div>
                        {move || plugins_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                        })}
                        <div class="settings-list plugin-list">
                            <For each=move || {
                                let query = plugin_search.get().trim().to_lowercase();
                                plugins_list.get().into_iter().filter(|plugin| {
                                    query.is_empty()
                                        || plugin.display_name.to_lowercase().contains(&query)
                                        || plugin.id.to_lowercase().contains(&query)
                                        || plugin.description.to_lowercase().contains(&query)
                                }).collect::<Vec<_>>()
                            }
                                key=|plugin| format!("{}:{}:{}", plugin.id, plugin.version, plugin.enabled)
                                let:plugin>
                                {
                                    let toggle_id = plugin.id.clone();
                                    let toggle_version = plugin.version.clone();
                                    let use_id = plugin.id.clone();
                                    let use_version = plugin.version.clone();
                                    let use_name = plugin.display_name.clone();
                                    let use_skills = plugin.skill_names.clone();
                                    let remove_id = plugin.id.clone();
                                    let remove_version = plugin.version.clone();
                                    let remove_name = plugin.display_name.clone();
                                    let command = plugin.commands.join(" · ");
                                    let skills = if plugin.skill_names.is_empty() {
                                        plugin.skill_count.to_string()
                                    } else {
                                        plugin.skill_names.join(", ")
                                    };
                                    let mcp = if command.is_empty() {
                                        plugin.mcp_server_count.to_string()
                                    } else {
                                        format!("{} · {}", plugin.mcp_server_count, command)
                                    };
                                    let runtime_errors = plugin.runtime_errors.join(" · ");
                                    let runtime_unavailable = plugin.runtime_status == "unavailable";
                                    let runtime_label_key = match plugin.runtime_status.as_str() {
                                        "ready" => "plugins.runtime_ready",
                                        "unavailable" => "plugins.runtime_unavailable",
                                        _ => "plugins.runtime_not_applicable",
                                    };
                                    let trust = plugin.trust_state.clone();
                                    let enabled = plugin.enabled;
                                    view! {
                                        <article class="settings-list-row plugin-row" data-plugin-id=plugin.id.clone()>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">
                                                    {plugin.display_name.clone()}
                                                    <span class="settings-list-version">{format!(" v{}", plugin.version)}</span>
                                                </span>
                                                {(!plugin.description.is_empty()).then(|| {
                                                    let description = plugin.description.clone();
                                                    view! { <span class="settings-list-sub">{description}</span> }
                                                })}
                                                <div class="plugin-state-line">
                                                    <span class="plugin-state" class:enabled=enabled>
                                                        {move || t(locale.get(), if enabled { "plugins.enabled_project" } else { "plugins.disabled_project" })}
                                                    </span>
                                                    <span class="plugin-runtime" class:fail=runtime_unavailable>
                                                        {move || t(locale.get(), runtime_label_key)}
                                                    </span>
                                                </div>
                                                <details class="skill-tags-editor plugin-details">
                                                    <summary><span>{move || t(locale.get(), "plugins.details")}</span></summary>
                                                    <dl class="plugin-detail-grid">
                                                        <dt>{move || t(locale.get(), "plugins.provides_skills")}</dt>
                                                        <dd>{skills}</dd>
                                                        <dt>{move || t(locale.get(), "plugins.mcp_servers")}</dt>
                                                        <dd>{mcp}</dd>
                                                        <dt>{move || t(locale.get(), "plugins.verify")}</dt>
                                                        <dd>{trust}</dd>
                                                        {(!runtime_errors.is_empty()).then(|| view! {
                                                            <dt>{move || t(locale.get(), "plugins.runtime")}</dt>
                                                            <dd class="fail">{runtime_errors}</dd>
                                                        })}
                                                    </dl>
                                                </details>
                                            </div>
                                            <div class="settings-list-actions plugin-actions">
                                                <button type="button" class="plugin-use-button"
                                                    disabled=runtime_unavailable
                                                    on:click=move |_| use_plugin.call((
                                                        use_id.clone(),
                                                        use_version.clone(),
                                                        use_name.clone(),
                                                        use_skills.clone(),
                                                        enabled,
                                                    ))>
                                                    {move || t(locale.get(), if enabled { "plugins.use_new_session" } else { "plugins.enable_and_use" })}
                                                </button>
                                                <button class="settings-list-remove" type="button"
                                                    title=move || t(locale.get(), "plugins.remove")
                                                    on:click=move |_| delete_confirm.set(Some(DeleteConfirm::Plugin {
                                                        id: remove_id.clone(),
                                                        version: remove_version.clone(),
                                                        label: remove_name.clone(),
                                                    }))>
                                                    {compose_icon("close")}
                                                </button>
                                                <label class="toggle" title=move || t(locale.get(), if enabled { "plugins.disable_project" } else { "plugins.enable_project" })>
                                                    <input type="checkbox" prop:checked=enabled
                                                        on:change=move |event| set_plugin_enabled.call((
                                                            toggle_id.clone(),
                                                            toggle_version.clone(),
                                                            event_target_checked(&event),
                                                        )) />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                            </div>
                                        </article>
                                    }
                                }
                            </For>
                        </div>
                        {move || {
                            let query = plugin_search.get().trim().to_lowercase();
                            let has_match = plugins_list.get().iter().any(|plugin| {
                                query.is_empty()
                                    || plugin.display_name.to_lowercase().contains(&query)
                                    || plugin.id.to_lowercase().contains(&query)
                                    || plugin.description.to_lowercase().contains(&query)
                            });
                            (!has_match).then(|| view! {
                                <p class="skill-filter-empty">{move || t(locale.get(), if plugins_list.get().is_empty() { "plugins.empty" } else { "plugins.no_match" })}</p>
                            })
                        }}
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "browser").then(|| view! {
                    <div class="settings-pane settings-pane-list browser-filter-pane" data-testid="browser-url-filters">
                        <div class="appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "browser.auto_launch")}</strong>
                                <span>{move || t(locale.get(), "browser.auto_launch_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="browser-auto-launch"
                                    prop:checked=move || browser_auto_launch.get()
                                    on:change=move |ev| save_browser_auto_launch.call(event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        <div class="appearance-config-row">
                            <div>
                                <strong>{move || t(locale.get(), "browser.auto_close_tabs")}</strong>
                                <span>{move || t(locale.get(), "browser.auto_close_tabs_hint")}</span>
                            </div>
                            <label class="toggle">
                                <input type="checkbox" data-testid="browser-auto-close-tabs"
                                    prop:checked=move || browser_auto_close_tabs.get()
                                    on:change=move |ev| save_browser_auto_close_tabs.call(event_target_checked(&ev)) />
                                <span class="toggle-track" aria-hidden="true"></span>
                            </label>
                        </div>
                        <p class="settings-note">{move || t(locale.get(), "browser.filters.hint")}</p>
                        {move || browser_filters_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                        })}
                        <div class="cred-group-heading">
                            <span class="conn-group-label">{move || t(locale.get(), "browser.filters.block")}</span>
                        </div>
                        <div class="browser-filter-add">
                            <input data-testid="browser-block-host"
                                prop:placeholder=move || t(locale.get(), "browser.filters.host_placeholder")
                                aria-label=move || t(locale.get(), "browser.filters.host")
                                prop:value=move || browser_block_host.get()
                                on:input=move |ev| browser_block_host.set(event_target_value(&ev)) />
                            <input data-testid="browser-block-reason"
                                prop:placeholder=move || t(locale.get(), "browser.filters.reason_block_placeholder")
                                aria-label=move || t(locale.get(), "browser.filters.reason")
                                prop:value=move || browser_block_reason.get()
                                on:input=move |ev| browser_block_reason.set(event_target_value(&ev)) />
                            <button type="button" class="settings-add-btn" data-testid="browser-block-add"
                                disabled=move || browser_filters_busy.get() || browser_block_host.get().trim().is_empty()
                                on:click=move |_| {
                                    let host = browser_block_host.get().trim().to_string();
                                    if host.is_empty() { return; }
                                    let reason = browser_block_reason.get().trim().to_string();
                                    let mut next = browser_filters.get();
                                    next.block.retain(|rule| !rule.host.eq_ignore_ascii_case(&host));
                                    next.block.push(BrowserUrlFilterRule { host, reason });
                                    browser_block_host.set(String::new());
                                    browser_block_reason.set(String::new());
                                    save_browser_filters.call(next);
                                }>{move || t(locale.get(), "browser.filters.add")}</button>
                        </div>
                        <div class="settings-list" data-testid="browser-block-list">
                            <For each=move || browser_filters.get().block
                                key=|rule| rule.host.clone()
                                let:rule>
                                {
                                    let host = rule.host.clone();
                                    let reason = rule.reason.clone();
                                    view! {
                                        <div class="settings-list-row">
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{host.clone()}</span>
                                                {(!reason.is_empty()).then(|| view! {
                                                    <span class="settings-list-sub">{reason.clone()}</span>
                                                })}
                                            </div>
                                            <div class="settings-list-actions">
                                                <button type="button" class="settings-list-remove"
                                                    data-testid="browser-block-remove"
                                                    title=move || t(locale.get(), "browser.filters.remove")
                                                    aria-label=move || t(locale.get(), "browser.filters.remove")
                                                    on:click=move |_| {
                                                        let mut next = browser_filters.get();
                                                        next.block.retain(|rule| rule.host != host);
                                                        save_browser_filters.call(next);
                                                    }>{compose_icon("close")}</button>
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                            {move || browser_filters.get().block.is_empty().then(|| view! {
                                <div class="settings-list-empty">{t(locale.get(), "browser.filters.block_empty")}</div>
                            })}
                        </div>
                        <div class="cred-group-heading">
                            <span class="conn-group-label">{move || t(locale.get(), "browser.filters.prefer")}</span>
                        </div>
                        <div class="browser-filter-add">
                            <input data-testid="browser-prefer-host"
                                prop:placeholder=move || t(locale.get(), "browser.filters.host_placeholder")
                                aria-label=move || t(locale.get(), "browser.filters.host")
                                prop:value=move || browser_prefer_host.get()
                                on:input=move |ev| browser_prefer_host.set(event_target_value(&ev)) />
                            <input data-testid="browser-prefer-reason"
                                prop:placeholder=move || t(locale.get(), "browser.filters.reason_prefer_placeholder")
                                aria-label=move || t(locale.get(), "browser.filters.reason")
                                prop:value=move || browser_prefer_reason.get()
                                on:input=move |ev| browser_prefer_reason.set(event_target_value(&ev)) />
                            <button type="button" class="settings-add-btn" data-testid="browser-prefer-add"
                                disabled=move || browser_filters_busy.get() || browser_prefer_host.get().trim().is_empty()
                                on:click=move |_| {
                                    let host = browser_prefer_host.get().trim().to_string();
                                    if host.is_empty() { return; }
                                    let reason = browser_prefer_reason.get().trim().to_string();
                                    let mut next = browser_filters.get();
                                    next.prefer.retain(|rule| !rule.host.eq_ignore_ascii_case(&host));
                                    next.prefer.push(BrowserUrlFilterRule { host, reason });
                                    browser_prefer_host.set(String::new());
                                    browser_prefer_reason.set(String::new());
                                    save_browser_filters.call(next);
                                }>{move || t(locale.get(), "browser.filters.add")}</button>
                        </div>
                        <div class="settings-list" data-testid="browser-prefer-list">
                            <For each=move || browser_filters.get().prefer
                                key=|rule| rule.host.clone()
                                let:rule>
                                {
                                    let host = rule.host.clone();
                                    let reason = rule.reason.clone();
                                    view! {
                                        <div class="settings-list-row">
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{host.clone()}</span>
                                                {(!reason.is_empty()).then(|| view! {
                                                    <span class="settings-list-sub">{reason.clone()}</span>
                                                })}
                                            </div>
                                            <div class="settings-list-actions">
                                                <button type="button" class="settings-list-remove"
                                                    data-testid="browser-prefer-remove"
                                                    title=move || t(locale.get(), "browser.filters.remove")
                                                    aria-label=move || t(locale.get(), "browser.filters.remove")
                                                    on:click=move |_| {
                                                        let mut next = browser_filters.get();
                                                        next.prefer.retain(|rule| rule.host != host);
                                                        save_browser_filters.call(next);
                                                    }>{compose_icon("close")}</button>
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                            {move || browser_filters.get().prefer.is_empty().then(|| view! {
                                <div class="settings-list-empty">{t(locale.get(), "browser.filters.prefer_empty")}</div>
                            })}
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "skills").then(|| view! {
                    <div class="settings-pane settings-pane-list">
                        <div class="settings-toolbar">
                            <span class="settings-filter">{move || {
                                let q = skills_search.get().trim().to_lowercase();
                                let tag = skill_filter_tag.get();
                                let skills = skills_list.get();
                                let visible = skills.iter().filter(|s| {
                                    skill_matches_filter(s, &tag, &q)
                                }).count();
                                let enabled = skills.iter().filter(|s| s.enabled).count();
                                tf(locale.get(), "skills.summary", &[
                                    ("visible", &visible.to_string()),
                                    ("enabled", &enabled.to_string()),
                                    ("total", &skills.len().to_string()),
                                ])
                            }}</span>
                            <input class="settings-search" type="text" inputmode="search"
                                autocomplete="off" autocorrect="off" autocapitalize="none" spellcheck="false"
                                placeholder=move || t(locale.get(), "skills.search_ph")
                                prop:value=move || skills_search.get()
                                on:input=move |ev| skills_search.set(event_target_input(&ev).value()) />
                            <button type="button" on:click=move |_| set_visible_skills_enabled.call(true)>
                                {move || t(locale.get(), "skills.enable_visible")}
                            </button>
                            <button type="button" on:click=move |_| set_visible_skills_enabled.call(false)>
                                {move || t(locale.get(), "skills.disable_visible")}
                            </button>
                            <button type="button" on:click=move |_| reload_skills.call(())>
                                {move || t(locale.get(), "skills.reload")}
                            </button>
                            <details class="settings-add-menu">
                                <summary>{move || t(locale.get(), "skills.add")}</summary>
                                <button type="button" on:click=move |_| {
                                    spawn_local(async move {
                                        let picked = invoke("pick_skill_source", JsValue::UNDEFINED).await;
                                        if let Some(path) = picked.as_string() {
                                            install_skill_from.call(path);
                                        }
                                    });
                                }>{move || t(locale.get(), "skills.add_file")}</button>
                                <button type="button" on:click=move |_| {
                                    spawn_local(async move {
                                        let picked = invoke("pick_directory", JsValue::UNDEFINED).await;
                                        if let Some(path) = picked.as_string() {
                                            install_skill_from.call(path);
                                        }
                                    });
                                }>{move || t(locale.get(), "skills.add_folder")}</button>
                            </details>
                        </div>
                        <div class="skill-tags-filter">
                            <button class:active=move || skill_filter_tag.get().is_empty()
                                on:click=move |_| skill_filter_tag.set(String::new())>
                                {move || t(locale.get(), "skills.all")}
                            </button>
                            <button class:active=move || skill_filter_tag.get() == "__untagged"
                                on:click=move |_| skill_filter_tag.set("__untagged".into())>
                                {move || t(locale.get(), "skills.untagged")}
                            </button>
                            <button class:active=move || skill_filter_tag.get() == "__enabled"
                                on:click=move |_| skill_filter_tag.set("__enabled".into())>
                                {move || t(locale.get(), "skills.enabled")}
                            </button>
                            <button class:active=move || skill_filter_tag.get() == "__disabled"
                                on:click=move |_| skill_filter_tag.set("__disabled".into())>
                                {move || t(locale.get(), "skills.disabled")}
                            </button>
                            {move || {
                                let tags = skills_list.get().iter()
                                    .flat_map(|s| s.tags.iter().cloned())
                                    .collect::<BTreeSet<_>>()
                                    .into_iter()
                                    .collect::<Vec<_>>();
                                tags.into_iter().map(|tag| {
                                    let active_tag = tag.clone();
                                    let set_tag = tag.clone();
                                    view! {
                                        <button class:active=move || skill_filter_tag.get() == active_tag
                                            on:click=move |_| skill_filter_tag.set(set_tag.clone())>
                                            {tag}
                                        </button>
                                    }
                                }).collect_view()
                            }}
                        </div>
                        <p class="settings-note">{move || t(locale.get(), "settings.auto_saved_new_session")}</p>
                        {move || skills_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=ok class:fail=move || !ok>{text}</div>
                        })}
                        {move || {
                            let q = skills_search.get().trim().to_lowercase();
                            let tag = skill_filter_tag.get();
                            (!skills_list.get().iter().any(|s| skill_matches_filter(s, &tag, &q))).then(|| view! {
                                <p class="skill-filter-empty">{move || t(locale.get(), "skills.empty")}</p>
                            })
                        }}
                        <div class="settings-list">
                            <For each=move || {
                                let q = skills_search.get().trim().to_lowercase();
                                let tag = skill_filter_tag.get();
                                skills_list.get().into_iter().filter(|s| {
                                    skill_matches_filter(s, &tag, &q)
                                }).collect::<Vec<_>>()
                            } key=|s| format!("{}:{}:{}", s.name, s.enabled, join_tags(&s.tags)) let:s>
                                {
                                    let name_toggle = s.name.clone();
                                    let name_remove = s.name.clone();
                                    let name_tags = s.name.clone();
                                    let enabled = s.enabled;
                                    let builtin = s.builtin;
                                    let managed = s.managed;
                                    let managed_by = s.managed_by.clone();
                                    let scope = s.scope.clone();
                                    let scope_label = t(locale.get(), &format!("skills.scope.{scope}"));
                                    let source_path = s.dir.clone();
                                    let tags_text = join_tags(&s.tags);
                                    let tags_input_text = tags_text.clone();
                                    let tags_cb = save_skill_tags.clone();
                                    view! {
                                        <div class="settings-list-row" data-skill-name=s.name.clone()>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">
                                                    {s.name.clone()}
                                                    <span class="skill-scope-badge" title=source_path>{scope_label}</span>
                                                </span>
                                                {(!s.description.is_empty() && s.description != ">").then(|| {
                                                    let desc = s.description.clone();
                                                    view! { <span class="settings-list-sub">{desc}</span> }
                                                })}
                                                {(!managed).then(|| view! {
                                                    <details class="skill-tags-editor">
                                                        <summary>
                                                            <span>{move || t(locale.get(), "skills.edit_tags")}</span>
                                                            <span class="skill-tags-summary">{tags_text}</span>
                                                        </summary>
                                                        <input class="skill-tags-input"
                                                            prop:value=tags_input_text
                                                            prop:placeholder=move || t(locale.get(), "skills.tags_placeholder")
                                                            on:change=move |ev| tags_cb.call((name_tags.clone(), event_target_value(&ev))) />
                                                    </details>
                                                })}
                                            </div>
                                            <div class="settings-list-actions">
                                                {(scope == "global" && !builtin).then(|| { let n = name_remove.clone(); view! {
                                                    <button class="settings-skill-remove" type="button"
                                                        title=move || t(locale.get(), "skills.remove")
                                                        on:click=move |_| delete_confirm.set(Some(DeleteConfirm::Skill {
                                                            name: n.clone(),
                                                            label: n.clone(),
                                                        }))>
                                                        {move || t(locale.get(), "skills.remove")}
                                                    </button>
                                                }})}
                                                {if managed {
                                                    let provider = managed_by.unwrap_or_else(|| t(locale.get(), "settings.nav.plugins").to_string());
                                                    view! {
                                                        <span class="skill-managed-badge">
                                                            {tf(locale.get(), "skills.managed_by", &[("plugin", &provider)])}
                                                        </span>
                                                    }.into_view()
                                                } else {
                                                    view! {
                                                        <label class="toggle">
                                                            <input type="checkbox" prop:checked=enabled on:change=move |ev| {
                                                                let n = name_toggle.clone();
                                                                let on = event_target_checked(&ev);
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "name": n, "enabled": on })).unwrap();
                                                                    let _ = invoke_checked("set_skill_enabled", arg).await;
                                                                    refresh_skills.call(());
                                                                });
                                                            } />
                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                        </label>
                                                    }.into_view()
                                                }}
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "credentials").then(|| view! {
                    <div class="settings-pane">
                        <p class="settings-note">{move || t(locale.get(), "cred.desc")}</p>
                        {CRED_GROUPS.iter().map(|g| {
                            let tooltip_id = format!("cred-help-{}", g.id);
                            let described_by = tooltip_id.clone();
                            view! {
                            <div class="cred-group-heading">
                                <div class="conn-group-label">{move || t(locale.get(), g.name_key)}</div>
                                <span class="cred-help">
                                    <button
                                        type="button"
                                        class="cred-help-trigger"
                                        aria-label=move || format!("{}: {}", t(locale.get(), g.name_key), t(locale.get(), "cred.help.aria"))
                                        aria-describedby=described_by
                                    >"?"</button>
                                    <span id=tooltip_id class="cred-help-tooltip" role="tooltip">
                                        <span class="cred-help-section">
                                            <strong>{move || t(locale.get(), "cred.help.what")}</strong>
                                            <span>{move || t(locale.get(), g.about_key)}</span>
                                        </span>
                                        <span class="cred-help-section">
                                            <strong>{move || t(locale.get(), "cred.help.configured")}</strong>
                                            <span>{move || t(locale.get(), g.configured_key)}</span>
                                        </span>
                                        <span class="cred-help-section">
                                            <strong>{move || t(locale.get(), "cred.help.unconfigured")}</strong>
                                            <span>{move || t(locale.get(), g.unconfigured_key)}</span>
                                        </span>
                                    </span>
                                </span>
                            </div>
                            <div class="settings-form-grid">
                                {g.fields.iter().map(|f| {
                                    let id = f.id;
                                    let stored = move || cred_status.get().get(id).copied().unwrap_or(false);
                                    view! {
                                        <label class="span-2">
                                            <span class="cred-field-head">
                                                <span>{move || format!("{} — {}", t(locale.get(), f.label_key),
                                                    if stored() { t(locale.get(), "cred.stored") } else { t(locale.get(), "cred.not_stored") })}</span>
                                                {move || stored().then(|| view! {
                                                    <button type="button" class="linklike" on:click=move |_| {
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "id": id, "value": "" })).unwrap();
                                                            match invoke_checked("set_credential", arg).await {
                                                                Ok(_) => {
                                                                    cred_inputs.update(|m| { m.remove(id); });
                                                                    cred_status.update(|m| { m.insert(id.into(), false); });
                                                                    cred_msg.set(Some((true, t(locale.get(), "cred.cleared").into())));
                                                                }
                                                                Err(e) => cred_msg.set(Some((false, localize_backend(locale.get(), &js_error_text(e))))),
                                                            }
                                                        });
                                                    }>{move || t(locale.get(), "cred.clear")}</button>
                                                })}
                                            </span>
                                            <input type=if f.secret { "password" } else { "text" }
                                                placeholder=move || if stored() { t(locale.get(), "settings.stored_key").to_string() } else { String::new() }
                                                prop:value=move || cred_inputs.get().get(id).cloned().unwrap_or_default()
                                                on:input=move |ev| { let v = event_target_input(&ev).value(); cred_inputs.update(|m| { m.insert(id.into(), v); }); } />
                                        </label>
                                    }
                                }).collect_view()}
                            </div>
                            <div class="cred-setup-note">
                                <span>{move || t(locale.get(), g.hint_key)}</span>
                                <span class="cred-setup-links">
                                    {g.links.iter().map(|link| {
                                        let url = link.url;
                                        view! {
                                            <button type="button" class="cred-external-link"
                                                on:click=move |_| crate::bindings::open_external_url(url.into())>
                                                <span>{move || t(locale.get(), link.label_key)}</span>
                                                <span aria-hidden="true">"↗"</span>
                                            </button>
                                        }
                                    }).collect_view()}
                                </span>
                            </div>
                        }}).collect_view()}
                        <div class="conn-group-label">{move || t(locale.get(), "cred.custom.name")}</div>
                        <p class="settings-note">{move || t(locale.get(), "cred.custom.hint")}</p>
                        <For
                            each=move || custom_credentials.get()
                            key=|credential| (credential.id.clone(), credential.name.clone())
                            let:credential
                        >
                            {
                                let id = credential.id.clone();
                                let status_id = id.clone();
                                let clear_id = id.clone();
                                let input_id = id.clone();
                                let edit_id = id.clone();
                                let remove_id = id.clone();
                                let initial_present = credential.present;
                                view! {
                                    <div class="custom-credential-card" data-custom-credential=credential.env_var.clone()>
                                        <div class="custom-credential-head">
                                            <div class="custom-credential-meta">
                                                <strong>{credential.name.clone()}</strong>
                                                <code>{credential.env_var.clone()}</code>
                                                <span>{move || if cred_status.get().get(&status_id).copied().unwrap_or(initial_present) {
                                                    t(locale.get(), "cred.stored")
                                                } else {
                                                    t(locale.get(), "cred.not_stored")
                                                }}</span>
                                            </div>
                                            <div class="custom-credential-actions">
                                                {move || cred_status.get().get(&clear_id).copied().unwrap_or(initial_present).then(|| {
                                                    let id = clear_id.clone();
                                                    view! {
                                                        <button type="button" class="linklike" on:click=move |_| {
                                                            let id = id.clone();
                                                            spawn_local(async move {
                                                                let arg = to_value(&serde_json::json!({ "id": id.clone(), "value": "" })).unwrap();
                                                                match invoke_checked("set_credential", arg).await {
                                                                    Ok(_) => {
                                                                        cred_inputs.update(|values| { values.remove(&id); });
                                                                        cred_status.update(|status| { status.insert(id, false); });
                                                                        cred_msg.set(Some((true, t(locale.get(), "cred.cleared").into())));
                                                                    }
                                                                    Err(error) => cred_msg.set(Some((false,
                                                                        localize_backend(locale.get(), &js_error_text(error))))),
                                                                }
                                                            });
                                                        }>{move || t(locale.get(), "cred.clear")}</button>
                                                    }
                                                })}
                                                <button type="button" class="linklike danger" on:click=move |_| {
                                                    let id = remove_id.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                        match invoke_checked("remove_custom_credential", arg).await {
                                                            Ok(_) => {
                                                                custom_credentials.update(|items| items.retain(|item| item.id != id));
                                                                cred_inputs.update(|values| { values.remove(&id); });
                                                                cred_status.update(|status| { status.remove(&id); });
                                                                cred_msg.set(Some((true, t(locale.get(), "cred.custom.removed").into())));
                                                            }
                                                            Err(error) => cred_msg.set(Some((false,
                                                                localize_backend(locale.get(), &js_error_text(error))))),
                                                        }
                                                    });
                                                }>{move || t(locale.get(), "specialists.remove")}</button>
                                            </div>
                                        </div>
                                        <input type="password"
                                            placeholder=move || if cred_status.get().get(&input_id).copied().unwrap_or(initial_present) {
                                                t(locale.get(), "settings.stored_key").to_string()
                                            } else {
                                                t(locale.get(), "cred.custom.value_placeholder").to_string()
                                            }
                                            prop:value=move || cred_inputs.get().get(&id).cloned().unwrap_or_default()
                                            on:input=move |event| {
                                                let value = event_target_input(&event).value();
                                                cred_inputs.update(|values| { values.insert(edit_id.clone(), value); });
                                            } />
                                    </div>
                                }
                            }
                        </For>
                        <div class="settings-sync-block custom-credential-add">
                            <h3>{move || t(locale.get(), "cred.custom.add")}</h3>
                            <div class="settings-form-grid">
                                <label>
                                    <span>{move || t(locale.get(), "cred.custom.service_name")}</span>
                                    <input type="text"
                                        placeholder=move || t(locale.get(), "cred.custom.service_placeholder")
                                        prop:value=move || custom_cred_name.get()
                                        on:input=move |event| custom_cred_name.set(event_target_input(&event).value()) />
                                </label>
                                <label>
                                    <span>{move || t(locale.get(), "cred.custom.env_var")}</span>
                                    <input type="text" class="mono"
                                        placeholder="METASO_API_KEY"
                                        prop:value=move || custom_cred_env.get()
                                        on:input=move |event| custom_cred_env.set(event_target_input(&event).value()) />
                                </label>
                                <label class="span-2">
                                    <span>{move || t(locale.get(), "cred.custom.value")}</span>
                                    <input type="password"
                                        placeholder=move || t(locale.get(), "cred.custom.value_placeholder")
                                        prop:value=move || custom_cred_value.get()
                                        on:input=move |event| custom_cred_value.set(event_target_input(&event).value()) />
                                </label>
                            </div>
                            <p class="settings-field-hint">{move || t(locale.get(), "cred.custom.env_hint")}</p>
                            <div class="row">
                                <button type="button" class="settings-add-btn"
                                    disabled=move || custom_cred_busy.get()
                                        || custom_cred_name.get().trim().is_empty()
                                        || custom_cred_env.get().trim().is_empty()
                                        || custom_cred_value.get().trim().is_empty()
                                    on:click=move |_| {
                                        if custom_cred_busy.get_untracked() { return; }
                                        let name = custom_cred_name.get_untracked();
                                        let env_var = custom_cred_env.get_untracked();
                                        let value = custom_cred_value.get_untracked();
                                        custom_cred_busy.set(true);
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({
                                                "name": name,
                                                "envVar": env_var,
                                                "value": value,
                                            })).unwrap();
                                            match invoke_checked("add_custom_credential", arg).await {
                                                Ok(value) => match serde_wasm_bindgen::from_value::<CustomCredentialStatus>(value) {
                                                    Ok(credential) => {
                                                        cred_status.update(|status| {
                                                            status.insert(credential.id.clone(), credential.present);
                                                        });
                                                        custom_credentials.update(|items| {
                                                            // Backend upserts by env var, so replace a matching row instead of duplicating it.
                                                            match items.iter_mut().find(|item| item.id == credential.id) {
                                                                Some(existing) => *existing = credential,
                                                                None => items.push(credential),
                                                            }
                                                        });
                                                        custom_cred_name.set(String::new());
                                                        custom_cred_env.set(String::new());
                                                        custom_cred_value.set(String::new());
                                                        cred_msg.set(Some((true, t(locale.get(), "cred.custom.added").into())));
                                                    }
                                                    Err(error) => cred_msg.set(Some((false, error.to_string()))),
                                                },
                                                Err(error) => cred_msg.set(Some((false,
                                                    localize_backend(locale.get(), &js_error_text(error))))),
                                            }
                                            custom_cred_busy.set(false);
                                        });
                                    }>{move || if custom_cred_busy.get() {
                                        t(locale.get(), "cred.custom.adding")
                                    } else {
                                        t(locale.get(), "cred.custom.add")
                                    }}</button>
                            </div>
                        </div>
                        {move || cred_msg.get().map(|(ok, text)| view! {
                            <div class="settings-status" class:ok=move || ok class:fail=move || !ok>{text}</div>
                        })}
                        <div class="row settings-footer">
                            <button type="button" class="primary" on:click=move |_| {
                                // Save every field that was edited (non-empty input); blank inputs
                                // leave a stored key untouched (placeholder communicates this).
                                let edits: Vec<(String, String)> = cred_inputs.get().into_iter()
                                    .filter(|(_, v)| !v.trim().is_empty()).collect();
                                if edits.is_empty() { return; }
                                spawn_local(async move {
                                    let mut ok_all = true;
                                    for (id, value) in edits {
                                        let arg = to_value(&serde_json::json!({ "id": id, "value": value })).unwrap();
                                        if let Err(e) = invoke_checked("set_credential", arg).await {
                                            ok_all = false;
                                            cred_msg.set(Some((false, localize_backend(locale.get(), &js_error_text(e)))));
                                            break;
                                        }
                                    }
                                    if ok_all {
                                        cred_inputs.set(std::collections::HashMap::new());
                                        cred_msg.set(Some((true, t(locale.get(), "cred.saved").into())));
                                    }
                                    let v = invoke("credential_status", JsValue::UNDEFINED).await;
                                    if let Ok(pairs) = serde_wasm_bindgen::from_value::<Vec<(String, bool)>>(v) {
                                        cred_status.set(pairs.into_iter().collect());
                                    }
                                });
                            }>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "channels" && channels_open.get().is_none()).then(|| view! {
                    <div class="settings-pane">
                        <div class="settings-form-grid">
                            <div class="span-2 settings-sync-block">
                                <h3>{move || t(locale.get(), "settings.sync.title")}</h3>
                                <p class="settings-field-hint">{move || t(locale.get(), "settings.sync.hint")}</p>
                                <label>{move || t(locale.get(), "settings.sync.backend")}
                                    <select data-testid="sync-backend"
                                        prop:value=move || settings.get().sync_backend
                                        on:change=move |ev| settings.update(|current| current.sync_backend = dom_value(&ev))>
                                        <option value="relay">{move || t(locale.get(), "settings.sync.relay")}</option>
                                        <option value="folder">{move || t(locale.get(), "settings.sync.folder")}</option>
                                    </select>
                                </label>
                                {move || if settings.get().sync_backend == "folder" {
                                    view! {
                                        <label>{move || t(locale.get(), "settings.sync.folder_path")}
                                            <div class="settings-path-row">
                                                <input class="settings-path-input" data-testid="sync-folder"
                                                    prop:value=move || settings.get().sync_folder
                                                    on:input=move |ev| settings.update(|current| current.sync_folder = event_target_input(&ev).value()) />
                                                <button type="button" class="settings-add-btn" data-testid="sync-choose-folder"
                                                    on:click=choose_sync_folder>
                                                    {move || t(locale.get(), "projects.choose_dir")}
                                                </button>
                                            </div>
                                            <span class="settings-field-hint">{move || t(locale.get(), "settings.sync.folder_hint")}</span>
                                        </label>
                                    }.into_view()
                                } else {
                                    view! {
                                        <label>{move || t(locale.get(), "settings.sync.relay_url")}
                                            <input data-testid="sync-relay-url" type="url"
                                                prop:value=move || settings.get().sync_relay_url
                                                placeholder="https://sync.example.com"
                                                on:input=move |ev| settings.update(|current| current.sync_relay_url = event_target_input(&ev).value()) />
                                        </label>
                                        <label>{move || t(locale.get(), "settings.sync.relay_token")}
                                            <input data-testid="sync-relay-token" type="password"
                                                prop:value=move || settings.get().sync_relay_token
                                                placeholder=move || if settings.get().has_sync_relay_token {
                                                    t(locale.get(), "settings.key_stored")
                                                } else {
                                                    t(locale.get(), "settings.sync.token_placeholder")
                                                }
                                                on:input=move |ev| settings.update(|current| current.sync_relay_token = event_target_input(&ev).value()) />
                                            <span class="settings-field-hint">{move || t(locale.get(), "settings.sync.relay_hint")}</span>
                                        </label>
                                    }.into_view()
                                }}
                                <p class="settings-field-hint">
                                    {move || t(locale.get(), "settings.sync.join_hint")}
                                </p>
                                <div class="row settings-sync-actions">
                                    <button type="button" on:click=open_sync_guide>
                                        {compose_icon("doc")}
                                        <span>{move || t(locale.get(), "projects.sync.guide")}</span>
                                    </button>
                                    <button type="button" class="primary"
                                        on:click=move |_| {
                                            join_error.set(None);
                                            joining.set(true);
                                        }>
                                        {compose_icon("link")}
                                        <span>{move || t(locale.get(), "projects.sync.join")}</span>
                                    </button>
                                </div>
                            </div>
                        </div>
                        <div class="row settings-footer">
                            <button type="button" disabled=move || settings_busy.get() on:click=move |_| show_settings.set(false)>{move || t(locale.get(), "settings.cancel")}</button>
                            <button type="button" class="primary" disabled=move || settings_busy.get() on:click=move |ev| save_settings.call(ev)>{move || t(locale.get(), "settings.save")}</button>
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "channels").then(|| view! {
                    <crate::channels_view::ChannelsPane locale=locale open=channels_open/>
                }.into_view())}
                {move || (settings_section.get() == "permissions").then(|| view! {
                    <div class="settings-pane settings-pane-list">
                        <div class="settings-toolbar settings-toolbar-end">
                            <span class="settings-filter">{move || {
                                format!("{} ({})", t(locale.get(), "settings.nav.permissions"), approval_grants.get().len())
                            }}</span>
                            <button type="button" class="settings-add-btn"
                                disabled=move || approval_grants.get().is_empty()
                                on:click=move |_| {
                                    spawn_local(async move {
                                        let _ = invoke_checked("revoke_all_approval_grants", JsValue::UNDEFINED).await;
                                        refresh_approval_grants.call(());
                                    });
                                }>{move || t(locale.get(), "permissions.revoke_all")}</button>
                        </div>
                        <p class="settings-note">{move || t(locale.get(), "permissions.note")}</p>
                        {move || approval_grants.get().is_empty().then(|| view! {
                            <div class="settings-status">{move || t(locale.get(), "permissions.empty")}</div>
                        })}
                        <div class="settings-list">
                            {move || approval_grants.get().into_iter().map(|row| {
                                let scope_label = match row.scope.as_str() {
                                    "session" => "permissions.scope.session",
                                    "project" => "permissions.scope.project",
                                    "global" => "permissions.scope.global",
                                    _ => "approval.scope.once",
                                };
                                let subtitle = format!("{} - {}", row.kind, row.target);
                                let scope = row.scope.clone();
                                let kind = row.kind.clone();
                                let target = row.target.clone();
                                let session_id = row.session_id.clone();
                                let project_id = row.project_id.clone();
                                view! {
                                    <div class="settings-list-row">
                                        <div class="settings-list-main">
                                            <span class="settings-list-title">{row.label}</span>
                                            <span class="settings-list-sub">{subtitle}</span>
                                        </div>
                                        <div class="settings-list-actions">
                                            <span class="badge">{move || t(locale.get(), scope_label)}</span>
                                            <button class="settings-list-remove" type="button"
                                                title=move || t(locale.get(), "permissions.revoke")
                                                on:click=move |_| {
                                                    let scope = scope.clone();
                                                    let kind = kind.clone();
                                                    let target = target.clone();
                                                    let session_id = session_id.clone();
                                                    let project_id = project_id.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({
                                                            "scope": scope,
                                                            "kind": kind,
                                                            "target": target,
                                                            "sessionId": session_id,
                                                            "projectId": project_id,
                                                        })).unwrap();
                                                        let _ = invoke_checked("revoke_approval_grant", arg).await;
                                                        refresh_approval_grants.call(());
                                                    });
                                                }>{compose_icon("close")}</button>
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                }.into_view())}
                {move || (settings_section.get() == "connections").then(|| {
                    if conn_form_open.get() {
                        view! {
                            <div class="settings-pane settings-pane-subpage">
                                <div class="conn-form">
                                    <label>{move || t(locale.get(),"conn.name")}
                                        <input prop:value=move || conn_form.get().map(|f| f.name.clone()).unwrap_or_default()
                                            disabled=move || oauth_authorizing.get()
                                            on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.name = event_target_input(&ev).value(); }) /></label>
                                    <label>{move || t(locale.get(),"conn.kind")}
                                        <select prop:value=move || conn_form.get().map(|f| f.kind.clone()).unwrap_or_else(|| "stdio".into())
                                            disabled=move || oauth_authorizing.get()
                                            on:change=move |ev| {
                                                let kind = dom_value(&ev);
                                                conn_form.update(|form| if let Some(form) = form {
                                                    form.kind = kind;
                                                });
                                            }>
                                            <option value="stdio">{move || t(locale.get(),"conn.kind.stdio")}</option>
                                            <option value="http">{move || t(locale.get(),"conn.kind.http")}</option>
                                        </select></label>
                                    {move || (conn_form_kind.get() == "stdio").then(|| view!{
                                        <label>{move || t(locale.get(),"conn.command")}
                                            <input prop:value=move || conn_form.get().map(|f| f.command.clone()).unwrap_or_default()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.command = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(),"conn.args")}
                                            <input placeholder="arg1 arg2" prop:value=move || conn_form.get().map(|f| f.args.clone()).unwrap_or_default()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.args = event_target_input(&ev).value(); }) /></label>
                                        <div class="conn-secret-fields">
                                            <span class="conn-secret-label">{move || t(locale.get(),"conn.env")}</span>
                                            <ConnSecretRows conn_form=conn_form kind=ConnSecretKind::Env disabled=oauth_authorizing />
                                            <button type="button" class="settings-add-btn conn-secret-add"
                                                on:click=move |_| conn_form.update(|o| if let Some(o)=o {
                                                    o.env.push(blank_conn_secret_field());
                                                })>
                                                {compose_icon("plus")}
                                                <span>{move || t(locale.get(), "conn.secret_add_env")}</span>
                                            </button>
                                            <p class="hint">{move || t(locale.get(), "conn.secret_hint")}</p>
                                        </div>
                                    })}
                                    {move || (conn_form_kind.get() == "http").then(|| view!{
                                        <label>{move || t(locale.get(),"conn.url")}
                                            <input placeholder="https://host/mcp" prop:value=move || conn_form.get().map(|f| f.url.clone()).unwrap_or_default()
                                                disabled=move || oauth_authorizing.get()
                                                on:input=move |ev| conn_form.update(|o| if let Some(o)=o { o.url = event_target_input(&ev).value(); }) /></label>
                                        <label>{move || t(locale.get(),"conn.auth")}
                                            <select prop:value=move || conn_form.get().map(|f| f.auth.clone()).filter(|v| !v.is_empty()).unwrap_or_else(|| "none".into())
                                                disabled=move || oauth_authorizing.get()
                                                on:change=move |ev| {
                                                    let auth = dom_value(&ev);
                                                    conn_form.update(|form| if let Some(form) = form { form.auth = auth; });
                                                }>
                                                <option value="none">{move || t(locale.get(),"conn.auth.none")}</option>
                                                <option value="oauth">{move || t(locale.get(),"conn.auth.oauth")}</option>
                                            </select>
                                        </label>
                                        <div class="conn-secret-fields">
                                            <span class="conn-secret-label">{move || t(locale.get(),"conn.headers")}</span>
                                            <ConnSecretRows conn_form=conn_form kind=ConnSecretKind::Headers disabled=oauth_authorizing />
                                            <button type="button" class="settings-add-btn conn-secret-add"
                                                disabled=move || oauth_authorizing.get()
                                                on:click=move |_| conn_form.update(|o| if let Some(o)=o {
                                                    o.headers.push(blank_conn_secret_field());
                                                })>
                                                {compose_icon("plus")}
                                                <span>{move || t(locale.get(), "conn.secret_add_header")}</span>
                                            </button>
                                            <p class="hint">{move || t(locale.get(), "conn.secret_hint")}</p>
                                        </div>
                                    })}
                                    {move || (conn_form_kind.get() == "http"
                                        && conn_form.get().is_some_and(|form| form.auth == "oauth")).then(|| view!{
                                        <p class="settings-note">{move || t(locale.get(), "conn.oauth.desc")}</p>
                                    })}
                                    {move || if conn_testing.get() {
                                        view! {
                                            <div class="settings-status pending" data-testid="conn-test-status"
                                                role="status" aria-live="polite">
                                                <span class="conn-test-spinner" aria-hidden="true"></span>
                                                {move || t(
                                                    locale.get(),
                                                    if oauth_authorizing.get() {
                                                        "conn.oauth.waiting"
                                                    } else {
                                                        "conn.testing_status"
                                                    },
                                                )}
                                            </div>
                                        }.into_view()
                                    } else {
                                        conn_test_msg.get().map(|(ok, msg)| view! {
                                            <div class="settings-status" class:ok=ok class:fail=!ok
                                                data-testid="conn-test-status">{msg}</div>
                                        }).into_view()
                                    }}
                                    <div class="row settings-footer">
                                        <button type="button" data-testid="conn-test"
                                            disabled=move || conn_testing.get() || oauth_authorizing.get()
                                            attr:aria-busy=move || if conn_testing.get() { "true" } else { "false" }
                                            on:click=move |_| {
                                                if conn_testing.get() || oauth_authorizing.get() {
                                                    return;
                                                }
                                                let f = conn_form.get().unwrap_or_default();
                                                conn_testing.set(true);
                                                conn_test_msg.set(None);
                                                conn_test_seq.update(|n| *n += 1);
                                                let seq = conn_test_seq.get_untracked();
                                                let oauth = f.kind == "http" && f.auth == "oauth";
                                                if oauth {
                                                    oauth_authorizing.set(true);
                                                }
                                                spawn_local(async move {
                                                    let conn = build_conn_json(&f, false);
                                                    let command = if oauth {
                                                        "test_oauth_mcp_connection"
                                                    } else {
                                                        "test_mcp_connection"
                                                    };
                                                    let outcome = invoke_checked(
                                                        command,
                                                        to_value(&serde_json::json!({"conn": conn})).unwrap(),
                                                    )
                                                    .await;
                                                    if conn_test_seq.get_untracked() != seq {
                                                        return;
                                                    }
                                                    match outcome {
                                                        Ok(v) => match serde_wasm_bindgen::from_value::<Vec<ConnectorTool>>(v) {
                                                            Ok(tools) => {
                                                                let n = tools.len();
                                                                if let Some(id) = f.id.clone() {
                                                                    custom_conn_tools.update(|m| { m.insert(id, tools); });
                                                                }
                                                                conn_test_msg.set(Some((true, format!("OK — {n} tools"))));
                                                            }
                                                            Err(e) => conn_test_msg.set(Some((false, e.to_string()))),
                                                        },
                                                        Err(e) => conn_test_msg.set(Some((false, js_error_text(e)))),
                                                    }
                                                    conn_testing.set(false);
                                                    if oauth {
                                                        oauth_authorizing.set(false);
                                                    }
                                                });
                                            }>
                                            {move || t(
                                                locale.get(),
                                                if conn_testing.get() { "conn.testing" } else { "conn.test" },
                                            )}
                                        </button>
                                        <button type="button"
                                            on:click=move |_| {
                                                if oauth_authorizing.get() {
                                                    spawn_local(async move {
                                                        let _ = invoke_checked("cancel_oauth_authorization", JsValue::UNDEFINED).await;
                                                    });
                                                }
                                                conn_test_seq.update(|n| *n += 1);
                                                conn_testing.set(false);
                                                oauth_authorizing.set(false);
                                                close_settings_subpage.call(());
                                            }>{move || t(locale.get(),"settings.cancel")}</button>
                                        <button type="button" class="primary" on:click=move |_| {
                                            if conn_testing.get() || oauth_authorizing.get() {
                                                return;
                                            }
                                            let f = conn_form.get().unwrap_or_default();
                                            spawn_local(async move {
                                                if f.kind == "http" && f.auth == "oauth" {
                                                    oauth_authorizing.set(true);
                                                    conn_test_msg.set(Some((true, t(locale.get(), "conn.oauth.waiting").into())));
                                                    let conn = build_conn_json(&f, true);
                                                    let args = to_value(&serde_json::json!({ "conn": conn })).unwrap();
                                                    match invoke_checked("authorize_http_connection", args).await {
                                                        Ok(_) => {
                                                            conn_form.set(None);
                                                            conn_test_msg.set(None);
                                                            refresh_conns.call(());
                                                        }
                                                        Err(error) => {
                                                            conn_test_msg.set(Some((false, js_error_text(error))));
                                                        }
                                                    }
                                                    oauth_authorizing.set(false);
                                                    return;
                                                }
                                                let editing = f.id.is_some();
                                                let conn = build_conn_json(&f, true);
                                                let cmd = if editing { "update_mcp_connection" } else { "add_mcp_connection" };
                                                if invoke_checked(cmd, to_value(&serde_json::json!({"conn": conn})).unwrap()).await.is_ok() {
                                                    conn_form.set(None); conn_test_msg.set(None); refresh_conns.call(());
                                                }
                                            });
                                        } disabled=move || conn_testing.get() || oauth_authorizing.get()>
                                            {move || t(locale.get(), "settings.save")}
                                        </button>
                                    </div>
                                </div>
                            </div>
                        }.into_view()
                    } else if open_conn_key.get().is_some() {
                        // Level 2 — connector detail. Bundled connectors have static approval controls;
                        // custom MCP tools are discovered on demand.
                        view! {
                            <div class="settings-pane settings-pane-subpage connector-detail" data-testid="connector-detail">
                                {move || {
                                    let key = open_conn_key.get();
                                    let conn = key.and_then(|k| connectors.get().and_then(|v| v.connectors.into_iter().find(|c| c.key == k)));
                                    conn.map(|c| {
                                        let is_custom = c.kind == "custom";
                                        let connector_key = c.key.clone();
                                        let key_enabled = c.key.clone();
                                        let description = if locale.get() == Locale::Zh && !c.description_zh.is_empty() {
                                            c.description_zh.clone()
                                        } else { c.description.clone() };
                                        let skip_on = c.skip_approvals;
                                        let key_skip = c.key.clone();
                                        let service = c.subtitle.clone();
                                        let enabled = c.enabled;
                                        let transport = c.transport.clone();
                                        let auth = c.auth.clone();
                                        let tools = if is_custom {
                                            custom_conn_tools.get().get(&c.key).cloned().unwrap_or_default()
                                        } else {
                                            c.tools.clone()
                                        };
                                        let loading = is_custom && custom_conn_tools_loading.get().contains(&c.key);
                                        let error = if is_custom {
                                            custom_conn_tool_errors.get().get(&c.key).cloned()
                                        } else {
                                            None
                                        };
                                        let has_error = error.is_some();
                                        view! {
                                            <div class="conn-detail-heading">
                                                <div class="conn-detail-title">{compose_icon("grid")}<h2>{c.name.clone()}</h2>
                                                    <span class="badge">{move || t(locale.get(), if is_custom { "conn.custom_badge" } else { "conn.bundled_badge" })}</span>
                                                </div>
                                                <label class="toggle">
                                                    <input type="checkbox" aria-label=move || t(locale.get(), "conn.enabled_toggle") prop:checked=enabled on:change=move |ev| {
                                                        let key = key_enabled.clone();
                                                        let enabled = event_target_checked(&ev);
                                                        spawn_local(async move {
                                                            let args = if is_custom { serde_json::json!({"id": key, "enabled": enabled}) } else { serde_json::json!({"key": key, "enabled": enabled}) };
                                                            let command = if is_custom { "set_mcp_connection_enabled" } else { "set_connector_enabled" };
                                                            let _ = invoke_checked(command, to_value(&args).unwrap()).await;
                                                            refresh_conns.call(());
                                                        });
                                                    } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                            </div>
                                            {(!description.is_empty()).then(|| view! { <p class="conn-introduction">{description}</p> })}
                                            <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                                            {is_custom.then(|| view! {
                                                <div class="settings-list">
                                                    <div class="settings-list-row">
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{move || t(locale.get(), "conn.service")}</span>
                                                            <span class="settings-list-sub">{service}</span>
                                                        </div>
                                                    </div>
                                                    <div class="settings-list-row">
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{move || t(locale.get(), "conn.status")}</span>
                                                            <span class="settings-list-sub">{move || t(locale.get(), if enabled {
                                                                "conn.status.enabled"
                                                            } else {
                                                                "conn.status.disabled"
                                                            })}</span>
                                                        </div>
                                                    </div>
                                                    {(transport == "http").then(|| view! {
                                                        <div class="settings-list-row">
                                                            <div class="settings-list-main">
                                                                <span class="settings-list-title">{move || t(locale.get(), "conn.auth")}</span>
                                                                <span class="settings-list-sub">{move || t(locale.get(), if auth == "oauth" {
                                                                    "conn.auth.oauth"
                                                                } else {
                                                                    "conn.auth.none"
                                                                })}</span>
                                                            </div>
                                                        </div>
                                                    })}
                                                </div>
                                            })}
                                            {(!is_custom).then(|| view! {
                                                <div class="settings-list">
                                                    <div class="settings-list-row">
                                                        <div class="settings-list-main">
                                                            <span class="settings-list-title">{move || t(locale.get(), "conn.skip_approvals")}</span>
                                                            <span class="settings-list-sub">{move || t(locale.get(), "conn.skip_approvals.desc")}</span>
                                                        </div>
                                                        <label class="toggle">
                                                            <input type="checkbox" prop:checked=skip_on on:change=move |ev| {
                                                                let key = key_skip.clone();
                                                                let on = event_target_checked(&ev);
                                                                spawn_local(async move {
                                                                    let arg = to_value(&serde_json::json!({ "key": key, "enabled": on })).unwrap();
                                                                    let _ = invoke_checked("set_connector_skip_approvals", arg).await;
                                                                    refresh_conns.call(());
                                                                });
                                                            } />
                                                            <span class="toggle-track" aria-hidden="true"></span>
                                                        </label>
                                                    </div>
                                                </div>
                                            })}
                                            <div class="conn-group-label">{move || t(locale.get(), "conn.tools")}</div>
                                            {loading.then(|| view! {
                                                <div class="settings-status">{move || t(locale.get(), "conn.tools_loading")}</div>
                                            })}
                                            {error.map(|msg| view! {
                                                <div class="settings-status fail">{move || tf(locale.get(), "conn.tools_failed", &[("msg", &msg)])}</div>
                                            })}
                                            {(!loading && !has_error && tools.is_empty()).then(|| view! {
                                                <div class="settings-status">{move || t(locale.get(), "conn.no_tools")}</div>
                                            })}
                                            <p class="hint conn-tools-hint">{move || t(locale.get(), "conn.tools_hint")}</p>
                                            <div class="settings-list conn-tools-list">
                                                {tools.iter().map(|tool| {
                                                    let name = tool.name.clone();
                                                    let mode = tool.mode.clone();
                                                    let tool_key = (connector_key.clone(), name.clone());
                                                    let key_read = tool_key.clone();
                                                    let key_toggle = tool_key.clone();
                                                    let key_body = tool_key.clone();
                                                    let documentation = tool.clone();
                                                    let seg = |m: &'static str, icon: &'static str, key: &'static str| {
                                                        let name2 = name.clone();
                                                        let active = mode.as_str() == m;
                                                        view! {
                                                            <button type="button" class=format!("approval-btn approval-{m}") class:active=active
                                                                disabled=skip_on aria-pressed=active.to_string()
                                                                title=move || t(locale.get(), key) aria-label=move || t(locale.get(), key)
                                                                on:click=move |_| {
                                                                    let name = name2.clone();
                                                                    spawn_local(async move {
                                                                        let arg = to_value(&serde_json::json!({ "tool": name, "mode": m })).unwrap();
                                                                        let _ = invoke_checked("set_tool_approval", arg).await;
                                                                        refresh_conns.call(());
                                                                    });
                                                                }>{compose_icon(icon)}</button>
                                                        }
                                                    };
                                                    view! {
                                                        <div class="conn-tool" data-tool=tool.name.clone()>
                                                            <div class="conn-tool-row">
                                                                <button type="button" class="conn-tool-disclosure"
                                                                    aria-expanded=move || expanded_connector_tools.with(|open| open.contains(&key_read)).to_string()
                                                                    on:click=move |_| expanded_connector_tools.update(|open| {
                                                                        if !open.remove(&key_toggle) { open.insert(key_toggle.clone()); }
                                                                    })>
                                                                    {compose_icon("chevron-right")}<span>{tool.name.clone()}</span>
                                                                </button>
                                                                {(!is_custom).then(|| view! {
                                                                    <div class="approval-seg" class:disabled=skip_on>
                                                                        {seg("allow", "check", "conn.approval.allow")}
                                                                        {seg("ask", "hand", "conn.approval.ask")}
                                                                        {seg("deny", "ban", "conn.approval.deny")}
                                                                    </div>
                                                                })}
                                                            </div>
                                                            {move || expanded_connector_tools.with(|open| open.contains(&key_body)).then(|| view! {
                                                                <ConnectorToolDocumentation locale=locale tool=documentation.clone() />
                                                            })}
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                            {(!c.maintainer.is_empty() || !c.links.is_empty()).then(|| view! {
                                                <section class="conn-source-details" data-testid="connector-source-details">
                                                    <h3>{move || t(locale.get(), "conn.details")}</h3>
                                                    {(!c.maintainer.is_empty()).then(|| view! {
                                                        <dl><dt>{move || t(locale.get(), "conn.maintainer")}</dt><dd>{c.maintainer.clone()}</dd></dl>
                                                    })}
                                                    {(!c.links.is_empty()).then(|| view! {
                                                        <h4>{move || t(locale.get(), "conn.sources")}</h4>
                                                        <ul>{c.links.iter().filter(|link| link.url.starts_with("https://") || link.url.starts_with("http://")).map(|link| {
                                                            let url = link.url.clone();
                                                            view! { <li><a href=link.url.clone() target="_blank" rel="noopener noreferrer" on:click=move |ev| {
                                                                ev.prevent_default(); open_external_url(url.clone());
                                                            }>{link.label.clone()}{compose_icon("link")}</a></li> }
                                                        }).collect_view()}</ul>
                                                    })}
                                                </section>
                                            })}
                                        }
                                    })
                                }}
                            </div>
                        }.into_view()
                    } else {
                        view! {
                    <div class="settings-pane settings-pane-list">
                        <div class="settings-toolbar settings-toolbar-end">
                            <span class="settings-filter">{move || {
                                let nb = connectors.get().map(|v| v.connectors.iter().filter(|c| c.kind == "bundled").count()).unwrap_or(0);
                                let nc = conns_view.get().map(|v| v.connections.len()).unwrap_or(0);
                                format!("{} ({})", t(locale.get(), "settings.nav.connections"), nb + nc)
                            }}</span>
                            <button type="button" class="settings-add-btn" on:click=move |_| {
                                conn_form.set(Some(new_conn_form()));
                                conn_test_msg.set(None);
                            }>{move || t(locale.get(), "conn.add")}</button>
                        </div>
                        <p class="settings-note">{move || t(locale.get(), "settings.applies_new_session")}</p>
                        <div class="settings-list">
                            <div class="settings-list-row">
                                <div class="settings-list-main">
                                    <span class="settings-list-title">{move || t(locale.get(), "conn.scope")}</span>
                                    <span class="settings-list-sub">{move || {
                                        let cur = connectors.get().map(|v| v.scope).unwrap_or_else(|| "ask".into());
                                        t(locale.get(), match cur.as_str() {
                                            "full" => "conn.scope.full.desc",
                                            "auto" => "conn.scope.auto.desc",
                                            _ => "conn.scope.ask.desc",
                                        })
                                    }}</span>
                                </div>
                                <div class="approval-seg">
                                    {["ask", "auto", "full"].into_iter().map(|val| {
                                        let label_key = match val {
                                            "full" => "conn.scope.full",
                                            "auto" => "conn.scope.auto",
                                            _ => "conn.scope.ask",
                                        };
                                        let active = move || connectors.get().map(|v| v.scope).unwrap_or_else(|| "ask".into()) == val;
                                        view! {
                                            <button type="button" class=format!("approval-btn scope-seg scope-{val}") class:active=active
                                                on:click=move |_| {
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "scope": val })).unwrap();
                                                        let _ = invoke_checked("set_approval_scope", arg).await;
                                                        refresh_conns.call(());
                                                    });
                                                }>{move || t(locale.get(), label_key)}</button>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        </div>
                        <div class="conn-group-label">{move || t(locale.get(), "conn.featured")}</div>
                        <div class="settings-list">
                            <For each=move || connectors.get().map(|v| v.connectors.into_iter().filter(|c| c.kind == "bundled").collect::<Vec<_>>()).unwrap_or_default() key=|c| c.key.clone() let:c>
                                {
                                    let key_open = c.key.clone();
                                    let key_toggle = c.key.clone();
                                    let n_tools = c.tools.len();
                                    let enabled = c.enabled;
                                    view! {
                                        <div class="settings-list-row settings-list-row-link"
                                            on:click=move |_| open_conn_key.set(Some(key_open.clone()))>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">{c.name.clone()}</span>
                                                <span class="settings-list-sub">{move || tf(locale.get(), "conn.tools_count", &[("n", &n_tools.to_string())])}</span>
                                            </div>
                                            <div class="settings-list-actions">
                                                <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                                    <input type="checkbox" prop:checked=enabled on:change=move |ev| {
                                                        let key = key_toggle.clone();
                                                        let on = event_target_checked(&ev);
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "key": key, "enabled": on })).unwrap();
                                                            let _ = invoke_checked("set_connector_enabled", arg).await;
                                                            refresh_conns.call(());
                                                        });
                                                    } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                                <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                        </div>
                        {move || conns_view.get().map(|v| v.connections.len()).unwrap_or(0).gt(&0).then(|| view! {
                            <div class="conn-group-label">{move || t(locale.get(), "conn.custom")}</div>
                        })}
                        <div class="settings-list">
                            <For each=move || conns_view.get().map(|v| v.connections).unwrap_or_default() key=|c| c.id.clone() let:c>
                                {
                                    let id_del = c.id.clone();
                                    let id_toggle = c.id.clone();
                                    let id_open = c.id.clone();
                                    let row_open = c.clone();
                                    let row_edit = c.clone();
                                    let kind_badge = match &c.transport {
                                        ConnTransport::Stdio { .. } => "stdio",
                                        ConnTransport::Http { .. } => "http",
                                    };
                                    let auth_badge = match &c.transport {
                                        ConnTransport::Http { auth, .. } if auth == "oauth" => Some("OAuth"),
                                        _ => None,
                                    };
                                    let enabled = c.enabled;
                                    view! {
                                        <div class="settings-list-row settings-list-row-link"
                                            on:click=move |_| {
                                                open_conn_key.set(Some(id_open.clone()));
                                                load_custom_conn_tools.call(row_open.clone());
                                            }>
                                            <div class="settings-list-main">
                                                <span class="settings-list-title">
                                                    {c.name.clone()}
                                                    " "
                                                    <span class="badge">{kind_badge}</span>
                                                    {auth_badge.map(|auth| view! { <span class="badge">{auth}</span> })}
                                                </span>
                                                <span class="settings-list-sub">
                                                    {match &c.transport {
                                                        ConnTransport::Stdio { command, .. } => command.clone(),
                                                        ConnTransport::Http { url, .. } => url.clone(),
                                                    }}
                                                </span>
                                                <span class="settings-list-sub">
                                                    {move || t(locale.get(), if enabled {
                                                        "conn.status.enabled"
                                                    } else {
                                                        "conn.status.disabled"
                                                    })}
                                                </span>
                                            </div>
                                            <div class="settings-list-actions">
                                                <button class="settings-list-edit" type="button"
                                                    title=move || t(locale.get(), "conn.edit")
                                                    aria-label=move || t(locale.get(), "conn.edit")
                                                    on:click=move |ev| {
                                                        ev.stop_propagation();
                                                        conn_form.set(Some(conn_form_from_row(&row_edit)));
                                                        conn_test_msg.set(None);
                                                    }>{compose_icon("edit")}</button>
                                                <button class="settings-list-remove" type="button" title="remove" on:click=move |ev| {
                                                    ev.stop_propagation();
                                                    let id = id_del.clone();
                                                    spawn_local(async move {
                                                        let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                        let _ = invoke_checked("delete_mcp_connection", arg).await;
                                                        refresh_conns.call(());
                                                    });
                                                }>{compose_icon("close")}</button>
                                                <label class="toggle" on:click=move |ev| ev.stop_propagation()>
                                                    <input type="checkbox" prop:checked=c.enabled on:change=move |ev| {
                                                        let id = id_toggle.clone();
                                                        let on = event_target_checked(&ev);
                                                        spawn_local(async move {
                                                            let arg = to_value(&serde_json::json!({ "id": id, "enabled": on })).unwrap();
                                                            let _ = invoke_checked("set_mcp_connection_enabled", arg).await;
                                                            refresh_conns.call(());
                                                        });
                                                    } />
                                                    <span class="toggle-track" aria-hidden="true"></span>
                                                </label>
                                                <span class="settings-list-chevron" aria-hidden="true">"›"</span>
                                            </div>
                                        </div>
                                    }
                                }
                            </For>
                        </div>
                    </div>
                        }.into_view()
                    }
                })}
            </div>
            {move || delete_confirm.get().map(|target| {
                let label = target.label().to_string();
                let is_plugin = matches!(target, DeleteConfirm::Plugin { .. });
                let is_skill = matches!(target, DeleteConfirm::Skill { .. });
                let is_host = matches!(target, DeleteConfirm::Host { .. });
                let host_detail = match &target {
                    DeleteConfirm::Host { detail, .. } => Some(detail.clone()),
                    _ => None,
                };
                let (message_key, placeholder, action_key, test_id) = if is_plugin {
                    ("plugins.remove_confirm", "plugin", "plugins.remove", "plugin-remove-confirm")
                } else if is_skill {
                    ("skills.remove_confirm", "skill", "skills.remove", "skill-remove-confirm")
                } else if is_host {
                    ("hosts.remove_confirm", "host", "environments.remove", "host-remove-confirm")
                } else {
                    ("models.remove_confirm", "model", "models.remove", "model-delete-confirm")
                };
                view! {
                    <div class="overlay" data-testid=test_id>
                        <div class="modal confirm-modal">
                            <h2>{move || t(locale.get(), "confirm.title")}</h2>
                            <div class="hint">{move || tf(
                                locale.get(),
                                message_key,
                                &[(placeholder, &label)],
                            )}</div>
                            {host_detail.clone().map(|detail| view! {
                                <div class="hint host-disposal-detail" data-testid="host-disposal-detail">{detail}</div>
                            })}
                            <div class="row">
                                <button on:click=move |_| delete_confirm.set(None)>
                                    {move || t(locale.get(), "settings.cancel")}
                                </button>
                                <button class="primary" on:click=move |_| {
                                    let target = target.clone();
                                    delete_confirm.set(None);
                                    spawn_local(async move {
                                        match target {
                                            DeleteConfirm::Model { id, .. } => {
                                                let arg = to_value(&serde_json::json!({ "id": id })).unwrap();
                                                if let Ok(value) = invoke_checked("remove_model", arg).await {
                                                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ModelProfile>>(value) {
                                                        models.set(list);
                                                    }
                                                }
                                            }
                                            DeleteConfirm::Acp { id, .. } => {
                                                settings_busy.set(true);
                                                let args = to_value(&serde_json::json!({ "id": id.clone() })).unwrap();
                                                match invoke_checked("remove_acp_agent", args).await {
                                                    Ok(value) => {
                                                        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<AcpAgentProfile>>(value) {
                                                            acp_agents.set(list);
                                                            acp_infos.update(|infos| {
                                                                infos.remove(&id);
                                                            });
                                                            if active_acp_agent_id.get().as_deref() == Some(id.as_str()) {
                                                                active_acp_agent_id.set(None);
                                                            }
                                                        }
                                                    }
                                                    Err(error) => acp_form_msg.set(Some((false, js_error_text(error)))),
                                                }
                                                settings_busy.set(false);
                                            }
                                            DeleteConfirm::Plugin { id, version, .. } => {
                                                remove_plugin.call((id, version));
                                            }
                                            DeleteConfirm::Host { alias, .. } => {
                                                remove_ssh_host.call(alias);
                                            }
                                            DeleteConfirm::Skill { name, .. } => {
                                                let arg = to_value(&serde_json::json!({ "name": name })).unwrap();
                                                match invoke_checked("remove_skill", arg).await {
                                                    Ok(_) => {
                                                        skills_msg.set(Some((true, t(locale.get(), "skills.removed").into())));
                                                        refresh_skills.call(());
                                                    }
                                                    Err(error) => {
                                                        skills_msg.set(Some((
                                                            false,
                                                            localize_backend(locale.get(), &js_error_text(error)),
                                                        )));
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }>{move || t(locale.get(), action_key)}</button>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
}.into_view())
    }
}
