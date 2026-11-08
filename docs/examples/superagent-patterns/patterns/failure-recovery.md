+++
schema = "cowboy.pattern/v1"
id = "work.failure-recovery"
revision = 1
role = "supervisor"
summary = "Diagnose and repair failed authorized work before stopping."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Diagnose and repair failed authorized work before stopping.

候选 pattern，未启用。适用前必须遵守 observed-routing.md 的共同边界及当前项目 procedure。

## 匹配条件

工具、构建或验收明确失败，原 worker 只报告失败，且诊断或修复仍在当前范围内。

## 最小证据

读取失败命令的真实退出码、该次日志和副作用回执。先区分未执行、明确失败和结果未知；管道尾部成功不是前面命令成功。

## 监督动作

请原 worker 找根因、做最小修复并验证受影响路径。若操作结果未知，只查询原 operation 的权威回执；不得换 ID 重放。证据确认未派发时才按已有恢复契约处理。

## 停止条件

相同原因重复失败且没有新证据、权限不足或预算耗尽时停止；归纳具体阻塞。不得通过跳过质量门禁、降低测试标准或扩大权限解围。

## 反例

网络超时可能发生在写入成功之后；失败的部署请求不能直接重试。测试环境故障也不证明产品代码有缺陷。

## 完成证据

修复有针对性验证，原失败步骤状态已明确，随后返回原任务的交付流程。

## 来源与成熟度

来自 2026-11-06 有界使用历史分析，见仓库文档
`docs/superagent-usage-patterns-2026-11.md`。属于待验证候选，
不是自治成功率证据；运行结果留在受控记录中，不回填原始 transcript 或业务数据。
