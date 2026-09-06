//! CIViC GraphQL retrieval. Endpoint POST https://civicdb.org/api/graphql.
use super::{
    bound_page, civic_endpoint, connection, graphql, node, page, require_id, require_text, tool,
    with_civic_url, CIVIC, CIVIC_GRAPHQL, CIVIC_MAX_PAGE, MAX_TEXT,
};
use crate::NativeBio;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use wisp_llm::ToolSchema;

const GENE_FIELDS: &str = "id name entrezId fullName featureAliases description link";
const VARIANT_FIELDS: &str = "id name link variantAliases variantTypes { id name soid } feature { id name } singleVariantMolecularProfileId ... on GeneVariant { alleleRegistryId clinvarIds hgvsDescriptions coordinates { chromosome start stop referenceBases variantBases referenceBuild representativeTranscript } }";
const EVIDENCE_FIELDS: &str = "id name status evidenceLevel evidenceType evidenceDirection significance evidenceRating variantOrigin therapyInteractionType description link disease { id name doid displayName } therapies { id name ncitId } molecularProfile { id name } source { id sourceType citationId citation } phenotypes { id hpoId name }";
const ASSERTION_FIELDS: &str = "id name status assertionType assertionDirection significance ampLevel summary description link variantOrigin therapyInteractionType regulatoryApproval fdaCompanionTest nccnGuideline { id name } nccnGuidelineVersion acmgCodes { id code } clingenCodes { id code } disease { id name doid displayName } therapies { id name ncitId } molecularProfile { id name } phenotypes { id hpoId name } evidenceItemsCount";
const MP_FIELDS: &str = "id name rawName link description molecularProfileScore isComplex isMultiVariant molecularProfileAliases variants { id name feature { id name } } evidenceCountsByStatus { acceptedCount submittedCount rejectedCount }";
const DISEASE_FIELDS: &str = "id name displayName doid diseaseUrl diseaseAliases link";
const THERAPY_FIELDS: &str = "id name ncitId therapyUrl therapyAliases link";

const EVIDENCE_LEVELS: &[&str] = &["A", "B", "C", "D", "E"];
const EVIDENCE_TYPES: &[&str] = &[
    "PREDICTIVE",
    "PROGNOSTIC",
    "DIAGNOSTIC",
    "PREDISPOSING",
    "ONCOGENIC",
    "FUNCTIONAL",
];
const ASSERTION_TYPES: &[&str] = &[
    "PREDICTIVE",
    "PROGNOSTIC",
    "DIAGNOSTIC",
    "PREDISPOSING",
    "ONCOGENIC",
];
const DIRECTIONS: &[&str] = &["SUPPORTS", "DOES_NOT_SUPPORT", "NA"];
const EVIDENCE_SIGNIFICANCE: &[&str] = &[
    "SENSITIVITYRESPONSE",
    "RESISTANCE",
    "ADVERSE_RESPONSE",
    "REDUCED_SENSITIVITY",
    "GAIN_OF_FUNCTION",
    "LOSS_OF_FUNCTION",
    "UNALTERED_FUNCTION",
    "NEOMORPHIC",
    "DOMINANT_NEGATIVE",
    "BETTER_OUTCOME",
    "POOR_OUTCOME",
    "POSITIVE",
    "NEGATIVE",
    "NA",
    "PATHOGENIC",
    "LIKELY_PATHOGENIC",
    "BENIGN",
    "LIKELY_BENIGN",
    "UNCERTAIN_SIGNIFICANCE",
    "PREDISPOSITION",
    "PROTECTIVENESS",
    "ONCOGENICITY",
    "LIKELY_ONCOGENIC",
    "UNKNOWN",
];
const ASSERTION_SIGNIFICANCE: &[&str] = &[
    "SENSITIVITYRESPONSE",
    "RESISTANCE",
    "ADVERSE_RESPONSE",
    "REDUCED_SENSITIVITY",
    "BETTER_OUTCOME",
    "POOR_OUTCOME",
    "POSITIVE",
    "NEGATIVE",
    "NA",
    "PATHOGENIC",
    "LIKELY_PATHOGENIC",
    "BENIGN",
    "LIKELY_BENIGN",
    "UNCERTAIN_SIGNIFICANCE",
    "ONCOGENIC",
    "LIKELY_ONCOGENIC",
];
const ORIGINS: &[&str] = &[
    "SOMATIC",
    "RARE_GERMLINE",
    "COMMON_GERMLINE",
    "COMBINED",
    "MIXED",
    "UNKNOWN",
    "NA",
];
const STATUSES: &[&str] = &["ACCEPTED", "SUBMITTED", "REJECTED", "NON_REJECTED", "ALL"];
const AMP_LEVELS: &[&str] = &[
    "TIER_I_LEVEL_A",
    "TIER_I_LEVEL_B",
    "TIER_II_LEVEL_C",
    "TIER_II_LEVEL_D",
    "TIER_III",
    "TIER_IV",
    "NA",
];

