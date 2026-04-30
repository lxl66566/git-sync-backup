# git-sync-backup

English | [简体中文](./README.md)

`git-sync-backup` (gsb) is a command-line tool designed to use Git to synchronize and backup local folders. It supports cross-device and cross-system usage, and provides flexible configuration options to manage file paths and synchronization behavior.

## Installation

Choose one of the following installation methods:

- Install via [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):
  ```bash
  cargo binstall git-sync-backup
  ```
- Build from source:
  ```bash
  cargo install git-sync-backup
  ```

## Usage

1. Run the `gsb d` command to get the device ID of the current device.
2. Create a repository and write a `.gsb.config.toml` file in the repository root.
3. Run the `gsb c` command to sync all specified content to the current Git repository.

All commands:

1. **`gsb c` or `gsb collect`**: Sync all configured files/folders to the current Git repository.
   - You can specify the `--autocommit` (or `-a`) flag to automatically commit the collected updates after completion.
2. **`gsb r` or `gsb restore`**: Restore all files/folders from the current Git repository to the specified local locations.
3. **`gsb s` or `gsb sync`**: Run in the background, periodically `fetch` updates from the remote repository at a fixed interval, and apply the updated files locally.
4. **`gsb d` or `gsb device`**: Output the device identifier of the current device. This is used to fill in fields such as `aliases`, `sources`, `ignore_collect`, `ignore_restore` in the configuration file.

## Configuration File Format

The `gsb` configuration file is `.gsb.config.toml` located at the root of the repository. It records all `gsb` settings and the mapping between files and folders.

```toml
sync_interval = 3600    # gsb sync interval, in seconds
version       = "0.3.0" # gsb version

# (Optional) Git related configuration
[git]
branch = "main"   # Branch name used during gsb sync (Optional, default = "main")
remote = "origin" # Remote repository name used during gsb sync (Optional, default = "origin")

# (Optional) Aliases for device IDs. Device IDs can be viewed via `gsb d` or `gsb device`.
[aliases]
device1 = "d37ef0ee-3c3f-419a-8c32-66526565b4ae"
device3 = "f4e5d1f2-4d5e-4a9f-9c3d-66526565b4ae"

# Define an item that needs to be synced or backed up. Multiple `[[item]]` sections can exist.
[[item]]
default_source = "C:/Program Files/gsb" # (Optional) Default path
is_hardlink    = true                   # (Optional) If set to `true`, indicates that the file in the repository and the file at `path` are hard linked. These files will not be processed during `collect` and `restore`. Cannot be used with folders.
path_in_repo   = "test"                 # Relative path of the item in the repository

[[item]]
path_in_repo = "test2" # Relative path of the item in the repository
ignore                                       = ["device3"]                              # (Optional) Equivalent to putting it in both `ignore_collect` and `ignore_restore`
ignore_collect                               = ["device1"]                              # (Optional) Devices on which the `collect` operation should not be executed for this item
ignore_restore                               = ["e48ff1f2-4d5e-4a9f-9c3d-66526565b4ae"] # (Optional) Devices on which the `restore` operation should not be executed for this item
# (Optional) Specify different local paths for specific devices.
# Both aliases and original device hashes can be used as keys.
sources.device1                              = "D:/Program Files/gsb"
sources.e48ff1f2-4d5e-4a9f-9c3d-66526565b4ae = "E:/Program Files/gsb"
```
