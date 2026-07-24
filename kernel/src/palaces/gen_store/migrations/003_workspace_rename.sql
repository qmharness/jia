-- 003_workspace_rename — 存量库(v1)概念重命名:project → workspace。
-- 一目录=一工作区；"project" 名称留给未来"工作区内创建的项目"。
-- 所有迁移对所有库执行,语句须自行幂等;新库(v0 经 001/002 已建新名)上本文件全部失败属预期噪音,仅存量库(v1)的失败阻止封版。
-- 执行方:gen_store/mod.rs 按分号切分逐条 execute,顺序即迁移顺序。
-- 注意:本文件的注释里不得出现分号(会破坏切分边界)。

ALTER TABLE projects RENAME TO workspaces;

ALTER TABLE sessions RENAME COLUMN project_id TO workspace_id;

ALTER TABLE seeds RENAME COLUMN project_id TO workspace_id;

DROP INDEX IF EXISTS idx_seeds_project;

CREATE INDEX IF NOT EXISTS idx_seeds_workspace ON seeds(workspace_id);
