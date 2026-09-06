use super::{
    cap_rows, gene_args, gnomad_gene_url, gnomad_variant_url, graphql, json_string,
    reference_genome, require_dataset, require_region, require_sv_dataset, require_text,
    require_variant_id, sort_by_pos, sorted_strings, ChromKind, Gql, NativeBio, GNOMAD_API,
    GNOMAD_BROWSER, LIST_CAP,
};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const VARIANT_Q: &str = r#"
query Variant($variantId: String!, $dataset: DatasetId!) {
  variant(variantId: $variantId, dataset: $dataset) {
    variant_id reference_genome chrom pos ref alt rsids
    exome { ac an af homozygote_count hemizygote_count filters }
    genome { ac an af homozygote_count hemizygote_count filters }
  }
}
"#;

const SEARCH_Q: &str = r#"
query VariantSearch($query: String!, $dataset: DatasetId!) {
  variant_search(query: $query, dataset: $dataset) { variant_id }
}
"#;

const GENE_VARIANTS_Q: &str = r#"
query GeneVariants($symbol: String, $geneId: String, $dataset: DatasetId!, $rg: ReferenceGenomeId!) {
  gene(gene_symbol: $symbol, gene_id: $geneId, reference_genome: $rg) {
    gene_id symbol chrom start stop
    variants(dataset: $dataset) {
      variant_id pos ref alt rsids
      exome { ac an af } genome { ac an af }
    }
  }
}
"#;

const CONSTRAINT_Q: &str = r#"
query GeneConstraint($symbol: String, $geneId: String) {
  gene(gene_symbol: $symbol, gene_id: $geneId, reference_genome: GRCh38) {
    gene_id symbol canonical_transcript_id chrom start stop strand
    gnomad_constraint {
      exp_lof obs_lof oe_lof oe_lof_lower oe_lof_upper
      exp_mis obs_mis oe_mis oe_mis_lower oe_mis_upper
      exp_syn obs_syn oe_syn oe_syn_lower oe_syn_upper
      pli lof_z mis_z syn_z
    }
  }
}
"#;

const REGION_Q: &str = r#"
query RegionVariants($chrom: String!, $start: Int!, $stop: Int!, $dataset: DatasetId!, $rg: ReferenceGenomeId!) {
  region(chrom: $chrom, start: $start, stop: $stop, reference_genome: $rg) {
    variants(dataset: $dataset) {
      variant_id pos ref alt rsids
      exome { ac an af } genome { ac an af }
    }
  }
}
"#;

const LIFTOVER_Q: &str = r#"
query Liftover($source: String!, $rg: ReferenceGenomeId!) {
  liftover(source_variant_id: $source, reference_genome: $rg) {
    source { variant_id reference_genome }
    liftover { variant_id reference_genome }
    datasets
  }
}
"#;

const CLINVAR_Q: &str = r#"
query ClinvarVariants($symbol: String, $geneId: String) {
  meta { clinvar_release_date }
  gene(gene_symbol: $symbol, gene_id: $geneId, reference_genome: GRCh38) {
    gene_id symbol
    clinvar_variants {
      variant_id clinvar_variation_id clinical_significance gold_stars
      review_status major_consequence pos transcript_id in_gnomad
    }
  }
}
"#;

const SV_GENE_Q: &str = r#"
query StructuralVariantsGene($symbol: String, $geneId: String, $dataset: StructuralVariantDatasetId!, $rg: ReferenceGenomeId!) {
  gene(gene_symbol: $symbol, gene_id: $geneId, reference_genome: $rg) {
    gene_id symbol
    structural_variants(dataset: $dataset) {
      variant_id major_consequence ac an af homozygote_count
      hemizygote_count chrom pos end chrom2 pos2 type length filters
    }
  }
}
"#;

const SV_Q: &str = r#"
query StructuralVariant($variantId: String!, $dataset: StructuralVariantDatasetId!) {
  structural_variant(variantId: $variantId, dataset: $dataset) {
    variant_id chrom pos end chrom2 pos2 type length ac an af
    homozygote_count hemizygote_count filters qual
    consequences { consequence genes }
    algorithms evidence
  }
}
"#;

