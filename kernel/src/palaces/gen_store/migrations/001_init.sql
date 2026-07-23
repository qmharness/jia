-- 001_init — 基础 schema(user_version 0 → 1 时整体执行,execute_batch)
-- 从 gen_store/mod.rs 内联 DDL 抽出(L2),内容与原内联版本逐字一致。

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    messages_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS seeds (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    nature TEXT NOT NULL,
    source TEXT NOT NULL,
    content_type TEXT NOT NULL,
    content_json TEXT NOT NULL,
    palace INTEGER NOT NULL,
    intent_stem INTEGER NOT NULL,
    geju_key TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    access_count INTEGER NOT NULL DEFAULT 0,
    last_accessed_at INTEGER NOT NULL DEFAULT 0,
    strength REAL NOT NULL DEFAULT 1.0,
    tier TEXT NOT NULL DEFAULT 'OnDemand',
    content_text TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_seeds_session ON seeds(session_id);
CREATE INDEX IF NOT EXISTS idx_seeds_palace ON seeds(palace);
CREATE INDEX IF NOT EXISTS idx_seeds_intent ON seeds(intent_stem);
CREATE INDEX IF NOT EXISTS idx_seeds_geju ON seeds(geju_key);
CREATE INDEX IF NOT EXISTS idx_seeds_strength ON seeds(strength);

CREATE TABLE IF NOT EXISTS manas (
    session_id TEXT PRIMARY KEY,
    atma_graha REAL NOT NULL,
    total_turns INTEGER NOT NULL DEFAULT 0,
    consolidation_count INTEGER NOT NULL DEFAULT 0,
    stable_pattern_count INTEGER NOT NULL DEFAULT 0,
    last_consolidation_at INTEGER NOT NULL DEFAULT 0,
    stable_epochs INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS principles (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    geju_key TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'session',
    constraint_type TEXT NOT NULL,
    constraint_json TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.5,
    source_seed_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_principles_session ON principles(session_id);
CREATE INDEX IF NOT EXISTS idx_principles_geju ON principles(geju_key);
CREATE INDEX IF NOT EXISTS idx_principles_scope ON principles(scope);

CREATE VIRTUAL TABLE IF NOT EXISTS seeds_fts USING fts5(
    id UNINDEXED,
    content_text,
    tokenize='unicode61'
);

CREATE TABLE IF NOT EXISTS dissolution_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    seeds_examined INTEGER NOT NULL,
    seeds_dissolved INTEGER NOT NULL,
    seeds_weakened INTEGER NOT NULL,
    seeds_downgraded INTEGER NOT NULL,
    entropy_before REAL NOT NULL,
    entropy_after REAL NOT NULL,
    entropy_dimensions_json TEXT NOT NULL,
    score_kept INTEGER NOT NULL,
    score_protected INTEGER NOT NULL,
    dissolved_sample_json TEXT NOT NULL
);
