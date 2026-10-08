//! Tokens live in one item of the OS credential store as a JSON map from account ID to token:
//! the Keychain on macOS, Credential Manager on Windows and the Secret Service on Linux.

use std::collections::HashMap;
use std::sync::Mutex;

/// Account name of the single item.
pub const ITEM_ACCOUNT: &str = "tokens";

/// Service name for the item.
pub fn service_name(bundle_id: &str) -> String {
    bundle_id.to_string()
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SecretError {
    #[error("Keychain access denied")]
    AccessDenied(String),
    #[error("Keychain write blocked after a failed load")]
    WriteBlocked,
    #[error("stored tokens are not valid: {0}")]
    Corrupt(String),
}

/// Backing store for the single item.
pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when the item does not exist.
    fn read(&self) -> Result<Option<String>, SecretError>;
    fn write(&self, value: &str) -> Result<(), SecretError>;
}

/// The OS credential store through the `keyring` crate.
pub struct KeychainStore {
    service: String,
}

impl KeychainStore {
    pub fn new(service: &str) -> KeychainStore {
        KeychainStore {
            service: service.to_string(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, ITEM_ACCOUNT)
            .map_err(|e| SecretError::AccessDenied(e.to_string()))
    }
}

impl SecretStore for KeychainStore {
    fn read(&self) -> Result<Option<String>, SecretError> {
        match self.entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::AccessDenied(e.to_string())),
        }
    }

    fn write(&self, value: &str) -> Result<(), SecretError> {
        self.entry()?
            .set_password(value)
            .map_err(|e| SecretError::AccessDenied(e.to_string()))
    }
}

/// Plain file store for debug builds. Every debug build carries a new ad-hoc signature, which
/// makes the Keychain deny access after each rebuild, so development uses a file instead.
pub struct FileStore {
    path: std::path::PathBuf,
}

impl FileStore {
    pub fn new(path: std::path::PathBuf) -> FileStore {
        FileStore { path }
    }
}

impl SecretStore for FileStore {
    fn read(&self) -> Result<Option<String>, SecretError> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => Ok(Some(raw)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SecretError::AccessDenied(e.to_string())),
        }
    }

    fn write(&self, value: &str) -> Result<(), SecretError> {
        use std::io::Write;
        let denied = |e: std::io::Error| SecretError::AccessDenied(e.to_string());
        let dir = self
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(dir).map_err(denied)?;
        // On Unix, temp files are created with mode 0600, so the tokens are never readable by others.
        let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(denied)?;
        temp.write_all(value.as_bytes()).map_err(denied)?;
        temp.as_file().sync_all().map_err(denied)?;
        temp.persist(&self.path).map_err(|e| denied(e.error))?;
        Ok(())
    }
}

/// In-memory store for tests.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct MemoryStore {
    value: Mutex<Option<String>>,
    pub fail_reads: Mutex<bool>,
}

#[cfg(any(test, feature = "test-support"))]
impl MemoryStore {
    pub fn with_value(value: &str) -> MemoryStore {
        MemoryStore {
            value: Mutex::new(Some(value.to_string())),
            fail_reads: Mutex::new(false),
        }
    }

    pub fn failing() -> MemoryStore {
        MemoryStore {
            value: Mutex::new(None),
            fail_reads: Mutex::new(true),
        }
    }

    pub fn value(&self) -> Option<String> {
        self.value.lock().unwrap().clone()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl SecretStore for MemoryStore {
    fn read(&self) -> Result<Option<String>, SecretError> {
        if *self.fail_reads.lock().unwrap() {
            return Err(SecretError::AccessDenied("denied".into()));
        }
        Ok(self.value.lock().unwrap().clone())
    }

    fn write(&self, value: &str) -> Result<(), SecretError> {
        *self.value.lock().unwrap() = Some(value.to_string());
        Ok(())
    }
}

/// Outcome of the last load. A failure carries a message that never quotes the item's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Loaded,
    NotFound,
    Failed(String),
}

impl std::fmt::Display for LoadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadState::Loaded => f.write_str("loaded"),
            LoadState::NotFound => f.write_str("not found"),
            LoadState::Failed(reason) => write!(f, "failed: {reason}"),
        }
    }
}

