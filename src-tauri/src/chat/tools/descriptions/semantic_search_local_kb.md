semantic_search_local_kb — 对作者本地法律知识库的 **raw 完整正文**做语义检索(embedding 向量,非关键词)

用途(本地 BM25 弱命中后的**第二跳**,比元典省):
- 找法条:整部法律(民法典、民诉法等)已在本地、**按法条切片建了向量索引**,用自然语言描述要找的内容(「合同解除的法定情形」「债权人代位权」),直接命中最相关的**那几条**条文 —— 命中后用 `read_kb_file(relative_path)` 拿全文条文,**0 元典积分**
- 找类案 / 原始实务材料 / 作者原始笔记:按语义找相关完整正文,而不是死扣关键词
- 向量语料严格限定为 `raw/notes/` 与 `raw/yuandian-cache/` 中法规/法条/案例完整详情。`wiki/sources`、`wiki/topics`、企业档案、办案经验卡、自建导航目录、`_inbox`/`00_ARCHIVE` 都不切向量
- 跟关键词工具 `search_local_kb` 互补：统一先跑本地 BM25；`weak_hits=true`、描述型问题或前排跑题时，再用本工具按语义补检

local-first:法规目录/精确标识 → Wiki 导航卡 → raw BM25 → raw 向量 → 元典。任一本地层强命中且够用就停；准确性确需补充法源时可以继续外查，不设固定付费次数上限。

输入:
- query: 必填,自然语言完整描述想找的内容(别只写一个词)
- top_n: 可选,返回最相关的几个片段,默认 6,最大 12

返回:`[{relative_path, score, excerpt}]`
- relative_path:KB 内相对路径(如 `raw/yuandian-cache/法规-xxx_中华人民共和国民法典.md`),用它 `read_kb_file` 拿对应段落全文
- score:余弦相似度(越高越相关)；`< 0.70` 只视为弱线索，必须回到原文核验，不能据此拦截外查
- excerpt:命中片段(整部法律按条切,通常就是对应法条原文)

注意:
- **不消耗元典积分**，但会调用用户配置的 embedding 服务并可能消耗该服务额度
- 引用本地内容必须加 `<CITATIONS>`:type=`"kb_local"`,source=relative_path,title=文件名(去 .md)
- 若用户没配置 embedding(返回提示)→ 改用 `search_local_kb` 关键词工具,不要反复调本工具
- 命中后**只读相关段落**(`read_kb_file` 带 offset/length),别把整部 334K 大法读进来
