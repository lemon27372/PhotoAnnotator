// 图片加载：解码磁盘图片 → 统一 RGBA8 → Slint Image
// 支持格式：PNG / JPG / BMP / GIF（静态帧）/ WebP（image crate 特性裁剪，见 Cargo.toml）

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// 加载图片文件并转为 Slint 可直接显示的 Image
/// 返回 (Image, 原始宽度, 原始高度, straight-alpha RGBA 原始像素)
/// RGBA 像素供合成导出（保存/Ctrl+C）使用——Slint Image 不提供像素访问
pub fn load_image(path: &str) -> Result<(Image, u32, u32, Vec<u8>), Box<dyn std::error::Error>> {
    let decoded = image::open(path)?;
    let rgba = decoded.to_rgba8(); // 统一转 RGBA8，屏蔽源格式差异

    let (width, height) = rgba.dimensions();
    let raw = rgba.into_raw(); // Vec<u8> straight-alpha
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    // 直接写入底层字节缓冲区（RGBA8 每像素 4 字节，与 image crate 输出一致）
    buffer.make_mut_bytes().copy_from_slice(&raw);

    Ok((Image::from_rgba8(buffer), width, height, raw))
}
