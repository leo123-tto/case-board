pub const DEFAULT_SPREADSHEET_DIGEST_MAX_CHARS: usize = 12_000;

const HEAD_ROWS: usize = 24;
const TAIL_ROWS: usize = 12;
const KEYWORD_ROWS: usize = 36;
const MAX_LINE_CHARS: usize = 260;

const KEYWORDS: &[&str] = &[
    "案号",
    "法院",
    "仲裁",
    "原告",
    "被告",
    "申请人",
    "被申请人",
    "项目",
    "房号",
    "客户",
    "业主",
    "姓名",
    "金额",
    "价款",
    "佣金",
    "提成",
    "回款",
    "扣款",
    "付款",
    "收款",
    "欠款",
    "逾期",
    "违约",
    "合同",
    "协议",
    "日期",
    "时间",
    "结算",
    "备注",
];

pub fn is_spreadsheet_filename(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    [".xls", ".xlsx", ".csv", ".tsv"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

pub fn spreadsheet_text_to_markdown_digest(
    filename: &str,
    text: &str,
    max_chars: usize,
) -> Option<String> {
    if !is_spreadsheet_filename(filename) {
        return None;
    }
    let max_chars = max_chars.max(800);
    let raw_chars = text.chars().count();
    let lines = normalized_non_empty_lines(text);
    if lines.is_empty() {
        return Some(format!(
            "# 表格材料摘要: {filename}\n\n- 原始表格文本为空。\n"
        ));
    }

    let mut digest = String::new();
    digest.push_str(&format!("# 表格材料摘要: {filename}\n\n"));
    digest.push_str(
        "> 该文件为表格材料,仅保留结构、首尾样本和关键词行供 LLM 分析;完整明细仍保留在本地原始提取文本中。\n\n",
    );
    digest.push_str("## 概览\n\n");
    digest.push_str(&format!(
        "- 原始字符数: {raw_chars}\n- 非空行数: {}\n",
        lines.len()
    ));
    if let Some(columns) = guess_columns(&lines) {
        digest.push_str(&format!("- 可能字段: {}\n", columns.join(" / ")));
    }
    digest.push('\n');

    append_lines_section(
        &mut digest,
        "开头样本",
        lines.iter().take(HEAD_ROWS).map(String::as_str),
    );

    let keyword_lines = collect_keyword_lines(&lines);
    if !keyword_lines.is_empty() {
        append_lines_section(
            &mut digest,
            "关键词行",
            keyword_lines.iter().map(String::as_str),
        );
    }

    let tail_start = lines.len().saturating_sub(TAIL_ROWS);
    append_lines_section(
        &mut digest,
        "结尾样本",
        lines.iter().skip(tail_start).map(String::as_str),
    );

    Some(clip_digest(&digest, max_chars))
}

fn normalized_non_empty_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(clip_line)
        .collect()
}

fn clip_line(line: &str) -> String {
    let count = line.chars().count();
    if count <= MAX_LINE_CHARS {
        return line.to_string();
    }
    let keep = MAX_LINE_CHARS.saturating_sub(12);
    format!("{}…(已截断)", line.chars().take(keep).collect::<String>())
}

fn guess_columns(lines: &[String]) -> Option<Vec<String>> {
    lines.iter().take(8).find_map(|line| {
        let cells = split_cells(line);
        (cells.len() >= 2).then(|| cells.into_iter().take(24).collect())
    })
}

fn split_cells(line: &str) -> Vec<String> {
    let separators = ['\t', '|', ',', '，', ';', '；'];
    let cells = if line.contains('\t') {
        line.split('\t').collect::<Vec<_>>()
    } else if line.contains('|') {
        line.split('|').collect::<Vec<_>>()
    } else if line.contains(',') || line.contains('，') {
        line.split([',', '，']).collect::<Vec<_>>()
    } else {
        line.split(|ch: char| separators.contains(&ch))
            .collect::<Vec<_>>()
    };
    cells
        .into_iter()
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .map(|cell| cell.chars().take(40).collect::<String>())
        .collect()
}

fn collect_keyword_lines(lines: &[String]) -> Vec<String> {
    let mut picked = Vec::new();
    for line in lines {
        if picked.len() >= KEYWORD_ROWS {
            break;
        }
        if KEYWORDS.iter().any(|kw| line.contains(kw)) && !picked.contains(line) {
            picked.push(line.clone());
        }
    }
    picked
}

fn append_lines_section<'a, I>(digest: &mut String, title: &str, lines: I)
where
    I: IntoIterator<Item = &'a str>,
{
    digest.push_str(&format!("## {title}\n\n```text\n"));
    for line in lines {
        digest.push_str(line);
        digest.push('\n');
    }
    digest.push_str("```\n\n");
}

fn clip_digest(digest: &str, max_chars: usize) -> String {
    let total = digest.chars().count();
    if total <= max_chars {
        return digest.to_string();
    }
    let marker = "\n\n【表格摘要已按长度预算截断】\n\n";
    let marker_len = marker.chars().count();
    if max_chars <= marker_len + 40 {
        return digest.chars().take(max_chars).collect();
    }
    let content_budget = max_chars - marker_len;
    let head_budget = content_budget * 3 / 4;
    let tail_budget = content_budget - head_budget;
    let head = digest.chars().take(head_budget).collect::<String>();
    let tail = digest
        .chars()
        .rev()
        .take(tail_budget)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}{marker}{tail}")
}
