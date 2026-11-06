+++
schema = "cowboy.pattern-profile/v1"
id = "global.observed"
scope = "global"
guidance = ["docs/examples/superagent-patterns/observed-routing.md"]
disable = []

[patterns]
"delivery.follow-through" = "patterns/follow-through.md"
"work.scope-continuation" = "patterns/scope-continuation.md"
"work.failure-recovery" = "patterns/failure-recovery.md"
"review.follow-through" = "patterns/review-follow-through.md"
"delivery.evidence" = "patterns/delivery-evidence.md"
"coordination.handoff" = "patterns/dependency-handoff.md"
"review.ready-evidence" = "patterns/review-ready.md"
"work.resource-closeout" = "patterns/resource-closeout.md"
+++

# 历史使用特征候选集

未安装、未启用。保留现有 global.md 示例不变，本文件是可选替代根 profile。
本库共八个 supervisor：沿用通用交付收尾，加七个有明确触发证据的细分模式。
先按项目选择有价值的少量模式，不把全部匹配变成八条催促。

采用时将文件复制到 owner 管理的 source，保持上述相对结构，或修改 guidance 路径。
workspace profile 的 extends 改为 global.observed，并提供现有 delivery.procedure；
Columbus 与 Suger 各自使用自己的 guidance/source root，不从 Cowboy 借用项目权限。
Cowboy 项目可继续显式替换 Columbus procedure。按实际来源一起打包验证，
再经既有配置入口导入；本文件独立缺少 procedure，不能视为完整可运行配置。
模型、账户、参与模式、学习模式与 follow-up 上限仍由 Cowboy DB 配置。
