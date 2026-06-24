search_local_kb — 在作者本地法律知识库 `~/Documents/知识库/` 整根目录检索已积累的笔记 / 专题页 / 来源页

适用场景:
- 用户问通用法律问题(「合同解除有哪几种情形」「商标侵权的赔偿计算」),**先在本地查作者已经整理过的资料**,比调元典更省 + 更贴合作者风格
- 用户把不同系统/不同命名习惯迁移来的知识库目录直接接进来时,默认也会扫整根目录下的 `.md` / `.txt`,不要求必须叫 `raw/notes` 或 `wiki/sources`
- **办理新案件 / 被问「以前办过类似案件吗」时,先查办案经验**:`raw/cases-experience/` 是作者结案后沉淀的办案经验卡片(争议焦点 / 裁判规则 / 法条适用 / 办案心得),办同类案件可直接检索复用
- 起草前看作者以前怎么写过类似条款 / 论点
- 看 wiki/sources/ 里整理过的法规 / 判例 / 学说要点
- 看 wiki/topics/ 里关于某主题的体系化梳理
- 看 gap-log.md 看是不是有未补全的研究缺口
- 「先本地后外查」优先级的核心体现 — KB 命中等于 0 元典积分

**关键词 vs 语义,怎么选**:本工具是**关键词精确匹配**(适合已知确切法条号 / 案号 / 人名 / 专有名词);**按含义 / 同义改写找法条或类案,优先用 `semantic_search_local_kb`(语义向量,整部法律已按法条切片,命中更准)**。两者互补。

不适用:
- 想看元典缓存(法规 / 案例 / 公司缓存) → 元典工具(`search_laws` / `search_cases_normal` 等)自带 KB-cache,本工具默认**不**搜元典缓存
- 想读当前案件的文档 → 用 `list_case_docs` / `read_case_doc`(本工具不进 case extracts/)
- 想读 KB 里某个具体文件 → 已知路径直接 `read_kb_file`,不必先搜

输入字段:
- keyword: 必填,中文关键词。**支持中文分词**,长 query 也行(整库 regex grep)
- scope: 可选,数组 `["root","notes","companies","cases_experience","sources","topics","gap_log"]` 任意子集,默认 `root` 整根知识库
  - root = 整个知识库根目录下的 `.md` / `.txt`(默认;排除技术目录和元典缓存碎片)
  - notes = raw/notes/(作者笔记)
  - companies = raw/companies/(企业档案 / 调查报告)
  - cases_experience = raw/cases-experience/(结案沉淀的办案经验卡片)
  - sources = wiki/sources/(整理过的来源页)
  - topics = wiki/topics/(专题页)
  - gap_log = gap-log.md(缺口清单)
- include_yuandian_cache: 可选,默认 false。`true` 时**额外**搜 raw/yuandian-cache/(慎用,会出现一堆元典缓存)
- max_results: 可选,默认 30,最大 100

注意事项:
- **本工具不消耗元典积分**(本地文件 grep)
- 返回结构:`[{relative_path, scope, match_count, snippet, modified_at}]`
  - `relative_path` 是 KB 内相对路径(如 `wiki/sources/合同解除-民法典-563.md`)
  - `snippet` 是命中位置前后 200 字符
  - `match_count` 命中次数,排序优先级:命中多 > 修改时间新
- 命中后挑 1-2 个最相关的用 `read_kb_file(relative_path)` 拿全文
- 引用 KB 内容时,**必须**加入 `<CITATIONS>`:type = `"kb_local"`,title = 文件名(去 .md),source = relative_path
- 如果 KB 未启用(用户没填 local_kb_root) → 本工具直接返回空数组,不报错(降级)
