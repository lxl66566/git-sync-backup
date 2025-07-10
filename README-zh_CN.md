# git-sync-backup

[English](./README.md) | 简体中文

`git-sync-backup` (gsb) 是一个命令行工具，旨在利用 Git 对本地文件夹进行同步和备份。它支持跨设备和跨系统使用，并提供了灵活的配置选项来管理文件路径和同步行为。

## 核心功能

`gsb` 提供了以下核心命令：

1.  **`gsb c` 或 `gsb collect`**: 将配置的所有文件/文件夹同步到当前 Git 仓库。
    - 可以指定 `--autocommit` 参数，在收集完成后自动提交当前收集的更新。
2.  **`gsb r` 或 `gsb restore`**: 将当前 Git 仓库中的所有文件/文件夹同步到本地指定位置。
3.  **`gsb s` 或 `gsb sync`**: 在后台持续运行，每隔固定时间间隔从远程仓库 `fetch` 更新，并将更新的文件应用到本地。

## 配置文件格式

`gsb` 的配置文件为仓库根目录下的 `.gsb.config.toml` 文件。它记录了所有 `gsb` 的设置以及文件和文件夹的对应关系。

### 示例配置

```toml
sync_interval = 3600    # gsb sync 同步间隔，单位为秒
version       = "0.2.0" # gsb 版本

# (Optional) Git 相关配置
[git]
branch = "main"   # gsb sync 时使用的分支名（Optional, default = "main"）
remote = "origin" # gsb sync 时使用的远程仓库名（Optional, default = "origin"）

# (Optional) 设备 ID 的别名。设备 ID 可以通过 gsb d 或 gsb device 查看。
[aliases]
device1 = "d37ef0ee-3c3f-419a-8c32-66526565b4ae"

# 定义一个需要同步或备份的项。可以有多个 `[[item]]`。
[[item]]
path_in_repo   = "test"                 # 项目在仓库中的相对路径
default_source = "C:/Program Files/gsb" # (Optional) 默认路径
is_hardlink    = true                   # (Optional) 如果设置为 `true`，则表示仓库中的文件与 `path` 位置是硬链接。在 `collect` 和 `restore` 时不会处理这些文件。不可对文件夹使用。

[[item]]
path_in_repo   = "test2"                                  # 项目在仓库中的相对路径
# (Optional) 为特定设备指定不同的本地路径。
sources.device1                              = "D:/Program Files/gsb"
sources.e48ff1f2-4d5e-4a9f-9c3d-66526565b4ae = "E:/Program Files/gsb"
ignore_collect = ["device1"]                              # (Optional) 当前 item 不需要执行 `collect` 操作的设备
ignore_restore = ["e48ff1f2-4d5e-4a9f-9c3d-66526565b4ae"] # (Optional) 当前 item 不需要执行 `restore` 操作的设备
```

## 安装与使用

### 安装

任选一种安装方式：

- 通过 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) 安装：
  ```bash
  cargo install cargo-binstall
  cargo binstall git-sync-backup
  ```
- 通过 cargo 从源码编译：
  ```bash
  cargo install git-sync-backup
  ```

### 使用

1. 创建仓库并在仓库根目录下编写 `.gsb.config.toml` 文件。
2. 运行 `gsb c` 命令。
