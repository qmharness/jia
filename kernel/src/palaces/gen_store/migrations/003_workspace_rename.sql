-- 003_workspace_rename — 存量库(v1)概念重命名:project → workspace。
-- 一目录=一工作区；"project" 名称留给未来"工作区内创建的项目"。
-- 新库(v0)不会执行本文件(001/002 已直接以新名建表)。
-- 执行方:gen_store/mod.rs 按分号切分逐条 execute,顺序即迁移顺序。
-- 注意:本文件的注释里不得出现分号(会破坏切分边界)。

ALTER TABLE projects RENAME TO workspaces;

ALTER TABLE sessions RENAME COLUMN project_id TO workspace_id;

ALTER TABLE seeds RENAME COLUMN project_id TO workspace_id;

DROP INDEX IF EXISTS idx_seeds_project;

CREATE INDEX IF NOT EXISTS idx_seeds_workspace ON seeds(workspace_id);
