//! 法规全文按条提取(V0.2.2 · 法规全文缓存优化路径 B 的核心算法)。
//!
//! 背景:LLM 查法条时若每次只拉单条(`get_law_article`/`rh_ft_detail`),每条一个缓存
//! key,几乎不可能重复命中,白白消耗元典积分。改为优先拉**整部法规全文**
//! (`get_regulation_detail`/`rh_fg_detail`,当前 5 积分、key=法规 ID/名称+时点版本)缓存到本地,之后所有
//! 该法规的条文都从本地全文里**按条号提取**,0 积分、高命中。
//!
//! 本模块提供两个纯函数:阿拉伯条号 → 中文数字、从全文按条号提取单条。
//! 元典 `rh_fg_detail` 返回的 `data.content` 是整部法规的**一整块全文文本**,条文以
//! 「第X条」(中文数字)起头,到下一条「第X+1条」之间即该条正文(实测民法典 1260 条全部命中)。

/// 阿拉伯数字 → 中文数字(支持 1..=9999,够中国法规条号用)。
///
/// 例:`1 → "一"`、`10 → "十"`、`100 → "一百"`、`585 → "五百八十五"`、
/// `1000 → "一千"`、`1260 → "一千二百六十"`。
pub fn arabic_to_chinese(n: u32) -> String {
    if n == 0 {
        return "零".to_string();
    }
    let digits = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
    let units = ["", "十", "百", "千"];
    let s = n.to_string();
    let l = s.len();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        let dnum = ch.to_digit(10).unwrap() as usize;
        let pos = l - i - 1; // 个=0 十=1 百=2 千=3(n<=9999 → pos<=3,不越界)
        if dnum == 0 {
            // 中间的 0 补一个「零」,但不重复、末位 0 不补(靠后面 rstrip 收尾)
            if !out.ends_with('零') && i != l - 1 {
                out.push('零');
            }
        } else {
            out.push(digits[dnum]);
            // D5-1:pos 越界(n≥10000)时饱和为空串而非 panic;条号场景 extract_article 已前置拦截,
            // 这里只作 pub 函数的防御性兜底(绝不 panic)。
            out.push_str(units.get(pos).copied().unwrap_or(""));
        }
    }
    while out.ends_with('零') {
        out.pop();
    }
    // 「一十X」→「十X」(十位在最高位时,中文习惯省略「一」,如 10→十、15→十五)
    if let Some(rest) = out.strip_prefix("一十") {
        out = format!("十{}", rest);
    }
    out
}

/// 从法规全文 `content` 里提取条号 `ftnum`(阿拉伯字符串,如 `"585"`)的完整条文。
///
/// 右边界用**组合策略**:① 优先精确定位「第X+1条」(找的是具体下一条号,正文里引用其它
/// 条号不会误切,已对民法典含引用的 129 条验证);② 下一条号缺失/废止/本条是末条时,
/// 退而用 regex 找「下一个任意条文起始标记」兜底。条号非法或找不到该条 → `None`
///(调用方应降级到元典单条接口或容错检索,**不得编造**)。
///
/// ⚠️ 已知局限:真实法规全文存在条号缺失、异体写法,纯文本提取无法 100% 鲁棒
///(民法典实测 ~6/1260 条边界异常)。故工具层应配完整性自检 + 容错检索兜底,不可独用。
pub fn extract_article(content: &str, ftnum: &str) -> Option<String> {
    let n: u32 = ftnum.trim().parse().ok()?;
    // D5-1:条号超出中国法规合理范围(arabic_to_chinese 仅支持 1..=9999;且下方要算 n+1)→ 直接降级,
    // 不进全文提取(调用方降级到单条接口,绝不编造)。防 n≥9999 引发的越界 panic 炸掉整个 chat 回合。
    if n == 0 || n >= 9999 {
        return None;
    }
    let start_marker = format!("第{}条", arabic_to_chinese(n));
    let start = content.find(&start_marker)?;
    let body_start = start + start_marker.len();
    let rest = &content[body_start..];
    // ① 精确:下一条号「第X+1条」。② 兜底:下一个任意条文起始标记。
    let next_marker = format!("第{}条", arabic_to_chinese(n + 1));
    let end_rel = rest.find(&next_marker).or_else(|| next_article_start(rest));
    let end = body_start + end_rel.unwrap_or(rest.len());
    let article = content[start..end].trim();
    if article.is_empty() {
        None
    } else {
        Some(article.to_string())
    }
}

/// 在 `s` 里找「下一个条文起始标记」的字节位置:特征是「第[中文数字]条」后紧跟空白
///(全角空格 / 空格 / 换行)—— 据此区分真正的条文起头与正文中「依照第X条的规定」式句中引用
///(后者「条」后跟「的 / 规定」等非空白字符)。
fn next_article_start(s: &str) -> Option<usize> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"第[一二三四五六七八九十百千零]+条[\s\u{3000}]").unwrap()
    });
    re.find(s).map(|m| m.start())
}