pub fn catalog() -> Vec<(&'static str, ToolSchema)> {
    vec![
        tool(
            "civic_gene_variants",
            "List CIViC variants for one gene by internal CIViC gene id via GraphQL variants(geneId). Returns a bounded page of variant ids, names, feature linkage, aliases and source URLs. total_count is the upstream connection size; a capped page is not the complete variant set.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["gene_id"],
                "properties": {
                    "gene_id": {"type": "integer", "minimum": 1},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "civic_get_assertion",
            "Retrieve one CIViC assertion by internal id: AMP/ASCO/CAP tier, significance, disease, therapies and linked molecular profile. found is false when the id is absent. Read-only.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["assertion_id"],
                "properties": {
                    "assertion_id": {"type": "integer", "minimum": 1}
                }
            }),
        ),
        tool(
            "civic_get_evidence_item",
            "Retrieve one CIViC evidence item by internal id: evidence level A–E, type, direction, significance, disease, therapies, molecular profile and citation. found is false when the id is absent.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["evidence_id"],
                "properties": {
                    "evidence_id": {"type": "integer", "minimum": 1}
                }
            }),
        ),
        tool(
            "civic_get_molecular_profile",
            "Retrieve one CIViC molecular profile by internal id, including parsed name and component variant ids. found is false when the id is absent.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["mp_id"],
                "properties": {
                    "mp_id": {"type": "integer", "minimum": 1}
                }
            }),
        ),
        tool(
            "civic_get_variant",
            "Retrieve one CIViC variant by internal id, including aliases, variant types and gene/feature linkage. Gene variants may include ClinVar ids, HGVS and coordinates. found is false when the id is absent.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["variant_id"],
                "properties": {
                    "variant_id": {"type": "integer", "minimum": 1}
                }
            }),
        ),
        tool(
            "civic_search_assertions",
            "Search CIViC assertions through the GraphQL assertions connection. Name filters are substrings; id filters are exact. Enums are CIViC GraphQL values (assertion_type PREDICTIVE|PROGNOSTIC|DIAGNOSTIC|PREDISPOSING|ONCOGENIC; amp_level e.g. TIER_I_LEVEL_A). Returns a bounded page; total_count is the unpaged hit count.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "disease_name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "therapy_name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "assertion_type": {"type": "string", "enum": ["PREDICTIVE", "PROGNOSTIC", "DIAGNOSTIC", "PREDISPOSING", "ONCOGENIC"]},
                    "assertion_direction": {"type": "string", "enum": ["SUPPORTS", "DOES_NOT_SUPPORT", "NA"]},
                    "significance": {"type": "string", "enum": ["SENSITIVITYRESPONSE", "RESISTANCE", "ADVERSE_RESPONSE", "REDUCED_SENSITIVITY", "BETTER_OUTCOME", "POOR_OUTCOME", "POSITIVE", "NEGATIVE", "NA", "PATHOGENIC", "LIKELY_PATHOGENIC", "BENIGN", "LIKELY_BENIGN", "UNCERTAIN_SIGNIFICANCE", "ONCOGENIC", "LIKELY_ONCOGENIC"]},
                    "amp_level": {"type": "string", "enum": ["TIER_I_LEVEL_A", "TIER_I_LEVEL_B", "TIER_II_LEVEL_C", "TIER_II_LEVEL_D", "TIER_III", "TIER_IV", "NA"]},
                    "status": {"type": "string", "enum": ["ACCEPTED", "SUBMITTED", "REJECTED", "NON_REJECTED", "ALL"]},
                    "molecular_profile_name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "molecular_profile_id": {"type": "integer", "minimum": 1},
                    "variant_id": {"type": "integer", "minimum": 1},
                    "variant_name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "disease_id": {"type": "integer", "minimum": 1},
                    "therapy_id": {"type": "integer", "minimum": 1},
                    "phenotype_id": {"type": "integer", "minimum": 1},
                    "evidence_id": {"type": "integer", "minimum": 1},
                    "summary": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "civic_search_diseases",
            "Search CIViC disease records by name substring through GraphQL diseases(name). Returns a bounded page of CIViC ids, display names, DOIDs and source URLs.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "civic_search_evidence",
            "Search CIViC evidence items through the GraphQL evidenceItems connection. Name filters are substrings; id filters are exact. Enums are CIViC GraphQL values (evidence_level A–E; evidence_type PREDICTIVE|PROGNOSTIC|DIAGNOSTIC|PREDISPOSING|ONCOGENIC|FUNCTIONAL; status ACCEPTED|SUBMITTED|REJECTED|ALL). Returns a bounded page; total_count is the unpaged hit count.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "disease_name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "therapy_name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "evidence_level": {"type": "string", "enum": ["A", "B", "C", "D", "E"]},
                    "evidence_type": {"type": "string", "enum": ["PREDICTIVE", "PROGNOSTIC", "DIAGNOSTIC", "PREDISPOSING", "ONCOGENIC", "FUNCTIONAL"]},
                    "evidence_direction": {"type": "string", "enum": ["SUPPORTS", "DOES_NOT_SUPPORT", "NA"]},
                    "significance": {"type": "string", "enum": ["SENSITIVITYRESPONSE", "RESISTANCE", "ADVERSE_RESPONSE", "REDUCED_SENSITIVITY", "GAIN_OF_FUNCTION", "LOSS_OF_FUNCTION", "UNALTERED_FUNCTION", "NEOMORPHIC", "DOMINANT_NEGATIVE", "BETTER_OUTCOME", "POOR_OUTCOME", "POSITIVE", "NEGATIVE", "NA", "PATHOGENIC", "LIKELY_PATHOGENIC", "BENIGN", "LIKELY_BENIGN", "UNCERTAIN_SIGNIFICANCE", "PREDISPOSITION", "PROTECTIVENESS", "ONCOGENICITY", "LIKELY_ONCOGENIC", "UNKNOWN"]},
                    "variant_origin": {"type": "string", "enum": ["SOMATIC", "RARE_GERMLINE", "COMMON_GERMLINE", "COMBINED", "MIXED", "UNKNOWN", "NA"]},
                    "evidence_rating": {"type": "integer", "minimum": 1, "maximum": 5},
                    "status": {"type": "string", "enum": ["ACCEPTED", "SUBMITTED", "REJECTED", "NON_REJECTED", "ALL"]},
                    "molecular_profile_name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "molecular_profile_id": {"type": "integer", "minimum": 1},
                    "variant_id": {"type": "integer", "minimum": 1},
                    "disease_id": {"type": "integer", "minimum": 1},
                    "therapy_id": {"type": "integer", "minimum": 1},
                    "phenotype_id": {"type": "integer", "minimum": 1},
                    "source_id": {"type": "integer", "minimum": 1},
                    "assertion_id": {"type": "integer", "minimum": 1},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "civic_search_genes",
            "Find CIViC gene records by exact Entrez symbol through GraphQL genes(entrezSymbols). Returns a bounded page of CIViC gene ids, Entrez ids, aliases and source URLs. Use the CIViC gene id with civic_gene_variants.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["entrez_symbol"],
                "properties": {
                    "entrez_symbol": {"type": "string", "minLength": 1, "maxLength": 64},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "civic_search_molecular_profiles",
            "Search CIViC molecular profiles by name substring through GraphQL molecularProfiles(name). Returns a bounded page of profile ids, names, component variants and source URLs.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "civic_search_therapies",
            "Search CIViC therapy records by name substring through GraphQL therapies(name). Returns a bounded page of CIViC ids, NCIt identifiers and source URLs.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
        tool(
            "civic_search_variants",
            "Search CIViC variants by name substring through GraphQL variants(name), optionally scoped to a CIViC gene id. Returns a bounded page of variant ids, names, feature linkage and source URLs.",
            json!({
                "type": "object", "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": 256},
                    "gene_id": {"type": "integer", "minimum": 1},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25}
                }
            }),
        ),
    ]
}

