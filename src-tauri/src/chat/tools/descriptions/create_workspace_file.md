# create_workspace_file

在当前派生工作区新建可编辑 Markdown 文稿，适用于用户明确要求保存报告、整理成果或输出文件。完整正文通过 `markdown` 传入，标题通过 `title` 传入；成功后返回真实 `document_id`。文件只能写入 CaseBoard AppData 并登记到数据库。不得在未调用成功时声称已经保存，也不得用它覆盖原始材料。
