// 存储层：SQLite 持久化
// 表结构见 doc/技术规范.md「存储方案（SQLite）」

pub mod db;

pub use db::init;
