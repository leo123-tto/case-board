# PR 跟踪表

> 维护:zzf516988659-del(对 leo123-tto/case-board 的所有 PR)
> 更新:每次 PR 创建 / 反馈 / 合并后更新
> 工具:`python scripts/contrib/list-my-prs.py` 拉取最新

---

## 状态图例

| 状态 | 含义 |
|---|---|
| 🟡 draft | 草稿,等自己补充 |
| 🟢 open | 已提交,等 reviewer |
| 🔵 review | reviewer 在看 |
| 🟠 changes-requested | reviewer 要求修改 |
| ✅ merged | 已合并 |
| ❌ closed | 已关闭(放弃) |

---

## Open(等 review)

| # | 标题 | 提交日 | 分支 | Base | CI | 评论 | URL |
|---|---|---|---|---|---|---|---|
| #35 | fix(chat): VisualizeCase 三连环 bug — 反伪造 + 反叙述 + 思考模型 idle 缩放 | 2026-07-14 | `pr/fix/visualize-fake-user-confirm` | v0.4.9 | ⚠️ 未跑 | 0 | https://github.com/leo123-tto/case-board/pull/35 |

---

## Merged(已合并)

| # | 标题 | 合并日 | 致谢/备注 |
|---|---|---|---|
| — | (待 list-my-prs.py 拉取) | — | — |

---

## Closed(关闭 / 放弃)

| # | 标题 | 关闭日 | 原因 |
|---|---|---|---|
| — | — | — | — |

---

## 历史(全量,按时间倒序)

> 完整列表由 `python scripts/contrib/list-my-prs.py` 生成。
> 这里只列**已合并 + Open** 的活跃 PR。

| # | 标题 | 类型 | 状态 | 合并日 | URL |
|---|---|---|---|---|---|
| #35 | VisualizeCase 三连环 bug | fix | 🟢 open | — | [#35](https://github.com/leo123-tto/case-board/pull/35) |

---

## 操作 SOP

**提 PR 后**:
1. 跑 `python scripts/contrib/check-pr-status.py <PR号>` 确认 CI 状态
2. 在本文档"Open"表格加一行

**PR 合并后**:
1. 跑 `git checkout main && git merge --ff-only upstream/main`
2. 跑 `git branch -d pr/fix/xxx && git push origin --delete pr/fix/xxx`
3. 把 PR 从"Open"挪到"Merged"

**PR 被关闭**:
1. 跑 `git branch -D pr/fix/xxx`(没合并可以直接强删)
2. 把 PR 挪到"Closed",写明原因

**每周一**:
- 跑 `python scripts/contrib/list-my-prs.py` 看所有 PR 总览
- 检查每个 open PR 是否有 reviewer 反馈
- 更新本文档

---

## 统计

| 指标 | 值 |
|---|---|
| 总 PR 数(累计) | 1+ |
| Open | 1 |
| Merged | 0 |
| Closed | 0 |
| 合并率 | — |
| 平均 review 时间 | — |

> 统计每周自动更新,见 `scripts/contrib/list-my-prs.py --stats`
