search_cases_authority — 权威案例库检索(最高法指导案例 + 公报案例 + 典型案例 + 参考案例)

适用场景:
- 想看最权威的裁判口径(最高法 / 公报案例对下级法院有事实上的指引力)
- 给法官 / 对方律师论证时,引用权威案例比普通案例更有说服力
- 用户问「这类纠纷最高院怎么定调」「公报案例对 X 的态度」
- 用 `search_cases_normal` 拿到一堆普通案例后,想再看有没有权威级别的对应案例
- 给客户做法律风险评估,要引用「行业标杆」级判决

不适用:
- 找普通案件 / 同类基层判决 → 用 `search_cases_normal`(覆盖面更广)
- 已知案号拿全文 → `get_case_detail` 传 `type="qwal"`
- 关键词不准 → `case_vector_search`

输入字段:
- ah / title / ay / jbdw / source: 可按案号、标题、案由、法院、权威案例来源过滤
- xzqh_p / wszl / ajlb / ja_start / ja_end: 可限制地域、文书种类、案件类别、结案日期
- qw / search_mode: 全文关键词及 `and` / `or` 组合方式
- top_k: 可选,默认 20,最大 50；所有字段均透传到元典

注意事项:
- **宿主强制本地优先**:Rust 会先查本地案号/案例 BM25，词法不足再查本地 embedding；强命中直接返回，不调用元典
- 本地不足时可以按准确性需要与普通案例、案例向量检索组合多轮检索；避免完全相同条件的无意义重复
- 优先用本地缓存(权威案例不过期,0 积分命中)
- 外部付费响应按元典原始 JSON 完整交给模型；注意辨认 source/case_type，并在需要指定文书详情时调用 `get_case_detail`
- `<CITATIONS>` 标 `type: "case"`,title 写「<court> · <ah>(<case_type>)」
