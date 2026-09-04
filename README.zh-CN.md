<p align="right">
  <a href="README.md">English</a> · <strong>简体中文</strong>
</p>

<p align="center">
  <a href="https://dravengarden.github.io/cowboy/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="site/assets/cowboy-readme-mark-dark-v2.png">
      <img src="site/assets/cowboy-readme-mark-light-v2.png" width="180" height="101" alt="Cowboy">
    </picture>
  </a>
</p>

<h1 align="center">Cowboy</h1>

<p align="center">
  <strong>在你的机器上运行编码 Agent，随处控制。</strong><br>
  面向持久任务的自托管远程 Agent IDE，覆盖桌面端、移动端和你掌控的每台 Machine。
</p>

<p align="center">
  <a href="https://dravengarden.github.io/cowboy/">
    <img src="https://img.shields.io/badge/website-live-6e56cf?style=flat-square" alt="Cowboy 官网">
  </a>
  <a href="https://github.com/dravengarden/cowboy/actions/workflows/website.yml">
    <img src="https://github.com/dravengarden/cowboy/actions/workflows/website.yml/badge.svg" alt="官网构建状态">
  </a>
  <a href="LICENSE">
    <img src="https://img.shields.io/badge/license-MIT-7c5cbf?style=flat-square" alt="MIT 许可证">
  </a>
  <a href="https://agentclientprotocol.com/">
    <img src="https://img.shields.io/badge/protocol-ACP_native-4a90d9?style=flat-square" alt="原生支持 ACP">
  </a>
</p>

<p align="center">
  <a href="https://dravengarden.github.io/cowboy/"><strong>官网</strong></a>
  · <a href="#快速开始">快速开始</a>
  · <a href="#架构">架构</a>
  · <a href="#插件生态">插件</a>
  · <a href="docs/INDEX.md">文档</a>
</p>

<p align="center">
  <a href="https://dravengarden.github.io/cowboy/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="site/assets/cowboy-remote-topology-dark-v3.webp">
      <img src="site/assets/cowboy-remote-topology-light-v3.webp" alt="Cowboy Desktop 与 Mobile 通过一个自托管 Hub 连接三台通用 macOS 和 Linux Machine，并运行多个编码 Agent" width="1100">
    </picture>
  </a>
</p>

<p align="center"><sub>客户端负责控制，Hub 负责记忆，你的 Machine 负责执行。</sub></p>

Cowboy 让长时间运行的编码 Agent 始终贴近实际工作。自托管一个 Hub，加入
Linux 和 macOS Machine，再从键盘优先的桌面工作区或触控优先的移动工作区调度
Codex、Claude Code、Gemini、Grok、DeepSeek 或你自己的插件。稍后重新连接时，
无需把 worktree、凭据或 Agent 进程移出它最初运行的 Machine。

> [!IMPORTANT]
> Cowboy 正在积极开发，尚未达到 1.0，并以源码使用为主。目前没有稳定的二进制
> 发布；请从源码构建，并预期接口仍会继续演进。

## 为什么选择 Cowboy

- **一个 Hub，多台 Machine。** 将 Session 放到任意已加入的主机上，无需切换
  IDE 或来回管理 SSH tab，即可在不同 Machine 之间切换。
- **客户端断开，worker 仍继续。** 关闭浏览器或重启 Controller 不会停止由
  Machine 持有的独立 ACP worker。
- **Agent 工作过程始终清晰。** Plan、工具调用、权限、代码、diff、队列和 runtime
  状态都保持可见，而不是被压缩成一个普通聊天框。
- **每种界面适配自己的输入方式。** Desktop 信息密度高并以键盘/Vim 为先；
  Mobile 以触控为先，按需逐步呈现功能。
- **插件是发布单元，不是脚本。** 已签名、不可变、以 Machine 为范围的 generation
  会经过暂存、探测、启用、drain 和显式回滚。
- **控制平面属于你。** Hub、数据库、Machine、凭据和发布策略都运行在你控制的
  基础设施上；Cowboy 不提供共享云服务。

## 快速开始

### 先决条件

