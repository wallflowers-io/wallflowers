//! Detached media end to end (O-79, `media.detached`, #56): sealed bytes up through the relay's
//! presign to object storage, and back, by a reader that needs only the reference. Object
//! storage is a stand-in on this machine; the relay presigns for it over plain http
//! (`Media::plain_on_loopback`), with the same signatures it gives R2.

use pacific_core::media::Delivery;
use pacific_core::media_store;

use crate::common::media::{object_store, relay_presigning_for, FETCHES};

#[tokio::test]
async fn sealed_bytes_go_up_and_come_back_to_anyone_holding_the_reference() {
    let (store, objects) = object_store().await;
    let relay = relay_presigning_for(&store).await;
    let photo = b"\xff\xd8\xff\xe0 a jpeg's bytes, as far as anyone storing them can tell".repeat(1000);

    let before = media_store::presigned();
    let r = media_store::put(&relay, None, &photo, "image/jpeg").await.unwrap();
    assert!(media_store::presigned() > before, "a presign is counted");
    let Delivery::Sealed { ref key, bytes, .. } = r.delivery else { panic!("a sealed reference: {r:?}") };
    let held = objects.lock().unwrap().get(&format!("/pacific-media/{key}")).cloned().expect("the object is stored under its key");
    assert_eq!(held.len() as u64, bytes);
    assert!(!held.windows(16).any(|w| photo.windows(16).next() == Some(w)), "what is stored is sealed");

    // Read by a reader with nothing but the reference: no Node, no group.
    assert_eq!(media_store::get(&relay, None, &r).await.unwrap(), photo);

    // A device that has the object never fetches it again.
    let kept = tempfile::tempdir().unwrap();
    let (presigns, fetches) = (media_store::presigned(), FETCHES.load(std::sync::atomic::Ordering::SeqCst));
    for _ in 0..2 {
        assert_eq!(media_store::get(&relay, Some(kept.path()), &r).await.unwrap(), photo);
    }
    assert_eq!(media_store::presigned() - presigns, 1, "one presign for two reads");
    assert_eq!(FETCHES.load(std::sync::atomic::Ordering::SeqCst) - fetches, 1, "one fetch for two reads");
    // What is kept is checked as it is read back: a changed copy is fetched again, not opened.
    std::fs::write(kept.path().join(key), b"not the object").unwrap();
    assert_eq!(media_store::get(&relay, Some(kept.path()), &r).await.unwrap(), photo);

    // A reference built by hand with a key that is not a name is refused before any file is read.
    let mut stray = r.clone();
    if let Delivery::Sealed { ref mut key, .. } = stray.delivery {
        *key = "../../etc/passwd".into();
    }
    assert!(media_store::get(&relay, Some(kept.path()), &stray).await.is_err());

    // Its uploader keeps it from the start.
    let made = tempfile::tempdir().unwrap();
    let mine = media_store::put(&relay, Some(made.path()), b"mine", "image/jpeg").await.unwrap();
    let presigns = media_store::presigned();
    assert_eq!(media_store::get(&relay, Some(made.path()), &mine).await.unwrap(), b"mine");
    assert_eq!(media_store::presigned(), presigns, "the uploader reads its own without asking");

    // An upload only creates: a second presigned PUT to the same key is refused.
    let mut s = pacific_core::transport::RelaySession::connect(&relay).await.unwrap();
    let again = s.media_url(key, Some(4)).await.unwrap();
    let res = reqwest::Client::new().put(again).header("If-None-Match", "*").body("junk").send().await.unwrap();
    assert_eq!(res.status(), 412);
    assert_eq!(media_store::get(&relay, None, &r).await.unwrap(), photo, "still the referenced object");

    // A longer object is refused by its length, before it is held.
    objects.lock().unwrap().get_mut(&format!("/pacific-media/{key}")).unwrap().push(0);
    assert!(media_store::get(&relay, None, &r).await.is_err());
    objects.lock().unwrap().get_mut(&format!("/pacific-media/{key}")).unwrap().pop();

    // A changed object is not the one referenced.
    objects.lock().unwrap().get_mut(&format!("/pacific-media/{key}")).unwrap()[30] ^= 1;
    assert!(media_store::get(&relay, None, &r).await.is_err());
}
