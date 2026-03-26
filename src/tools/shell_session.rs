// Shell session persistence — track env/cwd across sequential shell commands

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

const MAX_HISTORY: usize = 20;

const SENSITIVE_PATTERNS: &[&str] = &[
    "KEY", "SECRET", "TOKEN", "PASSWORD", "CREDENTIAL", "_AUTH", "API_KEY",
];

#[derive(Debug, Clone)]
pub struct ShellSession {
    pub id: String,
    pub working_dir: PathBuf,
    pub env_vars: HashMap<String, String>,
    pub last_used: Instant,
    pub history: Vec<String>,
}

pub struct ShellSessionManager {
    pub sessions: Mutex<HashMap<String, ShellSession>>,
    default_cwd: PathBuf,
}

impl ShellSessionManager {
    pub fn new(default_cwd: PathBuf) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            default_cwd,
        }
    }

    pub fn get_or_create(&self, session_id: &str) -> ShellSession {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.entry(session_id.to_string()).or_insert_with(|| ShellSession {
            id: session_id.to_string(),
            working_dir: self.default_cwd.clone(),
            env_vars: HashMap::new(),
            last_used: Instant::now(),
            history: Vec::new(),
        }).clone()
    }

    pub fn reset(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session_id.to_string(), ShellSession {
            id: session_id.to_string(),
            working_dir: self.default_cwd.clone(),
            env_vars: HashMap::new(),
            last_used: Instant::now(),
            history: Vec::new(),
        });
    }

    pub fn update_session(
        &self,
        session_id: &str,
        new_cwd: Option<PathBuf>,
        new_env: &HashMap<String, String>,
        command: &str,
    ) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            if let Some(cwd) = new_cwd {
                session.working_dir = cwd;
            }
            let filtered = filter_sensitive_env(new_env);
            if !filtered.is_empty() {
                session.env_vars = filtered;
            }
            session.last_used = Instant::now();
            session.history.push(command.to_string());
            if session.history.len() > MAX_HISTORY {
                session.history.drain(0..session.history.len() - MAX_HISTORY);
            }
        }
    }

    pub fn cleanup_idle(&self, threshold_secs: u64) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, s| s.last_used.elapsed().as_secs() < threshold_secs);
    }

    pub fn state_capture_suffix() -> &'static str {
        if cfg!(target_os = "windows") {
            " & echo ___PHANTOM_MESH_CWD___ & cd & echo ___PHANTOM_MESH_ENV___ & set"
        } else {
            "; echo '___PHANTOM_MESH_CWD___'; pwd; echo '___PHANTOM_MESH_ENV___'; env -0"
        }
    }
}

pub fn parse_state_capture(raw_output: &str) -> (String, Option<PathBuf>, HashMap<String, String>) {
    let cwd_marker = "___PHANTOM_MESH_CWD___";
    let env_marker = "___PHANTOM_MESH_ENV___";

    let cwd_pos = match raw_output.find(cwd_marker) {
        Some(pos) => pos,
        None => return (raw_output.to_string(), None, HashMap::new()),
    };

    let user_output = raw_output[..cwd_pos].to_string();
    let after_cwd = raw_output[cwd_pos + cwd_marker.len()..].trim_start_matches('\n').trim_start_matches('\r');

    let env_pos = match after_cwd.find(env_marker) {
        Some(pos) => pos,
        None => return (user_output, None, HashMap::new()),
    };

    let cwd_str = after_cwd[..env_pos].trim();
    let cwd = if cwd_str.is_empty() { None } else { Some(PathBuf::from(cwd_str)) };

    let env_str = after_cwd[env_pos + env_marker.len()..].trim_start_matches('\n').trim_start_matches('\r');
    let mut env = HashMap::new();

    // Handle both null-delimited (env -0 on Unix) and newline-delimited (Windows set)
    let entries: Vec<&str> = if env_str.contains('\0') {
        env_str.split('\0').collect()
    } else {
        env_str.lines().collect()
    };
    for entry in entries {
        if let Some((key, val)) = entry.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                env.insert(key.to_string(), val.to_string());
            }
        }
    }

    (user_output, cwd, env)
}

