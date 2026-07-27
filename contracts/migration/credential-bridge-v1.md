# CaseBoard 0.4 → 0.5 Credential Bridge 只读交接契约 v1

状态：冻结，等待 Claude 整段复审  
冻结基线：`0cea8da55f39956150844ae33fe6a4c3d48d2c13`  
合同标识：`caseboard-credential-handoff/v1`

## 1. 适用范围与权威边界

本文件是最终 0.4 credential bridge 向 0.5 Phase 1 提供的 tracked、非秘密、只读交接合同。
它不是某台设备的运行期 manifest，不包含 handle、revision、状态或本机路径的真实值。本文件
完整枚举所有固定槽位与动态展开规则；运行期 `MigrationManifest.credential_sources` 只列
冻结快照中实际存在、可闭合到 `metadata_only` source record 的来源。固定槽位无可闭合来源时，
由本合同派生非秘密 `reconnect_required` 结果，不制造 placeholder `CredentialSourceContract`。

0.5 侧字段名与
`legacy_migration::{manifest,source_inventory}` 的冻结类型逐字对齐：

```rust
pub struct MigrationManifest {
    pub manifest_schema_version: u32,
    pub run_id: Uuid,
    pub legacy_presence: LegacyPresenceKind,
    pub source_database: Option<SourceFileRecord>,
    pub source_schema: Option<LegacySchemaReport>,
    pub app_data_files: Vec<SourceFileRecord>,
    pub excluded_paths: Vec<ExcludedSourceRecord>,
    pub credential_sources: Vec<CredentialSourceContract>,
    pub created_at: DateTime<Utc>,
}

pub struct CredentialSourceContract {
    pub stable_inventory_id: String,
    pub authority: CredentialAuthority,
    pub source_root: LegacyRootKind,
    pub source_locator: String,
}
```

`authority` 只能是以下两类，任何凭据来源必须且只能出现一次：

- `bridge_authoritative`：最终 0.4 bridge 的 authenticated envelope 是 0.5 导入权威源；
- `deferred_to_v5`：冻结的 0.4 活动明文快照是权威源，3A pending envelope 不得作为回退。

`source_root` 使用 `app_data | identifier_app_data | legacy_snapshot`；`source_locator` 必须是
无绝对路径、无 `..` 的相对 locator。`#` 前是用于 source inventory closure 的文件相对路径；
`#` 后 selector grammar 固定为三种：JSON Pointer（`/a/b`，按 RFC 6901 转义）、SQLite
`table.column`（row identity 由 importer 从该表主键确定）或 bridge
`stable_inventory_id`（逐字匹配 journal）。每个 locator 必须闭合到本次只读 source inventory
的 `metadata_only` 记录；不得把真实路径、secret、secret-derived fingerprint、token claim、
account email、案件数据或 ciphertext 写入 manifest。

## 2. 冻结输入与只读规则

最终 0.4 bridge 根固定为 `credential-bridge/v1/`。下表已按 0.4 `BridgePaths` 逐项核验：

| 相对路径 | 用途 | v5 source inventory 处置 |
| --- | --- | --- |
| `credential-store.sqlite` | metadata、envelope、stable journal | `connector_binding` + `metadata_only`；逐项认证与解密 |
| `master-key.v1` | 0.4 bridge master key | `connector_binding` + `metadata_only`；只读且绝不复制到 v5 |
| `manifest.json` | bridge schema/identity | `connector_binding` + `metadata_only`；校验 schema、权限与 bridge identity |
| `pending-migration-manifest.json` | 3A 非秘密 pending 状态 | `connector_binding` + `metadata_only`；不得当活动权威源 |
| `sanitization-manifest.json` | 3B settings hash 与 Active identity | `connector_binding` + `metadata_only`；校验已桥接族净化完成 |
| `legacy-system-import-manifest.json` | 37 槽位非秘密导入结果 | `connector_binding` + `metadata_only`；缺失/失败项映射重连 |
| `pending-migration.lock` | 0.4 迁移互斥文件 | `ExcludedSourceRecord`；不导入、不删除、不改写 |

