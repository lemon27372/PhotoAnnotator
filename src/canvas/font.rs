// 文字导出渲染：字体栅格化（fontdue）
//
// TinySkia 不提供文本 API，导出/合成标注时需要把文字按像素画到图上。
// 使用系统中文字体（黑体 simhei.ttf），找不到时文字标注在导出中跳过（显示层不受影响）。

use std::path::Path;

/// 候选系统中文字体路径（Windows）
const FONT_CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\simhei.ttf",  // 黑体
    "C:\\Windows\\Fonts\\msyh.ttc",    // 微软雅黑（ttc 需集合索引，fontdue 支持有限，作后备）
    "C:\\Windows\\Fonts\\simsun.ttc",
];

/// 加载第一个可用的中文字体
pub fn load_cjk_font() -> Option<fontdue::Font> {
    for path in FONT_CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            // ttc 集合：尝试 collection index 0；纯 ttf 同样可经此解析
            match fontdue::Font::from_bytes(
                bytes,
                fontdue::FontSettings { collection_index: 0, ..Default::default() },
            ) {
                Ok(font) => return Some(font),
                Err(_) => continue,
            }
        }
    }
    None
}

/// 字体路径是否存在（供状态栏提示用）
pub fn font_available() -> bool {
    FONT_CANDIDATES.iter().any(|p| Path::new(p).exists())
}
