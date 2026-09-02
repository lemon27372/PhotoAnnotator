// PhotoAnnotator 入口（第二阶段：图片加载 + 视图交互状态机）
//
// 视图模式：
//   自适应(fit) —— 默认/常态：图片实时居中适应窗口（resize 自动跟随）
//   自由(free)  —— 拖动图片或手动缩放后进入；可自由平移/缩放
//   回到自适应    —— 点「重置」，或窗口尺寸变化后自动恢复
//
// 用法：
//   photo_annotator <图片路径>

mod canvas;
mod storage;

use canvas::ViewTransform;
use slint::{SharedString, Timer, TimerMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

slint::include_modules!();

// ---------- 视图变换读写 ----------

fn read_transform(app: &AppWindow) -> ViewTransform {
    ViewTransform {
        scale: app.get_view_scale(),
        offset_x: app.get_view_offset_x(),
        offset_y: app.get_view_offset_y(),
    }
}

fn write_transform(app: &AppWindow, t: &ViewTransform) {
    app.set_view_scale(t.scale);
    app.set_view_offset_x(t.offset_x);
    app.set_view_offset_y(t.offset_y);
}

/// 当前视口下的自适应变换（供 fit → free 固化用）
fn current_fit(app: &AppWindow) -> ViewTransform {
    ViewTransform::fit(
        app.get_viewport_width(),
        app.get_viewport_height(),
        app.get_image_width(),
        app.get_image_height(),
    )
}

/// 若处于自适应模式，先固化为自由模式（写入当前 fit 值作为手动基准）
fn ensure_free(app: &AppWindow) {
    if app.get_fit_mode() {
        write_transform(app, &current_fit(app));
        app.set_fit_mode(false);
    }
}

/// 进入自适应模式
fn enter_fit(app: &AppWindow, msg: &str) {
    app.set_fit_mode(true);
    app.set_status(SharedString::from(msg));
}

/// 手动缩放：以当前鼠标位置（画布坐标）为锚点
/// factor>1 放大，factor<1 缩小
fn zoom(app: &AppWindow, factor: f32) {
    ensure_free(app);
    let anchor_x = app.get_pointer_x();
    let anchor_y = app.get_pointer_y();
    let cur = read_transform(app);
    let next = cur.zoom_at(factor, anchor_x, anchor_y);
    write_transform(app, &next);
    app.set_status(SharedString::from(format!("自由 · 缩放 {:.0}%", next.scale * 100.0)));
}

// ---------- 拖动状态（平移） ----------

struct DragState {
    active: bool,
    last_x: f32,
    last_y: f32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 存储：初始化数据库（4 张表，幂等）
    let db = storage::init()?;
    println!("[storage] SQLite 初始化完成: {}", db.display());

    // 2. 窗口：启动 Slint 主窗口
    let app = AppWindow::new()?;

    // 3. 图片：命令行参数加载（后续接入文件对话框/工作目录）
    if let Some(path) = std::env::args().nth(1) {
        match canvas::load_image(&path) {
            Ok((image, w, h)) => {
                app.set_current_image(image);
                app.set_image_width(w as f32);
                app.set_image_height(h as f32);
                // 打开图片默认进入自适应模式
                app.set_fit_mode(true);
                println!("[canvas] 图片加载成功: {path} {w}x{h}");
            }
            Err(e) => {
                app.set_status(SharedString::from(format!("加载失败: {e}")));
                eprintln!("[canvas] 图片加载失败: {path}: {e}");
            }
        }
    }

    // 4. 视图工具条回调
    {
        let weak = app.as_weak();
        app.on_zoom_in(move || {
            if let Some(app) = weak.upgrade() {
                zoom(&app, 1.25);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_zoom_out(move || {
            if let Some(app) = weak.upgrade() {
                zoom(&app, 0.8);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_reset_view(move || {
            if let Some(app) = weak.upgrade() {
                enter_fit(&app, "自适应");
            }
        });
    }

    // 5. 画布拖动回调（拖动 → 自由模式平移）
    let drag = Rc::new(RefCell::new(DragState { active: false, last_x: 0.0, last_y: 0.0 }));
    {
        let weak = app.as_weak();
        let drag = drag.clone();
        app.on_pointer_down(move || {
            if let Some(app) = weak.upgrade() {
                ensure_free(&app);
                let mut d = drag.borrow_mut();
                d.active = true;
                d.last_x = app.get_pointer_x();
                d.last_y = app.get_pointer_y();
                app.set_status(SharedString::from("自由 · 拖动平移"));
            }
        });
    }
    {
        let weak = app.as_weak();
        let drag = drag.clone();
        app.on_pointer_move(move || {
            if let Some(app) = weak.upgrade() {
                let mut d = drag.borrow_mut();
                if d.active {
                    let x = app.get_pointer_x();
                    let y = app.get_pointer_y();
                    let dx = x - d.last_x;
                    let dy = y - d.last_y;
                    d.last_x = x;
                    d.last_y = y;
                    app.set_view_offset_x(app.get_view_offset_x() + dx);
                    app.set_view_offset_y(app.get_view_offset_y() + dy);
                }
            }
        });
    }
    {
        let weak = app.as_weak();
        let drag = drag.clone();
        app.on_pointer_up(move || {
            if let Some(_app) = weak.upgrade() {
                drag.borrow_mut().active = false;
            }
        });
    }

    // 6. 窗口尺寸变化检测：自由模式下视口变化 → 自动回到自适应
    let last_viewport = Rc::new(RefCell::new((app.get_viewport_width(), app.get_viewport_height())));
    let timer = Timer::default();
    {
        let weak = app.as_weak();
        let last_viewport = last_viewport.clone();
        timer.start(TimerMode::Repeated, Duration::from_millis(300), move || {
            if let Some(app) = weak.upgrade() {
                let vw = app.get_viewport_width();
                let vh = app.get_viewport_height();
                let mut last = last_viewport.borrow_mut();
                if (vw - last.0).abs() > 0.5 || (vh - last.1).abs() > 0.5 {
                    *last = (vw, vh);
                    if !app.get_fit_mode() {
                        enter_fit(&app, "自适应（窗口变化）");
                    }
                }
            }
        });
    }

    println!("[ui] Slint 窗口启动");
    app.run()?;

    // timer 需保持存活到窗口关闭
    std::mem::drop(timer);
    Ok(())
}
