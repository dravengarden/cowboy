+++
schema = "cowboy.pattern/v1"
id = "work.resource-closeout"
revision = 1
role = "supervisor"
summary = "Close completed task-owned temporary resources under current policy."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Close completed task-owned temporary resources under current policy.

候选 pattern，未启用。适用前必须遵守 observed-routing.md 的共同边界及当前项目 procedure。

## 匹配条件

任务完成后仍有本任务创建、已无后续用途的临时 worker 或辅助 session，且当前规则允许关闭。

## 最小证据

读取资源所有权、用途、最新任务状态和仍在运行的 call；区分关闭执行资源、保留历史与永久删除。不能仅凭闲置时间认定可关闭。

## 监督动作

请原 worker 按当前项目的关闭流程清理可确认的临时执行资源，并验证关闭结果。只使用现有能力；本 pattern 不新增直接 session-control 权限。

## 停止条件

资源所有权不明、仍被其他任务使用、结果尚未读取或维护影响活动 session 时停止。不得删除历史、全局清理所有 idle worker 或让协调者关闭仍需工作的自身 session。

## 反例

用户打开的长期 session 不是临时垃圾；测试子会话的结果未取回时不能因测试结束而立即关闭。

## 完成证据

已授权临时资源确认结束，必要产物及历史保留；剩余资源有明确保留原因。

## 来源与成熟度

来自 2026-11-06 有界使用历史分析，见仓库文档
`docs/superagent-usage-patterns-2026-11.md`。属于待验证候选，
不是自治成功率证据；运行结果留在受控记录中，不回填原始 transcript 或业务数据。
