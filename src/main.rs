// PhotoAnnotator 入口（第二/三阶段：图片加载 + 视图交互 + 标注 + 工作目录）
//
// 交互模型（2026-09-08 起无浏览工具）：
//   平移：任意工具下按住中键或右键拖动；滚轮缩放（锚点=鼠标）；窗口变化自动回自适应
//   矩形/椭圆/箭头/画笔：左键按下拖动绘制（图片坐标），松开加入标注集合并显示在标注层
//   文字：左键点击落点弹输入框，Enter 确认
//
// 用法：photo_annotator <图片路径 | 工作目录>

mod canvas;
mod storage;
mod workspace;

use canvas::{Annotation, AnnotationStore, Tool, ViewTransform};
use slint::{Image, Model, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
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
        Tool::Pen | Tool::Text => {}
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
    /// 中键/右键按住拖动 = 平移（任意工具下；浏览工具已移除）
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
    // ---- 工作目录（第三阶段）----
    /// 工作区根目录（nav_stack[0] 同义；决定 workspace_id 与标题）
    workspace_dir: Option<String>,
    /// DB workspace id（-1 = 单图模式未开工作区）
    workspace_id: i64,
    /// 面包屑导航栈（根在前，当前浏览目录 = last）
    nav_stack: Vec<String>,
    /// 当前浏览目录的直接子目录（扫描结果，排序）
    dirs: Vec<String>,
    /// 当前目录图片绝对路径列表
    files: Vec<String>,
    /// 每张图的 DB file id
    file_ids: Vec<i64>,
    /// 当前图片在 files 中的索引（-1 = 无）
    current_index: i64,
    /// 当前图片 DB file id（-1 = 未入库）
    file_id: i64,
    /// 缩略图内存缓存（按 files 图片索引；解码一次，重建列表复用）
    thumb_cache: Vec<Option<slint::Image>>,
    /// 双击检测：上一次点击的列表行（-1 无）与时刻（ns）
    last_click_row: i32,
    last_click_at: u128,
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
            workspace_dir: None,
            workspace_id: -1,
            nav_stack: Vec::new(),
            dirs: Vec::new(),
            files: Vec::new(),
            file_ids: Vec::new(),
            current_index: -1,
            file_id: -1,
            thumb_cache: Vec::new(),
            last_click_row: -1,
            last_click_at: 0,
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

// ---------- 工作目录（第三阶段 P1+P2） ----------

/// PNG 字节 → Slint Image（缩略图显示）
fn png_to_image(png: &[u8]) -> Option<Image> {
    let img = image::load_from_memory(png).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    buf.make_mut_bytes().copy_from_slice(rgba.as_raw());
    Some(Image::from_rgba8(buf))
}

/// 若当前图片已入库，把标注持久化到 DB（切图前调用）
fn persist_current(st: &mut AppState) {
    if st.file_id >= 0 {
        storage::db::save_annotations(st.file_id, &st.store.to_json());
    }
}

/// 获取/生成某文件缩略图（DB 缓存 → 生成并缓存），并写入内存缓存
fn ensure_thumbnail(st: &mut AppState, idx: usize) -> Option<Image> {
    if let Some(img) = st.thumb_cache.get(idx).and_then(|c| c.clone()) {
        return Some(img);
    }
    if st.thumb_cache.len() <= idx {
        st.thumb_cache.resize(idx + 1, None);
    }
    let img = (|| {
        let file_id = *st.file_ids.get(idx)?;
        // DB 缓存命中
        if let Some(png) = storage::db::load_thumbnail(file_id) {
            if let Some(img) = png_to_image(&png) {
                return Some(img);
            }
        }
        // 生成并缓存
        let path = std::path::Path::new(st.files.get(idx)?);
        let png = workspace::gen_thumbnail_png(path, 96)?;
        storage::db::save_thumbnail(file_id, &png);
        png_to_image(&png)
    })();
    st.thumb_cache[idx] = img.clone();
    img
}

/// 当前浏览目录（面包屑栈顶；无工作区时 None）
fn current_dir(st: &AppState) -> Option<&str> {
    st.nav_stack.last().map(|s| s.as_str())
}

/// 路径显示名（末段；盘符根回退整串）
fn dir_display_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// 当前图片在侧边栏列表中的行号（目录行在前 → 需偏移；-1 = 无选中图）
fn current_row(st: &AppState) -> i32 {
    if st.current_index >= 0 {
        st.dirs.len() as i32 + st.current_index as i32
    } else {
        -1
    }
}

/// 空占位条目（网格行末尾补齐，kind=-1 不可交互）
fn placeholder_entry() -> FileEntry {
    FileEntry {
        path: SharedString::new(),
        name: SharedString::new(),
        kind: -1,
        status: 0,
        thumb: Image::default(),
    }
}

/// 重建侧边栏：file-list（子目录行 + 图片行）+ grid-rows（每行 3 格）+ 面包屑 + 计数 + 当前行高亮
fn rebuild_sidebar(app: &AppWindow, st: &mut AppState) {
    // 1. 图片行元信息（借用分离，避免冲突）
    let imgs: Vec<(String, String, i32)> = st
        .files
        .iter()
        .map(|path| {
            let name = dir_display_name(path);
            let status = if st.workspace_id >= 0 {
                match storage::db::file_status(st.workspace_id, path).as_str() {
                    "done" => 1,
                    "ignored" => 2,
                    _ => 0,
                }
            } else {
                0
            };
            (path.clone(), name, status)
        })
        .collect();

    let mut entries: Vec<FileEntry> = Vec::with_capacity(st.dirs.len() + imgs.len());
    // 子目录行（kind=1，无缩略图；列在图片之前 → 行号前缀即 dirs.len()）
    for d in &st.dirs {
        entries.push(FileEntry {
            path: d.clone().into(),
            name: dir_display_name(d).into(),
            kind: 1,
            status: 0,
            thumb: Image::default(),
        });
    }
    // 图片行（kind=0）
    for (i, (path, name, status)) in imgs.into_iter().enumerate() {
        let thumb = ensure_thumbnail(st, i).unwrap_or_default();
        entries.push(FileEntry {
            path: path.into(),
            name: name.into(),
            kind: 0,
            status,
            thumb,
        });
    }
    app.set_file_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(entries.clone()))));

    // 2. 网格行（固定 3 格，不足补占位）
    let rows: Vec<GridRow> = entries
        .chunks(3)
        .map(|c| GridRow {
            c0: c[0].clone(),
            c1: c.get(1).cloned().unwrap_or_else(placeholder_entry),
            c2: c.get(2).cloned().unwrap_or_else(placeholder_entry),
        })
        .collect();
    app.set_grid_rows(slint::ModelRc::from(Rc::new(slint::VecModel::from(rows))));

    // 3. 面包屑（各级显示名；仅多级时可见）
    let crumbs: Vec<SharedString> = st
        .nav_stack
        .iter()
        .map(|d| dir_display_name(d).into())
        .collect();
    app.set_crumb_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(crumbs))));

    // 4. 计数信息
    let meta = if st.dirs.is_empty() && st.files.is_empty() {
        "空".to_string()
    } else {
        let mut parts = Vec::new();
        if !st.dirs.is_empty() {
            parts.push(format!("{} 目录", st.dirs.len()));
        }
        if !st.files.is_empty() {
            parts.push(format!("{} 图", st.files.len()));
        }
        parts.join(" · ")
    };
    app.set_sidebar_meta(SharedString::from(meta));

    // 5. 当前选中行高亮（目录切换后重算）
    app.set_current_row(current_row(st));
}

