-- ============================================================================
-- 0036 · 要素式审判智能辅助
--
-- 设计说明:
--   - element_templates: 案由×起诉/答辩 要素模板,预置 67 类案由×2 方向=134 条基准
--   - element_facts: 具体案件的要件事实逐条归依(证据→要件→争点)
--   - trial_strategies: 攻防策略记录(三级递进:主张责任→证明责任→举证行为)
--   - element_complaints: AI 生成的要素式起诉状/答辩状草稿
--
-- 关联: cases.id(FK CASCADE) → element_facts / trial_strategies / element_complaints
-- ============================================================================

PRAGMA foreign_keys = ON;

-- ----------------------------------------------------------------------------
-- 案由要素模板表(全局预置,不关联具体案件)
-- ----------------------------------------------------------------------------
CREATE TABLE element_templates (
    id              TEXT PRIMARY KEY NOT NULL,
    cause           TEXT NOT NULL,              -- 案由,如"机动车交通事故责任纠纷"
    direction       TEXT NOT NULL,              -- 起诉 / 答辩
    element_name    TEXT NOT NULL,              -- 要素名称,如"事故经过"
    element_desc    TEXT NOT NULL,              -- 要素描述
    is_required     INTEGER NOT NULL DEFAULT 1, -- 是否必备事实(bool)
    evidence_type   TEXT,                       -- 建议证据类型
    evidence_hint   TEXT,                       -- 证据说明
    burden_party    TEXT,                       -- 举证责任方(原告/被告)
    sort_order      INTEGER NOT NULL DEFAULT 0, -- 排序
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_element_templates_cause ON element_templates(cause, direction);

-- ----------------------------------------------------------------------------
-- 要件事实表(关联具体案件,逐条记录要件归依)
-- ----------------------------------------------------------------------------
CREATE TABLE element_facts (
    id                TEXT PRIMARY KEY NOT NULL,
    case_id           TEXT NOT NULL,
    stage             TEXT,                     -- 关联阶段(立案/一审/二审等)
    template_id       TEXT,                     -- → element_templates.id(可选)
    fact_name         TEXT NOT NULL,            -- 要件名称
    fact_desc         TEXT,                     -- 要件描述
    claim_party       TEXT,                     -- 主张方
    evidence_ids      TEXT,                     -- JSON 数组:关联的证据 document ids
    proof_status      TEXT DEFAULT 'pending',   -- pending/已举证/待补证/举证不能
    opponent_rebuttal TEXT,                     -- 对方抗辩
    court_finding     TEXT,                     -- 法院认定
    is_established    INTEGER,                  -- 成立与否(NULL=待定,1=成立,0=不成立)
    is_disputed       INTEGER NOT NULL DEFAULT 0, -- 是否形成争点(bool)
    notes             TEXT,
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE,
    FOREIGN KEY (template_id) REFERENCES element_templates(id) ON DELETE SET NULL
);
CREATE INDEX idx_element_facts_case ON element_facts(case_id);
CREATE INDEX idx_element_facts_disputed ON element_facts(case_id, is_disputed);

-- ----------------------------------------------------------------------------
-- 攻防策略记录表
-- ----------------------------------------------------------------------------
CREATE TABLE trial_strategies (
    id                TEXT PRIMARY KEY NOT NULL,
    case_id           TEXT NOT NULL,
    stage             TEXT,                     -- 关联阶段
    strategy_layer    TEXT NOT NULL,            -- 主张责任/证明责任/举证行为
    strategy_content  TEXT NOT NULL,            -- 策略内容(Markdown)
    target_fact_ids   TEXT,                     -- JSON 数组:关联的要件事实 ids
    predicted_opponent_strategy TEXT,           -- 对方抗辩预判
    evidence_gap_analysis    TEXT,              -- 证据链缺口分析
    recommended_actions      TEXT,              -- 建议取证/补充动作
    risk_level        TEXT,                     -- 风险等级(高/中/低)
    is_adopted        INTEGER NOT NULL DEFAULT 0, -- 是否已采纳
    notes             TEXT,
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE
);
CREATE INDEX idx_trial_strategies_case ON trial_strategies(case_id, stage);

-- ----------------------------------------------------------------------------
-- 要素式文书表(AI 生成的起诉状/答辩状草稿)
-- ----------------------------------------------------------------------------
CREATE TABLE element_complaints (
    id                TEXT PRIMARY KEY NOT NULL,
    case_id           TEXT NOT NULL,
    doc_type          TEXT NOT NULL,            -- 起诉状 / 答辩状
    direction         TEXT NOT NULL,            -- 原告视角 / 被告视角
    content_md        TEXT NOT NULL,            -- 文书正文(Markdown)
    filled_elements   TEXT,                     -- JSON:已填充的要素列表[{name,value}]
    ai_model          TEXT,                     -- 使用的 AI 模型
    generation_prompt TEXT,                     -- 生成时的 prompt(用于调优)
    version           INTEGER NOT NULL DEFAULT 1,
    is_final          INTEGER NOT NULL DEFAULT 0, -- 是否终稿
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (case_id) REFERENCES cases(id) ON DELETE CASCADE
);
CREATE INDEX idx_element_complaints_case ON element_complaints(case_id, doc_type);

-- ============================================================================
-- 预置要素模板(交通事故 — 起诉方向,9 项)
-- ============================================================================
INSERT INTO element_templates (id, cause, direction, element_name, element_desc, is_required, evidence_type, evidence_hint, burden_party, sort_order) VALUES
('et-traf-001', '机动车交通事故责任纠纷', '起诉', '事故经过', '事故发生的时间/地点/车辆/碰撞形态', 1, '书证', '事故认定书', '原告', 1),
('et-traf-002', '机动车交通事故责任纠纷', '起诉', '损害后果', '受害人伤情/车辆损失', 1, '书证', '病历/医疗费票据/鉴定报告/维修清单', '原告', 2),
('et-traf-003', '机动车交通事故责任纠纷', '起诉', '责任划分', '各方责任比例', 1, '书证', '事故认定书/法院判决', '原告', 3),
('et-traf-004', '机动车交通事故责任纠纷', '起诉', '保险情况', '交强险/商业险保单信息', 1, '书证', '保单/保险凭证', '原告', 4),
('et-traf-005', '机动车交通事故责任纠纷', '起诉', '赔偿项目', '医疗费/误工费/护理费/残疾赔偿金等20项', 1, '书证', '医疗票据/工资证明/鉴定报告', '原告', 5),
('et-traf-006', '机动车交通事故责任纠纷', '起诉', '车辆与驾驶资质', '肇事车辆信息/驾驶人驾驶证', 1, '书证', '行驶证/驾驶证', '原告', 6),
('et-traf-007', '机动车交通事故责任纠纷', '起诉', '受害人身份', '受害人户口性质/职业/收入', 1, '书证', '户口本/劳动合同/工资流水', '原告', 7),
('et-traf-008', '机动车交通事故责任纠纷', '起诉', '治疗经过', '住院天数/手术/康复情况', 1, '书证', '出院小结/诊断证明', '原告', 8),
('et-traf-009', '机动车交通事故责任纠纷', '起诉', '垫付情况', '被告/保险公司已垫付金额', 0, '书证', '转账记录/收条', '原告', 9);

-- 预置要素模板(交通事故 — 答辩方向,5 项)
INSERT INTO element_templates (id, cause, direction, element_name, element_desc, is_required, evidence_type, evidence_hint, burden_party, sort_order) VALUES
('et-traf-d01', '机动车交通事故责任纠纷', '答辩', '责任争议', '对事故认定书认定的责任有异议', 0, '书证/视听资料', '行车记录仪/现场照片/证人证言', '被告', 1),
('et-traf-d02', '机动车交通事故责任纠纷', '答辩', '赔偿项目争议', '对部分赔偿项目的合理性有异议', 0, '书证', '医保外费用说明/误工费异议', '被告', 2),
('et-traf-d03', '机动车交通事故责任纠纷', '答辩', '保险免责', '保险公司主张免责或减责条款', 0, '书证', '保险条款/投保单/免责告知书', '被告(保险公司)', 3),
('et-traf-d04', '机动车交通事故责任纠纷', '答辩', '受害人过错', '受害人自身存在过错(如闯红灯)', 0, '书证/视听资料', '监控录像/证人证言', '被告', 4),
('et-traf-d05', '机动车交通事故责任纠纷', '答辩', '已赔付', '已支付的赔偿金额及项目明细', 0, '书证', '转账记录/收条/调解协议', '被告', 5);

-- 预置要素模板(网络服务合同纠纷 — 起诉方向,4 项)
INSERT INTO element_templates (id, cause, direction, element_name, element_desc, is_required, evidence_type, evidence_hint, burden_party, sort_order) VALUES
('et-net-001', '网络服务合同纠纷', '起诉', '合同关系成立', '双方存在网络服务合同关系', 1, '书证', '平台服务协议/入驻协议', '原告', 1),
('et-net-002', '网络服务合同纠纷', '起诉', '违约行为', '被告存在违约行为(如无故下架/扣款)', 1, '电子数据', '后台处罚记录/扣款通知', '原告', 2),
('et-net-003', '网络服务合同纠纷', '起诉', '损失后果', '违约行为给原告造成的损失金额', 1, '书证/电子数据', '财务报表/交易记录', '原告', 3),
('et-net-004', '网络服务合同纠纷', '起诉', '因果关系', '违约行为与损失之间的因果关系', 1, '书证', '时间线对应关系说明', '原告', 4);

-- 预置要素模板(网络服务合同纠纷 — 答辩方向,3 项)
INSERT INTO element_templates (id, cause, direction, element_name, element_desc, is_required, evidence_type, evidence_hint, burden_party, sort_order) VALUES
('et-net-d01', '网络服务合同纠纷', '答辩', '违约行为不存在', '处罚符合平台规则', 1, '电子数据', '平台规则/违约证据截图', '被告', 1),
('et-net-d02', '网络服务合同纠纷', '答辩', '损失不成立', '原告主张的损失金额缺乏依据', 0, '书证', '财务核算说明', '被告', 2),
('et-net-d03', '网络服务合同纠纷', '答辩', '对方违约在先', '原告自身存在违约行为', 0, '电子数据', '平台通知/催促记录', '被告', 3);