pub fn filter_sensitive_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    env.iter()
        .filter(|(key, _)| {
            let upper = key.to_uppercase();
            !SENSITIVE_PATTERNS.iter().any(|p| upper.contains(p))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_session() {
        let mgr = ShellSessionManager::new(std::path::PathBuf::from("/tmp"));
        let session = mgr.get_or_create("test1");
        assert_eq!(session.working_dir, std::path::PathBuf::from("/tmp"));
        assert!(session.env_vars.is_empty());
    }

    #[test]
    fn test_session_isolation() {
        let mgr = ShellSessionManager::new(std::path::PathBuf::from("/tmp"));
        let s1 = mgr.get_or_create("a");
        let s2 = mgr.get_or_create("b");
        assert_eq!(s1.id, "a");
        assert_eq!(s2.id, "b");
    }

    #[test]
    fn test_parse_state_capture_unix() {
        let output = "hello world\n___PHANTOM_MESH_CWD___\n/tmp/subdir\n___PHANTOM_MESH_ENV___\nFOO=bar\nBAZ=qux\n";
        let (user_output, cwd, env) = parse_state_capture(output);
        assert_eq!(user_output, "hello world\n");
        assert_eq!(cwd, Some(std::path::PathBuf::from("/tmp/subdir")));
        assert_eq!(env.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(env.get("BAZ"), Some(&"qux".to_string()));
    }

    #[test]
    fn test_parse_state_capture_no_markers() {
        let output = "just normal output\n";
        let (user_output, cwd, env) = parse_state_capture(output);
        assert_eq!(user_output, "just normal output\n");
        assert!(cwd.is_none());
        assert!(env.is_empty());
    }

    #[test]
    fn test_parse_state_capture_null_delimited() {
        // Unix env -0 output
        let output = "output\n___PHANTOM_MESH_CWD___\n/home\n___PHANTOM_MESH_ENV___\nFOO=bar\0BAZ=qux\0";
        let (user_output, cwd, env) = parse_state_capture(output);
        assert_eq!(user_output, "output\n");
        assert_eq!(cwd, Some(std::path::PathBuf::from("/home")));
        assert_eq!(env.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(env.get("BAZ"), Some(&"qux".to_string()));
    }

    #[test]
    fn test_env_safety_filter() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        env.insert("SECRET_KEY".to_string(), "supersecret".to_string());
        env.insert("API_KEY".to_string(), "key123".to_string());
        env.insert("HOME".to_string(), "/home/user".to_string());
        env.insert("AWS_TOKEN".to_string(), "tok".to_string());
        env.insert("DB_PASSWORD".to_string(), "pass".to_string());
        env.insert("MY_AUTH_HEADER".to_string(), "bearer".to_string());
        env.insert("CREDENTIAL_FILE".to_string(), "/path".to_string());

        let filtered = filter_sensitive_env(&env);
        assert!(filtered.contains_key("PATH"));
        assert!(filtered.contains_key("HOME"));
        assert!(!filtered.contains_key("SECRET_KEY"));
        assert!(!filtered.contains_key("API_KEY"));
        assert!(!filtered.contains_key("AWS_TOKEN"));
        assert!(!filtered.contains_key("DB_PASSWORD"));
        assert!(!filtered.contains_key("MY_AUTH_HEADER"));
        assert!(!filtered.contains_key("CREDENTIAL_FILE"));
    }

    #[test]
    fn test_reset_session() {
        let mgr = ShellSessionManager::new(std::path::PathBuf::from("/tmp"));
        {
            let mut sessions = mgr.sessions.lock().unwrap();
            let mut env = HashMap::new();
            env.insert("FOO".to_string(), "bar".to_string());
            sessions.insert("test".to_string(), ShellSession {
                id: "test".to_string(),
                working_dir: std::path::PathBuf::from("/some/dir"),
                env_vars: env,
                last_used: Instant::now(),
                history: vec!["ls".to_string()],
            });
        }
        mgr.reset("test");
        let session = mgr.get_or_create("test");
        assert_eq!(session.working_dir, std::path::PathBuf::from("/tmp"));
        assert!(session.env_vars.is_empty());
        assert!(session.history.is_empty());
    }

    #[test]
    fn test_idle_cleanup() {
        let mgr = ShellSessionManager::new(std::path::PathBuf::from("/tmp"));
        {
            let mut sessions = mgr.sessions.lock().unwrap();
            sessions.insert("old".to_string(), ShellSession {
                id: "old".to_string(),
                working_dir: std::path::PathBuf::from("/tmp"),
                env_vars: HashMap::new(),
                last_used: Instant::now().checked_sub(std::time::Duration::from_secs(3600)).unwrap(),
                history: vec![],
            });
            sessions.insert("fresh".to_string(), ShellSession {
                id: "fresh".to_string(),
                working_dir: std::path::PathBuf::from("/tmp"),
                env_vars: HashMap::new(),
                last_used: Instant::now(),
                history: vec![],
            });
        }
        mgr.cleanup_idle(1800);
        let sessions = mgr.sessions.lock().unwrap();
        assert!(!sessions.contains_key("old"));
        assert!(sessions.contains_key("fresh"));
    }

    #[test]
    fn test_update_session_caps_history() {
        let mgr = ShellSessionManager::new(std::path::PathBuf::from("/tmp"));
        mgr.get_or_create("test");
        for i in 0..25 {
            mgr.update_session("test", None, &HashMap::new(), &format!("cmd_{}", i));
        }
        let sessions = mgr.sessions.lock().unwrap();
        let session = sessions.get("test").unwrap();
        assert!(session.history.len() <= 20);
    }

    #[test]
    fn test_windows_state_capture() {
        let output = "file1.txt\nfile2.txt\n___PHANTOM_MESH_CWD___\nC:\\Users\\test\n___PHANTOM_MESH_ENV___\nPATH=C:\\Windows\nHOME=C:\\Users\\test\n";
        let (user_output, cwd, env) = parse_state_capture(output);
        assert_eq!(user_output, "file1.txt\nfile2.txt\n");
        assert_eq!(cwd, Some(std::path::PathBuf::from("C:\\Users\\test")));
        assert_eq!(env.get("PATH"), Some(&"C:\\Windows".to_string()));
    }
}
