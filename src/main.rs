// PhotoAnnotator 入口（第二阶段：图片加载与画布显示）
//
// 用法：
//   photo_annotator                       # 空窗口
//   photo_annotator <图片路径>            # 加载图片并显示（PNG/JPG/BMP/GIF/WebP）
//
// 后续阶段按 doc/开发规范.md 目录结构拆分模块：
//   ui / workspace / canvas / storage / system / util

mod canvas;
mod storage;

use slint::SharedString;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 存储：初始化数据库（4 张表，幂等）
    let db = storage::init()?;
    println!("[storage] SQLite 初始化完成: {}", db.display());

    // 2. 窗口：启动 Slint 主窗口
    let app = AppWindow::new()?;
    let mut status = "未加载图片".to_string();

    // 3. 图片：命令行参数加载（第二阶段验证用，后续接入文件对话框/工作目录）
    if let Some(path) = std::env::args().nth(1) {
        match canvas::load_image(&path) {
            Ok(image) => {
                app.set_current_image(image);
                status = format!("已加载: {path}");
                println!("[canvas] 图片加载成功: {path}");
            }
            Err(e) => {
                status = format!("加载失败: {e}");
                eprintln!("[canvas] 图片加载失败: {path}: {e}");
            }
        }
    } else {
        println!("[canvas] 未提供图片路径，显示空白画布");
    }

    app.set_status(SharedString::from(status));
    println!("[ui] Slint 窗口启动");
    app.run()?;

    Ok(())
}
