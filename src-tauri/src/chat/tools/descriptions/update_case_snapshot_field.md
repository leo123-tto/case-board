修改案件画像(案件快照)中的单个字段。

使用场景:
- 用户说“案号不对,改成 XXX”
- 用户说“原告不是 A,应该是 B”
- 用户说“诉讼请求金额应该是 100000”
- 用户说“清空/删除某个字段”

本工具不会直接改 LLM 抽取的原始 agg_* 字段,而是把修改写入 `user_overrides_json`。
这与前端“编辑模式”手改共享同一套覆盖层,因此:
1. 用户通过 AI 助手纠正的值,与手动编辑效果完全一致;
2. 后续 LLM 重新全局抽取时,不会覆盖本次确认值;
3. 字段路径规范与前端 overlay 一致,支持顶层字段、当事人/法官列表项、子表行内字段。

可写路径示例:
- 顶层字段:`agg_case_no`、`agg_court`、`agg_cause`、`agg_filed_at`、`agg_claim_amount`、`agg_status_text`、`agg_resolution`、`agg_our_side`、`case_summary`、`case_stage`、`case_status`、`case_type`、`case_note`、`expected_close_at`
- 当事人/法官列表项:`agg_plaintiffs.0`、`agg_defendants.1`、`agg_third_parties.0`、`agg_judges.0`(数字为列表下标,从 0 开始)
- 子表行内字段:
  - `agg_party_contacts.{张三|原告}.phone`
  - `agg_court_contacts.{张法官|审判长}.phone`
  - `agg_key_dates.{开庭|2024-09-15}.note`
  - `agg_fees.{律师代理费|5000}.note`

参数规则:
- `value` 传空字符串表示“清空该字段”(会写 null);
- `reason` 必填(向用户说明为什么改),若用户没说你也要自己补一句,例如“根据用户口头纠正”;
- 如果用户一次性纠正多个字段,请依次调用本工具(每次一个字段),不要合并成一次调用;
- 修改完成后用自然语言向用户确认改了什么、改后值是多少。
