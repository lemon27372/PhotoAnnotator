// 缩略图：解码图片 → 等比缩略 → PNG 字节（SQLite thumbnails 表缓存）

use std::path::Path;

/// 生成等比缩略图 PNG（最长边限 size；不放大）
pub fn gen_thumbnail_png(path: &Path, size: u32) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?;
    // 计算目标尺寸（保持比例）
    let (w, h) = (img.width(), img.height());
    let scale = size as f32 / w.max(h).max(1) as f32;
    let (tw, th) = if scale >= 1.0 {
        (w, h) // 原图小于目标 → 不放大
    } else {
        ((w as f32 * scale).max(1.0) as u32, (h as f32 * scale).max(1.0) as u32)
    };
    let thumb = img.thumbnail(tw, th);
    let mut buf = std::io::Cursor::new(Vec::new());
    thumb.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}
