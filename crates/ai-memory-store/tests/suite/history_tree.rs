//! History isolation, source invalidation, byte bounds, and concurrent publication.

use ai_memory_core::{NewPage, PagePath, ProjectId, Tier, WorkspaceId};
use ai_memory_store::{Store, history::HistorySpan};

async fn scope(store: &Store, workspace: &str, project: &str) -> (WorkspaceId, ProjectId) {
    let ws = store
        .writer
        .get_or_create_workspace(workspace)
        .await
        .unwrap();
    let proj = store
        .writer
        .get_or_create_project(ws, project, None)
        .await
        .unwrap();
    (ws, proj)
}

fn page((workspace_id, project_id): (WorkspaceId, ProjectId), path: &str, body: &str) -> NewPage {
    NewPage {
        workspace_id,
        project_id,
        path: PagePath::new(path).unwrap(),
        title: path.into(),
        body: body.into(),
        tier: Tier::Semantic,
        frontmatter_json: serde_json::json!({}),
        pinned: false,
        links: vec![],
        author_id: None,
        expires_at: None,
        entities: vec![],
        evidence: vec![],
    }
}

async fn compact(store: &Store) {
    for _ in 0..1024 {
        let Some(pair) = store.reader.history_pending_merge().await.unwrap() else {
            return;
        };
        let text = pair.extractive_summary();
        assert!(text.len() <= 2048);
        assert!(
            store
                .writer
                .publish_history(pair, text, false)
                .await
                .unwrap()
        );
    }
    panic!("compaction did not settle");
}

#[tokio::test]
async fn history_refuses_foreign_scopes_and_internal_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    let home = scope(&store, "home", "app").await;
    let sibling = scope(&store, "home", "other").await;
    let foreign = scope(&store, "elsewhere", "app").await;
    for (scope, text) in [
        (home, "HOME_CANARY"),
        (sibling, "SIBLING_SECRET"),
        (foreign, "FOREIGN_SECRET"),
    ] {
        for path in ["notes/a.md", "notes/b.md"] {
            store
                .writer
                .upsert_page(page(scope, path, text))
                .await
                .unwrap();
        }
    }
    store
        .writer
        .upsert_page(page(home, "_slots/private.md", "PRIVATE_SLOT"))
        .await
        .unwrap();
    compact(&store).await;
    let wake = store
        .reader
        .history_wake(home.0, home.1, 8192)
        .await
        .unwrap();
    let json = serde_json::to_string(&wake).unwrap();
    assert!(json.contains("HOME_CANARY"));
    for secret in ["SIBLING_SECRET", "FOREIGN_SECRET", "PRIVATE_SLOT"] {
        assert!(!json.contains(secret));
        assert!(
            store
                .reader
                .history_recall(home.0, home.1, secret.into(), -1, 10)
                .await
                .unwrap()
                .hits
                .is_empty()
        );
    }
    let zoom = store
        .reader
        .history_zoom(home.0, home.1, HistorySpan::parse("0-1").unwrap())
        .await
        .unwrap();
    assert_eq!(zoom.len(), 2);
    assert!(zoom.iter().all(|e| e.summary.contains("HOME_CANARY")));
    // A real project id under the wrong workspace must not reveal anything.
    assert!(
        store
            .reader
            .history_zoom(foreign.0, home.1, HistorySpan::parse("0").unwrap())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .reader
            .history_wake(foreign.0, home.1, 8192)
            .await
            .unwrap()
            .live_leaves,
        0
    );
    assert!(
        store
            .reader
            .history_recall(foreign.0, home.1, "HOME_CANARY".into(), -1, 10)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
}

