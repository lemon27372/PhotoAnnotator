// TinySkia 渲染自检：绘制测试图并保存 PNG
// 验证绘图引擎可用（后续画布交互渲染基于此）
// 注：tiny-skia 0.11 API — Paint 使用 shader 字段，圆形用 fill_path

use std::path::PathBuf;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Shader, Transform};

/// 渲染一张 320x240 测试图（白色背景 + 强调蓝矩形 + 错误红圆）
/// 输出到 target/selftest_canvas.png
/// 注：第一阶段验证用，标注图元开发后此函数将退役；#[allow(dead_code)] 保留为回归工具
#[allow(dead_code)]
pub fn render_self_test() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut pixmap = Pixmap::new(320, 240).ok_or("创建画布失败")?;

    // 背景：纯白
    pixmap.fill(Color::WHITE);

    // 矩形：强调蓝 #4A90D9
    let rect = Rect::from_xywh(60.0, 50.0, 200.0, 120.0).ok_or("矩形坐标无效")?;
    let blue = Paint {
        shader: Shader::SolidColor(Color::from_rgba8(74, 144, 217, 255)),
        ..Default::default()
    };
    pixmap.fill_rect(rect, &blue, Transform::identity(), None);

    // 圆形：错误红 #F44336（画布中心）
    let mut pb = PathBuilder::new();
    pb.push_circle(160.0, 110.0, 30.0);
    let circle = pb.finish().ok_or("圆路径无效")?;
    let red = Paint {
        shader: Shader::SolidColor(Color::from_rgba8(244, 67, 54, 255)),
        ..Default::default()
    };
    pixmap.fill_path(&circle, &red, FillRule::Winding, Transform::identity(), None);

    let out = PathBuf::from("target").join("selftest_canvas.png");
    pixmap.save_png(&out)?;
    Ok(out)
}
