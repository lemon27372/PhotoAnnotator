// 标注数据模型
//
// 架构约定：所有图元坐标一律使用「图片像素坐标」存储，
// 与视图缩放/平移无关（见 canvas/view.rs 的换算说明）。
//
// 矩形与椭圆共用 BoxShape（外接矩形两点式）——两者数据同构，
// 仅渲染方式不同（stroke rect path / stroke oval path）

/// 当前激活的画布工具（id 与 .slint active-tool 对应）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Browse = 0,
    Rect = 1,
    Ellipse = 2,
    Arrow = 3,
    Pen = 4,
}

impl Tool {
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => Tool::Rect,
            2 => Tool::Ellipse,
            3 => Tool::Arrow,
            4 => Tool::Pen,
            _ => Tool::Browse,
        }
    }
}

/// 默认标注样式（后续支持颜色/粗细调整）
pub const DEFAULT_COLOR: u32 = 0xF44336; // 错误红
pub const DEFAULT_STROKE_WIDTH: f32 = 3.0; // 框类（矩形/椭圆）线宽，图片像素
pub const ARROW_STROKE_WIDTH: f32 = 5.0; // 箭头线宽（视觉上应比框粗，图片像素）

/// 通用"框形"标注数据：外接矩形两点式（图片像素坐标）+ 样式
#[derive(Clone, Debug)]
pub struct BoxShape {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub color: u32, // 0xRRGGBB
    pub width: f32, // 描边宽度（图片像素）
}

impl BoxShape {
    /// 规范化（x1<=x2, y1<=y2），默认红色描边
    fn normalized(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self {
            x1: x1.min(x2),
            y1: y1.min(y2),
            x2: x1.max(x2),
            y2: y1.max(y2),
            color: DEFAULT_COLOR,
            width: DEFAULT_STROKE_WIDTH,
        }
    }

    /// 是否有有效面积（零宽/零高视为无效，丢弃）
    fn has_area(&self) -> bool {
        self.x2 - self.x1 > 0.5 && self.y2 - self.y1 > 0.5
    }
}

/// 线段类标注（箭头）：起点→终点（图片像素坐标）+ 样式
#[derive(Clone, Debug)]
pub struct LineShape {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub color: u32,
    pub width: f32,
}

impl LineShape {
    fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2, color: DEFAULT_COLOR, width: ARROW_STROKE_WIDTH }
    }

    /// 线段长度（图片像素）；过短视为无效
    fn length(&self) -> f32 {
        let dx = self.x2 - self.x1;
        let dy = self.y2 - self.y1;
        (dx * dx + dy * dy).sqrt()
    }
}

/// 箭头头部三角形顶点（按图片像素计算）：[箭尖, 尾左, 尾右]
/// 头部尺寸随线宽缩放保持协调：长 = 4×线宽，半宽 = 1.6×线宽
pub fn arrow_head_points(x1: f32, y1: f32, x2: f32, y2: f32, width: f32) -> Option<[(f32, f32); 3]> {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len); // 单位方向
    let (bx, by) = (-uy, ux); // 垂直单位向量
    let head_len = (width * 4.0).max(12.0);
    let head_half = (width * 1.6).max(5.0);
    let (bx_, by_) = (x2 - ux * head_len, y2 - uy * head_len); // 尾基线中点
    Some([
        (x2, y2),
        (bx_ + bx * head_half, by_ + by * head_half),
        (bx_ - bx * head_half, by_ - by * head_half),
    ])
}

/// 自由画笔标注：连续折线（图片像素坐标点序列）+ 样式
#[derive(Clone, Debug)]
pub struct PenShape {
    pub points: Vec<(f32, f32)>,
    pub color: u32,
    pub width: f32,
}

/// 统一的标注图元（后续扩展 Text…）
#[derive(Clone, Debug)]
pub enum Annotation {
    Rect(BoxShape),
    Ellipse(BoxShape),
    Arrow(LineShape),
    Pen(PenShape),
}

impl Annotation {
    /// 构造规范化矩形（默认红色描边）
    pub fn rect(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Annotation::Rect(BoxShape::normalized(x1, y1, x2, y2))
    }

    /// 构造规范化椭圆（外接矩形两点式，默认红色描边）
    pub fn ellipse(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Annotation::Ellipse(BoxShape::normalized(x1, y1, x2, y2))
    }

    /// 构造箭头（起点→终点，默认红色描边）
    pub fn arrow(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Annotation::Arrow(LineShape::new(x1, y1, x2, y2))
    }

    /// 构造自由画笔（折线点序列，默认红色描边）
    pub fn pen(points: Vec<(f32, f32)>) -> Self {
        Annotation::Pen(PenShape {
            points,
            color: DEFAULT_COLOR,
            width: DEFAULT_STROKE_WIDTH,
        })
    }

    /// 图元是否有有效面积/长度
    pub fn has_area(&self) -> bool {
        match self {
            Annotation::Rect(b) | Annotation::Ellipse(b) => b.has_area(),
            Annotation::Arrow(l) => l.length() > 2.0,
            Annotation::Pen(p) => p.points.len() >= 2,
        }
    }
}

/// 标注集合（仅撤销；重做已按产品决策移除，见 2026-09-01 开发日志）
#[derive(Default)]
pub struct AnnotationStore {
    pub items: Vec<Annotation>,
    /// 撤销快照栈：每次 push 前压入当前状态
    undo: Vec<Vec<Annotation>>,
}

impl AnnotationStore {
    pub fn new() -> Self {
        Self { items: Vec::new(), undo: Vec::new() }
    }

    /// 添加图元：当前状态入撤销栈
    pub fn push(&mut self, a: Annotation) {
        self.undo.push(self.items.clone());
        self.items.push(a);
    }

    /// 撤销最近一次添加；无可撤销返回 false
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo.pop() {
            self.items = prev;
            true
        } else {
            false
        }
    }

    /// 清空撤销历史（保存后调用：保存=提交点，不再可撤销）
    pub fn clear_history(&mut self) {
        self.undo.clear();
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}
