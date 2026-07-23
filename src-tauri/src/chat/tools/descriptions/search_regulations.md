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
- search_mode: 可选,`AND` / `OR`,控制关键词分词组合方式
- xljb_1 / sxx / dy / fbbm: 可选,分别过滤效力级别、时效性、地域、发布部门
- fbrq_start / fbrq_end、ssrq_start / ssrq_end: 可选,YYYY-MM-DD,发布日期或实施日期范围
- top_k: 可选,默认 20

注意事项:
- Rust 宿主会先查法规全文文件名目录、BM25 和适用的本地向量索引；本地强命中时直接返回，不调用元典
- 本地不足时可按准确性需要与 `search_laws`、`law_vector_search` 组合多轮检索；避免用完全相同的条件重复调用
- 优先用本地缓存(命中 0 积分；miss 调法规关键词检索为 10 积分)
- 外部付费响应保留可用候选和字段，不做预览截断。普通请求先剔除失效、废止和尚未生效候选；用户明确选择非现行 `sxx` 时则带 `historical_research_only` 保留，引用必须标明版本、适用时点和非现行状态
- 看到列表后通常下一步是 `get_regulation_detail` 拿挑中的那部法规全文
- `<CITATIONS>` 标 `type: "law"`,title 写法规名(无条号)
