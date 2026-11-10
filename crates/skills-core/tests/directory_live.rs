//! Live checks against skills.sh. They hit the network, so they're ignored by
//! default. Run them when the site changes or before a release:
//!
//!     cargo test -p skills-core --test directory_live -- --ignored

use skills_core::directory::{AuditStatus, DirectoryClient};

fn client() -> DirectoryClient {
    DirectoryClient::new().expect("client")
}

#[tokio::test]
#[ignore = "network"]
async fn live_audit() {
    let audit = client()
        .audit("vercel-labs/agent-skills", "web-design-guidelines")
        .await
        .expect("request")
        .expect("audited");
    assert!(!audit.entries.is_empty());
    assert_ne!(audit.overall(), AuditStatus::Unknown);
}

#[tokio::test]
#[ignore = "network"]
async fn live_audit_not_found() {
    let audit = client()
        .audit("skillsmate-nonexistent", "nothing")
        .await
        .expect("request");
    assert!(audit.is_none());
}

#[tokio::test]
#[ignore = "network"]
async fn live_official() {
    let official = client().official().await.expect("official");
    assert!(
        official.owners.len() > 20,
        "{} owners",
        official.owners.len()
    );
    assert!(official.owners.iter().any(|o| o.owner == "anthropics"));
}

#[tokio::test]
#[ignore = "network"]
async fn live_topics() {
    let c = client();
    let topics = c.topics().await.expect("topics");
    assert!(topics.len() >= 4, "{topics:?}");
    let first = c.topic(&topics[0].slug).await.expect("topic");
    assert!(!first.skills.is_empty());
}

#[tokio::test]
#[ignore = "network"]
async fn live_download() {
    let files = client()
        .download("vercel-labs/agent-skills", "web-design-guidelines")
        .await
        .expect("request")
        .expect("snapshot");
    assert!(files.contains_key("SKILL.md"));
}
