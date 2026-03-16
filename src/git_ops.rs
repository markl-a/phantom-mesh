//! Git operations — branch-per-task isolation for agent file operations.
//!
//! Agents can create isolated branches for tasks, commit changes,
//! and merge back to the main branch when the task is complete.

use anyhow::Result;
use std::path::Path;
use std::process::Command;
use tracing::{debug, info, warn};

/// Git branch manager for task isolation
pub struct GitBranch {
    repo_path: String,
}

impl GitBranch {
    pub fn new(repo_path: &str) -> Self {
        Self {
            repo_path: repo_path.to_string(),
        }
    }

    /// Check if the path is a git repository
    pub fn is_repo(&self) -> bool {
        Path::new(&self.repo_path).join(".git").exists()
    }

    /// Get the current branch name
    pub fn current_branch(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to get current branch: {}", String::from_utf8_lossy(&output.stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Create and checkout a new branch for a task
    pub fn create_task_branch(&self, task_id: &str) -> Result<String> {
        let branch_name = format!("task/{}", sanitize_branch_name(task_id));

        let output = Command::new("git")
            .args(["checkout", "-b", &branch_name])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            // Branch might already exist
            if err.contains("already exists") {
                info!("Branch '{}' already exists, checking out", branch_name);
                self.checkout(&branch_name)?;
            } else {
                anyhow::bail!("Failed to create branch '{}': {}", branch_name, err);
            }
        } else {
            info!("Created task branch: {}", branch_name);
        }

        Ok(branch_name)
    }

