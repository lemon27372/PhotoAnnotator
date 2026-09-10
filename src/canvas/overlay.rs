// 标注层渲染：把标注矢量数据绘制到「图片尺寸的透明位图」上
//
// 显示：标注层作为独立 Image 覆盖在原图上，随视图一起缩放平移
// 导出：后续把标注层合成到原图像素即可得到最终结果（所见即所得）

use tiny_skia::{Color, LineCap, Paint, Path, PathBuilder, Pixmap, Rect, Shader, Stroke, Transform};

use super::annotation::{Annotation, AnnotationStore, MOSAIC_BLOCK, NUMBER_FONT_SIZE, NUMBER_RADIUS};

/// 渲染标注层，返回 (宽, 高, straight-alpha RGBA 字节)
/// 仅矢量图元入 overlay 位图；文字/序号由 Slint 元素显示（此处跳过）
/// base_rgba：原图 straight-alpha 像素——马赛克需要从中取样（长度须为 w*h*4，否则马赛克跳过）
pub fn render_overlay(
    width: u32,
    height: u32,
    store: &AnnotationStore,
    base_rgba: &[u8],
) -> Option<(u32, u32, Vec<u8>)> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?; // 初始全透明
    let base = (base_rgba.len() == width as usize * height as usize * 4)
        .then_some((base_rgba, width, height));

    for a in store.items.iter() {
        draw_annotation(&mut pixmap, a, None, base);
    }

    Some((width, height, unpremultiply(pixmap.data())))
}

