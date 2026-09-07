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

// ---------- 数据访问（每次操作独立打开连接，简化生命周期） ----------

fn open() -> rusqlite::Result<Connection> {
    Connection::open(db_path())
}

/// 记录/更新最近工作区，返回 workspace id
pub fn touch_workspace(path: &str) -> Option<i64> {
    let conn = open().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO workspaces (path, last_opened) VALUES (?1, ?2)
         ON CONFLICT(path) DO UPDATE SET last_opened = ?2",
        rusqlite::params![path, now],
    )
    .ok()?;
    conn.query_row("SELECT id FROM workspaces WHERE path = ?1", [path], |r| r.get(0)).ok()
}

/// 记录文件索引（含状态），返回 file id（幂等 upsert）
pub fn upsert_file(workspace_id: i64, path: &str, width: i32, height: i32) -> Option<i64> {
    let conn = open().ok()?;
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO files (workspace_id, path, width, height, modified_at, status)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending')
         ON CONFLICT(workspace_id, path) DO UPDATE SET modified_at = ?5",
        rusqlite::params![workspace_id, path, width, height, modified],
    )
    .ok()?;
    conn.query_row(
        "SELECT id FROM files WHERE workspace_id = ?1 AND path = ?2",
        rusqlite::params![workspace_id, path],
        |r| r.get(0),
    )
    .ok()
}

/// 读取文件状态（pending/done/ignored），未知文件返回 pending
pub fn file_status(workspace_id: i64, path: &str) -> String {
    let conn = match open() {
        Ok(c) => c,
        Err(_) => return "pending".into(),
    };
    conn.query_row(
        "SELECT status FROM files WHERE workspace_id = ?1 AND path = ?2",
        rusqlite::params![workspace_id, path],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| "pending".into())
}

/// 设置文件状态（标注完成 → done）
pub fn set_file_status(workspace_id: i64, path: &str, status: &str) {
    if let Ok(conn) = open() {
        let _ = conn.execute(
            "UPDATE files SET status = ?1 WHERE workspace_id = ?2 AND path = ?3",
            rusqlite::params![status, workspace_id, path],
        );
    }
}

/// 保存标注 JSON（覆盖式 upsert）
pub fn save_annotations(file_id: i64, json: &str) {
    if let Ok(conn) = open() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let _ = conn.execute(
            "INSERT INTO annotations (file_id, data, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(file_id) DO UPDATE SET data = ?2, updated_at = ?3",
            rusqlite::params![file_id, json, now],
        );
    }
}

/// 读取标注 JSON（无记录返回空串）
pub fn load_annotations(file_id: i64) -> String {
    let conn = match open() {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    conn.query_row(
        "SELECT data FROM annotations WHERE file_id = ?1",
        [file_id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_default()
}

/// 读取缩略图缓存 PNG
pub fn load_thumbnail(file_id: i64) -> Option<Vec<u8>> {
    let conn = open().ok()?;
    conn.query_row("SELECT data FROM thumbnails WHERE file_id = ?1", [file_id], |r| r.get(0)).ok()
}

/// 写入缩略图缓存
pub fn save_thumbnail(file_id: i64, png: &[u8]) {
    if let Ok(conn) = open() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let _ = conn.execute(
            "INSERT INTO thumbnails (file_id, data, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(file_id) DO UPDATE SET data = ?2, created_at = ?3",
            rusqlite::params![file_id, png, now],
        );
    }
}
