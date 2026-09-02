// PhotoAnnotator 入口（第二阶段：图片加载 + 视图交互 + 矩形标注）
//
// 交互模型：
//   浏览工具：拖动平移 / 滚轮缩放（锚点=鼠标）；窗口变化自动回自适应
//   矩形工具：按下拖动绘制（图片坐标），松开加入标注集合并显示在标注层
//
// 用法：photo_annotator <图片路径>

mod canvas;
mod storage;

use canvas::{Annotation, AnnotationStore, Tool, ViewTransform};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode};
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
/// 固化后视图无跳变，坐标换算（read_transform）才可靠
fn ensure_free(app: &AppWindow) {
    if app.get_fit_mode() {
        write_transform(app, &current_fit(app));
        app.set_fit_mode(false);
    }
}

fn enter_fit(app: &AppWindow, msg: &str) {
    app.set_fit_mode(true);
    app.set_status(SharedString::from(msg));
}

/// 滚轮缩放：以当前鼠标位置（画布坐标）为锚点
fn zoom(app: &AppWindow, factor: f32) {
    ensure_free(app);
    let anchor_x = app.get_pointer_x();
    let anchor_y = app.get_pointer_y();
    let cur = read_transform(app);
    let next = cur.zoom_at(factor, anchor_x, anchor_y);
    write_transform(app, &next);
    app.set_status(SharedString::from(format!("自由 · 缩放 {:.0}%", next.scale * 100.0)));
}

// ---------- 应用状态 ----------

/// 当前指针交互（一次按下 → 释放期间）
enum Interaction {
    None,
    /// 浏览工具下按住拖动 = 平移
    Panning { last: (f32, f32) },
    /// 矩形工具下按住拖动 = 绘制（起止点均为图片像素坐标）
    DrawingRect { start: (f32, f32), current: (f32, f32) },
}

struct AppState {
    store: AnnotationStore,
    interaction: Interaction,
    image_width: u32,
    image_height: u32,
}

impl AppState {
    fn new() -> Self {
        Self {
            store: AnnotationStore::new(),
            interaction: Interaction::None,
            image_width: 0,
            image_height: 0,
        }
    }
}

/// 重渲染标注层（仅已完成的图元）并推到 Slint
/// 注意：绘制中预览不经过此函数（走 Slint preview-* 属性），保证拖动流畅
fn update_overlay(app: &AppWindow, state: &AppState) {
    if let Some((w, h, bytes)) = canvas::overlay::render_overlay(
        state.image_width,
        state.image_height,
        &state.store,
    ) {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        buffer.make_mut_bytes().copy_from_slice(&bytes);
        app.set_overlay_image(Image::from_rgba8(buffer));
    }
}

/// 设置预览矩形（图片坐标，自动规范化），并显示
fn show_preview(app: &AppWindow, x1: f32, y1: f32, x2: f32, y2: f32) {
    let (x, y) = (x1.min(x2), y1.min(y2));
    app.set_preview_x(x);
    app.set_preview_y(y);
    app.set_preview_w(x1.max(x2) - x);
    app.set_preview_h(y1.max(y2) - y);
    app.set_preview_visible(true);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 存储：初始化数据库（4 张表，幂等）
    let db = storage::init()?;
    println!("[storage] SQLite 初始化完成: {}", db.display());

    // 2. 窗口
    let app = AppWindow::new()?;
    let state = Rc::new(RefCell::new(AppState::new()));

    // 3. 图片：命令行参数加载
    if let Some(path) = std::env::args().nth(1) {
        match canvas::load_image(&path) {
            Ok((image, w, h)) => {
                app.set_current_image(image);
                app.set_image_width(w as f32);
                app.set_image_height(h as f32);
                state.borrow_mut().image_width = w;
                state.borrow_mut().image_height = h;
                app.set_fit_mode(true); // 默认自适应
                app.set_status(SharedString::from(format!("已加载: {path} ({w}x{h})")));
                println!("[canvas] 图片加载成功: {path} {w}x{h}");
            }
            Err(e) => {
                app.set_status(SharedString::from(format!("加载失败: {e}")));
                eprintln!("[canvas] 图片加载失败: {path}: {e}");
            }
        }
    }

    // 4. 视图回调
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
    {
        // 工具切换：终止未完成的绘制/平移，并给出状态反馈
        let weak = app.as_weak();
        let state = state.clone();
        app.on_tool_changed(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            st.interaction = Interaction::None;
            let msg = match Tool::from_id(app.get_active_tool()) {
                Tool::Browse => "浏览工具：拖动平移 / 滚轮缩放",
                Tool::Rect => "矩形工具：按住左键拖动画框",
            };
            app.set_status(SharedString::from(msg));
        });
    }

    // 5. 指针交互回调（按下/拖动/释放，按当前工具分发）
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_pointer_down(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            let cx = app.get_pointer_x();
            let cy = app.get_pointer_y();
            match Tool::from_id(app.get_active_tool()) {
                Tool::Browse => {
                    // 平移需自由模式（视图固定，随鼠标移动）
                    ensure_free(&app);
                    st.interaction = Interaction::Panning { last: (cx, cy) };
                    app.set_status(SharedString::from("浏览 · 拖动平移"));
                }
                Tool::Rect => {
                    // 绘制前固化为自由模式，保证坐标换算使用真实视图变换
                    ensure_free(&app);
                    let (ix, iy) = read_transform(&app).canvas_to_image(cx, cy);
                    st.interaction = Interaction::DrawingRect { start: (ix, iy), current: (ix, iy) };
                    app.set_preview_visible(false); // 清除可能的残留预览
                    app.set_status(SharedString::from("矩形 · 拖动画框"));
                }
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_pointer_move(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            let cx = app.get_pointer_x();
            let cy = app.get_pointer_y();
            match st.interaction {
                Interaction::Panning { last } => {
                    let (lx, ly) = last;
                    let dx = cx - lx;
                    let dy = cy - ly;
                    st.interaction = Interaction::Panning { last: (cx, cy) };
                    app.set_view_offset_x(app.get_view_offset_x() + dx);
                    app.set_view_offset_y(app.get_view_offset_y() + dy);
                }
                Interaction::DrawingRect { start, .. } => {
                    let (ix, iy) = read_transform(&app).canvas_to_image(cx, cy);
                    st.interaction = Interaction::DrawingRect { start, current: (ix, iy) };
                    // 只更新预览元素属性（GPU 原生渲染），不重建标注层位图
                    show_preview(&app, start.0, start.1, ix, iy);
                }
                Interaction::None => {}
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_pointer_up(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            match std::mem::replace(&mut st.interaction, Interaction::None) {
                Interaction::DrawingRect { start, current } => {
                    // 隐藏预览，把完成的矩形并入标注层
                    app.set_preview_visible(false);
                    let a = Annotation::rect(start.0, start.1, current.0, current.1);
                    if a.rect_has_area() {
                        st.store.push(a);
                        let n = st.store.len();
                        app.set_status(SharedString::from(format!("已添加标注，共 {n} 个")));
                    } else {
                        app.set_status(SharedString::from("忽略零尺寸矩形"));
                    }
                    update_overlay(&app, &st);
                }
                _ => {}
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
