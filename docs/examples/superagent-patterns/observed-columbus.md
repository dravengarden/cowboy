+++
schema = "cowboy.pattern-profile/v1"
id = "workspace.columbus.observed"
scope = "workspace"
extends = "global.observed"
guidance = ["AGENTS.md", "docs/requirements.md"]
disable = []

[patterns]
"delivery.procedure" = "patterns/columbus-delivery.md"
+++

# Columbus 历史模式配置

未启用。与 observed-global.md 一起导入，从 Columbus source root
读取本 profile 的 guidance。将本文件与 patterns/columbus-delivery.md
复制到 Columbus 所属的配置目录，保持相对位置。

保留完整候选集用于观察。若要减少范围，可以在本 profile 的 disable
中列出继承的 pattern ID；disable 的对象必须确实存在。
此文件不选择 coordinator、不报名 worker，也不授予机器维护权限。
