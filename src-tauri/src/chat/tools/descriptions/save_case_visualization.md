为当前案件首次创建“案情可视化工作区”。本工具不是普通文本输出，也不是把 Mermaid、HTML、SVG 或任意 ECharts 配置塞进聊天，而是保存严格受控的 CaseGraph 语义结构，之后由 CaseBoard 的确定性渲染器画成时间线、主体与法律关系图、请求权基础思维导图、证据矩阵、量化图表或数据条表格。

调用前必须满足其一：用户本轮明确要求画图、生成时间线、关系图、思维导图、证据矩阵或其他可视化；或者你先通过一次多选 ask_user 提出全部合适视图，用户已选择同意。未经同意绝对不要调用。用户已明确要求画图时不要再次追问。用户选择“暂不生成”时不得调用。

创建前先读本案材料并区分事实状态。confirmed 只用于材料能够确认的内容；our_claim、opponent_claim、disputed、inferred、unknown 不得混写成确定事实。关键 confirmed 节点必须附 source_refs，document_id 和 filename 必须来自本案真实材料，locator 写页码、段落或可定位位置，quote 只放必要短摘录。日期只有确切到日时才能写 date；月份、期间或约数写 date_label，禁止为了排版补造具体日期。

当前案件由系统上下文自动绑定，不要在 graph 里填写、猜测或复述 case_id。节点和关系 id 使用新生成的稳定 UUID，后续更新必须复用，不得按标题或日期重新计算。只使用白名单 node kind、edge kind、view kind 和 status。不得输出 HTML、脚本、远程 URL、图片地址或任意渲染器 option。layout 通常传空对象，由应用内 ELK/原生布局生成。

本工具只用于首次创建。若本案已经有工作区，不要重建或覆盖，应先读取现有结构；用户已在多选中选择视图或主动要求修改时，调用 apply_case_visual_update 直接合并。工具返回工作区 id、修订号和视图摘要后，只需告诉用户可进入可视化工作台查看、核对来源和继续编辑。
