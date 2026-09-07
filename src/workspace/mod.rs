// 工作目录模块：目录扫描与缩略图生成
//
// 第三阶段：以"项目/文件夹"为单位组织待标注图片

pub mod scan;
pub mod thumb;

pub use scan::scan_images;
pub use thumb::gen_thumbnail_png;
