// PhotoAnnotator 入口（第二阶段：图片加载 + 视图交互 + 矩形/椭圆标注）
//
// 交互模型：
//   浏览工具：拖动平移 / 滚轮缩放（锚点=鼠标）；窗口变化自动回自适应
//   矩形/椭圆工具：按下拖动绘制（图片坐标），松开加入标注集合并显示在标注层
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

// ---------- 预览显示 ----------

use slint::{Brush, Color};

/// 隐藏所有绘制预览（矩形 Rectangle / Path 类）
fn hide_previews(app: &AppWindow) {
    app.set_preview_visible(false);
    app.set_preview_path_visible(false);
}

/// 矩形预览：原生 Rectangle 元素（图片坐标）
fn show_rect_preview(app: &AppWindow, x1: f32, y1: f32, x2: f32, y2: f32) {
    let (x, y) = (x1.min(x2), y1.min(y2));
    app.set_preview_x(x);
    app.set_preview_y(y);
    app.set_preview_w(x1.max(x2) - x);
    app.set_preview_h(y1.max(y2) - y);
    app.set_preview_visible(true);
}

/// 椭圆预览：Path + SVG commands（外接矩形 → 两段弧），仅描边
fn show_ellipse_preview(app: &AppWindow, x1: f32, y1: f32, x2: f32, y2: f32) {
    let (x, y) = (x1.min(x2), y1.min(y2));
    let (w, h) = ((x1.max(x2) - x), (y1.max(y2) - y));
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let cmd = format!(
        "M {} {} a {} {} 0 1 0 {} 0 a {} {} 0 1 0 {} 0",
        cx - rx,
        cy,
        rx,
        ry,
        2.0 * rx,
        rx,
        ry,
        -2.0 * rx
    );
    app.set_preview_commands(cmd.into());
    app.set_preview_fill(Brush::SolidColor(Color::from_argb_u8(0, 0, 0, 0))); // 透明：仅描边
    app.set_preview_stroke_width(3.0);
    app.set_preview_path_visible(true);
}

/// 箭头预览：线段 + 实心三角（Path + SVG commands），线宽 5px
fn show_arrow_preview(app: &AppWindow, x1: f32, y1: f32, x2: f32, y2: f32) {
    let width = canvas::annotation::ARROW_STROKE_WIDTH;
    let mut cmd = format!("M {} {} L {} {}", x1, y1, x2, y2);
    if let Some([tip, left, right]) = canvas::annotation::arrow_head_points(x1, y1, x2, y2, width) {
        cmd.push_str(&format!(
            " M {} {} L {} {} L {} {} Z",
            tip.0, tip.1, left.0, left.1, right.0, right.1
        ));
    }
    app.set_preview_commands(cmd.into());
    app.set_preview_fill(Brush::SolidColor(Color::from_rgb_u8(0xF4, 0x43, 0x36)));
    app.set_preview_stroke_width(width);
    app.set_preview_path_visible(true);
}

/// 按当前工具显示对应的预览
fn show_preview(app: &AppWindow, tool: Tool, x1: f32, y1: f32, x2: f32, y2: f32) {
    match tool {
        Tool::Rect => show_rect_preview(app, x1, y1, x2, y2),
        Tool::Ellipse => show_ellipse_preview(app, x1, y1, x2, y2),
        Tool::Arrow => show_arrow_preview(app, x1, y1, x2, y2),
        Tool::Pen | Tool::Browse => {}
    }
}

/// 画笔预览：折线 Path（仅描边 3px）
fn show_pen_preview(app: &AppWindow, points: &[(f32, f32)]) {
    let mut cmd = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            cmd.push_str(&format!("M {} {}", x, y));
        } else {
            cmd.push_str(&format!(" L {} {}", x, y));
        }
    }
    app.set_preview_commands(cmd.into());
    app.set_preview_fill(Brush::SolidColor(Color::from_argb_u8(0, 0, 0, 0))); // 透明
    app.set_preview_stroke_width(canvas::annotation::DEFAULT_STROKE_WIDTH);
    app.set_preview_path_visible(true);
}

// ---------- 应用状态 ----------

/// 当前指针交互（一次按下 → 释放期间）
enum Interaction {
    None,
    /// 浏览工具下按住拖动 = 平移
    Panning { last: (f32, f32) },
    /// 绘制工具下按住拖动 = 绘制图元（起止点均为图片像素坐标）
    DrawingShape { start: (f32, f32), current: (f32, f32) },
    /// 画笔工具：持续累积折线点（图片像素坐标）
    DrawingPen { points: Vec<(f32, f32)> },
}

struct AppState {
    store: AnnotationStore,
    interaction: Interaction,
    image_width: u32,
    image_height: u32,
    /// 当前图片路径（保存时生成 *_annotated.png）
    image_path: Option<String>,
    /// 原图 straight-alpha RGBA（合成导出用）
    image_rgba: Vec<u8>,
}

