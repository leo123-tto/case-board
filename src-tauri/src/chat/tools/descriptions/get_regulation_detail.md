get_regulation_detail — 拿一部法规的整部全文(目录 + 各章节条款)

适用场景:
- 起诉前要把某部法规过一遍,挑出可能用得上的条款集合
- 用户问「整部 X 法的核心制度是什么」「Y 法分几章,主要内容是什么」
- 想对一部法规做系统性整理,而不是只看零散几条
- 校验对方书状中的引用是否落在该法规的合理范围内

不适用:
- 只想看一条 → 用 `get_law_article`(单条粒度更省)
- 不知道法规名 / 元典 id → 先用 `search_regulations` 拿到候选,再用本工具
- 想看法条命中列表(粒度小)→ 用 `search_laws`

输入字段(`id` 跟 `fgmc` **二选一必填**,优先 id):
- id: 优先填,元典法规 ID,从 `search_regulations` 结果里拿
- fgmc: 法规名(精确,**全称**,如「中华人民共和国民法典」)
- refer_date: 可选,YYYY-MM-DD,定位时点版本(适用于修订过多次的法规)

注意事项:
- 优先用本地缓存(命中 0 积分；miss 调整部法规详情为 5 积分，并自动收口主库)
- 返回字段:`{id, fgmc, content, effect_level, publish_date, implement_date, valid, region, issuer}`
  - `content` 是整部法规全文,可能几千到上万字 — agent_loop 会自动落盘成 KB 文件,LLM 只看摘要 + 关键章节
- 长法规(如民法典 1260 条)单次调用积分仍是 1,**不要在 LLM 里反复调同部法规**,本地缓存 + KB 写盘后下次直接复用
- `<CITATIONS>` 标 `type: "law"`,title 写法规全名(无具体条号,因为引用的是整部)
