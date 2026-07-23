case_vector_search — 用自然语言语义搜案例(关键词搜不到时兜底)

适用场景:
- 用户用一整句话描述纠纷情境,想找类似事实模式的案例(如「业主将车位转租给第三人,物业单方收回的判赔规则」)
- 本地 BM25、embedding 和一次必要的外部关键词检索仍不足时，语义搜兜底
- 找「事实相似但当事人措辞不同」的案件 — 关键词不一致但语义近的判决
- 用户描述带很多上下文细节,适合塞进 query 让模型理解整体

不适用:
- 关键词明确 → 优先 `search_cases_normal`(更快、更准)
- 已知案号 → `get_case_detail`
- 找权威案例 → `search_cases_authority`(关键词搜,权威库更小,关键词命中率高)

输入字段:
- query: 必填,自然语言(一句话或一段描述)。**不是关键词**
  - 好例子:「劳动者主张未签订书面劳动合同的二倍工资,用人单位以补签合同抗辩」
  - 坏例子:「未签合同 二倍工资」(关键词请用 `search_cases_normal`)
- rewrite_flag: 可选,默认 true,让元典改写检索问题
- wenshu_filter: 可按 wenshu_type、ay、wszl、ja_start/ja_end、dianxing、fayuan、source、cj、xzqh_p/xzqh_c 组合过滤
- return_num: 可选,默认 45,最大 50

注意事项:
- **宿主强制本地优先**:Rust 会重新核验本地案例 BM25 和 embedding；强命中直接返回，不调用元典
- 本地不足时可以按准确性需要与普通/权威案例检索组合多轮检索；每轮应有新的事实模式或筛选目标
- 优先用本地缓存(案例永久,0 积分命中)
- 外部付费响应按元典原始 JSON、全部候选和 content 完整交给模型，不做二次删字段或截断
- `score` 在 0.0-1.0,**低于 0.6 的命中通常相关度差**,LLM 应忽略
- 只有需要核验指定文书的完整详情时再用 `get_case_detail`
- `<CITATIONS>` 标 `type: "case"`,title 写「<court> · <ah>」