const MITO_GENE_Q: &str = r#"
query MitochondrialVariantsGene($symbol: String, $geneId: String, $dataset: DatasetId!) {
  gene(gene_symbol: $symbol, gene_id: $geneId, reference_genome: GRCh38) {
    gene_id symbol
    mitochondrial_variants(dataset: $dataset) {
      variant_id pos ac_het ac_hom an max_heteroplasmy filters
    }
  }
}
"#;

const MITO_REGION_Q: &str = r#"
query MitochondrialVariantsRegion($start: Int!, $stop: Int!, $dataset: DatasetId!) {
  region(chrom: "M", start: $start, stop: $stop, reference_genome: GRCh38) {
    mitochondrial_variants(dataset: $dataset) {
      variant_id pos ac_het ac_hom an max_heteroplasmy filters
    }
  }
}
"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetVariant {
    variant_id: String,
    #[serde(default = "default_dataset")]
    dataset: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Search {
    query: String,
    #[serde(default = "default_dataset")]
    dataset: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GeneQuery {
    gene_symbol: Option<String>,
    gene_id: Option<String>,
    #[serde(default = "default_dataset")]
    dataset: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Constraint {
    gene_symbol: Option<String>,
    gene_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Region {
    chrom: String,
    start: i64,
    stop: i64,
    #[serde(default = "default_dataset")]
    dataset: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Liftover {
    variant_id: String,
    #[serde(default = "default_build")]
    source_build: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GeneOnly {
    gene_symbol: Option<String>,
    gene_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SvGene {
    gene_symbol: Option<String>,
    gene_id: Option<String>,
    #[serde(default = "default_sv")]
    dataset: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GetSv {
    sv_id: String,
    #[serde(default = "default_sv")]
    dataset: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mito {
    gene_symbol: Option<String>,
    gene_id: Option<String>,
    region_start: Option<i64>,
    region_stop: Option<i64>,
    #[serde(default = "default_dataset")]
    dataset: String,
}

fn default_dataset() -> String {
    "gnomad_r4".into()
}

fn default_sv() -> String {
    "gnomad_sv_r4".into()
}

fn default_build() -> String {
    "GRCh37".into()
}

pub(super) async fn get_variant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetVariant =
        serde_json::from_value(args.clone()).context("invalid get_variant arguments")?;
    let variant_id = require_variant_id(&args.variant_id)?;
    let dataset = require_dataset(&args.dataset)?;
    let data = graphql(
        bio,
        VARIANT_Q,
        json!({"variantId": variant_id, "dataset": dataset}),
    )
    .await?;
    let variant = match data {
        Gql::NotFound => None,
        Gql::Data(data) => data.get("variant").filter(|row| row.is_object()).cloned(),
    };
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "found": variant.is_some(),
        "variant_id": variant_id,
        "dataset": dataset,
        "variant": variant.as_ref().map(|row| project_variant(row, &dataset)),
        "url": gnomad_variant_url(&variant_id, &dataset)
    }))
}

pub(super) async fn search_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Search =
        serde_json::from_value(args.clone()).context("invalid search_variants arguments")?;
    let query = require_text(&args.query, 1, 128, "query")?;
    let dataset = require_dataset(&args.dataset)?;
    let data = match graphql(bio, SEARCH_Q, json!({"query": query, "dataset": dataset})).await? {
        Gql::NotFound => json!({"variant_search": []}),
        Gql::Data(data) => data,
    };
    let mut ids: Vec<String> = data
        .get("variant_search")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| json_string(&row["variant_id"]))
        .collect();
    ids.sort();
    ids.dedup();
    let n_matches = ids.len();
    let truncated = n_matches > super::SEARCH_CAP;
    if truncated {
        ids.truncate(super::SEARCH_CAP);
    }
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "query": query,
        "dataset": dataset,
        "n_matches": n_matches,
        "returned": ids.len(),
        "truncated": truncated,
        "variant_ids": ids
    }))
}

