// 标注数据模型
//
// 架构约定：所有图元坐标一律使用「图片像素坐标」存储，
// 与视图缩放/平移无关（见 canvas/view.rs 的换算说明）。

/// 当前激活的画布工具（0=浏览/平移, 1=矩形；后续扩展椭圆/箭头/画笔…）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Browse = 0,
    Rect = 1,
}

impl Tool {
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => Tool::Rect,
            _ => Tool::Browse,
        }
    }
}

/// 默认标注样式（后续支持颜色/粗细调整）
pub const DEFAULT_COLOR: u32 = 0xF44336; // 错误红
pub const DEFAULT_STROKE_WIDTH: f32 = 3.0; // 图片像素

/// 矩形框标注（两点式，图片像素坐标）
#[derive(Clone, Debug)]
pub struct RectAnnotation {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub color: u32, // 0xRRGGBB
    pub width: f32, // 描边宽度（图片像素）
}

/// 统一的标注图元（后续扩展 Ellipse/Arrow/Pen/Text…）
#[derive(Clone, Debug)]
pub enum Annotation {
    Rect(RectAnnotation),
}

impl Annotation {
    /// 构造规范化矩形（x1<=x2, y1<=y2），默认红色描边
    pub fn rect(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Annotation::Rect(RectAnnotation {
            x1: x1.min(x2),
            y1: y1.min(y2),
            x2: x1.max(x2),
            y2: y1.max(y2),
            color: DEFAULT_COLOR,
            width: DEFAULT_STROKE_WIDTH,
        })
    }

    /// 矩形是否有有效面积（零宽/零高视为无效，丢弃）
    pub fn rect_has_area(&self) -> bool {
        match self {
            Annotation::Rect(r) => r.x2 - r.x1 > 0.5 && r.y2 - r.y1 > 0.5,
        }
    }
}

/// 标注集合（后续撤销/重做、SQLite 持久化都基于此）
#[derive(Default)]
pub struct AnnotationStore {
    pub items: Vec<Annotation>,
}

impl AnnotationStore {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, a: Annotation) {
        self.items.push(a);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}
