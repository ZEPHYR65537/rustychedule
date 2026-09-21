# v0.2.0 验证记录

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