/// 按当前浏览目录（nav_stack 栈顶）重新扫描：dirs + files + file_ids + 缓存重置
/// （不触碰 workspace_id / 根目录；标注持久化由调用方负责）
fn rescan_current(app: &AppWindow, st: &mut AppState) {
    let Some(dir) = current_dir(st).map(str::to_string) else {
        return;
    };
    let dir_path = std::path::Path::new(&dir);
    st.dirs = workspace::scan_subdirs(dir_path)
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    st.files = workspace::scan_images(dir_path)
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    st.thumb_cache = Vec::new();
    st.current_index = -1;
    st.file_id = -1;
    st.file_ids.clear();
    // 入库获取 file id（宽高暂 0，打开时更新）
    if st.workspace_id >= 0 {
        for p in &st.files {
            st.file_ids
                .push(storage::db::upsert_file(st.workspace_id, p, 0, 0).unwrap_or(-1));
        }
    }
    app.set_current_index(-1);
    rebuild_sidebar(app, st);
    if st.dirs.is_empty() && st.files.is_empty() {
        app.set_status(SharedString::from(format!("{dir} 为空（无子目录/图片）")));
    }
}

/// 打开工作目录（切换工作区根）：persist + 重置导航栈 + 扫描 + 重建列表
fn open_workspace(app: &AppWindow, st: &mut AppState, dir: &str) {
    persist_current(st);
    st.workspace_dir = Some(dir.to_string());
    st.workspace_id = storage::db::touch_workspace(dir).unwrap_or(-1);
    st.nav_stack = vec![dir.to_string()];
    rescan_current(app, st);
    app.set_workspace_title(SharedString::from(dir_display_name(dir)));
}