0.5 当前只读 source inventory 在真实接线前必须完成这项精确 allowlist 更新；这是 Task 9
打开真实 final bridge 前的前置 gate。不得因旧 allowlist 只认识前三项而把最终 0.4 bridge
误判为未知目录，也不得通过放宽整个 `credential-bridge/v1/` 来接受未来未分类文件。运行期
导入只允许 Stable，Dev 在打开任何 bridge 路径前返回禁止；导入前后 0.4 bridge、settings、
旧数据库、旧 OS vault 与必要 AppData 的 hash/metadata 不得变化。

未来 importer validation gate 必须逐项校验：stable inventory ID、handle、
provider/connector、kind、owner、revision、state、journal mapping、envelope AAD 与
Active/snapshot 状态互相一致。这些是 importer 从 bridge metadata/store 验证的条件，不是
`CredentialSourceContract` 的新增字段。0.5 一次只解密一项，用独立 v5
handle/master key/AAD 重新 seal，提交幂等 receipt，清零后才能处理下一项；不得复制
ciphertext、master key、handle namespace，不双写、不回写 0.4。

`deferred_to_v5` 每项必须从冻结只读快照的精确 source/field 读取。缺失、格式漂移、不可读或
来源无法安全纳入快照时，该项显式进入 `reconnect_required`；不得读取 3A pending envelope，
不得回读旧 OS vault，不得猜测新 locator。

## 3. `bridge_authoritative` 完整枚举

### 3.1 已切换并完成 3B 净化的固定项

下表中的 `source_root` 均为 `app_data`，`source_locator` 均为
`credential-bridge/v1/credential-store.sqlite#<stable_inventory_id>`。运行期只为 bridge
store 中实际存在且可认证的条目生成 source contract；handle/revision 仍由 importer 在 store
内校验，不进入四字段 `CredentialSourceContract`。不存在或损坏的固定槽位以非秘密结果进入
`reconnect_required`，不得扫描旧 settings。

| inventory ID | stable_inventory_id | provider_or_connector_id | kind |
| --- | --- | --- | --- |
| `mineru-api-key` | `settings:mineru_api_key` | `ocr.mineru` | `api_key` |
| `paddle-vl-api-key` | `settings:paddle_vl_api_key` | `ocr.paddle_vl` | `api_key` |
| `cloud-llm-api-key` | `settings:cloud_llm_api_key` | `llm.deepseek` | `api_key` |
| `minimax-api-key` | `settings:minimax_api_key` | `llm.minimax` | `api_key` |
| `compat-llm-api-key` | `settings:compat_llm_api_key` | `llm.compat` | `api_key` |
| `glm-llm-api-key` | `settings:glm_llm_api_key` | `llm.glm` | `api_key` |
| `mimo-llm-api-key` | `settings:mimo_llm_api_key` | `llm.mimo` | `api_key` |
| `kimi-llm-api-key` | `settings:kimi_llm_api_key` | `llm.kimi` | `api_key` |
| `custom-llm-api-key` | `settings:custom_llm_api_key` | `llm.custom` | `api_key` |
| `yuandian-api-key` | `settings:yuandian_api_key` | `connector.yuandian` | `api_key` |
| `kuaidi100-key` | `settings:kuaidi100_key` | `connector.kuaidi100` | `api_key` |
| `embedding-api-key` | `settings:embedding_api_key` | `embedding` | `api_key` |
| `feishu-webhook-url` | `settings:feishu_webhook_url` | `feishu.reminder` | `webhook_secret` |

### 3.2 MCP 动态项

对冻结 settings 中每个具有不可变 UUID v4 `instance_id` 且 secret bundle 非空的 MCP，
运行期 manifest 必须逐项展开，不能保留宽泛 `mcp-env`/`mcp-headers` 槽位：

| stable_inventory_id 模板 | provider_or_connector_id | kind | 精确条件 |
| --- | --- | --- | --- |
| `settings:mcp:<instance_id>:env` | `mcp:<instance_id>` | `mcp_secret` | stdio `transport.env` bundle |
| `settings:mcp:<instance_id>:headers` | `mcp:<instance_id>` | `mcp_secret` | HTTP `transport.headers` bundle |