pub(super) async fn gene_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GeneQuery =
        serde_json::from_value(args.clone()).context("invalid gene_variants arguments")?;
    let (symbol, gene_id) = gene_args(&args.gene_symbol, &args.gene_id)?;
    let dataset = require_dataset(&args.dataset)?;
    let rg = reference_genome(&dataset);
    let gene = gene_data(
        bio,
        GENE_VARIANTS_Q,
        gene_vars(
            symbol.as_deref(),
            gene_id.as_deref(),
            json!({
                "dataset": dataset, "rg": rg
            }),
        ),
        symbol.as_deref(),
        gene_id.as_deref(),
    )
    .await?;
    let mut rows: Vec<Value> = gene
        .get("variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(project_short)
        .collect();
    sort_by_pos(&mut rows);
    let n_variants = rows.len();
    let (rows, truncated) = cap_rows(rows, LIST_CAP);
    let gene_id = json_string(&gene["gene_id"]).context("gnomAD gene omitted gene_id")?;
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "url": gnomad_gene_url(&gene_id, &dataset),
        "gene_id": gene_id,
        "symbol": gene.get("symbol"),
        "chrom": gene.get("chrom"),
        "start": gene.get("start"),
        "stop": gene.get("stop"),
        "dataset": dataset,
        "n_variants": n_variants,
        "returned": rows.len(),
        "truncated": truncated,
        "variants": rows
    }))
}

pub(super) async fn gene_constraint(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Constraint =
        serde_json::from_value(args.clone()).context("invalid gene_constraint arguments")?;
    let (symbol, gene_id) = gene_args(&args.gene_symbol, &args.gene_id)?;
    let gene = gene_data(
        bio,
        CONSTRAINT_Q,
        gene_vars(symbol.as_deref(), gene_id.as_deref(), json!({})),
        symbol.as_deref(),
        gene_id.as_deref(),
    )
    .await?;
    let constraint = gene
        .get("gnomad_constraint")
        .cloned()
        .unwrap_or(Value::Null);
    let constraint = if constraint.is_object() {
        let mut out = json!({});
        for key in [
            "exp_lof",
            "obs_lof",
            "oe_lof",
            "oe_lof_lower",
            "oe_lof_upper",
            "exp_mis",
            "obs_mis",
            "oe_mis",
            "oe_mis_lower",
            "oe_mis_upper",
            "exp_syn",
            "obs_syn",
            "oe_syn",
            "oe_syn_lower",
            "oe_syn_upper",
            "pli",
            "lof_z",
            "mis_z",
            "syn_z",
        ] {
            out[key] = constraint.get(key).cloned().unwrap_or(Value::Null);
        }
        out
    } else {
        Value::Null
    };
    let gene_id = json_string(&gene["gene_id"]).context("gnomAD gene omitted gene_id")?;
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "url": gnomad_gene_url(&gene_id, "gnomad_r4"),
        "gene_id": gene_id,
        "symbol": gene.get("symbol"),
        "canonical_transcript_id": gene.get("canonical_transcript_id"),
        "chrom": gene.get("chrom"),
        "start": gene.get("start"),
        "stop": gene.get("stop"),
        "strand": gene.get("strand"),
        "constraint": constraint
    }))
}

pub(super) async fn region_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Region =
        serde_json::from_value(args.clone()).context("invalid region_variants arguments")?;
    let chrom = super::normalize_chrom(&args.chrom, ChromKind::Nuclear)?;
    let (start, stop) = require_region(args.start, args.stop)?;
    let dataset = require_dataset(&args.dataset)?;
    let rg = reference_genome(&dataset);
    let data = match graphql(
        bio,
        REGION_Q,
        json!({"chrom": chrom, "start": start, "stop": stop, "dataset": dataset, "rg": rg}),
    )
    .await?
    {
        Gql::NotFound => json!({"region": {"variants": []}}),
        Gql::Data(data) => data,
    };
    let mut rows: Vec<Value> = data
        .pointer("/region/variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(project_short)
        .collect();
    sort_by_pos(&mut rows);
    let n_variants = rows.len();
    let (rows, truncated) = cap_rows(rows, LIST_CAP);
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "chrom": chrom,
        "start": start,
        "stop": stop,
        "dataset": dataset,
        "n_variants": n_variants,
        "returned": rows.len(),
        "truncated": truncated,
        "variants": rows
    }))
}

