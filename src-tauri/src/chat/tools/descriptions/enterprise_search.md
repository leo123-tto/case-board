enterprise_search — 按企业名称模糊匹配,拿候选企业列表(id + 统一信用代码)

适用场景:
- 用户问「无锡 X 科技有限公司的工商情况」,本工具是**所有企业类查询的入口**
- 立案前对被告做主体核查,拿到准确名称 + USCC 后再做后续深查
- 对方主体名字带「分公司 / 子公司」需要先确认主体身份
- 客户说不清完整名称,只给关键字「XX 科技」,本工具模糊匹配出候选

不适用:
- 已经拿到 id 或 USCC,**直接用 `enterprise_aggregation_summary` 等下游工具**,跳过本工具节省 1 次调用
- 自然人 → 元典 36 接口都是企业类,自然人没有专用接口(用 `search_cases_normal` 按姓名搜涉诉)
- 想看企业涉诉详情 → 用 `enterprise_aggregation_summary`(主入口)

输入字段:
- name: 必填,中文企业名(全称 / 简称 / 关键字皆可)
- top_k: 可选,默认 10,最大 20

注意事项:
- 优先用本地缓存(企业类 30 天 TTL,过期 stale 仍可返回但标 ⚠️)
- 返回字段:`[{id, name, tyshxydm(USCC), reg_status, legal_person, est_date}]`
- 命中后挑出唯一匹配那条,**记下 id 或 USCC**,后续企业类调用都用这个 id
- 同名公司很多时,看 `reg_status` 优先「存续」「在业」,排除「注销」「吊销」(除非用户专门要查已注销主体)
- `<CITATIONS>` 标 `type: "enterprise"`,title 写企业名,source 写 USCC(USCC 比 id 更稳定,不依赖元典)
- **本工具 1 积分**；聚合 summary 10 积分，所以先用本工具确认唯一主体最省
