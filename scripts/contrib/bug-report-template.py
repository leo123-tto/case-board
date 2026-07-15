#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""BUG 报告模板生成器

用法:
  python bug-report-template.py > BUG-001.md      # 输出到文件
  python bug-report-template.py | pbcopy          # macOS 复制到剪贴板
  python bug-report-template.py --id BUG-001      # 指定 ID

填写完模板后,可以直接在 GitHub issues 粘贴(用 .github/ISSUE_TEMPLATE/bug_report.yml 的格式)
"""

import argparse
import datetime
import sys

TEMPLATE = """## BUG-{id}

**发现日期**: {date}
**CaseBoard 版本**: v?.?.?
**操作系统**: macOS ?.?  /  Windows ?.?
**优先级**: P0 / P1 / P2 / P3

### 现象

[你看到了什么,具体描述]

### 预期

[应该看到什么]

### 复现步骤

1. 打开 App 版本 v?.?.?
2. 走到 ... 步骤
3. 触发 ... 操作
4. 看到 ... 现象

### 复现频率

- [ ] 100% 必现
- [ ] 经常(> 50%)
- [ ] 偶发(10-50%)
- [ ] 罕见(< 10%)

### 日志 / 报错堆栈

```
[粘贴关键日志,记得脱敏]
```

### 截图 / 录像

[不附图,用文字描述 UI 行为]

### 排查

[你做过的排查,比如:看了哪个文件、跑了什么命令、怀疑哪个模块]

### 建议修法(可选)

[如果已经有想法,可以写下来,方便 reviewer 评估]

---

**隐私检查清单**:
- [ ] 不含真实当事人姓名、案号、身份证号、电话、地址
- [ ] 不含真实案件文书内容或截图
- [ ] 文件路径已脱敏(不含真实案件子目录)
"""


def main():
    parser = argparse.ArgumentParser(description="BUG 报告模板生成器")
    parser.add_argument("--id", default="001", help="BUG 编号,默认 001")
    args = parser.parse_args()

    today = datetime.date.today().isoformat()
    out = TEMPLATE.format(id=args.id, date=today)
    print(out)


if __name__ == "__main__":
    main()
