use embedded_svc::storage::RawStorage;
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsCustom};
use serde::{de::DeserializeOwned, Serialize};
use smallvec::SmallVec;
use std::sync::{Mutex, MutexGuard, OnceLock, RwLock};

use anyhow::Context;

pub mod keys;

static KVS: OnceLock<Mutex<Kvs>> = OnceLock::new();
static NAMESPACE: &'static str = "gb";

/// TODO: a few possible changes:
///  * Use a single global lock for all keys and all keys' caches?
///  * use a macro when defining keys to ensure they're all in flush_all
pub struct Kvs {
    nvs_main: EspNvs<NvsCustom>,
    /// 只读分区，如果不存在则为 None（此时读写只读 key 会回退到主分区）
    nvs_ro: Option<EspNvs<NvsCustom>>,
}

impl Kvs {
    pub fn init() -> Result<(), anyhow::Error> {
        log::info!("[KVS] 开始初始化 NVS 存储系统");

        // 主 NVS 分区（必须成功）
        log::info!("[KVS] 正在挂载 NVS 分区 'nvs'...");
        let nvs_main_partition = EspNvsPartition::<NvsCustom>::take("nvs")
            .context("挂载 NVS 分区 'nvs' 失败：可能未烧录分区表、分区损坏或 Flash 硬件故障")?;
        let nvs_main = EspNvs::new(nvs_main_partition, NAMESPACE, true)
            .context("在 NVS 分区 'nvs' 上创建命名空间 'gb' 失败：分区可能已满或损坏")?;
        log::info!("[KVS] 主 NVS 分区 ('nvs') 初始化成功");

        // 尝试初始化只读 NVS 分区，如果不存在则忽略
        log::info!("[KVS] 尝试挂载只读 NVS 分区 'nvs_ro'（如不存在将回退到主分区）...");
        let nvs_ro = match EspNvsPartition::<NvsCustom>::take("nvs_ro") {
            Ok(partition) => {
                match EspNvs::new(partition, NAMESPACE, false) {
                    Ok(nvs) => {
                        log::info!("[KVS] 只读 NVS 分区 ('nvs_ro') 初始化成功");
                        Some(nvs)
                    }
                    Err(e) => {
                        log::warn!(
                            "[KVS] 只读 NVS 分区命名空间创建失败: {}，将使用主分区替代",
                            e
                        );
                        None
                    }
                }
            }
            Err(_) => {
                log::warn!(
                    "[KVS] 分区 'nvs_ro' 不存在或无法挂载，只读键将使用主分区（无只读保护）"
                );
                None
            }
        };

        let kvs = Kvs { nvs_main, nvs_ro };
        KVS.set(Mutex::new(kvs))
            .map_err(|_| ())
            .expect("KVS already initialized");

        log::info!("[KVS] NVS 存储系统初始化完成");
        Ok(())
    }

    pub fn get() -> MutexGuard<'static, Kvs> {
        KVS.get().unwrap().lock().unwrap()
    }

    /// 根据是否只读返回对应的 NVS 实例。
    /// 若只读分区不可用，则回退到主分区（读写）。
    fn nvs(&mut self, read_only: bool) -> &mut EspNvs<NvsCustom> {
        if read_only {
            if let Some(ref mut ro) = self.nvs_ro {
                return ro;
            }
            // 回退到主分区（将丢失只读保护）
            log::debug!("只读分区不可用，使用主分区进行只读访问");
        }
        &mut self.nvs_main
    }
}

// --- 其余代码（CacheEntry, KvsKey 等）保持不变 ---
const SMALL_SIZE: usize = 128;

pub struct CacheEntry<T> {
    value: Option<T>,
    dirty: bool,
}

pub struct KvsKey<T> {
    name: &'static str,
    read_only: bool,
    default: Option<T>,
    cache: RwLock<Option<CacheEntry<T>>>,
}

impl<T: Serialize + DeserializeOwned + Clone> KvsKey<T> {
    const fn new(name: &'static str) -> Self {
        assert!(name.len() < 16);
        KvsKey::<T> {
            name,
            read_only: false,
            default: None,
            cache: RwLock::new(None),
        }
    }

    const fn new_with_default(name: &'static str, default: T) -> Self {
        assert!(name.len() < 16);
        KvsKey::<T> {
            name,
            read_only: false,
            default: Some(default),
            cache: RwLock::new(None),
        }
    }

    const fn new_ro(name: &'static str) -> Self {
        assert!(name.len() < 16);
        KvsKey::<T> {
            name,
            read_only: true,
            default: None,
            cache: RwLock::new(None),
        }
    }

    /// Get the value directly from NVS, without going through the cache.
    fn get_direct(&self) -> Option<T> {
        let mut kvs = Kvs::get();
        let nvs = kvs.nvs(self.read_only);
        let len = nvs.len(self.name).expect("error reading len")?;
        let mut v = SmallVec::<[u8; SMALL_SIZE]>::from_elem(0, len);
        let data = nvs
            .get_raw(self.name, &mut v)
            .expect("error reading value")?;
        Some(postcard::from_bytes(data).expect("error deserializing"))
    }

    pub fn get(&self) -> Option<T> {
        let cache = self.cache.read().unwrap();
        if let Some(value) = cache.as_ref() {
            return value.value.as_ref().or(self.default.as_ref()).cloned();
        }

        let value = self.get_direct();

        std::mem::drop(cache);
        let mut cache = self.cache.write().unwrap();
        *cache = Some(CacheEntry {
            value: value.clone(),
            dirty: false,
        });

        value.or_else(|| self.default.clone())
    }

    fn set_direct(&self, value: &T) {
        let mut kvs = Kvs::get();
        let nvs = kvs.nvs(self.read_only);
        let mut v = SmallVec::<[u8; SMALL_SIZE]>::new();
        postcard::to_io(value, &mut v).expect("error serializing");
        nvs.set_raw(self.name, &v).expect("error writing");
    }

    pub fn set(&self, value: &T) {
        let mut cache = self.cache.write().unwrap();
        *cache = Some(CacheEntry {
            value: Some(value.clone()),
            dirty: true,
        });
    }

    pub fn flush(&self) {
        let mut cache = self.cache.write().unwrap();
        if let Some(cache) = cache.as_mut() {
            if cache.dirty {
                if let Some(value) = cache.value.as_ref() {
                    log::info!("Flushing KVS: {}", self.name);
                    self.set_direct(value);
                }
                cache.dirty = false;
            }
        }
    }
}
