# Commit 规范

> CaseBoard 沿用 [Conventional Commits 1.0.0](https://www.conventionalcommits.org/)。
> upstream CONTRIBUTING.md 已有总览,本文件讲实操细节。

---

## 1. 格式

```
<type>(<scope>): <subject>

[optional body]

[optional footer]
```

### Type 速查

| type | 用途 | 例子 |
|---|---|---|
| `feat` | 新功能 | `feat(scanner): 识别 AI 产物文件` |
| `fix` | BUG 修复 | `fix(import): 处理 macOS 文件夹访问权限拒绝` |
| `docs` | 文档 | `docs: 补 V0.2 MCP server 设计说明` |
| `style` | 格式调整(无逻辑变化) | `style: prettier 重排 HomeView` |
| `refactor` | 重构(无新功能/无 BUG 修复) | `refactor(db): 拆 case_visual 表` |
| `perf` | 性能 | `perf(llm): 缓存法律检索结果` |
| `test` | 测试 | `test(chat): 加 VisualizeCase 阶段一回归测试` |
| `chore` | 构建/依赖/工具链 | `chore: 升级 tauri 到 2.5` |
| `ci` | CI 配置 | `ci: 增加 windows-arm64 构建` |

### Scope 速查(常用)

| scope | 对应模块 |
|---|---|
| `chat` | `src-tauri/src/chat/` |
| `db` | `src-tauri/src/db/` |
| `ingest` | `src-tauri/src/ingest/` |
| `llm` | `src-tauri/src/llm/` |
| `feedback` | `src-tauri/src/feedback/` |
| `ui` | `src/components/`, `src/modules/` |
| `litigation` | `src/modules/litigation/` |
| `tools` | `src/modules/tools/` |
| `home-companion` | `src-tauri/src/home_companion.rs` |
| `wechat-qr` | 设置页微信加群 |
| `windows` / `macos` | 平台特定修复 |
| `release` | 发版相关 |

## 2. Subject 怎么写

- ✅ **祈使句**:"修复 X" 而不是 "修复了 X" 或 "已修复 X"
- ✅ **中文** 为主(项目以中文沟通),专有名词用英文
- ✅ **首字母不大写**(英文 subject)
- ✅ **末尾不加句号**
- ✅ **长度 ≤ 72 字符**
- ✅ **一句话说清楚做了什么**

| ✅ 好 | ❌ 差 |
|---|---|
| `fix(chat): 拦截 LLM 伪造用户授权` | `fix: 改个 bug` |
| `feat(scanner): 识别 AI 产物文件(总览/调查/精要)` | `feat: 改了点东西` |
| `docs: 补 V0.2 MCP server 设计说明` | `update docs` |
| `chore: 升级 tauri 到 2.5` | `chore: deps` |

## 3. Body 怎么写

可选,但**复杂 PR 必须写**。Body 回答:
- 改了什么(细节)
- 为什么这样改(动机)
- 跟之前 commit 的关系(如果是连环 fix)

```
fix(chat): 阶段二禁止 LLM 在正文复述 CaseGraph 结构

接 PR #34 bug 1 修完后,LLM 乖乖进入阶段二(收到多选结果),
但在调 save_case_visualization 之前,把 4 张视图的 19 个 event
节点 + 18 条 precedes 边 全部复述一遍。

结果 MiniMax-M3 输出上限 32K,CaseGraph 节点 50+ 时正文就把
token 吃光,工具还没调就被 finish_reason=length 截断。

修法:三处禁止正文复述结构(task_user_prompt / save_case_
visualization.md 描述 / task_contract success_criteria)。
```

## 4. Footer 怎么写

**引用 issue / PR**:
```
fix(chat): 拦截 LLM 伪造用户授权

Closes #34
Refs leo123-tto/case-board#33
```

**Breaking change**:
```
feat(api): 改 save_artifact 返回结构

BREAKING CHANGE: save_artifact 现在返回 { id, version } 而不是 { id }
迁移指南见 docs/migration-v0.5.md
```

**Co-authored-by**(多作者):
```
feat(scanner): 识别 AI 产物文件

Co-authored-by: Mavis <noreply@minimax.io>
```

## 5. 实操命令

### 5.1 普通 commit

```bash
git add src-tauri/src/chat/quality_gate.rs
git commit -m "fix(chat): 拦截 LLM 伪造用户授权"
```

### 5.2 改最近一次 commit

```bash
# 改 message
git commit --amend

# 改 message + 加文件
git add <forgotten-file>
git commit --amend --no-edit
```

### 5.3 拆 commit(连续改了好几个,想拆成独立)

```bash
# 看最近 3 个 commit
git rebase -i HEAD~3

# 把想拆的那个 commit 改成 'edit'
# 1. git reset HEAD~  拆出来
# 2. git add <part-1>  分批 add
# 3. git commit -m "..."
# 4. git add <part-2>
# 5. git commit -m "..."
# 6. git rebase --continue
```

### 5.4 fixup(在 PR 反馈后追加)

```bash
# 在 PR 分支上,改了文件后:
git add <files>
git commit --fixup=HEAD    # 或 --fixup=<原 commit sha>

# 推送前合并 fixup(让 PR 历史干净)
git rebase -i --autosquash HEAD~3
git push --force-with-lease
```

## 6. 反模式(不要做)

| 反模式 | 后果 |
|---|---|
| `fix: 改了一些东西` | 看不出改了什么,reviewer 痛苦 |
| 一次性 commit 5 个不相关改动 | 出问题难回滚,reviewer 难懂 |
| 用 emoji 开头的 commit | 不规范,upstream 不接受 |
| `update` / `misc` / `temp` 等含糊 type | 看不出是 feat / fix |
| 在 commit 里塞调试代码 | 上游会要求删除 |
| commit 信息用英文但项目用中文沟通 | 上下文不一致 |

## 7. 隐私铁律(再强调)

commit message 也不能写真实案件信息:

- ❌ `fix(chat): 修复 XX 律师事务所张三案件可视化`
- ✅ `fix(chat): 修复案件可视化阶段二 token 爆炸`

见 [`docs/PRIVACY_IRON_RULE.md`](./PRIVACY_IRON_RULE.md) 完整版。
