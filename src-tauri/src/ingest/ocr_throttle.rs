use std::sync::OnceLock;
use std::time::Duration;

const SUBMIT_MIN_INTERVAL_MS: u64 = 1400;

static GLOBAL_SUBMIT_THROTTLE: OnceLock<SubmitThrottle> = OnceLock::new();

pub(crate) fn global_submit_throttle() -> &'static SubmitThrottle {
    GLOBAL_SUBMIT_THROTTLE.get_or_init(SubmitThrottle::new)
}

/// 全应用共享的云端 OCR 提交频控，案件抽取和事务工作区共用同一个闸门。
pub(crate) struct SubmitThrottle {
    last_submit: tokio::sync::Mutex<std::time::Instant>,
    min_interval: Duration,
}

impl SubmitThrottle {
    fn new() -> Self {
        Self::with_interval(Duration::from_millis(SUBMIT_MIN_INTERVAL_MS))
    }

    fn with_interval(min_interval: Duration) -> Self {
        Self {
            last_submit: tokio::sync::Mutex::new(
                std::time::Instant::now() - Duration::from_secs(60),
            ),
            min_interval,
        }
    }

    pub(crate) async fn acquire(&self) {
        loop {
            let mut last = self.last_submit.lock().await;
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(*last);
            if elapsed >= self.min_interval {
                *last = now;
                return;
            }
            let wait = self.min_interval - elapsed;
            drop(last);
            tokio::time::sleep(wait).await;
        }
    }
}

pub(crate) fn might_hit_mineru(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    lower.ends_with(".pdf")
        || super::extractor::is_ocr_image_ext(&lower)
        || super::extractor::is_office_cloud_ext(&lower)
}