#[tokio::test]
async fn history_edits_delete_and_expiry_invalidate_ancestors_and_stale_jobs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    let home = scope(&store, "home", "app").await;
    for i in 0..8 {
        store
            .writer
            .upsert_page(page(home, &format!("notes/{i}.md"), "old canary"))
            .await
            .unwrap();
    }
    let stale = store.reader.history_pending_merge().await.unwrap().unwrap();
    store
        .writer
        .upsert_page(page(home, "notes/0.md", "corrected source"))
        .await
        .unwrap();
    assert!(
        !store
            .writer
            .publish_history(stale, "stale summary".into(), true)
            .await
            .unwrap()
    );
    compact(&store).await;
    let children = store
        .reader
        .history_zoom(home.0, home.1, HistorySpan::parse("0-7").unwrap())
        .await
        .unwrap();
    assert_eq!(children.len(), 2);
    assert!(children[0].summary.contains("corrected source"));
    // Editing one leaf invalidates only its ancestors; the sibling survives.
    store
        .writer
        .upsert_page(page(home, "notes/0.md", "new correction"))
        .await
        .unwrap();
    let children = store
        .reader
        .history_zoom(home.0, home.1, HistorySpan::parse("0-7").unwrap())
        .await
        .unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].node_id, "4-7");
    compact(&store).await;
    store
        .writer
        .delete_page(home.0, home.1, PagePath::new("notes/0.md").unwrap(), None)
        .await
        .unwrap();
    assert!(
        store
            .reader
            .history_zoom(home.0, home.1, HistorySpan::parse("0").unwrap())
            .await
            .unwrap()
            .is_empty()
    );
    let mut expired = page(home, "notes/4.md", "expired canary");
    expired.expires_at = Some(jiff::Timestamp::now() - std::time::Duration::from_secs(1));
    store.writer.upsert_page(expired).await.unwrap();
    let json = serde_json::to_string(
        &store
            .reader
            .history_wake(home.0, home.1, 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!json.contains("expired canary"));
    assert!(
        !store
            .reader
            .history_zoom(home.0, home.1, HistorySpan::parse("0-7").unwrap())
            .await
            .unwrap()
            .iter()
            .any(|e| e.node_id == "4-7")
    );
}

#[tokio::test]
async fn history_is_bounded_with_escaping_backlog_and_restarts() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    let home = scope(&store, "home", "app").await;
    for i in 0..32 {
        store
            .writer
            .upsert_page(page(
                home,
                &format!("notes/{i}.md"),
                &"🦀\"\\\n".repeat(600),
            ))
            .await
            .unwrap();
    }
    let wake = store
        .reader
        .history_wake(home.0, home.1, 1024)
        .await
        .unwrap();
    assert!(serde_json::to_vec(&wake).unwrap().len() <= 1024);
    assert!(!wake.complete);
    compact(&store).await;
    let wake = store
        .reader
        .history_wake(home.0, home.1, 8192)
        .await
        .unwrap();
    assert!(serde_json::to_vec(&wake).unwrap().len() <= 8192);
    assert!(wake.complete);
    assert_eq!(wake.covered_leaves, 32);
    drop(store);
    let reopened = Store::open(tmp.path()).unwrap();
    assert!(
        reopened
            .reader
            .history_pending_merge()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        reopened
            .reader
            .history_wake(home.0, home.1, 8192)
            .await
            .unwrap()
            .complete
    );
}

#[tokio::test]
async fn history_recall_searches_beyond_leaf_clips_and_paginates() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    let home = scope(&store, "home", "app").await;
    for i in 0..130 {
        let body = if i == 129 {
            format!("{}ExactSymbol", "x".repeat(5000))
        } else {
            "unrelated".into()
        };
        store
            .writer
            .upsert_page(page(home, &format!("notes/{i}.md"), &body))
            .await
            .unwrap();
    }
    let first = store
        .reader
        .history_recall(home.0, home.1, "ExactSymbol".into(), -1, 10)
        .await
        .unwrap();
    assert!(first.hits.is_empty());
    assert_eq!(first.scanned_positions, 128);
    let second = store
        .reader
        .history_recall(
            home.0,
            home.1,
            "ExactSymbol".into(),
            first.next_after.unwrap(),
            10,
        )
        .await
        .unwrap();
    assert_eq!(second.hits.len(), 1);
    assert!(second.hits[0].summary.contains("ExactSymbol"));
    assert!(second.next_after.is_none());
    assert!(
        store
            .reader
            .history_recall(home.0, home.1, "exactsymbol".into(), 127, 10)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
}

#[tokio::test]
async fn history_concurrent_publish_accepts_one_and_preserves_shared_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).unwrap();
    let home = scope(&store, "home", "app").await;
    let (a, b) = tokio::join!(
        store
            .writer
            .upsert_page(page(home, "notes/a.md", "operator A")),
        store
            .writer
            .upsert_page(page(home, "notes/b.md", "operator B")),
    );
    a.unwrap();
    b.unwrap();
    let pair = store.reader.history_pending_merge().await.unwrap().unwrap();
    let (a, b) = tokio::join!(
        store
            .writer
            .publish_history(pair.clone(), pair.extractive_summary(), false),
        store
            .writer
            .publish_history(pair.clone(), pair.extractive_summary(), false),
    );
    assert_ne!(a.unwrap(), b.unwrap());
    let wake = store
        .reader
        .history_wake(home.0, home.1, 8192)
        .await
        .unwrap();
    let json = serde_json::to_string(&wake).unwrap();
    assert!(json.contains("operator A") && json.contains("operator B"));
    for invalid in ["1-4", "2-5", "-1", "0-9223372036854775807", "0-2"] {
        assert!(HistorySpan::parse(invalid).is_none(), "{invalid}");
    }
}
