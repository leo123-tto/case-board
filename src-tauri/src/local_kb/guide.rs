use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};

const GUIDE_TEMPLATE: &str = include_str!("guide.md");
const AI_ENTRY_START: &str = "<!-- CASEBOARD:AI-KB-ENTRY:START -->";
const AI_ENTRY_END: &str = "<!-- CASEBOARD:AI-KB-ENTRY:END -->";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalAiEntry {
    pub guide_path: PathBuf,
    pub manifest_path: PathBuf,
    pub agents_path: PathBuf,
    pub claude_path: PathBuf,
    pub instruction: String,
}

pub fn render(kb_root: Option<&Path>) -> String {
    let root = kb_root
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "未绑定；请先在设置中选择知识库目录".to_string());
    GUIDE_TEMPLATE.replace("{{KB_ROOT}}", &root)
}

fn bootstrap_block() -> String {
    format!(
        "{AI_ENTRY_START}\n\
         ## CaseBoard 法律知识库入口\n\n\
         在检索、入库、创建目录或治理本知识库前，必须先完整读取 \
         `.caseboard/AI-KB-GUIDE.md` 和 `.caseboard/kb-manifest.json`。\n\
         两者是当前目录职责、检索顺序、写入位置和高风险操作边界的唯一真相源；\
         如本文其他历史说明与它们冲突，以 `.caseboard/` 下的文件为准。\n\
         {AI_ENTRY_END}"
    )
}

fn upsert_bootstrap(path: &Path) -> std::io::Result<()> {
    let block = bootstrap_block();
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let next = match (existing.find(AI_ENTRY_START), existing.find(AI_ENTRY_END)) {
        (Some(start), Some(end)) if end >= start => {
            let suffix_start = end + AI_ENTRY_END.len();
            format!(
                "{}{}{}",
                &existing[..start],
                block,
                &existing[suffix_start..]
            )
        }
        _ if existing.trim().is_empty() => format!("{block}\n"),
        _ => format!("{block}\n\n{existing}"),
    };
    std::fs::write(path, next)
}

pub fn install_external_ai_entry(kb_root: &Path) -> std::io::Result<ExternalAiEntry> {
    let managed_dir = kb_root.join(".caseboard");
    std::fs::create_dir_all(&managed_dir)?;
    for relative in [
        "raw/notes",
        "raw/yuandian-cache",
        "raw/companies",
        "raw/cases-experience",
        "wiki/sources",
        "wiki/topics",
        "00_ARCHIVE",
    ] {
        std::fs::create_dir_all(kb_root.join(relative))?;
    }

    let guide_path = managed_dir.join("AI-KB-GUIDE.md");
    let manifest_path = managed_dir.join("kb-manifest.json");
    let agents_path = kb_root.join("AGENTS.md");
    let claude_path = kb_root.join("CLAUDE.md");

    std::fs::write(&guide_path, render(Some(kb_root)))?;
    let manifest = json!({
        "schema_version": 1,
        "guide": ".caseboard/AI-KB-GUIDE.md",
        "root": ".",
        "read_order": [
            ".caseboard/AI-KB-GUIDE.md",
            ".wiki-schema.md",
            "purpose.md",
            "wiki/index.md",
            "wiki/overview.md"
        ],
        "directories": {
            "raw/notes": {"role": "完整法规、案例、文章和其他原始正文", "searchable": true, "embedding": true, "write_mode": "create_only"},
            "raw/yuandian-cache": {"role": "元典搜索与详情缓存", "searchable": true, "embedding": "details_only", "write_mode": "caseboard_managed"},
            "raw/companies": {"role": "带查询时间的企业档案", "searchable": true, "embedding": false, "write_mode": "create_only"},
            "raw/cases-experience": {"role": "已提炼的办案经验卡片", "searchable": true, "embedding": false, "write_mode": "create_only"},
            "wiki/sources": {"role": "带真实 raw 回链的单篇导航卡", "searchable": true, "embedding": false, "write_mode": "reviewed_create_only"},
            "wiki/topics": {"role": "问题和场景导向的专题导航", "searchable": true, "embedding": false, "write_mode": "reviewed_create_only"},
            "00_ARCHIVE": {"role": "历史备份和回滚材料", "searchable": false, "embedding": false, "write_mode": "archive_only"},
            "_inbox": {"role": "不再使用的旧过程目录", "searchable": false, "embedding": false, "write_mode": "forbidden"}
        },
        "safe_operations": ["inspect", "search", "read", "create_missing_standard_directory", "create_new_l1_raw"],
        "requires_explicit_confirmation": ["overwrite", "delete", "move", "batch_rename", "promote_to_wiki", "rebuild_embedding_index"]
    });
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    upsert_bootstrap(&agents_path)?;
    upsert_bootstrap(&claude_path)?;

    Ok(ExternalAiEntry {
        guide_path: guide_path.clone(),
        manifest_path,
        agents_path,
        claude_path,
        instruction: format!(
            "请将知识库目录作为工作区打开，并先完整读取 `{}` 与同目录的 `kb-manifest.json`，再按其中规则检索、入库或维护。",
            guide_path.to_string_lossy()
        ),
    })
}
