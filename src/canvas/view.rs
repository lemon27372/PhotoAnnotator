// 视图变换：图片像素坐标 ↔ 画布显示坐标
//
// 架构约定（核心）：
//   标注数据一律以「图片像素坐标」存储 —— 缩放/平移只影响显示层，
//   不影响标注坐标。显示层通过本结构在两种坐标系间换算。
//
//   图片坐标  --(scale, offset)-->  画布坐标
//   canvas = image * scale + offset

#[derive(Clone, Copy, Debug)]
pub struct ViewTransform {
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self { scale: 1.0, offset_x: 0.0, offset_y: 0.0 }
    }
}

impl ViewTransform {
    /// 图片像素坐标 → 画布显示坐标
    pub fn image_to_canvas(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + self.offset_x, y * self.scale + self.offset_y)
    }

    /// 画布显示坐标 → 图片像素坐标（标注工具落点用）
    pub fn canvas_to_image(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.offset_x) / self.scale, (y - self.offset_y) / self.scale)
    }

    /// 计算「适应视口」变换：等比缩放 + 居中；图片小于视口时不放大
    pub fn fit(viewport_w: f32, viewport_h: f32, image_w: f32, image_h: f32) -> Self {
        if image_w <= 0.0 || image_h <= 0.0 || viewport_w <= 0.0 || viewport_h <= 0.0 {
            return Self::default();
        }
        let scale = (viewport_w / image_w).min(viewport_h / image_h).min(1.0);
        let offset_x = ((viewport_w - image_w * scale) / 2.0).max(0.0);
        let offset_y = ((viewport_h - image_h * scale) / 2.0).max(0.0);
        Self { scale, offset_x, offset_y }
    }

    /// 以任意画布坐标点为锚点缩放：缩放后锚点下的图片像素保持不动
    /// （鼠标滚轮缩放用鼠标位置作锚点，体验自然）
    pub fn zoom_at(&self, factor: f32, anchor_x: f32, anchor_y: f32) -> Self {
        let new_scale = (self.scale * factor).clamp(0.05, 20.0);
        // 锚点处对应的图片坐标（缩放前）
        let (ix, iy) = self.canvas_to_image(anchor_x, anchor_y);
        // 让该图片坐标在缩放后仍落在锚点位置
        let offset_x = anchor_x - ix * new_scale;
        let offset_y = anchor_y - iy * new_scale;
        Self { scale: new_scale, offset_x, offset_y }
    }
}