/// Describes a JSON error by its category and position only. serde_json's own message can
/// quote part of the input, which here is a token.
fn describe_json_error(error: &serde_json::Error) -> String {
    let category = match error.classify() {
        serde_json::error::Category::Io => "I/O",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "unexpected end",
    };
    format!(
        "{category} error at line {}, column {}",
        error.line(),
        error.column()
    )
}

/// The token map, loaded once and kept in memory.
pub struct Secrets {
    store: Box<dyn SecretStore>,
    tokens: Mutex<HashMap<String, String>>,
    state: Mutex<LoadState>,
    /// Serializes writers so the store write can run without holding `tokens`.
    write_lock: Mutex<()>,
}

impl Secrets {
    pub fn new(store: Box<dyn SecretStore>) -> Secrets {
        let secrets = Secrets {
            store,
            tokens: Mutex::new(HashMap::new()),
            state: Mutex::new(LoadState::NotFound),
            write_lock: Mutex::new(()),
        };
        secrets.load();
        secrets
    }

    /// Reads the item. After a failure every write is blocked until a reload succeeds.
    pub fn load(&self) -> LoadState {
        let state = match self.store.read() {
            Ok(Some(raw)) => match serde_json::from_str::<HashMap<String, String>>(&raw) {
                Ok(map) => {
                    *self.tokens.lock().unwrap() = map;
                    LoadState::Loaded
                }
                Err(e) => {
                    LoadState::Failed(SecretError::Corrupt(describe_json_error(&e)).to_string())
                }
            },
            Ok(None) => {
                self.tokens.lock().unwrap().clear();
                LoadState::NotFound
            }
            Err(e) => LoadState::Failed(e.to_string()),
        };
        *self.state.lock().unwrap() = state.clone();
        state
    }