    /// Checkout an existing branch
    pub fn checkout(&self, branch: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["checkout", branch])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to checkout '{}': {}",
                branch, String::from_utf8_lossy(&output.stderr));
        }
        debug!("Checked out branch: {}", branch);
        Ok(())
    }

    /// Stage all changes and commit
    pub fn commit_all(&self, message: &str) -> Result<String> {
        // Stage all changes
        let output = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to stage changes: {}", String::from_utf8_lossy(&output.stderr));
        }

        // Check if there are changes to commit
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.repo_path)
            .output()?;

        let status_text = String::from_utf8_lossy(&status.stdout);
        if status_text.trim().is_empty() {
            return Ok("No changes to commit".to_string());
        }

        // Commit
        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to commit: {}", String::from_utf8_lossy(&output.stderr));
        }

        // Get the commit hash
        let hash_output = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&self.repo_path)
            .output()?;

        let hash = String::from_utf8_lossy(&hash_output.stdout).trim().to_string();
        info!("Committed: {} ({})", message, hash);
        Ok(hash)
    }

    /// Merge a task branch back into the target branch
    pub fn merge_task(&self, task_branch: &str, target_branch: &str) -> Result<()> {
        // Switch to target branch
        self.checkout(target_branch)?;

        // Merge
        let output = Command::new("git")
            .args(["merge", "--no-ff", task_branch, "-m",
                   &format!("Merge task branch '{}'", task_branch)])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            warn!("Merge conflict on '{}': {}", task_branch, err);
            // Abort the merge
            let _ = Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(&self.repo_path)
                .output();
            anyhow::bail!("Merge conflict: {}", err);
        }

        info!("Merged '{}' into '{}'", task_branch, target_branch);
        Ok(())
    }

    /// Delete a task branch (after successful merge)
    pub fn delete_branch(&self, branch: &str) -> Result<()> {
        if branch == "main" || branch == "master" {
            anyhow::bail!("Refusing to delete protected branch: {}", branch);
        }

        let output = Command::new("git")
            .args(["branch", "-d", branch])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to delete branch '{}': {}",
                branch, String::from_utf8_lossy(&output.stderr));
        }
        debug!("Deleted branch: {}", branch);
        Ok(())
    }

    /// List all task branches
    pub fn list_task_branches(&self) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["branch", "--list", "task/*"])
            .current_dir(&self.repo_path)
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let branches = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.trim().trim_start_matches("* ").to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(branches)
    }

    /// Get the diff of changes on the current branch vs target
    pub fn diff_from(&self, base_branch: &str) -> Result<String> {
        let output = Command::new("git")
            .args(["diff", &format!("{}..HEAD", base_branch)])
            .current_dir(&self.repo_path)
            .output()?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Sanitize a string for use as a git branch name
fn sanitize_branch_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_branch_name() {
        assert_eq!(sanitize_branch_name("hello world"), "hello-world");
        assert_eq!(sanitize_branch_name("fix/bug#123"), "fix-bug-123");
        assert_eq!(sanitize_branch_name("simple"), "simple");
        assert_eq!(sanitize_branch_name("a b c"), "a-b-c");
        assert_eq!(sanitize_branch_name("under_score"), "under_score");
    }

    #[test]
    fn test_git_branch_not_repo() {
        let dir = tempfile::tempdir().unwrap();
        let gb = GitBranch::new(dir.path().to_str().unwrap());
        assert!(!gb.is_repo());
    }

    #[test]
    fn test_git_branch_init_repo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();

        // Init a git repo
        let _ = Command::new("git").args(["init"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(path).output();

        // Create initial commit
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        let _ = Command::new("git").args(["add", "."]).current_dir(path).output();
        let _ = Command::new("git").args(["commit", "-m", "initial"]).current_dir(path).output();

        let gb = GitBranch::new(path);
        assert!(gb.is_repo());

        let branch = gb.current_branch().unwrap();
        // Git default branch could be "main" or "master"
        assert!(branch == "main" || branch == "master", "Got: {}", branch);
    }

    #[test]
    fn test_create_task_branch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();

        // Init repo with initial commit
        let _ = Command::new("git").args(["init"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(path).output();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        let _ = Command::new("git").args(["add", "."]).current_dir(path).output();
        let _ = Command::new("git").args(["commit", "-m", "initial"]).current_dir(path).output();

        let gb = GitBranch::new(path);
        let branch = gb.create_task_branch("write-article-123").unwrap();
        assert_eq!(branch, "task/write-article-123");
        assert_eq!(gb.current_branch().unwrap(), "task/write-article-123");
    }

    #[test]
    fn test_commit_and_merge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();

        // Init repo
        let _ = Command::new("git").args(["init"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(path).output();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        let _ = Command::new("git").args(["add", "."]).current_dir(path).output();
        let _ = Command::new("git").args(["commit", "-m", "initial"]).current_dir(path).output();

        let main_branch = {
            let gb = GitBranch::new(path);
            gb.current_branch().unwrap()
        };

        let gb = GitBranch::new(path);

        // Create task branch
        let task_branch = gb.create_task_branch("test-task").unwrap();

        // Make changes
        std::fs::write(dir.path().join("output.txt"), "Task output").unwrap();
        let hash = gb.commit_all("Task completed").unwrap();
        assert!(!hash.is_empty());
        assert_ne!(hash, "No changes to commit");

        // Merge back
        gb.merge_task(&task_branch, &main_branch).unwrap();
        assert_eq!(gb.current_branch().unwrap(), main_branch);

        // output.txt should exist on main now
        assert!(dir.path().join("output.txt").exists());

        // Clean up branch
        gb.delete_branch(&task_branch).unwrap();
    }

    #[test]
    fn test_no_changes_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();

        let _ = Command::new("git").args(["init"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(path).output();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        let _ = Command::new("git").args(["add", "."]).current_dir(path).output();
        let _ = Command::new("git").args(["commit", "-m", "initial"]).current_dir(path).output();

        let gb = GitBranch::new(path);
        let result = gb.commit_all("Nothing to do").unwrap();
        assert_eq!(result, "No changes to commit");
    }

    #[test]
    fn test_protected_branch_delete() {
        let gb = GitBranch::new("/tmp/fake");
        assert!(gb.delete_branch("main").is_err());
        assert!(gb.delete_branch("master").is_err());
    }

    #[test]
    fn test_list_task_branches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();

        let _ = Command::new("git").args(["init"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(path).output();
        let _ = Command::new("git").args(["config", "user.name", "Test"]).current_dir(path).output();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        let _ = Command::new("git").args(["add", "."]).current_dir(path).output();
        let _ = Command::new("git").args(["commit", "-m", "initial"]).current_dir(path).output();

        let gb = GitBranch::new(path);
        gb.create_task_branch("task-a").unwrap();
        gb.checkout(&gb.current_branch().unwrap()).ok(); // stay on task/task-a
        // Go back to main to create another branch
        let _main = "master"; // or "main" depending on default
        let _ = Command::new("git").args(["checkout", "-b", "main"]).current_dir(path).output();
        let _ = gb.create_task_branch("task-b");

        let branches = gb.list_task_branches().unwrap();
        assert!(branches.len() >= 1); // at least task-a
    }
}
