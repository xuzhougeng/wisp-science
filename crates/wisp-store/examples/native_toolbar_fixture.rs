//! Create a fresh, synthetic native-toolbar QA store. Never opens user data.
//! cargo run -p wisp-store --example native_toolbar_fixture -- /tmp/new-qa-root
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use wisp_llm::{FunctionCall, Message, ToolCall};
use wisp_store::Store;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .context("Provide a new, nonexistent QA directory")?,
    );
    if args.next().is_some() {
        bail!("Usage: native_toolbar_fixture <new-directory>");
    }
    // Atomic refusal protects existing databases, including symlink targets.
    std::fs::create_dir(&root)
        .context("QA directory must not already exist; no files were replaced")?;
    // SQLite URLs must not receive Windows' verbatim \\?\ path prefix.
    let root = dunce::canonicalize(root)?;
    let database = root.join("wisp.sqlite");
    let store = Store::open(&database).await?;
    let now = chrono::Utc::now().timestamp();
    for (project, name) in [
        ("toolbar-qa", "原生工具栏验收"),
        ("toolbar-qa-other", "跨项目验收"),
    ] {
        let workspace = root.join(project);
        std::fs::create_dir(&workspace)?;
        std::fs::create_dir(workspace.join("results"))?;
        std::fs::write(
            workspace.join("README.md"),
            "# Synthetic native QA\n\n这些文件仅用于界面验收，不包含真实研究数据。\n",
        )?;
        std::fs::write(workspace.join("results/report.md"), "# 检查报告\n\n样本 **质量合格**。\n\n| 样本 | 数量 |\n| --- | ---: |\n| A | 12 |\n| B | 24 |\n")?;
        std::fs::write(
            workspace.join("results/counts.csv"),
            "sample,count\nA,12\nB,24\n",
        )?;
        store
            .create_project(project, name, &workspace.to_string_lossy())
            .await?;
        let session = format!("{project}-session");
        store
            .create_frame(&session, project, "Wisp", "qa-offline")
            .await?;
        let mut messages = Vec::new();
        for turn in 0..35 {
            messages.push(Message::user(if turn % 2 == 0 {
                "检查样本质量".into()
            } else {
                format!("第 {} 轮：解释结果", turn + 1)
            }));
            let mut assistant = Message::assistant(format!("第 {} 轮：样本 **质量合格**。\n\n```python\nprint('sample A', 12)\n```\n\n这是合成验收内容，不调用模型或远程环境。", turn + 1));
            if turn == 34 {
                assistant.tool_calls.push(ToolCall {
                    id: "qa-tool".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: "python".into(),
                        arguments: r#"{"code":"print('sample A', 12)"}"#.into(),
                    },
                });
            }
            messages.push(assistant);
            if turn == 34 {
                messages.push(Message::tool(
                    "qa-tool",
                    "python",
                    "sample A 12\n样本质量合格",
                ));
            }
        }
        for (index, message) in messages.iter_mut().enumerate() {
            message.ts = now - 600 + index as i64;
            store
                .append_message(&session, index as i64, message)
                .await?;
        }
        store
            .save_artifact(
                &format!("{project}-report"),
                project,
                &session,
                "report.md",
                "text/markdown",
                &workspace.join("results/report.md").to_string_lossy(),
            )
            .await?;
        store
            .save_artifact(
                &format!("{project}-counts"),
                project,
                &session,
                "counts.csv",
                "text/csv",
                &workspace.join("results/counts.csv").to_string_lossy(),
            )
            .await?;
        store
            .create_frame(&format!("{project}-empty"), project, "Wisp", "qa-offline")
            .await?;
    }
    // Fixture-only final state, through a connection to the newly created store.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&database))
        .await?;
    sqlx::query("UPDATE frames SET status='completed',completed_at=?,title=CASE WHEN id LIKE '%-empty' THEN '空会话验收' ELSE '35 轮工具栏验收' END")
        .bind(now).execute(&pool).await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&pool)
        .await?;
    pool.close().await;
    println!("Synthetic database: {}", database.display());
    println!("Projects: toolbar-qa, toolbar-qa-other; 35 turns and an empty conversation each.");
    println!("Use only with an isolated native host whose database matches this path.");
    Ok(())
}