pub(super) async fn liftover_variant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Liftover =
        serde_json::from_value(args.clone()).context("invalid liftover_variant arguments")?;
    let variant_id = require_variant_id(&args.variant_id)?;
    let build = args.source_build.trim();
    if build != "GRCh37" && build != "GRCh38" {
        bail!("source_build must be GRCh37 or GRCh38");
    }
    let data = match graphql(bio, LIFTOVER_Q, json!({"source": variant_id, "rg": build})).await? {
        Gql::NotFound => json!({"liftover": []}),
        Gql::Data(data) => data,
    };
    let mut results: Vec<Value> = data
        .get("liftover")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let source = row.get("source")?.clone();
            let liftover = row.get("liftover")?.clone();
            Some(json!({
                "source": source,
                "liftover": liftover,
                "datasets": sorted_strings(row.get("datasets"))
            }))
        })
        .collect();
    results.sort_by(|a, b| {
        a.pointer("/liftover/variant_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(
                b.pointer("/liftover/variant_id")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
    });
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "source_variant_id": variant_id,
        "source_build": build,
        "n_results": results.len(),
        "results": results
    }))
}

pub(super) async fn clinvar_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GeneOnly =
        serde_json::from_value(args.clone()).context("invalid clinvar_variants arguments")?;
    let (symbol, gene_id) = gene_args(&args.gene_symbol, &args.gene_id)?;
    let data = match graphql(
        bio,
        CLINVAR_Q,
        gene_vars(symbol.as_deref(), gene_id.as_deref(), json!({})),
    )
    .await?
    {
        Gql::NotFound => bail!(
            "gnomAD has no gene matching {}",
            symbol
                .as_deref()
                .or(gene_id.as_deref())
                .unwrap_or("the query")
        ),
        Gql::Data(data) => data,
    };
    let gene = data
        .get("gene")
        .filter(|row| row.is_object())
        .cloned()
        .with_context(|| {
            format!(
                "gnomAD has no gene matching {}",
                symbol
                    .as_deref()
                    .or(gene_id.as_deref())
                    .unwrap_or("the query")
            )
        })?;
    let mut rows: Vec<Value> = gene
        .get("clinvar_variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let variant_id = json_string(&row["variant_id"])?;
            Some(json!({
                "variant_id": variant_id,
                "clinvar_variation_id": row.get("clinvar_variation_id"),
                "clinical_significance": row.get("clinical_significance"),
                "gold_stars": row.get("gold_stars"),
                "review_status": row.get("review_status"),
                "major_consequence": row.get("major_consequence"),
                "pos": row.get("pos"),
                "transcript_id": row.get("transcript_id"),
                "in_gnomad": row.get("in_gnomad")
            }))
        })
        .collect();
    sort_by_pos(&mut rows);
    let n_variants = rows.len();
    let (rows, truncated) = cap_rows(rows, LIST_CAP);
    let gene_id = json_string(&gene["gene_id"]).context("gnomAD gene omitted gene_id")?;
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "url": gnomad_gene_url(&gene_id, "gnomad_r4"),
        "gene_id": gene_id,
        "symbol": gene.get("symbol"),
        "clinvar_release_date": data.pointer("/meta/clinvar_release_date"),
        "n_variants": n_variants,
        "returned": rows.len(),
        "truncated": truncated,
        "variants": rows
    }))
}

pub(super) async fn structural_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SvGene =
        serde_json::from_value(args.clone()).context("invalid structural_variants arguments")?;
    let (symbol, gene_id) = gene_args(&args.gene_symbol, &args.gene_id)?;
    let dataset = require_sv_dataset(&args.dataset)?;
    let rg = reference_genome(&dataset);
    let gene = gene_data(
        bio,
        SV_GENE_Q,
        gene_vars(
            symbol.as_deref(),
            gene_id.as_deref(),
            json!({
                "dataset": dataset, "rg": rg
            }),
        ),
        symbol.as_deref(),
        gene_id.as_deref(),
    )
    .await?;
    let mut rows: Vec<Value> = gene
        .get("structural_variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(project_sv)
        .collect();
    rows.sort_by(|a, b| {
        a["variant_id"]
            .as_str()
            .unwrap_or("")
            .cmp(b["variant_id"].as_str().unwrap_or(""))
    });
    let n_variants = rows.len();
    let (rows, truncated) = cap_rows(rows, LIST_CAP);
    let gene_id = json_string(&gene["gene_id"]).context("gnomAD gene omitted gene_id")?;
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "url": gnomad_gene_url(&gene_id, &dataset),
        "gene_id": gene_id,
        "symbol": gene.get("symbol"),
        "dataset": dataset,
        "n_variants": n_variants,
        "returned": rows.len(),
        "truncated": truncated,
        "variants": rows
    }))
}

