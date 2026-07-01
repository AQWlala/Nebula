//! v1.7: 文件关联 + OS 文件拖入处理。
//!
//! 设计文档 v7.0 §6 OS 集成 — 文件关联与拖拽。
//!
//! Phase 6 实现范围：
//! * 双击 .md/.txt/.hermes/.hmemory 文件 → 通过 argv 或 open-file 事件接收
//!   → emit `nine-snake://open-file` 事件给前端
//! * 拖拽文件到窗口 → 通过 WindowEvent::DragDrop 接收
//!   → emit `nine-snake://drag-drop` 事件给前端
//!
//! 前端监听这两个事件，根据文件扩展名路由到对应模式：
//! * .md/.txt → 写作模式打开
//! * .hermes → Code 模式（项目文件）
//! * .hmemory → 记忆导入
//! * 其他 → Code 模式（通用文件）

use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// 处理通过命令行 argv 传入的文件路径（双击文件打开时 OS 会把路径作为 argv）。
/// 在 setup 闭包中调用，返回传入的文件路径（如果有）。
pub fn handle_argv_files(app: &AppHandle) {
    let args: Vec<String> = std::env::args().collect();
    // 跳过 args[0]（程序路径），找第一个看起来像文件路径的参数。
    for arg in args.iter().skip(1) {
        let path = PathBuf::from(arg);
        if path.is_file() {
            emit_open_file(app, &path);
            return; // 只处理第一个有效文件
        }
    }
}

/// 发射 `nine-snake://open-file` 事件给前端。
pub fn emit_open_file(app: &AppHandle, path: &std::path::Path) {
    let path_str = path.to_string_lossy().to_string();
    tracing::info!(
        target: "nine_snake.os.file_handler",
        path = %path_str,
        "file open requested"
    );
    let _ = app.emit("nine-snake://open-file", &path_str);
}

/// 发射 `nine-snake://drag-drop` 事件给前端。
pub fn emit_drag_drop(app: &AppHandle, paths: &[PathBuf]) {
    let path_strs: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    tracing::info!(
        target: "nine_snake.os.file_handler",
        count = path_strs.len(),
        "files dropped onto window"
    );
    let _ = app.emit("nine-snake://drag-drop", &path_strs);
}

/// 判断窗口是否可见（用于决定是否需要先 show 窗口）。
pub fn ensure_window_visible(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        if !w.is_visible().unwrap_or(false) {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}
