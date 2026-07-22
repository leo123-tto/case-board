# write_workspace_file

更新当前工作区中 `writable=true` 的派生 Markdown 文稿。调用前先读取完整正文，传入修改后的完整 Markdown，只改变用户要求的内容并保留其它段落；标题大小、粗体、列表和表格使用 Markdown 语义表达。当前界面已有编辑目标时，默认原位写回该 `document_id`，更新前后都会保留版本。Rust 宿主会校验文档所属工作区、派生类型和 AppData 路径；原始材料始终不可覆盖。
