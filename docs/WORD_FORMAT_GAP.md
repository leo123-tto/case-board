# CaseBoard · Word 导出格式差距清单（GAP · V1）

> 蒸馏日期：2026-07-24
> 配套规范：`docs/WORD_FORMAT_SPEC.md`（V1，13 节 + 2 附录）
> 对照基准：2026-07-24 仓库 `src-tauri/src/docx_filing.rs`（755 行 + Profile 3 档）+ `docx_extract.rs`（120 行 zip 解析）

---

## 〇、TL;DR · 一图看全

| 维度 | 规范要求 | 当前实现 | 差距 | 优先级 |
|---|---|---|---|---|
| 仿宋/Times 双字体 | ✅ 必 | ✅ 已 | — | — |
| 14pt 正文 / 1.5 倍行距 / 首行缩进 2 字 | ✅ 必 | ✅ 已 | — | — |
| 中文标点全角化 | ✅ 必 | ✅ 已（`normalize_cjk_punct`） | — | — |
| 表格六向单线边框 | ✅ 必 | ✅ 已 | — | — |
| 空段丢弃 | ✅ 必 | ✅ 已 | — | — |
| GFM 表格 + 嵌套列表 + 分隔线 | ✅ 必 | ✅ 已 | — | — |
| 纯 Rust 零外部依赖 | ✅ 必 | ✅ 已（zip + pulldown-cmark） | — | — |
| **页眉**（机构 + 案件名 + 页码） | ✅ 必 | ❌ **缺失** | 新增部件 | **P0** |
| **页脚页码字段**（PAGE/NUMPAGES） | ✅ 必 | ❌ **缺失** | 新增部件 | **P0** |
| **交付前 Gate**（9 项检查） | ✅ 必 | ❌ **缺失** | 新增模块 | **P0** |
| Heading 样式引用 | ⚠️ 软 | ⚠️ 故意 inline rPr | 规范豁免 | P2 |
| 横排 A4 适配 | ⚠️ 按需 | ❌ 缺 | 用则做 | P3 |

**一句话总结**：CaseBoard 的"基础排版已经做得很扎实"，**缺三件大事**——页眉、页脚页码、Gate 门禁。

---

## 一、已对齐（✅ · 不动）

| 项 | 实现位置 | 备注 |
|---|---|---|
| 仿宋_GB2312 + Times New Roman 双字体 | `docx_filing.rs::render_run` | inline rPr，同 run 上 `ascii` + `eastAsia`，**这是法律文书灵魂** |
| 方正小标宋居中标题 / SimHei 黑体小标题 | `Role::east_asia` | 三档 Profile 区分 Base/Filing/Editor |
| 14pt 正文 / 15pt 一级 / 14pt 二级 | `Role::sz` | 半点单位（28 / 30 / 28） |
| 1.5 倍行距 | `<w:spacing w:line="360" w:lineRule="auto"/>` | 段落渲染内联 |
| 首行缩进 2 字（560 twip） | `<w:ind w:firstLine="560"/>` | Title 角色豁免 |
| 两端对齐（除居中） | `<w:jc w:val="both"/>` 或 `center` | 列表项 Editor 档左对齐 |
| 中文标点全角化 | `normalize_cjk_punct` | 6 类标点 + 数字豁免 |
| 表格六向单线边框 | `render_table` | sz=4 = 0.5pt，黑色 |
| 表后空段（OOXML 必需） | `render_document_xml` | `last_is_table` 检测 |
| 空段丢弃 | `Walker::flush_cur` | 只丢弃纯空白段 |
| 列表圆点（base）/ 编号（filing） | `Walker::start` | 按 profile 切换 |
| 嵌套左悬挂缩进 | `render_para` | `420*depth + 280` twip |
| GFM 表格解析 | `Walker::start/end` | 沿 pulldown-cmark |
| `---` 分隔线 | `render_rule` | base 档渲染，filing 丢弃 |
| HTML 注释剥除 | `strip_html_comments` | 防止 artifact 头污染 |
| 容器骨架（zip + OOXML XML） | `CONTENT_TYPES / RELS / STYLES / SETTINGS` | 取自样本，Word 完整打开 |
| A4 页面 + 1 英寸边距 | `SECTPR` | `11906×16838 / 1440` |
| 纯 Rust 零外部依赖 | `Cargo.toml` | `zip` + `pulldown-cmark` 已编进二进制 |

**评估**：基础排版 100% 对齐 quote.law 15 份样本的口径。**这一块不动**。

---

## 二、缺失项（❌ · 需要补）

### Gap #1 · 页眉（机构 + 案件名 + 页码）

**规范要求**（SPEC § 三）：所有对外发送的 .docx 必须有页眉，含机构名 / 案件名 / 页码。

**当前实现**：

```rust
// docx_filing.rs:740
const SECTPR: &str = r#"<w:sectPr>
  <w:pgSz w:w="11906" w:h="16838" w:orient="portrait"/>
  <w:pgMar ... w:header="708" w:footer="708" .../>  ← 仅是页眉/页脚区高度
  <w:docGrid w:linePitch="360"/>
</w:sectPr>"#;
```

