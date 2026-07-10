use super::{
    research_edge_from_row, research_node_from_row, ResearchEdge, ResearchGraph, ResearchNode,
    ResearchNodeKind, Store,
};
use anyhow::Result;

impl Store {
    pub async fn save_research_node(&self, node: &ResearchNode) -> Result<()> {
        node.validate()?;
        sqlx::query(
            "INSERT INTO research_nodes(id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
                project_id=excluded.project_id, kind=excluded.kind, title=excluded.title, \
                ref_id=excluded.ref_id, metadata_json=excluded.metadata_json, updated_at=excluded.updated_at",
        )
        .bind(&node.id)
        .bind(&node.project_id)
        .bind(node.kind.as_str())
        .bind(&node.title)
        .bind(node.ref_id.as_deref())
        .bind(&node.metadata_json)
        .bind(node.created_at)
        .bind(node.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_research_nodes(
        &self,
        project_id: &str,
        kind: Option<ResearchNodeKind>,
    ) -> Result<Vec<ResearchNode>> {
        let rows = if let Some(kind) = kind {
            sqlx::query(
                "SELECT id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at \
                 FROM research_nodes WHERE project_id=? AND kind=? ORDER BY created_at ASC, id ASC",
            )
            .bind(project_id)
            .bind(kind.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at \
                 FROM research_nodes WHERE project_id=? ORDER BY created_at ASC, id ASC",
            )
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(research_node_from_row).collect()
    }

    pub async fn save_research_edge(&self, edge: &ResearchEdge) -> Result<()> {
        edge.validate()?;
        let endpoints: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM research_nodes WHERE project_id=? AND id IN (?,?)",
        )
        .bind(&edge.project_id)
        .bind(&edge.source_id)
        .bind(&edge.target_id)
        .fetch_one(&self.pool)
        .await?;
        if endpoints.0 != 2 {
            anyhow::bail!("Research edge endpoints must belong to the same project");
        }
        sqlx::query(
            "INSERT INTO research_edges(id,project_id,source_id,target_id,relation,metadata_json,created_at) \
             VALUES(?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
                project_id=excluded.project_id, source_id=excluded.source_id, \
                target_id=excluded.target_id, relation=excluded.relation, metadata_json=excluded.metadata_json",
        )
        .bind(&edge.id)
        .bind(&edge.project_id)
        .bind(&edge.source_id)
        .bind(&edge.target_id)
        .bind(&edge.relation)
        .bind(&edge.metadata_json)
        .bind(edge.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_research_edges(&self, project_id: &str) -> Result<Vec<ResearchEdge>> {
        let rows = sqlx::query(
            "SELECT id,project_id,source_id,target_id,relation,metadata_json,created_at \
             FROM research_edges WHERE project_id=? ORDER BY created_at ASC, id ASC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(research_edge_from_row).collect()
    }

    pub async fn research_graph(&self, project_id: &str) -> Result<ResearchGraph> {
        Ok(ResearchGraph {
            nodes: self.list_research_nodes(project_id, None).await?,
            edges: self.list_research_edges(project_id).await?,
        })
    }
}
