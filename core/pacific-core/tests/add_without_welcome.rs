//! An Add whose Welcome cannot be delivered is undone (NC-36): a roster member with no
//! Welcome has no way in, and would be counted as present.

mod common;

use common::Harness;

#[tokio::test]
async fn an_add_whose_welcome_cannot_go_is_undone() {
    let h = Harness::new(&["ada", "bo"]).await;
    // The name rides the GroupContext, so the Welcome carries it and no commit or Delta
    // does: 200,000 characters put the Welcome over the relay's default cap, and nothing
    // else near it.
    let obj = h.form_named_forum(0, &"n".repeat(200_000));
    let before = h.roster_people(0, &obj);
    let bundle = h.node(1).build_contact_bundle().unwrap();
    let err = h.node(0).group_add_member(&obj, &bundle).await.expect_err("an Add with no Welcome must not stand");
    assert!(err.to_string().contains("undone"), "{err}");
    assert_eq!(h.roster_people(0, &obj), before, "bo is not left on the roster with no way in");
}
