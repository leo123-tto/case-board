from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path


def load_party_info_handler_module():
    root = Path(__file__).resolve().parents[1]
    module_path = root / "sites" / "court_zxfw_filing" / "party_info_handler.py"

    for name in [
        "court_filing_cli",
        "court_filing_cli.sites",
        "court_filing_cli.sites.court_zxfw_filing",
    ]:
        pkg = sys.modules.setdefault(name, types.ModuleType(name))
        if not hasattr(pkg, "__path__"):
            pkg.__path__ = []  # type: ignore[attr-defined]

    playwright = sys.modules.setdefault("playwright", types.ModuleType("playwright"))
    sync_api = sys.modules.setdefault("playwright.sync_api", types.ModuleType("playwright.sync_api"))
    sync_api.Page = object
    playwright.sync_api = sync_api

    form_utils_name = "court_filing_cli.sites.court_zxfw_filing.form_utils"
    form_utils = types.ModuleType(form_utils_name)

    class FormUtilsMixin:
        pass

    form_utils.FormUtilsMixin = FormUtilsMixin
    sys.modules[form_utils_name] = form_utils

    spec = importlib.util.spec_from_file_location(
        "court_filing_cli.sites.court_zxfw_filing.party_info_handler",
        module_path,
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


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


class PartyInfoHandlerTest(unittest.TestCase):
    def test_opponent_parties_do_not_fallback_to_our_agent_phone(self):
        module = load_party_info_handler_module()

        class Handler(module.PartyInfoHandlerMixin):
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

        handler = Handler()
        handler._step5_complete_info(
            {
                "agents": [{"phone": "13900001111"}],
                "plaintiffs": [{"name": "原告公司", "client_type": "legal"}],
                "defendants": [{"name": "被告公司", "client_type": "legal"}],
                "third_parties": [{"name": "第三人公司", "client_type": "legal"}],
            }
        )

        by_section = {call["section_title"]: call for call in handler.calls}
        self.assertEqual(by_section["原告信息"]["party_phone"], "13900001111")
        self.assertEqual(by_section["原告信息"]["agent_phone"], "13900001111")
        self.assertEqual(by_section["被告信息"]["party_phone"], "")
        self.assertEqual(by_section["被告信息"]["agent_phone"], "")
        self.assertEqual(by_section["第三人信息"]["party_phone"], "")
        self.assertEqual(by_section["第三人信息"]["agent_phone"], "")


if __name__ == "__main__":
    unittest.main()
