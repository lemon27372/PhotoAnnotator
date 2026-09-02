// 数据库初始化：创建 4 张核心表
// 表结构见 doc/技术规范.md「存储方案（SQLite）」

use rusqlite::Connection;
use std::path::PathBuf;

/// 数据库路径：%APPDATA%/PhotoAnnotator/annotator.db
pub fn db_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("PhotoAnnotator").join("annotator.db")
}

/// 初始化数据库（建目录 + 建 4 张表），幂等
pub fn init() -> rusqlite::Result<PathBuf> {
    let path = db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("创建数据库目录失败");
    }

    let conn = Connection::open(&path)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS workspaces (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            path        TEXT NOT NULL UNIQUE,
            last_opened INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS files (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
            path         TEXT NOT NULL,
            width        INTEGER,
            height       INTEGER,
            modified_at  INTEGER,
            status       TEXT NOT NULL DEFAULT 'pending',  -- pending | done | ignored
            UNIQUE (workspace_id, path)
        );

        CREATE TABLE IF NOT EXISTS annotations (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            file_id    INTEGER NOT NULL REFERENCES files(id),
            data       TEXT NOT NULL,   -- 标注层 JSON（矢量路径 + 样式 + 序号）
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS thumbnails (
            file_id INTEGER PRIMARY KEY REFERENCES files(id),
            data    BLOB NOT NULL,      -- 低分辨率位图缓存
            created_at INTEGER NOT NULL
        );
        ",
    )?;

    Ok(path)
}
