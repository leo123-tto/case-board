//! 文件夹扫描与文档分类。
//!
//! 入口: [`scanner::scan_folder`]
//! - 输入: 案件文件夹路径
//! - 输出: 该文件夹下所有文档的元数据 + 自动归类(stage / category / AI 产物标记)
//!
//! V0.1 阶段纯规则,不调 LLM。后续 V0.2 可在规则之上叠 LLM 兜底。

pub mod case_split;
pub mod extractor;
pub mod global_pipeline;
pub mod mineru_http;
pub mod ocr;
pub mod ocr_throttle;
pub mod paddle_vl_http;
pub mod pipeline;
pub mod ppocrv6_http;
pub mod sanitize;
pub mod scanner;
