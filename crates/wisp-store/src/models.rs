use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

#[derive(Debug, Clone, Default)]
pub struct ExecLog {
    pub id: String,
    pub frame_id: String,
    pub cell_index: i64,
    pub tool: String,
    pub language: String,
    pub source: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_status: String,
    pub wall_s: Option<f64>,
    pub files_written: Vec<String>,
    pub files_read: Vec<String>,
    pub env_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub id: String,
    pub artifact_id: String,
    pub version_number: i64,
    pub content_type: String,
    pub storage_path: String,
    pub size_bytes: Option<i64>,
    pub checksum: Option<String>,
    pub parent_version_id: Option<String>,
    pub producing_run_id: Option<String>,
    pub env_snapshot_hash: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRegistry {
    pub id: String,
    pub name: String,
    pub root_path: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl LabRegistry {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        root_path: Option<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let registry = Self {
            id: id.into(),
            name: name.into(),
            root_path: root_path.filter(|path| !path.trim().is_empty()),
            created_at: now,
            updated_at: now,
        };
        registry.validate()?;
        Ok(registry)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Lab registry id is required");
        }
        if self.name.trim().is_empty() {
            anyhow::bail!("Lab registry name is required");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabEntityKind {
    ResourceDefinition,
    Lot,
    MaterialUnit,
    Subject,
    Location,
    ProtocolSource,
}

impl LabEntityKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ResourceDefinition => "resource_definition",
            Self::Lot => "lot",
            Self::MaterialUnit => "material_unit",
            Self::Subject => "subject",
            Self::Location => "location",
            Self::ProtocolSource => "protocol_source",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "resource_definition" => Ok(Self::ResourceDefinition),
            "lot" => Ok(Self::Lot),
            "material_unit" => Ok(Self::MaterialUnit),
            "subject" => Ok(Self::Subject),
            "location" => Ok(Self::Location),
            "protocol_source" => Ok(Self::ProtocolSource),
            _ => anyhow::bail!("Unknown lab entity kind"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabEntity {
    pub id: String,
    pub registry_id: String,
    pub display_id: String,
    pub kind: LabEntityKind,
    pub subtype: Option<String>,
    pub title: String,
    pub revision: i64,
    pub metadata_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Typed fields for a reusable laboratory resource. A resource definition is
/// deliberately not a physical vial, batch, or consumable balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabResourceDefinition {
    pub entity_id: String,
    pub category: String,
    pub supplier: Option<String>,
    pub catalog_number: Option<String>,
    pub attributes_json: String,
}

impl LabResourceDefinition {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.entity_id.trim().is_empty() || self.category.trim().is_empty() {
            anyhow::bail!("Resource definition requires entity_id and category");
        }
        if serde_json::from_str::<serde_json::Value>(&self.attributes_json).is_err() {
            anyhow::bail!("Resource definition attributes_json must be valid JSON");
        }
        Ok(())
    }
}

/// An external or human-facing key for an entity. Only explicitly namespaced
/// identity aliases (barcode and legacy/internal IDs) are globally unique;
/// free-form names intentionally remain ambiguous.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabAlias {
    pub id: String,
    pub registry_id: String,
    pub entity_id: String,
    pub alias_type: String,
    pub namespace: Option<String>,
    pub value: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabAliasCreate {
    pub alias_type: String,
    pub namespace: Option<String>,
    pub value: String,
}

impl LabAliasCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        let alias_type = self.alias_type.trim();
        if alias_type.is_empty() || self.value.trim().is_empty() {
            anyhow::bail!("Lab alias requires a type and value");
        }
        if alias_type == "legacy_id" || alias_type == "internal_id" {
            if self
                .namespace
                .as_deref()
                .is_none_or(|namespace| namespace.trim().is_empty())
            {
                anyhow::bail!("Legacy/internal aliases require a namespace");
            }
        }
        Ok(())
    }
}

impl LabEntity {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Lab entity id is required");
        }
        if self.registry_id.trim().is_empty() {
            anyhow::bail!("Lab entity registry_id is required");
        }
        if self.display_id.trim().is_empty() {
            anyhow::bail!("Lab entity display_id is required");
        }
        if self.title.trim().is_empty() {
            anyhow::bail!("Lab entity title is required");
        }
        if self.revision <= 0 {
            anyhow::bail!("Lab entity revision must be positive");
        }
        if serde_json::from_str::<serde_json::Value>(&self.metadata_json).is_err() {
            anyhow::bail!("Lab entity metadata_json must be valid JSON");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabActorKind {
    User,
    Agent,
    Import,
    System,
}

impl LabActorKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Import => "import",
            Self::System => "system",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            "import" => Ok(Self::Import),
            "system" => Ok(Self::System),
            _ => anyhow::bail!("Unknown lab actor kind"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabTransactionStatus {
    Committed,
}

impl LabTransactionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
        }
    }

