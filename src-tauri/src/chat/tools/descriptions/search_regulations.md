search_regulations — 检索法规(整部),返回法规元信息列表(法规名 + 发布部门 + 实施日期),不返回正文

适用场景:
- 用户问「跟 X 相关的有哪些法规」「江苏省关于 Y 的地方性法规」
- 想看一部法规的存在性与基本信息(发布部门 / 实施日期 / 是否还有效),决定要不要拿全文
- 起草前先看相关法规的整体清单,挑出最权威的 1-2 部精读
- 用户给的法规名字不太确切,先模糊搜确认完整名称

不适用:
- 想要某条法律的具体条文 → 用 `search_laws`(法条粒度)或 `get_law_article`(已知条号)
- 已经知道法规名,要拿整部全文 → 直接用 `get_regulation_detail`,省一次调用
- 找案例 → `search_cases_normal` / `search_cases_authority`

输入字段(**至少填 keyword 或 fgmc 之一**,纯过滤无关键词时元典容易返回过宽):
- keyword: 可选,中文关键词,搜法规标题或内容片段
- fgmc: 可选,法规名模糊匹配
- effect_level: 可选,枚举「宪法 / 法律 / 行政法规 / 地方性法规 / 司法解释」
- region: 可选,地方法规过滤(省级名)
- valid_only: 可选,布尔,默认 true(只返回现行有效法规)
- publish_date_start / publish_date_end: 可选,YYYY-MM-DD,发布日期范围
- implement_date_start / implement_date_end: 可选,YYYY-MM-DD,实施日期范围
- top_k: 可选,默认 20

注意事项:
- 优先用本地缓存(命中 0 积分；miss 调法规关键词检索为 10 积分)
- 返回字段:`[{id, fgmc, effect_level, publish_date, implement_date, valid, region, content?}]`
  - `content` 仅在 keyword 命中正文时返回高亮片段
- 看到列表后通常下一步是 `get_regulation_detail` 拿挑中的那部法规全文
- `<CITATIONS>` 标 `type: "law"`,title 写法规名(无条号)
