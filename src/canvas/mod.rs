// 画布层：图片加载 / 视图变换 / 标注模型 / 标注层渲染
//
// 阶段目标：
//   第二阶段 —— 图片加载、视图状态机、标注图元（矩形起步）

pub mod annotation;
pub mod loader;
pub mod overlay;
pub mod render;
pub mod view;

pub use annotation::{Annotation, AnnotationStore, Tool};
pub use loader::load_image;
pub use view::ViewTransform;
