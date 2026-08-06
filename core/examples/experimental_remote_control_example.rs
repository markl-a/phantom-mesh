//! Example: experimental-remote-control (O2).
//!
//! Builds a WhatsApp stub + Slack stub, exercises the Channel trait:
//!   (1) is_user_allowed respects the allowlist
//!   (2) send_message returns ChannelError::NotImplemented (loud failure)
//!
//! Run:
//!   CARGO_TARGET_DIR=D:/tmp/skill-docs-target \
//!     cargo run -p spectyn-mesh \
//!       --example experimental_remote_control_example \
//!       --features experimental-remote-control
//!
//! Expected last line: `experimental-remote-control OK`. Exit code 0.

use spectyn_mesh::remote_control::slack::SlackStub;
use spectyn_mesh::remote_control::whatsapp::WhatsappStub;
use spectyn_mesh::remote_control::{Channel, ChannelError};

async fn exercise<C: Channel>(c: &C, allowed: i64, denied: i64) -> &'static str {
    assert!(
        c.is_user_allowed(allowed),
        "{} must allow {allowed}",
        c.name()
    );
    assert!(
        !c.is_user_allowed(denied),
        "{} must deny {denied}",
        c.name()
    );
    let err = c
        .send_message(allowed, "ping")
        .await
        .expect_err("stub must fail");
    match err {
        ChannelError::NotImplemented { channel, .. } => channel,
        other => panic!("expected NotImplemented, got {other:?}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let wa = WhatsappStub::with_allowed_users(vec![42]);
    let got = exercise(&wa, 42, 99).await;
    assert_eq!(got, "whatsapp");
    println!("[1] whatsapp stub: allowlist gates + send_message=NotImplemented (channel={got})");

    let sl = SlackStub::with_allowed_users(vec![100, 200]);
    let got = exercise(&sl, 200, 7).await;
    assert_eq!(got, "slack");
    println!("[2] slack stub:    allowlist gates + send_message=NotImplemented (channel={got})");

    // Empty allowlist = open access.
    let wa_open = WhatsappStub::new();
    assert!(wa_open.is_user_allowed(1));
    assert!(wa_open.is_user_allowed(99_999));
    println!("[3] empty allowlist permits everyone");

    println!("experimental-remote-control OK");
    Ok(())
}