同一 `instance_id` 只能按真实 transport 形态产生对应条目；无 secret 的空 bundle 不制造假
credential。两行分别对应 inventory ID `mcp-env` 与 `mcp-headers`。缺失、非 UUID v4 或
重复 identity 必须 fail visibly，不得派生或替换 ID。

### 3.3 `legacy_system_import_snapshot` 固定 37 槽位

以下项目的 `authority` 仍写 `bridge_authoritative`，但必须额外按本节语义解释为
`legacy_system_import_snapshot`。它只表示“用户显式导入时写入 bridge 的快照”是 0.5
唯一可读交接源，**不表示 bridge 是 0.4 Pi/Research runtime 权威源**。

本 tracked 合同固定枚举全部 37 槽位；运行期处理分为：

- imported/already_imported 且最新 revision 可认证时，`source_locator` 指向
  `credential-bridge/v1/credential-store.sqlite#<stable_inventory_id>`；
- pending/missing/unreadable/failed 或无 authenticated envelope，但
  `legacy-system-import-manifest.json` 实际存在时，可用
  `credential-bridge/v1/legacy-system-import-manifest.json#<stable_inventory_id>` 闭合到
  非秘密结果，0.5 进入 `reconnect_required`；
- 用户从未执行显式导入、因而 system-import manifest 不存在时，该槽位不产生
  `CredentialSourceContract`；0.5 依据本固定枚举直接产生 `reconnect_required`，不得创建
  假文件、假 locator 或打开旧 OS vault。

| stable_inventory_id | provider_or_connector_id | kind |
| --- | --- | --- |
| `legacy-system:pi:amazon-bedrock:credential` | `pi.amazon-bedrock` | `pi_credential_bundle` |
| `legacy-system:pi:ant-ling:credential` | `pi.ant-ling` | `pi_credential_bundle` |
| `legacy-system:pi:anthropic:credential` | `pi.anthropic` | `pi_credential_bundle` |
| `legacy-system:pi:azure-openai-responses:credential` | `pi.azure-openai-responses` | `pi_credential_bundle` |
| `legacy-system:pi:cerebras:credential` | `pi.cerebras` | `pi_credential_bundle` |
| `legacy-system:pi:cloudflare-ai-gateway:credential` | `pi.cloudflare-ai-gateway` | `pi_credential_bundle` |
| `legacy-system:pi:cloudflare-workers-ai:credential` | `pi.cloudflare-workers-ai` | `pi_credential_bundle` |
| `legacy-system:pi:deepseek:credential` | `pi.deepseek` | `pi_credential_bundle` |
| `legacy-system:pi:fireworks:credential` | `pi.fireworks` | `pi_credential_bundle` |
| `legacy-system:pi:github-copilot:credential` | `pi.github-copilot` | `pi_credential_bundle` |
| `legacy-system:pi:google:credential` | `pi.google` | `pi_credential_bundle` |
| `legacy-system:pi:google-vertex:credential` | `pi.google-vertex` | `pi_credential_bundle` |
| `legacy-system:pi:groq:credential` | `pi.groq` | `pi_credential_bundle` |
| `legacy-system:pi:huggingface:credential` | `pi.huggingface` | `pi_credential_bundle` |
| `legacy-system:pi:kimi-coding:credential` | `pi.kimi-coding` | `pi_credential_bundle` |
| `legacy-system:pi:minimax:credential` | `pi.minimax` | `pi_credential_bundle` |
| `legacy-system:pi:minimax-cn:credential` | `pi.minimax-cn` | `pi_credential_bundle` |
| `legacy-system:pi:mistral:credential` | `pi.mistral` | `pi_credential_bundle` |
| `legacy-system:pi:moonshotai:credential` | `pi.moonshotai` | `pi_credential_bundle` |
| `legacy-system:pi:moonshotai-cn:credential` | `pi.moonshotai-cn` | `pi_credential_bundle` |
| `legacy-system:pi:nvidia:credential` | `pi.nvidia` | `pi_credential_bundle` |
| `legacy-system:pi:openai:credential` | `pi.openai` | `pi_credential_bundle` |
| `legacy-system:pi:openai-codex:credential` | `pi.openai-codex` | `pi_credential_bundle` |
| `legacy-system:pi:opencode:credential` | `pi.opencode` | `pi_credential_bundle` |
| `legacy-system:pi:opencode-go:credential` | `pi.opencode-go` | `pi_credential_bundle` |
| `legacy-system:pi:openrouter:credential` | `pi.openrouter` | `pi_credential_bundle` |
| `legacy-system:pi:together:credential` | `pi.together` | `pi_credential_bundle` |
| `legacy-system:pi:vercel-ai-gateway:credential` | `pi.vercel-ai-gateway` | `pi_credential_bundle` |
| `legacy-system:pi:xai:credential` | `pi.xai` | `pi_credential_bundle` |
| `legacy-system:pi:xiaomi:credential` | `pi.xiaomi` | `pi_credential_bundle` |
| `legacy-system:pi:xiaomi-token-plan-ams:credential` | `pi.xiaomi-token-plan-ams` | `pi_credential_bundle` |
| `legacy-system:pi:xiaomi-token-plan-cn:credential` | `pi.xiaomi-token-plan-cn` | `pi_credential_bundle` |
| `legacy-system:pi:xiaomi-token-plan-sgp:credential` | `pi.xiaomi-token-plan-sgp` | `pi_credential_bundle` |
| `legacy-system:pi:zai:credential` | `pi.zai` | `pi_credential_bundle` |
| `legacy-system:pi:zai-coding-cn:credential` | `pi.zai-coding-cn` | `pi_credential_bundle` |
| `legacy-system:research:exa:api-key` | `research.exa` | `api_key` |
| `legacy-system:research:firecrawl:api-key` | `research.firecrawl` | `api_key` |

