// 标注数据模型
//
// 架构约定：所有图元坐标一律使用「图片像素坐标」存储，
// 与视图缩放/平移无关（见 canvas/view.rs 的换算说明）。
//
// 矩形与椭圆共用 BoxShape（外接矩形两点式）——两者数据同构，
// 仅渲染方式不同（stroke rect path / stroke oval path）

/// 当前激活的画布工具（id 与 .slint active-tool 对应）
/// 无「浏览」工具（2026-09-08 产品决策：平移 = 任意工具下中/右键按住拖动）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Rect = 0,
    Ellipse = 1,
    Arrow = 2,
    Pen = 3,
    Text = 4,
    /// 马赛克（拖框像素化，用于敏感信息打码）
    Mosaic = 5,
}

impl Tool {
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => Tool::Ellipse,
            2 => Tool::Arrow,
            3 => Tool::Pen,
            4 => Tool::Text,
            5 => Tool::Mosaic,
            _ => Tool::Rect,
        }
    }
}

/// 默认标注样式（后续支持颜色/粗细调整）
pub const DEFAULT_COLOR: u32 = 0xF44336; // 错误红
pub const DEFAULT_STROKE_WIDTH: f32 = 3.0; // 框类（矩形/椭圆）线宽，图片像素
pub const ARROW_STROKE_WIDTH: f32 = 5.0; // 箭头线宽（视觉上应比框粗，图片像素）
pub const DEFAULT_FONT_SIZE: f32 = 24.0; // 文字标注字号（图片像素）
pub const MOSAIC_BLOCK: f32 = 12.0; // 马赛克块边长（图片像素，固定粒度）

/// 通用"框形"标注数据：外接矩形两点式（图片像素坐标）+ 样式
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PenShape {
    pub points: Vec<(f32, f32)>,
    pub color: u32,
    pub width: f32,
}

/// 统一的标注图元
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Annotation {
    Rect(BoxShape),
    Ellipse(BoxShape),
    Arrow(LineShape),
    Pen(PenShape),
    Text(TextAnnotation),
    /// 马赛克区域（渲染/导出时从基底像素取样像素化）
    Mosaic(BoxShape),
}

/// 文字标注：锚点为文字左上角（图片像素坐标）
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TextAnnotation {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: u32,     // 0xRRGGBB
    pub font_size: f32, // 图片像素字号
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

    /// 构造文字标注（左上角锚点，默认红色）
    pub fn text(x: f32, y: f32, text: String) -> Self {
        Annotation::Text(TextAnnotation {
            x,
            y,
            text,
            color: DEFAULT_COLOR,
            font_size: DEFAULT_FONT_SIZE,
        })
    }

    /// 构造马赛克区域（外接矩形两点式；渲染时按 MOSAIC_BLOCK 取样像素化）
    pub fn mosaic(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Annotation::Mosaic(BoxShape::normalized(x1, y1, x2, y2))
    }

    /// 图元是否有有效面积/长度
    pub fn has_area(&self) -> bool {
        match self {
            Annotation::Rect(b)
            | Annotation::Ellipse(b)
            | Annotation::Mosaic(b) => b.has_area(),
            Annotation::Arrow(l) => l.length() > 2.0,
            Annotation::Pen(p) => p.points.len() >= 2,
            Annotation::Text(t) => !t.text.trim().is_empty(),
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

    /// 整体替换标注列表（加载持久化数据用；历史重置为加载时点）
    pub fn set_items(&mut self, items: Vec<Annotation>) {
        self.items = items;
        self.undo.clear();
    }

    /// 序列化当前标注（annotations 表 data 字段）
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.items).unwrap_or_else(|_| "[]".into())
    }

    /// 反序列化标注列表（逐条容错：单条无法解析则丢弃该条，保留其余；
    /// 例如旧版本写入过已被移除的图元类型时，不至于整份标注丢失）
    pub fn from_json(json: &str) -> Vec<Annotation> {
        let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
            return Vec::new();
        };
        values
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Annotation>(v).ok())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全类型 serde 往返（含马赛克）：持久化与切图恢复依赖此路径
    #[test]
    fn annotation_json_round_trip() {
        let mut store = AnnotationStore::new();
        store.push(Annotation::rect(1.0, 2.0, 30.0, 40.0));
        store.push(Annotation::ellipse(5.0, 6.0, 20.0, 25.0));
        store.push(Annotation::arrow(0.0, 0.0, 50.0, 50.0));
        store.push(Annotation::pen(vec![(1.0, 1.0), (3.0, 4.0)]));
        store.push(Annotation::text(7.0, 8.0, "标注".into()));
        store.push(Annotation::mosaic(10.0, 10.0, 60.0, 40.0));

        let json = store.to_json();
        let items = AnnotationStore::from_json(&json);
        assert_eq!(items.len(), 6);
        assert!(matches!(&items[5], Annotation::Mosaic(b) if (b.x2 - b.x1 - 50.0).abs() < 0.01));
    }

    /// 逐条容错：JSON 含无法解析的条目（如旧版本已移除的图元）时，其余标注仍保留
    #[test]
    fn from_json_skips_unknown_items() {
        let json = r#"[
            {"Rect":{"x1":1.0,"y1":2.0,"x2":30.0,"y2":40.0,"color":16711680,"width":3.0}},
            {"Number":{"x":15.0,"y":20.0,"value":3,"color":16711680}},
            {"Mosaic":{"x1":0.0,"y1":0.0,"x2":10.0,"y2":10.0,"color":16711680,"width":3.0}}
        ]"#;
        let items = AnnotationStore::from_json(json);
        assert_eq!(items.len(), 2, "未知图元应被跳过，其余两条保留");
        assert!(matches!(&items[0], Annotation::Rect(_)));
        assert!(matches!(&items[1], Annotation::Mosaic(_)));
    }

    /// 零尺寸马赛克/矩形视为无效（与既有 has_area 语义一致）
    #[test]
    fn zero_size_shapes_rejected() {
        assert!(!Annotation::mosaic(10.0, 10.0, 10.0, 30.0).has_area());
        assert!(Annotation::mosaic(10.0, 10.0, 40.0, 30.0).has_area());
    }
}