/// 后台解码完成的结果（Send，可跨线程排队）
struct LoadResult {
    /// 发起时的序号（过期的丢弃）
    seq: u32,
    /// 目标索引（更新选中态）
    current_index: i64,
    path: String,
    workspace_id: i64,
    /// Ok(宽, 高, RGBA) / Err(描述)
    decoded: Result<(u32, u32, Vec<u8>), String>,
}

/// 解码完成的图片数据（缓存单元）
struct Decoded {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

/// 后台预解码一张邻图写入缓存（切图秒开的关键）
fn spawn_prefetch(idx: usize, files: Vec<String>, cache: &Arc<Mutex<HashMap<usize, Arc<Decoded>>>>) {
    if idx >= files.len() {
        return;
    }
    let cache = cache.clone();
    std::thread::spawn(move || {
        if cache.lock().map(|c| c.contains_key(&idx)).unwrap_or(true) {
            return;
        }
        if let Some(path) = files.get(idx) {
            if let Ok((w, h, rgba)) = canvas::decode_image(path) {
                if let Ok(mut c) = cache.lock() {
                    c.entry(idx).or_insert_with(|| Arc::new(Decoded { w, h, rgba }));
                }
            }
        }
    });
}

/// 发起异步加载：命中预取缓存则立即应用；否则后台解码
/// 同时预取相邻图片到缓存
fn load_image_by_index(
    app: &AppWindow,
    state: &Rc<RefCell<AppState>>,
    idx: usize,
    load_seq: &Arc<AtomicU32>,
    queue: &Arc<Mutex<Vec<LoadResult>>>,
    cache: &Arc<Mutex<HashMap<usize, Arc<Decoded>>>>,
) {
    // 阶段 1（借用 state）：持久化旧标注 + 清理 + 提示加载中
    let (path, workspace_id, current_index) = {
        let mut st = state.borrow_mut();
        if idx >= st.files.len() {
            return;
        }
        persist_current(&mut st);
        st.interaction = Interaction::None;
        hide_previews(app);
        hide_text_input(app);
        app.set_loading(true);
        app.set_status(SharedString::from("加载中…"));
        (st.files[idx].clone(), st.workspace_id, idx as i64)
    };

    // 递增序号（本次为最新，旧任务结果将被丢弃）
    let this_seq = load_seq.fetch_add(1, Ordering::Relaxed) + 1;

    // 缓存命中：立即应用（无需线程等待）
    let cached = cache.lock().ok().and_then(|c| c.get(&idx).cloned());
    if let Some(dec) = cached {
        apply_load_result(
            app,
            state,
            LoadResult {
                seq: this_seq,
                current_index,
                path,
                workspace_id,
                decoded: Ok((dec.w, dec.h, dec.rgba.clone())),
            },
        );
        // 命中后仍预取相邻图片（±2）
        let files = state.borrow().files.clone();
        for off in [1i64, -1i64, 2i64, -2i64] {
            let n = idx as i64 + off;
            if n >= 0 {
                spawn_prefetch(n as usize, files.clone(), cache);
            }
        }
        return;
    }

    // 未命中：后台解码，结果入队（轮询线程应用）
    let queue = queue.clone();
    std::thread::spawn(move || {
        let decoded = canvas::decode_image(&path).map_err(|e| e.to_string());
        let result = LoadResult { seq: this_seq, current_index, path, workspace_id, decoded };
        if let Ok(mut q) = queue.lock() {
            q.push(result);
        }
    });

    // 缓存维护：只保留 idx 附近（±2），预取相邻图片
    {
        if let Ok(mut c) = cache.lock() {
            let keep = idx as isize;
            c.retain(|k, _| {
                let k = *k as isize;
                (k - keep).abs() <= 2
            });
        }
    }
    let files = state.borrow().files.clone();
    for off in [1i64, -1i64, 2i64, -2i64] {
        let n = idx as i64 + off;
        if n >= 0 {
            spawn_prefetch(n as usize, files.clone(), cache);
        }
    }
}

/// 应用一次加载结果（主线程 Timer 轮询队列时调用）
fn apply_load_result(app: &AppWindow, state: &Rc<RefCell<AppState>>, r: LoadResult) {
    let mut st = state.borrow_mut();
    let (w, h, rgba) = match r.decoded {
        Ok(v) => v,
        Err(e) => {
            app.set_loading(false);
            app.set_status(SharedString::from(format!("加载失败: {e}")));
            return;
        }
    };
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    buffer.make_mut_bytes().copy_from_slice(&rgba);
    let image = Image::from_rgba8(buffer);

    app.set_current_image(image);
    app.set_image_width(w as f32);
    app.set_image_height(h as f32);
    st.image_width = w;
    st.image_height = h;
    st.image_path = Some(r.path.clone());
    st.image_rgba = rgba;
    st.current_index = r.current_index;
    // 更新尺寸并读取历史标注（DB 读写快，主线程即可）
    if r.workspace_id >= 0 {
        if let Some(fid) = storage::db::upsert_file(r.workspace_id, &r.path, w as i32, h as i32) {
            st.file_id = fid;
            if r.current_index >= 0 && (r.current_index as usize) < st.file_ids.len() {
                st.file_ids[r.current_index as usize] = fid;
            }
            let json = storage::db::load_annotations(fid);
            let items = AnnotationStore::from_json(&json);
            st.store.set_items(items);
        }
    } else {
        st.file_id = -1;
        st.store.set_items(Vec::new());
    }
    update_overlay(app, &st);
    app.set_current_index(r.current_index as i32);
    app.set_current_row(current_row(&st));
    app.set_fit_mode(true);
    app.set_loading(false);
    let name = std::path::Path::new(&r.path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| r.path.clone());
    app.set_status(SharedString::from(format!(
        "已加载: {name} ({w}x{h}) · 标注 {}",
        st.store.len()
    )));
}

/// 重渲染标注层（仅已完成的图元）并推到 Slint
/// 同时同步文字标注列表到 Slint 显示层（撤销/新增后调用）
fn update_overlay(app: &AppWindow, state: &AppState) {
    if state.store.items.is_empty() {
        // 无标注：置 1x1 透明 overlay，避免全图重建/上传
        let mut tiny = SharedPixelBuffer::<Rgba8Pixel>::new(1, 1);
        tiny.make_mut_bytes().copy_from_slice(&[0, 0, 0, 0]);
        app.set_overlay_image(Image::from_rgba8(tiny));
    } else if let Some((w, h, bytes)) = canvas::overlay::render_overlay(
        state.image_width,
        state.image_height,
        &state.store,
    ) {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        buffer.make_mut_bytes().copy_from_slice(&bytes);
        app.set_overlay_image(Image::from_rgba8(buffer));
    }
    // 文字标注走 Slint 原生 Text 显示
    let texts: Vec<TextDisplay> = state
        .store
        .items
        .iter()
        .filter_map(|a| match a {
            canvas::Annotation::Text(t) => Some(TextDisplay {
                x: t.x,
                y: t.y,
                text: t.text.clone().into(),
                font_size: t.font_size,
            }),
            _ => None,
        })
        .collect();
    app.set_text_annotations(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(texts))));
}