Pi API-key snapshot 是完整原子 `key + env` bundle；OAuth snapshot 是完整原子
`access + refresh + expires + extra` bundle。不得拆 leaf、混合不同 revision 或把
`caseboard-custom` 加进 37 槽位。0.4 auth/refresh/save 与 Research
status/verify/save/remove 在显式导入后仍可能更新旧 OS vault，因此该 snapshot 随时间可能
过期。0.4 system-import manifest 只记录
`pending/imported/already_imported/missing/unreadable/failed`；`stale`、OAuth expires
已过和 AAD/revision mismatch 是未来 v5 importer 对 snapshot 的派生判断，不是虚构的 0.4
manifest state。0.5 只取最终冻结前该 stable handle 的最新 authenticated revision；上述任一
派生失败均进入 `reconnect_required`，不得静默宣称成功。

## 4. `deferred_to_v5` 完整枚举

下列项的 0.4 活动明文位置是唯一权威源。运行期 manifest 必须按实际 identity/row 展开；
3A pending envelope 即使存在也只算历史 sealed copy。

| inventory / stable ID | source_root | source_locator / 展开规则 | v5 处理 |
| --- | --- | --- | --- |
| `feishu-app-token` / `settings:feishu_app_token` | `app_data` | `settings.json#/feishu_app_token` | 逐项重加密 |
| `court-filing-password` / `settings:court_filing_password` | `app_data` | `settings.json#/court_filing_password` | 逐项重加密 |
| `team-secret` / `settings:team:<team_id>:team_secret` | `app_data` | `settings.json#/team/team_secret` | 按真实 `team_id` 展开 |
| `team-pairing-code` / `settings:team:<team_id>:pairing_code` | `app_data` | `settings.json#/team/pairing_code` | 按真实 `team_id` 展开 |
| `device-group-secret` / `settings:device_sync:<group_id>:group_secret` | `app_data` | `settings.json#/device_sync/group_secret` | 按真实 `group_id` 展开 |
| `device-pairing-code` / `settings:device_sync:<group_id>:pairing_code` | `app_data` | `settings.json#/device_sync/pairing_code` | 按真实 `group_id` 展开 |
| `ticktick-access-token` / `ticktick:tokens:accessToken` | `app_data` | `ticktick_sync.json#/tokens/accessToken` | 逐项重加密 |
| `ticktick-refresh-token` / `ticktick:tokens:refreshToken` | `app_data` | `ticktick_sync.json#/tokens/refreshToken` | 逐项重加密 |
| `court-cookie-json` / `court_filing:cookie_store` | 仅当真实文件被合法 source inventory 接纳时确定 | 实际文件格式 `court_zxfw_<仅保留字母数字的 account>.json`；settings 的 `court_filing_cookie_dir` 与 `court_filing_account` 只是非秘密定位输入，不能冒充 Cookie source locator | 当前外置文件无法闭合时不产生 `CredentialSourceContract`，直接重连 |
| `case-sync-key` | `legacy_snapshot` | `caseboard.db#cases.sync_key` | 按每个实际 row identity 展开 |
| `artifact-case-sync-key` | `legacy_snapshot` | `caseboard.db#device_sync_artifacts.case_sync_key` | 按每个实际 row identity 展开 |
| `inbox-case-sync-key` | `legacy_snapshot` | `caseboard.db#device_sync_inbox.case_sync_key` | 按每个实际 row identity 展开 |
| `source-inbox-case-sync-key` | `legacy_snapshot` | `caseboard.db#device_sync_source_inbox.case_sync_key` | 按每个实际 row identity 展开 |

