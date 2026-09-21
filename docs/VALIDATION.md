# v0.3.0 验证记录

日期：2026-09-21。执行环境：Windows x64 / Rust 1.94.0。

| 检查 | 结果 |
|---|---|
| cargo fmt --check | 通过 |
| cargo test --locked | 73 项通过：原有 57 项及 16 项 v0.3 集成回归 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告 |
| cargo build --release --locked | 通过，生成 schedule.exe、tc.exe、tongchou.exe |
| 发布程序实际操作 | 创建 A，导入 X/Y，建立 Z，树状查看、校验、提交、导出及来源绑定通过 |
| 发布程序文件边界 | 中文目录和记忆正文原样恢复，默认排除 .env，嵌套代码仓库只登记机器位置、不复制代码 |

## v0.3 关键回归

- 逻辑任务与 agent 项目/聊天多对多关联；移动、拆分和合并后规划账本逐字不变。
- A/X/Y/Z 目录层级、普通附件、嵌套 Git 仓库、Git ignore 规则、导入预览及导出恢复。
- 稳定机器身份、别名修改、远程/本地仓库索引、复用引用；拒绝含凭据的远程 URL。
- 来源/基准/中央三方比较；来源缺失、显式 prune、双边修改冲突；中央独立编辑不会被旧来源覆盖。
- 导出后绑定来源并再次收集的往返更新。
- 两个独立设备数据目录通过临时 bare remote 同步，按项目或文件夹选择正文；未检出正文不当作删除。
- 仅同步 agent 资料不改写本机规划账本；规划分支双方修改时拒绝自动文本合并。
- 分叉分支正常合并；文本冲突、结构语义冲突失败时原 HEAD 和工作树不变。
- 拒绝父子循环、同级大小写冲突、路径越界；Unix 另有符号链接回归（CI 在 Unix 平台额外执行 1 项）。
- 删除要求事先提交，可通过 Git 恢复，且保留原始目录和代码仓库。
- 中断的多文件修改由事务备份恢复。
- 旧 hub 迁移保留账本、记忆和 bundle；重复迁移可识别，旧 writer 拒绝新格式。
- 第二台旧设备跟随中央迁移，复用同一批身份；尚无 hub 的旧用户也能保留已登记项目和记忆。

本机测试和发布程序验证均使用隔离数据及临时 Git 仓库，没有注册或上传用户真实项目。Windows 本机结果如上；Linux/macOS 的额外符号链接检查及跨平台构建结果以对应提交的 [GitHub Actions](https://github.com/ZEPHYR65537/rustychedule/actions) 为准。

---

# v0.2.0 验证记录（历史）

日期：2026-09-21。执行环境：Windows x64 / Rust 1.94.0。

| 检查 | 结果 |
|---|---|
| cargo fmt --check | 通过 |
| cargo test --locked | 57 项通过：30 核心、5 CLI、21 v0.2 回归、1 终端字符宽度 |
| cargo clippy --locked --all-targets -- -D warnings | 通过，无警告 |
| cargo build --release --locked | 通过，生成 schedule.exe、tc.exe、tongchou.exe |
| 发布程序实际操作 | 模型/订阅/API 配置、来源策略、百分比校准、三片段预留、复盘、卡和 Tibo 登记、重规划通过 |
| 两台设备模拟 | 独立 data 目录及镜像，通过本地 bare Git 仓库 push/clone/pull，恢复进度、项目索引、长期记忆并绑定本机路径 |
| 终端视图 | 总览、分来源预算表、订阅续期、用量条、实际分钟、卡库存检查通过 |

## 关键回归

- API 上限 0 直接降级；正数只在本模型剩余上界内用；不借其他模型 API。
- subscription-first / model-first / subscription-only 的分配区别。
- 订阅、API 的实际消费与预留隔离；卡/Tibo/自然周期不补充 API。
- 不可跨模型分段的任务可组合本模型订阅和 API。
- 估计容量变化保留原始百分比，当前基线按新估计重算。
- 停用 API 释放未用 API 预算；允许记录真实超限但不继续安排超配。
- 自定义订阅权重不能被标准订阅配置静默重新解释。
- 工作复盘关联一次用量、撤销不回退显式进度。
- 不再接受价格参数，新 JSON 无价格、cost、currency；v1 保留任务与使用记录。
- 项目路径保持本机，引用模式不同步源代码。
- 干净 HEAD bundle 能在新目录恢复，原项目不被修改；脏项目、非法名称被拒绝。
- Windows 扩展路径兼容 Git clone。
- 双端修改拒绝覆盖，包括只修改来源策略的状态；显式 replace 可接受远端。
- hub push 不包含任意未跟踪文件或未知已暂存文件。
- 原有偏序/依赖、能力底线、倍率降级、整数约束、重复预留、重规划范围和失败回滚。
- 原有三类刷新、滚动窗口、时间边界、文件锁、原子替换和损坏恢复。

同步测试仅使用临时 bare 仓库；没有自动为用户创建 GitHub 私有工作空间，也没有上传用户任务、代码或记忆。

仓库 CI 对 Windows、Linux、macOS 分别执行格式、测试、Clippy 和 release 构建，上传三个命令的可执行文件。本文件记录本机实际完成结果；对应提交的远程结果见 [GitHub Actions](https://github.com/ZEPHYR65537/rustychedule/actions)。
