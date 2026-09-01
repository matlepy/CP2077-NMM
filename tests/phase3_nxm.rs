//! Phase 3: Nexus API integration tests (URL parser + DTO roundtrip).

use nexus_mod_manager::api::NexusClient;

#[test]
fn parse_nxm_extracts_mod_file_and_key() {
    let uri = "nxm://cyberpunk2077/mods/42/files/1234?key=ABCD&expires=1700000000";
    let parsed = NexusClient::parse_nxm(uri).unwrap();
    assert_eq!(parsed.game_domain, "cyberpunk2077");
    assert_eq!(parsed.mod_id, 42);
    assert_eq!(parsed.file_id, 1234);
    assert_eq!(parsed.key.as_deref(), Some("ABCD"));
    assert_eq!(parsed.expires, Some(1700000000));
}

#[test]
fn parse_nxm_rejects_non_nxm_uri() {
    let result = NexusClient::parse_nxm("https://example.com/mods/42");
    assert!(result.is_err());
}

#[test]
fn parse_nxm_rejects_malformed_path() {
    let result = NexusClient::parse_nxm("nxm://cyberpunk2077/notmods/42");
    assert!(result.is_err());
}

#[test]
fn parse_nxm_accepts_missing_query_params() {
    let uri = "nxm://cyberpunk2077/mods/7/files/8";
    let parsed = NexusClient::parse_nxm(uri).unwrap();
    assert_eq!(parsed.mod_id, 7);
    assert_eq!(parsed.file_id, 8);
    assert!(parsed.key.is_none());
    assert!(parsed.expires.is_none());
}
