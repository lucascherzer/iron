use super::*;
use crate::test_utils::{TestPrivateFileWriter, test_endpoint_id};

#[test]
fn validates_checkpoint_status() {
    let cases = [
        ((Some(0), Some(0), Some(0)), true),
        ((Some(0), Some(4), Some(4)), true),
        ((Some(0), Some(-1), Some(-1)), true),
        ((None, None, None), true),
        ((Some(1), Some(4), Some(4)), false),
        ((Some(0), Some(4), Some(3)), false),
        ((Some(0), None, Some(0)), false),
        ((None, Some(0), Some(0)), false),
        ((Some(0), Some(0), None), false),
    ];
    for (status, expected) in cases {
        assert_eq!(
            validate_checkpoint_status(status).is_ok(),
            expected,
            "{status:?}"
        );
    }
}

#[tokio::test]
async fn checkpoint_persists_database_main_file_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let state = AppState::open(&path, &TestPrivateFileWriter).await.unwrap();
    let peer = test_endpoint_id(41);
    state.save_peers([peer]).await.unwrap();
    state.checkpoint().await.unwrap();
    drop(state);

    let main_file_copy = dir.path().join("copied.db");
    std::fs::copy(&path, &main_file_copy).unwrap();
    let copied = AppState::open(&main_file_copy, &TestPrivateFileWriter)
        .await
        .unwrap();
    assert_eq!(copied.load_peers().await.unwrap(), vec![peer]);
}
