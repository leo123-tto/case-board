search_local_kb — 在作者本地法律知识库做不消耗元典积分的关键词检索

适用场景:
- 用户问通用法律问题(「合同解除有哪几种情形」「商标侵权的赔偿计算」),**先在本地查作者已经整理过的资料**,比调元典更省 + 更贴合作者风格
- 宽口径研究时可以搜索根目录下未归档的 `.md` / `.txt`；但法规、案例、企业等正式检索编排仍按规定目录分层，不把自建目录结果直接当作强命中原文
- **办理新案件 / 被问「以前办过类似案件吗」时,先查办案经验**:`raw/cases-experience/` 是作者结案后沉淀的办案经验卡片(争议焦点 / 裁判规则 / 法条适用 / 办案心得),办同类案件可直接检索复用
- 起草前看作者以前怎么写过类似条款 / 论点
- 看 wiki/sources/ 里整理过的法规 / 判例 / 学说要点
- 看 wiki/topics/ 里关于某主题的体系化梳理
- 看 gap-log.md 看是不是有未补全的研究缺口
- 「先本地后外查」优先级的核心体现 — KB 命中等于 0 元典积分

**本地导航跳**:本工具使用中文 bigram + BM25 排序。正式的法规/案例编排会先单独查 `wiki/topics` / `wiki/sources`，再沿卡片 `source_path` 回到真实 raw，然后查 raw 关键词；本工具返回的宽口径结果不能替代该分层。对标题、法规名和「法规名 + 第 X 条」做结构加权；关键词仍不足时，再用 raw embedding 补召回。

**关键词 vs 语义,怎么选**:已知法规名 / 条号 / 案号 / 专有名词时，本工具通常最准；描述型问题、本工具 `weak_hits=true` 或前排明显跑题时，再用 `semantic_search_local_kb` 做本地语义补检。两者都不足才去元典。

不适用:
- 想看元典详情缓存 → 正式编排会显式搜索法规/法条/案例完整详情；本工具默认不搜缓存，且 `SEARCH-*` 结果列表始终排除
- 想读当前案件的文档 → 用 `list_case_docs` / `read_case_doc`(本工具不进 case extracts/)
- 想读 KB 里某个具体文件 → 已知路径直接 `read_kb_file`,不必先搜

输入字段:
- keyword: 必填,中文关键词。支持中文 bigram、长 query 和精确条号结构识别
- scope: 可选,数组 `["root","notes","companies","cases_experience","sources","topics","gap_log"]` 任意子集,默认 `root` 整根知识库
  - root = 整个知识库根目录下的 `.md` / `.txt`(默认宽口径搜索；排除 `_inbox`、`00_ARCHIVE`、技术目录和元典缓存)
  - notes = raw/notes/(作者笔记)
  - companies = raw/companies/(企业档案 / 调查报告)
  - cases_experience = raw/cases-experience/(结案沉淀的办案经验卡片)
  - sources = wiki/sources/(整理过的来源页)
  - topics = wiki/topics/(专题页)
  - gap_log = gap-log.md(缺口清单)
- include_yuandian_cache: 可选,默认 false。`true` 时**额外**搜 raw/yuandian-cache/(慎用,会出现一堆元典缓存)
- max_results: 可选,默认 30,最大 100

注意事项:
- **本工具不消耗元典积分**(本地 BM25 词法检索)
- 返回结构:`{query, kb_root, weak_hits, results:[{relative_path, scope, title, doc_type, score, match_count, snippet, modified_at}]}`
  - `weak_hits=true` 表示 0 结果，或前 3 条全是 raw/cache 且没有高置信“法规名+精确条号”结构命中；只是快速分流信号，仍要看前排标题、分数和片段
  - `relative_path` 是 KB 内相对路径(如 `wiki/sources/合同解除-民法典-563.md`)
  - `snippet` 是命中位置前后 200 字符
  - `score` 是 BM25 + 标题/法规名/精确条号结构加权分，越高越相关
- 命中 Wiki source 后先读卡片，再沿其 `source_path` 读取真实 raw；直接命中 raw 时只读相关文件/段落
- 引用 KB 内容时,**必须**加入 `<CITATIONS>`:type = `"kb_local"`,title = 文件名(去 .md),source = relative_path
- 如果 KB 未启用(用户没填 local_kb_root) → 本工具直接返回空数组,不报错(降级)
