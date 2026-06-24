from court_filing_cli.sites.court_zxfw_filing.party_info_handler import PartyInfoHandlerMixin


class _FirstLocator:
    def wait_for(self, **_kwargs):
        return None


class _Locator:
    @property
    def first(self):
        return _FirstLocator()


class _Page:
    def locator(self, _selector):
        return _Locator()


class _Handler(PartyInfoHandlerMixin):
    CIVIL_SECTION_MAP = {
        "plaintiffs": "原告信息",
        "defendants": "被告信息",
        "third_parties": "第三人信息",
    }
    EXEC_SECTION_MAP = {}

    def __init__(self):
        self.page = _Page()
        self.calls = []

    def _clear_auto_recognized_parties(self):
        return None

    def _complete_agent_info(self, _case_data):
        return None

    def _add_party_by_type(self, **kwargs):
        self.calls.append(kwargs)


def test_defendant_without_phone_does_not_fallback_to_agent_phone():
    handler = _Handler()

    handler._step5_complete_info(
        {
            "agents": [{"phone": "13900001111"}],
            "plaintiffs": [{"name": "原告公司", "client_type": "legal"}],
            "defendants": [{"name": "被告公司", "client_type": "legal"}],
        }
    )

    plaintiff_call = next(call for call in handler.calls if call["section_title"] == "原告信息")
    defendant_call = next(call for call in handler.calls if call["section_title"] == "被告信息")

    assert plaintiff_call["party_phone"] == "13900001111"
    assert plaintiff_call["agent_phone"] == "13900001111"
    assert defendant_call["party_phone"] == ""
    assert defendant_call["agent_phone"] == ""
