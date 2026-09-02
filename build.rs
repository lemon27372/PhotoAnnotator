// Slint UI 编译脚本：将 ui/app.slint 编译为 Rust 模块
fn main() {
    slint_build::compile("ui/app.slint").expect("Slint UI 编译失败");
}
