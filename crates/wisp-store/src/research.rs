use super::{
    research_edge_from_row, research_node_from_row, ResearchEdge, ResearchGraph, ResearchNode,
    ResearchNodeKind, StateScope, Store,
};
use anyhow::Result;

impl Store {
    pub async fn save_research_node(&self, node: &ResearchNode) -> Result<()> {
        if let Some(store) = self.route_project(&node.project_id).await? {
            return Box::pin(store.save_research_node(node)).await;
        }
        self.save_research_node_in_scope(node, &StateScope::mainline(&node.project_id))
            .await
    }

    pub async fn save_research_node_in_scope(
        &self,
        node: &ResearchNode,
        scope: &StateScope,
    ) -> Result<()> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.save_research_node_in_scope(node, scope)).await;
        }
        node.validate()?;
        if scope.project_id() != node.project_id {
            anyhow::bail!("Research node scope does not belong to its project");
        }
        let exploration_id = exploration_id(scope);
        let mut tx = self.begin_write().await?;
        let existing_scope: Option<Option<String>> =
            sqlx::query_scalar("SELECT exploration_id FROM research_nodes WHERE id=?")
                .bind(&node.id)
                .fetch_optional(&mut *tx)
                .await?;
        if existing_scope.is_some_and(|existing| existing.as_deref() != exploration_id) {
            anyhow::bail!("Research node cannot move between state scopes");
        }
        sqlx::query(
            "INSERT INTO research_nodes(\
               id,project_id,kind,title,ref_id,metadata_json,exploration_id,created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
                project_id=excluded.project_id,kind=excluded.kind,title=excluded.title,\
                ref_id=excluded.ref_id,metadata_json=excluded.metadata_json,updated_at=excluded.updated_at",
        )
        .bind(&node.id)
        .bind(&node.project_id)
        .bind(node.kind.as_str())
        .bind(&node.title)
        .bind(node.ref_id.as_deref())
        .bind(&node.metadata_json)
        .bind(exploration_id)
        .bind(node.created_at)
        .bind(node.updated_at)
        .execute(&mut *tx)
        .await?;
        self.bump_state_generation_in_tx(&mut tx, scope).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_research_nodes(
        &self,
        project_id: &str,
        kind: Option<ResearchNodeKind>,
    ) -> Result<Vec<ResearchNode>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_research_nodes(project_id, kind)).await;
        }
        let rows = if let Some(kind) = kind {
            sqlx::query(
                "SELECT id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at \
                 FROM research_nodes WHERE project_id=? AND exploration_id IS NULL AND kind=? \
                 ORDER BY created_at,id",
            )
            .bind(project_id)
            .bind(kind.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at \
                 FROM research_nodes WHERE project_id=? AND exploration_id IS NULL \
                 ORDER BY created_at,id",
            )
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(research_node_from_row).collect()
    }

    pub async fn save_research_edge(&self, edge: &ResearchEdge) -> Result<()> {
        if let Some(store) = self.route_project(&edge.project_id).await? {
            return Box::pin(store.save_research_edge(edge)).await;
        }
        self.save_research_edge_in_scope(edge, &StateScope::mainline(&edge.project_id))
            .await
    }

    pub async fn save_research_edge_in_scope(
        &self,
        edge: &ResearchEdge,
        scope: &StateScope,
    ) -> Result<()> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.save_research_edge_in_scope(edge, scope)).await;
        }
        edge.validate()?;
        if scope.project_id() != edge.project_id {
            anyhow::bail!("Research edge scope does not belong to its project");
        }
        let mut tx = self.begin_write().await?;
        let endpoints: (i64,) = match scope {
            StateScope::Mainline { .. } => sqlx::query_as(
                "SELECT COUNT(*) FROM research_nodes \
                 WHERE project_id=? AND exploration_id IS NULL AND id IN (?,?)",
            )
            .bind(&edge.project_id)
            .bind(&edge.source_id)
            .bind(&edge.target_id)
            .fetch_one(&mut *tx)
            .await?,
            StateScope::Exploration { exploration_id, .. } => sqlx::query_as(
                "SELECT COUNT(*) FROM research_nodes node WHERE node.project_id=? AND node.id IN (?,?) \
                 AND (node.exploration_id=? OR (node.exploration_id IS NULL AND EXISTS(\
                   SELECT 1 FROM explorations exploration \
                   JOIN exploration_baseline_entities baseline \
                     ON baseline.checkpoint_id=exploration.checkpoint_id \
                   WHERE exploration.id=? AND baseline.entity_kind='research_node' \
                     AND baseline.entity_id=node.id)))",
            )
            .bind(&edge.project_id)
            .bind(&edge.source_id)
            .bind(&edge.target_id)
            .bind(exploration_id)
            .bind(exploration_id)
            .fetch_one(&mut *tx)
            .await?,
        };
        if endpoints.0 != 2 {
            anyhow::bail!("Research edge endpoints must be visible in the same state scope");
        }
        let exploration_id = exploration_id(scope);
        let existing_scope: Option<Option<String>> =
            sqlx::query_scalar("SELECT exploration_id FROM research_edges WHERE id=?")
                .bind(&edge.id)
                .fetch_optional(&mut *tx)
                .await?;
        if existing_scope.is_some_and(|existing| existing.as_deref() != exploration_id) {
            anyhow::bail!("Research edge cannot move between state scopes");
        }
        sqlx::query(
            "INSERT INTO research_edges(\
               id,project_id,source_id,target_id,relation,metadata_json,exploration_id,created_at\
             ) VALUES(?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
                project_id=excluded.project_id,source_id=excluded.source_id,\
                target_id=excluded.target_id,relation=excluded.relation,metadata_json=excluded.metadata_json",
        )
        .bind(&edge.id)
        .bind(&edge.project_id)
        .bind(&edge.source_id)
        .bind(&edge.target_id)
        .bind(&edge.relation)
        .bind(&edge.metadata_json)
        .bind(exploration_id)
        .bind(edge.created_at)
        .execute(&mut *tx)
        .await?;
        self.bump_state_generation_in_tx(&mut tx, scope).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_research_edges(&self, project_id: &str) -> Result<Vec<ResearchEdge>> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.list_research_edges(project_id)).await;
        }
        let rows = sqlx::query(
            "SELECT id,project_id,source_id,target_id,relation,metadata_json,created_at \
             FROM research_edges WHERE project_id=? AND exploration_id IS NULL \
             ORDER BY created_at,id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(research_edge_from_row).collect()
    }

    pub async fn research_graph(&self, project_id: &str) -> Result<ResearchGraph> {
        if let Some(store) = self.route_project(project_id).await? {
            return Box::pin(store.research_graph(project_id)).await;
        }
        Ok(ResearchGraph {
            nodes: self.list_research_nodes(project_id, None).await?,
            edges: self.list_research_edges(project_id).await?,
        })
    }

    pub async fn research_graph_in_scope(&self, scope: &StateScope) -> Result<ResearchGraph> {
        if let Some(store) = self.route_project(scope.project_id()).await? {
            return Box::pin(store.research_graph_in_scope(scope)).await;
        }
        let StateScope::Exploration {
            project_id,
            exploration_id,
        } = scope
        else {
            return self.research_graph(scope.project_id()).await;
        };
        let node_rows = sqlx::query(
            "SELECT node.id,node.project_id,node.kind,node.title,node.ref_id,node.metadata_json,\
                    node.created_at,node.updated_at FROM research_nodes node \
             WHERE node.project_id=? AND (node.exploration_id=? OR (node.exploration_id IS NULL \
               AND EXISTS(SELECT 1 FROM explorations exploration \
                 JOIN exploration_baseline_entities baseline \
                   ON baseline.checkpoint_id=exploration.checkpoint_id \
                 WHERE exploration.id=? AND baseline.entity_kind='research_node' \
                   AND baseline.entity_id=node.id))) ORDER BY node.created_at,node.id",
        )
        .bind(project_id)
        .bind(exploration_id)
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        let edge_rows = sqlx::query(
            "SELECT edge.id,edge.project_id,edge.source_id,edge.target_id,edge.relation,\
                    edge.metadata_json,edge.created_at FROM research_edges edge \
             WHERE edge.project_id=? AND (edge.exploration_id=? OR (edge.exploration_id IS NULL \
               AND EXISTS(SELECT 1 FROM explorations exploration \
                 JOIN exploration_baseline_entities baseline \
                   ON baseline.checkpoint_id=exploration.checkpoint_id \
                 WHERE exploration.id=? AND baseline.entity_kind='research_edge' \
                   AND baseline.entity_id=edge.id))) ORDER BY edge.created_at,edge.id",
        )
        .bind(project_id)
        .bind(exploration_id)
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(ResearchGraph {
            nodes: node_rows
                .into_iter()
                .map(research_node_from_row)
                .collect::<Result<Vec<_>>>()?,
            edges: edge_rows
                .into_iter()
                .map(research_edge_from_row)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn research_graph_owned_by_exploration(
        &self,
        exploration_id: &str,
    ) -> Result<ResearchGraph> {
        if let Some(store) = self
            .route_entity("explorations", "id", exploration_id)
            .await?
        {
            return Box::pin(store.research_graph_owned_by_exploration(exploration_id)).await;
        }
        let node_rows = sqlx::query(
            "SELECT id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at \
             FROM research_nodes WHERE exploration_id=? ORDER BY created_at,id",
        )
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        let edge_rows = sqlx::query(
            "SELECT id,project_id,source_id,target_id,relation,metadata_json,created_at \
             FROM research_edges WHERE exploration_id=? ORDER BY created_at,id",
        )
        .bind(exploration_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(ResearchGraph {
            nodes: node_rows
                .into_iter()
                .map(research_node_from_row)
                .collect::<Result<Vec<_>>>()?,
            edges: edge_rows
                .into_iter()
                .map(research_edge_from_row)
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

fn exploration_id(scope: &StateScope) -> Option<&str> {
    match scope {
        StateScope::Mainline { .. } => None,
        StateScope::Exploration { exploration_id, .. } => Some(exploration_id),
    }
}