pub(super) async fn get_structural_variant(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GetSv =
        serde_json::from_value(args.clone()).context("invalid get_structural_variant arguments")?;
    let sv_id = require_text(&args.sv_id, 1, 128, "sv_id")?;
    if sv_id.chars().any(char::is_whitespace) {
        bail!("sv_id must not contain whitespace");
    }
    let dataset = require_sv_dataset(&args.dataset)?;
    let row = match graphql(bio, SV_Q, json!({"variantId": sv_id, "dataset": dataset})).await? {
        Gql::NotFound => None,
        Gql::Data(data) => data
            .get("structural_variant")
            .filter(|row| row.is_object())
            .and_then(project_sv)
            .map(|mut row| {
                row["dataset"] = json!(dataset);
                row
            }),
    };
    Ok(json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "found": row.is_some(),
        "sv_id": sv_id,
        "dataset": dataset,
        "structural_variant": row,
        "url": format!("{GNOMAD_BROWSER}/variant/{sv_id}?dataset={dataset}")
    }))
}

pub(super) async fn mitochondrial_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: Mito =
        serde_json::from_value(args.clone()).context("invalid mitochondrial_variants arguments")?;
    let dataset = require_dataset(&args.dataset)?;
    let has_gene = args.gene_symbol.is_some() || args.gene_id.is_some();
    let has_region = args.region_start.is_some() || args.region_stop.is_some();
    if has_gene == has_region {
        bail!("pass a mitochondrial gene (symbol or ID) or region_start+region_stop, not both");
    }
    if has_region {
        let start = args
            .region_start
            .context("pass region_start and region_stop together")?;
        let stop = args
            .region_stop
            .context("pass region_start and region_stop together")?;
        let (start, stop) = require_region(start, stop)?;
        let data = match graphql(
            bio,
            MITO_REGION_Q,
            json!({"start": start, "stop": stop, "dataset": dataset}),
        )
        .await?
        {
            Gql::NotFound => json!({"region": {"mitochondrial_variants": []}}),
            Gql::Data(data) => data,
        };
        return mito_page(
            data.pointer("/region/mitochondrial_variants"),
            dataset,
            json!({"region": format!("M:{start}-{stop}"), "chrom": "M", "start": start, "stop": stop}),
        );
    }
    let (symbol, gene_id) = gene_args(&args.gene_symbol, &args.gene_id)?;
    let gene = gene_data(
        bio,
        MITO_GENE_Q,
        gene_vars(
            symbol.as_deref(),
            gene_id.as_deref(),
            json!({"dataset": dataset}),
        ),
        symbol.as_deref(),
        gene_id.as_deref(),
    )
    .await?;
    let gene_id = json_string(&gene["gene_id"]).context("gnomAD gene omitted gene_id")?;
    mito_page(
        gene.get("mitochondrial_variants"),
        dataset,
        json!({
            "gene_id": gene_id,
            "symbol": gene.get("symbol"),
            "url": gnomad_gene_url(&gene_id, "gnomad_r4")
        }),
    )
}

fn mito_page(rows: Option<&Value>, dataset: String, scope: Value) -> Result<Value> {
    let mut variants: Vec<Value> = rows
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(project_mito)
        .collect();
    sort_by_pos(&mut variants);
    let n_variants = variants.len();
    let (variants, truncated) = cap_rows(variants, LIST_CAP);
    let mut out = json!({
        "source": "gnomAD",
        "source_url": GNOMAD_API,
        "dataset": dataset,
        "n_variants": n_variants,
        "returned": variants.len(),
        "truncated": truncated,
        "variants": variants
    });
    if let Value::Object(map) = scope {
        for (key, value) in map {
            out[key] = value;
        }
    }
    Ok(out)
}

async fn gene_data(
    bio: &NativeBio,
    query: &str,
    variables: Value,
    symbol: Option<&str>,
    gene_id: Option<&str>,
) -> Result<Value> {
    let label = symbol.or(gene_id).unwrap_or("the query");
    match graphql(bio, query, variables).await? {
        Gql::NotFound => bail!("gnomAD has no gene matching {label}"),
        Gql::Data(data) => data
            .get("gene")
            .filter(|row| row.is_object())
            .cloned()
            .with_context(|| format!("gnomAD has no gene matching {label}")),
    }
}

