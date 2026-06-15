"""CLI 输入数据结构定义（纯 Python，无 Django 依赖）。

case_data.json 的字段定义（与法穿 court_filing_helpers._run_filing 期望的 case_data dict 对齐）。
materials.json 的格式：{slot号: [[绝对路径, 原始文件名], ...]}
"""

from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class Party:
    """当事人（原告/被告/第三人）"""
    client_type: str = "natural"  # "natural" 或 "legal"
    type: str = "natural"         # 同 client_type（法穿兼容）
    name: str = ""
    address: str = ""
    phone: str = ""
    # 自然人
    id_number: str = ""
    gender: str = ""
    # 法人
    uscc: str = ""
    legal_rep: str = ""
    legal_rep_id_number: str = ""


@dataclass
class Agent:
    """代理律师"""
    name: str = ""
    id_number: str = ""
    bar_number: str = ""
    law_firm: str = ""
    address: str = ""
    phone: str = ""


@dataclass
class CaseData:
    """立案所需案件数据（对应法穿 _run_filing 的 case_data dict）。"""
    # 基础
    court_name: str = ""
    cause_of_action: str = ""
    target_amount: str = "0"
    province: str = ""
    city: str = ""
    district: str = ""
    filing_type: str = "civil"  # "civil" 或 "execution"

    # 当事人
    plaintiffs: list[Party] = field(default_factory=list)
    defendants: list[Party] = field(default_factory=list)
    third_parties: list[Party] = field(default_factory=list)

    # 代理律师
    agents: list[Agent] = field(default_factory=list)

    # 执行立案额外字段
    original_case_number: str = ""
    execution_basis_type: str = "民商"
    execution_reason: str = ""
    execution_request: str = ""

    # 填充字段（由 CLI runner 补充）
    case_id: str = ""
    filing_engine: str = "playwright"

    def to_dict(self) -> dict:
        """转为 dict，字段名与法穿 case_data 对齐。"""
        return {
            "court_name": self.court_name,
            "cause_of_action": self.cause_of_action,
            "target_amount": self.target_amount,
            "province": self.province,
            "city": self.city,
            "district": self.district,
            "filing_type": self.filing_type,
            "plaintiffs": [self._party_to_dict(p) for p in self.plaintiffs],
            "defendants": [self._party_to_dict(p) for p in self.defendants],
            "third_parties": [self._party_to_dict(p) for p in self.third_parties],
            "agents": [self._agent_to_dict(a) for a in self.agents],
            "agent": self._agent_to_dict(self.agents[0]) if self.agents else {},
            "materials": {},  # 由外部传入（见 --materials）
            "original_case_number": self.original_case_number,
            "execution_basis_type": self.execution_basis_type,
            "execution_reason": self.execution_reason,
            "execution_request": self.execution_request,
            "case_id": self.case_id,
            "filing_engine": self.filing_engine,
        }

    @staticmethod
    def _party_to_dict(p: Party) -> dict:
        return {
            "client_type": p.client_type,
            "type": p.type or p.client_type,
            "name": p.name,
            "address": p.address,
            "phone": p.phone,
            "id_number": p.id_number,
            "gender": p.gender,
            "uscc": p.uscc,
            "legal_rep": p.legal_rep,
            "legal_rep_id_number": p.legal_rep_id_number,
        }

    @staticmethod
    def _agent_to_dict(a: Agent) -> dict:
        return {
            "name": a.name,
            "id_number": a.id_number,
            "bar_number": a.bar_number,
            "law_firm": a.law_firm,
            "address": a.address,
            "phone": a.phone,
        }


def parse_case_data_json(raw: dict) -> CaseData:
    """从原始 JSON dict 解析为 CaseData。"""
    def _parse_parties(items: list[dict]) -> list[Party]:
        parties = []
        for item in items or []:
            p = Party()
            for k, v in item.items():
                if hasattr(p, k):
                    setattr(p, k, v)
            # 兼容：client_type 缺省时按 uscc 有无推断
            if not p.client_type and p.uscc:
                p.client_type = "legal"
                p.type = "legal"
            parties.append(p)
        return parties

    def _parse_agents(items: list[dict]) -> list[Agent]:
        agents = []
        for item in items or []:
            a = Agent()
            for k, v in item.items():
                if hasattr(a, k):
                    setattr(a, k, str(v))
            agents.append(a)
        return agents

    return CaseData(
        court_name=raw.get("court_name", ""),
        cause_of_action=raw.get("cause_of_action", ""),
        target_amount=str(raw.get("target_amount", "0")),
        province=raw.get("province", ""),
        city=raw.get("city", ""),
        district=raw.get("district", ""),
        filing_type=raw.get("filing_type", "civil"),
        plaintiffs=_parse_parties(raw.get("plaintiffs", [])),
        defendants=_parse_parties(raw.get("defendants", [])),
        third_parties=_parse_parties(raw.get("third_parties", [])),
        agents=_parse_agents(raw.get("agents", [])),
        original_case_number=raw.get("original_case_number", ""),
        execution_basis_type=raw.get("execution_basis_type", "民商"),
        execution_reason=raw.get("execution_reason", ""),
        execution_request=raw.get("execution_request", ""),
        case_id=raw.get("case_id", ""),
    )


def load_case_data(path: str | Path) -> CaseData:
    """从 JSON 文件加载 CaseData。"""
    import json
    p = Path(path)
    raw = json.loads(p.read_text(encoding="utf-8"))
    return parse_case_data_json(raw)


def load_materials(path: str | Path) -> dict[str, list[tuple[str, str]]]:
    """从 JSON 文件加载材料槽位映射。

    Returns:
        {slot号: [(绝对路径, 原始文件名), ...]}
    """
    import json
    p = Path(path)
    raw = json.loads(p.read_text(encoding="utf-8"))
    result: dict[str, list[tuple[str, str]]] = {}
    for slot, items in raw.items():
        result[slot] = [(item[0], item[1]) for item in items if len(item) >= 2]
    return result


def validate_case_data(data: CaseData) -> list[str]:
    """校验 CaseData 必填字段，返回错误列表。"""
    errors: list[str] = []
    if not data.court_name:
        errors.append("court_name 不能为空")
    if not data.cause_of_action:
        errors.append("cause_of_action 不能为空")
    if not data.plaintiffs and not data.defendants:
        errors.append("plaintiffs 和 defendants 至少有一方不为空")
    for i, p in enumerate(data.plaintiffs + data.defendants):
        if not p.name:
            errors.append(f"当事人第 {i+1} 条 name 不能为空")
    if data.filing_type == "execution" and not data.original_case_number:
        errors.append("执行立案 original_case_number 不能为空")
    return errors