/// 隐藏文字输入框并清空草稿
fn hide_text_input(app: &AppWindow) {
    app.set_text_input_visible(false);
    app.set_text_draft(SharedString::from(""));
}

/// 若输入框有非空文字 → 提交为文字标注（返回是否提交）
/// 输入框位置（画布坐标）换算为图片坐标作为文字锚点
fn commit_text_input(app: &AppWindow, st: &mut AppState) -> bool {
    let text = app.get_text_draft().trim().to_string();
    if text.is_empty() {
        return false;
    }
    let cx = app.get_text_input_x();
    let cy = app.get_text_input_y();
    let (ix, iy) = read_transform(app).canvas_to_image(cx, cy);
    st.store.push(canvas::Annotation::text(ix, iy, text));
    true
}

/// 按工具把两点式绘制结果构造成图元
fn annotation_from_shape(tool: Tool, x1: f32, y1: f32, x2: f32, y2: f32) -> Annotation {
    match tool {
        Tool::Rect => Annotation::rect(x1, y1, x2, y2),
        Tool::Ellipse => Annotation::ellipse(x1, y1, x2, y2),
        Tool::Arrow => Annotation::arrow(x1, y1, x2, y2),
        Tool::Pen | Tool::Text => unreachable!("非两点式工具不走此创建路径"),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 存储：初始化数据库（4 张表，幂等）
    let db = storage::init()?;
    println!("[storage] SQLite 初始化完成: {}", db.display());

    // 2. 窗口
    let app = AppWindow::new()?;
    let state = Rc::new(RefCell::new(AppState::new()));
    // 异步切图：序号（edition2024 中 gen 是保留字）+ 完成队列 + 邻图预取缓存
    let load_seq = Arc::new(AtomicU32::new(0));
    let load_queue: Arc<Mutex<Vec<LoadResult>>> = Arc::new(Mutex::new(Vec::new()));
    let decode_cache: Arc<Mutex<HashMap<usize, Arc<Decoded>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // 3. 命令行参数：目录 → 工作目录模式；文件 → 单图模式
    if let Some(arg) = std::env::args().nth(1) {
        let arg_path = std::path::Path::new(&arg);
        if arg_path.is_dir() {
            // 工作目录模式：打开目录并加载第一张
            println!("[workspace] 打开工作目录: {arg}");
            {
                let mut st = state.borrow_mut();
                open_workspace(&app, &mut st, &arg);
            }
            // 自动加载第一张（异步）
            let count = state.borrow().files.len();
            if count > 0 {
                load_image_by_index(&app, &state, 0, &load_seq, &load_queue, &decode_cache);
            }
        } else {
            // 单图模式
            match canvas::load_image(&arg) {
                Ok((image, w, h, rgba)) => {
                    app.set_current_image(image);
                    app.set_image_width(w as f32);
                    app.set_image_height(h as f32);
                    let mut st = state.borrow_mut();
                    st.image_width = w;
                    st.image_height = h;
                    st.image_path = Some(arg.clone());
                    st.image_rgba = rgba;
                    drop(st);
                    app.set_fit_mode(true); // 默认自适应
                    app.set_status(SharedString::from(format!("已加载: {arg} ({w}x{h})")));
                    println!("[canvas] 图片加载成功: {arg} {w}x{h}");
                }
                Err(e) => {
                    app.set_status(SharedString::from(format!("加载失败: {e}")));
                    eprintln!("[canvas] 图片加载失败: {arg}: {e}");
                }
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
        // 工具切换：终止未完成的绘制/平移，隐藏残留预览/输入框，给出状态反馈
        let weak = app.as_weak();
        let state = state.clone();
        app.on_tool_changed(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            st.interaction = Interaction::None;
            hide_previews(&app);
            hide_text_input(&app);
            let msg = match Tool::from_id(app.get_active_tool()) {
                Tool::Rect => "矩形工具：按住左键拖动画框（中键/右键拖动平移）",
                Tool::Ellipse => "椭圆工具：按住左键拖动画框（中键/右键拖动平移）",
                Tool::Arrow => "箭头工具：从起点拖到终点（中键/右键拖动平移）",
                Tool::Pen => "画笔工具：按住左键手绘（中键/右键拖动平移）",
                Tool::Text => "文字工具：点击画布放置文字（中键/右键拖动平移）",
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
                    // 工作目录模式：标注已保存 → 状态置 done 并刷新列表
                    if st.workspace_id >= 0 {
                        if let Some(p) = st.image_path.clone() {
                            storage::db::set_file_status(st.workspace_id, &p, "done");
                            persist_current(&mut st);
                            rebuild_sidebar(&app, &mut st);
                        }
                    }
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
        // Esc 层级处理：关最近面板 > 取消文字输入 > 取消绘制 > 退出极简模式 > 提示
        let weak = app.as_weak();
        let state = state.clone();
        app.on_escape_pressed(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            if app.get_recent_visible() {
                // 0. 最近工作区面板打开 → 关闭
                app.set_recent_visible(false);
            } else if app.get_text_input_visible() {
                // 1. 文字输入中 → 取消输入
                hide_text_input(&app);
                app.set_status(SharedString::from("已取消文字输入"));
            } else if !matches!(st.interaction, Interaction::None) {
                // 2. 有绘制/平移进行中 → 取消
                st.interaction = Interaction::None;
                hide_previews(&app);
                app.set_status(SharedString::from("已取消"));
            } else if app.get_minimal_mode() {
                // 3. 极简模式且无绘制 → 退出极简
                app.set_minimal_mode(false);
                app.set_status(SharedString::from("已退出极简模式"));
            } else {
                // 4. 完整模式无操作 → 提示入口
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
    {
        // 文字输入确认（Enter）：提交文字标注到点击落点
        let weak = app.as_weak();
        let state = state.clone();
        app.on_text_commit(move || {
            let (Some(app), mut st) = (weak.upgrade(), state.borrow_mut()) else {
                return;
            };
            let submitted = commit_text_input(&app, &mut st);
            hide_text_input(&app);
            if submitted {
                update_overlay(&app, &st);
                app.set_status(SharedString::from(format!(
                    "已添加文字，共 {} 个标注",
                    st.store.len()
                )));
            } else {
                app.set_status(SharedString::from("输入为空，已取消"));
            }
        });
    }
    {
        // 侧边栏点击（列表/网格共用行号）：
        //   子目录行 → 双击进入（单击仅提示）；图片行 → 异步加载（先持久化旧标注）
        let weak = app.as_weak();
        let state = state.clone();
        let load_seq = load_seq.clone();
        let load_queue = load_queue.clone();
        let decode_cache = decode_cache.clone();
        app.on_select_file(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let row = app.get_clicked_file();
            if row < 0 {
                return;
            }
            let mut st = state.borrow_mut();
            // 双击判定（同行 500ms 内）
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let is_double = row == st.last_click_row
                && st.last_click_at != 0
                && now.saturating_sub(st.last_click_at) < 500_000_000;
            st.last_click_row = row;
            st.last_click_at = now;

            let dir_count = st.dirs.len() as i32;
            if row < dir_count {
                // 子目录行
                let Some(dir) = st.dirs.get(row as usize).cloned() else {
                    return;
                };
                if !is_double {
                    let name = dir_display_name(&dir);
                    app.set_status(SharedString::from(format!("双击进入子目录: {name}")));
                    return;
                }
                // 双击 → 进入子目录（持久化旧标注 + 重扫 + 加载首图）
                persist_current(&mut st);
                st.nav_stack.push(dir);
                // 解码缓存按图片索引区分目录 → 目录变更时清空，防同索引命中旧目录图片
                if let Ok(mut c) = decode_cache.lock() {
                    c.clear();
                }
                rescan_current(&app, &mut st);
                let count = st.files.len();
                drop(st);
                if count > 0 {
                    load_image_by_index(&app, &state, 0, &load_seq, &load_queue, &decode_cache);
                }
            } else {
                // 图片行（行号 - 目录数 = 图片索引）
                let img_idx = (row - dir_count) as usize;
                if img_idx >= st.files.len() {
                    return;
                }
                drop(st);
                load_image_by_index(&app, &state, img_idx, &load_seq, &load_queue, &decode_cache);
            }
        });
    }
    {
        // 面包屑点击：跳转到对应层级目录（含当前层 no-op）
        let weak = app.as_weak();
        let state = state.clone();
        let load_seq = load_seq.clone();
        let load_queue = load_queue.clone();
        let decode_cache = decode_cache.clone();
        app.on_crumb_clicked(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let depth = app.get_clicked_crumb();
            if depth < 0 {
                return;
            }
            let mut st = state.borrow_mut();
            let len = st.nav_stack.len();
            let d = depth as usize;
            if d >= len || d == len - 1 {
                return; // 越界 / 已在该层
            }
            persist_current(&mut st);
            st.nav_stack.truncate(d + 1);
            if let Ok(mut c) = decode_cache.lock() {
                c.clear();
            }
            rescan_current(&app, &mut st);
            let count = st.files.len();
            drop(st);
            if count > 0 {
                load_image_by_index(&app, &state, 0, &load_seq, &load_queue, &decode_cache);
            }
        });
    }
    {
        // 「最近」按钮：打开时拉取最近工作区列表；已打开则关闭
        let weak = app.as_weak();
        app.on_open_recent(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            if app.get_recent_visible() {
                app.set_recent_visible(false);
                return;
            }
            let list: Vec<RecentEntry> = storage::db::list_workspaces(10)
                .into_iter()
                // 目录已不存在的历史跳过
                .filter(|(_, p, _)| std::path::Path::new(p).is_dir())
                .take(8)
                .map(|(_, p, _)| RecentEntry {
                    name: dir_display_name(&p).into(),
                    path: p.into(),
                })
                .collect();
            app.set_recent_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(list))));
            app.set_recent_visible(true);
        });
    }
    {
        // 最近工作区条目点击：切换到该工作区（等同命令行打开目录）
        let weak = app.as_weak();
        let state = state.clone();
        let load_seq = load_seq.clone();
        let load_queue = load_queue.clone();
        let decode_cache = decode_cache.clone();
        app.on_recent_selected(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let idx = app.get_clicked_recent();
            if idx < 0 {
                return;
            }
            let entry = app.get_recent_list().row_data(idx as usize);
            let Some(entry) = entry else {
                return;
            };
            app.set_recent_visible(false);
            let path = entry.path.to_string();
            if let Ok(mut c) = decode_cache.lock() {
                c.clear();
            }
            let mut st = state.borrow_mut();
            open_workspace(&app, &mut st, &path);
            let count = st.files.len();
            drop(st);
            if count > 0 {
                load_image_by_index(&app, &state, 0, &load_seq, &load_queue, &decode_cache);
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
            // 中键(1)/右键(2) → 任意工具下平移（无需浏览工具）
            if app.get_pointer_button() != 0 {
                ensure_free(&app);
                // 拖动平移不应把输入框草稿误提交（锚点随视图偏移）
                if app.get_text_input_visible() {
                    hide_text_input(&app);
                }
                hide_previews(&app);
                st.interaction = Interaction::Panning { last: (cx, cy) };
                app.set_status(SharedString::from("平移中 · 拖动移动画布"));
                return;
            }
            // 输入框流转：文字工具下再次点击 = 先提交当前文字再开新框；
            // 其他工具下点击（含切工具回调） = 丢弃输入
            let current_tool = Tool::from_id(app.get_active_tool());
            if app.get_text_input_visible() && current_tool != Tool::Text {
                hide_text_input(&app);
            }
            match current_tool {
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
                Tool::Text => {
                    // 点击落点 → 在点击处弹出输入框（画布坐标定位）
                    // 若上一个输入框有内容，先提交（点他处 = 确认当前文字）
                    if app.get_text_input_visible() && commit_text_input(&app, &mut st) {
                        update_overlay(&app, &st);
                    }
                    hide_previews(&app);
                    app.set_text_input_x(cx);
                    app.set_text_input_y(cy);
                    app.set_text_draft(SharedString::from(""));
                    app.set_text_input_visible(true);
                    app.set_status(SharedString::from("输入文字后按 Enter 确认"));
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

    // 7. 异步加载结果轮询：主线程应用解码完成的切图（过期序号丢弃）
    let load_timer = Timer::default();
    {
        let weak = app.as_weak();
        let state = state.clone();
        let load_seq = load_seq.clone();
        let load_queue = load_queue.clone();
        load_timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
            // 取出队列中最新一条未过期结果应用
            let result: Option<LoadResult> = {
                let mut q = match load_queue.lock() {
                    Ok(q) => q,
                    Err(_) => return,
                };
                let latest_seq = load_seq.load(Ordering::Relaxed);
                // 只保留序号最新的结果（保留最后一个匹配项）
                let mut picked = None;
                while let Some(r) = q.pop() {
                    if r.seq == latest_seq {
                        picked = Some(r);
                    }
                }
                picked
            };
            if let Some(r) = result {
                if let Some(app) = weak.upgrade() {
                    apply_load_result(&app, &state, r);
                }
            }
        });
    }

    println!("[ui] Slint 窗口启动");
    app.run()?;

    // timer 需保持存活到窗口关闭
    std::mem::drop(timer);
    std::mem::drop(load_timer);
    Ok(())
}