    pub fn state(&self) -> LoadState {
        self.state.lock().unwrap().clone()
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self.state(), LoadState::Failed(_))
    }

    pub fn token(&self, account_id: &str) -> Option<String> {
        self.tokens.lock().unwrap().get(account_id).cloned()
    }

    pub fn set_token(&self, account_id: &str, token: &str) -> Result<(), SecretError> {
        self.write_with(|map| {
            map.insert(account_id.to_string(), token.to_string());
        })
    }

    pub fn remove_token(&self, account_id: &str) -> Result<(), SecretError> {
        self.write_with(|map| {
            map.remove(account_id);
        })
    }

    /// Overwrites the item with an empty map and clears a blocked state. Discards every token.
    pub fn reset(&self) -> Result<(), SecretError> {
        let _writer = self.write_lock.lock().unwrap();
        self.store.write("{}")?;
        self.tokens.lock().unwrap().clear();
        *self.state.lock().unwrap() = LoadState::Loaded;
        Ok(())
    }

    /// Drops tokens whose account ID is not in `keep`. Returns how many were removed. Does
    /// nothing while writes are blocked.
    pub fn prune(&self, keep: &[String]) -> Result<usize, SecretError> {
        let _writer = self.write_lock.lock().unwrap();
        if self.is_blocked() {
            return Ok(0);
        }
        let mut next = self.tokens.lock().unwrap().clone();
        let before = next.len();
        next.retain(|id, _| keep.contains(id));
        let removed = before - next.len();
        if removed > 0 {
            self.commit(next)?;
        }
        Ok(removed)
    }

    fn write_with(
        &self,
        change: impl FnOnce(&mut HashMap<String, String>),
    ) -> Result<(), SecretError> {
        let _writer = self.write_lock.lock().unwrap();
        if self.is_blocked() {
            return Err(SecretError::WriteBlocked);
        }
        let mut next = self.tokens.lock().unwrap().clone();
        change(&mut next);
        self.commit(next)
    }

    /// Writes `next` to the store, then publishes it. The caller holds `write_lock`.
    fn commit(&self, next: HashMap<String, String>) -> Result<(), SecretError> {
        let raw = serde_json::to_string(&next)
            .map_err(|e| SecretError::Corrupt(describe_json_error(&e)))?;
        self.store.write(&raw)?;
        *self.tokens.lock().unwrap() = next;
        *self.state.lock().unwrap() = LoadState::Loaded;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_item_starts_empty_and_first_save_creates_it() {
        let store = Box::new(MemoryStore::default());
        let secrets = Secrets::new(store);
        assert_eq!(secrets.state(), LoadState::NotFound);
        secrets.set_token("a", "tok").unwrap();
        assert_eq!(secrets.token("a").as_deref(), Some("tok"));
        assert_eq!(secrets.state(), LoadState::Loaded);
    }

    #[test]
    fn loads_existing_map() {
        let store = Box::new(MemoryStore::with_value(r#"{"a":"x","b":"y"}"#));
        let secrets = Secrets::new(store);
        assert_eq!(secrets.state(), LoadState::Loaded);
        assert_eq!(secrets.token("b").as_deref(), Some("y"));
        secrets.remove_token("a").unwrap();
        assert!(secrets.token("a").is_none());
    }

    #[test]
    fn failed_load_blocks_writes_until_retry_succeeds() {
        let store = MemoryStore::failing();
        let secrets = Secrets::new(Box::new(store));
        assert!(matches!(secrets.state(), LoadState::Failed(_)));
        assert!(secrets.is_blocked());
        assert_eq!(
            secrets.set_token("a", "tok"),
            Err(SecretError::WriteBlocked)
        );

        // A later successful load clears the block.
        let store = MemoryStore::failing();
        *store.fail_reads.lock().unwrap() = false;
        let secrets = Secrets::new(Box::new(store));
        assert_eq!(secrets.state(), LoadState::NotFound);
        assert!(!secrets.is_blocked());
    }

    #[test]
    fn corrupt_item_is_a_failed_load() {
        let store = Box::new(MemoryStore::with_value("not json"));
        let secrets = Secrets::new(store);
        assert!(secrets.is_blocked());
    }

    #[test]
    fn reset_clears_a_failed_state() {
        let store = Box::new(MemoryStore::with_value("not json"));
        let secrets = Secrets::new(store);
        assert!(secrets.is_blocked());
        secrets.reset().unwrap();
        assert!(!secrets.is_blocked());
        assert_eq!(secrets.state(), LoadState::Loaded);
        secrets.set_token("a", "tok").unwrap();
        assert_eq!(secrets.token("a").as_deref(), Some("tok"));
    }

    #[test]
    fn prune_removes_orphan_tokens_only() {
        let store = Box::new(MemoryStore::with_value(r#"{"a":"x","b":"y","c":"z"}"#));
        let secrets = Secrets::new(store);
        let removed = secrets.prune(&["b".to_string()]).unwrap();
        assert_eq!(removed, 2);
        assert!(secrets.token("a").is_none());
        assert_eq!(secrets.token("b").as_deref(), Some("y"));
        assert!(secrets.token("c").is_none());
    }

    #[test]
    fn prune_skips_a_blocked_store() {
        let store = Box::new(MemoryStore::with_value("not json"));
        let secrets = Secrets::new(store);
        assert_eq!(secrets.prune(&[]).unwrap(), 0);
        assert!(secrets.is_blocked());
    }

    struct ScriptedStore {
        read: Result<Option<String>, SecretError>,
        write: Result<(), SecretError>,
    }

    impl SecretStore for ScriptedStore {
        fn read(&self) -> Result<Option<String>, SecretError> {
            self.read.clone()
        }

        fn write(&self, _value: &str) -> Result<(), SecretError> {
            self.write.clone()
        }
    }

    #[test]
    fn service_name_is_the_bundle_id() {
        assert_eq!(service_name("test.example.vigia"), "test.example.vigia");
    }

    #[test]
    fn memory_store_with_value_exposes_it() {
        let store = MemoryStore::with_value("{}");
        assert_eq!(store.value().as_deref(), Some("{}"));
        assert_eq!(MemoryStore::default().value(), None);
    }

    #[test]
    fn failing_memory_store_denies_reads_but_accepts_writes() {
        let store = MemoryStore::failing();
        assert_eq!(
            store.read(),
            Err(SecretError::AccessDenied("denied".into()))
        );
        store.write("x").unwrap();
        assert_eq!(store.value().as_deref(), Some("x"));
    }

    #[test]
    fn a_failed_read_reports_the_error_text_in_the_state() {
        let secrets = Secrets::new(Box::new(MemoryStore::failing()));
        assert_eq!(
            secrets.state(),
            LoadState::Failed("Keychain access denied".into())
        );
    }

    #[test]
    fn a_corrupt_item_reports_why_in_the_state() {
        let secrets = Secrets::new(Box::new(MemoryStore::with_value("[1]")));
        match secrets.state() {
            LoadState::Failed(message) => {
                assert!(message.starts_with("stored tokens are not valid"))
            }
            other => panic!("unexpected state {other:?}"),
        }
    }

    #[test]
    fn a_corrupt_item_never_quotes_its_content() {
        let secret = "fabricated-token-value";
        let secrets = Secrets::new(Box::new(MemoryStore::with_value(&format!("\"{secret}\""))));
        let state = secrets.state();
        let shown = format!("{state} {state:?}");
        assert!(!shown.contains(secret), "{shown}");
        assert!(shown.contains("line 1, column"), "{shown}");
        assert!(state
            .to_string()
            .starts_with("failed: stored tokens are not valid"));
    }

    #[test]
    fn load_states_display_their_kind() {
        assert_eq!(LoadState::Loaded.to_string(), "loaded");
        assert_eq!(LoadState::NotFound.to_string(), "not found");
        assert_eq!(
            LoadState::Failed("Keychain access denied".into()).to_string(),
            "failed: Keychain access denied"
        );
    }

    #[test]
    fn reload_after_the_item_disappears_clears_tokens() {
        let secrets = Secrets::new(Box::new(ScriptedStore {
            read: Ok(None),
            write: Ok(()),
        }));
        secrets.set_token("a", "tok").unwrap();
        assert_eq!(secrets.load(), LoadState::NotFound);
        assert!(secrets.token("a").is_none());
    }

    #[test]
    fn a_failed_write_keeps_the_previous_tokens() {
        let secrets = Secrets::new(Box::new(ScriptedStore {
            read: Ok(Some(r#"{"a":"x"}"#.into())),
            write: Err(SecretError::AccessDenied("no".into())),
        }));
        assert_eq!(
            secrets.set_token("b", "y"),
            Err(SecretError::AccessDenied("no".into()))
        );
        assert_eq!(
            secrets.remove_token("a"),
            Err(SecretError::AccessDenied("no".into()))
        );
        assert_eq!(
            secrets.prune(&[]),
            Err(SecretError::AccessDenied("no".into()))
        );
        assert_eq!(secrets.reset(), Err(SecretError::AccessDenied("no".into())));
        assert_eq!(secrets.token("a").as_deref(), Some("x"));
        assert!(secrets.token("b").is_none());
    }

    #[test]
    fn blocked_secrets_reject_every_write_and_keep_the_block() {
        let secrets = Secrets::new(Box::new(MemoryStore::failing()));
        assert_eq!(secrets.remove_token("a"), Err(SecretError::WriteBlocked));
        assert!(secrets.is_blocked());
    }

    #[test]
    fn prune_keeps_everything_when_nothing_is_orphaned() {
        let secrets = Secrets::new(Box::new(MemoryStore::with_value(r#"{"a":"x"}"#)));
        assert_eq!(secrets.prune(&["a".to_string()]).unwrap(), 0);
        assert_eq!(secrets.token("a").as_deref(), Some("x"));
    }

    #[test]
    fn file_store_reports_unreadable_and_unwritable_paths() {
        let dir = tempfile::tempdir().unwrap();
        // A directory in place of the file cannot be read as a string.
        let as_dir = FileStore::new(dir.path().to_path_buf());
        assert!(matches!(as_dir.read(), Err(SecretError::AccessDenied(_))));
        assert!(matches!(
            as_dir.write("x"),
            Err(SecretError::AccessDenied(_))
        ));
        // A regular file in place of the parent directory cannot be created into.
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let under_file = FileStore::new(file.join("tokens.json"));
        assert!(matches!(
            under_file.write("x"),
            Err(SecretError::AccessDenied(_))
        ));
    }

    #[test]
    fn keychain_store_without_a_valid_item_name_is_denied_before_any_access() {
        let store = KeychainStore::new("");
        assert!(matches!(store.read(), Err(SecretError::AccessDenied(_))));
        assert!(matches!(
            store.write("x"),
            Err(SecretError::AccessDenied(_))
        ));
    }

    #[test]
    fn file_store_round_trips_with_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("tokens.json");
        let store = FileStore::new(path.clone());
        assert_eq!(store.read().unwrap(), None);
        store.write(r#"{"a":"x"}"#).unwrap();
        assert_eq!(store.read().unwrap().as_deref(), Some(r#"{"a":"x"}"#));
        // Windows has no mode bits; the file takes the access list of its folder.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
