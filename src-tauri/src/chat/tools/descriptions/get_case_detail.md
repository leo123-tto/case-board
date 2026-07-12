get_case_detail — 拿单个案例的完整裁判文书(裁判要旨 / 当事人 / 事实 / 适用法律 / 判决主文)

适用场景:
- `search_cases_normal` / `search_cases_authority` 命中后,挑出最相关 1-2 条拿全文
- 用户直接给出案号(如「(2021)苏02民终123号」),想看完整判决
- 校验对方书状里引用的案例真实存在 + 案号正确
- 起草前精读类案的「本院认为」段,提炼裁判逻辑写进自己的论述

不适用:
- 不知道案号 → 先用搜索类 tool 拿候选
- 想看一批同类案件的概况 → 用 `search_cases_normal`(列表粒度)
- 想找法条 → `get_law_article`

输入字段(**`type` + `case_no` 都必填**):
- type: 必填,枚举值
  - `"ptal"` = 普通案例库(配 `search_cases_normal` 命中的结果)
  - `"qwal"` = 权威案例库(配 `search_cases_authority` 命中的结果)
  - 不知道走哪个 → 回看检索结果来自普通库还是权威库，不要盲猜
- case_no: 必填,完整案号(中文),如「(2021)苏02民终123号」。**`ah` 字段同义**,工具内部统一

注意事项:
- 优先用本地缓存(案例永久缓存,0 积分命中)
- 返回字段:`{id, ah, title, court, judge_date, content(全文), party_info, judgment_summary}`
- 命中后 agent_loop 会自动把全文落盘到 KB,下次同案号查询 0 积分
- 长判决文书 (几万字) 单次调用积分仍是 5；默认最多精读 3 个远端案例
- `<CITATIONS>` 标 `type: "case"`,title 写「<court> · <ah>」,source 写元典 id
- 当前直接调用官方 `rh_case_details` GET 详情接口，不再用关键词搜索冒充详情
