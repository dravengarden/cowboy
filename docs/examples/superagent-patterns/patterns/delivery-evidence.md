+++
schema = "cowboy.pattern/v1"
id = "delivery.evidence"
revision = 1
role = "supervisor"
summary = "Reconcile completion claims with exact delivery evidence."
events = ["turn.completed"]
uses = ["delivery.procedure"]
effects = ["session.follow_up"]
+++

# Reconcile completion claims with exact delivery evidence.

候选 pattern，未启用。适用前必须遵守 observed-routing.md 的共同边界及当前项目 procedure。

## 匹配条件

完成结论与现有证据矛盾，或用户还需要追问已授权的 push、deploy、测试是否真正完成。

## 最小证据

按任务要求核对精确 SHA、测试运行、远端引用、目标组件和部署回执。采用新鲜必要证据，不查询无关机器；提交受理、构建成功和激活成功是不同事实。

## 监督动作

请原 worker 补齐缺失的已授权步骤，或把已有证据清楚呈现为改了什么、验证了什么、发布到了哪里、仍有哪些限制。已有对应版本的有效测试无需仅为安抚而重跑。

## 停止条件

研究或文档任务没有上线目标时不部署；历史 push/deploy 请求不是本次授权。发现未知效果时只观察原操作。

## 反例

用户问“E2E 测过了吗”可能是在确认已完成测试；本地绿色检查不能证明生产已更新，旧日志不能证明本次测试执行。

## 完成证据

声明中的每个交付阶段都有对应版本和目标的证据，未覆盖项明确；不把部分交付包装成全任务完成。

## 来源与成熟度

来自 2026-11-06 有界使用历史分析，见仓库文档
`docs/superagent-usage-patterns-2026-11.md`。属于待验证候选，
不是自治成功率证据；运行结果留在受控记录中，不回填原始 transcript 或业务数据。
