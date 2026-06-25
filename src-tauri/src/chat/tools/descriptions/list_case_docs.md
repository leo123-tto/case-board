list_case_docs — 列当前案件所有文档(filename + category + 是否 AI 产物 + extracted_text_path 可读性)

适用场景:
- 用户问「这个案件有哪些文档」「证据材料都在哪」
- LLM 接到任务后,**第一步**就是列文档清单,了解可用材料范围
- 起草前先看文档清单决定要读哪些(配合 `read_case_doc` 拿全文)
- 给用户做案件画像 / 证据目录 / 时间线时的入口

不适用:
- 想看具体文档内容 → 用 `read_case_doc(doc_id)`
- 想在某份文档里找字符串 → 用 `find_in_document(doc_id, pattern)`
- 想搜本地知识库(不是案件文档) → 用 `search_local_kb`
- 当前没绑定案件(自由问答模式) → 本工具会报错 `NoCaseBound`

输入字段:
- 无参数,自动用 ctx.case_id

注意事项:
- **本工具不消耗元典积分**(本地 sqlite 查询)
- 返回结构:`[{id, filename, display_name, category, organized_category, importance, party_side, evidence_attitude, submission_stage, organize_tags, is_ai_artifact, source(scan/llm_extract/chat), has_extracted_text, pinned_at, size_bytes}]`
  - **id** 是文档主键,后续 `read_case_doc` / `find_in_document` 都用这个 id 引用
  - **category**:文档分类(起诉状 / 合同 / 判决 / ...),由扫描时的分类器打的
  - **display_name**:看板内显示名;若非空,优先按它理解材料类型
  - **organized_category**:AI/人工整理后的材料归类(起诉材料 / 证据 / 法院文书 / 对方材料 / 程序文书 / 参考材料 / 其他),优先级高于扫描 `category`
  - **importance**:重要 / 忽略。`忽略` 的材料默认不要纳入一键任务语料,除非用户明确点名
  - **party_side**:原告 / 被告 / 第三人,可多值。用于区分我方材料和对方材料
  - **evidence_attitude**:有利 / 不利 / 中性,用于证据目录、质证意见、攻防分析
  - **submission_stage**:起诉/答辩随附 / 举证期限内 / 补充提交 / 二审新证据 / 未提交或待确认
  - **is_ai_artifact**:true = AI 全局抽生成的 .md 报告(画像 / 风险报告 / 深挖等);false = 原始扫描件
  - **source**:`scan` = 原始文件,`llm_extract` = LLM 全局抽产物,`chat` = chat artifact
  - **has_extracted_text**:true = 已抽取过文字可以 `read_case_doc` 拿全文;false = 抽取未完成 / 失败
  - **pinned_at**:用户在引用弹窗里把这份文档置顶的时间(非 null 表示置顶)
- 返回按 stage(扫描阶段)+ filename 排序,置顶文档不影响排序(置顶仅前端用)
- 文档很多(几十 / 上百份)时,LLM **不要全列给用户**,挑用户问题相关的 5-10 份汇报
- 列文档时,优先告诉用户哪些是「原始证据 vs. AI 产物」,帮用户区分

一键任务筛材纪律:
- 先读案件快照里的「我方代理立场」,再结合 `organized_category` / `party_side` / `importance` / `evidence_attitude` 选材料。
- 已标 `importance=忽略`、`organized_category=法院文书/程序文书/参考材料/其他` 的材料,默认不要放进写证据目录、写答辩状、出质证意见的主语料。
- 写我方证据目录:只选我方一侧 `organized_category=证据` 且非忽略的材料;传票、开庭通知、法院文书、对方证据不列入我方证据目录,除非用户明确要求。
- 写答辩状/质证意见:我方为被告方时,优先读原告起诉材料、原告证据、我方被告证据;按“诉讼请求/事实理由/原告证据证明目的 → 被告抗辩/反证/质证意见”的顺序组织。
