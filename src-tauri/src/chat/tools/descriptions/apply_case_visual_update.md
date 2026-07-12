把用户本轮已经明确选择或主动要求的可视化修改直接应用到现有案情工作台。调用前必须先用 get_case_visualization 读取当前 revision 和稳定 id；patch.base_revision 必须等于当前 revision。

本工具只用于明确授权后的写入，例如用户在 ask_user 多选中选定图表，或用户在工作台点击「让 AI 调整」并提出具体要求。此时不要再调用 propose_case_visual_update，也不要要求用户审核节点、关系、数据集或 UUID。

只提交完成用户目标所需的最小 patch。人工锁定字段由后端保护；关键事实继续绑定真实材料来源并区分确认事实、双方主张、争议、推断和未知。成功后修改立即进入工作台，同时形成新 revision，用户可以撤销。

更新已有节点、关系、数据集和视图时必须复用 get_case_visualization 返回的稳定 id，不得为同一对象另造重复 id。仅调整方向、排序、标签等展示方式时，只更新目标 view 的受控 config，不要重写事实节点；没有用户明确要求时不要删除现有内容，也不要改写 layout 中的人工位置。若工具提示 revision 已变化，重新读取最新工作区并基于新 revision 生成一次最小 patch，不要反复提交旧补丁。
