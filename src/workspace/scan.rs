// 目录扫描：收集支持的图片文件

/// 支持的图片扩展名（与 image crate 启用特性一致）
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp"];

/// 扫描目录下的直接子目录（仅一级，P3 双击导航用），按文件名排序
pub fn scan_subdirs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // 目录（跳过隐藏？Windows 无 . 约定，保留全部）
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}

/// 扫描目录下的图片文件（仅当前目录，不递归），按文件名排序
pub fn scan_images(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if IMAGE_EXTS.iter().any(|s| s.eq_ignore_ascii_case(ext)) {
                    out.push(path);
                }
            }
        }
    }
    // 按文件名排序（稳定可预期；后续可加自然排序）
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}
