// 图片加载：解码磁盘图片 → 统一 RGBA8 → Slint Image
// 支持格式：PNG / JPG / BMP / GIF（静态帧）/ WebP（image crate 特性裁剪，见 Cargo.toml）

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// 加载图片文件并转为 Slint 可直接显示的 Image
/// 返回 (Image, 原始宽度, 原始高度) —— 宽高用于视图变换与标注坐标换算
pub fn load_image(path: &str) -> Result<(Image, u32, u32), Box<dyn std::error::Error>> {
    let decoded = image::open(path)?;
    let rgba = decoded.to_rgba8(); // 统一转 RGBA8，屏蔽源格式差异

    let (width, height) = rgba.dimensions();
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    // 直接写入底层字节缓冲区（RGBA8 每像素 4 字节，与 image crate 输出一致）
    buffer.make_mut_bytes().copy_from_slice(rgba.as_raw());

    Ok((Image::from_rgba8(buffer), width, height))
}