pub async fn call(bio: &NativeBio, name: &str, args: &Value) -> Result<Value> {
    match name {
        "civic_gene_variants" => gene_variants(bio, args).await,
        "civic_get_assertion" => {
            get_named(bio, args, "assertion_id", "assertion", ASSERTION_FIELDS).await
        }
        "civic_get_evidence_item" => {
            get_named(bio, args, "evidence_id", "evidenceItem", EVIDENCE_FIELDS).await
        }
        "civic_get_molecular_profile" => {
            get_named(bio, args, "mp_id", "molecularProfile", MP_FIELDS).await
        }
        "civic_get_variant" => get_named(bio, args, "variant_id", "variant", VARIANT_FIELDS).await,
        "civic_search_assertions" => search_assertions(bio, args).await,
        "civic_search_diseases" => {
            search_named(bio, args, "CivicDiseases", "diseases", DISEASE_FIELDS).await
        }
        "civic_search_evidence" => search_evidence(bio, args).await,
        "civic_search_genes" => search_genes(bio, args).await,
        "civic_search_molecular_profiles" => {
            search_named(
                bio,
                args,
                "CivicMolecularProfiles",
                "molecularProfiles",
                MP_FIELDS,
            )
            .await
        }
        "civic_search_therapies" => {
            search_named(bio, args, "CivicTherapies", "therapies", THERAPY_FIELDS).await
        }
        "civic_search_variants" => search_variants(bio, args).await,
        _ => bail!("unknown native biological tool: {name}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneVariants {
    gene_id: i64,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchName {
    name: String,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchVariants {
    name: String,
    #[serde(default)]
    gene_id: Option<i64>,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchGenes {
    entrez_symbol: String,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchEvidence {
    disease_name: Option<String>,
    therapy_name: Option<String>,
    evidence_level: Option<String>,
    evidence_type: Option<String>,
    evidence_direction: Option<String>,
    significance: Option<String>,
    variant_origin: Option<String>,
    evidence_rating: Option<i64>,
    status: Option<String>,
    molecular_profile_name: Option<String>,
    molecular_profile_id: Option<i64>,
    variant_id: Option<i64>,
    disease_id: Option<i64>,
    therapy_id: Option<i64>,
    phenotype_id: Option<i64>,
    source_id: Option<i64>,
    assertion_id: Option<i64>,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchAssertions {
    disease_name: Option<String>,
    therapy_name: Option<String>,
    assertion_type: Option<String>,
    assertion_direction: Option<String>,
    significance: Option<String>,
    amp_level: Option<String>,
    status: Option<String>,
    molecular_profile_name: Option<String>,
    molecular_profile_id: Option<i64>,
    variant_id: Option<i64>,
    variant_name: Option<String>,
    disease_id: Option<i64>,
    therapy_id: Option<i64>,
    phenotype_id: Option<i64>,
    evidence_id: Option<i64>,
    summary: Option<String>,
    #[serde(default = "super::default_page")]
    max_results: u32,
}

async fn search_genes(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchGenes =
        serde_json::from_value(args.clone()).context("invalid CIViC gene search arguments")?;
    let symbol = super::require_symbol(&args.entrez_symbol, "entrez_symbol")?;
    let cap = bound_page(args.max_results, CIVIC_MAX_PAGE)?;
    let query = format!(
        "query CivicGenes($first: Int!, $entrezSymbols: [String!]!) {{ genes(first: $first, entrezSymbols: $entrezSymbols) {{ totalCount pageInfo {{ hasNextPage }} nodes {{ {GENE_FIELDS} }} }} }}"
    );
    civic_page(
        bio,
        &query,
        json!({"first": cap, "entrezSymbols": [symbol.clone()]}),
        "genes",
        json!({"entrez_symbol": symbol, "max_results": cap}),
        cap,
    )
    .await
}

async fn gene_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: GeneVariants =
        serde_json::from_value(args.clone()).context("invalid CIViC gene variant arguments")?;
    let gene_id = require_id(args.gene_id, "gene_id")?;
    let cap = bound_page(args.max_results, CIVIC_MAX_PAGE)?;
    let query = format!(
        "query CivicGeneVariants($first: Int!, $geneId: Int) {{ variants(first: $first, geneId: $geneId) {{ totalCount pageInfo {{ hasNextPage }} nodes {{ {VARIANT_FIELDS} }} }} }}"
    );
    civic_page(
        bio,
        &query,
        json!({"first": cap, "geneId": gene_id}),
        "variants",
        json!({"gene_id": gene_id, "max_results": cap}),
        cap,
    )
    .await
}

async fn search_variants(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchVariants =
        serde_json::from_value(args.clone()).context("invalid CIViC variant search arguments")?;
    let name = require_text(&args.name, "name", MAX_TEXT)?;
    let cap = bound_page(args.max_results, CIVIC_MAX_PAGE)?;
    let mut filters = vec![Filter {
        gql: "name",
        ty: "String",
        value: json!(name),
    }];
    let mut query_echo = json!({"name": name, "max_results": cap});
    if let Some(gene_id) = args.gene_id {
        let gene_id = require_id(gene_id, "gene_id")?;
        filters.push(Filter {
            gql: "geneId",
            ty: "Int",
            value: json!(gene_id),
        });
        query_echo["gene_id"] = json!(gene_id);
    }
    filtered_page(
        bio,
        "CivicVariants",
        "variants",
        VARIANT_FIELDS,
        filters,
        None,
        query_echo,
        cap,
    )
    .await
}

async fn search_named(
    bio: &NativeBio,
    args: &Value,
    operation: &str,
    field: &str,
    node_fields: &str,
) -> Result<Value> {
    let args: SearchName =
        serde_json::from_value(args.clone()).context("invalid CIViC search arguments")?;
    let name = require_text(&args.name, "name", MAX_TEXT)?;
    let cap = bound_page(args.max_results, CIVIC_MAX_PAGE)?;
    filtered_page(
        bio,
        operation,
        field,
        node_fields,
        vec![Filter {
            gql: "name",
            ty: "String",
            value: json!(name),
        }],
        None,
        json!({"name": name, "max_results": cap}),
        cap,
    )
    .await
}

async fn search_evidence(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchEvidence =
        serde_json::from_value(args.clone()).context("invalid CIViC evidence search arguments")?;
    let cap = bound_page(args.max_results, CIVIC_MAX_PAGE)?;
    let mut filters = Vec::new();
    let mut echo = Map::new();
    push_text(
        &mut filters,
        &mut echo,
        "diseaseName",
        "disease_name",
        args.disease_name,
    )?;
    push_text(
        &mut filters,
        &mut echo,
        "therapyName",
        "therapy_name",
        args.therapy_name,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "evidenceLevel",
        "EvidenceLevel",
        "evidence_level",
        args.evidence_level,
        EVIDENCE_LEVELS,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "evidenceType",
        "EvidenceType",
        "evidence_type",
        args.evidence_type,
        EVIDENCE_TYPES,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "evidenceDirection",
        "EvidenceDirection",
        "evidence_direction",
        args.evidence_direction,
        DIRECTIONS,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "significance",
        "EvidenceSignificance",
        "significance",
        args.significance,
        EVIDENCE_SIGNIFICANCE,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "variantOrigin",
        "VariantOrigin",
        "variant_origin",
        args.variant_origin,
        ORIGINS,
    )?;
    if let Some(rating) = args.evidence_rating {
        if !(1..=5).contains(&rating) {
            bail!("evidence_rating must be between 1 and 5");
        }
        filters.push(Filter {
            gql: "evidenceRating",
            ty: "Int",
            value: json!(rating),
        });
        echo.insert("evidence_rating".into(), json!(rating));
    }
    push_enum(
        &mut filters,
        &mut echo,
        "status",
        "EvidenceStatusFilter",
        "status",
        args.status,
        STATUSES,
    )?;
    push_text(
        &mut filters,
        &mut echo,
        "molecularProfileName",
        "molecular_profile_name",
        args.molecular_profile_name,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "molecularProfileId",
        "molecular_profile_id",
        args.molecular_profile_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "variantId",
        "variant_id",
        args.variant_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "diseaseId",
        "disease_id",
        args.disease_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "therapyId",
        "therapy_id",
        args.therapy_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "phenotypeId",
        "phenotype_id",
        args.phenotype_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "sourceId",
        "source_id",
        args.source_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "assertionId",
        "assertion_id",
        args.assertion_id,
    )?;
    echo.insert("max_results".into(), json!(cap));
    filtered_page(
        bio,
        "CivicEvidenceItems",
        "evidenceItems",
        EVIDENCE_FIELDS,
        filters,
        Some((
            "$sortBy: EvidenceSort",
            "sortBy: $sortBy",
            json!({"column": "ID", "direction": "ASC"}),
        )),
        Value::Object(echo),
        cap,
    )
    .await
}

async fn search_assertions(bio: &NativeBio, args: &Value) -> Result<Value> {
    let args: SearchAssertions =
        serde_json::from_value(args.clone()).context("invalid CIViC assertion search arguments")?;
    let cap = bound_page(args.max_results, CIVIC_MAX_PAGE)?;
    let mut filters = Vec::new();
    let mut echo = Map::new();
    push_text(
        &mut filters,
        &mut echo,
        "diseaseName",
        "disease_name",
        args.disease_name,
    )?;
    push_text(
        &mut filters,
        &mut echo,
        "therapyName",
        "therapy_name",
        args.therapy_name,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "assertionType",
        "EvidenceType",
        "assertion_type",
        args.assertion_type,
        ASSERTION_TYPES,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "assertionDirection",
        "EvidenceDirection",
        "assertion_direction",
        args.assertion_direction,
        DIRECTIONS,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "significance",
        "AssertionSignificance",
        "significance",
        args.significance,
        ASSERTION_SIGNIFICANCE,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "ampLevel",
        "AmpLevel",
        "amp_level",
        args.amp_level,
        AMP_LEVELS,
    )?;
    push_enum(
        &mut filters,
        &mut echo,
        "status",
        "EvidenceStatusFilter",
        "status",
        args.status,
        STATUSES,
    )?;
    push_text(
        &mut filters,
        &mut echo,
        "molecularProfileName",
        "molecular_profile_name",
        args.molecular_profile_name,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "molecularProfileId",
        "molecular_profile_id",
        args.molecular_profile_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "variantId",
        "variant_id",
        args.variant_id,
    )?;
    push_text(
        &mut filters,
        &mut echo,
        "variantName",
        "variant_name",
        args.variant_name,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "diseaseId",
        "disease_id",
        args.disease_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "therapyId",
        "therapy_id",
        args.therapy_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "phenotypeId",
        "phenotype_id",
        args.phenotype_id,
    )?;
    push_id(
        &mut filters,
        &mut echo,
        "evidenceId",
        "evidence_id",
        args.evidence_id,
    )?;
    push_text(&mut filters, &mut echo, "summary", "summary", args.summary)?;
    echo.insert("max_results".into(), json!(cap));
    filtered_page(
        bio,
        "CivicAssertions",
        "assertions",
        ASSERTION_FIELDS,
        filters,
        Some((
            "$sortBy: AssertionSort",
            "sortBy: $sortBy",
            json!({"column": "ID", "direction": "ASC"}),
        )),
        Value::Object(echo),
        cap,
    )
    .await
}

async fn get_named(
    bio: &NativeBio,
    args: &Value,
    arg_key: &str,
    field: &str,
    node_fields: &str,
) -> Result<Value> {
    let id = take_id(args, arg_key)?;
    let query = format!("query CivicGet($id: Int!) {{ {field}(id: $id) {{ {node_fields} }} }}");
    let payload = civic_graphql(bio, &query, json!({"id": id})).await?;
    let record = node(&payload, field, "CIViC")?.map(with_civic_url);
    let mut result = json!({
        "source": "CIViC",
        "source_url": CIVIC_GRAPHQL,
        "found": record.is_some(),
        "record": record
    });
    result["query"] = json!({});
    result["query"][arg_key] = json!(id);
    Ok(result)
}

fn take_id(args: &Value, key: &str) -> Result<i32> {
    let mut map: Map<String, Value> =
        serde_json::from_value(args.clone()).context("invalid CIViC identifier arguments")?;
    let value = map.remove(key).with_context(|| format!("missing {key}"))?;
    if !map.is_empty() {
        bail!("unexpected fields in CIViC identifier arguments");
    }
    let id = value
        .as_i64()
        .with_context(|| format!("{key} must be an integer"))?;
    require_id(id, key)
}

struct Filter {
    gql: &'static str,
    ty: &'static str,
    value: Value,
}

async fn filtered_page(
    bio: &NativeBio,
    operation: &str,
    field: &str,
    node_fields: &str,
    filters: Vec<Filter>,
    sort: Option<(&'static str, &'static str, Value)>,
    query_echo: Value,
    cap: u32,
) -> Result<Value> {
    let mut decls = String::from("$first: Int!");
    let mut args = String::from("first: $first");
    let mut variables = json!({"first": cap});
    for filter in &filters {
        decls.push_str(&format!(", ${}: {}", filter.gql, filter.ty));
        args.push_str(&format!(", {0}: ${0}", filter.gql));
        variables[filter.gql] = filter.value.clone();
    }
    if let Some((decl, arg, value)) = sort {
        decls.push_str(", ");
        decls.push_str(decl);
        args.push_str(", ");
        args.push_str(arg);
        variables["sortBy"] = value;
    }
    let query = format!(
        "query {operation}({decls}) {{ {field}({args}) {{ totalCount pageInfo {{ hasNextPage }} nodes {{ {node_fields} }} }} }}"
    );
    civic_page(bio, &query, variables, field, query_echo, cap).await
}

async fn civic_page(
    bio: &NativeBio,
    query: &str,
    variables: Value,
    field: &str,
    query_echo: Value,
    cap: u32,
) -> Result<Value> {
    let payload = civic_graphql(bio, query, variables).await?;
    let (records, total, has_more) = connection(&payload, field, "CIViC")?;
    let records: Vec<Value> = records.into_iter().map(with_civic_url).collect();
    Ok(page(
        "CIViC",
        CIVIC_GRAPHQL,
        query_echo,
        records,
        total,
        cap,
        has_more,
    ))
}

async fn civic_graphql(bio: &NativeBio, query: &str, variables: Value) -> Result<Value> {
    graphql(
        bio,
        CIVIC,
        &civic_endpoint(bio),
        query,
        variables,
        bio.credential("CIVIC_API_KEY"),
        true,
    )
    .await
}

fn push_text(
    filters: &mut Vec<Filter>,
    echo: &mut Map<String, Value>,
    gql: &'static str,
    key: &str,
    value: Option<String>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let text = require_text(&value, key, MAX_TEXT)?;
    filters.push(Filter {
        gql,
        ty: "String",
        value: json!(text),
    });
    echo.insert(key.to_string(), json!(text));
    Ok(())
}

fn push_enum(
    filters: &mut Vec<Filter>,
    echo: &mut Map<String, Value>,
    gql: &'static str,
    ty: &'static str,
    key: &str,
    value: Option<String>,
    allowed: &[&str],
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let text = require_text(&value, key, 64)?;
    if !allowed.iter().any(|item| *item == text) {
        bail!("{key} must be one of {}", allowed.join(", "));
    }
    filters.push(Filter {
        gql,
        ty,
        value: json!(text),
    });
    echo.insert(key.to_string(), json!(text));
    Ok(())
}

fn push_id(
    filters: &mut Vec<Filter>,
    echo: &mut Map<String, Value>,
    gql: &'static str,
    key: &str,
    value: Option<i64>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let id = require_id(value, key)?;
    filters.push(Filter {
        gql,
        ty: "Int",
        value: json!(id),
    });
    echo.insert(key.to_string(), json!(id));
    Ok(())
}