`pgMar` 里的 `header="708"` 只是"页眉到顶边的距离"，**根本没有 `word/header1.xml` 部件**。

**落地成本**：**中**

- 新增 `word/header1.xml` 字符串（~300 字）
- `_rels/document.xml.rels` 加一行 `Relationship Id="rId4" Type=".../header" Target="header1.xml"`
- `[Content_Types].xml` 加 `Default Extension="xml"` 已有，**Override 加一个** `PartName="/word/header1.xml" ContentType="...header+xml"`
- `SECTPR` 加 `<w:headerReference r:id="rId4" w:type="default"/>`
- `build_docx_bytes` 的 `put` 闭包加一行 `put("word/header1.xml", HEADER1_XML)?`
- 设计一个 `HeaderInputs { org, case_name, case_no }` 让调用方传入

**影响**（不做的话）：**对外发送的 .docx 没有机构抬头，律师/当事人看到的第一眼就是"非正式"**。

**优先级**：**P0**

---

### Gap #2 · 页脚页码字段（PAGE / NUMPAGES）

**规范要求**（SPEC § 四）：页脚必须用 `PAGE` 字段，**禁止硬编码数字**。

**当前实现**：同上，无 `word/footer1.xml`。

**落地成本**：**低**

- 与 Gap #1 几乎对称（再加一个 footer1.xml）
- 关键是 `<w:fldSimple w:instr="PAGE">` 与 `NUMPAGES` 字段，**不要写成 `<w:t>1</w:t>`**
- 字段渲染失败时降级为硬编码 + 标 WARNING（SPEC § 十二）

**影响**：法院 / 当事人看到 1/1、5/5 的硬编码页码 = 不会自动重排 = 删段后页码错乱 = 不专业。

**优先级**：**P0**

---

### Gap #3 · 交付前 Gate（9 项检查 · md2word-gate 同款）

**规范要求**（SPEC § 十一）：所有"对外发送"路径必须跑门禁，9 项检查 + 4 档退出码。

**当前实现**：**完全没有**。`build_docx_bytes` 直接 `zip.finish()` 就交差。

**落地成本**：**中**

- 新建 `src-tauri/src/docx_gate.rs`（预估 250-350 行）
- **零新依赖**：`zip` 和 `quick-xml` 已经在 `Cargo.toml`
  - `zip = { version = "2", default-features = false, features = ["deflate"] }` ← docx_filing 用的
  - `quick-xml = { version = "0.39", features = ["encoding"] }` ← docx_extract 用的
- **可复用 `docx_extract.rs` 的 zip 读取范式**（`zip::ZipArchive::new(f) → by_name("word/document.xml")`）
- 9 项检查的具体实现：

| # | 检查 | 实现 |
|---|---|---|
| 1 | OpenXML 完整性 | zip 列出 `word/document.xml` `word/styles.xml` `word/settings.xml` 全部存在 |
| 2 | 页眉存在 | 列出 `word/header*.xml` |
| 3 | 页脚页码字段 | 解析 `word/footer*.xml` 找 `w:instr="PAGE"` 字符串 |
| 4 | 标题样式 | 扫描 `document.xml` 找 `w:pStyle` 或 inline rPr（H1 字号=30/34 校验） |
| 5 | 表格边框 | 扫描 `w:tblBorders` 6 个 `<w:.../>` 元素 |
| 6 | 字体合规 | 扫描 `w:rFonts w:eastAsia` 取值范围（仿宋/黑体/方正小标宋/Songti/Menlo） |
| 7 | 中文标点全角 | 解析所有 `w:t` 内容，正则匹配 CJK 旁的半角标点 |
| 8 | 空段检测 | 扫描连续 `<w:p/>` 数量 |
| 9 | 字符数统计 | `w:t` 内容累加 |

- 入口 `pub fn run_gate(docx_path: &Path) -> GateReport`，返回结构体 + 退出码
- `docx_filing::build_docx_bytes` 末尾调用一次，返回 `Result<Vec<u8>, String>`，让调用方决定是否阻断
- 前端（React）拿到 `GateReport` 显示给用户

**影响**（不做的话）：md2word-gate 第一次实跑就抓出 42 处半角标点，**没有 Gate 意味着"差不多就行"在 CaseBoard 重演**。

**优先级**：**P0**

---

## 三、软对齐（⚠️ · 规范豁免或按需做）

### Gap #4 · Heading 样式引用

**当前实现**（`docx_filing.rs` 注释）：

> "inline rPr 不靠段落样式（quote.law 签名,本模块完全复刻）"

**规范要求**（SPEC § 十一 检查 4）：希望有 `<w:pStyle w:val="Heading1"/>` 引用，**但**法律文书普遍用 inline 字号定义，**SPEC 已豁免**（WARNING 级别）。

**结论**：**保持现状**。Gate 工具扫描到 inline rPr 不报 FAIL。

**优先级**：**P2 · 现状合规**

