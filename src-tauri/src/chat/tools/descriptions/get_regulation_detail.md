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
- 优先用本地缓存(命中 0 积分；miss 调整部法规详情为 5 积分)。普通请求只把现行有效全文作为当前依据；失效、废止或尚未生效详情会被拒绝并自动补检现行替代法源。明确 `refer_date` 时可保留历史全文到 raw/cache，并带 `historical_research_only` 警告
- 返回字段:`{id, fgmc, content, effect_level, publish_date, implement_date, valid, region, issuer}`
  - `content` 是整部法规全文,可能几千到上万字；现行全文或明确历史时点全文通过对应时效策略后完整交给模型并落盘，不做预览裁剪
- 本接口当前为 **5 积分/次**；不要反复调同部法规，本地缓存 + KB 写盘后下次直接复用
- 工具返回 `inactive_source_rejected` 时，只能使用 `replacement_search` 中真实取得的现行法源；明确历史时点返回 `historical_research_only` 时，只能按历史版本引用，不得称为现行有效
- `<CITATIONS>` 标 `type: "law"`,title 写法规全名(无具体条号,因为引用的是整部)
