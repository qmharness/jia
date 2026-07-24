-- 002_alters — 增量迁移(逐条独立容错执行:ALTER 列已存在时失败无害,
-- CREATE 均带 IF NOT EXISTS),与原内联 let _ = conn.execute(...) 语义一致。
-- 执行方:gen_store/mod.rs 按分号切分逐条 execute,顺序即迁移顺序。
-- 注意:本文件的注释里不得出现分号(会破坏切分边界)。

-- Migrate: remove old tool_events_json column (now merged into messages_json)
ALTER TABLE sessions DROP COLUMN tool_events_json;

-- Migrate: add title column (user-customizable session title, overrides derived)
ALTER TABLE sessions ADD COLUMN title TEXT;

-- Migrate: add stable_epochs to manas table (agent stability tracking)
ALTER TABLE manas ADD COLUMN stable_epochs INTEGER NOT NULL DEFAULT 0;

-- Migrate: rename ego_grasp → atma_graha (ātma-grāha)
ALTER TABLE manas RENAME COLUMN ego_grasp TO atma_graha;

-- Migrate: persist distilled thought hashes to avoid redundant LLM calls
ALTER TABLE sessions ADD COLUMN distilled_hashes_json TEXT NOT NULL DEFAULT '[]';

-- Migrate: archive support (0=active, 1=archived)
ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;

-- Migrate: add content_text column for FTS5 indexing
ALTER TABLE seeds ADD COLUMN content_text TEXT NOT NULL DEFAULT '';

-- Migrate: add tier column for seed injection policy (existence encoding)
ALTER TABLE seeds ADD COLUMN tier TEXT NOT NULL DEFAULT 'OnDemand';

-- Migrate: add cwd column for project/workspace tracking
ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT '';

-- Migrate: workspaces table
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    cwd TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    tags_json TEXT NOT NULL DEFAULT '[]',
    archived INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Migrate: add name/desc/tags columns if upgrading from old schema
ALTER TABLE workspaces ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE workspaces ADD COLUMN description TEXT NOT NULL DEFAULT '';
ALTER TABLE workspaces ADD COLUMN tags_json TEXT NOT NULL DEFAULT '[]';

-- Migrate: add workspace_id to sessions
ALTER TABLE sessions ADD COLUMN workspace_id TEXT REFERENCES workspaces(id);

-- Migrate: stamp workspace_id on seeds for same-project recall bias and
-- per-project filtering. '' = global/legacy seed (no project affiliation).
ALTER TABLE seeds ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_seeds_workspace ON seeds(workspace_id);

-- Create tier_access index after migration ensures the column exists
CREATE INDEX IF NOT EXISTS idx_seeds_tier_access ON seeds(tier, access_count);

-- Index for profile seed queries (nature + content_type filtered)
CREATE INDEX IF NOT EXISTS idx_seeds_nature_content ON seeds(nature, content_type);

-- Migrate: create FTS5 virtual table for semantic search
CREATE VIRTUAL TABLE IF NOT EXISTS seeds_fts USING fts5(
    id UNINDEXED,
    content_text,
    tokenize='unicode61'
);

-- Migrate: skill evolution tables (Phase 0)
CREATE TABLE IF NOT EXISTS skill_reflections (
    id TEXT PRIMARY KEY,
    skill_name TEXT NOT NULL,
    session_id TEXT NOT NULL,
    reflection_type TEXT NOT NULL,
    content_json TEXT NOT NULL,
    confidence REAL NOT NULL,
    turn_numbers TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_skill_reflections_skill ON skill_reflections(skill_name, session_id);

CREATE TABLE IF NOT EXISTS skill_revisions (
    id TEXT PRIMARY KEY,
    skill_name TEXT NOT NULL,
    session_id TEXT NOT NULL,
    old_content TEXT NOT NULL,
    new_content TEXT NOT NULL,
    diff_text TEXT NOT NULL,
    avg_confidence REAL NOT NULL,
    reflection_ids TEXT NOT NULL,
    pre_revision_error_rate REAL,
    post_revision_error_rate REAL,
    applied INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_skill_revisions_skill ON skill_revisions(skill_name);

-- Migrate: sub-agent session persistence (P1)
CREATE TABLE IF NOT EXISTS subagent_sessions (
    id TEXT PRIMARY KEY,
    messages_json TEXT NOT NULL,
    subagent_type TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used INTEGER NOT NULL
);

-- Migrate: principle archive support (0=active, 1=archived by user)
ALTER TABLE principles ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;

-- Migrate: manas history for atma-graha time series
CREATE TABLE IF NOT EXISTS manas_history (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    atma_graha REAL NOT NULL,
    entropy_total REAL NOT NULL,
    seed_count INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
