# read_workspace_file

按真实 `document_id` 读取工作区文件。读取 `source` 时只返回 CaseBoard OCR、抽取或转换得到的派生文本，不把任意磁盘路径交给模型，也不会改动原文件；读取 `artifact` 时返回当前 Markdown 工作副本。材料尚未完成抽取、超过读取上限或 id 不属于当前工作区时必须明确失败，禁止跨工作区尝试。
