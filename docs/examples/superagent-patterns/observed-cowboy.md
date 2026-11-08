+++
schema = "cowboy.pattern-profile/v1"
id = "project.cowboy.observed"
scope = "project"
extends = "workspace.columbus.observed"
guidance = ["AGENTS.md"]
disable = []

[replace."delivery.procedure"]
from = "workspace.columbus.observed/delivery.procedure@1"
file = "patterns/cowboy-delivery.md"
+++

# Cowboy 历史模式配置

未启用。与 observed-global.md、observed-columbus.md 一起导入；
各 source root 分别属于全局库、Columbus 和 Cowboy。
本文件可以从 Cowboy 仓库当前位置打包，guidance 使用 Cowboy AGENTS.md。

替换引用显式绑定 Columbus observed procedure 的 revision 1。
若上游 procedure 升版，先审核变化再更新 from，不能自动接受未知新规则。
