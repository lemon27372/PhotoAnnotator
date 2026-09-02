// 标注层渲染：把标注矢量数据绘制到「图片尺寸的透明位图」上
//
// 显示：标注层作为独立 Image 覆盖在原图上，随视图一起缩放平移
// 导出：后续把标注层合成到原图像素即可得到最终结果（所见即所得）

use tiny_skia::{Color, LineCap, Paint, Path, PathBuilder, Pixmap, Rect, Shader, Stroke, Transform};

use super::annotation::{Annotation, AnnotationStore};

/// 渲染标注层，返回 (宽, 高, straight-alpha RGBA 字节)
/// 仅包含已完成的图元；绘制中预览走 Slint 原生元素（见 app.slint preview-*），
/// 避免拖动时全图重渲染导致卡顿
pub fn render_overlay(width: u32, height: u32, store: &AnnotationStore) -> Option<(u32, u32, Vec<u8>)> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?; // 初始全透明

    for a in store.items.iter() {
        draw_annotation(&mut pixmap, a);
    }

    Some((width, height, unpremultiply(pixmap.data())))
}

/// 合成最终结果：把标注绘制到原图（RGBA，不透明）之上 → PNG 字节
/// background_rgba 为 straight-alpha RGBA 原图数据（长度须为 w*h*4）
/// 这是导出/保存的统一路径：所见即所得
pub fn composite_png(
    width: u32,
    height: u32,
    background_rgba: &[u8],
    store: &AnnotationStore,
) -> Option<Vec<u8>> {
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

    for a in store.items.iter() {
        draw_annotation(&mut pixmap, a);
    }

    pixmap.encode_png().ok()
}

fn draw_annotation(pixmap: &mut Pixmap, a: &Annotation) {
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
                Annotation::Arrow(_) | Annotation::Pen(_) => unreachable!(),
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
