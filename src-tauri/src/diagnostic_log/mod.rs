//! 进程内诊断日志环形缓冲(2026-05-26 V0.1.11)。
//!
//! 给反馈通道用——朋友的 Mac 上偶尔有报错但 stderr 上不来,
//! 把最近 200 行运行时日志收在内存里,反馈 MD 带出来,作者能复现。
//!
//! 设计:
//!   - 线程安全的 `Arc<Mutex<VecDeque<String>>>`,上限 200 行,FIFO
//!   - `dlog!()` 宏 = `eprintln!()` + push 到 buffer,跟原 `eprintln!` 行为一致
//!   - `install_panic_hook()` 把 panic 也写进 buffer
//!   - **隐私**:写入的每条都过一次 `feedback::sanitize_paths`,
//!     把 `/Users/xxx/...` 替换成 `<path>/basename`(防当事人姓名出现在路径里泄漏)
//!
//! 不用 `tracing` / `log` crate 的原因:依赖小、行为透明、迁移成本只是 sed 一次,
//! 也不用跟 tokio runtime 抢 subscriber 所有权。

use std::collections::VecDeque;
use std::sync::Mutex;

const RING_CAPACITY: usize = 200;
const RUNTIME_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

static RING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static FILE_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 推一条日志进 ring buffer。同时 `eprintln!` 到原 stderr(开发时仍可见)。
///
/// 内部用,生产代码请用 `dlog!()` 宏。
pub fn push_log(line: String) {
    // 先脱敏(路径里可能含当事人姓名)
    let safe = crate::feedback::sanitize_paths(&line);
    eprintln!("{}", safe);
    let ts = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string();
    if let Ok(mut g) = RING.lock() {
        if g.len() >= RING_CAPACITY {
            g.pop_front();
        }
        g.push_back(format!("[{}] {}", &ts[11..], safe));
    }
    append_runtime_log(&ts, &safe);
}

/// 持续运行日志落盘。达到 5 MiB 时把上一份轮换为 `caseboard.log.1`，最多保留两份，
/// 避免长期运行无限占用磁盘。失败静默，日志系统本身绝不能拖垮业务任务。
fn append_runtime_log(ts: &str, line: &str) {
    let Ok(_guard) = FILE_WRITE_LOCK.lock() else {
        return;
    };
    let Ok(base) = crate::db::app_data_dir() else {
        return;
    };
    let dir = base.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("caseboard.log");
    if std::fs::metadata(&path)
        .map(|meta| meta.len() >= RUNTIME_LOG_MAX_BYTES)
        .unwrap_or(false)
    {
        let backup = dir.join("caseboard.log.1");
        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::rename(&path, backup);
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[{}] {}", ts, line);
    }
}

/// 拿当前 ring buffer 快照(给反馈 MD 用)。
pub fn snapshot() -> Vec<String> {
    RING.lock()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default()
}

/// 安装 panic hook,把 panic 信息也写进 ring buffer(主进程崩溃前留下线索)。
///
/// 由 `lib.rs` 在启动早期调一次。
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "?".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<non-string payload>");
        push_log(format!("[panic] {} @ {}", payload, location));
        // 2026-06-11 Windows 闪退排查:release 是 panic=abort,任何线程 panic 都直接杀进程,
        // ring buffer(内存)随之蒸发 → 用户侧死无对证。这里在 abort 前**同步**把 panic 现场
        // 追加写进 <app_data_dir>/crash.log(含最近日志),用户把这个文件发来即可定位。
        write_crash_log(payload, &location);
        // 调原 hook,保持默认 stderr 输出 / abort 行为
        prev(info);
    }));
}

/// panic 现场落盘(append)。一切失败静默 —— 崩溃路径上绝不再制造二次 panic。
fn write_crash_log(payload: &str, location: &str) {
    let Ok(dir) = crate::db::app_data_dir() else {
        return;
    };
    // app 可能在首次建库前就崩,目录未必存在
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("crash.log");
    let recent = snapshot();
    let body = format!(
        "==== CaseBoard v{} panic @ {} ====\n[panic] {} @ {}\n--- 最近日志({} 行) ---\n{}\n\n",
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        crate::feedback::sanitize_paths(payload),
        location,
        recent.len(),
        recent.join("\n"),
    );
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(body.as_bytes());
        let _ = f.flush();
    }
}

/// 跟 `eprintln!` 等价的宏 — 但同时落到 ring buffer。
/// 用法跟 `eprintln!` 一致:`dlog!("[ocr] 限流被拒,等 {}s 重试", secs)`。
#[macro_export]
macro_rules! dlog {
    () => {
        $crate::diagnostic_log::push_log(String::new())
    };
    ($($arg:tt)*) => {
        $crate::diagnostic_log::push_log(format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_log_persists_runtime_log() {
        let marker = format!("runtime-log-test-{}", uuid::Uuid::new_v4());
        push_log(marker.clone());
        let path = crate::db::app_data_dir()
            .expect("app data dir")
            .join("logs")
            .join("caseboard.log");
        let body = std::fs::read_to_string(path).expect("runtime log should exist");
        assert!(body.contains(&marker));
    }
}
