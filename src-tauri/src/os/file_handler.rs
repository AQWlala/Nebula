//! v1.7: 文件关联 + OS 文件拖入处理。
//!
//! 设计文档 v7.0 §6 OS 集成 — 文件关联与拖拽。
//!
//! Phase 6 实现范围：
//! * 双击 .md/.txt/.hermes/.hmemory 文件 → 通过 argv 或 open-file 事件接收
//!   → emit `nebula://open-file` 事件给前端
//! * 拖拽文件到窗口 → 通过 WindowEvent::DragDrop 接收
//!   → emit `nebula://drag-drop` 事件给前端
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

/// 发射 `nebula://open-file` 事件给前端。
pub fn emit_open_file(app: &AppHandle, path: &std::path::Path) {
    let path_str = path.to_string_lossy().to_string();
    tracing::info!(
        target: "nebula.os.file_handler",
        path = %path_str,
        "file open requested"
    );
    let _ = app.emit("nebula://open-file", &path_str);
}

/// 发射 `nebula://drag-drop` 事件给前端。
pub fn emit_drag_drop(app: &AppHandle, paths: &[PathBuf]) {
    let path_strs: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    tracing::info!(
        target: "nebula.os.file_handler",
        count = path_strs.len(),
        "files dropped onto window"
    );
    let _ = app.emit("nebula://drag-drop", &path_strs);
}

/// T-E-D-06: 发射 `nebula://ball-drag-drop` 事件 — 悬浮球窗口专用拖拽事件。
///
/// 与 `emit_drag_drop` 分开,避免主窗口的 drag-drop 监听器
/// (在 App.tsx 中切换到 code 模式 + 打开文件)与悬浮球的 absorb
/// 监听器同时触发。悬浮球拖拽语义是"吸收到记忆",而主窗口拖拽
/// 语义是"打开为代码文件",两者互斥。
pub fn emit_ball_drag_drop(app: &AppHandle, paths: &[PathBuf]) {
    let path_strs: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    tracing::info!(
        target: "nebula.os.file_handler",
        count = path_strs.len(),
        "files dropped onto floating-ball window"
    );
    let _ = app.emit("nebula://ball-drag-drop", &path_strs);
}

/// T-E-D-06: 发射 `nebula://ask-file` 事件 — 右键"问Nebula"触发。
///
/// 右键链路:用户右键文件 → "问Nebula" → `nebula.exe --ask <path>`
/// → single-instance 拦截(未实现) / 当前进程 argv 解析 →
/// emit 此事件 → App.tsx listen → 切到 chat + 预填输入框。
pub fn emit_ask_file(app: &AppHandle, path: &std::path::Path) {
    let path_str = path.to_string_lossy().to_string();
    tracing::info!(
        target: "nebula.os.file_handler",
        path = %path_str,
        "ask-file requested (right-click menu)"
    );
    let _ = app.emit("nebula://ask-file", &path_str);
}

/// T-E-D-06: 检测 argv 中的 `--ask <path>` 参数,若存在则 emit
/// `nebula://ask-file` 事件。
///
/// 在 lib.rs setup 中调用。argv 格式:`nebula.exe --ask <file-path>`。
/// 只处理第一个 `--ask` 后的第一个路径参数,后续参数忽略。
pub fn handle_ask_argv(app: &AppHandle) {
    let args: Vec<String> = std::env::args().collect();
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--ask" {
            if let Some(path_arg) = iter.next() {
                let path = PathBuf::from(path_arg);
                if path.is_file() {
                    emit_ask_file(app, &path);
                } else {
                    tracing::warn!(
                        target: "nebula.os.file_handler",
                        path = %path_arg,
                        "--ask path is not a file; ignoring"
                    );
                }
                return;
            }
        }
    }
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