fn gene_vars(symbol: Option<&str>, gene_id: Option<&str>, mut extra: Value) -> Value {
    if let Some(symbol) = symbol {
        extra["symbol"] = json!(symbol);
    }
    if let Some(gene_id) = gene_id {
        extra["geneId"] = json!(gene_id);
    }
    extra
}

fn project_variant(row: &Value, dataset: &str) -> Value {
    json!({
        "variant_id": row.get("variant_id"),
        "dataset": dataset,
        "reference_genome": row.get("reference_genome"),
        "chrom": row.get("chrom"),
        "pos": row.get("pos"),
        "ref": row.get("ref"),
        "alt": row.get("alt"),
        "rsids": sorted_strings(row.get("rsids")),
        "exome": freq_block(row.get("exome"), true),
        "genome": freq_block(row.get("genome"), true),
        "url": json_string(&row["variant_id"]).map(|id| gnomad_variant_url(&id, dataset))
    })
}

fn project_short(row: &Value) -> Option<Value> {
    let variant_id = json_string(&row["variant_id"])?;
    Some(json!({
        "variant_id": variant_id,
        "pos": row.get("pos"),
        "ref": row.get("ref"),
        "alt": row.get("alt"),
        "rsids": sorted_strings(row.get("rsids")),
        "exome": freq_block(row.get("exome"), false),
        "genome": freq_block(row.get("genome"), false)
    }))
}

fn project_sv(row: &Value) -> Option<Value> {
    let variant_id = json_string(&row["variant_id"])?;
    let mut out = json!({
        "variant_id": variant_id,
        "major_consequence": row.get("major_consequence"),
        "ac": row.get("ac"),
        "an": row.get("an"),
        "af": row.get("af"),
        "homozygote_count": row.get("homozygote_count"),
        "hemizygote_count": row.get("hemizygote_count"),
        "chrom": row.get("chrom"),
        "pos": row.get("pos"),
        "end": row.get("end"),
        "chrom2": row.get("chrom2"),
        "pos2": row.get("pos2"),
        "type": row.get("type"),
        "length": row.get("length"),
        "filters": sorted_strings(row.get("filters"))
    });
    if let Some(qual) = row.get("qual") {
        out["qual"] = qual.clone();
    }
    if let Some(algorithms) = row.get("algorithms") {
        out["algorithms"] = sorted_strings(Some(algorithms));
    }
    if let Some(evidence) = row.get("evidence") {
        out["evidence"] = sorted_strings(Some(evidence));
    }
    if let Some(consequences) = row.get("consequences").and_then(Value::as_array) {
        let mut rows: Vec<Value> = consequences
            .iter()
            .map(|item| {
                json!({
                    "consequence": item.get("consequence"),
                    "genes": sorted_strings(item.get("genes"))
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            a["consequence"]
                .as_str()
                .unwrap_or("")
                .cmp(b["consequence"].as_str().unwrap_or(""))
        });
        out["consequences"] = json!(rows);
    }
    Some(out)
}

fn project_mito(row: &Value) -> Option<Value> {
    let variant_id = json_string(&row["variant_id"])?;
    Some(json!({
        "variant_id": variant_id,
        "pos": row.get("pos"),
        "ac_het": row.get("ac_het"),
        "ac_hom": row.get("ac_hom"),
        "an": row.get("an"),
        "max_heteroplasmy": row.get("max_heteroplasmy"),
        "filters": sorted_strings(row.get("filters"))
    }))
}

fn freq_block(value: Option<&Value>, detailed: bool) -> Value {
    let Some(value) = value.filter(|row| row.is_object()) else {
        return Value::Null;
    };
    let mut out = json!({
        "ac": value.get("ac"),
        "an": value.get("an"),
        "af": value.get("af")
    });
    if detailed {
        out["homozygote_count"] = value
            .get("homozygote_count")
            .cloned()
            .unwrap_or(Value::Null);
        out["hemizygote_count"] = value
            .get("hemizygote_count")
            .cloned()
            .unwrap_or(Value::Null);
        out["filters"] = sorted_strings(value.get("filters"));
    }
    out
}
