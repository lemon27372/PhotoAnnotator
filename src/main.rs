// PhotoAnnotator 入口（第一阶段：环境验证）
//
// 目标：验证"窗口 + 渲染 + 存储"三件套
//   1. Slint     — 最小主窗口
//   2. TinySkia  — 画布渲染自检（输出测试图）
//   3. SQLite    — 初始化 4 张表（workspaces/files/annotations/thumbnails）
//
// 后续阶段按 doc/开发规范.md 目录结构拆分模块：
//   ui / workspace / canvas / storage / system / util

mod canvas;
mod storage;

use slint::SharedString;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 存储：初始化数据库（4 张表）
    let db = storage::init()?;
    println!("[storage] SQLite 初始化完成: {}", db.display());

    // 2. 渲染：TinySkia 画布自检（输出测试图到 target/）
    let out = canvas::render_self_test()?;
    println!("[canvas] TinySkia 自检通过: {}", out.display());

    // 3. 窗口：启动 Slint 主窗口
    let app = AppWindow::new()?;
    app.set_status(SharedString::from("窗口 + 渲染 + 存储 三件套就绪"));
    println!("[ui] Slint 窗口启动");
    app.run()?;

    Ok(())
}
