// 画布层：图片加载 + TinySkia 2D 渲染
// 第二阶段：图片加载 → Slint Image；渲染自检仍在 render 模块

pub mod loader;
pub mod render;

pub use loader::load_image;
