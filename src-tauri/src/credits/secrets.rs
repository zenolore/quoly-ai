use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
#[cfg(target_os = "macos")]
pub(crate) mod macos;

/// Windows 上把全部 secret 打包成一个 DPAPI 加密文件。
///
/// 背景：Windows 凭据库每用户上限实测约 347KB，单个账号要 3.3KB（两段 JWT
/// 各 1.6KB 且不能裁剪），一百个号就撞墙——`CredWriteW` 返回
/// `ERROR_NOT_ENOUGH_MEMORY(8)`，新账号全部注入失败。改用文件后容量只受磁盘
/// 限制，DPAPI 仍保证只有当前 Windows 用户能解密。
#[cfg(target_os = "windows")]
mod vault_file {
    use std::ffi::c_void;
    use std::path::PathBuf;

    #[repr(C)]
    struct Blob {
        cb: u32,
        pb: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut Blob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    /// 禁止 DPAPI 弹窗（后台刷新时不能阻塞用户）。
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ai-toolkit")
            .join("credentials.vault")
    }

    pub fn read() -> Result<Option<Vec<u8>>, String> {
        read_from(&path())
    }

    pub fn read_from(file: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
        let sealed = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("无法读取本地凭据文件，请检查文件权限".into()),
        };
        if sealed.is_empty() {
            return Ok(Some(b"{}".to_vec()));
        }
        unsafe {
            let mut input = Blob {
                cb: sealed.len() as u32,
                pb: sealed.as_ptr() as *mut u8,
            };
            let mut output = Blob {
                cb: 0,
                pb: std::ptr::null_mut(),
            };
            let ok = CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            );
            if ok == 0 {
                return Err("本地凭据文件解密失败，可能由其他 Windows 用户创建".into());
            }
            let plain = std::slice::from_raw_parts(output.pb, output.cb as usize).to_vec();
            LocalFree(output.pb as *mut c_void);
            Ok(Some(plain))
        }
    }

    pub fn write(plain: &[u8]) -> Result<(), String> {
        write_to(&path(), plain)
    }

    pub fn write_to(file: &std::path::Path, plain: &[u8]) -> Result<(), String> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|_| "无法创建本地凭据目录".to_string())?;
        }
        unsafe {
            let mut input = Blob {
                cb: plain.len() as u32,
                pb: plain.as_ptr() as *mut u8,
            };
            let mut output = Blob {
                cb: 0,
                pb: std::ptr::null_mut(),
            };
            let ok = CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            );
            if ok == 0 {
                return Err("本地凭据加密失败".into());
            }
            let sealed = std::slice::from_raw_parts(output.pb, output.cb as usize);
            // 先写临时文件再替换，避免写到一半崩溃留下半截凭据。
            let temp = file.with_extension("vault.tmp");
            let result = std::fs::write(&temp, sealed).and_then(|_| std::fs::rename(&temp, &file));
            LocalFree(output.pb as *mut c_void);
            result.map_err(|_| "无法写入本地凭据文件".to_string())
        }
    }

    pub fn remove() -> Result<(), String> {
        remove_at(&path())
    }

    pub fn remove_at(file: &std::path::Path) -> Result<(), String> {
        match std::fs::remove_file(file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法删除本地凭据文件".into()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn real_vault_file_parses() {
            let file = path();
            if !file.exists() {
                return; // 没有迁移文件时不校验
            }
            let bytes = read_from(&file).expect("解密失败").expect("文件为空");
            let text = std::str::from_utf8(&bytes).expect("不是 UTF-8");
            let map: std::collections::BTreeMap<String, String> =
                serde_json::from_str(text).expect("整包 JSON 解析失败");
            assert!(map.len() > 100, "账号数异常：{}", map.len());
            let mut parsed = 0;
            for (id, value) in &map {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(value) {
                    assert!(json.get("accessToken").is_some(), "{id} 缺 accessToken");
                    parsed += 1;
                }
            }
            assert!(parsed > 100, "标准凭据只有 {} 条", parsed);
            println!("真实 vault 文件：{} 个键，{} 条标准凭据，{} 字节", map.len(), parsed, bytes.len());
        }

        #[test]
        fn dpapi_roundtrip_isolated() {
            let dir = std::env::temp_dir().join(format!("ait-vault-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("credentials.vault");

            assert!(read_from(&file).unwrap().is_none(), "文件不存在时应返回 None");

            let payload = br#"{"acc-1":"{\"accessToken\":\"a.b.c\"}","acc-2":"x"}"#.to_vec();
            write_to(&file, &payload).unwrap();
            assert_eq!(read_from(&file).unwrap().unwrap(), payload, "加解密必须无损");

            let sealed = std::fs::read(&file).unwrap();
            assert_ne!(sealed, payload, "落盘内容必须是密文");
            assert!(sealed.len() > payload.len(), "密文应带 DPAPI 头部");

            let second = b"{}".to_vec();
            write_to(&file, &second).unwrap();
            assert_eq!(read_from(&file).unwrap().unwrap(), second, "覆盖写应生效");

            remove_at(&file).unwrap();
            assert!(read_from(&file).unwrap().is_none(), "删除后应回到 None");
            std::fs::remove_dir_all(&dir).ok();
        }
    }
}
const CHUNK_SIZE: usize = 2000;
const PREFIX: &str = "ait-secret-v1:";
const MAX_CHUNKS: usize = 128;
const VAULT_ACCOUNT: &str = "vault";
const VAULT_PREFIX: &str = "ait-vault-v1:";

#[derive(Clone, Copy)]
enum Layout {
    Vault,
    Chunked,
}
#[derive(Default)]
struct Cache {
    values: HashMap<String, Option<String>>,
    vault: Option<BTreeMap<String, String>>,
    legacy_errors: HashMap<String, String>,
}
#[cfg(test)]
#[derive(Default)]
struct MemoryBackend {
    values: HashMap<String, Vec<u8>>,
    reads: Vec<String>,
    writes: Vec<String>,
    read_error: Option<String>,
    write_error: Option<String>,
    protected: std::collections::HashSet<String>,
    prompts: Vec<String>,
}

pub struct SecretStore {
    gate: Mutex<Cache>,
    blocked: Mutex<Option<String>>,
    layout: Layout,
    #[cfg(test)]
    memory: Option<Mutex<MemoryBackend>>,
}
impl SecretStore {
    pub fn new() -> Self {
        Self {
            gate: Mutex::new(Cache::default()),
            blocked: Mutex::new(None),
            // Windows 也走 Vault（整包）布局：凭据库每用户上限约 347KB，按号分片
            // 存到一百个号就写不进去了，改成整包落 DPAPI 文件后只受磁盘限制。
            layout: if cfg!(any(target_os = "macos", target_os = "windows")) {
                Layout::Vault
            } else {
                Layout::Chunked
            },
            #[cfg(test)]
            memory: None,
        }
    }
    #[cfg(test)]
    pub fn memory() -> Self {
        Self::memory_with_layout(if cfg!(target_os = "macos") {
            Layout::Vault
        } else {
            Layout::Chunked
        })
    }
    #[cfg(test)]
    fn memory_with_layout(layout: Layout) -> Self {
        Self {
            gate: Mutex::new(Cache::default()),
            blocked: Mutex::new(None),
            layout,
            memory: Some(Mutex::new(MemoryBackend::default())),
        }
    }
    /// 丢弃内存缓存并重新从存储加载。
    /// 外部进程（批量注号脚本）直接改写 vault 文件后，本进程的内存副本不会自动
    /// 感知，表现为 dashboard 报「账号凭据不存在」。调用本方法即可无重启同步。
    pub fn reload_cache(&self) -> Result<usize, String> {
        let mut cache = self.gate.lock();
        cache.values.clear();
        cache.legacy_errors.clear();
        cache.vault = None;
        let map = self.vault_map(&mut cache)?;
        Ok(map.len())
    }
    pub fn get(&self, id: &str) -> Result<Option<String>, String> {
        let mut cache = self.gate.lock();
        self.access_ready()?;
        if let Some(value) = cache.values.get(id) {
            return Ok(value.clone());
        }
        let value = match self.layout {
            Layout::Vault => self.vault_get(id, &mut cache),
            Layout::Chunked => self.native_result(self.chunked_get(id)),
        }?;
        cache.values.insert(id.into(), value.clone());
        Ok(value)
    }
    pub fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        let mut cache = self.gate.lock();
        self.access_ready()?;
        if cache.values.get(id).and_then(Option::as_deref) == Some(secret) {
            return Ok(());
        }
        self.native_result(match self.layout {
            Layout::Vault => self.vault_set(id, secret, &mut cache),
            Layout::Chunked => self.chunked_set(id, secret),
        })?;
        cache.legacy_errors.remove(id);
        cache.values.insert(id.into(), Some(secret.into()));
        Ok(())
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut cache = self.gate.lock();
        self.access_ready()?;
        if cache.values.get(id) == Some(&None) {
            return Ok(());
        }
        self.native_result(match self.layout {
            Layout::Vault => self.vault_delete(id, &mut cache),
            Layout::Chunked => self.chunked_delete(id),
        })?;
        cache.values.insert(id.into(), None);
        Ok(())
    }
    /// Called only by explicit user actions, never by polling or proxy requests.
    pub fn retry_access(&self) -> Result<(), String> {
        let mut cache = self.gate.lock();
        if self.blocked.lock().take().is_some() {
            *cache = Cache::default();
        }
        cache.legacy_errors.clear();
        if matches!(self.layout, Layout::Vault)
            && cache.vault.as_ref().is_none_or(BTreeMap::is_empty)
        {
            // Authorize synchronously within this user action. Never leave a
            // permission flag for a later scheduler/proxy request to consume.
            cache.vault = Some(self.native_result(self.vault_read(true))?);
        }
        Ok(())
    }
    /// Authorize only the secret needed by this explicit action. Account-wide
    /// refreshes must keep using retry_access so legacy accounts stay silent.
    pub fn retry_access_for(&self, id: &str) -> Result<(), String> {
        self.retry_access()?;
        if !matches!(self.layout, Layout::Vault) {
            return Ok(());
        }
        let mut cache = self.gate.lock();
        self.access_ready()?;
        if cache.values.contains_key(id) || self.vault_map(&mut cache)?.contains_key(id) {
            return Ok(());
        }
        let value = self.chunked_get_with_ui(id, true).map_err(|error| {
            cache.legacy_errors.insert(id.into(), error.clone());
            error
        })?;
        if let Some(secret) = &value {
            self.vault_set(id, secret, &mut cache)?;
            self.delete_legacy(id)?;
        }
        cache.values.insert(id.into(), value);
        Ok(())
    }
    /// Replacing a verified login must not first decrypt the old legacy chunks.
    pub fn get_for_update(&self, id: &str) -> Result<Option<String>, String> {
        match self.layout {
            Layout::Vault => {
                let mut cache = self.gate.lock();
                self.access_ready()?;
                Ok(self.vault_map(&mut cache)?.get(id).cloned())
            }
            Layout::Chunked => self.get(id),
        }
    }
    /// Roll back a failed account save without deleting untouched legacy data.
    pub fn restore(&self, id: &str, previous: Option<&str>) -> Result<(), String> {
        if let Some(previous) = previous {
            return self.set(id, previous);
        }
        match self.layout {
            Layout::Vault => {
                let mut cache = self.gate.lock();
                self.access_ready()?;
                let mut map = self.vault_map(&mut cache)?.clone();
                map.remove(id);
                let merged = self.vault_write(&map, Some(id))?;
                cache.vault = Some(merged);
                cache.values.remove(id);
                Ok(())
            }
            Layout::Chunked => self.delete(id),
        }
    }
    fn access_ready(&self) -> Result<(), String> {
        match self.blocked.lock().as_ref() {
            Some(error) => Err(format!("{error}；已暂停自动访问凭据，请点击“刷新”重试")),
            None => Ok(()),
        }
    }
    fn native_result<T>(&self, result: Result<T, String>) -> Result<T, String> {
        if let Err(error) = &result {
            *self.blocked.lock() = Some(error.clone());
        }
        result
    }

    // macOS uses one item and caches its decoded map for this process lifetime.
    // Always Allow is still tied to the app's code signing identity by macOS.
    fn vault_read(&self, allow_ui: bool) -> Result<BTreeMap<String, String>, String> {
        let Some(raw) = self.read_with_ui(VAULT_ACCOUNT, allow_ui)? else {
            if allow_ui {
                // Creation may require unlocking the keychain, so do it within
                // this explicit action rather than during a later background save.
                self.write_with_ui(VAULT_ACCOUNT, b"{}", true)?;
            }
            return Ok(BTreeMap::new());
        };
        let raw = std::str::from_utf8(&raw).map_err(|_| "系统凭据编码无效".to_string())?;
        serde_json::from_str(raw.strip_prefix(VAULT_PREFIX).unwrap_or(raw))
            .map_err(|_| "系统凭据编码无效".into())
    }
    /// 合并写：先读盘上最新内容，保留「本进程启动后由外部写入」的账号，
    /// 再用本次进程掌管的键覆盖。否则进程内缓存会把外部新增项整份冲掉。
    /// （实测：脚本注入的新号会在客户端下一次刷新 token 时被静默抹除）
    fn vault_write(
        &self,
        map: &BTreeMap<String, String>,
        removed: Option<&str>,
    ) -> Result<BTreeMap<String, String>, String> {
        let mut merged = match self.read_with_ui(VAULT_ACCOUNT, false) {
            Ok(Some(raw)) => {
                let text = String::from_utf8_lossy(&raw);
                let text = text.strip_prefix(VAULT_PREFIX).unwrap_or(&text);
                serde_json::from_str::<BTreeMap<String, String>>(text).unwrap_or_default()
            }
            _ => BTreeMap::new(),
        };
        for (key, value) in map {
            merged.insert(key.clone(), value.clone());
        }
        // 合并写只做「覆盖/新增」，删除必须显式从这里扣掉，
        // 否则盘上那条会被读回来重新写进去（删号变成静默无效）。
        if let Some(id) = removed {
            merged.remove(id);
        }
        let raw = serde_json::to_string(&merged).map_err(|_| "系统凭据编码无效".to_string())?;
        self.native_result(self.write(VAULT_ACCOUNT, raw.as_bytes()))?;
        Ok(merged)
    }
    fn vault_map<'a>(&self, cache: &'a mut Cache) -> Result<&'a BTreeMap<String, String>, String> {
        if cache.vault.is_none() {
            cache.vault = Some(self.native_result(self.vault_read(false))?);
        }
        Ok(cache.vault.as_ref().expect("vault loaded"))
    }
    fn vault_get(&self, id: &str, cache: &mut Cache) -> Result<Option<String>, String> {
        if let Some(value) = self.vault_map(cache)?.get(id) {
            return Ok(Some(value.clone()));
        }
        if let Some(error) = cache.legacy_errors.get(id) {
            return Err(error.clone());
        }
        // Never request UI for legacy items: each chunk has its own ACL and
        // allowing the shared vault does not authorize those separate records.
        let legacy = self.chunked_get(id).map_err(|cause| {
            let error = format!("{cause}；旧版凭据无法自动迁移，请重新登录该账号");
            cache.legacy_errors.insert(id.into(), error.clone());
            error
        })?;
        if let Some(raw) = legacy {
            let mut map = self.vault_map(cache)?.clone();
            map.insert(id.into(), raw.clone());
            let merged = self.vault_write(&map, None)?;
            cache.vault = Some(merged);
            self.delete_legacy(id)?;
            return Ok(Some(raw));
        }
        Ok(None)
    }
    fn vault_set(&self, id: &str, secret: &str, cache: &mut Cache) -> Result<(), String> {
        let mut map = self.vault_map(cache)?.clone();
        map.insert(id.into(), secret.into());
        let merged = self.vault_write(&map, None)?;
        cache.vault = Some(merged);
        Ok(())
    }
    fn vault_delete(&self, id: &str, cache: &mut Cache) -> Result<(), String> {
        let mut map = self.vault_map(cache)?.clone();
        // Metadata-only discovery avoids reading a protected manifest to delete it.
        self.delete_legacy(id)?;
        // 不判 map 里有没有：盘上可能存在一份内存不知道的条目，一律显式删。
        map.remove(id);
        let merged = self.vault_write(&map, Some(id))?;
        cache.vault = Some(merged);
        Ok(())
    }

    // Legacy per-id storage (still the Windows/Linux layout; on macOS it is
    // only read during the one-time vault migration).
    fn chunked_get(&self, id: &str) -> Result<Option<String>, String> {
        self.chunked_get_with_ui(id, false)
    }
    fn chunked_get_with_ui(&self, id: &str, allow_ui: bool) -> Result<Option<String>, String> {
        let read = |account: &str| {
            if allow_ui {
                self.read_with_ui(account, true)
            } else {
                self.read(account)
            }
        };
        let Some(raw) = read(id)? else {
            return Ok(None);
        };
        let bytes = if let Some((generation, count)) = manifest(&raw)? {
            let mut all = Vec::new();
            for index in 0..count {
                let chunk = read(&chunk_id(id, &generation, index))?
                    .ok_or("系统凭据不完整，请重新授权")?;
                if chunk.len() > CHUNK_SIZE {
                    return Err("系统凭据格式无效".into());
                }
                all.extend(chunk);
            }
            all
        } else {
            raw
        };
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "系统凭据编码无效".into())
    }
    fn chunked_set(&self, id: &str, secret: &str) -> Result<(), String> {
        if secret.len() > CHUNK_SIZE * MAX_CHUNKS {
            return Err("账号凭据过大".into());
        }
        let previous = self
            .read(id)?
            .map(|raw| manifest(&raw))
            .transpose()?
            .flatten();
        let generation = uuid::Uuid::new_v4().to_string();
        let count = (secret.len() + CHUNK_SIZE - 1) / CHUNK_SIZE;
        for (index, chunk) in secret.as_bytes().chunks(CHUNK_SIZE).enumerate() {
            if let Err(error) = self.write(&chunk_id(id, &generation, index), chunk) {
                self.cleanup(id, &generation, count);
                return Err(error);
            }
        }
        if let Err(error) = self.write(id, format!("{PREFIX}{generation}:{count}").as_bytes()) {
            self.cleanup(id, &generation, count);
            return Err(error);
        }
        if let Some((generation, count)) = previous {
            self.cleanup(id, &generation, count);
        }
        Ok(())
    }
    fn chunked_delete(&self, id: &str) -> Result<(), String> {
        if let Some(raw) = self.read(id)? {
            if let Some((generation, count)) = manifest(&raw)? {
                for index in 0..count {
                    self.remove(&chunk_id(id, &generation, index))?;
                }
            }
            self.remove(id)?;
        }
        Ok(())
    }
    fn cleanup(&self, id: &str, generation: &str, count: usize) {
        for index in 0..count {
            let _ = self.remove(&chunk_id(id, generation, index));
        }
    }
    fn delete_legacy(&self, id: &str) -> Result<(), String> {
        #[cfg(test)]
        if let Some(memory) = &self.memory {
            let mut memory = memory.lock();
            memory
                .values
                .retain(|account, _| !is_legacy_account(id, account));
            memory
                .protected
                .retain(|account| !is_legacy_account(id, account));
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            for account in macos::legacy_accounts(id)? {
                self.remove(&account)?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        self.chunked_delete(id)
    }
    fn read(&self, id: &str) -> Result<Option<Vec<u8>>, String> {
        if matches!(self.layout, Layout::Chunked) {
            self.access_ready()?;
            self.native_result(self.read_with_ui(id, true))
        } else {
            self.read_with_ui(id, false)
        }
    }
    fn read_with_ui(&self, id: &str, _allow_ui: bool) -> Result<Option<Vec<u8>>, String> {
        #[cfg(test)]
        if let Some(memory) = &self.memory {
            let mut memory = memory.lock();
            memory.reads.push(id.into());
            if memory.protected.contains(id) {
                if !_allow_ui {
                    return Err("系统凭据需要授权".into());
                }
                memory.prompts.push(id.into());
            }
            return match &memory.read_error {
                Some(error) => Err(error.clone()),
                None => Ok(memory.values.get(id).cloned()),
            };
        }
        #[cfg(target_os = "macos")]
        return macos::read(id, _allow_ui);
        #[cfg(not(target_os = "macos"))]
        {
            // 整包只落一个文件；其余 id 仍是旧版按号分片，迁移时逐条读出来。
            #[cfg(target_os = "windows")]
            {
                if id == VAULT_ACCOUNT {
                    return vault_file::read();
                }
            }
            entry(id).and_then(|credential| match credential.get_secret() {
                Ok(s) => Ok(Some(s)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err("无法读取系统凭据库，请检查钥匙串权限".into()),
            })
        }
    }
    fn write(&self, id: &str, secret: &[u8]) -> Result<(), String> {
        self.access_ready()?;
        self.native_result(self.write_with_ui(id, secret, false))
    }
    fn write_with_ui(&self, id: &str, secret: &[u8], _allow_ui: bool) -> Result<(), String> {
        #[cfg(test)]
        if let Some(memory) = &self.memory {
            let mut memory = memory.lock();
            memory.writes.push(id.into());
            if let Some(error) = &memory.write_error {
                return Err(error.clone());
            }
            memory.values.insert(id.into(), secret.to_vec());
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        return macos::write(id, secret, _allow_ui);
        #[cfg(not(target_os = "macos"))]
        {
            #[cfg(target_os = "windows")]
            {
                if id == VAULT_ACCOUNT {
                    return vault_file::write(secret);
                }
            }
            entry(id).and_then(|credential| {
                credential
                    .set_secret(secret)
                    .map_err(|_| "无法保存到系统凭据库，请检查凭据库权限".into())
            })
        }
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        self.access_ready()?;
        self.native_result(self.remove_native(id))
    }
    fn remove_native(&self, id: &str) -> Result<(), String> {
        #[cfg(test)]
        if let Some(memory) = &self.memory {
            let mut memory = memory.lock();
            memory.values.remove(id);
            memory.protected.remove(id);
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        return macos::remove(id);
        #[cfg(not(target_os = "macos"))]
        {
            #[cfg(target_os = "windows")]
            {
                if id == VAULT_ACCOUNT {
                    return vault_file::remove();
                }
            }
            entry(id).and_then(|credential| match credential.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err("无法删除系统凭据库中的凭据".into()),
            })
        }
    }
}
#[cfg(not(target_os = "macos"))]
fn entry(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("com.ai-toolkit.credits", id).map_err(|_| "系统凭据库不可用".into())
}
#[cfg(any(test, target_os = "macos"))]
fn is_legacy_account(id: &str, account: &str) -> bool {
    if account == id {
        return true;
    }
    account
        .strip_prefix(id)
        .and_then(|tail| tail.strip_prefix('/'))
        .and_then(|tail| tail.split_once('/'))
        .is_some_and(|(generation, index)| {
            uuid::Uuid::parse_str(generation).is_ok()
                && index.parse::<usize>().is_ok_and(|index| index < MAX_CHUNKS)
        })
}
fn chunk_id(id: &str, generation: &str, index: usize) -> String {
    format!("{id}/{generation}/{index}")
}
fn manifest(raw: &[u8]) -> Result<Option<(String, usize)>, String> {
    let raw = std::str::from_utf8(raw).map_err(|_| "系统凭据编码无效")?;
    let Some(raw) = raw.strip_prefix(PREFIX) else {
        return Ok(None);
    };
    let (generation, count) = raw.split_once(':').ok_or("系统凭据清单无效")?;
    let count: usize = count.parse().map_err(|_| "系统凭据清单无效")?;
    if uuid::Uuid::parse_str(generation).is_err() || !(1..=MAX_CHUNKS).contains(&count) {
        return Err("系统凭据清单无效".into());
    }
    Ok(Some((generation.into(), count)))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn seed(store: &SecretStore, id: &str, secret: &str) {
        let mut memory = store.memory.as_ref().unwrap().lock();
        match store.layout {
            Layout::Vault => {
                memory.values.insert(
                    VAULT_ACCOUNT.into(),
                    serde_json::to_vec(&serde_json::json!({id: secret})).unwrap(),
                );
            }
            Layout::Chunked => {
                memory.values.insert(id.into(), secret.as_bytes().to_vec());
            }
        }
    }
    fn legacy_proxy_manager() -> (std::sync::Arc<crate::credits::CreditManager>, &'static str) {
        let key = "ait_synthetic_preserved_proxy_key";
        let id = crate::credits::GATEWAY_API_KEY_ID;
        let legacy = SecretStore::memory_with_layout(Layout::Chunked);
        legacy.set(id, key).unwrap();
        legacy.set("old-account", "synthetic-legacy-account").unwrap();
        let store = SecretStore::memory_with_layout(Layout::Vault);
        {
            let mut memory = store.memory.as_ref().unwrap().lock();
            memory.values = legacy.memory.as_ref().unwrap().lock().values.clone();
            memory.protected = memory.values.keys().cloned().collect();
            memory.values.insert(
                VAULT_ACCOUNT.into(),
                br#"{"ready-account":"synthetic-ready-account"}"#.to_vec(),
            );
            memory.protected.insert(VAULT_ACCOUNT.into());
        }
        let mut manager = crate::credits::CreditManager::test();
        std::sync::Arc::get_mut(&mut manager).unwrap().secrets = store;
        (manager, key)
    }

    #[tokio::test]
    async fn denied_proxy_start_can_retry_shared_and_legacy_key_authorization_without_rotation() {
        for deny_shared_vault in [false, true] {
            let (manager, original_key) = legacy_proxy_manager();
            let store = &manager.secrets;
            if !deny_shared_vault {
                store.retry_access().unwrap();
            }
            store.memory.as_ref().unwrap().lock().read_error = Some("用户拒绝钥匙串授权".into());
            let error = manager.start_proxy_for_user(0).await.unwrap_err();
            assert!(error.contains("本地代理密钥"));
            assert!(!error.contains("重新登录"));
            assert!(!manager.gateway.status(0).running);
            let denied_reads = store.memory.as_ref().unwrap().lock().reads.len();
            let denied_prompts = store.memory.as_ref().unwrap().lock().prompts.len();
            for _ in 0..20 {
                assert!(manager.gateway_key().is_err());
            }
            {
                let mut memory = store.memory.as_ref().unwrap().lock();
                assert_eq!(memory.reads.len(), denied_reads);
                assert_eq!(memory.prompts.len(), denied_prompts);
                memory.read_error = None;
            }
            let status = manager.start_proxy_for_user(0).await.unwrap();
            assert!(status.running);
            assert_eq!(manager.gateway_key().unwrap(), original_key);
            assert_eq!(
                store.get("ready-account").unwrap().as_deref(),
                Some("synthetic-ready-account")
            );
            let response = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap()
                .get(format!("{}/models", status.base_url))
                .bearer_auth(original_key)
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            let successful_reads = store.memory.as_ref().unwrap().lock().reads.len();
            let successful_prompts = store.memory.as_ref().unwrap().lock().prompts.len();
            for _ in 0..20 {
                manager.retry_gateway_key_access().unwrap();
                assert_eq!(manager.gateway_key().unwrap(), original_key);
            }
            {
                let memory = store.memory.as_ref().unwrap().lock();
                assert_eq!(memory.reads.len(), successful_reads);
                assert_eq!(memory.prompts.len(), successful_prompts);
                assert!(memory.prompts.iter().all(|account| {
                    account == VAULT_ACCOUNT
                        || is_legacy_account(crate::credits::GATEWAY_API_KEY_ID, account)
                }));
                assert!(!memory
                    .reads
                    .iter()
                    .any(|account| is_legacy_account("old-account", account)));
                assert!(memory.values.contains_key("old-account"));
                assert!(!memory
                    .values
                    .keys()
                    .any(|account| is_legacy_account(crate::credits::GATEWAY_API_KEY_ID, account)));
            }
            manager.gateway.stop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn failed_proxy_key_migration_keeps_the_original_key_available_for_retry() {
        let (manager, original_key) = legacy_proxy_manager();
        let store = &manager.secrets;
        let original = store.memory.as_ref().unwrap().lock().values.clone();
        store.memory.as_ref().unwrap().lock().write_error = Some("synthetic write denial".into());
        assert!(manager.start_proxy_for_user(0).await.is_err());
        assert!(!manager.gateway.status(0).running);
        {
            let mut memory = store.memory.as_ref().unwrap().lock();
            assert_eq!(memory.values, original);
            memory.write_error = None;
        }
        assert!(manager.start_proxy_for_user(0).await.unwrap().running);
        assert_eq!(manager.gateway_key().unwrap(), original_key);
        manager.gateway.stop().await.unwrap();
    }

    #[test]
    fn long_secrets_round_trip_and_replace_atomically() {
        for layout in [Layout::Vault, Layout::Chunked] {
            let store = SecretStore::memory_with_layout(layout);
            let secret = "账号refreshJWT".repeat(600);
            store.set("account", &secret).unwrap();
            *store.gate.lock() = Cache::default();
            assert_eq!(
                store.get("account").unwrap().as_deref(),
                Some(secret.as_str())
            );
            store.set("account", "replacement").unwrap();
            assert_eq!(
                store.get("account").unwrap().as_deref(),
                Some("replacement")
            );
            store.delete("account").unwrap();
            assert!(store.get("account").unwrap().is_none());
        }
    }
    #[test]
    fn delete_is_idempotent_and_ids_are_isolated() {
        for layout in [Layout::Vault, Layout::Chunked] {
            let store = SecretStore::memory_with_layout(layout);
            store.set("a", "one").unwrap();
            store.set("b", "two").unwrap();
            assert_eq!(store.get("a").unwrap().as_deref(), Some("one"));
            store.delete("a").unwrap();
            store.delete("a").unwrap();
            assert!(store.get("a").unwrap().is_none());
            assert_eq!(store.get("b").unwrap().as_deref(), Some("two"));
        }
    }
    #[test]
    fn mac_vault_is_read_once_across_accounts_and_updates() {
        let store = SecretStore::memory_with_layout(Layout::Vault);
        store
            .memory
            .as_ref()
            .unwrap()
            .lock()
            .values
            .insert(VAULT_ACCOUNT.into(), br#"{"a":"one","b":"two"}"#.to_vec());
        for _ in 0..20 {
            assert_eq!(store.get("a").unwrap().as_deref(), Some("one"));
            assert_eq!(store.get("b").unwrap().as_deref(), Some("two"));
        }
        store.set("a", "new").unwrap();
        store.set("c", "three").unwrap();
        store.delete("b").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("new"));
        assert!(store.get("b").unwrap().is_none());
        assert_eq!(store.memory.as_ref().unwrap().lock().reads, [VAULT_ACCOUNT]);
    }
    #[test]
    fn rejected_access_stops_reads_and_writes_until_user_retries() {
        for layout in [Layout::Vault, Layout::Chunked] {
            let store = SecretStore::memory_with_layout(layout);
            seed(&store, "a", "token");
            store.memory.as_ref().unwrap().lock().read_error = Some("用户取消了凭据授权".into());
            assert!(store.get("a").is_err());
            for _ in 0..20 {
                assert!(store.get("a").is_err());
                assert!(store.get("b").is_err());
                assert!(store.set("b", "value").is_err());
                assert!(store.delete("b").is_err());
            }
            assert_eq!(store.memory.as_ref().unwrap().lock().reads.len(), 1);
            assert!(store.memory.as_ref().unwrap().lock().writes.is_empty());
            store.memory.as_ref().unwrap().lock().read_error = None;
            assert!(store.get("a").is_err());
            store.retry_access().unwrap();
            assert_eq!(store.get("a").unwrap().as_deref(), Some("token"));
            assert_eq!(store.memory.as_ref().unwrap().lock().reads.len(), 2);
        }
    }
    #[test]
    fn missing_credentials_are_cached_without_repeated_native_queries() {
        for layout in [Layout::Vault, Layout::Chunked] {
            let store = SecretStore::memory_with_layout(layout);
            assert!(store.get("missing").unwrap().is_none());
            let before = store.memory.as_ref().unwrap().lock().reads.len();
            for _ in 0..20 {
                assert!(store.get("missing").unwrap().is_none());
            }
            assert_eq!(store.memory.as_ref().unwrap().lock().reads.len(), before);
        }
    }
    #[test]
    fn unchanged_credentials_do_not_write_again_or_invalidate_cache() {
        for layout in [Layout::Vault, Layout::Chunked] {
            let store = SecretStore::memory_with_layout(layout);
            store.set("a", "token").unwrap();
            let before = store.memory.as_ref().unwrap().lock().writes.len();
            store.retry_access().unwrap(); // A successful session needs no fresh OS authorization.
            store.set("a", "token").unwrap();
            assert_eq!(store.memory.as_ref().unwrap().lock().writes.len(), before);
        }
    }
    #[test]
    fn failed_native_write_preserves_persisted_credentials() {
        for layout in [Layout::Vault, Layout::Chunked] {
            let store = SecretStore::memory_with_layout(layout);
            seed(&store, "a", "old");
            assert_eq!(store.get("a").unwrap().as_deref(), Some("old"));
            store.memory.as_ref().unwrap().lock().write_error = Some("凭据库已锁定".into());
            assert!(store.set("a", "new").is_err());
            assert!(store.get("a").is_err());
            store.memory.as_ref().unwrap().lock().write_error = None;
            store.retry_access().unwrap();
            assert_eq!(store.get("a").unwrap().as_deref(), Some("old"));
        }
    }
    #[test]
    fn legacy_chunks_migrate_once_and_are_not_reopened_by_token_updates() {
        let legacy = SecretStore::memory_with_layout(Layout::Chunked);
        let secret = "legacy-token".repeat(600);
        legacy.set("a", &secret).unwrap();
        let store = SecretStore::memory_with_layout(Layout::Vault);
        store.memory.as_ref().unwrap().lock().values =
            legacy.memory.as_ref().unwrap().lock().values.clone();
        assert_eq!(store.get("a").unwrap().as_deref(), Some(secret.as_str()));
        assert!(!store
            .memory
            .as_ref()
            .unwrap()
            .lock()
            .values
            .contains_key("a"));
        let before = store.memory.as_ref().unwrap().lock().reads.len();
        store.set("a", "refreshed-token").unwrap();
        assert_eq!(store.memory.as_ref().unwrap().lock().reads.len(), before);
        store.delete("a").unwrap();
        assert_eq!(store.memory.as_ref().unwrap().lock().reads.len(), before);
        *store.gate.lock() = Cache::default();
        assert!(store.get("a").unwrap().is_none());
    }
    #[test]
    fn concurrent_account_reads_share_one_os_vault_access() {
        let store = std::sync::Arc::new(SecretStore::memory_with_layout(Layout::Vault));
        seed(&store, "a", "token");
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    assert_eq!(store.get("a").unwrap().as_deref(), Some("token"))
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(store.memory.as_ref().unwrap().lock().reads.len(), 1);
    }

    #[test]
    fn startup_never_prompts_for_shared_vault_and_manual_refresh_authorizes_it_once() {
        let store = SecretStore::memory_with_layout(Layout::Vault);
        seed(&store, "a", "token");
        store
            .memory
            .as_ref()
            .unwrap()
            .lock()
            .protected
            .insert(VAULT_ACCOUNT.into());
        for _ in 0..20 {
            assert!(store.get("a").is_err());
            assert!(store.get("b").is_err());
        }
        {
            let memory = store.memory.as_ref().unwrap().lock();
            assert_eq!(memory.reads, [VAULT_ACCOUNT]);
            assert!(memory.prompts.is_empty());
        }
        store.retry_access().unwrap();
        for _ in 0..20 {
            assert_eq!(store.get("a").unwrap().as_deref(), Some("token"));
        }
        let memory = store.memory.as_ref().unwrap().lock();
        assert_eq!(memory.prompts, [VAULT_ACCOUNT]);
        assert_eq!(memory.reads, [VAULT_ACCOUNT, VAULT_ACCOUNT]);
    }

    #[test]
    fn protected_legacy_accounts_do_not_prompt_or_block_migrated_accounts() {
        let legacy = SecretStore::memory_with_layout(Layout::Chunked);
        for id in ["old-a", "old-b"] {
            legacy.set(id, &"legacy-token".repeat(600)).unwrap();
        }
        let store = SecretStore::memory_with_layout(Layout::Vault);
        {
            let mut memory = store.memory.as_ref().unwrap().lock();
            memory.values = legacy.memory.as_ref().unwrap().lock().values.clone();
            // The manifests may already be allowed while their chunks are not.
            memory.protected = memory
                .values
                .keys()
                .filter(|id| id.contains('/'))
                .cloned()
                .collect();
            memory
                .values
                .insert(VAULT_ACCOUNT.into(), br#"{"ready":"token"}"#.to_vec());
            memory.protected.insert(VAULT_ACCOUNT.into());
        }
        store.retry_access().unwrap();
        for _ in 0..20 {
            assert!(store.get("old-a").unwrap_err().contains("重新登录"));
            assert!(store.get("old-b").unwrap_err().contains("重新登录"));
            assert_eq!(store.get("ready").unwrap().as_deref(), Some("token"));
        }
        let memory = store.memory.as_ref().unwrap().lock();
        assert_eq!(memory.prompts, [VAULT_ACCOUNT]);
        // One vault read and, per legacy account, only its manifest and first chunk.
        assert_eq!(memory.reads.len(), 5);
        assert!(memory.writes.is_empty());
    }

    #[test]
    fn a_new_login_and_its_rollback_never_read_or_destroy_protected_legacy_entries() {
        let legacy = SecretStore::memory_with_layout(Layout::Chunked);
        legacy.set("a", &"old-token".repeat(600)).unwrap();
        let original = legacy.memory.as_ref().unwrap().lock().values.clone();
        let store = SecretStore::memory_with_layout(Layout::Vault);
        {
            let mut memory = store.memory.as_ref().unwrap().lock();
            memory.values = original.clone();
            memory.protected = original.keys().cloned().collect();
        }
        store.retry_access().unwrap();
        let previous = store.get_for_update("a").unwrap();
        assert!(previous.is_none());
        store.set("a", "new-verified-login").unwrap();
        store.restore("a", previous.as_deref()).unwrap();
        {
            let memory = store.memory.as_ref().unwrap().lock();
            assert_eq!(memory.reads, [VAULT_ACCOUNT]);
            assert!(memory.prompts.is_empty());
            for (id, raw) in &original {
                assert_eq!(memory.values.get(id), Some(raw));
            }
        }
        assert!(store.get("a").is_err());
        store.set("a", "new-verified-login").unwrap();
        assert_eq!(
            store.get("a").unwrap().as_deref(),
            Some("new-verified-login")
        );
    }

    #[test]
    fn deleting_a_vault_account_cleans_legacy_generations_without_reading_their_data() {
        let store = SecretStore::memory_with_layout(Layout::Vault);
        seed(&store, "a", "token");
        let generation = uuid::Uuid::new_v4().to_string();
        let old_generation = uuid::Uuid::new_v4().to_string();
        let targets = [
            "a".into(),
            chunk_id("a", &generation, 0),
            chunk_id("a", &old_generation, 1),
        ];
        let unrelated = [
            "ab".into(),
            chunk_id("ab", &generation, 0),
            "a/not-a-generation/0".into(),
        ];
        {
            let mut memory = store.memory.as_ref().unwrap().lock();
            for id in targets.iter().chain(&unrelated) {
                memory
                    .values
                    .insert(id.clone(), b"protected-legacy".to_vec());
                memory.protected.insert(id.clone());
            }
        }
        store.delete("a").unwrap();
        {
            let memory = store.memory.as_ref().unwrap().lock();
            assert_eq!(memory.reads, [VAULT_ACCOUNT]);
            assert!(memory.prompts.is_empty());
            assert!(targets.iter().all(|id| !memory.values.contains_key(id)));
            assert!(unrelated.iter().all(|id| memory.values.contains_key(id)));
        }
        *store.gate.lock() = Cache::default();
        assert!(store.get("a").unwrap().is_none());
    }

    #[test]
    fn manual_refresh_retries_a_denied_shared_vault_only_once_for_concurrent_accounts() {
        let store = std::sync::Arc::new(SecretStore::memory_with_layout(Layout::Vault));
        store
            .memory
            .as_ref()
            .unwrap()
            .lock()
            .protected
            .insert(VAULT_ACCOUNT.into());
        store.memory.as_ref().unwrap().lock().read_error = Some("用户拒绝".into());
        assert!(store.retry_access().is_err());
        let threads: Vec<_> = (0..8)
            .map(|index| {
                let store = store.clone();
                std::thread::spawn(move || assert!(store.get(&format!("account-{index}")).is_err()))
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let memory = store.memory.as_ref().unwrap().lock();
        assert_eq!(memory.reads, [VAULT_ACCOUNT]);
        assert_eq!(memory.prompts, [VAULT_ACCOUNT]);
    }
}
