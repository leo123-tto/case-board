#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""新需求模板生成器

用法:
  python feature-request-template.py > FEAT-001.md
  python feature-request-template.py --id FEAT-005

填写完模板后,在 GitHub issues 粘贴(用 .github/ISSUE_TEMPLATE/feature_request.yml 的格式)
"""

import argparse
import datetime
import sys

TEMPLATE = """## {id}

**创建日期**: {date}
**优先级**: P0 / P1 / P2 / P3
**目标版本**: v?.?.?

### 用户故事

作为 [律师/律所助手/...]
我希望 [做什么]
以便 [达到什么目的]

### 背景

[为什么需要这个功能,解决了什么痛点]

### 期望行为

[具体描述:用户操作 → 系统响应]

#### 主流程

1. 用户 ...
2. 系统 ...
3. ...

#### 边界情况

- 如果 ... 则 ...
- 如果 ... 则 ...

### API / Schema 变化

[如果有,详细列出;没有就写"无"]

### 替代方案(可选)

[如果考虑过其它方案,简单说一下为什么选这个]

### 风险点

- [风险 1]
- [风险 2]

### 测试策略

- 单元测试:[覆盖哪些场景]
- 端到端测试:[怎么验证]
- 兼容性:[对老用户的影响]

### 设计稿(可选,复杂需求必须)

详见 `docs/design/{id}-<short>.md`

---

**检查清单**:
- [ ] 已在 upstream 提 issue 讨论(大改动必做)
- [ ] 不破坏现有 schema / 公共 API(若破坏,需写迁移指南)
- [ ] 不含真实当事人 / 案件数据
- [ ] 已在本地 `pnpm tauri dev` 验证
- [ ] 已在 `docs/FEATURE_BACKLOG.md` 登记
"""


def main():
    parser = argparse.ArgumentParser(description="新需求模板生成器")
    parser.add_argument("--id", default="FEAT-001", help="需求编号")
    args = parser.parse_args()

    today = datetime.date.today().isoformat()
    out = TEMPLATE.format(id=args.id, date=today)
    print(out)


if __name__ == "__main__":
    main()
