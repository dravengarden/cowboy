+++
schema = "cowboy.pattern/v1"
id = "work.scope-continuation"
revision = 1
role = "supervisor"
summary = "Resume remaining accepted scope after a partial batch."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Resume remaining accepted scope after a partial batch.

候选 pattern，未启用。适用前必须遵守 observed-routing.md 的共同边界及当前项目 procedure。

## 匹配条件

原任务仍有明确未完成项，原 worker 在完成一个批次后结束回合；没有真实外部依赖阻止下一步。

## 最小证据

读取当前任务的接受范围、用户指定来源 session 的必要片段、最近完成项及阻塞证据。不能只看当前 worktree，也不能把历史愿望全部变成本次范围。

## 监督动作

请原 worker 继续一个具体、已授权且可执行的剩余步骤；保留原生任务状态，不建立第二份任务队列。若原文声称全部完成而仍列有未完成项，要求纠正完成结论并继续。

## 停止条件

范围已经完成、用户仅要求调研、剩余项需要新决策或外部输入时停止；能独立推进的已授权准备工作可以继续。

## 反例

用户说“先完成这一批”时，结束该批是正确行为；某功能缺少安全测试环境时不能以继续为由操作生产。

## 完成证据

任务整体范围与完成证据一致；否则清楚列出真实阻塞及已完成的准备。

## 来源与成熟度

来自 2026-11-06 有界使用历史分析，见仓库文档
`docs/superagent-usage-patterns-2026-11.md`。属于待验证候选，
不是自治成功率证据；运行结果留在受控记录中，不回填原始 transcript 或业务数据。
