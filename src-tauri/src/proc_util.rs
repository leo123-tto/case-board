//! 跨平台子进程小工具。

use tokio::process::Command;

/// 创建 tokio 子进程命令。Windows 的 npm/脚本入口通常只有 `.cmd`/`.bat`
/// (如 npx、lark-cli),`CreateProcessW` 不会像终端一样自动解析这些 shim。
pub(crate) fn tokio_command(program: &str) -> Command {
    #[cfg(windows)]
    if needs_windows_shell(program) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/D", "/C", program]);
        return cmd;
    }
    Command::new(program)
}

#[cfg(windows)]
fn needs_windows_shell(program: &str) -> bool {
    let trimmed = program.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        return true;
    }
    // 裸命令名在 Windows 上可能由 PATH 命中 npm 的 `.cmd` shim；显式路径/`.exe`
    // 继续直调,避免 cmd 对带空格绝对路径的二次 quoting。
    !trimmed.contains('/') && !trimmed.contains('\\') && !trimmed.contains('.')
}

/// Windows 下隐藏子进程的控制台窗口(`CREATE_NO_WINDOW`),避免 spawn 外部命令
/// (python / lark-cli 等)时闪一个黑色命令框。非 Windows 平台是 no-op。
///
/// 坑 #21:Windows 用户反馈「点辅助立案一直弹命令框」「飞书日历后台闪窗」,根因是
/// `Command::new` spawn 控制台子进程默认会带窗口。新增 spawn 外部命令的代码记得调一下。
pub(crate) fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// 同 [`hide_console_window`],但作用于标准库的 [`std::process::Command`]
/// (本仓库不少 spawn 用的是 `std::process::Command` 而非 tokio 版)。
pub(crate) fn hide_console_window_std(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
