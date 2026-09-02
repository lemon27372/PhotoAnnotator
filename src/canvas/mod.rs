// 画布层：TinySkia 2D 渲染
// 第一阶段仅做渲染自检；标注图元（矩形/椭圆/箭头/画笔/马赛克）在第二阶段实现

pub mod render;

pub use render::render_self_test;