这些路径在 0.4 继续保留现役 argv、Cookie 文件、TickTick 跨设备同步、join DTO、pairing
轮换写回与 per-case store 行为；Task 7 不切 consumer、不净化、不实现 v5 importer。
0.5 必须从冻结快照读，导入后也不得反向净化或改写 0.4。

`ticktick-tokens-bundle` 是 inventory 中的结构容器分类（`derive_from_leaf_entries`），不是第三个
credential，也不进入 `credential_sources`；它只要求上述 access/refresh 两个 leaf 完整覆盖。
0.4 源码从根 AppData 的 `ticktick_sync.json` 读取，0.5 当前只接受
`identifier_app_data/ticktick_sync.json` 的实现必须在真实 Task 9 前按此当前真相修正并补负向
测试。

## 5. manifest 完整性与失败映射

1. `credential_sources` 先按 `authority`、再按 `stable_inventory_id` 确定性排序。
2. 固定 bridge 槽位与固定 deferred inventory 不得从本 tracked 合同中消失；运行期无值或无
   可闭合 source record 时不制造 placeholder contract，而是进入非秘密逐项结果。动态 MCP、
   team/device identity 与 SQLite row 只按冻结快照中真实存在的对象展开，不制造 placeholder
   handle。
3. 同一 `stable_inventory_id` 重复、跨 authority 重复、bridge 条目指向活动明文、
   deferred 条目指向 `credential-bridge/`、不安全 locator、未知 kind/state、manifest/store
   identity 不闭合均 fail-closed。
4. `bridge_authoritative` 的缺失、损坏、过期、revoked/unreadable 与
   `legacy_system_import_snapshot` 的 stale/expired/missing/unreadable/failed 映射为
   `reconnect_required`；单项失败不回滚已成功导入项，也不阻断 App 启动。
5. `deferred_to_v5` 的 source missing/drifted/unreadable 映射为 `reconnect_required`，绝不
   回退 3A pending envelope。
6. 同一 source bridge ID/handle/revision 的 receipt 重跑零解密、零写入；v5 已有
   `user_entered` 更新值时不得被 bridge import 回灌覆盖。

上述失败映射同时落实 2026-07-27 增量决策 3 的分层兜底：配置/凭据允许用户重填，但失败
必须逐项可见、可重试，0.4 数据保持只读可退回；“可重填”绝不能被解释成静默迁移成功。

## 6. 非目标与审查清单

- 本 Task 只冻结 tracked 契约，不创建 synthetic store/key，不实现或提前调用 v5 importer。
- 不读取真实 AppData、Keychain、Credential Manager、案件库、Cookie 或用户路径。
- 不修改 0001–0049 migration，不修改 0.4 runtime/writeback，不做网络或付费调用。
- 不授权 commit、push、tag、Release、部署、metadata 更新或 `/Applications` 替换。
- Claude 整段复审时必须同时核对：双 authority 完整枚举、37 槽位数量、MCP/deferred 动态
  展开规则、真实 bridge 目录 allowlist、只读/独立重加密、逐项 receipt 与
  `reconnect_required` 失败映射。
