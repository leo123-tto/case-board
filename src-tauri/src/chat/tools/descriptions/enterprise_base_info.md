enterprise_base_info — 详细工商档案(股东出资比例 + 十大股东 + 核心成员 + 分支机构)

适用场景:
- 已经看了 `enterprise_aggregation_summary` 的总览,需要**深入主体信息**(聚合不带这些)
- 用户问「这家公司股东出资多少」「实际控制人是谁」「有几家分公司」
- 立案前确认被告主体身份 + 法定代表人 + 注册资本
- 拒执案件:看股东出资有没有抽逃可能 + 实控人在哪
- 上市公司:`top10holder` / `top10circulate` 拿前十大股东(普通公司没这两字段)

不适用:
- 只想要简单工商信息(名称 / 法人 / 状态 / 注册资本) → `enterprise_aggregation_summary` 已含
- 看变更历史 → `enterprise_change_info`(本工具是当前快照)
- 看年报对比 → `enterprise_annual_report`(按年份)

输入字段(`id` 或 `tyshxydm` 二选一):
- id: 优先,从 `enterprise_search` 拿
- tyshxydm: USCC 18 位

注意事项:
- 优先用本地缓存(30 天 TTL)
- 返回字段:`{basic, partner, top10holder, top10circulate, members, branches}`
  - **basic**:工商基本信息(注册资本 + 经营范围 + 法人 + 注册地等)
  - **partner**:股东出资清单 `[{name, invest_amount, invest_ratio, invest_type}]` — **拒执核心:有没有股东实缴?**
  - **top10holder / top10circulate**:仅上市公司有,非上市公司为空
  - **members**:核心成员 `[{name, position, type}]` — 看监事 / 高管 / 实控人
  - **branches**:分支机构 `[{name, region, status}]` — 看其他主体线索
- **本工具 10 积分**
- LLM 引用具体出资 / 股东 / 成员时,**必须**完整复述名字 + 数字,不要四舍五入(法律精度)
- `<CITATIONS>` 标 `type: "enterprise"`,title 写企业名 + 「(主体档案)」
