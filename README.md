# git-sync-backup

English | [简体中文](./README-zh_CN.md)

`git-sync-backup` (gsb) is a command-line tool designed to synchronize and back up local folders using Git. It supports cross-device and cross-system usage, providing flexible configuration options to manage file paths and synchronization behavior.

## Core Features

`gsb` offers the following core commands:

1.  **`gsb c` or `gsb collect`**: Synchronizes all configured files/folders to the current Git repository.
    - You can specify the `--autocommit` argument to automatically commit the collected updates.
2.  **`gsb r` or `gsb restore`**: Synchronizes all files/folders from the current Git repository to specified local locations.
3.  **`gsb s` or `gsb sync`**: Runs continuously in the background, fetching updates from the remote repository at fixed intervals and applying the updated files locally.

## Configuration File Format

The `gsb` configuration file is `.gsb.config.toml` located at the root of the repository. It records all `gsb` settings and the mapping between files and folders.

### Example Configuration

```toml
sync_interval = 3600    # GSB sync interval in seconds
version       = "0.2.0" # GSB version

# (Optional) Git-related configurations
[git]
branch = "main"   # Branch name used for gsb sync (Optional, default = "main")
remote = "origin" # Remote repository name used for gsb sync (Optional, default = "origin")

# (Optional) Aliases for device IDs. Device IDs can be viewed via `gsb d` or `gsb device`.
[aliases]
device1 = "d37ef0ee-3c3f-419a-8c32-66526565b4ae"

# Defines an item to be synced or backed up. Multiple `[[item]]` sections can be added.
[[item]]
path_in_repo   = "test"                 # Relative path of the item in the repository
default_source = "C:/Program Files/gsb" # (Optional) Default path
is_hardlink    = true                   # (Optional) If set to `true`, indicates files in the repository are hard-linked to the `path` location. These files won't be processed during `collect` and `restore`. Cannot be used for folders.

[[item]]
path_in_repo   = "test2"                                  # Relative path of the item in the repository
# (Optional) Specifies different local paths for specific devices.
sources.device1                              = "D:/Program Files/gsb"
sources.e48ff1f2-4d5e-4a9f-9c3d-66526565b4ae = "E:/Program Files/gsb"
ignore_collect = ["device1"]                              # (Optional) Devices excluded from `collect` operation for this item
ignore_restore = ["e48ff1f2-4d5e-4a9f-9c3d-66526565b4ae"] # (Optional) Devices excluded from `restore` operation for this item
```

## Installation and Usage

### Installation

Choose one of the following installation methods:

- Install via [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):
  ```bash
  cargo install cargo-binstall
  cargo binstall git-sync-backup
  ```
- Compile from source via cargo:
  ```bash
  cargo install git-sync-backup
  ```

### Usage

1. Create a repository and write the `.gsb.config.toml` file in the repository root.
2. Run the `gsb c` command.
