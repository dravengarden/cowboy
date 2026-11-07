+++
schema = "cowboy.pattern-profile/v1"
id = "workspace.suger.observed"
scope = "workspace"
extends = "global.observed"
guidance = ["AGENTS.md", "docs/playbook/00-task-loop.md"]
disable = []

[patterns]
"delivery.procedure" = "patterns/suger-delivery.md"
+++

# Suger 历史模式配置

未启用。与 observed-global.md 一起导入，从 Suger source root
读取本 profile 的 guidance。将本文件与 patterns/suger-delivery.md
复制到 Suger 所属的配置目录，保持相对位置。

项目的审批、PR、发布和生产数据修改规则以真实 owning guidance 为准。
不要使用 Columbus 或 Cowboy 的 guidance 代替缺失的 Suger 文件。
如果路径已经变化，先找到当前权威文件再修改 guidance，不能放入空白占位。
