enterprise_annual_report — 企业年报(按年份),拒执时立案前 vs. 当年资产 / 股东出资对比

适用场景:
- 拒执判断硬需求:对比「立案前一年」 vs.「立案当年」的年报,看资产 / 股东出资有无异常变化(如:总资产减半 / 股东实缴减少 / 重大资产处置)
- 用户问「这家公司去年营收多少」「股东实缴到位了吗」
- 评估企业履行能力的最权威依据(年报是企业法定披露,法律责任在)
- 看股东出资历史(配 `enterprise_base_info.partner` 看当前快照,本工具看历史填报)

不适用:
- 只想看当前快照 → `enterprise_base_info`
- 看涉诉 → `enterprise_aggregation_summary` / `enterprise_writ_list`
- 拒执场景的变更证据 → 配合 `enterprise_change_info`

输入字段(`id` 或 `tyshxydm` 二选一,**year 必填**):
- id 或 tyshxydm
- year: 必填,纯数字(如 `2024`),企业年报对应自然年(每年 6 月底前报上一年度)

注意事项:
- 优先用本地缓存(年报落定后不再变,30 天 TTL 实际命中率很高)
- 返回字段:`{total_assets, total_liab, total_equity, revenue, profit, paid_in_capital, employee_count, ...}`
- **拒执核心查法**:LLM 拿到立案日后,自动调本工具拿 立案前年 + 当年 两份年报,对比 4 个数字:
  1. 总资产变化(资产突减 = 警示)
  2. 实缴出资变化(实缴下调 = 抽逃出资警示)
  3. 主营业务收入(业务持续性)
  4. 重大投资 / 处置披露
- **本工具 10 积分** × 年份数(查 2 年 = 20 积分)，只查确有必要的年度
- 如果某年没年报 / 还没披露,返回空 — LLM 应该提示用户「该年度年报暂未披露」
- `<CITATIONS>` 标 `type: "enterprise"`,title 写「<企业名> · <year> 年报」
