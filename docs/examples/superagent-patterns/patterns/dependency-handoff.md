+++
schema = "cowboy.pattern/v1"
id = "coordination.handoff"
revision = 1
role = "supervisor"
summary = "Resume an explicitly dependent task after verified handoff."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Resume an explicitly dependent task after verified handoff.

候选 pattern，未启用。适用前必须遵守 observed-routing.md 的共同边界及当前项目 procedure。

## 匹配条件

当前已参与监督的任务明确依赖另一已授权可读任务的产物，且能验证该产物已就绪；消费者仍停留在旧阻塞判断。

## 最小证据

读取显式依赖和最小产物引用：仓库、提交、接口约定、验证结果。重新确认实际产物存在且适用；另一 agent 的“完成”陈述只是线索。

## 监督动作

仅向原消费者 worker 发一次合并后的 follow-up，附确切产物和应继续的已授权步骤。生产者无需收到重复催促；不替消费者改 worktree、不抢占其工作。

## 停止条件

无法在当前能力与授权范围读取依赖、跨 Machine 不受支持、消费者暂停或依赖接口改变时停止自动派发，报告具体缺口。

## 反例

另一个 task 完成一个相似功能，并不表示它属于本任务依赖；远端分支名相同也不证明两台机器内容一致。

## 完成证据

消费者依据正确产物恢复原任务，或者提供新的真实阻塞；不维护独立于原生任务的依赖调度器。

## 来源与成熟度

来自 2026-11-06 有界使用历史分析，见仓库文档
`docs/superagent-usage-patterns-2026-11.md`。属于待验证候选，
不是自治成功率证据；运行结果留在受控记录中，不回填原始 transcript 或业务数据。