impl AppState {
    fn new() -> Self {
        Self {
            store: AnnotationStore::new(),
            interaction: Interaction::None,
            image_width: 0,
            image_height: 0,
            image_path: None,
            image_rgba: Vec::new(),
        }
    }
}

/// 保存输出路径：原图同目录 + `_annotated.png`
fn annotated_save_path(original: Option<&str>) -> String {
    match original {
        Some(p) => {
            let path = std::path::Path::new(p);
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
            let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            dir.join(format!("{stem}_annotated.png")).to_string_lossy().into_owned()
        }
        None => "annotated.png".to_string(),
    }
}

/// 重渲染标注层（仅已完成的图元）并推到 Slint
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

/// 按工具把两点式绘制结果构造成图元
fn annotation_from_shape(tool: Tool, x1: f32, y1: f32, x2: f32, y2: f32) -> Annotation {
    match tool {
        Tool::Rect => Annotation::rect(x1, y1, x2, y2),
        Tool::Ellipse => Annotation::ellipse(x1, y1, x2, y2),
        Tool::Arrow => Annotation::arrow(x1, y1, x2, y2),
        Tool::Browse | Tool::Pen => unreachable!("浏览/画笔工具不会走两点式创建图元"),
    }
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
            Ok((image, w, h, rgba)) => {
                app.set_current_image(image);
                app.set_image_width(w as f32);
                app.set_image_height(h as f32);
                let mut st = state.borrow_mut();
                st.image_width = w;
                st.image_height = h;
                st.image_path = Some(path.clone());
                st.image_rgba = rgba;
                drop(st);
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
        // 工具切换：终止未完成的绘制/平移，隐藏残留预览，给出状态反馈
        let weak = app.as_weak();
        let state = state.clone();
        app.on_tool_changed(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            st.interaction = Interaction::None;
            hide_previews(&app);
            let msg = match Tool::from_id(app.get_active_tool()) {
                Tool::Browse => "浏览工具：拖动平移 / 滚轮缩放",
                Tool::Rect => "矩形工具：按住左键拖动画框",
                Tool::Ellipse => "椭圆工具：按住左键拖动画框",
                Tool::Arrow => "箭头工具：从起点拖到终点",
                Tool::Pen => "画笔工具：按住左键手绘",
            };
            app.set_status(SharedString::from(msg));
        });
    }
    {
        // 撤销（重做已按产品决策移除）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_undo(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            st.interaction = Interaction::None;
            hide_previews(&app);
            if st.store.undo() {
                update_overlay(&app, &st);
                app.set_status(SharedString::from(format!("已撤销，剩余 {} 个标注", st.store.len())));
            } else {
                app.set_status(SharedString::from("没有可撤销的操作"));
            }
        });
    }
    {
        // 保存：合成标注到原图 → PNG；保存=提交点，撤销历史清空
        let weak = app.as_weak();
        let state = state.clone();
        app.on_save(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            if st.image_width == 0 {
                app.set_status(SharedString::from("没有可保存的图片"));
                return;
            }
            let Some(png) = canvas::overlay::composite_png(
                st.image_width,
                st.image_height,
                &st.image_rgba,
                &st.store,
            ) else {
                app.set_status(SharedString::from("合成失败"));
                return;
            };
            let out_path = annotated_save_path(st.image_path.as_deref());
            match std::fs::write(&out_path, png) {
                Ok(_) => {
                    st.store.clear_history();
                    let n = st.store.len();
                    app.set_status(SharedString::from(format!(
                        "已保存: {out_path}（{n} 个标注，撤销历史已重置）"
                    )));
                }
                Err(e) => {
                    app.set_status(SharedString::from(format!("保存失败: {e}")));
                }
            }
        });
    }
    {
        // 复制：合成标注到原图 → 剪贴板（Ctrl+C 的基础）
        let weak = app.as_weak();
        let state = state.clone();
        app.on_copy(move || {            let (Some(app), st) = (weak.upgrade(), state.borrow()) else {
                return;
            };
            if st.image_width == 0 {
                app.set_status(SharedString::from("没有可复制的图片"));
                return;
            }
            let Some(rgba) = canvas::overlay::composite_rgba(
                st.image_width,
                st.image_height,
                &st.image_rgba,
                &st.store,
            ) else {
                app.set_status(SharedString::from("合成失败"));
                return;
            };
            let w = st.image_width as usize;
            let h = st.image_height as usize;
            drop(st);
            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    let img = arboard::ImageData {
                        width: w,
                        height: h,
                        bytes: std::borrow::Cow::Owned(rgba),
                    };
                    match cb.set_image(img) {
                        Ok(_) => {
                            app.set_status(SharedString::from("已复制到剪贴板"));
                        }
                        Err(e) => {
                            app.set_status(SharedString::from(format!("复制失败: {e}")));
                        }
                    }
                }
                Err(e) => {
                    app.set_status(SharedString::from(format!("剪贴板不可用: {e}")));
                }
            }
        });
    }
    {
        // Esc 层级处理：取消绘制 > 退出极简模式 > 提示
        let weak = app.as_weak();
        let state = state.clone();
        app.on_escape_pressed(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            if !matches!(st.interaction, Interaction::None) {
                // 1. 有绘制/平移进行中 → 取消
                st.interaction = Interaction::None;
                hide_previews(&app);
                app.set_status(SharedString::from("已取消"));
            } else if app.get_minimal_mode() {
                // 2. 极简模式且无绘制 → 退出极简
                app.set_minimal_mode(false);
                app.set_status(SharedString::from("已退出极简模式"));
            } else {
                // 3. 完整模式无操作 → 提示入口
                app.set_status(SharedString::from("极简模式: Ctrl+M"));
            }
        });
    }
    {
        // Ctrl+M：切换极简模式；进入时点亮提示条，3 秒后自动熄灭
        let weak = app.as_weak();
        let tip_timer = Rc::new(RefCell::new(Timer::default()));
        app.on_toggle_minimal(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let minimal = !app.get_minimal_mode();
            app.set_minimal_mode(minimal);
            if minimal {
                app.set_minimal_tip_visible(true);
                // 重置/启动 3 秒熄灭定时器
                let weak = weak.clone();
                let timer = tip_timer.clone();
                timer.borrow().stop();
                timer.borrow().start(TimerMode::SingleShot, Duration::from_millis(3000), move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_minimal_tip_visible(false);
                    }
                });
                app.set_status(SharedString::from("极简模式（提示 3 秒后消失）"));
            } else {
                app.set_minimal_tip_visible(false);
                app.set_status(SharedString::from("已退出极简模式"));
            }
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
                Tool::Rect | Tool::Ellipse | Tool::Arrow => {
                    // 绘制前固化为自由模式，保证坐标换算使用真实视图变换
                    ensure_free(&app);
                    hide_previews(&app);
                    let (ix, iy) = read_transform(&app).canvas_to_image(cx, cy);
                    st.interaction =
                        Interaction::DrawingShape { start: (ix, iy), current: (ix, iy) };
                    app.set_status(SharedString::from("绘制中 · 拖拽成形"));
                }
                Tool::Pen => {
                    ensure_free(&app);
                    hide_previews(&app);
                    let (ix, iy) = read_transform(&app).canvas_to_image(cx, cy);
                    st.interaction = Interaction::DrawingPen {
                        points: vec![(ix, iy)],
                    };
                    app.set_status(SharedString::from("画笔 · 按住手绘"));
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
            let (ix, iy) = read_transform(&app).canvas_to_image(cx, cy);

            // 画笔：按屏幕位移采样（约 0.5 屏幕像素一点），避免点过密
            if let Interaction::DrawingPen { points } = &mut st.interaction {
                let scale = read_transform(&app).scale.max(0.05);
                let min_d = 0.5 / scale;
                let append = match points.last() {
                    Some(last) => {
                        let d = ((ix - last.0).powi(2) + (iy - last.1).powi(2)).sqrt();
                        d >= min_d
                    }
                    None => true,
                };
                if append {
                    points.push((ix, iy));
                    show_pen_preview(&app, points);
                }
                return;
            }

            match st.interaction {
                Interaction::Panning { last } => {
                    let (lx, ly) = last;
                    let dx = cx - lx;
                    let dy = cy - ly;
                    st.interaction = Interaction::Panning { last: (cx, cy) };
                    app.set_view_offset_x(app.get_view_offset_x() + dx);
                    app.set_view_offset_y(app.get_view_offset_y() + dy);
                }
                Interaction::DrawingShape { start, .. } => {
                    st.interaction = Interaction::DrawingShape { start, current: (ix, iy) };
                    // 只更新预览元素属性（GPU 原生渲染），不重建标注层位图
                    let tool = Tool::from_id(app.get_active_tool());
                    show_preview(&app, tool, start.0, start.1, ix, iy);
                }
                Interaction::None | Interaction::DrawingPen { .. } => {}
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
                Interaction::DrawingShape { start, current } => {
                    // 隐藏预览，把完成的图元并入标注层
                    hide_previews(&app);
                    let tool = Tool::from_id(app.get_active_tool());
                    let a = annotation_from_shape(tool, start.0, start.1, current.0, current.1);
                    if a.has_area() {
                        st.store.push(a);
                        let n = st.store.len();
                        app.set_status(SharedString::from(format!("已添加标注，共 {n} 个")));
                    } else {
                        app.set_status(SharedString::from("忽略零尺寸图元"));
                    }
                    update_overlay(&app, &st);
                }
                Interaction::DrawingPen { points } => {
                    hide_previews(&app);
                    let a = Annotation::pen(points);
                    if a.has_area() {
                        st.store.push(a);
                        let n = st.store.len();
                        app.set_status(SharedString::from(format!("已添加标注，共 {n} 个")));
                    } else {
                        app.set_status(SharedString::from("忽略过短笔画"));
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
