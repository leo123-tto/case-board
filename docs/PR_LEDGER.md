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

## Open(等 review)— 4 个

| # | 标题 | 提交日 | 分支 | Base | 状态 |
|---|---|---|---|---|---|
| [#35](https://github.com/leo123-tto/case-board/pull/35) | fix(chat): VisualizeCase 三连环 bug — 反伪造 + 反叙述 + 思考模型 idle 缩放 | 2026-07-14 | `pr/fix/visualize-fake-user-confirm` | v0.4.9 | 🟢 open, mergeable, ⚠️ CI 未跑(fork PR 触发问题) |
| [#33](https://github.com/leo123-tto/case-board/pull/33) | fix(home-companion): 修复首页看板助手天气 + LLM 中文输出 | 2026-07-13 | `pr/fix/home-companion-v0.4.9` | v0.4.9 | 🟢 open |
| [#32](https://github.com/leo123-tto/case-board/pull/32) | fix(wechat-qr): 修复设置页微信加群二维码 hover 闪屏 | 2026-07-13 | `pr/fix/wechat-qr-hover-flash` | v0.4.9 | 🟢 open |
| [#23](https://github.com/leo123-tto/case-board/pull/23) | Feature/element trial assist | 2026-06-22 | `feature/element-trial-assist` | (旧) | 🟢 open(需跟进) |

---

## Closed(关闭)— 28 个

> 按时间倒序

| # | 标题 | 关闭日 | 备注 |
|---|---|---|---|
| #31 | feat(caseboard): 接入 Kimi Coding Plan 兼容后端 | 2026-07-12 | ❌ 关闭 |
| #30 | feat(caseboard): 增强类案、案例库检索与可控性 | 2026-07-12 | ❌ 关闭 |
| #29 | fix: avoid defendant phone fallback in court filing | 2026-06-24 | ❌ 关闭 |
| #28 | [codex] Add contract context files and court SMS local download | 2026-06-23 | ❌ 关闭(已被维护者功能覆盖,见 v0.3.29) |
| #27 | fix(chat): Windows 独立窗口全屏覆盖,紧急绕过 WebView2 | 2026-06-23 | ❌ 关闭(已用 0.3.27 替代,见 CHANGELOG) |
| #26 | fix: prioritize court region in similar case search | 2026-06-23 | ❌ 关闭(已用 0.3.27 替代) |
| #25 | fix(chat): 重写 DetachedChatWindow 绕过 CaseChatPanel 渲染 | 2026-06-23 | ❌ 关闭(被替代) |
| #24 | fix(chat): 独立窗口不再卡死 + WebView2 layout thrashing 修复 | 2026-06-23 | ❌ 关闭(被替代) |
| #22 | feat: daily reminder webhook | 2026-06-22 | ❌ 关闭 |
| #21 | [codex] feat(element): 要求式要素抽取 | 2026-06-22 | ❌ 关闭(已合并同类功能) |
| #20 | fix(cases): delete_case 补全 17 张 FK 子表 | 2026-06-22 | ❌ 关闭(已用 FK 修复) |
| #19 | fix: open Office documents with system app | 2026-06-21 | ❌ 关闭(已合并) |
| #18 | feat: improve case chat window and markdown rendering | 2026-06-21 | ❌ 关闭 |
| #17 | fix(lawyer-profile): is_default 改为 boolean,修复 save_lawyer_profile | 2026-06-17 | ❌ 关闭(已合并修复) |
| #16 | fix: 修复 MiMo 与 OpenAI 兼容 API 的 404 错误 | 2026-06-17 | ❌ 关闭(已合并) |
| #15 | Fix OpenAI-compatible model settings | 2026-06-17 | ❌ 关闭 |
| #14 | fix(llm): global_extract 多字段 content fallback + dlog 写文件 | 2026-06-17 | ❌ 关闭(已合并) |
| #13 | fix(db): 反向补全 fork 漏提交的 migration 0029/0030 | 2026-06-17 | ❌ 关闭(已合并) |
| #12 | fix(ui): 导入案件 toast 后端标签按 llm_model 判定 | 2026-06-16 | ❌ 关闭(已合并) |
| #10 | fix(docx): 跨平台 .docx 文本提取,摆脱 macOS-only textutil | 2026-06-16 | ❌ 关闭(已合并) |
| #9 | feat: 工具箱架构 + 移动 LLM 提供商扩展 | 2026-06-15 | ❌ 关闭 |
| #8 | feat: 开庭一键录入,智能推荐 | 2026-06-15 | ❌ 关闭 |
| #7 | fix(kb): KB 子目录切换不再自动重新扫描,保留旧 rel_path | 2026-06-15 | ❌ 关闭 |
| #6 | fix(kb): collect_corpus fallback 扫根目录,支持用户自定义根 | 2026-06-15 | ❌ 关闭 |
| #5 | feat(minimax): 模型选择器换成 select 组件,把 M2.7-highspeed 下位 | 2026-06-15 | ❌ 关闭 |
| #4 | fix(kb): save_case_experience 漏 tilde 展开,修 Windows KB 路径 | 2026-06-15 | ❌ 关闭 |
| #3 | fix(import): 校验按 cloud_llm_backend 路由 (MiniMax/DeepSeek) | 2026-06-15 | ❌ 关闭 |
| #1 | 优化首页加载体验、视图与案件列表流畅度 | 2026-06-13 | ❌ 关闭 |

---

## Merged(已合并)— 0 个

> ⚠️ 截至 2026-07-15,**所有 PR 都是 closed 状态,无 merged**。
> 这意味着之前的工作要么被维护者用其他方式实现(等价的 v0.3.X release commit),
> 要么维护者直接跳过了 PR。
>
> 建议复盘:为啥 merged 率这么低?看 [复盘](#复盘) 段。

---

## 复盘(2026-07-15)

**事实**:
- 32 个 PR,0 个 merged,28 个 closed,4 个 open
- 维护者节奏极快(每天 1-2 个 release),常常 PR 没合就已经有等价 fix
- 维护者有时直接"另开一版"绕开 PR

**模式**:
1. 同一类 BUG 提了多次 PR(例:Windows 独立窗口 #24 #25 #27)
2. 维护者 fix 速度经常比 PR review 快
3. PR 经常是"补丁"被维护者重写成"正式版"

**改进方向**:
- 减少同类 PR 重复,先确认维护者是否在做
- **PR 前先开 issue** 确认方向,避免做无用功
- 大改动(>500 行)慎提,小修小补提了也是白做
- **多在 issue 评论** 跟维护者对齐,别一个人闷头做

---

## 统计

| 指标 | 值 |
|---|---|
| 总 PR 数(累计) | 32 |
| Open | 4 |
| Merged | 0 |
| Closed | 28 |
| 合并率 | 0% |
| 平均 review 时间 | N/A(无 merge) |

> 每周跑 `python scripts/contrib/list-my-prs.py --stats` 更新

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
3. 复盘:为啥被关?改进工作流

**每周一**:
- 跑 `python scripts/contrib/list-my-prs.py` 看所有 PR 总览
- 检查每个 open PR 是否有 reviewer 反馈
- 更新本文档