- 已启用 flakes 的 [Nix](https://nixos.org/download/)
- Linux 或 macOS
- Git

克隆 Cowboy，进入固定版本的开发环境，安装当前 checkout 的前端依赖并构建发布：

```sh
git clone https://github.com/dravengarden/cowboy.git
cd cowboy
nix develop
just install
just build
```

使用 SQLite 持久化启动本地 Hub：

```sh
./target/release/cowboy serve \
  --database-url sqlite:///tmp/cowboy.sqlite3
```

打开 <http://127.0.0.1:3333>。本地开发默认关闭产品登录。SQLite 是零运维的
存储方案；规模更大的部署可以通过同一个 Store API 使用 PostgreSQL。

如需加入另一台 Linux 或 macOS 主机，请运行
<code>just build-machine-bootstrap</code> 构建目标平台的 bootstrap。在 Cowboy 中
创建一次性 enrollment code，然后在那台 Machine 上运行生成的命令：

```sh
cowboy register https://cowboy.example --background
```

Token 通过掩码提示输入，无需出现在 shell history 中。安装、身份验证、后台服务和
多 Service 隔离方式请参阅 [Machine 运维](docs/machine-operations.md)。

## 架构

<p align="center">
  <img src="docs/architecture/multi-machine.svg" alt="Desktop 与 Mobile 客户端连接自托管 Cowboy Hub；Hub 保存权威状态，并把工作路由到三台由 Machine 持有的 runtime" width="1100">
</p>

| 边界                  | 负责                                                                 | 不负责                                      |
| --------------------- | -------------------------------------------------------------------- | ------------------------------------------- |
| **Desktop / Mobile**  | 输入、导航、渲染和重新连接                                           | worker 生命周期或权威 Session 状态          |
| **自托管 Hub**        | 顺序、持久化、路由、权限、快照和 fan-out                             | worktree、Provider 进程或 Machine secret    |
| **已加入的 Machine**  | 身份、worktree、独立 ACP worker 和已安装 generation                   | 全局顺序或跨客户端呈现                      |
| **插件 generation**   | 精确的 Provider runtime、类型化 UI、组件和能力契约                    | 浏览器或主机的环境能力                      |

远程 Machine 主动建立经过身份验证的 outbound WebSocket 连接，因此开发主机无需公开
inbound listener；本地 Machine 可以使用 UDS。每个 Session 都会记录对应的 Machine
和精确 Provider generation。fencing、快照、replay 与幂等命令让重连和滚动更新保持
显式可控，不会静默移动或替换正在运行的工作。

组件图和端到端请求流程请参阅[架构概览](docs/architecture/00-overview.md)。

## 产品界面

### Desktop — 键盘优先

高密度 Session 导航、分栏工作区、可见的工具输出，以及面向持续工程任务的
键盘/Vim 控制。

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="site/assets/cowboy-desktop-surface-dark-v2.webp">
    <img src="site/assets/cowboy-desktop-surface-light-v2.webp" alt="Cowboy Desktop 抽象工作区，包含 Session 导航、prompt 编辑器和实时 Agent timeline" width="1100">
  </picture>
</p>

### Mobile — 触控优先

触控优先的 Session 控制、Agent 跟进和代码审查，并重新连接到同一个持久 worker。

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="site/assets/cowboy-mobile-dark-v2.webp">
    <img src="site/assets/cowboy-mobile-light-v2.webp" alt="Cowboy Mobile 的 Session、Agent 工作与代码审查抽象界面" width="960">
  </picture>
</p>

两个客户端观察同一个由服务端权威管理的模型，同时不会把桌面 IDE 拉伸成移动布局，
也不会让移动工作流背负桌面端的界面负担。

## 插件生态

**插件（Plugin）** 是 Cowboy 中唯一可发现、可安装、可升级、可回滚和可卸载的
单元。发布只会让一个不可变 release 变得可用；它绝不会在 Machine 上静默安装或
启用该 release。

```text
源码 → 已签名的 .cowboy-plugin → Catalog → 暂存 + 探测 → 原子启用 → 固定版本的 Session
```

| 类型              | 第一方插件                                                          |
| ----------------- | ------------------------------------------------------------------- |
| Agent Provider    | Codex、Claude Code、Gemini、Grok、Codex + DeepSeek、Claude + DeepSeek |
| 代码智能          | Zed                                                                 |

扩展边界被刻意限制在很小的范围内：

- 包身份会绑定 manifest、payload、契约 fingerprint、runtime 工件和发布者签名；
- 每台 Machine 都会在启用前暂存并探测一个完整 generation；
- 失败时继续使用上一代 generation，新旧 generation 可以并行 drain；
- Session 始终固定到精确的 Provider 和 authentication generation；
- Provider UI 是由 Cowboy 渲染的类型化纯数据 IR；插件不能注入任意 JavaScript、
  HTML、CSS，也不能直接访问 DOM。

建议先阅读[可安装插件包](docs/plugin-packages.md)，再阅读
[插件与共享组件](docs/plugin-components.md)以及规范性的
[核心需求](docs/requirements.md)。

## 安全与所有权

| 不变量                         | Cowboy 的契约                                                                       |
| ------------------------------ | ----------------------------------------------------------------------------------- |
| **自托管状态**                 | Hub 与 SQLite/PostgreSQL 数据保留在你控制的基础设施上。                              |
| **Machine 主动建立连接**       | 远程 Machine 主动连接 Hub；开发主机不暴露 public listener。                         |
| **绑定 Machine 的身份**        | Ed25519 私钥以 mode 0600 保留在所属 Machine 上。                                     |
| **有边界的 enrollment**        | Code 只能使用一次、15 分钟后过期，并且只保存 digest。                                |
| **限定范围的凭据**             | Service 凭据只会到达兼容且已安装对应 generation 的 Machine。                         |
| **故障关闭的启用流程**         | 签名、digest、schema、包或平台不匹配时，不能替换当前有效字节。                        |
| **纯数据插件 UI**              | 插件界面不能直接访问 DOM、文件系统、进程、网络、时钟或随机数。                        |

详细契约请参阅[核心需求](docs/requirements.md)和
[运维文档](docs/architecture/11-operations.md)。

## 开发

在固定版本的 Nix shell 中运行项目命令：

```sh
nix develop
just install
just check
```

<code>just check</code> 覆盖格式、Clippy、依赖策略、Rust 测试、Web
typecheck/lint/test、插件一致性、官网测试和 release build。本地 HMR 需要在两个
terminal 中分别运行 <code>just dev</code> 和 <code>just dev-web</code>。

<details>
<summary><strong>仓库结构</strong></summary>

| 路径                               | 职责                                                                 |
| ---------------------------------- | -------------------------------------------------------------------- |
| <code>src/</code>                  | Rust Hub、API、持久化、Machine 路由、worker 和 CLI                    |
| <code>web/</code>                  | React Desktop、Mobile、Agent、Code、setup 和 admin 界面               |
| <code>plugins/</code>              | 第一方 Agent Provider 与代码智能插件                                  |
| <code>components/</code>           | 带版本的 Plugin、Provider、状态、UI 和代码智能契约                     |
| <code>apps/macos-installer/</code> | 原生 macOS 菜单栏安装器与 Machine 管理器                              |
| <code>site/</code>                 | 公共产品官网与不包含私有信息的插图                                   |
| <code>docs/</code>                 | 架构、产品契约、部署和运维文档                                       |

</details>

已部署的 SQL baseline 必须保持不可变；如需修改，请新增 migration，而不是编辑已经
发布的 migration。重要的 Provider 或架构变更必须继续遵守
[docs/requirements.md](docs/requirements.md) 中的契约。

## 文档

| 从这里开始                                              | 内容                                                            |
| ------------------------------------------------------- | --------------------------------------------------------------- |
| [文档索引](docs/INDEX.md)                               | 完整的架构与产品文档                                            |
| [架构概览](docs/architecture/00-overview.md)            | Hub、Machine、worker、存储和客户端拓扑                           |
| [Machine 运维](docs/machine-operations.md)              | Enrollment、身份、Service 和 Provider 安装                       |
| [插件包](docs/plugin-packages.md)                       | 包、类型化 UI、runtime、authentication 和 release 契约           |
| [代码审查](docs/architecture/13-code-review.md)         | Worktree、Git、文件、diff 和语言智能数据平面                     |
| [构建与部署](docs/architecture/10-deploy-build.md)      | 固定版本构建与按组件发布                                        |

## 项目状态

Cowboy 以公开方式开发，目前仍处于 pre-stable 阶段。架构与测试套件已经较为完整，
但打包、兼容性保证和升级策略仍在向 1.0 收敛。现在即可从源码使用；请固定部署的
commit，并在升级前阅读仓库历史。

## 参与贡献

欢迎提交 Issue 和 Pull Request。请先阅读 [CONTRIBUTING.md](CONTRIBUTING.md)，
了解环境设置、项目契约、验证要求，以及提交变更时需要附带的信息。

- 使用 [GitHub Issues](https://github.com/dravengarden/cowboy/issues) 提交可复现的
  bug 和聚焦的功能提案。
- 请求 review 前运行 <code>just check</code>。
- 视觉变更请附截图，行为变更请补回归测试。

## 许可证

Cowboy 使用 [MIT License](LICENSE)。
