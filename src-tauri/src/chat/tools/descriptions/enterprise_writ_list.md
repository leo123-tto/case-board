enterprise_writ_list — 涉诉文书完整列表(案号 / 案由 / 法院 / 日期),配合 `get_case_detail` 拿全案号

适用场景:
- 已经从聚合看到企业涉诉数量大(如 50+ 案件),需要**完整案号列表**做下一步深查
- 想看这家公司过往涉诉的整体画像(原告 / 被告分布 / 案由集中度 / 法院层级)
- 立案前看对方应诉经验:涉诉多 = 老练,涉诉少 = 可能不专业
- 拒执场景:看相关执行案号,后续配 `get_case_detail` 拿执行细节

不适用:
- 涉诉量小(<20)→ `enterprise_aggregation_summary` 的 writ_list 维度 Top 20 已够
- 单独看失信 / 普执 / 公告 → 聚合 Top 20 已涵盖(本工具是普通涉诉文书列表,跟失信普执是不同维度)
- 已知具体案号要全文 → 直接 `get_case_detail`

输入字段(`id` 或 `tyshxydm` 二选一):
- id: 优先
- tyshxydm: USCC
- page: 可选,默认 1,每页 20

注意事项:
- 优先用本地缓存(30 天 TTL)
- 返回字段:`[{ah(案号), title, court, case_type, judge_date, party_role}]`
  - **party_role**:本企业在该案中是「原告 / 被告 / 第三人」
- 拿到案号后,**精选 3-5 个** 最相关的(优先看 case_type = 「执行」「破产」+ 近 2 年),用 `get_case_detail` 拿全文
- **本工具 10 积分** × 翻页次数；`get_case_detail` 每次 5 积分，先看摘要再决定取全文
- LLM **不要把 50 个案号全列给用户**,挑最有代表性 / 最新的报告;海量列表只在用户明确要求「列出所有案件」时全展开
- `<CITATIONS>` 标 `type: "enterprise"`,title 写企业名 + 「(涉诉文书)」
