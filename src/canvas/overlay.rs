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
            };
            stroke_path(pixmap, &path, b.color, b.width);
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
