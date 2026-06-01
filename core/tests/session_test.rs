/// Integration tests for ConversationStore session management:
/// disk persistence, markdown export, and session search.
use phantom_mesh::{providers::traits::ChatMessage, session::ConversationStore};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn msg(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: role.into(),
        content: content.into(),
        tool_calls: None,
    }
}

// ---------------------------------------------------------------------------
// 1. Disk persistence — survives a fresh ConversationStore pointing at the
//    same directory.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_conversation_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();

    // Write messages with one store instance.
    {
        let store = ConversationStore::new_with_dir(path.clone());
        store
            .append(
                "persist_session",
                msg("user", "first message"),
                msg("assistant", "first reply"),
            )
            .await;
        store
            .append(
                "persist_session",
                msg("user", "second message"),
                msg("assistant", "second reply"),
            )
            .await;
    }

    // Open a brand-new store pointing at the same directory.
    let store2 = ConversationStore::new_with_dir(path);
    let history = store2.get_history("persist_session").await;

    assert_eq!(
        history.len(),
        4,
        "all 4 messages should be reloaded from disk"
    );
    assert_eq!(history[0].content, "first message");
    assert_eq!(history[1].content, "first reply");
    assert_eq!(history[2].content, "second message");
    assert_eq!(history[3].content, "second reply");
}

// ---------------------------------------------------------------------------
// 2. Markdown export — verify structural markers in the output.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_export_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    store
        .append(
            "md_session",
            msg("user", "What is Rust?"),
            msg("assistant", "A systems programming language."),
        )
        .await;

    let markdown = store.export_markdown("md_session").await;

    // Should open with the session heading.
    assert!(
        markdown.contains("# Session: md_session"),
        "missing session heading"
    );

    // Should contain a turn heading.
    assert!(markdown.contains("## Turn 1"), "missing turn heading");

    // Should contain both the user and assistant content.
    assert!(markdown.contains("What is Rust?"), "missing user content");
    assert!(
        markdown.contains("A systems programming language."),
        "missing assistant content"
    );

    // User and assistant labels should be present.
    assert!(markdown.contains("**User:**"), "missing user label");
    assert!(
        markdown.contains("**Assistant:**"),
        "missing assistant label"
    );
}

// ---------------------------------------------------------------------------
// 3. Session search — find sessions / messages by keyword.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_search() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    // Three sessions with distinct content.
    store
        .append(
            "alpha",
            msg("user", "tell me about tokio async runtime"),
            msg("assistant", "Tokio is a Rust async runtime."),
        )
        .await;
    store
        .append(
            "beta",
            msg("user", "explain ownership in Rust"),
            msg("assistant", "Ownership prevents data races."),
        )
        .await;
    store
        .append(
            "gamma",
            msg("user", "what is a borrow checker"),
            msg("assistant", "It enforces ownership rules."),
        )
        .await;

    // Search across all sessions for "tokio".
    let matching = store.search("tokio").await;
    assert_eq!(matching.len(), 1, "only 'alpha' contains 'tokio'");
    assert_eq!(matching[0], "alpha");

    // Search within the "beta" session for "ownership".
    let in_session = store.search_in_session("beta", "ownership").await;
    assert!(
        !in_session.is_empty(),
        "should find 'ownership' in beta session"
    );
    assert!(
        in_session.iter().any(|c| c.contains("ownership")),
        "matched content should contain 'ownership'"
    );

    // Search for a term that does not exist.
    let none = store.search("xyzzy_not_found_9999").await;
    assert!(none.is_empty(), "should find nothing for a missing term");
}

// ---------------------------------------------------------------------------
// 4. Multi-append — verify ordering across multiple appends.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_multiple_appends_ordering() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new_with_dir(dir.path().to_path_buf());

    for i in 0..5u32 {
        store
            .append(
                "order_test",
                msg("user", &format!("question {}", i)),
                msg("assistant", &format!("answer {}", i)),
            )
            .await;
    }

    let history = store.get_history("order_test").await;
    assert_eq!(history.len(), 10);

    // Verify interleaved ordering.
    for i in 0..5usize {
        assert_eq!(history[i * 2].role, "user");
        assert_eq!(history[i * 2].content, format!("question {}", i));
        assert_eq!(history[i * 2 + 1].role, "assistant");
        assert_eq!(history[i * 2 + 1].content, format!("answer {}", i));
    }
}
