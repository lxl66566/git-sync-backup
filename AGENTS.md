# git-sync-backup

# 行为准则

你是一个资深 Rust 工程师，注重代码可维护性和性能优化，并且遵循 Rust 工程开发的最佳实践。

- 少造轮子，如果有合适的、高性能的第三方库就用
- 少写重复代码，多抽离出可复用的组件，并考虑向后扩展性
  - 你应该使用在编译期就能进行错误检查的设计，而不是推到运行期检查，例如多用枚举，不用硬编码。
- 使用简体中文进行交流与代码注释。
- 不要删除运行逻辑相关的关键注释
- 不硬性要求单测覆盖率，但是关键部分需要编写单测
- 你的上下文有限，请合理安排 Sub Agent 进行任务分配。
- 写出符合工程实践的代码，多复用，注重性能优化。不要为了偷懒写出一些性能差的 naive 实现。

## 项目简介

本项目主要使用 git 对文件夹进行同步或备份。

1. gsb c 或 gsb collect 命令，将所有文件夹同步到当前 git 仓库。
2. gsb r 或 gsb restore 命令，将当前 git 仓库中的所有文件夹同步到本地指定位置。
3. gsb s 或 gsb sync 命令，持续后台运行，每一段固定间隔去 fetch 远程仓库的更新，并将更新文件应用到本地。

### 配置文件格式

配置文件为仓库下根目录的 `.gsb.config.toml` 文件，记录了所有 gsb 的设置与文件对应关系。

gsb 设计为跨设备跨系统与架构使用，同一个文件在不同设备上的存储路径可能不同。每一个文件可以有一个 default_source 路径在所有设备上通用，也可以使用 device name 指定具体设备的路径。

每一个 path 可以是文件或文件夹。如果是文件，还可以指定 is_hardlink 为 true，则默认仓库中文件与 path 位置是 hardlink，collect 和 restore 时无需处理。

```toml
sync_interval = 3600    # gsb sync 同步间隔，单位为秒
version       = "0.2.1" # gsb 版本

[git] # （Optional）Git 相关配置
branch = "main"   # gsb sync 时使用的分支名（Optional, default = "main"）
remote = "origin" # gsb sync 时使用的远程仓库名（Optional, default = "origin"）

# 定义设备 id 的 alias，允许混用
[aliases]
main = "ff19a810-c0ea-44b0-b297-f2209a48bfc3"
work = "25f758c0-d868-45ed-95d8-4db9494c8a38"

# 定义一个需要同步或备份的项。可以有多个 `[[item]]`。
[[item]]
default_source = "C:/Program Files/gsb" # （Optional）默认路径
is_hardlink    = true                   # （Optional）如果设置为 `true`，则表示仓库中的文件与 `path` 位置是硬链接。在 `collect` 和 `restore` 时不会处理这些文件。不可对文件夹使用。
path_in_repo   = "test"                 # 项目在仓库中的相对路径

[[item]]
path_in_repo = "test2" # 项目在仓库中的相对路径
ignore_collect                               = ["d37ef0ee-3c3f-419a-8c32-66526565b4ae"]    # （Optional）当前 item 不需要执行 `collect` 操作的设备
ignore_restore                               = ["work"]    # （Optional）当前 item 不需要执行 `restore` 操作的设备
# （Optional）为特定设备指定不同的本地路径。
sources.e48ff1f2-4d5e-4a9f-9c3d-66526565b4ae = "E:/Program Files/gsb"
sources.main = "D:/Program Files/gsb"
```