/// 合成最终结果：把标注绘制到原图之上（统一合成路径）
/// background_rgba 为 straight-alpha RGBA 原图数据（长度须为 w*h*4）
/// 返回 premultiplied 的 tiny-skia Pixmap（内部状态），供 PNG 编码或转换
pub fn composite_to_pixmap(
    width: u32,
    height: u32,
    background_rgba: &[u8],
    store: &AnnotationStore,
) -> Option<Pixmap> {
    let len = width as usize * height as usize * 4;
    if len == 0 || background_rgba.len() != len {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?;
    // straight-alpha → premultiplied 逐像素转换（透明像素需处理，不能直接拷贝）
    {
        let data = pixmap.data_mut();
        for (src, dst) in background_rgba.chunks_exact(4).zip(data.chunks_exact_mut(4)) {
            let a = src[3] as u32;
            dst[3] = src[3];
            if a == 0 {
                dst[0] = 0;
                dst[1] = 0;
                dst[2] = 0;
            } else {
                dst[0] = ((src[0] as u32 * a) / 255) as u8;
                dst[1] = ((src[1] as u32 * a) / 255) as u8;
                dst[2] = ((src[2] as u32 * a) / 255) as u8;
            }
        }
    }

    // 导出合成需要文字渲染 → 加载中文字体（一次）
    let font = super::font::load_cjk_font();
    let base = Some((background_rgba, width, height));
    for a in store.items.iter() {
        draw_annotation(&mut pixmap, a, font.as_ref(), base);
    }

    Some(pixmap)
}

/// 合成 → straight-alpha RGBA 字节（剪贴板粘贴/覆盖保存用）
pub fn composite_rgba(
    width: u32,
    height: u32,
    background_rgba: &[u8],
    store: &AnnotationStore,
) -> Option<Vec<u8>> {
    let pixmap = composite_to_pixmap(width, height, background_rgba, store)?;
    Some(unpremultiply(pixmap.data()))
}

fn draw_annotation(
    pixmap: &mut Pixmap,
    a: &Annotation,
    font: Option<&fontdue::Font>,
    base: Option<(&[u8], u32, u32)>,
) {
    match a {
        Annotation::Rect(b) | Annotation::Ellipse(b) => {
            let w = b.x2 - b.x1;
            let h = b.y2 - b.y1;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let Some(rect) = Rect::from_xywh(b.x1, b.y1, w, h) else {
                return;
            };
            let path = match a {
                Annotation::Rect(_) => PathBuilder::from_rect(rect),
                // 椭圆用外接矩形 push_oval
                Annotation::Ellipse(_) => {
                    let mut pb = PathBuilder::new();
                    pb.push_oval(rect);
                    pb.finish().unwrap_or_else(|| PathBuilder::from_rect(rect))
                }
                _ => unreachable!(),
            };
            stroke_path(pixmap, &path, b.color, b.width);
        }
        Annotation::Arrow(l) => {
            // 箭头线体：平头(Butt)截止于终点，避免圆头凸出盖过箭尖
            let mut pb = PathBuilder::new();
            pb.move_to(l.x1, l.y1);
            pb.line_to(l.x2, l.y2);
            if let Some(path) = pb.finish() {
                let paint = Paint {
                    shader: Shader::SolidColor(color_from_u32(l.color)),
                    ..Default::default()
                };
                let stroke = Stroke {
                    width: l.width,
                    line_cap: LineCap::Butt,
                    ..Default::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
            // 箭头头部（实心三角）
            if let Some([tip, left, right]) =
                super::annotation::arrow_head_points(l.x1, l.y1, l.x2, l.y2, l.width)
            {
                let mut pb = PathBuilder::new();
                pb.move_to(tip.0, tip.1);
                pb.line_to(left.0, left.1);
                pb.line_to(right.0, right.1);
                pb.close();
                if let Some(path) = pb.finish() {
                    let paint = Paint {
                        shader: Shader::SolidColor(color_from_u32(l.color)),
                        ..Default::default()
                    };
                    pixmap.fill_path(
                        &path,
                        &paint,
                        tiny_skia::FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
        }
        Annotation::Pen(p) => {
            // 自由画笔：折线描边（圆头圆角连接，手感平滑）
            let mut pb = PathBuilder::new();
            if let Some((x, y)) = p.points.first() {
                pb.move_to(*x, *y);
            }
            for (x, y) in p.points.iter().skip(1) {
                pb.line_to(*x, *y);
            }
            if let Some(path) = pb.finish() {
                let paint = Paint {
                    shader: Shader::SolidColor(color_from_u32(p.color)),
                    ..Default::default()
                };
                let stroke = Stroke {
                    width: p.width,
                    line_cap: LineCap::Round,
                    line_join: tiny_skia::LineJoin::Round,
                    ..Default::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
        Annotation::Text(t) => {
            // 导出时文字栅格化绘制；无字体可用则跳过（显示层不受影响）
            if let Some(font) = font {
                draw_text(pixmap, t, font);
            }
        }
        Annotation::Mosaic(b) => {
            // 马赛克：从基底像素分块取样（overlay 显示与导出合成共用同一条路径）
            draw_mosaic(pixmap, b, base);
        }
        Annotation::Number(n) => {
            // 序号：圆底 + 白色数字（导出/合成时栅格化；画布显示由 Slint 元素负责）
            if let Some(font) = font {
                draw_number(pixmap, n, font);
            }
        }
    }
}

/// 马赛克：按 MOSAIC_BLOCK 分块取基底平均色，回填为不透明色块
fn draw_mosaic(
    pixmap: &mut Pixmap,
    b: &super::annotation::BoxShape,
    base: Option<(&[u8], u32, u32)>,
) {
    let Some((bg, bw, _bh)) = base else {
        return;
    };
    if b.x2 - b.x1 <= 0.5 || b.y2 - b.y1 <= 0.5 {
        return;
    }
    let (pw, ph) = (pixmap.width(), pixmap.height());
    // 区域裁剪到图片范围内
    let x0 = b.x1.max(0.0).min(pw as f32) as i32;
    let y0 = b.y1.max(0.0).min(ph as f32) as i32;
    let x1 = b.x2.max(0.0).min(pw as f32) as i32;
    let y1 = b.y2.max(0.0).min(ph as f32) as i32;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let block = MOSAIC_BLOCK.max(2.0) as i32;
    let data = pixmap.data_mut();
    let mut by = y0;
    while by < y1 {
        let ey = (by + block).min(y1);
        let mut bx = x0;
        while bx < x1 {
            let ex = (bx + block).min(x1);
            // 块内平均（straight RGBA）
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for yy in by..ey {
                for xx in bx..ex {
                    let i = ((yy as u32 * bw + xx as u32) * 4) as usize;
                    if i + 3 >= bg.len() {
                        continue;
                    }
                    sum[0] += bg[i] as u32;
                    sum[1] += bg[i + 1] as u32;
                    sum[2] += bg[i + 2] as u32;
                    sum[3] += bg[i + 3] as u32;
                    n += 1;
                }
            }
            if n == 0 {
                bx = ex;
                continue;
            }
            let (ar, ag, ab, aa) = (sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n);
            // 写回 pixmap（premultiplied）
            let (pr, pg, pb) = if aa == 0 {
                (0, 0, 0)
            } else {
                ((ar * aa) / 255, (ag * aa) / 255, (ab * aa) / 255)
            };
            for yy in by..ey {
                for xx in bx..ex {
                    let i = ((yy as u32 * pw + xx as u32) * 4) as usize;
                    data[i] = pr as u8;
                    data[i + 1] = pg as u8;
                    data[i + 2] = pb as u8;
                    data[i + 3] = aa as u8;
                }
            }
            bx = ex;
        }
        by = ey;
    }
}

/// 序号：实心圆底 + 白色数字（水平/垂直居中）
fn draw_number(
    pixmap: &mut Pixmap,
    n: &super::annotation::NumberShape,
    font: &fontdue::Font,
) {
    let mut pb = PathBuilder::new();
    pb.push_circle(n.x, n.y, NUMBER_RADIUS);
    if let Some(path) = pb.finish() {
        let paint = Paint {
            shader: Shader::SolidColor(color_from_u32(n.color)),
            ..Default::default()
        };
        pixmap.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let text = n.value.to_string();
    let size = NUMBER_FONT_SIZE;
    let total_w: f32 = text
        .chars()
        .map(|c| font.metrics(c, size).advance_width)
        .sum();
    let metrics = font.horizontal_line_metrics(size);
    let ascent = metrics.as_ref().map(|m| m.ascent).unwrap_or(size * 0.8);
    let descent = metrics.as_ref().map(|m| m.descent).unwrap_or(size * 0.2);
    let baseline = n.y + (ascent - descent) * 0.5;
    let mut pen_x = n.x - total_w * 0.5;
    for ch in text.chars() {
        let (m, coverage) = font.rasterize(ch, size);
        let px = pen_x + m.xmin as f32;
        let py = baseline - m.ymin as f32 - m.height as f32;
        blend_coverage(pixmap, px, py, m.width as u32, m.height as u32, &coverage, 255.0, 255.0, 255.0);
        pen_x += m.advance_width;
    }
}

/// 文字逐字符栅格化并按 src-over 合成到 premultiplied pixmap
fn draw_text(pixmap: &mut Pixmap, t: &super::annotation::TextAnnotation, font: &fontdue::Font) {
    let size = t.font_size.max(4.0);
    let ascent = font
        .horizontal_line_metrics(size)
        .map(|m| m.ascent)
        .unwrap_or(size * 0.8);
    let baseline = t.y + ascent;
    // straight 颜色分量（alpha 全不透明）
    let (cr, cg, cb) = (
        ((t.color >> 16) & 0xFF) as f32,
        ((t.color >> 8) & 0xFF) as f32,
        (t.color & 0xFF) as f32,
    );
    let mut pen_x = t.x;
    for ch in t.text.chars() {
        let (metrics, coverage) = font.rasterize(ch, size);
        let px = pen_x + metrics.xmin as f32;
        let py = baseline + metrics.ymin as f32;
        blend_coverage(pixmap, px, py, metrics.width as u32, metrics.height as u32, &coverage, cr, cg, cb);
        pen_x += metrics.advance_width;
    }
}

/// 将 coverage（0-255 alpha）按颜色 src-over 合成到 pixmap（premultiplied）
fn blend_coverage(
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    w: u32,
    h: u32,
    coverage: &[u8],
    cr: f32,
    cg: f32,
    cb: f32,
) {
    let pw = pixmap.width() as f32;
    let ph = pixmap.height() as f32;
    for row in 0..h {
        for col in 0..w {
            let a = coverage[(row * w + col) as usize];
            if a == 0 {
                continue;
            }
            let dx = x + col as f32;
            let dy = y + row as f32;
            if dx < 0.0 || dy < 0.0 || dx >= pw || dy >= ph {
                continue;
            }
            let idx = (dy as u32 * pixmap.width() + dx as u32) as usize * 4;
            let sa = a as f32 / 255.0;
            let inv = 1.0 - sa;
            let data = pixmap.data_mut();
            let (dr, dg, db, da) = (
                data[idx] as f32,
                data[idx + 1] as f32,
                data[idx + 2] as f32,
                data[idx + 3] as f32,
            );
            data[idx] = (cr * sa + dr * inv).round() as u8;
            data[idx + 1] = (cg * sa + dg * inv).round() as u8;
            data[idx + 2] = (cb * sa + db * inv).round() as u8;
            data[idx + 3] = (255.0 * sa + da * inv).round() as u8;
        }
    }
}

/// 用指定颜色/线宽描边一条路径（tiny-skia 0.11 API）
fn stroke_path(pixmap: &mut Pixmap, path: &Path, color_u32: u32, width: f32) {
    let paint = Paint {
        shader: Shader::SolidColor(color_from_u32(color_u32)),
        ..Default::default()
    };
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        ..Default::default()
    };
    pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
}

/// 0xRRGGBB → tiny_skia::Color（不透明）
fn color_from_u32(c: u32) -> Color {
    Color::from_rgba8(((c >> 16) & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, (c & 0xFF) as u8, 255)
}

/// tiny-skia 输出为 premultiplied alpha；Slint 的 RGBA8 期望 straight alpha，
/// 需转换避免抗锯齿边缘发暗
fn unpremultiply(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    for (px, dst) in data.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let a = px[3] as u32;
        if a == 0 {
            continue; // 全透明：RGB 保持 0
        }
        let (r, g, b) = (px[0] as u32, px[1] as u32, px[2] as u32);
        dst[0] = ((r * 255) / a).min(255) as u8;
        dst[1] = ((g * 255) / a).min(255) as u8;
        dst[2] = ((b * 255) / a).min(255) as u8;
        dst[3] = px[3];
    }
    out
}