---

### Gap #5 · 横排 A4 适配

**当前实现**：只支持纵向。

**规范要求**（SPEC § 一）：仅"多列大表格"场景按 VB-word 横表自适应切横排。

**判断**：CaseBoard 报告档的表格列数目前 ≤6（看 `render_table` 9026/列数等分），**没有触发横排需求**。**用则做**。

**优先级**：**P3 · 不主动实现**

---

## 四、文档缺口（📄 · 顺手补）

### Gap #6 · V0.3-文书格式规范 文档未建

**当前状态**：`docx_filing.rs` 注释引用 `docs/V0.3-文书格式规范`（line 15），**但 docs/ 下不存在**——只有贡献流程文档（BRANCH_STRATEGY / COMMIT_CONVENTIONS / CONTRIBUTING_FLOW 等）。

**本轮已补建**：

- `docs/WORD_FORMAT_SPEC.md`（V1，13 节 + 2 附录）— 蒸馏目标规范
- `docs/WORD_FORMAT_GAP.md`（本文件）— 现状 vs 规范

**建议**：把 `docx_filing.rs` 注释里的"见 docs/V0.3-文书格式规范"改为"见 docs/WORD_FORMAT_SPEC.md"。

**优先级**：**P1 · 注释同步**

---

## 五、落地优先级矩阵

| Gap | 工作量 | 业务影响 | 技术风险 | 落地顺序 |
|---|---|---|---|---|
| #3 交付前 Gate | 中（~300 行） | **极高**（防"差不多就行"） | 低（已有 zip + quick-xml） | **P0 · 1st** |
| #2 页脚页码字段 | 低（~50 行） | 高（法院/当事人必看） | 低（fldSimple 成熟） | **P0 · 2nd** |
| #1 页眉 | 中（~150 行） | 高（机构抬头第一印象） | 低 | **P0 · 3rd** |
| #6 注释同步 | 极低（一行改动） | 低 | 无 | **P1** |
| #4 Heading 样式 | — | — | — | 豁免 |
| #5 横排 A4 | — | — | — | 不主动做 |

**建议落地节奏**：

1. **P0 · 本周**：Gate 工具 + 页脚页码 + 页眉（3 个一起做，整体 PR）
2. **P1 · 顺手**：docx_filing.rs 注释路径修正
3. **P2/P3 · 不动**：豁免 / 按需

---

## 六、依赖复用情况（无需新增 crate）

| 能力 | 现有 crate | 复用位置 |
|---|---|---|
| zip 读写 | `zip = "2"` | `docx_filing`（写）+ `docx_extract`（读） |
| XML 解析 | `quick-xml = "0.39"` | `docx_extract`（仅 Event 流） |
| 时间戳 | `chrono = "0.4"` | `chrono::Local::now()`（已在 `export.rs` 用） |
| 文件路径 | `std::path::Path` | 已有 |
| 错误处理 | `String`（项目惯例） | 已有 |

**结论**：Gate 工具**完全零新依赖**，符合"降级路径是生产级底线"原则（SPEC § 十二）。

---

## 七、隐私红线复述（与 `docs/PRIVACY_IRON_RULE.md` 联动）

落地时**必须**确认：

- ❌ Gate 工具不读取也不向 LLM 提交 docx **内容**——只做结构扫描
- ❌ 测试 fixture 用占位符（`{{PLAINTIFF}}` / `{{CASE_NO}}`），不写死真实案件
- ❌ 页眉的"机构名"必须用配置注入（settings），不写死"信拓集团"在代码里（其他用户跑会有问题）
- ✅ 报告档的"案件速览"等元信息在 `cases` 表已有，跟 `export.rs` 口径一致（已对齐）

---

## 附录 A · 本 GAP 文档的版本演进

| 版本 | 日期 | 变更 |
|---|---|---|
| V1 | 2026-07-24 | 首版（6 个 gap + 落地矩阵 + 复用表） |

---

## 附录 B · 落地前自检 Checklist

落地时按这张表打钩：

- [ ] P0 #1 页眉：机构名从 settings 注入，案件名从 `cases.name` 注入
- [ ] P0 #2 页脚：PAGE + NUMPAGES 字段验证通过（Word 里删一段页码会自动变）
- [ ] P0 #3 Gate：9 项检查全部实现 + 4 档退出码
- [ ] P0 #3 Gate：测试覆盖 base / filing / editor 三档
- [ ] P0 #3 Gate：测试 fixture 全用占位符（不写真实案件）
- [ ] P0 #3 Gate：失败时**不阻塞导出**，仅报告
- [ ] P1 #6：`docx_filing.rs` 注释路径修正
- [ ] 全程 `cargo fmt && cargo clippy --all-targets -- -D warnings` 通过
- [ ] `pnpm tauri dev` 端到端走一遍：导出 → 打开 .docx → 验页眉/页脚/Gate 报告

---

**维护**：任何 docx_filing.rs 改动必须**先更新 WORD_FORMAT_SPEC.md 与本文件，再写代码**。
