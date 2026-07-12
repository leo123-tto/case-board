search_cases_normal — 普通裁判文书库关键词检索(覆盖范围最广,中级 / 基层法院判决多)

适用场景:
- 用户问「跟 X 类似的案件法院怎么判」「Y 类纠纷的判赔标准」
- 起草前找类案,看法院的常见裁判思路 + 判赔区间
- 想看大量同类判决的整体倾向(基层 / 中院的实务做法,而非最高法的指导口径)
- 找对方曾经做过类似行为的判决(给对方画像)

不适用:
- 想找最高法指导案例 / 公报案例 / 典型案例 → 用 `search_cases_authority`(权威库)
- 已知案号要拿全文 → `get_case_detail`
- 关键词不准、语义模糊 → 改用 `case_vector_search`
- 想搜法律条文 → `search_laws`

输入字段:
- qw: 必填,中文关键词(检索全文)。**支持「+」组合关键词**,如「合同解除+违约金」表示两词都出现的案例
- top_k: 可选,默认 20,最大 50

**高级过滤未启用**(V0.2 D2-D3 当前版本):
计划字段 court / cause / region / judge_date_range / case_no 等过滤参数,本次工具层暂不暴露,LLM 拿到全量结果后自己看 court / cause 字段筛选;若发现这是高频需求,后续版本 yuandian/mod.rs 会扩 Params struct 再开放。

注意事项:
- **先查作者整理过的全库**:找类案可先用 `search_local_kb` 看作者整理过的判例 / 类案笔记(0 积分),本地没有再调本工具外查
- 优先用本地缓存(案例不过期,命中即返回 0 积分；miss 调普通案例检索为 10 积分)
- 返回字段每条带 `{id, ah(案号), title, court, cause(案由), judge_date, content(摘要), score}`
- 命中后挑最相关 1-2 条用 `get_case_detail` 拿全文(全文里有完整裁判要旨 / 当事人 / 事实)
- `<CITATIONS>` 标 `type: "case"`,title 写「<court> · <ah>」,source 写元典 id
- 一次性命中通常 20 条,LLM **不要无脑列举所有**,精选 3-5 条最相关的告诉用户
