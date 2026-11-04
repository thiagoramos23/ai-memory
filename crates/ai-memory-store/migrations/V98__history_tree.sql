-- Derived history positions survive page supersession; source bodies stay in pages.
CREATE TABLE history_leaves (
    workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK(seq >= 0),
    path TEXT NOT NULL,
    page_id BLOB REFERENCES pages(id) ON DELETE SET NULL,
    revision INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(workspace_id, project_id, seq),
    UNIQUE(workspace_id, project_id, path)
);
CREATE INDEX history_leaves_page ON history_leaves(page_id);
CREATE TABLE history_nodes (
    workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_id BLOB NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    lo INTEGER NOT NULL,
    hi INTEGER NOT NULL,
    revision TEXT NOT NULL,
    summary TEXT NOT NULL CHECK(length(CAST(summary AS BLOB)) <= 2048),
    method TEXT NOT NULL CHECK(method IN ('extractive', 'llm')),
    expires_at INTEGER,
    PRIMARY KEY(workspace_id, project_id, lo, hi),
    CHECK(hi > lo AND hi - lo >= 2 AND ((hi - lo) & (hi - lo - 1)) = 0
          AND lo % (hi - lo) = 0)
);

INSERT INTO history_leaves(workspace_id, project_id, seq, path, page_id)
SELECT workspace_id, project_id,
       ROW_NUMBER() OVER (PARTITION BY workspace_id, project_id ORDER BY created_at, path) - 1,
       path, id FROM pages WHERE is_latest = 1 AND substr(path, 1, 1) != '_';

CREATE TRIGGER history_leaf_changed AFTER UPDATE ON history_leaves BEGIN
    DELETE FROM history_nodes WHERE workspace_id = old.workspace_id
      AND project_id = old.project_id AND lo <= old.seq AND hi > old.seq;
END;
CREATE TRIGGER history_leaf_removed BEFORE DELETE ON history_leaves BEGIN
    DELETE FROM history_nodes WHERE workspace_id = old.workspace_id
      AND project_id = old.project_id AND lo <= old.seq AND hi > old.seq;
END;
CREATE TRIGGER history_page_added AFTER INSERT ON pages BEGIN

    INSERT INTO history_leaves(workspace_id, project_id, seq, path, page_id)
    SELECT new.workspace_id, new.project_id,
           COALESCE((SELECT MAX(seq) + 1 FROM history_leaves
                     WHERE workspace_id = new.workspace_id AND project_id = new.project_id), 0),
           new.path, new.id
    WHERE new.is_latest = 1 AND substr(new.path, 1, 1) != '_'
    ON CONFLICT(workspace_id, project_id, path) DO UPDATE
      SET page_id = excluded.page_id, revision = history_leaves.revision + 1;
END;
CREATE TRIGGER history_page_changed AFTER UPDATE OF
    is_latest, body, title, frontmatter_json, expires_at, workspace_id, project_id, path ON pages BEGIN
    UPDATE history_leaves SET page_id = NULL, revision = revision + 1 WHERE page_id = old.id;

    INSERT INTO history_leaves(workspace_id, project_id, seq, path, page_id)
    SELECT new.workspace_id, new.project_id,
           COALESCE((SELECT MAX(seq) + 1 FROM history_leaves
                     WHERE workspace_id = new.workspace_id AND project_id = new.project_id), 0),
           new.path, new.id
    WHERE new.is_latest = 1 AND substr(new.path, 1, 1) != '_'
    ON CONFLICT(workspace_id, project_id, path) DO UPDATE
      SET page_id = excluded.page_id, revision = history_leaves.revision + 1;
END;
