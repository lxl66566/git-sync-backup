use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use config_file2::*;
use serde::{Deserialize, Serialize};
use whoami::devicename;

use crate::git_command::REPO_PATH;

const CONFIG_NAME: &str = ".gsb.config.toml";

use std::sync::LazyLock;

pub static CONFIG: LazyLock<Arc<RwLock<Config>>> = LazyLock::new(|| {
    Arc::new(RwLock::new(
        Config::load_or_default(REPO_PATH.clone().join(CONFIG_NAME))
            .expect("failed to load config"),
    ))
});

/// The files in [`SyncGroup`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct SyncFile {
    /// The absolute path of file in multiple devices. The key is the device
    /// name, and the value is the absolute path on the device.
    pub path_on_devices: BTreeMap<String, PathBuf>,
    /// Whether the file is a hardlink. If not, it needs a copy sync.
    pub is_hardlink: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct BackupFile {
    /// The absolute path of file in this device.
    pub path_on_device: PathBuf,
    /// Whether the file is a hardlink. If not, it needs a copy backup.
    pub is_hardlink: bool,
}

pub trait Getable<'a> {
    type Output;
    fn get_on_device(&'a self) -> Self::Output;
}

impl<'a> Getable<'a> for SyncFile {
    type Output = Option<&'a PathBuf>;
    fn get_on_device(&'a self) -> Self::Output {
        self.path_on_devices.get(&devicename())
    }
}

/// The `Sync` group. Files in this group will be synced between devices. There
/// will be only one sync group in a repository (among all devices), and all
/// files in [`SyncGroup`] will be stored in `sync` branch.
///
/// Key: relative path in the repository.
/// Value: [`SyncFile`].
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SyncGroup(pub BTreeMap<PathBuf, SyncFile>);

/// The `Backup` group. Files in this group will be backed up, but not be
/// synced. Each [`BackupGroup`] will take up one branch named
/// `backup-${devide_name}`. There will be only one backup group in a device,
/// but a repo could has multiple backup groups.
///
/// Key: relative path in the repository.
/// Value: [`SyncFile`].
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BackupGroup(pub BTreeMap<PathBuf, BackupFile>);

/// The config file contains the information of current device, as well as the
/// sync group and the backup group on current device.
///
/// There is only one SyncGroup in a repository.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub device_name: String,
    pub remote: Option<String>,
    pub sync_group: SyncGroup,
    pub backup_group: BackupGroup,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_name: devicename(),
            remote: None,
            sync_group: SyncGroup::default(),
            backup_group: Default::default(),
        }
    }
}
