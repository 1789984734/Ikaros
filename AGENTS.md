# 项目开发规范

## Rust 代码格式

- 所有 Rust 代码统一使用 stable Rust 工具链自带的 `rustfmt` 默认规则，不引入仅 Nightly 可用的格式化选项。
- 修改任何 `.rs` 文件后，运行 `cargo fmt --all` 格式化整个工作区，不要只格式化单个文件。
- 交付前必须通过 `cargo fmt --all -- --check`。该检查是强制格式门禁，不使用 baseline、忽略列表或新增例外。
- `rustfmt` 只负责 Rust 代码；Slint 等其他文件沿用各自现有格式，不要用手工排版代替 Rust 格式化。
