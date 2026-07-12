enterprise_aggregation_summary — ⭐⭐ 企业聚合摘要(一次拿所有维度统计 + Top 20 命中),企业类查询的核心入口

适用场景:
- 已经拿到企业 id 或 USCC,要看整体涉诉 / 经营风险概览(失信 / 普执 / 文书 / 股权 / 投资 / 担保 / 处罚 / 异常 / 欠税 / 法院公告等十几个维度)
- 立案前评估被告履行能力 + 财产线索摘要
- 用户问「这家公司涉诉多吗」「有没有失信记录」「资产状况怎么样」
- 给客户出风险报告的第一步,后续按需深查
- **对 90% 的企业查询需求,本工具已经够用**,不必再单独调失信 / 普执 / 股权 等细分接口

不适用:
- 没有 id / USCC → 先用 `enterprise_search` 拿候选
- 看完聚合发现某维度 >20 条需要全列 → 用对应专用工具(`enterprise_change_info` / `enterprise_writ_list` 等)
- 要十大股东 / 核心成员 / 分支机构详情 → 用 `enterprise_base_info`(聚合不带)
- 按年份对比资产 → 用 `enterprise_annual_report`

输入字段(`id` 或 `tyshxydm` **二选一必填**):
- id: 优先填,从 `enterprise_search` 拿
- tyshxydm: 统一信用代码,18 位字符,如「91320200MA1XXXXXX」

注意事项:
- 优先用本地缓存(企业类 30 天 TTL)
- 返回字段是 nested object,关键维度都有 `{count, items[]}` 结构,**items 最多 20 条 Top 摘要**
  - 维度包括(部分):executions(失信)/ executed_person(普执)/ writ_list(文书)/ change_info(变更)/ frozen_equity(股权冻结)/ out_invest(对外投资)/ pledge(出质)/ guaranty(担保)/ punishment(处罚)/ abnormal(异常)/ tax_arrears(欠税)/ court_notice(公告)
- **本工具 10 积分**；先看聚合轮廓，只有摘要不足时才追加 5/10 分明细接口
- LLM 拿到结果后做 **维度优先级判断**:失信 + 普执 + 文书 / 财产线索(股权冻结 / 对外投资)是首要;变更 / 担保等次要
- `<CITATIONS>` 标 `type: "enterprise"`,title 写公司名,source 写 USCC
- 拒执判断场景:聚合 + `enterprise_change_info`(按立案日 cutoff 过滤变更)+ `enterprise_annual_report`(立案前后资产对比)三件套