    fn from_storage(value: &str) -> Result<Self> {
        match value {
            "committed" => Ok(Self::Committed),
            _ => anyhow::bail!("Unknown lab transaction status"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabTransaction {
    pub id: String,
    pub display_id: String,
    pub registry_id: String,
    pub project_id: Option<String>,
    pub command_id: String,
    pub schema_version: i64,
    pub actor_kind: LabActorKind,
    pub actor_ref: Option<String>,
    pub confirmation_json: String,
    pub request_json: String,
    pub receipt_json: String,
    pub status: LabTransactionStatus,
    pub created_at: i64,
    pub committed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabEvent {
    pub id: String,
    pub registry_id: String,
    pub project_id: Option<String>,
    pub transaction_id: String,
    pub sequence: i64,
    pub entity_id: Option<String>,
    pub prior_event_id: Option<String>,
    pub kind: String,
    pub schema_version: i64,
    pub payload_json: String,
    pub occurred_at: i64,
    pub recorded_at: i64,
    pub expected_revision: Option<i64>,
    pub resulting_revision: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabEntityCreate {
    pub kind: LabEntityKind,
    pub prefix: String,
    pub title: String,
    pub subtype: Option<String>,
    pub metadata_json: String,
    pub project_relation: Option<String>,
    #[serde(default)]
    pub resource_definition: Option<LabResourceDefinitionCreate>,
    #[serde(default)]
    pub aliases: Vec<LabAliasCreate>,
    #[serde(default)]
    pub lot: Option<LabLotCreate>,
    #[serde(default)]
    pub material_unit: Option<LabMaterialUnitCreate>,
    #[serde(default)]
    pub location: Option<LabLocationCreate>,
    #[serde(default)]
    pub subject: Option<LabSubjectCreate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabSubjectCreate {
    pub species: String,
    pub strain: Option<String>,
    pub sex: Option<String>,
    pub date_of_birth: Option<String>,
    pub origin_kind: String,
}

impl LabSubjectCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.species.trim().is_empty()
            || !matches!(
                self.origin_kind.as_str(),
                "birth" | "receipt" | "legacy_import"
            )
            || self
                .sex
                .as_ref()
                .is_some_and(|sex| !matches!(sex.as_str(), "female" | "male" | "unknown"))
        {
            anyhow::bail!("Subject fields are invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabSubject {
    pub entity_id: String,
    pub species: String,
    pub strain: Option<String>,
    pub sex: Option<String>,
    pub date_of_birth: Option<String>,
    pub origin_kind: String,
    pub established_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabResourceDefinitionCreate {
    pub category: String,
    pub supplier: Option<String>,
    pub catalog_number: Option<String>,
    pub attributes_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabLot {
    pub entity_id: String,
    pub resource_definition_id: String,
    pub supplier: Option<String>,
    pub catalog_number: Option<String>,
    pub lot_number: String,
    pub received_at: Option<i64>,
    pub expiry_at: Option<i64>,
    pub origin_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabLotCreate {
    pub resource_definition_id: String,
    pub supplier: Option<String>,
    pub catalog_number: Option<String>,
    pub lot_number: String,
    pub received_at: Option<i64>,
    pub expiry_at: Option<i64>,
    pub origin_kind: String,
}

impl LabLotCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.resource_definition_id.trim().is_empty() || self.lot_number.trim().is_empty() {
            anyhow::bail!("Lot requires a resource definition and lot number");
        }
        if !matches!(
            self.origin_kind.as_str(),
            "receipt" | "prepared" | "legacy_import"
        ) {
            anyhow::bail!("Lot origin_kind must be receipt, prepared, or legacy_import");
        }
        if self
            .expiry_at
            .zip(self.received_at)
            .is_some_and(|(expiry, received)| expiry < received)
        {
            anyhow::bail!("Lot expiry cannot be before received date");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabQuantityState {
    Measured,
    Unknown,
    NotMeasured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabQuantity {
    pub state: LabQuantityState,
    pub value: Option<String>,
    pub unit: Option<String>,
}

impl LabQuantity {
    pub fn measured(value: impl Into<String>, unit: impl Into<String>) -> Self {
        Self {
            state: LabQuantityState::Measured,
            value: Some(value.into()),
            unit: Some(unit.into()),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        match self.state {
            LabQuantityState::Measured => {
                let value = self
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Measured quantity needs a value"))?;
                let unit = self
                    .unit
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Measured quantity needs a unit"))?;
                validate_canonical_decimal(value)?;
                if lab_unit_dimension(unit).is_none() {
                    anyhow::bail!("Unsupported lab quantity unit: {unit}");
                }
            }
            LabQuantityState::Unknown | LabQuantityState::NotMeasured => {
                if self.value.is_some() || self.unit.is_some() {
                    anyhow::bail!("Unknown/not_measured quantity cannot carry a value or unit");
                }
            }
        }
        Ok(())
    }

    pub fn dimension(&self) -> Option<&'static str> {
        self.unit.as_deref().and_then(lab_unit_dimension)
    }
}

fn validate_canonical_decimal(value: &str) -> Result<()> {
    if value.is_empty() || value.starts_with('+') || value.starts_with('-') || value.trim() != value
    {
        anyhow::bail!("Quantity must be a non-negative canonical decimal string");
    }
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fractional = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
    {
        anyhow::bail!("Quantity must be a non-negative canonical decimal string");
    }
    if whole.len() > 1 && whole.starts_with('0') {
        anyhow::bail!("Quantity must not have leading zeroes");
    }
    if let Some(fractional) = fractional {
        if fractional.is_empty()
            || !fractional.bytes().all(|byte| byte.is_ascii_digit())
            || fractional.ends_with('0')
        {
            anyhow::bail!("Quantity decimal fraction must be canonical");
        }
    }
    Ok(())
}

pub fn lab_unit_dimension(unit: &str) -> Option<&'static str> {
    match unit {
        "uL" | "mL" | "L" => Some("volume"),
        "ng" | "ug" | "mg" | "g" => Some("mass"),
        "cells" | "reactions" | "each" | "box" => Some("count"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMaterialUnit {
    pub entity_id: String,
    pub lot_id: Option<String>,
    pub usage_class: String,
    pub quantity: LabQuantity,
    pub vessel_description: Option<String>,
    pub lifecycle: String,
    pub availability: String,
    pub identity_state: String,
    pub origin_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMaterialUnitCreate {
    pub lot_id: Option<String>,
    pub usage_class: String,
    pub quantity: LabQuantity,
    pub vessel_description: Option<String>,
    pub availability: String,
    pub origin_kind: String,
}

impl LabMaterialUnitCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if !matches!(self.usage_class.as_str(), "inventory" | "sample") {
            anyhow::bail!("Material unit usage_class must be inventory or sample");
        }
        if !matches!(self.availability.as_str(), "available" | "quarantined") {
            anyhow::bail!("Material unit availability is invalid");
        }
        if !matches!(
            self.origin_kind.as_str(),
            "receipt" | "legacy_import" | "prepared"
        ) {
            anyhow::bail!("Material unit origin_kind must be receipt, prepared, or legacy_import");
        }
        self.quantity.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabLocation {
    pub entity_id: String,
    pub parent_location_id: Option<String>,
    pub location_class: String,
    pub single_occupancy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabLocationCreate {
    pub parent_location_id: Option<String>,
    pub location_class: String,
    pub single_occupancy: bool,
}

impl LabLocationCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.location_class.trim().is_empty() {
            anyhow::bail!("Location requires a location_class");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMaterialMove {
    pub material_unit_id: String,
    pub location_id: Option<String>,
    pub expected_revision: i64,
    pub occurred_at: i64,
    pub reason: Option<String>,
    pub prior_event_id: Option<String>,
}

impl LabMaterialMove {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.material_unit_id.trim().is_empty() || self.expected_revision <= 0 {
            anyhow::bail!("Material move requires a material ID and positive expected revision");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMaterialAdjustment {
    pub material_unit_id: String,
    pub expected_revision: i64,
    pub quantity: LabQuantity,
    #[serde(default)]
    pub lifecycle: Option<String>,
    pub availability: Option<String>,
    #[serde(default)]
    pub identity_state: Option<String>,
    pub occurred_at: i64,
    pub reason: String,
    pub prior_event_id: Option<String>,
}

/// The database-owned narrative and Frontmatter extension fields for a lab
/// record. The rendered Markdown file is a portable projection, not a second
/// mutable source of truth for identity or inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabDocument {
    pub id: String,
    pub registry_id: String,
    pub entity_id: String,
    pub relative_path: String,
    pub schema_version: i64,
    pub narrative_markdown: String,
    pub extension_json: String,
    pub last_projected_content: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabDocumentUpsert {
    pub entity_id: String,
    pub relative_path: String,
    pub narrative_markdown: String,
    pub extension_json: String,
    pub expected_revision: Option<i64>,
}

impl LabDocumentUpsert {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.entity_id.trim().is_empty() || self.relative_path.trim().is_empty() {
            anyhow::bail!("Lab document requires an entity ID and relative path");
        }
        let path = self.relative_path.replace('\\', "/");
        if path.starts_with('/') || path.contains("../") || path == ".." {
            anyhow::bail!("Lab document path must stay within the registry root");
        }
        if serde_json::from_str::<serde_json::Value>(&self.extension_json).is_err() {
            anyhow::bail!("Lab document extension_json must be valid JSON");
        }
        if self.expected_revision.is_some_and(|revision| revision <= 0) {
            anyhow::bail!("Lab document expected revision must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabProjectionOutboxItem {
    pub id: String,
    pub registry_id: String,
    pub document_id: String,
    pub target_path: String,
    pub content: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}

/// Schema-aware representation of an on-disk dossier. Unknown Frontmatter is
/// deliberately retained so a projection/import cycle does not destroy a
/// laboratory's local annotations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedLabDocument {
    pub display_id: String,
    pub entity_id: String,
    pub kind: String,
    pub title: String,
    pub entity_revision: i64,
    pub document_revision: i64,
    pub schema_version: i64,
    pub extensions: serde_json::Value,
    pub unknown_frontmatter: serde_json::Map<String, serde_json::Value>,
    pub narrative_markdown: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabDocumentImportPreview {
    pub entity_id: String,
    pub status: String,
    pub conflicts: Vec<String>,
    pub parsed: ParsedLabDocument,
}

pub fn parse_lab_document(markdown: &str) -> Result<ParsedLabDocument> {
    let body = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))
        .ok_or_else(|| anyhow::anyhow!("Lab dossier must start with YAML Frontmatter"))?;
    let (yaml, narrative) = body
        .split_once("\n---\n")
        .or_else(|| body.split_once("\r\n---\r\n"))
        .ok_or_else(|| anyhow::anyhow!("Lab dossier Frontmatter is not closed"))?;
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(yaml)?;
    let mapping = match yaml_value {
        serde_yaml::Value::Mapping(mapping) => mapping,
        _ => anyhow::bail!("Lab dossier Frontmatter must be a mapping"),
    };
    let mut fields = serde_json::Map::new();
    for (key, value) in mapping {
        let key = match key {
            serde_yaml::Value::String(key) => key,
            _ => anyhow::bail!("Lab dossier Frontmatter keys must be strings"),
        };
        fields.insert(key, serde_json::to_value(value)?);
    }
    let string =
        |name: &str, fields: &mut serde_json::Map<String, serde_json::Value>| -> Result<String> {
            fields
                .remove(name)
                .and_then(|value| value.as_str().map(str::to_string))
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("Lab dossier requires string field {name}"))
        };
    let integer = |name: &str,
                   fields: &mut serde_json::Map<String, serde_json::Value>|
     -> Result<i64> {
        fields
            .remove(name)
            .and_then(|value| value.as_i64())
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow::anyhow!("Lab dossier requires positive integer field {name}"))
    };
    let display_id = string("id", &mut fields)?;
    let entity_id = string("entity_id", &mut fields)?;
    let kind = string("kind", &mut fields)?;
    let title = string("title", &mut fields)?;
    let entity_revision = integer("entity_revision", &mut fields)?;
    let document_revision = integer("document_revision", &mut fields)?;
    let schema_version = integer("schema_version", &mut fields)?;
    let extensions = fields
        .remove("extensions")
        .unwrap_or_else(|| serde_json::json!({}));
    if !extensions.is_object() {
        anyhow::bail!("Lab dossier extensions must be an object");
    }
    Ok(ParsedLabDocument {
        display_id,
        entity_id,
        kind,
        title,
        entity_revision,
        document_revision,
        schema_version,
        extensions,
        unknown_frontmatter: fields,
        narrative_markdown: narrative.to_string(),
    })
}

pub fn serialize_lab_document(document: &ParsedLabDocument) -> Result<String> {
    let mut fields = document.unknown_frontmatter.clone();
    fields.insert("id".into(), serde_json::json!(document.display_id));
    fields.insert("entity_id".into(), serde_json::json!(document.entity_id));
    fields.insert("kind".into(), serde_json::json!(document.kind));
    fields.insert("title".into(), serde_json::json!(document.title));
    fields.insert(
        "entity_revision".into(),
        serde_json::json!(document.entity_revision),
    );
    fields.insert(
        "document_revision".into(),
        serde_json::json!(document.document_revision),
    );
    fields.insert(
        "schema_version".into(),
        serde_json::json!(document.schema_version),
    );
    fields.insert("extensions".into(), document.extensions.clone());
    let yaml = serde_yaml::to_string(&serde_json::Value::Object(fields))?;
    Ok(format!("---\n{yaml}---\n{}", document.narrative_markdown))
}

impl LabMaterialAdjustment {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.material_unit_id.trim().is_empty()
            || self.expected_revision <= 0
            || self.reason.trim().is_empty()
        {
            anyhow::bail!("Material adjustment requires ID, expected revision, and reason");
        }
        if let Some(availability) = self.availability.as_deref() {
            if !matches!(availability, "available" | "quarantined") {
                anyhow::bail!("Material adjustment availability is invalid");
            }
        }
        if self.lifecycle.as_deref().is_some_and(|lifecycle| {
            !matches!(
                lifecycle,
                "planned" | "active" | "depleted" | "discarded" | "lost" | "void"
            )
        }) {
            anyhow::bail!("Material adjustment lifecycle is invalid");
        }
        if self
            .identity_state
            .as_deref()
            .is_some_and(|identity_state| {
                !matches!(identity_state, "verified" | "suspect" | "mislabeled")
            })
        {
            anyhow::bail!("Material adjustment identity_state is invalid");
        }
        self.quantity.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabEntityUpdate {
    pub entity_id: String,
    pub expected_revision: i64,
    pub title: String,
    pub subtype: Option<String>,
    pub metadata_json: String,
    pub event_kind: String,
    pub event_payload_json: String,
    pub occurred_at: i64,
    pub prior_event_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabEventRecord {
    pub entity_id: Option<String>,
    pub kind: String,
    pub payload_json: String,
    pub occurred_at: i64,
    pub prior_event_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabAmendment {
    pub id: String,
    pub display_id: String,
    pub registry_id: String,
    pub run_id: String,
    pub original_event_id: String,
    pub reason: String,
    pub correction_json: String,
    pub affected_ids: Vec<String>,
    pub established_event_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabAmendmentCreate {
    pub original_event_id: String,
    pub reason: String,
    pub correction_json: String,
}

impl LabAmendmentCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.original_event_id.trim().is_empty()
            || self.reason.trim().is_empty()
            || serde_json::from_str::<serde_json::Value>(&self.correction_json).is_err()
        {
            anyhow::bail!(
                "Amendment requires an original event, reason, and valid correction JSON"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabTransactionRequest {
    pub registry_id: String,
    pub project_id: Option<String>,
    pub command_id: String,
    pub schema_version: i64,
    pub actor_kind: LabActorKind,
    pub actor_ref: Option<String>,
    pub confirmation_json: String,
    pub occurred_at: i64,
    pub entity_creates: Vec<LabEntityCreate>,
    pub entity_updates: Vec<LabEntityUpdate>,
    pub event_records: Vec<LabEventRecord>,
    #[serde(default)]
    pub material_moves: Vec<LabMaterialMove>,
    #[serde(default)]
    pub material_adjustments: Vec<LabMaterialAdjustment>,
    #[serde(default)]
    pub document_upserts: Vec<LabDocumentUpsert>,
    #[serde(default)]
    pub run_participants: Vec<LabRunParticipantCreate>,
    #[serde(default)]
    pub subject_participants: Vec<LabSubjectParticipantCreate>,
    #[serde(default)]
    pub run_output_creates: Vec<LabRunOutputCreate>,
    #[serde(default)]
    pub derivation_creates: Vec<LabMaterialDerivationCreate>,
    #[serde(default)]
    pub run_deviation_creates: Vec<LabRunDeviationCreate>,
    #[serde(default)]
    pub data_evidence_creates: Vec<LabDataEvidenceCreate>,
    #[serde(default)]
    pub amendment_creates: Vec<LabAmendmentCreate>,
    #[serde(default)]
    pub protocol_revision_creates: Vec<LabProtocolRevisionCreate>,
    #[serde(default)]
    pub run_protocol_pins: Vec<LabRunProtocolPin>,
    #[serde(default)]
    pub run_status_updates: Vec<LabRunStatusUpdate>,
    #[serde(default)]
    pub conclusion_creates: Vec<LabConclusionCreate>,
    #[serde(default)]
    pub reservation_creates: Vec<LabReservationCreate>,
    #[serde(default)]
    pub qc_observation_creates: Vec<LabQcObservationCreate>,
    #[serde(default)]
    pub qc_assessment_creates: Vec<LabQcAssessmentCreate>,
}

impl LabTransactionRequest {
    pub fn new(
        registry_id: impl Into<String>,
        command_id: impl Into<String>,
        actor_kind: LabActorKind,
    ) -> Self {
        Self {
            registry_id: registry_id.into(),
            project_id: None,
            command_id: command_id.into(),
            schema_version: 1,
            actor_kind,
            actor_ref: None,
            confirmation_json: "{}".into(),
            occurred_at: chrono::Utc::now().timestamp(),
            entity_creates: vec![],
            entity_updates: vec![],
            event_records: vec![],
            material_moves: vec![],
            material_adjustments: vec![],
            document_upserts: vec![],
            run_participants: vec![],
            subject_participants: vec![],
            run_output_creates: vec![],
            derivation_creates: vec![],
            run_deviation_creates: vec![],
            data_evidence_creates: vec![],
            amendment_creates: vec![],
            protocol_revision_creates: vec![],
            run_protocol_pins: vec![],
            run_status_updates: vec![],
            conclusion_creates: vec![],
            reservation_creates: vec![],
            qc_observation_creates: vec![],
            qc_assessment_creates: vec![],
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.registry_id.trim().is_empty() {
            anyhow::bail!("Lab transaction registry_id is required");
        }
        if self.command_id.trim().is_empty() {
            anyhow::bail!("Lab transaction command_id is required");
        }
        if self.schema_version <= 0 {
            anyhow::bail!("Lab transaction schema_version must be positive");
        }
        if serde_json::from_str::<serde_json::Value>(&self.confirmation_json).is_err() {
            anyhow::bail!("Lab transaction confirmation_json must be valid JSON");
        }
        for create in &self.entity_creates {
            if create.title.trim().is_empty() {
                anyhow::bail!("Lab entity title is required");
            }
            if serde_json::from_str::<serde_json::Value>(&create.metadata_json).is_err() {
                anyhow::bail!("Lab entity metadata_json must be valid JSON");
            }
            if let Some(definition) = &create.resource_definition {
                if create.kind != LabEntityKind::ResourceDefinition {
                    anyhow::bail!("Only ResourceDefinition entities can have resource fields");
                }
                if definition.category.trim().is_empty()
                    || serde_json::from_str::<serde_json::Value>(&definition.attributes_json)
                        .is_err()
                {
                    anyhow::bail!(
                        "Resource definition requires category and valid attributes_json"
                    );
                }
            }
            for alias in &create.aliases {
                alias.validate()?;
            }
            if let Some(lot) = &create.lot {
                if create.kind != LabEntityKind::Lot {
                    anyhow::bail!("Only Lot entities can have lot fields");
                }
                lot.validate()?;
            }
            if let Some(material) = &create.material_unit {
                if create.kind != LabEntityKind::MaterialUnit {
                    anyhow::bail!("Only MaterialUnit entities can have material fields");
                }
                material.validate()?;
            }
            if let Some(location) = &create.location {
                if create.kind != LabEntityKind::Location {
                    anyhow::bail!("Only Location entities can have location fields");
                }
                location.validate()?;
            }
            if let Some(subject) = &create.subject {
                if create.kind != LabEntityKind::Subject {
                    anyhow::bail!("Only Subject entities can have subject fields");
                }
                subject.validate()?;
            }
        }
        for update in &self.entity_updates {
            if update.entity_id.trim().is_empty() || update.expected_revision <= 0 {
                anyhow::bail!("Lab entity update requires an id and positive expected revision");
            }
            if update.title.trim().is_empty() || update.event_kind.trim().is_empty() {
                anyhow::bail!("Lab entity update requires title and event kind");
            }
            if serde_json::from_str::<serde_json::Value>(&update.metadata_json).is_err()
                || serde_json::from_str::<serde_json::Value>(&update.event_payload_json).is_err()
            {
                anyhow::bail!("Lab entity update JSON fields must be valid JSON");
            }
        }
        for event in &self.event_records {
            if event.kind.trim().is_empty()
                || serde_json::from_str::<serde_json::Value>(&event.payload_json).is_err()
            {
                anyhow::bail!("Lab event requires a kind and valid JSON payload");
            }
        }
        for material_move in &self.material_moves {
            material_move.validate()?;
        }
        for material_adjustment in &self.material_adjustments {
            material_adjustment.validate()?;
        }
        for document in &self.document_upserts {
            document.validate()?;
        }
        for participant in &self.run_participants {
            participant.validate()?;
            if participant.direction == "output" {
                anyhow::bail!(
                    "New Run outputs must use run_output_creates so identity and lineage commit together"
                );
            }
        }
        for participant in &self.subject_participants {
            participant.validate()?;
        }
        for output in &self.run_output_creates {
            output.validate()?;
            if output.entity.title.trim().is_empty()
                || serde_json::from_str::<serde_json::Value>(&output.entity.metadata_json).is_err()
            {
                anyhow::bail!("Run output requires a title and valid metadata_json");
            }
            for alias in &output.entity.aliases {
                alias.validate()?;
            }
        }
        for derivation in &self.derivation_creates {
            derivation.validate()?;
        }
        for deviation in &self.run_deviation_creates {
            deviation.validate()?;
        }
        for evidence in &self.data_evidence_creates {
            evidence.validate()?;
        }
        for amendment in &self.amendment_creates {
            amendment.validate()?;
        }
        for revision in &self.protocol_revision_creates {
            revision.validate()?;
        }
        for pin in &self.run_protocol_pins {
            pin.validate()?;
        }
        for update in &self.run_status_updates {
            update.validate()?;
        }
        for conclusion in &self.conclusion_creates {
            conclusion.validate()?;
        }
        for reservation in &self.reservation_creates {
            reservation.validate()?;
        }
        for observation in &self.qc_observation_creates {
            observation.validate()?;
        }
        for assessment in &self.qc_assessment_creates {
            assessment.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabTransactionResult {
    pub transaction: LabTransaction,
    pub events: Vec<LabEvent>,
    pub created_entities: Vec<LabEntity>,
    pub idempotent: bool,
}

#[derive(Debug, Clone)]
pub struct RecentSessionDetail {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: i64,
    pub activity_at: i64,
    pub last_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSearchResult {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub ts: i64,
    pub project_id: String,
    pub project_name: String,
    pub project_root: String,
    pub session_id: String,
    pub session_title: String,
    pub size_bytes: Option<i64>,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchResult {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub title: String,
    pub created_at: i64,
    pub activity_at: i64,
    pub last_role: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionContextKind {
    Local,
    Ssh,
    Wsl,
}

impl ExecutionContextKind {
    pub fn from_id(id: &str) -> Result<Self> {
        if id != id.trim() || id.is_empty() {
            anyhow::bail!("Invalid execution context id");
        }
        if id == "local" {
            return Ok(Self::Local);
        }
        if let Some(alias) = id.strip_prefix("ssh:") {
            validate_context_suffix(alias)?;
            return Ok(Self::Ssh);
        }
        if let Some(distro) = id.strip_prefix("wsl:") {
            validate_context_suffix(distro)?;
            return Ok(Self::Wsl);
        }
        anyhow::bail!("Unknown execution context id prefix");
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Ssh => "ssh",
            Self::Wsl => "wsl",
        }
    }

    fn from_storage(s: &str) -> Result<Self> {
        match s {
            "local" => Ok(Self::Local),
            "ssh" => Ok(Self::Ssh),
            "wsl" => Ok(Self::Wsl),
            _ => anyhow::bail!("Unknown execution context kind"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub id: String,
    pub kind: ExecutionContextKind,
    pub label: String,
    pub config_json: String,
    pub capabilities_json: String,
    pub last_probe_at: Option<i64>,
    pub last_probe_status: Option<String>,
    pub last_probe_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Draft,
    Submitted,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Lost,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Submitted => "submitted",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Lost => "lost",
        }
    }

    pub(crate) fn from_storage(s: &str) -> Result<Self> {
        match s {
            "draft" => Ok(Self::Draft),
            "submitted" => Ok(Self::Submitted),
            "running" => Ok(Self::Running),
            "cancelling" => Ok(Self::Cancelling),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "lost" => Ok(Self::Lost),
            _ => anyhow::bail!("Unknown run status"),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Lost
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub project_id: String,
    pub frame_id: Option<String>,
    pub context_id: String,
    pub title: String,
    pub kind: String,
    pub status: RunStatus,
    pub command: Option<String>,
    pub script_path: Option<String>,
    pub input_refs_json: String,
    pub output_specs_json: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub remote_workdir: Option<String>,
    pub remote_handle_json: Option<String>,
    pub timeout_secs: Option<i64>,
    pub last_polled_at: Option<i64>,
    pub last_poll_error: Option<String>,
    pub env_snapshot_json: String,
}

/// Wet-lab details sidecar for the existing project-owned Run identity. Its
/// lifecycle remains in `runs.status`; there is deliberately no process lease
/// or fake execution context here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabWetRun {
    pub run_id: String,
    pub registry_id: String,
    pub display_id: String,
    pub command_id: String,
    pub operator: Option<String>,
    pub protocol_revision_id: Option<String>,
    pub deviations_json: String,
    pub created_at: i64,
}

/// The wet-lab Run bound to a saved conversation.  The two records deliberately
/// stay distinct: `RunRecord` owns project lifecycle, while `LabWetRun` owns
/// registry-scoped lab identity and protocol information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabConversationRun {
    pub run: RunRecord,
    pub wet_lab_run: LabWetRun,
}

/// Read-only pre-close snapshot. Issues are intentionally advisory: a failed or
/// incomplete experiment may still be closed after the user sees and confirms them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunCloseoutSummary {
    pub run_id: String,
    pub status: RunStatus,
    pub protocol_revision_id: Option<String>,
    pub input_count: i64,
    pub subject_participant_count: i64,
    pub output_count: i64,
    pub unlocated_output_ids: Vec<String>,
    pub deviation_count: i64,
    pub unresolved_deviation_ids: Vec<String>,
    pub qc_observation_count: i64,
    pub qc_assessment_count: i64,
    pub data_evidence_count: i64,
    pub evidence_without_checksum_ids: Vec<String>,
    pub active_reservation_count: i64,
    pub amendment_count: i64,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabProtocolRevision {
    pub id: String,
    pub registry_id: String,
    pub protocol_entity_id: String,
    pub revision_number: i64,
    pub checksum_sha256: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabProtocolRevisionCreate {
    pub protocol_entity_id: String,
    pub content: String,
}

impl LabProtocolRevisionCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.protocol_entity_id.trim().is_empty() || self.content.trim().is_empty() {
            anyhow::bail!("Protocol revision requires a source entity and content");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunProtocolPin {
    pub run_id: String,
    pub protocol_revision_id: String,
}

impl LabRunProtocolPin {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty() || self.protocol_revision_id.trim().is_empty() {
            anyhow::bail!("Protocol pin requires a Run and revision");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunStatusUpdate {
    pub run_id: String,
    pub status: RunStatus,
}

impl LabRunStatusUpdate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty()
            || !matches!(
                self.status,
                RunStatus::Draft
                    | RunStatus::Running
                    | RunStatus::Succeeded
                    | RunStatus::Failed
                    | RunStatus::Cancelled
            )
        {
            anyhow::bail!("Wet-lab Run status update is invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunParticipant {
    pub id: String,
    pub run_id: String,
    pub material_unit_id: String,
    pub direction: String,
    pub role: String,
    pub effect: String,
    pub quantity: Option<LabQuantity>,
    pub transformation_group: Option<String>,
    pub established_event_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunParticipantCreate {
    pub run_id: String,
    pub material_unit_id: String,
    pub direction: String,
    pub role: String,
    pub effect: String,
    pub quantity: Option<LabQuantity>,
    pub transformation_group: Option<String>,
    #[serde(default)]
    pub expected_material_revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabSubjectParticipant {
    pub id: String,
    pub run_id: String,
    pub subject_id: String,
    pub role: String,
    pub effect: String,
    pub established_event_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabSubjectParticipantCreate {
    pub run_id: String,
    pub subject_id: String,
    pub role: String,
    pub effect: String,
}

impl LabSubjectParticipantCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty()
            || self.subject_id.trim().is_empty()
            || self.role.trim().is_empty()
            || !matches!(
                self.effect.as_str(),
                "observed" | "handled" | "sample_collected"
            )
        {
            anyhow::bail!("Subject participant fields are invalid");
        }
        Ok(())
    }
}

impl LabRunParticipantCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty() || self.material_unit_id.trim().is_empty() {
            anyhow::bail!("Run participant requires a Run and MaterialUnit");
        }
        if !matches!(
            self.role.as_str(),
            "sample" | "reagent" | "control" | "product" | "waste"
        ) {
            anyhow::bail!("Run participant role is invalid");
        }
        match self.direction.as_str() {
            "input"
                if matches!(
                    self.effect.as_str(),
                    "observed"
                        | "returned"
                        | "partially_consumed"
                        | "fully_consumed"
                        | "transformed"
                        | "sampled_from"
                ) => {}
            "output" if matches!(self.effect.as_str(), "produced" | "produced_as_waste") => {}
            _ => anyhow::bail!("Run participant effect is incompatible with direction"),
        }
        if let Some(quantity) = &self.quantity {
            quantity.validate()?;
        }
        if matches!(
            self.effect.as_str(),
            "partially_consumed" | "fully_consumed" | "transformed" | "sampled_from"
        ) && !self
            .expected_material_revision
            .is_some_and(|revision| revision > 0)
        {
            anyhow::bail!("Consuming a MaterialUnit requires its expected revision");
        }
        if matches!(self.effect.as_str(), "partially_consumed" | "sampled_from")
            && !self
                .quantity
                .as_ref()
                .is_some_and(|quantity| quantity.state == LabQuantityState::Measured)
        {
            anyhow::bail!("Partial consumption requires a measured quantity");
        }
        Ok(())
    }
}

/// A realized output sample and its producing Run edge, committed atomically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunOutputCreate {
    pub run_id: String,
    pub entity: LabEntityCreate,
    pub role: String,
    pub effect: String,
    pub quantity: Option<LabQuantity>,
    pub transformation_group: Option<String>,
    #[serde(default)]
    pub initial_location_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMaterialDerivation {
    pub id: String,
    pub run_id: String,
    pub operation: String,
    pub group_id: String,
    pub parent_material_unit_id: String,
    pub child_material_unit_id: String,
    pub established_event_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabMaterialDerivationCreate {
    pub run_id: String,
    pub operation: String,
    pub inputs: Vec<LabRunParticipantCreate>,
    pub outputs: Vec<LabRunOutputCreate>,
}

impl LabMaterialDerivationCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty()
            || !matches!(
                self.operation.as_str(),
                "split" | "aliquot" | "merge" | "pool" | "passage" | "transform"
            )
            || self.inputs.is_empty()
            || self.outputs.is_empty()
            || self
                .inputs
                .iter()
                .any(|input| input.run_id != self.run_id || input.direction != "input")
            || self
                .outputs
                .iter()
                .any(|output| output.run_id != self.run_id)
        {
            anyhow::bail!("Material derivation fields are invalid");
        }
        match self.operation.as_str() {
            "split" | "aliquot" | "passage" | "transform" if self.inputs.len() != 1 => {
                anyhow::bail!("This derivation operation requires exactly one input")
            }
            "merge" | "pool" if self.inputs.len() < 2 || self.outputs.len() != 1 => {
                anyhow::bail!("Merge/pool requires multiple inputs and exactly one output")
            }
            _ => {}
        }
        for input in &self.inputs {
            input.validate()?;
            if !matches!(
                input.effect.as_str(),
                "partially_consumed" | "fully_consumed" | "transformed" | "sampled_from"
            ) {
                anyhow::bail!(
                    "Material derivation inputs must consume, transform, or sample their source"
                );
            }
        }
        for output in &self.outputs {
            output.validate()?;
        }
        if self.inputs.iter().any(|input| {
            !input
                .quantity
                .as_ref()
                .is_some_and(|quantity| quantity.state == LabQuantityState::Measured)
        }) || self.outputs.iter().any(|output| {
            !output
                .quantity
                .as_ref()
                .is_some_and(|quantity| quantity.state == LabQuantityState::Measured)
        }) {
            anyhow::bail!("Material derivation requires measured input and output quantities");
        }
        Ok(())
    }
}

impl LabRunOutputCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty() {
            anyhow::bail!("Run output requires a Run");
        }
        let material = self
            .entity
            .material_unit
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Run output requires MaterialUnit details"))?;
        if self.entity.kind != LabEntityKind::MaterialUnit
            || material.usage_class != "sample"
            || material.origin_kind != "prepared"
        {
            anyhow::bail!("Run output must create a prepared sample MaterialUnit");
        }
        if !matches!(self.role.as_str(), "sample" | "product" | "waste")
            || !matches!(self.effect.as_str(), "produced" | "produced_as_waste")
            || (self.effect == "produced_as_waste" && self.role != "waste")
        {
            anyhow::bail!("Run output role or effect is invalid");
        }
        if let Some(quantity) = &self.quantity {
            quantity.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunDeviation {
    pub id: String,
    pub run_id: String,
    pub step_ref: Option<String>,
    pub description: String,
    pub impact: String,
    pub disposition: Option<String>,
    pub occurred_at: i64,
    pub recorded_at: i64,
    pub established_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunDeviationCreate {
    pub run_id: String,
    pub step_ref: Option<String>,
    pub description: String,
    pub impact: String,
    pub disposition: Option<String>,
    pub occurred_at: i64,
}

/// Typed reference to potentially large evidence. The bytes remain at `uri`;
/// identity and provenance remain stable when that URI later changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabDataEvidence {
    pub id: String,
    pub display_id: String,
    pub registry_id: String,
    pub owner_project_id: Option<String>,
    pub owner_registry_id: Option<String>,
    pub producing_run_id: Option<String>,
    pub role: String,
    pub uri: String,
    pub format: Option<String>,
    pub size_bytes: Option<i64>,
    pub checksum_sha256: Option<String>,
    pub origin: String,
    pub manifest_json: String,
    pub created_at: i64,
    pub established_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabDataEvidenceCreate {
    pub owner_project_id: Option<String>,
    pub owner_registry_id: Option<String>,
    pub producing_run_id: Option<String>,
    pub role: String,
    pub uri: String,
    pub format: Option<String>,
    pub size_bytes: Option<i64>,
    pub checksum_sha256: Option<String>,
    pub origin: String,
    pub manifest_json: String,
}

impl LabDataEvidenceCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.owner_project_id.is_some() == self.owner_registry_id.is_some() {
            anyhow::bail!("Data evidence requires exactly one Project or Registry owner");
        }
        if self.role.trim().is_empty()
            || self.uri.trim().is_empty()
            || self.origin.trim().is_empty()
            || serde_json::from_str::<serde_json::Value>(&self.manifest_json).is_err()
            || self.size_bytes.is_some_and(|size| size < 0)
            || self.checksum_sha256.as_ref().is_some_and(|checksum| {
                checksum.len() != 64 || !checksum.chars().all(|value| value.is_ascii_hexdigit())
            })
        {
            anyhow::bail!("Data evidence fields are invalid");
        }
        Ok(())
    }
}

impl LabRunDeviationCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty() || self.description.trim().is_empty() {
            anyhow::bail!("Run deviation requires a Run and description");
        }
        if !matches!(self.impact.as_str(), "none" | "minor" | "major" | "unknown") {
            anyhow::bail!("Run deviation impact is invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabReservation {
    pub id: String,
    pub run_id: String,
    pub material_unit_id: String,
    pub quantity: LabQuantity,
    pub status: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub released_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabReservationCreate {
    pub run_id: String,
    pub material_unit_id: String,
    pub quantity: LabQuantity,
    pub expires_at: Option<i64>,
}

impl LabReservationCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty() || self.material_unit_id.trim().is_empty() {
            anyhow::bail!("Reservation requires a Run and MaterialUnit");
        }
        if self.quantity.state != LabQuantityState::Measured {
            anyhow::bail!("Reservation requires a measured quantity");
        }
        self.quantity.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabQcObservation {
    pub id: String,
    pub registry_id: String,
    pub entity_id: String,
    pub run_id: Option<String>,
    pub method_revision_id: Option<String>,
    pub measurement_json: String,
    pub evidence_json: String,
    pub observed_at: i64,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabQcObservationCreate {
    pub entity_id: String,
    pub run_id: Option<String>,
    pub method_revision_id: Option<String>,
    pub measurement_json: String,
    pub evidence_json: String,
    pub observed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabQcAssessment {
    pub id: String,
    pub registry_id: String,
    pub entity_id: String,
    pub observation_ids: Vec<String>,
    pub criteria_json: String,
    pub verdict: String,
    pub rationale: String,
    pub created_at: i64,
}

impl LabQcObservationCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.entity_id.trim().is_empty()
            || serde_json::from_str::<serde_json::Value>(&self.measurement_json).is_err()
            || serde_json::from_str::<serde_json::Value>(&self.evidence_json).is_err()
        {
            anyhow::bail!("QC observation requires entity ID and valid measurement/evidence JSON");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabQcAssessmentCreate {
    pub entity_id: String,
    pub observation_ids: Vec<String>,
    pub criteria_json: String,
    pub verdict: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabEntityProvenance {
    pub entity: LabEntity,
    pub location_id: Option<String>,
    pub producing_runs: Vec<String>,
    pub consuming_runs: Vec<String>,
    pub qc_observation_count: i64,
    pub latest_qc_verdict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRunProvenance {
    pub run: RunRecord,
    pub wet_lab_run: LabWetRun,
    pub protocol_revision: Option<LabProtocolRevision>,
    pub participants: Vec<LabRunParticipant>,
    pub subject_participants: Vec<LabSubjectParticipant>,
    pub deviations: Vec<LabRunDeviation>,
    /// Raw evidence precedes observations and assessments in the serialized read model.
    pub raw_evidence: Vec<LabDataEvidence>,
    pub observations: Vec<LabQcObservation>,
    pub assessments: Vec<LabQcAssessment>,
    pub amendments: Vec<LabAmendment>,
    pub decisions: Vec<ResearchNode>,
    pub closeout: LabRunCloseoutSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabConclusionCreate {
    pub run_id: String,
    pub title: String,
    pub conclusion: String,
    pub evidence_ids: Vec<String>,
    pub lesson: Option<String>,
}

impl LabConclusionCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.run_id.trim().is_empty()
            || self.title.trim().is_empty()
            || self.conclusion.trim().is_empty()
        {
            anyhow::bail!("Conclusion requires a Run, title, and confirmed conclusion");
        }
        Ok(())
    }
}

impl LabQcAssessmentCreate {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.entity_id.trim().is_empty()
            || self.observation_ids.is_empty()
            || self.rationale.trim().is_empty()
            || !matches!(
                self.verdict.as_str(),
                "pending" | "pass" | "conditional" | "fail" | "not_applicable"
            )
            || serde_json::from_str::<serde_json::Value>(&self.criteria_json).is_err()
        {
            anyhow::bail!("QC assessment fields are invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchNodeKind {
    Decision,
    Paper,
    DataAsset,
    Run,
    Artifact,
}

impl ResearchNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Paper => "paper",
            Self::DataAsset => "data_asset",
            Self::Run => "run",
            Self::Artifact => "artifact",
        }
    }

    fn from_storage(s: &str) -> Result<Self> {
        match s {
            "decision" => Ok(Self::Decision),
            "paper" => Ok(Self::Paper),
            "data_asset" => Ok(Self::DataAsset),
            "run" => Ok(Self::Run),
            "artifact" => Ok(Self::Artifact),
            _ => anyhow::bail!("Unknown research node kind"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchNode {
    pub id: String,
    pub project_id: String,
    pub kind: ResearchNodeKind,
    pub title: String,
    pub ref_id: Option<String>,
    pub metadata_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ResearchNode {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        kind: ResearchNodeKind,
        title: impl Into<String>,
    ) -> Result<Self> {
        let now = chrono::Utc::now().timestamp();
        let node = Self {
            id: id.into(),
            project_id: project_id.into(),
            kind,
            title: title.into(),
            ref_id: None,
            metadata_json: "{}".into(),
            created_at: now,
            updated_at: now,
        };
        node.validate()?;
        Ok(node)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Research node id is required");
        }
        if self.project_id.trim().is_empty() {
            anyhow::bail!("Research node project_id is required");
        }
        if self.title.trim().is_empty() {
            anyhow::bail!("Research node title is required");
        }
        if serde_json::from_str::<serde_json::Value>(&self.metadata_json).is_err() {
            anyhow::bail!("Research node metadata_json must be valid JSON");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchEdge {
    pub id: String,
    pub project_id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub metadata_json: String,
    pub created_at: i64,
}

impl ResearchEdge {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        source_id: impl Into<String>,
        target_id: impl Into<String>,
        relation: impl Into<String>,
    ) -> Result<Self> {
        let edge = Self {
            id: id.into(),
            project_id: project_id.into(),
            source_id: source_id.into(),
            target_id: target_id.into(),
            relation: relation.into(),
            metadata_json: "{}".into(),
            created_at: chrono::Utc::now().timestamp(),
        };
        edge.validate()?;
        Ok(edge)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Research edge id is required");
        }
        if self.project_id.trim().is_empty() {
            anyhow::bail!("Research edge project_id is required");
        }
        if self.source_id.trim().is_empty() || self.target_id.trim().is_empty() {
            anyhow::bail!("Research edge endpoints are required");
        }
        if self.relation.trim().is_empty() {
            anyhow::bail!("Research edge relation is required");
        }
        if serde_json::from_str::<serde_json::Value>(&self.metadata_json).is_err() {
            anyhow::bail!("Research edge metadata_json must be valid JSON");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchGraph {
    pub nodes: Vec<ResearchNode>,
    pub edges: Vec<ResearchEdge>,
}

impl RunRecord {
    pub fn new(
        id: impl Into<String>,
        project_id: impl Into<String>,
        context_id: impl Into<String>,
        title: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: id.into(),
            project_id: project_id.into(),
            frame_id: None,
            context_id: context_id.into(),
            title: title.into(),
            kind: kind.into(),
            status: RunStatus::Draft,
            command: None,
            script_path: None,
            input_refs_json: "[]".into(),
            output_specs_json: "[]".into(),
            created_at: now,
            started_at: None,
            ended_at: None,
            exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            remote_workdir: None,
            remote_handle_json: None,
            timeout_secs: None,
            last_polled_at: None,
            last_poll_error: None,
            env_snapshot_json: "{}".into(),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            anyhow::bail!("Run id is required");
        }
        if self.project_id.trim().is_empty() {
            anyhow::bail!("Run project_id is required");
        }
        if self.kind == "wet_lab" {
            if !self.context_id.is_empty() {
                anyhow::bail!("Wet-lab Runs must not use a compute execution context");
            }
        } else {
            ExecutionContextKind::from_id(&self.context_id)?;
        }
        if self.title.trim().is_empty() {
            anyhow::bail!("Run title is required");
        }
        if self.kind.trim().is_empty() {
            anyhow::bail!("Run kind is required");
        }
        Ok(())
    }
}

impl ExecutionContext {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Result<Self> {
        let id = id.into();
        let kind = ExecutionContextKind::from_id(&id)?;
        let label = label.into();
        if label.trim().is_empty() {
            anyhow::bail!("Execution context label is required");
        }
        let now = chrono::Utc::now().timestamp();
        Ok(Self {
            id,
            kind,
            label,
            config_json: "{}".into(),
            capabilities_json: "{}".into(),
            last_probe_at: None,
            last_probe_status: None,
            last_probe_error: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let kind = ExecutionContextKind::from_id(&self.id)?;
        if kind != self.kind {
            anyhow::bail!("Execution context kind does not match id");
        }
        if self.label.trim().is_empty() {
            anyhow::bail!("Execution context label is required");
        }
        Ok(())
    }
}

fn validate_context_suffix(s: &str) -> Result<()> {
    if s.is_empty() || s != s.trim() || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        anyhow::bail!("Invalid execution context id suffix");
    }
    Ok(())
}

pub(crate) fn execution_context_from_row(row: SqliteRow) -> Result<ExecutionContext> {
    let kind: String = row.try_get("kind")?;
    Ok(ExecutionContext {
        id: row.try_get("id")?,
        kind: ExecutionContextKind::from_storage(&kind)?,
        label: row.try_get("label")?,
        config_json: row.try_get("config_json")?,
        capabilities_json: row.try_get("capabilities_json")?,
        last_probe_at: row.try_get("last_probe_at")?,
        last_probe_status: row.try_get("last_probe_status")?,
        last_probe_error: row.try_get("last_probe_error")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn run_from_row(row: SqliteRow) -> Result<RunRecord> {
    let status: String = row.try_get("status")?;
    Ok(RunRecord {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        frame_id: row.try_get("frame_id")?,
        context_id: row.try_get("context_id")?,
        title: row.try_get("title")?,
        kind: row.try_get("kind")?,
        status: RunStatus::from_storage(&status)?,
        command: row.try_get("command")?,
        script_path: row.try_get("script_path")?,
        input_refs_json: row.try_get("input_refs_json")?,
        output_specs_json: row.try_get("output_specs_json")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
        exit_code: row.try_get("exit_code")?,
        stdout_tail: row.try_get("stdout_tail")?,
        stderr_tail: row.try_get("stderr_tail")?,
        remote_workdir: row.try_get("remote_workdir")?,
        remote_handle_json: row.try_get("remote_handle_json")?,
        timeout_secs: row.try_get("timeout_secs")?,
        last_polled_at: row.try_get("last_polled_at")?,
        last_poll_error: row.try_get("last_poll_error")?,
        env_snapshot_json: row.try_get("env_snapshot_json")?,
    })
}

pub(crate) fn artifact_version_from_row(row: SqliteRow) -> Result<ArtifactVersion> {
    Ok(ArtifactVersion {
        id: row.try_get("id")?,
        artifact_id: row.try_get("artifact_id")?,
        version_number: row.try_get("version_number")?,
        content_type: row.try_get("content_type")?,
        storage_path: row.try_get("storage_path")?,
        size_bytes: row.try_get("size_bytes")?,
        checksum: row.try_get("checksum")?,
        parent_version_id: row.try_get("parent_version_id")?,
        producing_run_id: row.try_get("producing_run_id")?,
        env_snapshot_hash: row.try_get("env_snapshot_hash")?,
        created_at: row.try_get("created_at")?,
    })
}

pub(crate) fn lab_registry_from_row(row: SqliteRow) -> Result<LabRegistry> {
    Ok(LabRegistry {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        root_path: row.try_get("root_path")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn lab_entity_from_row(row: SqliteRow) -> Result<LabEntity> {
    let kind: String = row.try_get("kind")?;
    Ok(LabEntity {
        id: row.try_get("id")?,
        registry_id: row.try_get("registry_id")?,
        display_id: row.try_get("display_id")?,
        kind: LabEntityKind::from_storage(&kind)?,
        subtype: row.try_get("subtype")?,
        title: row.try_get("title")?,
        revision: row.try_get("revision")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn lab_transaction_from_row(row: SqliteRow) -> Result<LabTransaction> {
    let actor_kind: String = row.try_get("actor_kind")?;
    let status: String = row.try_get("status")?;
    Ok(LabTransaction {
        id: row.try_get("id")?,
        display_id: row.try_get("display_id")?,
        registry_id: row.try_get("registry_id")?,
        project_id: row.try_get("project_id")?,
        command_id: row.try_get("command_id")?,
        schema_version: row.try_get("schema_version")?,
        actor_kind: LabActorKind::from_storage(&actor_kind)?,
        actor_ref: row.try_get("actor_ref")?,
        confirmation_json: row.try_get("confirmation_json")?,
        request_json: row.try_get("request_json")?,
        receipt_json: row.try_get("receipt_json")?,
        status: LabTransactionStatus::from_storage(&status)?,
        created_at: row.try_get("created_at")?,
        committed_at: row.try_get("committed_at")?,
    })
}

pub(crate) fn lab_event_from_row(row: SqliteRow) -> Result<LabEvent> {
    Ok(LabEvent {
        id: row.try_get("id")?,
        registry_id: row.try_get("registry_id")?,
        project_id: row.try_get("project_id")?,
        transaction_id: row.try_get("transaction_id")?,
        sequence: row.try_get("sequence")?,
        entity_id: row.try_get("entity_id")?,
        prior_event_id: row.try_get("prior_event_id")?,
        kind: row.try_get("kind")?,
        schema_version: row.try_get("schema_version")?,
        payload_json: row.try_get("payload_json")?,
        occurred_at: row.try_get("occurred_at")?,
        recorded_at: row.try_get("recorded_at")?,
        expected_revision: row.try_get("expected_revision")?,
        resulting_revision: row.try_get("resulting_revision")?,
        reason: row.try_get("reason")?,
    })
}

pub(crate) fn run_node_id(run_id: &str) -> String {
    format!("run:{run_id}")
}

pub(crate) fn artifact_node_id(artifact_id: &str) -> String {
    format!("artifact:{artifact_id}")
}

pub(crate) fn research_node_from_row(row: SqliteRow) -> Result<ResearchNode> {
    let kind: String = row.try_get("kind")?;
    Ok(ResearchNode {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        kind: ResearchNodeKind::from_storage(&kind)?,
        title: row.try_get("title")?,
        ref_id: row.try_get("ref_id")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub(crate) fn research_edge_from_row(row: SqliteRow) -> Result<ResearchEdge> {
    Ok(ResearchEdge {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        source_id: row.try_get("source_id")?,
        target_id: row.try_get("target_id")?,
        relation: row.try_get("relation")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: row.try_get("created_at")?,
    })
}

pub(crate) fn validate_run_transition(from: RunStatus, to: RunStatus) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let ok = match from {
        RunStatus::Draft => matches!(
            to,
            RunStatus::Submitted | RunStatus::Running | RunStatus::Cancelled
        ),
        RunStatus::Submitted => matches!(
            to,
            RunStatus::Running
                | RunStatus::Cancelling
                | RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::TimedOut
                | RunStatus::Lost
        ),
        RunStatus::Running => matches!(
            to,
            RunStatus::Cancelling
                | RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::TimedOut
                | RunStatus::Lost
        ),
        RunStatus::Cancelling => matches!(
            to,
            RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::TimedOut
                | RunStatus::Lost
        ),
        RunStatus::Succeeded
        | RunStatus::Failed
        | RunStatus::Cancelled
        | RunStatus::TimedOut
        | RunStatus::Lost => false,
    };
    if ok {
        Ok(())
    } else {
        anyhow::bail!(
            "Invalid run status transition: {} -> {}",
            from.as_str(),
            to.as_str()
        )
    }
}

pub(crate) fn session_display_title(
    custom_title: Option<String>,
    first_user: Option<String>,
) -> String {
    if let Some(t) = custom_title {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    first_user
        .and_then(|c| serde_json::from_str::<wisp_llm::Content>(&c).ok())
        .map(|c| c.as_text().chars().take(80).collect::<String>())
        .unwrap_or_default()
}

pub(crate) fn parse_role(s: &str) -> wisp_llm::Role {
    match s {
        "system" => wisp_llm::Role::System,
        "user" => wisp_llm::Role::User,
        "assistant" => wisp_llm::Role::Assistant,
        "tool" => wisp_llm::Role::Tool,
        _ => wisp_llm::Role::User,
    }
}
