// 画布层：图片加载 + 视图变换 + TinySkia 2D 渲染
// 第二阶段：图片加载 / 缩放平移（视图变换层）

pub mod loader;
pub mod render;
pub mod view;

pub use loader::load_image;
pub use view::ViewTransform;
