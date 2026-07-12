enterprise_change_info — 完整工商变更记录(拒执判断 cutoff 必需,聚合 Top 20 不够时翻全量用)

适用场景:
- 拒执判断:看立案日之后,被告有没有突击转让股权 / 减资 / 法人变更等规避执行行为
- 用户问「这家公司股权变更频繁吗」「最近半年法人是不是变了」
- `enterprise_aggregation_summary` 里的 change_info 维度只有 Top 20,想看 20 条之外的历史细节
- 给客户做履行能力评估时,看主体稳定性

不适用:
- 只想看最近几次变更概览 → 聚合 Top 20 已够
- 看当前主体快照 → 用 `enterprise_base_info`(本工具是历史变更日志)
- 看年度财务变化 → 用 `enterprise_annual_report`

输入字段(`id` 或 `tyshxydm` 二选一):
- id: 优先
- tyshxydm: USCC
- page: 可选,默认 1。每页 20 条;>20 条需要翻页查询

注意事项:
- 优先用本地缓存(30 天 TTL,变更记录新增频率低,缓存命中率高)
- 返回字段:`[{change_item, before, after, change_date}]`,按 `change_date desc` 排列
- **拒执场景**:LLM 应该把立案日作为 cutoff,自动筛立案日之后的变更,重点报告:
  - 股东变更(转让 / 减资 / 退伙)
  - 法定代表人变更
  - 注册资本减少
  - 主营业务范围缩减
- **本工具 5 积分** × 翻页次数；默认只取必要页
- 大公司变更 100+ 条很常见,LLM 翻页前先看聚合 count,**count 超 50 才翻**;否则聚合 Top 20 通常够
- `<CITATIONS>` 标 `type: "enterprise"`,title 写企业名 + 「(变更记录)」
